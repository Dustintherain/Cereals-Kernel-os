//! 系统调用号到内核处理函数的表（v0.6）
//!
//! **编号、号段规则与「已命名未实现」清单都不在本文件**：它们是 ABI，
//! 单一事实源在 `abi::syscall`（`docs/pure-kernel.md` 阶段三，C6）。
//! 本文件只做三件事：
//!
//! 1. **再导出**编号（`pub use abi::syscall::*`），保证内核其它模块与 libc
//!    读的是同一批数字；
//! 2. 定义 handler 类型 [`SyscallHandler`] 与查表 [`parse`]；
//! 3. 排满整张 [`SYS_TABLE`]（`0..SYS_MAX`）：未单独实现的号落在公共 ENOSYS 桩上，
//!    因此「号合法但未实现」与「号越界」是两种可区分的失败
//!    （前者在 `abi::syscall::UNIMPLEMENTED` 里有名字，后者的 `parse` 返回 `None`）。
//!
//! 号段（`abi::syscall` 是权威定义）：`0..=255` 沿用 Linux x86_64 语义
//! （`read=0 … wait4=61`、`getdents=78`），`>= 256` 是 Cerlesse 扩展段 ——
//! `getprocs=256`、CKM `257..=260`、存储 `261..=266`，`SYS_MAX=267`（不含）。

// 编号表本身就是对外契约：尚未实现的号也必须有名字，否则调用方只能靠魔法数。
#![allow(dead_code)]

use crate::syscall::address_space::UserSpace;

/// 系统调用号与号段规则（**再导出**，定义在 `abi::syscall`）。
///
/// 内核侧引用路径保持 `crate::syscall::syscall_table::SYS_*` 不变 ——
/// 换的只是「数字写在哪」，不是「谁在用」。
///
/// `#[allow(unused_imports)]`：**全部编号都是对外契约**（包括「已命名未实现」的
/// 260/266 与号段边界常量），即使本文件不直接引用也要留在这里 ——
/// 与模块级 `#![allow(dead_code)]` 是同一个理由，不是「消警告」。
#[allow(unused_imports)]
pub use abi::syscall::{
    is_cerlesse_segment, is_linux_segment, unimplemented_name, CERLESSE_SEGMENT_START,
    LINUX_SEGMENT_END, SYS_BIND, SYS_BRK, SYS_CKM_AUDIT_READ, SYS_CKM_COMPONENT_STATE,
    SYS_CKM_POLICY_QUERY, SYS_CKM_POLICY_SET, SYS_CLOSE, SYS_CONNECT, SYS_EXEC, SYS_EXIT,
    SYS_FORK, SYS_GETDENTS, SYS_GETPID, SYS_GETPROCS, SYS_MMAP, SYS_MPROTECT, SYS_MUNMAP,
    SYS_OPEN, SYS_READ, SYS_RECVFROM, SYS_SENDTO, SYS_SLEEP, SYS_SOCKET,
    SYS_STORAGE_ASSIGN_LETTER, SYS_STORAGE_FORMAT, SYS_STORAGE_MOUNT, SYS_STORAGE_RESCAN,
    SYS_STORAGE_UNMOUNT, SYS_STORAGE_VOLUMES, SYS_WAIT, SYS_WRITE, SYS_MAX,
};

/// 系统调用处理函数类型。`args[0..5]` = `rdi, rsi, rdx, r10, r8`。
pub type SyscallHandler = unsafe fn(args: &[u64; 5], space: *const UserSpace) -> i64;

/// 按系统调用号查表：号越界返回 `None`，否则返回槽位中的 handler
/// （未实现的号命中 [`syscall_enosys`]）。
pub fn parse(nid: u64) -> Option<SyscallHandler> {
    if nid >= SYS_MAX {
        return None;
    }
    Some(SYS_TABLE[nid as usize])
}

