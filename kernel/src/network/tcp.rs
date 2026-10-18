//! TCP（客户端最小集：三次握手 + 按序收发 + FIN 关闭）
//!
//! v0.9 的「简化网络」只到 UDP（`docs/architecture.md` 把完整 TCP/IP 列为非目标）；
//! 这里补的是 **socket 可用的最小 TCP** —— 能对 QEMU slirp 后面的主机服务完成
//! 一次 `SYN → SYN-ACK → ACK → 数据往返 → FIN`，而不是工业栈：
//!
//! ```text
//! 做：客户端主动打开（connect）、序号/确认号、单段发送 + ACK 等待 + 重传、
//!     按序接收 + 累积 ACK、RST 识别（ECONNREFUSED）、FIN 半关闭与完整关闭。
//! 不做：服务端（listen/accept）、窗口通告与滑动窗口（通告固定 65535，从不按它流控）、
//!       SACK/时间戳/窗口扩大（发出的段无选项；收到的段选项跳过只读序号）、
//!       乱序重组（只收 `seq == rcv_nxt`，其余丢弃并重发当前 ACK）、
//!       快重传/RTO 估算（固定超时 + 固定次数，见下面的常量）、
//!       IP 分片（ipv4 层已丢弃 MF/offset 报文，这里只会看到完整段）。
//! ```
//!
//! 同步模型与块设备/UDP 一致：**一次只飞一个段**，用 PIT tick 做超时（100Hz）。
//! 超时与次数的选择理由：slirp 是本机回环量级，握手 1 秒内必回；5 次 SYN 重传
//! （≈5 秒）后仍无应答，说明对端根本不在（或网络不可达），报超时而不是无限等。
//!
//! 校验和（含 IPv4 伪头）收发两端都算：发出的段算好填入；收到的段重算校验
//! （`finish(sum) == 0`，与 [`crate::network::udp`] 同一约定）——
//! 校验错的段直接丢弃（当作没收到，等超时重传），不算协议错误。

use crate::driver::net::{self, NetworkDevice};
use crate::driver::serial;
use crate::network::{arp, ethernet, ipv4};
use crate::time::clock;

/// TCP 协议号（IPv4 头的 protocol 字段）。
pub const PROTO_TCP: u8 = 6;
/// TCP 头长度（无选项，20 字节；收到的段选项按 `data_offset` 跳过）。
pub const HEADER_LEN: usize = 20;
/// 单个段的最大载荷（1500 MTU − 20 IP − 20 TCP，不做路径发现）。
pub const MSS: usize = 1460;

/// 标志位：FIN（关闭）。
pub const FLAG_FIN: u8 = 0x01;
/// 标志位：SYN（同步序号）。
pub const FLAG_SYN: u8 = 0x02;
/// 标志位：RST（拒绝/重置）。
pub const FLAG_RST: u8 = 0x04;
/// 标志位：PSH（接收方尽快交付；发送数据段恒置）。
pub const FLAG_PSH: u8 = 0x08;
/// 标志位：ACK（确认号有效；SYN 之后的所有段恒置）。
pub const FLAG_ACK: u8 = 0x10;

/// SYN 每次等待 SYN-ACK 的上限（100 tick = 1 秒）。
const SYN_TIMEOUT_TICKS: u64 = 100;
/// SYN 重传次数（含首次，共发 1 + 4 次）。
const SYN_ATTEMPTS: usize = 5;
/// 数据段每次等待 ACK 的上限（200 tick = 2 秒）。
const DATA_TIMEOUT_TICKS: u64 = 200;
/// 数据段重传次数（含首次）。
const DATA_ATTEMPTS: usize = 4;
/// 等待首个数据段的总上限（300 tick = 3 秒）。
const RECV_TIMEOUT_TICKS: u64 = 300;
/// 关闭（FIN 交换）的总上限（400 tick = 4 秒）。
const CLOSE_TIMEOUT_TICKS: u64 = 400;
/// ARP 解析等待上限（与 [`crate::network::Stack`] 的 ARP 超时一致）。
const ARP_TIMEOUT_TICKS: u64 = 100;
/// 每连接接收缓冲容量（字节）。
///
/// 为什么需要它：`send` 等待某一段的 ACK 时，对端可能已经在回数据
///（echo/请求-应答协议）。这些字节必须按序留在连接里等 `recv` 取走，
/// 不能“收下并 ACK 后丢掉” —— 那会让字节流出现空洞。容量取 4 KiB
///（≥ 2 个 MSS），足以覆盖一次往返里对端能塞进来的数据。
const RX_CAP: usize = 4096;

