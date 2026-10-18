//! Socket 类系统调用（TCP 补齐：`socket` / `bind` / `connect` / `sendto` / `recvfrom`）
//!
//! 号沿用 Linux x86_64（`socket=41`、`connect=42`、`sendto=44`、`recvfrom=45`、
//! `bind=49`，权威定义在 `abi::syscall`），地址用 16 字节 `sockaddr_in`
//! （布局见 `abi::sock`）。后端是 [`crate::network::socket`] 的套接字表，
//! fd 落在独立号段（[`crate::network::socket::SOCK_FD_BASE`] 起，不与文件 fd 重叠）。
//!
//! 三条贯穿所有 handler 的规则（与 `syscall::file` 一致）：
//!
//! 1. **用户指针先校验再用**（`sockaddr` 要求可读，`recv` 缓冲要求可写），
//!    否则 `-EFAULT` 并留痕；
//! 2. **单次 I/O 有上限**（[`IO_MAX`]，与 `read`/`write` 相同）；
//! 3. **参数形状收紧**：`addr_len != 16`、`port == 0`、
//!    `sendto`/`recvfrom` 带地址（`addr_ptr != 0`）一律 `-EINVAL` ——
//!    本内核只做「已连接字节流」这一个子集，不静默接受不支持的用法。
//!
//! `read`/`write`/`close` 对套接字 fd 同样有效（见 [`socket_read`] /
//! [`socket_write`] / [`socket_close`]，由 `syscall::file` 按号段分发）：
//! `read` = `recv`（首段即返），`write` = `send`（按 MSS 切块，可能只发出去一部分，
//! 返回值就是**已确认的字节数**；调用方据此重发剩余部分，不要整段重发）。

use abi::errno::{EAFNOSUPPORT, EINVAL};
use abi::sock::{ADDR_OFFSET, AF_INET, FAMILY_OFFSET, PORT_OFFSET, SOCKADDR_IN_LEN};

use crate::driver::serial;
use crate::network::socket;
use crate::syscall::address_space::UserSpace;
use crate::syscall::file::{user_bytes, user_bytes_mut};

/// 单次 `sendto`/`recvfrom` 允许的最大字节数（与 `read`/`write` 一致，`abi::limits`）。
const IO_MAX: u64 = abi::limits::IO_MAX;

/// 解析用户态的 `sockaddr_in`（指针 + 长度 → 本机序端口 + IPv4 四字节）。
///
/// # Safety
/// `ptr` 必须来自 ring3，且 `space` 必须是当前生效的用户地址空间。
unsafe fn parse_sockaddr(
    space: *const UserSpace,
    ptr: u64,
    len: u64,
    op: &str,
) -> Result<(u16, [u8; 4]), i64> {
    if len as usize != SOCKADDR_IN_LEN {
        return Err(EINVAL);
    }
    let raw = match unsafe { user_bytes(space, ptr, len, op) } {
        Ok(raw) => raw,
        Err(errno) => return Err(errno),
    };
    let family = u16::from_le_bytes([raw[FAMILY_OFFSET], raw[FAMILY_OFFSET + 1]]);
    if family != AF_INET {
        return Err(EAFNOSUPPORT);
    }
    let port = u16::from_be_bytes([raw[PORT_OFFSET], raw[PORT_OFFSET + 1]]);
    let addr = [raw[ADDR_OFFSET], raw[ADDR_OFFSET + 1], raw[ADDR_OFFSET + 2], raw[ADDR_OFFSET + 3]];
    Ok((port, addr))
}

/// `user: socket fd=<n>` / 失败行。
fn log_socket(fd_result: Result<i32, i64>, domain: u64, ty: u64, proto: u64) {
    match fd_result {
        Ok(fd) => {
            serial::print("user: socket fd=");
            serial::print_dec(fd as u64);
            serial::println("");
        }
        Err(errno) => {
            serial::print("user: socket domain=");
            serial::print_dec(domain);
            serial::print(" type=");
            serial::print_dec(ty);
            serial::print(" protocol=");
            serial::print_dec(proto);
            serial::print(" errno=");
            serial::print_dec(errno.unsigned_abs());
            serial::println("");
        }
    }
}

