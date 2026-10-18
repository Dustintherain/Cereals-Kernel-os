//! libc —— Cerlesse 最小用户态运行时（v0.6 脚手架，v0.7 文件 syscall，v0.9 用户堆）
//!
//! 这一层把「内核 ABI」封装成用户程序可以直接调用的形状，让新用户程序
//! **不需要**再手写机器码：
//!
//! - [`syscall`]：`syscall` 指令封装 + 调用（`write` / `read` / `open` / `close` /
//!   `getpid` / `nanosleep` / `exit` / `brk` / `mmap` / `munmap`）；
//!   v1.0 第一步补上 `mprotect`（改写已映射页的保护属性，支持 `PROT_NONE`）——
//!   W^X 的「先 `mmap(RW)` 写、再 `mprotect(RX)` 执行」因此成立；
//!   v0.9 第三步起 `fork` / `execve` / `wait4` 也不再是桩：它们有真实的进程对象
//!   （独立地址空间 + 独立内核栈 + 完整用户帧）作为后端，`user/shell` 用它跑外部程序；
//!   v0.9 第五步补上用户态读内核表的两个调用：`getdents`（目录项，`user/ls`）与
//!   `getprocs`（进程表快照，`user/ps`）—— 两者都是**定长记录**，配套的
//!   `dirent_*` / `procinfo_*` 解析器也在这一层（用户程序不该自己算偏移）；
//! - [`args`]：`argc` / `argv`（由 `execve` 与内核运行驱动一起送入 `_start`）；
//! - [`heap`]：`#[global_allocator]`（bump over `mmap`）—— 用户程序因此可以用
//!   `Vec`/`String`/`Box`（在此之前只能把数据放在栈或静态数组里）；
//! - [`console`]：`print!` / `println!`（固定缓冲，一次 `write` 送出一整行）；
//! - `_start`（本文件底部 `global_asm!`）：`_start` → 用户 `main` → `exit(main())`；
//! - `#[panic_handler]`：用户态 panic 打到 stderr 后 `exit(101)`，不会卡死在内核里。
//!
//! 用户程序的约定（`user/*`）：
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! use libc::{getpid, println};
//!
//! #[no_mangle]
//! pub extern "C" fn main() -> i32 {
//!     println!("pid={}", getpid());
//!     0 // 退出码
//! }
//! ```
//!
//! 三条硬约束（都有实测理由，改错会在链接期或 ring3 里立刻炸）：
//!
//! 1. **每个段必须页对齐**（`user.ld` 里的 `ALIGN(0x1000)`）。v0.7 的 ELF Loader
//!    按页设置权限（`R` → `NX`、`W` → 可写、`X` → 可执行），同页既 `W` 又 `X`
//!    会被装载器直接拒绝（`user: ELF rejected: W and X segments share a page`）；
//! 2. **必须固定链接到 `USER_VIRT_BASE`（0x0000_0080_0000_0000）**：链接地址 = 运行地址，
//!    程序里的绝对地址因此不需要重定位（内核不做加载时重定位，只接受 `ET_EXEC`）；
//! 3. **程序有可写数据段**（`.data`/`.bss` 映射为 ring3 可写、不可执行）
//!    与**用户堆**（`brk` 窗口 256 KiB + 匿名 `mmap` 窗口 256 KiB，见 [`heap`]）；
//!    `mmap`/`mprotect` 都拒绝 `PROT_WRITE|PROT_EXEC`（W^X），且 `mmap` 没有 `offset` 参数。
//!
//! 系统调用号与错误约定见 `kernel/src/syscall/syscall_table.rs`（Linux x86_64 语义）。

#![no_std]

pub mod args;
pub mod console;
pub mod heap;
pub mod path;
pub mod syscall;

