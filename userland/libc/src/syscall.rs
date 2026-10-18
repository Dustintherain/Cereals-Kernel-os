//! 系统调用封装（v0.6 首批：`write` / `getpid` / `nanosleep` / `exit`；
//! v0.7 增加 `open` / `read` / `close`）
//!
//! 约定与内核 `kernel/src/syscall/syscall_table.rs` 完全对齐（Linux x86_64 语义）：
//!
//! | 项 | 取值 |
//! | ---- | ---- |
//! | 指令 | `syscall`（入口 `IA32_LSTAR` → 内核 `syscall_entry`，见 `kernel/src/syscall/trampoline.rs`） |
//! | 号 | `rax` |
//! | 参数 | `rdi, rsi, rdx, r10, r8` |
//! | 返回 | `rax`；负数 = `-errno`（如 `-38` = `ENOSYS`、`-14` = `EFAULT`） |
//!
//! v0.7 的文件 syscall 形状（与 Linux 的差异写在类型里，避免误用）：
//!
//! - `open(path_ptr, path_len, flags)`：路径用**指针 + 长度**而不是 NUL 结尾 ——
//!   内核已经校验过用户缓冲区间，不需要在不可信内存上找终止符；
//! - `read(fd, buf, count)`：`buf` 必须对 ring3 **可写**（内核按页检查）；
//! - `write(fd, buf, count)`：`buf` 必须可读；fd 1/2 写串口，fd ≥ 3 写文件；
//! - `close(fd)`：只接受 fd ≥ 3（0/1/2 是保留位）。
//!
//! v0.9 第一步加入用户堆（`mmap` / `munmap` / `brk`，内核侧 `kernel/src/syscall/memory.rs`）：
//!
//! - `mmap(len, prot, flags)` 只支持**私有匿名**映射（`MAP_PRIVATE|MAP_ANONYMOUS`），
//!   且**没有 offset 参数** —— 内核的 syscall ABI 传 5 个参数（`rdi, rsi, rdx, r10, r8`），
//!   匿名映射不需要偏移；文件映射要先扩 ABI（见内核模块文档）；
//! - `PROT_WRITE|PROT_EXEC` 被内核拒绝（W^X）；`MAP_FIXED` 同样拒绝（固定地址放置未实现），
//!   `PROT_NONE` 现在合法（v1.0 起可以再 `mprotect` 启用）；
//! - `brk` 沿用 Linux 语义：**不返回错误**，失败时返回原值（用 `ret < addr` 判断）。
//!
//! v1.0 第一步补上 `mprotect`（内核侧 `syscall/memory.rs::sys_mprotect`）：改写**已映射**
//! 页的保护属性，W^X 的「先 `mmap(RW)` 写、再 `mprotect(RX)` 执行」这条路因此成立。
//! [`PROT_NONE`] 表示「本进程不可访问」（内核保留页帧与内容，可以再改回来）。
//!
//! v0.9 第三步加入了进程管理（内核侧 `kernel/src/syscall/process.rs`）：
//!
//! - `fork()`：父进程返回子进程 PID、子进程返回 0；子进程从**同一条用户指令**
//!   继续执行（内核克隆了 `fork` 那一刻的完整寄存器帧）；
//! - `execve(path, argv)`：换映像，成功后**不返回**；`argv` 用「指针 + 长度」对数组传
//!   （内核逐段校验后写进新进程的用户栈，见 `libc::args`）；
//! - `wait4(pid)`：运行并回收直接子进程，返回退出码。
//!
//! **v1.0 第一步起没有「号已定义但未实现」的调用了**（`mprotect` 是最后一个，见上）。
//!
//! 与 Linux 的调度差异：本内核单 CPU、用户态无抢占，子进程不会与父进程并发运行，
//! 它要等父进程 `wait4` 才被恢复 —— 标准写法 `if pid == 0 { execve } else { wait4 }`
//! 因此原样可用（详见 `docs/process.md`）。

/// 系统调用号（与内核表一致；不要用魔法数调用 `raw`）。
pub const SYS_READ: u64 = abi::syscall::SYS_READ;
pub const SYS_WRITE: u64 = abi::syscall::SYS_WRITE;
pub const SYS_OPEN: u64 = abi::syscall::SYS_OPEN;
pub const SYS_CLOSE: u64 = abi::syscall::SYS_CLOSE;
pub const SYS_MMAP: u64 = abi::syscall::SYS_MMAP;
pub const SYS_MPROTECT: u64 = abi::syscall::SYS_MPROTECT;
pub const SYS_MUNMAP: u64 = abi::syscall::SYS_MUNMAP;
pub const SYS_BRK: u64 = abi::syscall::SYS_BRK;
pub const SYS_SLEEP: u64 = abi::syscall::SYS_SLEEP; // nanosleep
pub const SYS_GETPID: u64 = abi::syscall::SYS_GETPID;
pub const SYS_FORK: u64 = abi::syscall::SYS_FORK;
pub const SYS_EXEC: u64 = abi::syscall::SYS_EXEC;
pub const SYS_EXIT: u64 = abi::syscall::SYS_EXIT;
pub const SYS_WAIT: u64 = abi::syscall::SYS_WAIT;

/// 标准输入（v0.7 未接通：`read` 返回 0 = EOF）。
pub const STDIN: i32 = abi::flags::STDIN;
/// 标准输出（内核把它接到串口）。
pub const STDOUT: i32 = abi::flags::STDOUT;
/// 标准错误（同样接到串口）。
pub const STDERR: i32 = abi::flags::STDERR;
/// 第一个动态 fd（内核 `fs::file::FD_FIRST_DYNAMIC`）。
pub const FD_FIRST_DYNAMIC: i32 = abi::flags::FD_FIRST_DYNAMIC;

/// `open` 标志：只读。
pub const O_RDONLY: i32 = abi::flags::O_RDONLY;
/// `open` 标志：只写。
pub const O_WRONLY: i32 = abi::flags::O_WRONLY;
/// `open` 标志：读写。
pub const O_RDWR: i32 = abi::flags::O_RDWR;
/// `open` 标志：不存在则创建（父目录必须已存在）。
pub const O_CREAT: i32 = abi::flags::O_CREAT;

/// `-ENOENT`：路径不存在。
pub const ENOENT: isize = abi::errno::ENOENT as isize;
/// `-EBADF`：fd 无效 / 未打开 / 打开模式与操作不符。
pub const EBADF: isize = abi::errno::EBADF as isize;
/// `-EAGAIN`：本次没有数据可用（非阻塞语义，不是错误）。
///
/// v0.9 只有 `read(0)` 会返回它：1.5 秒内键盘没有交付一整行时
/// （内核在无输入时不会无限等待，理由见 `kernel/src/syscall/file.rs::read_stdin`）。
pub const EAGAIN: isize = abi::errno::EAGAIN as isize;
/// `-ENOMEM`：内核内存不足。
pub const ENOMEM: isize = abi::errno::ENOMEM as isize;
/// `-EFAULT`：用户传了不属于自己的地址区间。
pub const EFAULT: isize = abi::errno::EFAULT as isize;
/// `-EISDIR`：目标是目录，但调用方要当文件用。
pub const EISDIR: isize = abi::errno::EISDIR as isize;
/// `-EINVAL`：参数非法（含 `flags` 里的未知位、非法路径）。
pub const EINVAL: isize = abi::errno::EINVAL as isize;
/// `-ENOSPC`：空间不足（fd 用尽 / 文件超过 RAMFS 上限）。
pub const ENOSPC: isize = abi::errno::ENOSPC as isize;
/// `-ENOSYS`：号合法但内核尚未实现（v1.0 第一步实现 `mprotect` 后没有这样的号了，
/// 保留常量是因为内核仍有 `ENOSYS` 桩与「号越界」两种不可实现的失败）。
pub const ENOSYS: isize = abi::errno::ENOSYS as isize;
/// `-ECHILD`：`wait4` 找不到该 PID 的直接子进程。
pub const ECHILD: isize = abi::errno::ECHILD as isize;