/// 系统调用 `socket(domain, type, protocol)`。
///
/// # Safety
/// `space` 必须是当前生效的用户地址空间（本调用不用用户缓冲，仅透传）。
pub unsafe fn sys_socket(args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let (domain, ty, proto) = (args[0], args[1] as i32, args[2] as i32);
    let ret = socket::create(domain, ty, proto);
    log_socket(ret, domain, args[1], args[2]);
    ret.map(|fd| fd as i64).unwrap_or_else(|errno| errno)
}

/// 系统调用 `bind(fd, addr_ptr, addr_len)`：固定本地端口（地址只接受本机）。
///
/// # Safety
/// 同 [`parse_sockaddr`]。
pub unsafe fn sys_bind(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let fd = args[0] as i32;
    let (port, addr) = match unsafe { parse_sockaddr(space, args[1], args[2], "bind") } {
        Ok(parsed) => parsed,
        Err(errno) => return errno,
    };
    // 只接受「本机」地址：全 0（bind-any，标准写法）—— 其它地址本机没有，不编造。
    if addr != [0, 0, 0, 0] {
        return EINVAL;
    }
    match socket::bind(fd, port) {
        Ok(()) => {
            serial::print("user: bind fd=");
            serial::print_dec(fd as u64);
            serial::print(" port=");
            serial::print_dec(port as u64);
            serial::println("");
            0
        }
        Err(errno) => {
            log_sock_errno("bind", fd, errno);
            errno
        }
    }
}

/// 系统调用 `connect(fd, addr_ptr, addr_len)`：三次握手（阻塞到建立或失败）。
///
/// # Safety
/// 同 [`parse_sockaddr`]。
pub unsafe fn sys_connect(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let fd = args[0] as i32;
    let (port, addr) = match unsafe { parse_sockaddr(space, args[1], args[2], "connect") } {
        Ok(parsed) => parsed,
        Err(errno) => return errno,
    };
    match socket::connect(fd, addr, port) {
        Ok(()) => {
            serial::print("user: connect fd=");
            serial::print_dec(fd as u64);
            serial::print(" ");
            serial::print_dec(addr[0] as u64);
            serial::print(".");
            serial::print_dec(addr[1] as u64);
            serial::print(".");
            serial::print_dec(addr[2] as u64);
            serial::print(".");
            serial::print_dec(addr[3] as u64);
            serial::print(":");
            serial::print_dec(port as u64);
            serial::println(" ok");
            0
        }
        Err(errno) => {
            log_sock_errno("connect", fd, errno);
            errno
        }
    }
}

/// 系统调用 `sendto(fd, buf, len, 0, 0)`：已连接套接字上发送。
///
/// 返回已确认的字节数：中途超时/RST 时可能少于 `len`（前几段已送达），
/// 只有一段都没发出去时才返回负 errno。
///
/// # Safety
/// `space` 必须是当前生效的用户地址空间；`buf` 必须可读。
pub unsafe fn sys_sendto(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let (fd, buf, len) = (args[0] as i32, args[1], args[2]);
    if args[3] != 0 || args[4] != 0 {
        return EINVAL; // 已连接字节流不接受逐包目的地址（UDP 套接字未实现）
    }
    if len > IO_MAX {
        return EINVAL;
    }
    if len == 0 {
        return 0;
    }
    let bytes = match unsafe { user_bytes(space, buf, len, "sendto") } {
        Ok(bytes) => bytes,
        Err(errno) => return errno,
    };
    match socket::send(fd, bytes) {
        Ok(n) => {
            serial::print("user: send fd=");
            serial::print_dec(fd as u64);
            serial::print(" len=");
            serial::print_dec(n as u64);
            serial::println("");
            n as i64
        }
        Err(errno) => {
            log_sock_errno("send", fd, errno);
            errno
        }
    }
}