/// TCP 路径的失败（调用方按它换 errno：超时与拒绝是两种不同的事实）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// 重传耗尽仍无应答 → `-ETIMEDOUT`。
    Timeout,
    /// 对端 RST → `-ECONNREFUSED`。
    Refused,
}

impl Error {
    /// 换算成 `-errno`（定义在 `abi::errno`）。
    pub const fn errno(self) -> i64 {
        match self {
            Error::Timeout => abi::errno::ETIMEDOUT,
            Error::Refused => abi::errno::ECONNREFUSED,
        }
    }

    /// 串口证据里的名字（`net: tcp FAIL: <name>`）。
    pub const fn name(self) -> &'static str {
        match self {
            Error::Timeout => "timeout",
            Error::Refused => "refused",
        }
    }
}

/// 一条已建立的客户端连接（序号状态；I/O 时连同设备一起传进来）。
#[derive(Clone, Copy, Debug)]
pub struct Conn {
    /// 本机地址（含端口）。
    pub local_ip: ipv4::Address,
    /// 本机端口。
    pub local_port: u16,
    /// 对端地址（含端口）。
    pub remote_ip: ipv4::Address,
    /// 对端端口。
    pub remote_port: u16,
    /// 对端 MAC（ARP 解析得到，发送帧的目的地址）。
    pub remote_mac: [u8; 6],
    /// 下一个要发送的序号。
    pub snd_nxt: u32,
    /// 对端已确认到的序号（累积 ACK 的右沿）。
    pub snd_una: u32,
    /// 下一个期望收到的序号。
    pub rcv_nxt: u32,
    /// 已收到、尚未交付给调用方的字节（FIFO；`rx[..rx_len]` 有效）。
    pub rx: [u8; RX_CAP],
    /// `rx` 里的有效字节数。
    pub rx_len: usize,
    /// 对端已发 FIN：此后 `recv` 立即返回 0，`close` 不必再等对端 FIN。
    pub peer_closed: bool,
}

/// 把 `data` 追加到接收 FIFO；装不下返回 `false`（调用方不得推进 `rcv_nxt`，
/// 让对端按自己的重传节奏再来一次，而不是把半段数据拼进流里）。
fn rx_push(conn: &mut Conn, data: &[u8]) -> bool {
    if conn.rx_len + data.len() > RX_CAP {
        return false;
    }
    let end = conn.rx_len + data.len();
    conn.rx[conn.rx_len..end].copy_from_slice(data);
    conn.rx_len = end;
    true
}

/// 从接收 FIFO 取最多 `out.len()` 字节，返回实际取出的字节数。
fn rx_pop(conn: &mut Conn, out: &mut [u8]) -> usize {
    let n = conn.rx_len.min(out.len());
    out[..n].copy_from_slice(&conn.rx[..n]);
    conn.rx.copy_within(n..conn.rx_len, 0);
    conn.rx_len -= n;
    n
}

/// 收到的一个匹配段（载荷已拷进调用方缓冲）。
struct Recv {
    /// 段序号。
    seq: u32,
    /// 段确认号。
    ack: u32,
    /// 标志位。
    flags: u8,
    /// 载荷字节数（`payload[..len]` 有效）。
    len: usize,
}

/// 读取 PIT tick（`clock::tick_count` 是 unsafe：调用方须处于内核态）。
fn ticks() -> u64 {
    // SAFETY: TCP 只在 ring0（启动自测与 syscall handler）里运行。
    unsafe { clock::tick_count() }
}

/// 由端口与 tick 拼一个初始序号（同一 tick 内不同端口不重；单调性不保证，够用）。
fn make_iss(local_port: u16) -> u32 {
    (ticks() as u32)
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add((local_port as u32) << 16 | 0x4345)
}