/// `mmap` / `mprotect` 保护位：不可访问（页帧仍在，可用 [`mprotect`] 再启用）。
pub const PROT_NONE: i32 = abi::flags::PROT_NONE as i32;
/// `mmap` / `mprotect` 保护位：可读。
pub const PROT_READ: i32 = abi::flags::PROT_READ as i32;
/// `mmap` 保护位：可写。
pub const PROT_WRITE: i32 = abi::flags::PROT_WRITE as i32;
/// `mmap` 保护位：可执行（**不能**与 [`PROT_WRITE`] 同时使用：内核强制 W^X）。
pub const PROT_EXEC: i32 = abi::flags::PROT_EXEC as i32;

/// `mmap` 标志：私有映射（与 [`MAP_ANONYMOUS`] 一起是最小可用组合）。
pub const MAP_PRIVATE: i32 = abi::flags::MAP_PRIVATE as i32;
/// `mmap` 标志：匿名映射（本内核**只**支持它，且要求 `fd = -1`）。
pub const MAP_ANONYMOUS: i32 = abi::flags::MAP_ANONYMOUS as i32;

/// 原始系统调用：`rax` = 号，`rdi/rsi/rdx/r10/r8` = 参数，返回 `rax`。
///
/// **寄存器约定**：与 Linux syscall ABI 一致 —— 一次往返后只有 `rax`（返回值）、
/// `rcx`、`r11` 变化，其余寄存器（包括 `rdi/rsi/rdx/r10/r8`）保持进入前的值。
/// 内核入口汇编为此在调用 Rust 分发函数前保存 `rdi/rsi/rdx/r10/r8/r9`、
/// 返回前逐个恢复（见 `kernel/src/syscall/trampoline.rs` 的栈帧注释）。
///
/// 因此这里用 `in(..)` 而不是 `inout(..) => _`：后者会告诉编译器这些寄存器
/// 被破坏，虽然保守但更悲观。**不要再改回 `inout`** —— 真正需要的是内核侧
/// 恢复寄存器，那样用户代码才能把活跃值留在任意 caller-saved 寄存器里。
///
/// # Safety
/// 这是一个裸系统调用：参数与返回值的语义由内核决定（见模块文档）。
#[inline(always)]
pub unsafe fn raw(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> isize {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret as isize
}

/// `write(fd, buf, count)`：fd 1/2 写串口，fd ≥ 3 写文件。
///
/// 返回实际写入字节数，负数表示错误（`EBADF` / `EFAULT` / `EINVAL`）。
/// 内核要求缓冲区落在**当前用户地址空间**内（代码页/数据段/用户栈），
/// 否则返回 `EFAULT` 并在串口留下 `user: EFAULT write ...` 记录。
pub fn write(fd: i32, buf: &[u8]) -> isize {
    unsafe { raw(SYS_WRITE, fd as u64, buf.as_ptr() as u64, buf.len() as u64, 0, 0) }
}

/// `read(fd, buf, count)`：从 `fd` 读入 `buf`，返回读到的字节数（0 = EOF）。
///
/// 内核要求 `buf` 对 ring3 **可写**（栈或 `RW` 段），否则 `EFAULT`。
pub fn read(fd: i32, buf: &mut [u8]) -> isize {
    unsafe { raw(SYS_READ, fd as u64, buf.as_ptr() as u64, buf.len() as u64, 0, 0) }
}

/// `open(path, flags)`：打开（必要时创建）文件，返回 fd（≥ 3）或负 errno。
///
/// 路径用「指针 + 长度」传递；`flags` 只允许 [`O_RDONLY`]/[`O_WRONLY`]/[`O_RDWR`]
/// 与 [`O_CREAT`] 的组合，其余位内核返回 [`EINVAL`]。
///
/// **CSS-3**：Windows 式路径（`C:\x`）在这一层被重写成 POSIX 路径（`crate::path`，
/// `docs/storage.md` 4.2）—— 内核仍然只看到绝对路径，因此 syscall ABI 不变；
/// 盘符未分配/卷不在表里时直接返回 [`crate::path::ENOENT`]（不把 `C:\x` 丢给内核
/// 去当普通文件名——那会把「没有这个盘符」变成「这个怪名字的文件不存在」）。
pub fn open(path: &str, flags: i32) -> isize {
    let mut rewritten = [0u8; 128];
    let path = if crate::path::is_windows(path) {
        match crate::path::resolve_str(path, &mut rewritten) {
            Ok(resolved) => resolved,
            Err(errno) => return errno,
        }
    } else {
        path
    };
    unsafe {
        raw(
            SYS_OPEN,
            path.as_ptr() as u64,
            path.len() as u64,
            flags as u64,
            0,
            0,
        )
    }
}

/// `close(fd)`：释放 fd（0/1/2 是保留位，关闭它们返回 [`EBADF`]）。
pub fn close(fd: i32) -> isize {
    unsafe { raw(SYS_CLOSE, fd as u64, 0, 0, 0, 0) }
}

/// `brk(addr)`：调整用户堆顶（内核的 brk 堆窗口 256 KiB）。
///
/// `addr = 0` 只查询当前堆顶。返回当前 break —— **失败时返回原值**（Linux 语义），
/// 调用方用 `ret >= addr` 判断成功；`addr` 会被内核向上取整到页。
pub fn brk(addr: u64) -> u64 {
    let ret = unsafe { raw(SYS_BRK, addr, 0, 0, 0, 0) };
    ret as u64
}

/// `mmap(len, prot, flags)`：匿名映射 `len` 字节，返回页对齐地址或负 errno。
///
/// 内核只接受 `MAP_PRIVATE|MAP_ANONYMOUS`（`fd` 由本函数固定传 -1）、
/// 拒绝 `PROT_WRITE|PROT_EXEC`（W^X）与未知保护位；新页保证**全零**。
pub fn mmap(len: usize, prot: i32, flags: i32) -> isize {
    unsafe {
        raw(
            SYS_MMAP,
            0, // addr 只是提示：内核忽略它（不支持 MAP_FIXED）
            len as u64,
            prot as u64,
            flags as u64,
            u64::MAX, // fd = -1
        )
    }
}

/// `mprotect(addr, len, prot)`：改写已映射页的保护属性（v1.0 第一步）。
///
/// 内核要求：`addr` 页对齐、`len > 0`、范围完整落在**同一个**窗口
/// （映像 / brk 堆 / 匿名 mmap / 共享库）且每一页都已映射，否则 [`EINVAL`]；
/// [`PROT_WRITE`]|[`PROT_EXEC`] 仍被拒（W^X）。用户栈的保护属性固定（`RW` + `NX`），
/// `mprotect` 碰它同样是 [`EINVAL`]。
///
/// [`PROT_NONE`] 使这一段对本进程不可访问（页帧与内容保留）：内核连「拿它当缓冲」
/// 都不允许（`write` 这种按页校验的调用会返回 [`EFAULT`]），再 `mprotect` 回来即可用。
pub fn mprotect(addr: u64, len: usize, prot: i32) -> isize {
    unsafe { raw(SYS_MPROTECT, addr, len as u64, prot as u64, 0, 0) }
}

/// `munmap(addr, len)`：整段归还匿名映射（要求页对齐且全段已映射）。
pub fn munmap(addr: u64, len: usize) -> isize {
    unsafe { raw(SYS_MUNMAP, addr, len as u64, 0, 0, 0) }
}

/// `getpid()`：当前用户程序的 PID（内核分配的顺序号）。
pub fn getpid() -> u64 {
    unsafe { raw(SYS_GETPID, 0, 0, 0, 0, 0) as u64 }
}

/// `nanosleep(ticks)`：睡眠 `ticks` 个 PIT tick（100Hz → 1 tick = 10ms）。
///
/// 等待期间内核 `sti; hlt`，PIT 继续推进 tick，因此返回值是真实流逝的 tick 数。
pub fn nanosleep(ticks: u64) -> isize {
    unsafe { raw(SYS_SLEEP, ticks, 0, 0, 0, 0) }
}

/// `execve` 的 `argv` 项数上限（与内核 `syscall::process::ARG_MAX` 一致）。
pub const ARG_MAX: usize = abi::limits::ARG_MAX as usize;

/// `fork()`：复制当前进程。
///
/// - **父进程**：返回子进程 PID（> 0）；
/// - **子进程**：返回 0，并从**同一条用户指令**继续执行（内核恢复了
///   `fork` 那一刻的完整寄存器现场，只把 `rax` 改成 0）；
/// - 失败：返回负 errno（进程表满 → [`EAGAIN`]，地址空间复制失败 → [`ENOMEM`]）。
///
/// 本内核单 CPU、用户态无抢占：子进程**不会与父进程并发运行**，它要等父进程
/// `wait4` 才被恢复。对这段调用代码来说两者没有区别 —— 标准的
/// `if pid == 0 { execve(..) } else { wait4(pid) }` 原样可用。
pub fn fork() -> isize {
    unsafe { raw(SYS_FORK, 0, 0, 0, 0, 0) }
}

/// `execve(path, argv)`：用 `path` 的映像替换当前进程。
///
/// **成功后不返回**（内核换 CR3 + 恢复新入口的帧）；失败返回负 errno，调用方
/// 通常立刻 [`exit`]（Shell 的 `cmd_external` 就是这么写的）。
///
/// `argv` 用「指针 + 长度」对数组传递（与 `open`/`write` 的路径同一种风格）：
/// 内核逐段校验后把字符串与指针数组写进新进程的用户栈。用户态没有堆也能用 ——
/// 这里的临时数组在栈上。
pub fn execve(path: &str, argv: &[&str]) -> isize {
    let n = argv.len().min(ARG_MAX);
    let mut pairs = [0u64; ARG_MAX * 2];
    for (i, arg) in argv.iter().take(n).enumerate() {
        pairs[i * 2] = arg.as_ptr() as u64;
        pairs[i * 2 + 1] = arg.len() as u64;
    }
    unsafe {
        raw(
            SYS_EXEC,
            path.as_ptr() as u64,
            path.len() as u64,
            pairs.as_ptr() as u64,
            n as u64,
            0,
        )
    }
}

/// `wait4(pid)`：运行并回收直接子进程，返回它的退出码（负数 = errno）。
///
/// 找不到该 PID 的直接子进程返回 [`ECHILD`]；重复 `wait4` 同一个 PID 返回
/// 同一个退出码（内核已把它回收成僵尸后立刻释放，第二次会得到 [`ECHILD`]）。
pub fn wait4(pid: i64) -> isize {
    unsafe { raw(SYS_WAIT, pid as u64, 0, 0, 0, 0) }
}

// ---------------------------------------------------------------- 目录列举（getdents）

/// `getdents` 的系统调用号（Linux 同名号；**记录格式与 Linux 不同**，见下）。
pub const SYS_GETDENTS: u64 = abi::syscall::SYS_GETDENTS;
/// 单条目录记录的字节数（必须与内核 `syscall::dir::DIRENT_SIZE` 一致）。
pub const DIRENT_SIZE: usize = abi::dirent::DIRENT_SIZE;
/// 目录记录里名字字段的容量。
pub const DIRENT_NAME_MAX: usize = abi::dirent::NAME_MAX;
/// 目录记录里名字字段的偏移。
pub const DIRENT_NAME_OFFSET: usize = abi::dirent::NAME_OFFSET;

/// `getdents(path, buf)`：把 `path` 的目录项写进 `buf`，返回**记录数**（负数 = errno）。
///
/// 一次调用写满整个目录（装不下就截断，每条记录都是完整的），因此不需要
/// `lseek` 式的续读循环。记录布局（内核侧 `syscall/dir.rs` 是权威定义）：
///
/// ```text
/// 0   kind(1) + reserved(7)   8   size(u64)   16  name_len(u16) + reserved(6)   24  name(64)
/// ```
///
/// 用 [`dirent_name`] / [`dirent_is_dir`] / [`dirent_size`] 解析记录，
/// 不要自己算偏移。`buf` 至少要有 [`DIRENT_SIZE`] 字节，否则内核返回 `-EINVAL`。
pub fn getdents(path: &str, buf: &mut [u8]) -> isize {
    unsafe {
        raw(
            SYS_GETDENTS,
            path.as_ptr() as u64,
            path.len() as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
            0,
        )
    }
}

/// 取第 `index` 条目录记录的原始字节（越界或缓冲过短返回 `None`）。
fn record(buf: &[u8], index: usize, size: usize) -> Option<&[u8]> {
    let start = index.checked_mul(size)?;
    buf.get(start..start.checked_add(size)?)
}

/// 第 `index` 条目录项的名字（非法记录返回空串，不 panic）。
pub fn dirent_name(buf: &[u8], index: usize) -> &str {
    let Some(rec) = record(buf, index, DIRENT_SIZE) else {
        return "";
    };
    let len = (u16::from_le_bytes([rec[16], rec[17]]) as usize).min(DIRENT_NAME_MAX);
    let end = DIRENT_NAME_OFFSET + len;
    match rec.get(DIRENT_NAME_OFFSET..end) {
        Some(bytes) => core::str::from_utf8(bytes).unwrap_or("?"),
        None => "",
    }
}

/// 第 `index` 条目录项是否是目录。
pub fn dirent_is_dir(buf: &[u8], index: usize) -> bool {
    record(buf, index, DIRENT_SIZE).map_or(false, |rec| rec[0] == 1)
}

/// 第 `index` 条目录项的字节数（目录恒 0）。
pub fn dirent_size(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, DIRENT_SIZE) {
        Some(rec) => u64::from_le_bytes([
            rec[8], rec[9], rec[10], rec[11], rec[12], rec[13], rec[14], rec[15],
        ]),
        None => 0,
    }
}

