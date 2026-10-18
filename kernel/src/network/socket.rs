//! Socket 层：AF_INET / SOCK_STREAM（TCP）套接字表。
//!
//! v0.9 只在内核内部验证网络路径（`network::selftest`），`socket.rs` 是占位；
//! 这里补的是用户态 `socket` / `bind` / `connect` / `sendto` / `recvfrom` 的后端
//! （syscall 入口在 [`crate::syscall::net`]，libc 封装在 `userland/libc`）：
//!
//! | 项 | 取值 |
//! | ---- | ---- |
//! | 域 / 类型 / 协议 | 只认 `AF_INET` + `SOCK_STREAM` + `IPPROTO_TCP`（其余 `-EAFNOSUPPORT`） |
//! | fd | [`SOCK_FD_BASE`] 起的独立号段（16..=23），不与文件 fd（0..=15）重叠 |
//! | 本机地址 | [`crate::network::local_ip`]（DHCP 租约落地后的地址；未租到时是 slirp 默认值） |
//! | 本机端口 | `bind` 固定，或 `connect` 时从临时段自动分配 |
//!
//! 约束（与文件 fd 表同一模型）：**全局表、一次只服务一个用户程序**，
//! 每次进入用户态前由 [`reset`] 清空（`syscall::user::run_process` 调用）。
//! `fork` 不复制套接字（进程表也没有每进程 fd 表，见 `fs::file` 模块头）。
//!
//! 不做：UDP 套接字（`SOCK_DGRAM` 具名拒绝）、服务端（`listen`/`accept`）、
//! 非阻塞标志（`recv` 等到首段或超时，超时即 `-ETIMEDOUT`）、
//! `sendto` 带目的地址（已连接套接字上传 `NULL`，见 syscall 层校验）。

use abi::errno::{EAFNOSUPPORT, EBADF, EINVAL, ENOTCONN};
use abi::sock::{AF_INET, IPPROTO_TCP, SOCK_STREAM};

use crate::driver::net;
use crate::network::{ipv4, tcp};

/// 套接字 fd 起始号（文件 fd 表是 0..=15，见 `fs::file::FD_MAX`）。
pub const SOCK_FD_BASE: i32 = 16;
/// 套接字数量上限（8 个，静态表）。
pub const MAX_SOCKETS: usize = 8;
/// 临时端口段起点（`bind` 未固定时 `connect` 自动分配，循环使用）。
const EPHEMERAL_START: u16 = 49152;

/// 套接字状态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    /// 空槽位。
    Free,
    /// 已 `socket`： Model 端口待定（`bind` 固定或 `connect` 自动分配）。
    Created,
    /// 已 `bind`：本地端口固定，尚未 `connect`。
    Bound,
    /// 已 `connect`：[`tcp::Conn`] 持有序号状态。
    Connected,
}

/// 一个套接字（纯数据，可进静态表；`Copy` 是为了 `[None; N]` 初始化）。
#[derive(Clone, Copy)]
struct Socket {
    /// 状态。
    state: State,
    /// 本机端口（0 = 未分配）。
    local_port: u16,
    /// 连接状态（`Connected` 时有效）。
    conn: Option<tcp::Conn>,
}

impl Socket {
    const EMPTY: Self = Self {
        state: State::Free,
        local_port: 0,
        conn: None,
    };
}

static mut TABLE: [Socket; MAX_SOCKETS] = [Socket::EMPTY; MAX_SOCKETS];
/// 下一个自动分配的临时端口。
static mut NEXT_EPHEMERAL: u16 = EPHEMERAL_START;
/// IPv4 标识计数器（TCP 段用；与 `network::Stack::next_id` 各自递增，互不干扰）。
static mut NEXT_ID: u16 = 1;


/// 是否为套接字 fd（号段判定；不查表，因此 `EBADF` 与「不是套接字」可区分）。
pub fn is_socket_fd(fd: i32) -> bool {
    // 严格的半开区间 `[BASE, BASE + MAX_SOCKETS)`：`BASE + MAX_SOCKETS` 本身越界。
    fd >= SOCK_FD_BASE && ((fd - SOCK_FD_BASE) as usize) < MAX_SOCKETS
}

/// 槽位下标（调用方已用 [`is_socket_fd`] 确认号段）。
fn index(fd: i32) -> usize {
    (fd - SOCK_FD_BASE) as usize
}