/// 公共 ENOSYS 桩（`-ENOSYS`，见 `abi::errno`）。
pub(crate) fn syscall_enosys(_args: &[u64; 5], _space: *const UserSpace) -> i64 {
    abi::errno::ENOSYS
}

/// 系统调用表。已注册的号在 `const` 求值期替换为具体 handler。
pub const SYS_TABLE: [SyscallHandler; SYS_MAX as usize] = {
    let mut table: [SyscallHandler; SYS_MAX as usize] = [syscall_enosys; SYS_MAX as usize];
    table[SYS_READ as usize] = crate::syscall::file::sys_read;
    table[SYS_WRITE as usize] = crate::syscall::file::sys_write;
    table[SYS_OPEN as usize] = crate::syscall::file::sys_open;
    table[SYS_CLOSE as usize] = crate::syscall::file::sys_close;
    table[SYS_MMAP as usize] = crate::syscall::memory::sys_mmap;
    // v1.0 第一步：mprotect 不再是 ENOSYS 桩（W^X 的「先写后换只执行」）
    table[SYS_MPROTECT as usize] = crate::syscall::memory::sys_mprotect;
    table[SYS_MUNMAP as usize] = crate::syscall::memory::sys_munmap;
    table[SYS_BRK as usize] = crate::syscall::memory::sys_brk;
    table[SYS_GETPID as usize] = crate::syscall::process::sys_getpid;
    table[SYS_SLEEP as usize] = crate::syscall::process::sys_sleep;
    table[SYS_EXIT as usize] = crate::syscall::process::sys_exit;
    // v0.9 第三步：进程对象落地后 fork/execve/wait4 不再是 ENOSYS 桩
    table[SYS_FORK as usize] = crate::syscall::process::sys_fork;
    table[SYS_EXEC as usize] = crate::syscall::process::sys_exec;
    table[SYS_WAIT as usize] = crate::syscall::process::sys_wait;
    // v0.9 第五步：用户态可枚举目录（`ls`）与进程表（`ps`）
    // TCP/socket 补齐：AF_INET/SOCK_STREAM 套接字（后端见 network::socket）
    table[SYS_SOCKET as usize] = crate::syscall::net::sys_socket;
    table[SYS_BIND as usize] = crate::syscall::net::sys_bind;
    table[SYS_CONNECT as usize] = crate::syscall::net::sys_connect;
    table[SYS_SENDTO as usize] = crate::syscall::net::sys_sendto;
    table[SYS_RECVFROM as usize] = crate::syscall::net::sys_recvfrom;
    table[SYS_GETDENTS as usize] = crate::syscall::dir::sys_getdents;
    table[SYS_GETPROCS as usize] = crate::syscall::process::sys_getprocs;
    // CKM-1：策略查询 / 审计读取 / 组件状态（260 落 UNIMPLEMENTED 桩）
    table[SYS_CKM_POLICY_QUERY as usize] = crate::syscall::ckm::sys_ckm_policy_query;
    table[SYS_CKM_AUDIT_READ as usize] = crate::syscall::ckm::sys_ckm_audit_read;
    table[SYS_CKM_COMPONENT_STATE as usize] = crate::syscall::ckm::sys_ckm_component_state;
    // CSS-1（H-08）：卷表快照与重扫
    table[SYS_STORAGE_VOLUMES as usize] = crate::syscall::storage::sys_storage_volumes;
    table[SYS_STORAGE_RESCAN as usize] = crate::syscall::storage::sys_storage_rescan;
    // CSS-3：按卷身份挂载 / 卸载 / 盘符（266 格式化仍落 UNIMPLEMENTED 桩）
    table[SYS_STORAGE_MOUNT as usize] = crate::syscall::storage::sys_storage_mount;
    table[SYS_STORAGE_UNMOUNT as usize] = crate::syscall::storage::sys_storage_unmount;
    table[SYS_STORAGE_ASSIGN_LETTER as usize] = crate::syscall::storage::sys_storage_assign_letter;
    table
};