// ---------------------------------------------------------------- 进程表（getprocs）

/// `getprocs` 的系统调用号（**Cerlesse 扩展号段**，Linux x86_64 不使用 256）。
pub const SYS_GETPROCS: u64 = abi::syscall::SYS_GETPROCS;
/// 单条进程记录的字节数（必须与内核 `syscall::process::PROCINFO_SIZE` 一致）。
pub const PROCINFO_SIZE: usize = abi::procinfo::PROCINFO_SIZE;
/// 进程记录里 `comm` 字段的容量。
pub const PROCINFO_NAME_MAX: usize = abi::procinfo::NAME_MAX;
/// 进程记录里 `comm` 字段的偏移。
pub const PROCINFO_NAME_OFFSET: usize = abi::procinfo::NAME_OFFSET;

/// `getprocs(buf)`：把进程表快照写进 `buf`，返回**记录数**（负数 = errno）。
///
/// 记录布局（内核侧 `syscall/process.rs` 是权威定义）：
///
/// ```text
/// 0  pid(u64)   8  ppid(u64)   16  state(1) + reserved(7)   24  comm(16)
/// ```
///
/// 枚举顺序 = 进程表槽位顺序（不承诺按 PID 排序）。`buf` 至少要有
/// [`PROCINFO_SIZE`] 字节，否则内核返回 `-EINVAL`。
pub fn getprocs(buf: &mut [u8]) -> isize {
    unsafe { raw(SYS_GETPROCS, buf.as_ptr() as u64, buf.len() as u64, 0, 0, 0) }
}