/// TCP 校验和：伪头（源/目的 IP + 零 + 协议 + 段长）+ 整个段。
fn checksum(src: ipv4::Address, dst: ipv4::Address, segment: &[u8]) -> u16 {
    let mut pseudo = [0u8; 12];
    pseudo[0..4].copy_from_slice(&src.octets());
    pseudo[4..8].copy_from_slice(&dst.octets());
    pseudo[8] = 0;
    pseudo[9] = PROTO_TCP;
    pseudo[10..12].copy_from_slice(&(segment.len() as u16).to_be_bytes());
    let sum = ipv4::sum16(0, &pseudo);
    let sum = ipv4::sum16(sum, segment);
    ipv4::finish(sum)
}

/// 写一个 TCP 段（20 字节头 + 载荷），返回段总字节数。
///
/// 校验和按规范先置 0 再计算。`header_reserve` 必须 ≥ 20（调用方传 IPv4 payload 切片）。
pub fn build(
    buf: &mut [u8],
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
    src_ip: ipv4::Address,
    dst_ip: ipv4::Address,
) -> Option<usize> {
    let total = HEADER_LEN.checked_add(payload.len())?;
    if buf.len() < total {
        return None;
    }
    buf[0..2].copy_from_slice(&src_port.to_be_bytes());
    buf[2..4].copy_from_slice(&dst_port.to_be_bytes());
    buf[4..8].copy_from_slice(&seq.to_be_bytes());
    buf[8..12].copy_from_slice(&ack.to_be_bytes());
    buf[12] = 0x50; // data_offset = 5（20 字节，无选项），保留位 0
    buf[13] = flags;
    buf[14..16].copy_from_slice(&65535u16.to_be_bytes()); // 通告窗口（固定，不流控）
    buf[16..18].copy_from_slice(&0u16.to_be_bytes()); // 校验和占位
    buf[18..20].copy_from_slice(&0u16.to_be_bytes()); // 紧急指针恒 0
    buf[HEADER_LEN..total].copy_from_slice(payload);
    let sum = checksum(src_ip, dst_ip, &buf[..total]);
    buf[16..18].copy_from_slice(&sum.to_be_bytes());
    Some(total)
}

/// 解析出的段头（载荷是切片；调用方先看序号再决定拷不拷）。
pub struct Segment<'a> {
    /// 源端口。
    pub src_port: u16,
    /// 目的端口。
    pub dst_port: u16,
    /// 序号。
    pub seq: u32,
    /// 确认号。
    pub ack: u32,
    /// 标志位。
    pub flags: u8,
    /// 载荷（已跳过选项）。
    pub payload: &'a [u8],
}

/// 解析一个 TCP 段（`buf` 是 IPv4 payload）。
///
/// 接受带选项的段（`data_offset > 5`，真实协议栈的 SYN-ACK 恒带 MSS/窗口扩大等
/// 选项）：头长按 `data_offset * 4` 算，只要求段字节数足够。拒绝：段不足 20 字节、
/// 头长自称超过段长、校验和错误。
pub fn parse(buf: &[u8], src_ip: ipv4::Address, dst_ip: ipv4::Address) -> Option<Segment<'_>> {
    if buf.len() < HEADER_LEN {
        return None;
    }
    let header_len = ((buf[12] >> 4) as usize) * 4;
    if header_len < HEADER_LEN || header_len > buf.len() {
        return None;
    }
    if checksum(src_ip, dst_ip, buf) != 0 {
        return None; // 校验和应当让「伪头 + 整段」求和为 0（与 UDP 同一约定）
    }
    Some(Segment {
        src_port: u16::from_be_bytes([buf[0], buf[1]]),
        dst_port: u16::from_be_bytes([buf[2], buf[3]]),
        seq: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        ack: u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
        flags: buf[13],
        payload: &buf[header_len..],
    })
}

