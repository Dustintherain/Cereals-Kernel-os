//! 系统调用 / 用户态（v0.6 实现）
//!
//! 组成：
//! - [`trampoline`]：`syscall` 入口、进入/离开 ring3 的汇编；
//! - [`address_space`]：用户页表（独立 PML4 + 按段权限逐页映射 + 内核 USER 位清理）；
//! - [`elf`]：ELF64 装载器（RAMFS 上的 `/init` → 用户地址空间）；
//! - [`syscall_table`]：编号 → handler 的分发表；
//! - [`process`] / [`file`] / [`memory`] / [`dir`] / [`net`]：已实现的 syscall
//!   （exit/getpid/sleep/getprocs、read/write/open/close、mmap/munmap/brk、getdents、
//!   socket/bind/connect/sendto/recvfrom）；
//! - [`ckm`]：CKM 管理接口（CKM-1）—— `ckm_policy_query`(257) / `ckm_audit_read`(258)
//!   / `ckm_component_state`(259)，判定点落点见 `docs/ckm.md` 3.4；
//! - [`user`]：用户程序与 ring3 运行驱动 —— `/init`（`user/hello` 的 ELF，从 RAMFS 装载）
//!   与隔离探针（内嵌机器码）。
//!
//! 约定（Linux x86_64 风格，固化为 v0.6 的入口选型）：
//! - 入口指令：**`syscall` / `sysret`**（ADR：v0.6 采用快速路径，非 `int 0x80`）。
//!   `syscall` 把 RIP 存 `rcx`、RFLAGS 存 `r11`，**不切换栈**，故入口汇编
//!   自行切到专用内核栈（见 `trampoline`）。
//! - 编号：`rax`；参数：`rdi, rsi, rdx, r10, r8`；返回值：`rax`。
//! - 错误返回：负数表示 `-errno`（如 `-ENOSYS = -38`），非负数为正常返回值。

pub mod address_space;
pub mod ckm;
pub mod dir;
pub mod dynamic;
pub mod elf;
pub mod file;
pub mod memory;
pub mod net;
pub mod process;
pub mod storage;
pub mod syscall_table;
pub mod trampoline;
pub mod user;

use crate::driver::serial;

pub use crate::syscall::address_space::UserSpace;

/// 未实现系统调用的统一返回码（`-ENOSYS`）—— **值来自 ABI 单一事实源**
/// （`abi::errno`，见 `docs/pure-kernel.md` 阶段三）。
pub use abi::errno::ENOSYS;

/// `syscall` 指令的 Rust 分发入口（由 `trampoline::syscall_entry` 调用）。
///
/// # Safety
///
/// - `args` 必须指向 5 个 `u64`（`rdi, rsi, rdx, r10, r8`），且 **8 字节对齐**；
///   参数帧由入口放在对齐过的内核栈顶下方（见 `process::kstack_top` 的对齐说明），
///   不对齐时下面的 `&*(args as *const [u64; 5])` 会触发 debug 构建的 UB 检查；
/// - `space` 必须是当前正在运行的用户地址空间（或空指针，表示内核自身发起）。
#[no_mangle]
pub unsafe extern "C" fn syscall_dispatch(
    nid: u64,
    args: *const u64,
    space: *const UserSpace,
) -> u64 {
    let args_ref: &[u64; 5] = unsafe { &*(args as *const [u64; 5]) };
    // 已命名但未实现的号：留下「谁在依赖它」的记录。
    // 这是一条**依赖关系证据**，不是诊断噪声：调用未实现的号时串口上会出现
    // `syscall: ENOSYS nr=57 name=fork`（v0.9 第三步之前），而不是只靠一句
    // 「还没实现」的说明（见 docs/syscall.md 3.4）。
    // CSS-1 起 `UNIMPLEMENTED` 里重新有号（262/263/264/266 的存储类调用）：
    // 命中时串口会打出 `syscall: ENOSYS nr=262 name=storage_mount`，
    // 与「号越界」区分开。
    let names = syscall_table::unimplemented_name(nid);
    if let Some(name) = names {
        serial::print("syscall: ENOSYS nr=");
        serial::print_dec(nid);
        serial::print(" name=");
        serial::println(name);
    }
    let parsed = syscall_table::parse(nid);
    match parsed {
        Some(handler) => {
            let ret = unsafe { handler(args_ref, space) };
            ret as u64 // 负数按二进制补码返回，用户态自行解释
        }
        None => ENOSYS as u64,
    }
}

/// 初始化 syscall/用户态所需的处理器状态（幂等，进入用户态前调用一次）。
pub fn init() {
    use crate::arch::x86_64::{cpu, gdt};

    // 1. EFER：使能 syscall/sysret 与 NX
    cpu::enable_syscall_extensions();
    cpu::enable_nx();

    // 2. CR4：清 SMEP/SMAP（v0.6 的 ring0 需要直接读写用户页）
    cpu::clear_cr4_smep_smap();

    // 3. STAR/LSTAR/FMASK
    cpu::set_star(gdt::KERNEL_CODE as u64, gdt::SYSRET_USER_BASE);
    cpu::set_lstar(trampoline::syscall_entry_addr());
    // 进入内核时清 IF/DF/TF：syscall handler 在关中断下运行，避免在用户栈
    // 尚未完全脱离时被中断打断
    cpu::set_fmask(0x200 | 0x400 | 0x100);

    // 4. 进程表（每进程一条 64 KiB 内核栈：syscall 与 ring3 中断/异常共用同一栈顶）
    crate::process::process::init();

    crate::driver::serial::println("syscall: syscall/sysret entry armed (EFER.SCE, STAR/LSTAR/FMASK)");
}
