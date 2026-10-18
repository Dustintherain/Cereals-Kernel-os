//! ELF64 装载器（v0.7 实现）
//!
//! v0.6 的用户程序是「构建期摊平成连续映像 + 内核原样拷贝」，代价是：代码页
//! 只能整体只读，用户程序**不能写静态数据**，也不是从文件加载的。v0.7 换掉这条路：
//!
//! ```text
//! RAMFS(/init) --vfs::read_all--> ELF 字节 --elf::load--> 用户地址空间（按段权限）
//! ```
//!
//! 装载做的事（每一步都在**内核 CR3 下**、切换 CR3 之前完成）：
//!
//! 1. **解析**：只接受 `ELF64 / little-endian / ET_EXEC / EM_X86_64`，
//!    且每个 `PT_LOAD` 都落在用户映像窗口 `[USER_VIRT_BASE, +64 KiB)` 内；
//! 2. **映射**：按**页**并集各段的权限位（`R` → `NX`、`W` → `WRITABLE`、
//!    `X` → 可执行），页帧在首次映射时分配并清零；
//! 3. **拷贝**：把每段 `p_filesz` 字节写进对应页，`p_memsz - p_filesz`
//!    的部分保持 0（这就是 `.bss`）；
//! 4. **入口**：直接用 ELF 头的 `e_entry`（不再要求「映像第 0 字节就是入口」）。
//!
//! 三条硬约束（与 `user/hello/user.ld` 成对，见 libc/README.md）：
//!
//! - 链接地址必须 = [`USER_VIRT_BASE`]（内核不做加载时重定位，链接地址 = 运行地址）；
//! - 段必须**页对齐**（`ALIGN(0x1000)`）：否则一个页里同时有 `R E` 与 `RW`
//!   数据，页级权限只能取并集，`W^X` 就退化了；
//! - 不允许可装载重定位（`SHT_REL/RELA` + `SHF_ALLOC`）：v0.7 无动态链接。
//!   构建期由 `kernel/build.rs` 断言，装载期再查一次（`ENOEXEC`）。
//!
//! 未做（属 v0.8+）：`PT_INTERP`/动态链接、`PT_GNU_STACK` 大小、`argc/argv`、
//! 用户堆（`brk`/`mmap`）、`PIE`（当前只接受 `ET_EXEC`）。

use alloc::vec::Vec;

use crate::arch::x86_64::paging::{NO_EXEC, WRITABLE};
use crate::driver::serial;
use crate::fs::vfs::ENOEXEC;
use crate::memory::page::{align_down, align_up, PAGE_SIZE};
use crate::syscall::address_space::{LoadStats, UserSpace, USER_IMAGE_MAX, USER_VIRT_BASE};

/// `p_type`：可装载段。
pub(crate) const PT_LOAD: u32 = 1;
/// `p_flags`：可执行。
const PF_X: u32 = 1;
/// `p_flags`：可写。
const PF_W: u32 = 2;
/// 段数上限（`user/*` 实测 4 段；上限只为挡住畸形 ELF）。
const MAX_SEGMENTS: usize = 8;
/// ELF64 文件头大小。
pub(crate) const EHDR_SIZE: usize = 64;
/// ELF64 程序头大小。
pub(crate) const PHDR_SIZE: usize = 56;

/// 一个可装载段。
#[derive(Clone, Copy, Debug)]
struct Segment {
    /// 虚拟地址（链接地址 = 运行地址）
    vaddr: u64,
    /// 文件内字节数
    filesz: u64,
    /// 内存内字节数（`filesz` 之后补零 = `.bss`）
    memsz: u64,
    /// 文件内偏移
    offset: u64,
    /// `p_flags`
    flags: u32,
}