/// 发一个 TCP 段（Ethernet + IPv4 封装由这里负责）。
#[allow(clippy::too_many_arguments)]
fn send_segment(
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    dst_mac: [u8; 6],
    src_ip: ipv4::Address,
    dst_ip: ipv4::Address,
    next_id: &mut u16,
    src_port: u16,
    dst_port: u16,
    seq: u32,
    ack: u32,
    flags: u8,
    payload: &[u8],
) -> Result<(), Error> {
    if payload.len() > MSS {
        return Err(Error::Timeout); // 内部断言：调用方必须按 MSS 切块（不静默截断）
    }
    let mut frame = [0u8; net::FRAME_MAX];
    let id = *next_id;
    *next_id = next_id.wrapping_add(1);
    let eth = ethernet::build(&mut frame, dst_mac, src_mac, ethernet::ETHERTYPE_IPV4)
        .ok_or(Error::Timeout)?;
    let ip = ipv4::build(eth, src_ip, dst_ip, PROTO_TCP, 64, id, HEADER_LEN + payload.len())
        .ok_or(Error::Timeout)?;
    build(ip, src_port, dst_port, seq, ack, flags, payload, src_ip, dst_ip)
        .ok_or(Error::Timeout)?;
    let total = ethernet::HEADER_LEN + ipv4::HEADER_LEN + HEADER_LEN + payload.len();
    dev.transmit(&frame[..total]).map_err(|_| Error::Timeout)?;
    Ok(())
}

/// 发一个纯 ACK（载荷为空；`PSH` 不置）。
fn send_ack(
    dev: &mut dyn NetworkDevice,
    conn: &Conn,
    src_mac: [u8; 6],
    next_id: &mut u16,
) -> Result<(), Error> {
    send_segment(
        dev, src_mac, conn.remote_mac, conn.local_ip, conn.remote_ip, next_id,
        conn.local_port, conn.remote_port, conn.snd_nxt, conn.rcv_nxt, FLAG_ACK, &[],
    )
}