/// 第 `index` 条进程记录的 PID（非法记录返回 0）。
pub fn procinfo_pid(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, PROCINFO_SIZE) {
        Some(rec) => u64::from_le_bytes([
            rec[0], rec[1], rec[2], rec[3], rec[4], rec[5], rec[6], rec[7],
        ]),
        None => 0,
    }
}

/// 第 `index` 条进程记录的父进程 PID（0 = 内核启动上下文）。
pub fn procinfo_ppid(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, PROCINFO_SIZE) {
        Some(rec) => u64::from_le_bytes([
            rec[8], rec[9], rec[10], rec[11], rec[12], rec[13], rec[14], rec[15],
        ]),
        None => 0,
    }
}

/// 第 `index` 条进程记录的状态名（内核的 `ProcState::name()` 同名字串）。
pub fn procinfo_state(buf: &[u8], index: usize) -> &'static str {
    match record(buf, index, PROCINFO_SIZE).map(|rec| rec[16]) {
        Some(0) => "READY",
        Some(1) => "RUNNING",
        Some(2) => "ZOMBIE",
        _ => "?",
    }
}

/// 第 `index` 条进程记录的命令名（定长 16 字节，内核已补 0）。
pub fn procinfo_name(buf: &[u8], index: usize) -> &str {
    let Some(rec) = record(buf, index, PROCINFO_SIZE) else {
        return "";
    };
    let end = PROCINFO_NAME_OFFSET + PROCINFO_NAME_MAX;
    match rec.get(PROCINFO_NAME_OFFSET..end) {
        Some(bytes) => {
            let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            core::str::from_utf8(&bytes[..len]).unwrap_or("?")
        }
        None => "",
    }
}

// ---------------------------------------------------------------- 存储卷表（CSS-1）

/// `storage_volumes` 的系统调用号（CSS-1；号段见 `docs/storage.md` 6.3）。
pub const SYS_STORAGE_VOLUMES: u64 = abi::syscall::SYS_STORAGE_VOLUMES;
/// `storage_rescan` 的系统调用号（CSS-1）。
pub const SYS_STORAGE_RESCAN: u64 = abi::syscall::SYS_STORAGE_RESCAN;
/// `storage_mount` 的系统调用号（**CSS-3**）：把卷挂到路径。
pub const SYS_STORAGE_MOUNT: u64 = abi::syscall::SYS_STORAGE_MOUNT;
/// `storage_unmount` 的系统调用号（CSS-3）：卸掉一个挂载点。
pub const SYS_STORAGE_UNMOUNT: u64 = abi::syscall::SYS_STORAGE_UNMOUNT;
/// `storage_assign_letter` 的系统调用号（CSS-3）：分配/取消盘符。
pub const SYS_STORAGE_ASSIGN_LETTER: u64 = abi::syscall::SYS_STORAGE_ASSIGN_LETTER;
/// `storage_mount` 的 `flags` bit0：只读挂载（→ 只读原因 `ro-mount`）。
pub const STORAGE_MOUNT_RDONLY: u64 = abi::volume::MOUNT_RDONLY;
/// `storage_assign_letter` 的「取消」取值（回到缺省规则，而不是「没有盘符」）。
pub const STORAGE_LETTER_NONE: u8 = abi::volume::LETTER_NONE;
/// 单条卷记录的字节数（必须与内核 `storage::volume::VOLUME_RECORD_SIZE` 一致）。
pub const VOLUME_RECORD_SIZE: usize = abi::volume::RECORD_SIZE;
/// `vol_id` 字段里可读字符的上限（记录占 16 字节：15 字符 + NUL）。
pub const VOLUME_ID_MAX: usize = abi::volume::ID_MAX;
/// 卷标字段容量（11 字节，FAT 风格）。
pub const VOLUME_LABEL_MAX: usize = abi::volume::LABEL_MAX;
/// `free_bytes` 的 unknown 哨兵（内核用 `u64::MAX`：0 会让「盘满」与「没实现」分不开）。
pub const VOLUME_FREE_UNKNOWN: u64 = abi::volume::FREE_UNKNOWN;
/// `storage_rescan` 的「全部设备」下标。
pub const STORAGE_RESCAN_ALL: u32 = abi::volume::RESCAN_ALL;

/// `storage_volumes(buf, out_count)`：导出卷表快照，返回**实际写入的记录数**。
///
/// 记录布局（内核 `storage/volume.rs::write_record` 是权威定义，这里只读）：
///
/// ```text
///  0  id(16, NUL 填充)      16  disk_id(u16 LE)   18  part_index(1)
/// 19  fs_kind(1)            20  read_only(1)       21  health(1)
/// 22  flags(u16 LE)         24  letter(1)          25  label(11)
/// 36  uuid(4)               40  total_bytes(u64 LE)
/// 48  free_bytes(u64 LE)    56  mounts(u32 LE)     60  reserved(4, = 0)
/// ```
///
/// `flags` 位：0 可移动介质 / 1 有卷自带 UUID / 2 `free_bytes` 未知 / 3 卷标被截断。
///
/// `buf` 至少要装下一条记录（否则内核返回 `-EINVAL`）；装不下的部分被**整条**截断，
/// `out_count` 回填**总卷数**（先问大小、再决定缓冲）。枚举顺序是确定性的：
/// 磁盘升序 → 分区升序 → `vol_id` 升序。
pub fn storage_volumes(buf: &mut [u8], out_count: Option<&mut u64>) -> isize {
    let count_ptr = match out_count {
        Some(slot) => slot as *mut u64 as u64,
        None => 0,
    };
    unsafe {
        raw(
            SYS_STORAGE_VOLUMES,
            buf.as_ptr() as u64,
            buf.len() as u64,
            count_ptr,
            0,
            0,
        )
    }
}