/// 解析 ELF 并把它装进 `space`。
///
/// 入口按 `e_type` 分流：
/// - `ET_EXEC`（2）：v0.7 起的静态用户程序，走下面的固定地址装载路径；
/// - `ET_DYN`（3）：v0.9 的动态链接对象（PIE 主程序 / 共享库），交给
///   [`crate::syscall::dynamic::load`]（`PT_INTERP` + `DT_NEEDED` + 重定位）。
pub fn load(space: &mut UserSpace, elf: &[u8]) -> Result<LoadStats, i64> {
    if elf.len() >= EHDR_SIZE && &elf[..4] == b"\x7fELF" && u16_at(elf, 16) == Some(3) {
        return crate::syscall::dynamic::load(space, elf);
    }
    let header = parse_header(elf)?;
    let segments = parse_segments(elf, header.phoff, header.phentsize, header.phnum)?;

    // 入口必须落在某个段里（否则装载成功但一进去就 #PF，报错点离原因太远）
    let entry = header.entry;
    let in_segment = segments
        .iter()
        .any(|seg| entry >= seg.vaddr && entry < seg.vaddr + seg.memsz);
    if !in_segment {
        return Err(reject("entry point outside PT_LOAD segments"));
    }
    check_wx(&segments)?;

    // 1. 逐段逐页映射 + 拷贝。同一个页被多个段覆盖时 `map_image_page` 只并集权限，
    //    因此这里不会重复清零、也不会擦掉先前段的数据。
    let mut writable_pages = 0u32;
    let mut bytes = 0u64;
    for seg in &segments {
        bytes = bytes
            .checked_add(seg.memsz)
            .ok_or_else(|| reject("segment size overflow"))?;
        let start = align_down(seg.vaddr, PAGE_SIZE);
        let end = align_up(seg.vaddr + seg.memsz, PAGE_SIZE);
        let mut page = start;
        while page < end {
            let index = ((page - USER_VIRT_BASE) / PAGE_SIZE) as usize;
            let phys = space.map_image_page(index, page_flags(seg.flags))?;

            if seg.flags & PF_W != 0 {
                writable_pages |= 1 << index;
            }

            // 该段落在本页内的文件字节区间
            let copy_start = seg.vaddr.max(page);
            let copy_end = (seg.vaddr + seg.filesz).min(page + PAGE_SIZE);
            if copy_start < copy_end {
                let in_page = (copy_start - page) as usize;
                let file_at = seg
                    .offset
                    .checked_add(copy_start - seg.vaddr)
                    .ok_or_else(|| reject("segment file offset overflow"))?
                    as usize;
                let len = (copy_end - copy_start) as usize;
                // SAFETY: 物理帧由本空间拥有；内核 CR3 下物理地址恒等可写
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        elf.as_ptr().add(file_at),
                        (phys + in_page as u64) as *mut u8,
                        len,
                    );
                }
            }
            page += PAGE_SIZE;
        }
    }

    let pages = space.image_pages();
    if pages == 0 {
        return Err(reject("no page mapped"));
    }

    Ok(LoadStats {
        entry,
        segments: segments.len(),
        bytes,
        pages,
        writable_pages: writable_pages.count_ones() as usize,
    })
}

/// ELF 头里用到的字段。
struct Header {
    entry: u64,
    phoff: u64,
    phentsize: u16,
    phnum: u16,
}

/// 校验并读出 ELF 头字段。
fn parse_header(elf: &[u8]) -> Result<Header, i64> {
    if elf.len() < EHDR_SIZE {
        return Err(reject("file shorter than ELF64 header"));
    }
    if &elf[..4] != b"\x7fELF" {
        return Err(reject("bad magic"));
    }
    if elf[4] != 2 {
        return Err(reject("not ELF64 (EI_CLASS)"));
    }
    if elf[5] != 1 {
        return Err(reject("not little-endian (EI_DATA)"));
    }
    // e_type：ET_EXEC = 2（PIE/ET_DYN 需要加载时重定位，v0.7 不支持）
    let e_type = u16_at(elf, 16).ok_or_else(|| reject("bad e_type"))?;
    if e_type != 2 {
        return Err(reject("not ET_EXEC (link with -C relocation-model=static)"));
    }
    // e_machine：EM_X86_64 = 62
    let e_machine = u16_at(elf, 18).ok_or_else(|| reject("bad e_machine"))?;
    if e_machine != 62 {
        return Err(reject("not x86_64"));
    }
    let entry = u64_at(elf, 24).ok_or_else(|| reject("bad e_entry"))?;
    let phoff = u64_at(elf, 32).ok_or_else(|| reject("bad e_phoff"))?;
    let phentsize = u16_at(elf, 54).ok_or_else(|| reject("bad e_phentsize"))?;
    let phnum = u16_at(elf, 56).ok_or_else(|| reject("bad e_phnum"))?;
    if phentsize as usize != PHDR_SIZE {
        return Err(reject("unexpected program header size"));
    }
    if phnum == 0 || phnum as usize > MAX_SEGMENTS {
        return Err(reject("program header count out of range"));
    }
    if !within_image(entry) {
        return Err(reject("entry point outside user image window"));
    }
    // 程序头表必须整段落在文件内（`phoff + phnum*phentsize` 用 checked，
    // 否则畸形文件的一次溢出就能让后面的解析越界或直接 panic）。
    let table_len = (phnum as u64)
        .checked_mul(phentsize as u64)
        .and_then(|n| phoff.checked_add(n))
        .ok_or_else(|| reject("program header table overflow"))?;
    if table_len > elf.len() as u64 {
        return Err(reject("program headers outside file"));
    }
    Ok(Header {
        entry,
        phoff,
        phentsize,
        phnum,
    })
}