/// 下标是否落在表内（号段末尾 `BASE + MAX` 本身越界，返回 `EBADF` 而不是踩表）。
fn in_range(idx: usize) -> bool {
    idx < MAX_SOCKETS
}

/// 清空套接字表（进入用户态前调用；语义与 `fs::file::reset` 一致）。
pub fn reset() {
    unsafe {
        TABLE = [Socket::EMPTY; MAX_SOCKETS];
        NEXT_EPHEMERAL = EPHEMERAL_START;
        NEXT_ID = 1;
    }
}

/// 取设备（无网卡时调套接字的任何网络动作都是 `-EIO`，见 `abi::errno::EIO`）。
fn device() -> Result<&'static mut dyn net::NetworkDevice, i64> {
    net::first().ok_or(abi::errno::EIO)
}

/// 本机 MAC（设备配置空间）。
fn src_mac(dev: &mut dyn net::NetworkDevice) -> [u8; 6] {
    dev.mac()
}

/// 分配一个临时端口（`bind` 未固定时用；跳过表内已占用的）。
fn alloc_ephemeral() -> u16 {
    unsafe {
        for _ in 0..(u16::MAX - EPHEMERAL_START) {
            let port = NEXT_EPHEMERAL;
            NEXT_EPHEMERAL = if port >= u16::MAX {
                EPHEMERAL_START
            } else {
                port + 1
            };
            let taken = (&raw const TABLE)
                .as_ref()
                .map(|t| t.iter().any(|s| s.state != State::Free && s.local_port == port))
                .unwrap_or(false);
            if !taken {
                return port;
            }
        }
        EPHEMERAL_START // 理论上不可达（表最多 8 项）：兜底而不是 panic
    }
}

/// `socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)`：分配槽位，返回 fd。
pub fn create(domain: u64, ty: i32, protocol: i32) -> Result<i32, i64> {
    // 必须比较完整的 u64：`domain as u16` 会让 0x1_0002 这种值被截断成 AF_INET。
    if domain != AF_INET as u64 {
        return Err(EAFNOSUPPORT);
    }
    if ty != SOCK_STREAM {
        return Err(EAFNOSUPPORT);
    }
    if protocol != 0 && protocol != IPPROTO_TCP {
        return Err(EAFNOSUPPORT);
    }
    unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        for (i, slot) in table.iter_mut().enumerate() {
            if slot.state == State::Free {
                *slot = Socket {
                    state: State::Created,
                    local_port: 0,
                    conn: None,
                };
                return Ok(SOCK_FD_BASE + i as i32);
            }
        }
    }
    Err(abi::errno::ENOSPC) // 套接字用尽（与 fd 用尽同一 errno）
}

/// `bind(fd, port)`：固定本地端口（地址只接受本机，见 syscall 层）。
pub fn bind(fd: i32, port: u16) -> Result<(), i64> {
    if port == 0 {
        return Err(EINVAL); // 端口 0（自动分配）只在 `connect` 里合法，`bind` 必须明确
    }
    let idx = index(fd);
    if !in_range(idx) {
        return Err(EBADF);
    }
    unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        let state = table.get(idx).ok_or(EBADF)?.state;
        if state == State::Free {
            return Err(EBADF);
        }
        if state != State::Created {
            // 已绑定/已连接上再 bind：Linux 报 EINVAL（不是 EBADF：fd 本身有效）
            return Err(EINVAL);
        }
        if table
            .iter()
            .any(|s| s.state != State::Free && s.local_port == port)
        {
            return Err(EINVAL); // 端口已被本表另一套接字占用（ABI 尚无 EADDRINUSE）
        }
        let slot = table.get_mut(idx).ok_or(EBADF)?;
        slot.state = State::Bound;
        slot.local_port = port;
        Ok(())
    }
}

/// `connect(fd, remote_ip, remote_port)`：三次握手（阻塞到建立或失败）。
pub fn connect(fd: i32, remote_ip: [u8; 4], remote_port: u16) -> Result<(), i64> {
    if remote_port == 0 {
        return Err(EINVAL);
    }
    let idx = index(fd);
    if !in_range(idx) {
        return Err(EBADF);
    }
    let local_port = unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        let slot = table.get_mut(idx).ok_or(EBADF)?;
        if slot.state != State::Created && slot.state != State::Bound {
            return Err(EINVAL); // 已连接上再 connect：fd 有效但状态不对
        }
        if slot.state == State::Created {
            slot.local_port = alloc_ephemeral();
        }
        slot.local_port
    };
    let dev = device()?;
    let mac = src_mac(&mut *dev);
    let next_id = unsafe { &mut *(&raw mut NEXT_ID) };
    match tcp::connect(
        &mut *dev,
        mac,
        crate::network::local_ip(),
        next_id,
        ipv4::Address::from_octets(remote_ip),
        remote_port,
        local_port,
    ) {
        Ok(conn) => {
            unsafe {
                let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
                let slot = table.get_mut(idx).ok_or(EBADF)?;
                slot.state = State::Connected;
                slot.conn = Some(conn);
            }
            Ok(())
        }
        Err(err) => Err(err.errno()),
    }
}