/// `storage_rescan(disk)`：重扫一块盘（`STORAGE_RESCAN_ALL` = 全部），返回重扫后的卷数。
///
/// CSS-1 里这是受控动作（CKM 落地后需要 `CAP_STORAGE_ADMIN`，见 `docs/storage.md`）。
pub fn storage_rescan(disk: u32) -> isize {
    unsafe { raw(SYS_STORAGE_RESCAN, disk as u64, 0, 0, 0, 0) }
}

/// `storage_mount(vol_id, point, flags)`（**CSS-3**）：把卷挂到路径。
///
/// 需要 `CAP_STORAGE_ADMIN`（判定点 8）：未授权进程拿到 `-EPERM`，
/// 且内核会同时留一条审计事件与一行串口证据（“拒绝三件套”）。
/// 已挂载的卷再加挂载点只加**别名**：两个路径下是同一份数据（不是两份）。
pub fn storage_mount(vol_id: &str, point: &str, flags: u64) -> isize {
    unsafe {
        raw(
            SYS_STORAGE_MOUNT,
            vol_id.as_ptr() as u64,
            vol_id.len() as u64,
            point.as_ptr() as u64,
            point.len() as u64,
            flags,
        )
    }
}

/// `storage_unmount(point)`（CSS-3）：卸掉一个挂载点（卷仍在卷表里）。
///
/// 需要 `CAP_STORAGE_ADMIN`；`point` 不是挂载点时是 `-ENOENT`，卸根是 `-EINVAL`。
pub fn storage_unmount(point: &str) -> isize {
    unsafe {
        raw(
            SYS_STORAGE_UNMOUNT,
            point.as_ptr() as u64,
            point.len() as u64,
            0,
            0,
            0,
        )
    }
}

/// `storage_assign_letter(vol_id, letter)`（CSS-3）：分配/取消盘符。
///
/// `letter` 是 ASCII `'A'..='Z'`；[`STORAGE_LETTER_NONE`] 表示取消自定义映射、
/// 回到缺省规则。字母被别的卷占着是 `-EEXIST`，非法字母是 `-EINVAL`，
/// 卷不存在是 `-ENOENT`。映射会写进 CKM 状态区（重启后字母不变）。
pub fn storage_assign_letter(vol_id: &str, letter: u8) -> isize {
    unsafe {
        raw(
            SYS_STORAGE_ASSIGN_LETTER,
            vol_id.as_ptr() as u64,
            vol_id.len() as u64,
            letter as u64,
            0,
            0,
        )
    }
}

/// 第 `index` 条卷记录的盘符（**CSS-3**，记录偏移 24）。
///
/// `0` = 未分配。盘符是**别名**：真正的身份是 [`volume_id`]（同一 `vol_id` 的两个卷
/// 只有第一个带盘符 —— 一个字母不能同时是两个卷的入口）。
pub fn volume_letter(buf: &[u8], index: usize) -> u8 {
    record(buf, index, VOLUME_RECORD_SIZE).map_or(0, |rec| rec[24])
}

/// 盘符的文本形式（未分配返回 `"-"`）。
pub fn volume_letter_str(buf: &[u8], index: usize) -> &'static str {
    letter_name(volume_letter(buf, index))
}

/// 盘符取值 → 单字符文本（非法或未分配 → `"-"`）。
pub fn letter_name(letter: u8) -> &'static str {
    const NAMES: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
        "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    match letter {
        b'A'..=b'Z' => NAMES[(letter - b'A') as usize],
        _ => "-",
    }
}

/// 第 `index` 条卷记录的稳定标识（`fs:…` / `pt:…` / `raw:…`）。
pub fn volume_id(buf: &[u8], index: usize) -> &str {
    let Some(rec) = record(buf, index, VOLUME_RECORD_SIZE) else {
        return "";
    };
    match rec.get(..VOLUME_ID_MAX + 1) {
        Some(bytes) => {
            let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            core::str::from_utf8(&bytes[..len]).unwrap_or("?")
        }
        None => "",
    }
}

/// 第 `index` 条卷记录所属的块设备号。
pub fn volume_disk(buf: &[u8], index: usize) -> u16 {
    record(buf, index, VOLUME_RECORD_SIZE).map_or(0, |rec| u16::from_le_bytes([rec[16], rec[17]]))
}

/// 第 `index` 条卷记录的分区号（0 = 无分区表的整盘）。
pub fn volume_part(buf: &[u8], index: usize) -> u8 {
    record(buf, index, VOLUME_RECORD_SIZE).map_or(0, |rec| rec[18])
}

/// 第 `index` 条卷记录的文件系统短名（与内核 `FsKind::name()` 同一批字串）。
pub fn volume_fs(buf: &[u8], index: usize) -> &'static str {
    match record(buf, index, VOLUME_RECORD_SIZE).map(|rec| rec[19]) {
        Some(1) => "fat12",
        Some(2) => "fat16",
        Some(3) => "fat32",
        Some(4) => "exfat",
        Some(5) => "ext2",
        Some(6) => "ext4",
        Some(7) => "ntfs",
        Some(8) => "iso9660",
        _ => "unsupported",
    }
}

/// 第 `index` 条卷记录的只读原因（`rw` / `ro-backend` / `ro-mount` / `ro-device` / `ro-dirty`）。
pub fn volume_read_only(buf: &[u8], index: usize) -> &'static str {
    match record(buf, index, VOLUME_RECORD_SIZE).map(|rec| rec[20]) {
        Some(1) => "ro-backend",
        Some(2) => "ro-mount",
        Some(3) => "ro-device",
        Some(4) => "ro-dirty",
        _ => "rw",
    }
}

/// 第 `index` 条卷记录的健康状态（`ok` / `degraded` / `offline` / `unsupported`）。
pub fn volume_health(buf: &[u8], index: usize) -> &'static str {
    match record(buf, index, VOLUME_RECORD_SIZE).map(|rec| rec[21]) {
        Some(1) => "degraded",
        Some(2) => "offline",
        Some(3) => "unsupported",
        _ => "ok",
    }
}

/// 第 `index` 条卷记录的卷标（空卷标返回空串；被截断时 [`volume_label_truncated`] 为真）。
pub fn volume_label(buf: &[u8], index: usize) -> &str {
    let Some(rec) = record(buf, index, VOLUME_RECORD_SIZE) else {
        return "";
    };
    let bytes = &rec[25..25 + VOLUME_LABEL_MAX];
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    core::str::from_utf8(&bytes[..len]).unwrap_or("?")
}

/// 卷标是否被截断（音量太大而 11 字节装不下）。
pub fn volume_label_truncated(buf: &[u8], index: usize) -> bool {
    record(buf, index, VOLUME_RECORD_SIZE).map_or(false, |rec| rec[22] & 0b1000 != 0)
}

/// 容量（字节）。
pub fn volume_total_bytes(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, VOLUME_RECORD_SIZE) {
        Some(rec) => u64::from_le_bytes(rec[40..48].try_into().unwrap_or([0; 8])),
        None => 0,
    }
}