/// 建 `PT_LOAD` 段表（同时完成全部边界校验）。
fn parse_segments(elf: &[u8], phoff: u64, phentsize: u16, phnum: u16) -> Result<Vec<Segment>, i64> {
    let phoff = phoff as usize;
    let ehentry = phentsize as usize;
    let mut segments = Vec::new();
    for i in 0..phnum as usize {
        // 头部已经保证整张表在文件内；这里仍用 checked 算子，"绝不 panic"
        // 这条约束不能依赖调用方（畸形 ELF 是正常的输入）。
        let ph = i
            .checked_mul(ehentry)
            .and_then(|n| phoff.checked_add(n))
            .ok_or_else(|| reject("program header offset overflow"))?;
        if ph.checked_add(PHDR_SIZE).map_or(true, |end| end > elf.len()) {
            return Err(reject("program header outside file"));
        }
        if u32_at(elf, ph).ok_or_else(|| reject("bad program header"))? != PT_LOAD {
            continue;
        }
        let flags = u32_at(elf, ph + 4).ok_or_else(|| reject("bad PT_LOAD flags"))?;
        let offset = u64_at(elf, ph + 8).ok_or_else(|| reject("bad PT_LOAD offset"))?;
        let vaddr = u64_at(elf, ph + 16).ok_or_else(|| reject("bad PT_LOAD vaddr"))?;
        let filesz = u64_at(elf, ph + 32).ok_or_else(|| reject("bad PT_LOAD filesz"))?;
        let memsz = u64_at(elf, ph + 40).ok_or_else(|| reject("bad PT_LOAD memsz"))?;

        if filesz > memsz {
            return Err(reject("PT_LOAD p_filesz > p_memsz"));
        }
        let file_end = offset
            .checked_add(filesz)
            .ok_or_else(|| reject("PT_LOAD offset overflow"))?;
        if file_end > elf.len() as u64 {
            return Err(reject("PT_LOAD extends past end of file"));
        }
        if !within_image(vaddr) {
            return Err(reject("PT_LOAD below user image base"));
        }
        let end = vaddr
            .checked_add(memsz)
            .ok_or_else(|| reject("PT_LOAD address overflow"))?;
        if end > USER_VIRT_BASE + USER_IMAGE_MAX {
            return Err(reject("PT_LOAD past end of user image window"));
        }
        segments.push(Segment {
            vaddr,
            filesz,
            memsz,
            offset,
            flags,
        });
    }
    if segments.is_empty() {
        return Err(reject("no PT_LOAD segment"));
    }
    check_alloc_free(elf)?;
    Ok(segments)
}