/// 等一个四元组匹配的段，载荷拷进 `payload_out`（装不下则截断到缓冲长）。
///
/// 不匹配的帧（ARP/DHCP/别人的段）直接跳过；RST 无论序号一律返回（调用方判拒绝）。
/// 超时返回 `None`（调用方决定重传还是放弃）。
fn recv_match(
    dev: &mut dyn NetworkDevice,
    payload_out: &mut [u8],
    timeout_ticks: u64,
    local_ip: ipv4::Address,
    local_port: u16,
    remote_ip: ipv4::Address,
    remote_port: u16,
) -> Option<Recv> {
    let mut rx = [0u8; net::FRAME_MAX];
    let start = ticks();
    loop {
        match dev.receive(&mut rx) {
            Ok(Some(n)) => {
                let Some(eth) = ethernet::parse(&rx[..n]) else {
                    continue;
                };
                if eth.ethertype != ethernet::ETHERTYPE_IPV4 {
                    continue;
                }
                let Some((header, ip_payload)) = ipv4::parse(eth.payload) else {
                    continue;
                };
                if header.protocol != PROTO_TCP
                    || header.src != remote_ip
                    || header.dst != local_ip
                {
                    continue;
                }
                let Some(seg) = parse(ip_payload, header.src, header.dst) else {
                    continue;
                };
                if seg.src_port != remote_port || seg.dst_port != local_port {
                    continue;
                }
                if seg.flags & FLAG_RST != 0 {
                    return Some(Recv {
                        seq: seg.seq,
                        ack: seg.ack,
                        flags: seg.flags,
                        len: 0,
                    });
                }
                let n = seg.payload.len().min(payload_out.len());
                payload_out[..n].copy_from_slice(&seg.payload[..n]);
                return Some(Recv {
                    seq: seg.seq,
                    ack: seg.ack,
                    flags: seg.flags,
                    len: n,
                });
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        if ticks().wrapping_sub(start) > timeout_ticks {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// 三次握手：`SYN → SYN-ACK → ACK`（`next_id` 是 IPv4 标识计数器，由调用方持有）。
pub fn connect(
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    src_ip: ipv4::Address,
    next_id: &mut u16,
    remote_ip: ipv4::Address,
    remote_port: u16,
    local_port: u16,
) -> Result<Conn, Error> {
    let remote_mac =
        arp::resolve(dev, src_mac, src_ip, remote_ip, ARP_TIMEOUT_TICKS).ok_or(Error::Timeout)?;
    let iss = make_iss(local_port);
    for _ in 0..SYN_ATTEMPTS {
        send_segment(
            dev, src_mac, remote_mac, src_ip, remote_ip, next_id, local_port, remote_port, iss, 0,
            FLAG_SYN, &[],
        )?;
        let mut tmp = [0u8; MSS];
        let Some(seg) = recv_match(
            dev,
            &mut tmp,
            SYN_TIMEOUT_TICKS,
            src_ip,
            local_port,
            remote_ip,
            remote_port,
        ) else {
            continue; // 超时 → 重传 SYN
        };
        if seg.flags & FLAG_RST != 0 {
            return Err(Error::Refused);
        }
        if seg.flags & FLAG_SYN != 0
            && seg.flags & FLAG_ACK != 0
            && seg.ack == iss.wrapping_add(1)
        {
            let conn = Conn {
                local_ip: src_ip,
                local_port,
                remote_ip,
                remote_port,
                remote_mac,
                snd_nxt: iss.wrapping_add(1),
                snd_una: seg.ack,
                rcv_nxt: seg.seq.wrapping_add(1),
                rx: [0u8; RX_CAP],
                rx_len: 0,
                peer_closed: false,
            };
            send_ack(dev, &conn, src_mac, next_id)?;
            return Ok(conn);
        }
        // 其它段（迟到的重传/纯 ACK）忽略，等本轮超时后重传 SYN
    }
    Err(Error::Timeout)
}

/// 发送 `data`（按 MSS 切块，每块等累积 ACK）。
///
/// 返回**已确认的字节数**：中途失败时可能是 `Ok(sent)`（`0 < sent < data.len()`），
/// 调用方（`socket::send`）原样把它交给用户态，避免“报错但前几段已送达”被重发成重复数据。
/// 只有一段都没发出去时才返回 `Err`。
pub fn send(
    conn: &mut Conn,
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    next_id: &mut u16,
    mut data: &[u8],
) -> Result<usize, Error> {
    let mut sent = 0usize;
    while !data.is_empty() {
        let chunk_len = data.len().min(MSS);
        let (chunk, rest) = data.split_at(chunk_len);
        let mut attempt = 0;
        let mut acked = false;
        while !acked {
            if attempt >= DATA_ATTEMPTS {
                return if sent > 0 { Ok(sent) } else { Err(Error::Timeout) };
            }
            attempt += 1;
            if let Err(err) = send_segment(
                dev, src_mac, conn.remote_mac, conn.local_ip, conn.remote_ip, next_id,
                conn.local_port, conn.remote_port, conn.snd_nxt, conn.rcv_nxt,
                FLAG_PSH | FLAG_ACK, chunk,
            ) {
                return if sent > 0 { Ok(sent) } else { Err(err) };
            }
            // 一次 attempt 的总等待上限：不能因为对端持续发重复 ACK 就无限等下去。
            let attempt_start = ticks();
            loop {
                let elapsed = ticks().wrapping_sub(attempt_start);
                if elapsed >= DATA_TIMEOUT_TICKS {
                    break; // 本轮超时 → 重传本块
                }
                let mut tmp = [0u8; MSS];
                let Some(seg) = recv_match(
                    dev,
                    &mut tmp,
                    DATA_TIMEOUT_TICKS - elapsed,
                    conn.local_ip,
                    conn.local_port,
                    conn.remote_ip,
                    conn.remote_port,
                ) else {
                    break; // 超时 → 重传本块
                };
                if seg.flags & FLAG_RST != 0 {
                    return if sent > 0 { Ok(sent) } else { Err(Error::Refused) };
                }
                // 对端抢发的数据：按序收进接收 FIFO 并 ACK（不能收下就丢）。
                if seg.len > 0 && seg.seq == conn.rcv_nxt {
                    if rx_push(conn, &tmp[..seg.len]) {
                        conn.rcv_nxt = conn.rcv_nxt.wrapping_add(seg.len as u32);
                        if seg.flags & FLAG_FIN != 0 {
                            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                            conn.peer_closed = true;
                        }
                    }
                    // 缓冲满时只 ACK 已收下的部分（不推进），对端会按自己的重传节奏再来。
                    let _ = send_ack(dev, conn, src_mac, next_id);
                } else if seg.flags & FLAG_FIN != 0 && seg.seq == conn.rcv_nxt {
                    // 对端在确认本块之前就半关闭：TCP 允许继续单向发送，收下 FIN 即可。
                    conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                    conn.peer_closed = true;
                    let _ = send_ack(dev, conn, src_mac, next_id);
                }
                if seg.flags & FLAG_ACK != 0
                    && seg.ack.wrapping_sub(conn.snd_nxt) >= chunk_len as u32
                {
                    conn.snd_una = seg.ack;
                    conn.snd_nxt = conn.snd_nxt.wrapping_add(chunk_len as u32);
                    sent += chunk_len;
                    acked = true;
                    break;
                }
                // 纯重复 ACK / 未覆盖本块 → 本轮剩余时间内继续等（不额外重传）
            }
        }
        data = rest;
    }
    Ok(sent)
}

/// 收最多 `out.len()` 字节（有已到达的字节立即返回；`Ok(0)` = 对端有序关闭）。
pub fn recv_upto(
    conn: &mut Conn,
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    next_id: &mut u16,
    out: &mut [u8],
) -> Result<usize, Error> {
    if out.is_empty() {
        return Ok(0);
    }
    // 1) 先交付 `send` 期间收下的字节（它们已经 ACK 过，不能再丢）。
    if conn.rx_len > 0 {
        return Ok(rx_pop(conn, out));
    }
    // 2) 对端已 FIN：EOF 是粘性的，重复 recv 仍返回 0，而不是重新等一个不会再来的 FIN。
    if conn.peer_closed {
        return Ok(0);
    }
    let start = ticks();
    loop {
        let elapsed = ticks().wrapping_sub(start);
        if elapsed > RECV_TIMEOUT_TICKS {
            return Err(Error::Timeout);
        }
        let mut tmp = [0u8; MSS];
        let Some(seg) = recv_match(
            dev,
            &mut tmp,
            RECV_TIMEOUT_TICKS.saturating_sub(elapsed),
            conn.local_ip,
            conn.local_port,
            conn.remote_ip,
            conn.remote_port,
        ) else {
            return Err(Error::Timeout);
        };
        if seg.flags & FLAG_RST != 0 {
            return Err(Error::Refused);
        }
        if seg.seq != conn.rcv_nxt {
            // 乱序/重复：重发当前 ACK（不推进），继续等正确的段
            let _ = send_ack(dev, conn, src_mac, next_id);
            continue;
        }
        if seg.len > 0 {
            // 整段先入 FIFO 再推进序号：`rx_push` 失败时绝不半收。
            if !rx_push(conn, &tmp[..seg.len]) {
                let _ = send_ack(dev, conn, src_mac, next_id);
                continue;
            }
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(seg.len as u32);
            if seg.flags & FLAG_FIN != 0 {
                conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
                conn.peer_closed = true;
            }
            send_ack(dev, conn, src_mac, next_id)?;
            return Ok(rx_pop(conn, out));
        }
        if seg.flags & FLAG_FIN != 0 {
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(1);
            conn.peer_closed = true;
            let _ = send_ack(dev, conn, src_mac, next_id);
            return Ok(0); // 有序关闭：读到 EOF
        }
        // 纯 ACK：继续等数据
    }
}

/// 关闭：发 `FIN|ACK`，等对端 ACK + FIN，再回最终 ACK。
pub fn close(
    conn: &mut Conn,
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    next_id: &mut u16,
) -> Result<(), Error> {
    send_segment(
        dev, src_mac, conn.remote_mac, conn.local_ip, conn.remote_ip, next_id, conn.local_port,
        conn.remote_port, conn.snd_nxt, conn.rcv_nxt, FLAG_FIN | FLAG_ACK, &[],
    )?;
    let fin_seq = conn.snd_nxt;
    conn.snd_nxt = conn.snd_nxt.wrapping_add(1);
    let start = ticks();
    let mut fin_acked = false;
    // 对端的 FIN 可能已经在 `send`/`recv` 期间收下（`peer_closed`）：
    // 那就只需要等自己的 FIN 被 ACK，不必再等一个不会重发的 FIN（否则白等 4 秒）。
    let mut fin_seen = conn.peer_closed;
    loop {
        if ticks().wrapping_sub(start) > CLOSE_TIMEOUT_TICKS {
            // FIN 已确认但对端 FIN 迟迟不来：半关闭亦可接受（对端进程已退出）
            if fin_acked {
                return Ok(());
            }
            return Err(Error::Timeout);
        }
        let mut tmp = [0u8; MSS];
        let remaining = CLOSE_TIMEOUT_TICKS.saturating_sub(ticks().wrapping_sub(start));
        let Some(seg) = recv_match(
            dev,
            &mut tmp,
            remaining,
            conn.local_ip,
            conn.local_port,
            conn.remote_ip,
            conn.remote_port,
        ) else {
            // 超时：FIN 未确认就重传一次 FIN（对端可能丢了它）
            if !fin_acked {
                let _ = send_segment(
                    dev, src_mac, conn.remote_mac, conn.local_ip, conn.remote_ip, next_id,
                    conn.local_port, conn.remote_port, fin_seq, conn.rcv_nxt,
                    FLAG_FIN | FLAG_ACK, &[],
                );
            }
            continue;
        };
        if seg.flags & FLAG_RST != 0 {
            return Ok(()); // 对端已重置：连接无残留，关闭即完成
        }
        if seg.flags & FLAG_ACK != 0 && seg.ack.wrapping_sub(fin_seq) >= 1 {
            fin_acked = true;
        }
        if seg.flags & FLAG_FIN != 0 && seg.seq == conn.rcv_nxt {
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(seg.len as u32).wrapping_add(1);
            let _ = send_ack(dev, conn, src_mac, next_id);
            fin_seen = true;
        } else if seg.len > 0 && seg.seq == conn.rcv_nxt {
            // 关闭 race 里对端最后的数据：收下并 ACK（调用方不再读，序号推进即可）
            conn.rcv_nxt = conn.rcv_nxt.wrapping_add(seg.len as u32);
            let _ = send_ack(dev, conn, src_mac, next_id);
        }
        if fin_acked && fin_seen {
            return Ok(());
        }
    }
}

/// TCP 回声验收：`connect → send → recv(逐字节比对) → close` 全走真实网络。
///
/// `server` 是 slirp 后面的主机回声服务（测试侧在宿主起，见 `tests/test_boot.py`
/// 的 `--tcp-echo-port`）；`payload` 是发送与期望收到的字节。断言的都是
/// 网络上回来的真实内容（SYN-ACK 的确认号、回声字节、FIN 交换），不是日志措辞。
pub fn echo(
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    src_ip: ipv4::Address,
    next_id: &mut u16,
    server: ipv4::Address,
    port: u16,
    payload: &[u8],
    local_port: u16,
) -> Result<(), Error> {
    let mut conn = connect(dev, src_mac, src_ip, next_id, server, port, local_port)?;
    serial::print("net: tcp connect ");
    crate::network::print_addr(server);
    serial::print(":");
    serial::print_dec(port as u64);
    serial::println(" ESTABLISHED");

    send(&mut conn, dev, src_mac, next_id, payload)?;
    serial::print("net: tcp send len=");
    serial::print_dec(payload.len() as u64);
    serial::println(" acked");

    // 回声可能分段回来：收到与发出等长且逐字节一致才算数
    let mut got = [0u8; MSS];
    let want = payload.len().min(got.len());
    let mut filled = 0;
    while filled < want {
        let n = recv_upto(&mut conn, dev, src_mac, next_id, &mut got[filled..want])?;
        if n == 0 {
            break; // 对端提前关闭：按实际收到比对（长度不同即失败）
        }
        filled += n;
    }
    if filled != want || got[..want] != payload[..want] {
        return Err(Error::Timeout); // 内容/长度对不上：与「没收到」同等失败
    }
    serial::print("net: tcp recv len=");
    serial::print_dec(filled as u64);
    serial::println(" match=PASS");

    close(&mut conn, dev, src_mac, next_id)?;
    serial::println("net: tcp close FIN-ACK PASS");
    serial::println("net: tcp PASS");
    Ok(())
}