/// 可用容量（字节）；`None` = 未知（内核的 `free_bytes` 哨兵）。
pub fn volume_free_bytes(buf: &[u8], index: usize) -> Option<u64> {
    let rec = record(buf, index, VOLUME_RECORD_SIZE)?;
    let value = u64::from_le_bytes(rec[48..56].try_into().ok()?);
    if value == VOLUME_FREE_UNKNOWN {
        None
    } else {
        Some(value)
    }
}

/// 当前挂载点数（CSS-1 恒 0：挂载表还没接到卷身份上）。
pub fn volume_mounts(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, VOLUME_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[56..60].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

// ---------------------------------------------------------------- Socket（TCP 补齐）

/// `socket` 的系统调用号（Linux x86_64 号 41；只认 `AF_INET` + `SOCK_STREAM` + TCP）。
pub const SYS_SOCKET: u64 = abi::syscall::SYS_SOCKET;
/// `connect` 的系统调用号（Linux 号 42）。
pub const SYS_CONNECT: u64 = abi::syscall::SYS_CONNECT;
/// `sendto` 的系统调用号（Linux 号 44）。
pub const SYS_SENDTO: u64 = abi::syscall::SYS_SENDTO;
/// `recvfrom` 的系统调用号（Linux 号 45）。
pub const SYS_RECVFROM: u64 = abi::syscall::SYS_RECVFROM;
/// `bind` 的系统调用号（Linux 号 49）。
pub const SYS_BIND: u64 = abi::syscall::SYS_BIND;
/// 地址族：IPv4（本内核唯一支持的族）。
pub const AF_INET: u16 = abi::sock::AF_INET;
/// 套接字类型：字节流（TCP）。
pub const SOCK_STREAM: i32 = abi::sock::SOCK_STREAM;
/// 协议：TCP（传 0 等价于选它，见内核 `network::socket::create`）。
pub const IPPROTO_TCP: i32 = abi::sock::IPPROTO_TCP;
/// `sockaddr_in` 长度（16 字节；内核要求 `addr_len` 精确等于它）。
pub const SOCKADDR_IN_LEN: usize = abi::sock::SOCKADDR_IN_LEN;
/// `-EAFNOSUPPORT`：不支持的地址族/类型/协议。
pub const EAFNOSUPPORT: isize = abi::errno::EAFNOSUPPORT as isize;
/// `-ENOTCONN`：套接字未连接（`send`/`recv` 之前必须先 `connect`）。
pub const ENOTCONN: isize = abi::errno::ENOTCONN as isize;
/// `-ETIMEDOUT`：TCP 握手/收发超时（重传耗尽）。
pub const ETIMEDOUT: isize = abi::errno::ETIMEDOUT as isize;
/// `-ECONNREFUSED`：对端 RST（端口未监听或连接被拒绝）。
pub const ECONNREFUSED: isize = abi::errno::ECONNREFUSED as isize;

/// `sockaddr_in(ip, port)`：把地址写进 16 字节数组（family LE + port BE + addr + 补 0）。
///
/// 布局的权威定义在 `abi::sock`，这里只是调用侧的构造器 ——
/// 用户程序不该自己拼偏移（字节序错一位就是连不上的 `-ETIMEDOUT`）。
pub fn sockaddr_in(ip: [u8; 4], port: u16) -> [u8; 16] {
    let mut addr = [0u8; SOCKADDR_IN_LEN];
    addr[abi::sock::FAMILY_OFFSET..abi::sock::FAMILY_OFFSET + 2]
        .copy_from_slice(&AF_INET.to_le_bytes());
    addr[abi::sock::PORT_OFFSET..abi::sock::PORT_OFFSET + 2]
        .copy_from_slice(&port.to_be_bytes());
    addr[abi::sock::ADDR_OFFSET..abi::sock::ADDR_OFFSET + 4].copy_from_slice(&ip);
    addr
}

/// `socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)`：分配套接字，返回 fd（≥ 16）或负 errno。
///
/// 套接字 fd 与文件 fd 是**两个号段**（16..=23 vs 0..=15），但 `read`/`write`/`close`
/// 通用：`read` = `recv`（首段即返），`write` = `send`（全发）。
pub fn socket(domain: u16, ty: i32, protocol: i32) -> isize {
    unsafe { raw(SYS_SOCKET, domain as u64, ty as u64, protocol as u64, 0, 0) }
}

/// `connect(fd, addr)`：三次握手（阻塞到建立或失败；失败的 errno 见上）。
///
/// `addr` 用 [`sockaddr_in`] 构造。未分配本地端口时内核从临时段自动分配。
pub fn connect(fd: i32, addr: &[u8; 16]) -> isize {
    unsafe {
        raw(
            SYS_CONNECT,
            fd as u64,
            addr.as_ptr() as u64,
            SOCKADDR_IN_LEN as u64,
            0,
            0,
        )
    }
}

/// `bind(fd, addr)`：固定本地端口（地址只接受全 0 的 bind-any）。
pub fn bind(fd: i32, addr: &[u8; 16]) -> isize {
    unsafe {
        raw(
            SYS_BIND,
            fd as u64,
            addr.as_ptr() as u64,
            SOCKADDR_IN_LEN as u64,
            0,
            0,
        )
    }
}

/// `sendto(fd, buf)`：已连接套接字上全发，返回发送字节数（负数 = errno）。
///
/// 地址参数固定传 `NULL`（内核要求）：已连接字节流不接受逐包目的地址，
/// 传了就是 `-EINVAL`。单次上限与 `write` 一致（`IO_MAX` = 4096）。
pub fn sendto(fd: i32, buf: &[u8]) -> isize {
    unsafe {
        raw(
            SYS_SENDTO,
            fd as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
            0,
            0,
        )
    }
}

/// `recvfrom(fd, buf)`：首段即返，返回收到字节数（`0` = 对端有序关闭；负数 = errno）。
///
/// 同 `sendto`：地址回填未实现，固定传 `NULL`。要收满整段请循环调用。
pub fn recvfrom(fd: i32, buf: &mut [u8]) -> isize {
    unsafe {
        raw(
            SYS_RECVFROM,
            fd as u64,
            buf.as_ptr() as u64,
            buf.len() as u64,
            0,
            0,
        )
    }
}

// ---------------------------------------------------------------- CKM（CKM-1）

/// `ckm_policy_query` 的系统调用号（CKM-1，`docs/ckm.md` 4.3）。
pub const SYS_CKM_POLICY_QUERY: u64 = abi::syscall::SYS_CKM_POLICY_QUERY;
/// `ckm_audit_read` 的系统调用号（CKM-1）。
pub const SYS_CKM_AUDIT_READ: u64 = abi::syscall::SYS_CKM_AUDIT_READ;
/// `ckm_component_state` 的系统调用号（CKM-1）。
pub const SYS_CKM_COMPONENT_STATE: u64 = abi::syscall::SYS_CKM_COMPONENT_STATE;

/// CKM ABI 版本（必须与内核 `ckm::CKM_ABI_VERSION` 相等）。
pub const CKM_ABI_VERSION: u32 = abi::ckm::ABI_VERSION;

/// 审计记录长度（必须与内核 `ckm::audit::AUDIT_RECORD_SIZE` 一致）。
pub const AUDIT_RECORD_SIZE: usize = abi::ckm::AUDIT_RECORD_SIZE;
/// 组件状态记录长度（必须与内核 `ckm::state::COMPONENT_RECORD_SIZE` 一致）。
pub const COMPONENT_RECORD_SIZE: usize = abi::ckm::COMPONENT_RECORD_SIZE;
/// 组件 `id` 字段里可读字符的上限（记录占 16 字节：15 字符 + NUL）。
pub const COMPONENT_ID_MAX: usize = abi::ckm::COMPONENT_ID_MAX;
/// 组件状态表容量（内核 `ckm::STATE_SLOTS`）。
pub const MAX_COMPONENTS: usize = abi::ckm::MAX_COMPONENTS;

/// `-EPERM`：**能力不足**（受保护前缀），与下面的 `EACCES` 刻意区分。
pub const EPERM: isize = abi::errno::EPERM as isize;
/// `-EACCES`：**mode 不允许**（POSIX；文件不属于你）。
pub const EACCES: isize = abi::errno::EACCES as isize;

/// `CAP_CKM_ADMIN`：改 CKM 策略（uid 1000）。
pub const CAP_CKM_ADMIN: u32 = abi::ckm::CAP_CKM_ADMIN;
/// `CAP_CKM_SERVICE`：执行扩展事务（`ckmd`，uid 1001）。
pub const CAP_CKM_SERVICE: u32 = abi::ckm::CAP_CKM_SERVICE;
/// `CAP_FS_SYSTEM_WRITE`：写受保护前缀（`/etc`、`/bin`、`/lib`、`/init`）。
pub const CAP_FS_SYSTEM_WRITE: u32 = abi::ckm::CAP_FS_SYSTEM_WRITE;
/// `CAP_AUDIT_READ`：读审计环形缓冲。
pub const CAP_AUDIT_READ: u32 = abi::ckm::CAP_AUDIT_READ;
/// `CAP_STORAGE_ADMIN`：存储管理的可逆动作（挂载/卸载/盘符/重扫）。
pub const CAP_STORAGE_ADMIN: u32 = abi::ckm::CAP_STORAGE_ADMIN;
/// `CAP_STORAGE_FORMAT`：创建/格式化文件系统（破坏性）。
pub const CAP_STORAGE_FORMAT: u32 = abi::ckm::CAP_STORAGE_FORMAT;

// op / 事件编号（`docs/ckm.md` 4.6；与内核 `ckm::policy::Op` 一致）
pub const CKM_OP_FS_WRITE: u16 = abi::ckm::OP_FS_WRITE;
pub const CKM_OP_STATE_WRITE: u16 = abi::ckm::OP_STATE_WRITE;
pub const CKM_OP_POLICY_SET: u16 = abi::ckm::OP_POLICY_SET;
pub const CKM_OP_AUDIT_READ: u16 = abi::ckm::OP_AUDIT_READ;
pub const CKM_OP_POLICY_QUERY: u16 = abi::ckm::OP_POLICY_QUERY;
pub const CKM_OP_EXEC_COMPONENT: u16 = abi::ckm::OP_EXEC_COMPONENT;
pub const CKM_OP_OVERLAY_REGISTER: u16 = abi::ckm::OP_OVERLAY_REGISTER;
pub const CKM_OP_OVERLAY_CONFLICT: u16 = abi::ckm::OP_OVERLAY_CONFLICT;
pub const CKM_OP_SAFE_MODE_ENTER: u16 = abi::ckm::OP_SAFE_MODE_ENTER;
pub const CKM_OP_TXN_COMMIT: u16 = abi::ckm::OP_TXN_COMMIT;
pub const CKM_OP_TXN_ROLLBACK: u16 = abi::ckm::OP_TXN_ROLLBACK;
pub const CKM_OP_STORAGE_MOUNT: u16 = abi::ckm::OP_STORAGE_MOUNT;
pub const CKM_OP_STORAGE_UNMOUNT: u16 = abi::ckm::OP_STORAGE_UNMOUNT;
pub const CKM_OP_STORAGE_LETTER: u16 = abi::ckm::OP_STORAGE_LETTER;
pub const CKM_OP_STORAGE_FORMAT: u16 = abi::ckm::OP_STORAGE_FORMAT;
pub const CKM_OP_STORAGE_RESCAN: u16 = abi::ckm::OP_STORAGE_RESCAN;

/// 受保护前缀下标（`fs_write` 的 `arg` 语义：它就是内核 `detail` 里的那个下标）。
pub const CKM_PREFIX_ETC: u64 = abi::ckm::PREFIX_ETC;
/// `/bin`
pub const CKM_PREFIX_BIN: u64 = abi::ckm::PREFIX_BIN;
/// `/lib`
pub const CKM_PREFIX_LIB: u64 = abi::ckm::PREFIX_LIB;
/// `/init`
pub const CKM_PREFIX_INIT: u64 = abi::ckm::PREFIX_INIT;

/// op 短名（与内核串口证据同一批字串；日志与 CLI 显示用）。
pub fn ckm_op_name(op: u16) -> &'static str {
    match op {
        CKM_OP_FS_WRITE => "fs_write",
        CKM_OP_STATE_WRITE => "state_write",
        CKM_OP_POLICY_SET => "policy_set",
        CKM_OP_AUDIT_READ => "audit_read",
        CKM_OP_POLICY_QUERY => "policy_query",
        CKM_OP_EXEC_COMPONENT => "exec_component",
        CKM_OP_OVERLAY_REGISTER => "overlay_register",
        CKM_OP_OVERLAY_CONFLICT => "overlay_conflict",
        CKM_OP_SAFE_MODE_ENTER => "safe_mode_enter",
        CKM_OP_TXN_COMMIT => "txn_commit",
        CKM_OP_TXN_ROLLBACK => "txn_rollback",
        CKM_OP_STORAGE_MOUNT => "storage_mount",
        CKM_OP_STORAGE_UNMOUNT => "storage_unmount",
        CKM_OP_STORAGE_LETTER => "storage_letter",
        CKM_OP_STORAGE_FORMAT => "storage_format",
        CKM_OP_STORAGE_RESCAN => "storage_rescan",
        _ => "unknown",
    }
}