pub use crate::args::{arg, arg_str, argc, Args};
pub use crate::console::Writer;
pub use crate::syscall::{
    bind, brk, ckm_audit_read, ckm_component_state, ckm_op_name, ckm_op_required_caps,
    ckm_policy_query, ckm_record_detail, ckm_record_gid, ckm_record_is_deny, ckm_record_need,
    ckm_record_op, ckm_record_pid, ckm_record_seq, ckm_record_uid, close, component_abi_kernel,
    component_id, component_kind, component_kind_name, component_priority, component_state,
    component_state_name, component_version_code, connect, dirent_is_dir, dirent_name, dirent_size,
    execve, exit, fork, getdents, getpid, getprocs, mmap, mprotect, munmap, nanosleep, open,
    procinfo_name, procinfo_pid, procinfo_ppid, procinfo_state, read, recvfrom, sendto, socket,
    sockaddr_in, storage_assign_letter, storage_mount, storage_rescan, storage_unmount,
    storage_volumes, volume_disk, volume_free_bytes, volume_fs, volume_health, volume_id,
    volume_label, volume_label_truncated, volume_letter, volume_letter_str, volume_mounts,
    volume_part, volume_read_only, volume_total_bytes, wait4, write, AF_INET, ARG_MAX,
    AUDIT_RECORD_SIZE, CAP_AUDIT_READ, CAP_CKM_ADMIN, CAP_CKM_SERVICE, CAP_FS_SYSTEM_WRITE,
    CAP_STORAGE_ADMIN, CAP_STORAGE_FORMAT, CKM_ABI_VERSION, CKM_OP_AUDIT_READ, CKM_OP_EXEC_COMPONENT,
    CKM_OP_FS_WRITE, CKM_OP_OVERLAY_CONFLICT, CKM_OP_OVERLAY_REGISTER, CKM_OP_POLICY_QUERY,
    CKM_OP_POLICY_SET, CKM_OP_SAFE_MODE_ENTER, CKM_OP_STATE_WRITE, CKM_OP_STORAGE_FORMAT,
    CKM_OP_STORAGE_LETTER, CKM_OP_STORAGE_MOUNT, CKM_OP_STORAGE_RESCAN, CKM_OP_STORAGE_UNMOUNT,
    CKM_OP_TXN_COMMIT, CKM_OP_TXN_ROLLBACK, CKM_PREFIX_BIN, CKM_PREFIX_ETC, CKM_PREFIX_INIT,
    CKM_PREFIX_LIB, COMPONENT_ID_MAX, COMPONENT_RECORD_SIZE, DIRENT_NAME_MAX, DIRENT_SIZE, EACCES,
    EAFNOSUPPORT, EAGAIN, EBADF, ECHILD, ECONNREFUSED, EFAULT, EISDIR, EINVAL, ENOENT, ENOMEM,
    ENOSPC, ENOSYS, ENOTCONN, EPERM, ETIMEDOUT, IPPROTO_TCP, MAP_ANONYMOUS, MAP_PRIVATE,
    MAX_COMPONENTS, O_CREAT, O_RDONLY, O_RDWR, O_WRONLY, PROCINFO_NAME_MAX, PROCINFO_SIZE,
    PROT_EXEC, PROT_NONE, PROT_READ, PROT_WRITE, SOCKADDR_IN_LEN, SOCK_STREAM, STDERR, STDIN,
    STDOUT, STORAGE_LETTER_NONE, STORAGE_MOUNT_RDONLY, STORAGE_RESCAN_ALL, VOLUME_FREE_UNKNOWN,
    VOLUME_LABEL_MAX, VOLUME_RECORD_SIZE,
};

// CSS-3：Windows 式路径（`C:\x`）→ POSIX 路径的重写。用户程序用 `libc::path::resolve`。
pub use crate::path::{is_windows, resolve as resolve_windows_path, resolve_str};

/// 卷表缓冲的默认容量（内核 `storage::MAX_VOLUMES` = 16 条）。
pub const MAX_VOLUMES: usize = 16;

use core::arch::global_asm;

// 用户程序入口。放在独立段 `.text.start` 并由 `user.ld` 排在该段最前面，
// 使入口落在映像第 0 页（ELF 头里的 `e_entry` 由装载器直接使用）。
//
// `main` 由用户程序提供（`#[no_mangle] pub extern "C" fn main() -> i32`），
// 返回值即退出码，交给 `exit` 系统调用。
//
// **入口寄存器 ABI**：内核把 `argc` 放在 `rdi`、`argv` 放在 `rsi`（见
// `kernel/src/syscall/process.rs::build_user_frame` 与 `docs/syscall.md`）。
// `_start` 原样把它们传给 [`cer_start`]（Rust 侧），由它存进 [`args`] 模块
// 再调用用户 `main` —— 用户程序不必关心这段汇编。
//
// 注意：`global_asm!` 是宏调用，不能挂文档注释（会被判为 unused doc comment）。
global_asm!(
    ".section .text.start,\"ax\"",
    ".global _start",
    ".type _start, @function",
    "_start:",
    "xor ebp, ebp",              // 栈回溯终止
    "and rsp, -16",              // SysV：call 前保持 16 字节对齐（栈顶本就对齐，这里是防御）
    "call cer_start",            // rdi = argc，rsi = argv（内核已放好）
    "ud2",                       // cer_start 不返回；到达这里说明入口 ABI 被破坏
    ".size _start, . - _start",
);

/// 用户程序的总入口：记录 `argc`/`argv` → 调用用户 `main` → `exit`。不返回。
#[no_mangle]
pub extern "C" fn cer_start(argc: u64, argv: u64) -> ! {
    args::set(argc, argv);
    // SAFETY: `main` 由用户程序提供，链接期保证存在（缺失就是未定义符号）。
    let code = unsafe { main() };
    exit(code)
}

// 只被 `cer_start` 引用，Rust 侧看不到外部定义的使用点由这一行表达。
extern "C" {
    /// 由用户程序提供的 C ABI 入口，返回值作为退出码。
    fn main() -> i32;
}

/// 用户态 panic：打到 stderr 后以 101 退出。
///
/// 放在 libc 里是为了让每个用户程序不必各自实现一遍；
/// 因此用户程序**不要**再定义 `#[panic_handler]`（会产生重复符号）。
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write as _;

    struct Stderr;

    impl core::fmt::Write for Stderr {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let _ = write(STDERR, s.as_bytes());
            Ok(())
        }
    }

    let _ = core::write!(Stderr, "user panic: {}\n", info);
    exit(101)
}