/// 拒绝「可装载段里带重定位」的 ELF（v0.7 无动态链接/加载时重定位）。
///
/// 与 `kernel/build.rs` 的构建期断言同一条件；构建期拦在编译阶段，这里再拦一次
/// 运行期（例如程序来自别处、或链接脚本被改坏）。
fn check_alloc_free(elf: &[u8]) -> Result<(), i64> {
    let shoff = u64_at(elf, 40).ok_or_else(|| reject("bad e_shoff"))?;
    let shentsize = u16_at(elf, 58).ok_or_else(|| reject("bad e_shentsize"))? as usize;
    let shnum = u16_at(elf, 60).ok_or_else(|| reject("bad e_shnum"))? as usize;
    if shoff == 0 || shnum == 0 {
        return Ok(()); // 没有节表（strip 过的映像）：没有可装载重定位段
    }
    // 节头表同样要整段落在文件内，且条目至少 64 字节（没有这条，`shentsize=0`
    // 会把每个循环都读到同一个偏移，永远不变）。
    if shentsize < 64 {
        return Err(reject("unexpected section header size"));
    }
    let table_len = (shnum as u64)
        .checked_mul(shentsize as u64)
        .and_then(|n| shoff.checked_add(n))
        .ok_or_else(|| reject("section header table overflow"))?;
    if table_len > elf.len() as u64 {
        return Err(reject("section headers outside file"));
    }
    let shoff = shoff as usize;
    for i in 0..shnum {
        let sh = shoff + i * shentsize;
        let sh_type = u32_at(elf, sh + 4).ok_or_else(|| reject("bad section header"))?;
        let sh_flags = u64_at(elf, sh + 8).ok_or_else(|| reject("bad section flags"))?;
        // SHT_REL = 9, SHT_RELA = 4；SHF_ALLOC = 0x2
        if (sh_type == 4 || sh_type == 9) && sh_flags & 0x2 != 0 {
            return Err(reject("loadable relocations present"));
        }
    }
    Ok(())
}

/// 拒绝「可写段与可执行段落在同一页」的 ELF。
///
/// 页权限只能取并集：同页既 `W` 又 `X` 会让 `W^X` 退化（数据页可执行）。
/// 触发条件只有一个 —— 链接脚本没有把段页对齐（见 `user/hello/user.ld`）。
fn check_wx(segments: &[Segment]) -> Result<(), i64> {
    for (i, seg) in segments.iter().enumerate() {
        if seg.flags & PF_W == 0 {
            continue;
        }
        let start = align_down(seg.vaddr, PAGE_SIZE);
        let end = align_up(seg.vaddr + seg.memsz, PAGE_SIZE);
        for other in segments.iter().skip(i + 1) {
            if other.flags & PF_X == 0 {
                continue;
            }
            let o_start = align_down(other.vaddr, PAGE_SIZE);
            let o_end = align_up(other.vaddr + other.memsz, PAGE_SIZE);
            if start < o_end && o_start < end {
                return Err(reject(
                    "W and X segments share a page (linker script not page-aligned)",
                ));
            }
        }
    }
    Ok(())
}

/// 段权限位 → 页表项标志。
fn page_flags(p_flags: u32) -> u64 {
    let mut flags = 0;
    if p_flags & PF_W != 0 {
        flags |= WRITABLE;
    }
    if p_flags & PF_X == 0 {
        flags |= NO_EXEC; // 数据段不可执行（W^X：只写不执行）
    }
    flags
}

/// 地址是否在用户映像窗口内。
fn within_image(addr: u64) -> bool {
    addr >= USER_VIRT_BASE && addr < USER_VIRT_BASE + USER_IMAGE_MAX
}

/// 统一拒绝出口：串口留痕（`user: ELF rejected: <reason>`）+ `-ENOEXEC`。
fn reject(reason: &str) -> i64 {
    serial::print("user: ELF rejected: ");
    serial::println(reason);
    ENOEXEC
}

// ---- 小端读取（越界返回 `None`，绝不 panic：输入是不可信的文件字节） ----

pub(crate) fn u16_at(buf: &[u8], off: usize) -> Option<u16> {
    let bytes = buf.get(off..off + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    let bytes = buf.get(off..off + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub(crate) fn u64_at(buf: &[u8], off: usize) -> Option<u64> {
    let bytes = buf.get(off..off + 8)?;
    let mut raw = [0u8; 8];
    raw.copy_from_slice(bytes);
    Some(u64::from_le_bytes(raw))
}