/// 一个 op 需要的 capability 位（用户态镜像；显示与自检用，**不是**权威来源）。
pub fn ckm_op_required_caps(op: u16) -> u32 {
    match op {
        CKM_OP_FS_WRITE => CAP_FS_SYSTEM_WRITE,
        CKM_OP_STATE_WRITE | CKM_OP_OVERLAY_REGISTER | CKM_OP_OVERLAY_CONFLICT
        | CKM_OP_TXN_COMMIT | CKM_OP_TXN_ROLLBACK => CAP_CKM_SERVICE,
        CKM_OP_POLICY_SET | CKM_OP_SAFE_MODE_ENTER => CAP_CKM_ADMIN,
        CKM_OP_AUDIT_READ => CAP_AUDIT_READ,
        CKM_OP_STORAGE_MOUNT | CKM_OP_STORAGE_UNMOUNT | CKM_OP_STORAGE_LETTER
        | CKM_OP_STORAGE_RESCAN => CAP_STORAGE_ADMIN,
        CKM_OP_STORAGE_FORMAT => CAP_STORAGE_FORMAT,
        _ => 0,
    }
}

/// `ckm_policy_query(op, arg, out)`：查当前进程对 `op` 的判定，返回 `0` 或 `-EPERM`。
///
/// 同时把一条 [`AUDIT_RECORD_SIZE`] 字节记录写进 `out`（**复用审计记录布局**，
/// `seq = 0` 表示它不是事件；解析器与 [`ckm_audit_read`] 共用）。
/// 没有参数能指定「查别的进程」—— 这个调用只能问自己。
///
/// `out` 至少要有 [`AUDIT_RECORD_SIZE`] 字节，否则内核返回 `-EINVAL`。
pub fn ckm_policy_query(op: u16, arg: u64, out: &mut [u8]) -> isize {
    unsafe {
        raw(
            SYS_CKM_POLICY_QUERY,
            op as u64,
            arg,
            out.as_ptr() as u64,
            out.len() as u64,
            0,
        )
    }
}

