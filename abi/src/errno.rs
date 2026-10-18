//! `-errno` 约定（编号沿用 Linux x86_64 的取值）。
//!
//! 系统调用的返回值**非负即正常值**，**负数即 `-errno`**
//! （`errno = -(返回值)`，不是「返回 errno 的负值」之外的任何约定：
//! `rax = -14` 表示 `EFAULT`）。规范类型是 `i64`：寄存器是 64 位，
//! 内核的 handler 返回 `i64`，libc 侧把它按 `isize` 使用（同宽，显式转换一次）。
//!
//! 只列**内核真的会返回**的 errno —— 不预先声明一批用不上的号，
//! 否则「哪个错误可能发生」就没法从定义里读出来。

/// `-EPERM`：**能力不足**（CKM 判定拒绝，例如写受保护前缀）。
///
/// 与 [`EACCES`] 刻意区分：`EPERM` = 「你的能力不够」，`EACCES` = 「文件的 mode 不允许」。
pub const EPERM: i64 = -1;
/// `-ENOENT`：路径不存在。
pub const ENOENT: i64 = -2;
/// `-EINTR`：等待被中断（`nanosleep` 检测到计时器停摆时的兜底）。
pub const EINTR: i64 = -4;
/// `-EIO`：设备读写/扫描失败（存储类调用）。
pub const EIO: i64 = -5;
/// `-E2BIG`：`argv` 总量超过用户栈预留区（`execve`）。
pub const E2BIG: i64 = -7;
/// `-ENOEXEC`：可执行文件格式非法（ELF 装载器拒绝）。
pub const ENOEXEC: i64 = -8;
/// `-EBADF`：fd 无效 / 未打开 / 打开模式与操作不符。
pub const EBADF: i64 = -9;
/// `-ECHILD`：`wait4` 找不到该 PID 的直接子进程。
pub const ECHILD: i64 = -10;
/// `-EAGAIN`：本次没有数据可用（非阻塞语义，不是错误：`read(0)` 无输入时会返回它）。
pub const EAGAIN: i64 = -11;
/// `-ENOMEM`：内核内存不足。
pub const ENOMEM: i64 = -12;
/// `-EACCES`：mode 不允许（POSIX）。
pub const EACCES: i64 = -13;
/// `-EEXIST`：目标已存在（`O_CREAT` 之外的创建路径）。
pub const EEXIST: i64 = -17;
/// `-ENODEV`：设备不存在 / 已消失（与「坏盘」可区分）。
pub const ENODEV: i64 = -19;
/// `-ENOTDIR`：路径中间分量不是目录。
pub const ENOTDIR: i64 = -20;
/// `-EFAULT`：用户传了不属于自己的地址区间。
pub const EFAULT: i64 = -14;
/// `-EISDIR`：目标是目录，但调用方要当文件用。
pub const EISDIR: i64 = -21;
/// `-EINVAL`：参数非法（未知 `flags` 位、越界长度、缓冲太小等）。
pub const EINVAL: i64 = -22;
/// `-ENOSPC`：空间不足（fd 用尽 / 超过 RAMFS 上限）。
pub const ENOSPC: i64 = -28;
/// `-EROFS`：只读文件系统（写落到只读后端）。
pub const EROFS: i64 = -30;
/// `-ENOSYS`：号合法但内核未实现（或号越界时的兜底返回）。
pub const ENOSYS: i64 = -38;
/// `-ENOTSUP`：识别到文件系统/特性，但内核没有对应实现（与「路径不存在」区分）。
pub const ENOTSUP: i64 = -95;
/// `-EAFNOSUPPORT`：不支持的地址族/类型/协议（socket 只认 `AF_INET` + `SOCK_STREAM` + TCP）。
pub const EAFNOSUPPORT: i64 = -97;
/// `-EUCLEAN`：磁盘上的结构损坏（簇链断/越界、目录项非法）。
pub const EUCLEAN: i64 = -117;
/// `-ENOTCONN`：套接字未连接（`send`/`recv` 之前必须先 `connect`）。
pub const ENOTCONN: i64 = -107;
/// `-ETIMEDOUT`：TCP 握手/收发超时（重传耗尽，对端无应答）。
pub const ETIMEDOUT: i64 = -110;
/// `-ECONNREFUSED`：对端 RST（端口未监听或连接被拒绝）。
pub const ECONNREFUSED: i64 = -111;