/// 在已连接套接字上发送 `data`，返回**已确认的字节数**（可能少于 `data.len()`）。
pub fn send(fd: i32, data: &[u8]) -> Result<usize, i64> {
    if data.is_empty() {
        return Ok(0);
    }
    let idx = index(fd);
    if !in_range(idx) {
        return Err(EBADF);
    }
    // 先把连接状态拷出来（`tcp::send` 要 `&mut Conn`，不能在持有表借用的同时调它）。
    let mut conn = unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        let slot = table.get_mut(idx).ok_or(EBADF)?;
        slot.conn.ok_or(ENOTCONN)?
    };
    let dev = device()?;
    let mac = src_mac(&mut *dev);
    let next_id = unsafe { &mut *(&raw mut NEXT_ID) };
    match tcp::send(&mut conn, &mut *dev, mac, next_id, data) {
        // 可能只发出去一部分（对端在中途 RST/超时）：把**已确认**的字节数交回用户态，
        // 由它决定是否重发剩余部分 —— 报错却丢掉进度会让调用方整段重发成重复数据。
        Ok(n) => {
            unsafe {
                let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
                if let Some(slot) = table.get_mut(idx) {
                    slot.conn = Some(conn);
                }
            }
            Ok(n)
        }
        Err(err) => {
            // 序号状态已推进（部分段被确认）：写回再报错，不丢确认进度
            unsafe {
                let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
                if let Some(slot) = table.get_mut(idx) {
                    slot.conn = Some(conn);
                }
            }
            Err(err.errno())
        }
    }
}

/// 在已连接套接字上收最多 `out.len()` 字节（首段即返；`Ok(0)` = 对端有序关闭）。
pub fn recv(fd: i32, out: &mut [u8]) -> Result<usize, i64> {
    if out.is_empty() {
        return Ok(0);
    }
    let idx = index(fd);
    if !in_range(idx) {
        return Err(EBADF);
    }
    let mut conn = unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        let slot = table.get_mut(idx).ok_or(EBADF)?;
        slot.conn.ok_or(ENOTCONN)?
    };
    let dev = device()?;
    let mac = src_mac(&mut *dev);
    let next_id = unsafe { &mut *(&raw mut NEXT_ID) };
    match tcp::recv_upto(&mut conn, &mut *dev, mac, next_id, out) {
        Ok(n) => {
            unsafe {
                let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
                if let Some(slot) = table.get_mut(idx) {
                    slot.conn = Some(conn);
                }
            }
            Ok(n)
        }
        Err(err) => {
            unsafe {
                let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
                if let Some(slot) = table.get_mut(idx) {
                    slot.conn = Some(conn);
                }
            }
            Err(err.errno())
        }
    }
}

/// `close(fd)`：已连接则先 FIN 关闭（握手失败也照常释放槽位），再释放。
pub fn close(fd: i32) -> Result<(), i64> {
    let idx = index(fd);
    if !in_range(idx) {
        return Err(EBADF);
    }
    let conn = unsafe {
        let table = (&raw mut TABLE).as_mut().expect("TABLE 是静态量");
        let slot = table.get_mut(idx).ok_or(EBADF)?;
        if slot.state == State::Free {
            return Err(EBADF);
        }
        let conn = slot.conn.take();
        *slot = Socket::EMPTY;
        conn
    };
    if let Some(mut conn) = conn {
        // FIN 握手是「尽力」：对端已死的连接照样要释放，错误只上报不残留
        if let Ok(dev) = device() {
            let mac = src_mac(&mut *dev);
            let next_id = unsafe { &mut *(&raw mut NEXT_ID) };
            if let Err(err) = tcp::close(&mut conn, &mut *dev, mac, next_id) {
                return Err(err.errno());
            }
        }
    }
    Ok(())
}