/// `ckm_audit_read(buf, since_seq, out_count)`：读审计环里 `seq > since_seq` 的记录。
///
/// 需要 `CAP_AUDIT_READ`（否则返回 `-EPERM` 且**不写缓冲**）；返回实际写入条数，
/// `out_count` 回填环内当前条数。缓冲至少 [`AUDIT_RECORD_SIZE`] 字节。
pub fn ckm_audit_read(buf: &mut [u8], since_seq: u64, out_count: Option<&mut u64>) -> isize {
    let count_ptr = match out_count {
        Some(slot) => slot as *mut u64 as u64,
        None => 0,
    };
    unsafe {
        raw(
            SYS_CKM_AUDIT_READ,
            buf.as_ptr() as u64,
            buf.len() as u64,
            since_seq,
            count_ptr,
            0,
        )
    }
}

/// `ckm_component_state(buf, out_count)`：按确定性顺序导出组件状态记录，返回条数。
///
/// 只读、无能力要求；缓冲装不下整张表返回 `-ENOSPC`（**要么写完整记录，要么明确失败**）。
/// `out_count` 回填条数。
pub fn ckm_component_state(buf: &mut [u8], out_count: Option<&mut u64>) -> isize {
    let count_ptr = match out_count {
        Some(slot) => slot as *mut u64 as u64,
        None => 0,
    };
    unsafe {
        raw(
            SYS_CKM_COMPONENT_STATE,
            buf.as_ptr() as u64,
            buf.len() as u64,
            count_ptr,
            0,
            0,
        )
    }
}

// ---- 记录解析器（内核是权威定义，这里不自己算偏移）----

/// 第 `index` 条审计记录的 `seq`。
pub fn ckm_record_seq(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u64::from_le_bytes(rec[0..8].try_into().unwrap_or([0; 8])),
        None => 0,
    }
}

/// 第 `index` 条审计记录的 `pid`。
pub fn ckm_record_pid(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u64::from_le_bytes(rec[8..16].try_into().unwrap_or([0; 8])),
        None => 0,
    }
}

/// 第 `index` 条审计记录的 `uid`。
pub fn ckm_record_uid(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[16..20].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// 第 `index` 条审计记录的 `gid`。
pub fn ckm_record_gid(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[20..24].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// 第 `index` 条审计记录的 `op` 编号（短名见 [`ckm_op_name`]）。
pub fn ckm_record_op(buf: &[u8], index: usize) -> u16 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u16::from_le_bytes([rec[24], rec[25]]),
        None => 0,
    }
}

/// 第 `index` 条记录是否是**拒绝**（`result == 1`）。
pub fn ckm_record_is_deny(buf: &[u8], index: usize) -> bool {
    record(buf, index, AUDIT_RECORD_SIZE).map_or(false, |rec| rec[26] == 1)
}

/// 第 `index` 条审计记录 `deny` 时需要的 capability 位（`allow` 时为 0）。
pub fn ckm_record_need(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[28..32].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// 第 `index` 条记录的 `detail`（op 相关：前缀下标 / 组件槽位 / errno）。
pub fn ckm_record_detail(buf: &[u8], index: usize) -> u64 {
    match record(buf, index, AUDIT_RECORD_SIZE) {
        Some(rec) => u64::from_le_bytes(rec[32..40].try_into().unwrap_or([0; 8])),
        None => 0,
    }
}

/// 第 `index` 条组件状态记录的 `id`（NUL 之前的字节）。
pub fn component_id(buf: &[u8], index: usize) -> &str {
    let Some(rec) = record(buf, index, COMPONENT_RECORD_SIZE) else {
        return "";
    };
    match rec.get(..COMPONENT_ID_MAX + 1) {
        Some(bytes) => {
            let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            core::str::from_utf8(&bytes[..len]).unwrap_or("?")
        }
        None => "",
    }
}

/// 第 `index` 条组件记录的 `kind`（`1` 用户态 / `2` 系统组件 / `3` 覆盖层 / `4` 内核模块）。
pub fn component_kind(buf: &[u8], index: usize) -> u16 {
    match record(buf, index, COMPONENT_RECORD_SIZE) {
        Some(rec) => u16::from_le_bytes([rec[16], rec[17]]),
        None => 0,
    }
}

/// `kind` 的短名（与内核 `ckm::state::Kind` 同一套语义）。
pub fn component_kind_name(kind: u16) -> &'static str {
    match kind {
        1 => "userland",
        2 => "system-component",
        3 => "system-override",
        4 => "kernel-module",
        _ => "?",
    }
}

/// 第 `index` 条组件记录的 `state` 编号。
pub fn component_state(buf: &[u8], index: usize) -> u8 {
    record(buf, index, COMPONENT_RECORD_SIZE).map_or(0, |rec| rec[18])
}

/// `state` 的短名（`docs/ckm.md` 5.3 的状态机）。
pub fn component_state_name(state: u8) -> &'static str {
    match state {
        0 => "absent",
        1 => "staged",
        2 => "installed",
        3 => "active",
        4 => "disabled",
        5 => "failed",
        6 => "removed-pending",
        _ => "?",
    }
}

/// 第 `index` 条组件记录的整数版本号。
pub fn component_version_code(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, COMPONENT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[20..24].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// 第 `index` 条组件记录声明的 CKM ABI 版本。
pub fn component_abi_kernel(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, COMPONENT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[24..28].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// 第 `index` 条组件记录的覆盖层优先级（非覆盖层恒 0）。
pub fn component_priority(buf: &[u8], index: usize) -> u32 {
    match record(buf, index, COMPONENT_RECORD_SIZE) {
        Some(rec) => u32::from_le_bytes(rec[28..32].try_into().unwrap_or([0; 4])),
        None => 0,
    }
}

/// `exit(code)`：结束当前进程并把退出码交回内核。
///
/// 顶层程序（`/init`、`/bin/sh`）结束后控制权回到内核运行驱动；
/// 子进程结束后回到父进程的 [`wait4`]。
pub fn exit(code: i32) -> ! {
    unsafe {
        raw(SYS_EXIT, code as u64, 0, 0, 0, 0);
    }
    // 内核的 exit 路径不会回到用户态（它恢复父进程/驱动方的内核现场）；
    // 真回到这里说明 ABI 被破坏了，用 `ud2` 立刻显形而不是继续跑。
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}