/// 系统调用 `recvfrom(fd, buf, len, 0, 0)`：首段即返（`0` = 对端有序关闭）。
///
/// # Safety
/// `space` 必须是当前生效的用户地址空间；`buf` 必须可写。
pub unsafe fn sys_recvfrom(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let (fd, buf, len) = (args[0] as i32, args[1], args[2]);
    if args[3] != 0 || args[4] != 0 {
        return EINVAL; // 地址回填未实现：传 0（见模块文档）
    }
    if len > IO_MAX {
        return EINVAL;
    }
    if len == 0 {
        return 0;
    }
    let out = match unsafe { user_bytes_mut(space, buf, len, "recvfrom") } {
        Ok(out) => out,
        Err(errno) => return errno,
    };
    match socket::recv(fd, out) {
        Ok(n) => {
            serial::print("user: recv fd=");
            serial::print_dec(fd as u64);
            serial::print(" want=");
            serial::print_dec(len);
            serial::print(" got=");
            serial::print_dec(n as u64);
            serial::println("");
            n as i64
        }
        Err(errno) => {
            log_sock_errno("recv", fd, errno);
            errno
        }
    }
}

/// `user: <op> fd=<n> errno=<n>`（套接字失败留痕，与文件类的 `log_errno` 同形）。
fn log_sock_errno(op: &str, fd: i32, err: i64) {
    serial::print("user: ");
    serial::print(op);
    serial::print(" fd=");
    serial::print_dec(fd as u64);
    serial::print(" errno=");
    serial::print_dec(err.unsigned_abs());
    serial::println("");
}

/// `read(fd, ...)` 落到套接字 fd 时的实际处理（由 `syscall::file::sys_read` 分发）。
///
/// # Safety
/// 同 [`sys_recvfrom`]。
pub unsafe fn socket_read(fd: i32, space: *const UserSpace, buf: u64, count: u64) -> i64 {
    if count > IO_MAX {
        return EINVAL;
    }
    if count == 0 {
        return 0;
    }
    let out = match unsafe { user_bytes_mut(space, buf, count, "read") } {
        Ok(out) => out,
        Err(errno) => return errno,
    };
    match socket::recv(fd, out) {
        Ok(n) => {
            serial::print("user: read fd=");
            serial::print_dec(fd as u64);
            serial::print(" want=");
            serial::print_dec(count);
            serial::print(" got=");
            serial::print_dec(n as u64);
            serial::println("");
            n as i64
        }
        Err(errno) => {
            log_sock_errno("read", fd, errno);
            errno
        }
    }
}

/// `write(fd, ...)` 落到套接字 fd 时的实际处理（由 `syscall::file::sys_write` 分发）。
///
/// # Safety
/// `space` 必须是当前生效的用户地址空间；`buf` 必须可读。
pub unsafe fn socket_write(fd: i32, space: *const UserSpace, buf: u64, count: u64) -> i64 {
    if count > IO_MAX {
        return EINVAL;
    }
    if count == 0 {
        return 0;
    }
    let bytes = match unsafe { user_bytes(space, buf, count, "write") } {
        Ok(bytes) => bytes,
        Err(errno) => return errno,
    };
    match socket::send(fd, bytes) {
        Ok(n) => {
            serial::print("user: write fd=");
            serial::print_dec(fd as u64);
            serial::print(" len=");
            serial::print_dec(n as u64);
            serial::println("");
            n as i64
        }
        Err(errno) => {
            log_sock_errno("write", fd, errno);
            errno
        }
    }
}

/// `close(fd)` 落到套接字 fd 时的实际处理（由 `syscall::file::sys_close` 分发）。
pub fn socket_close(fd: i32) -> i64 {
    match socket::close(fd) {
        Ok(()) => {
            serial::print("user: close fd=");
            serial::print_dec(fd as u64);
            serial::println("");
            0
        }
        Err(errno) => {
            log_sock_errno("close", fd, errno);
            errno
        }
    }
}
