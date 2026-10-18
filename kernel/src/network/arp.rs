//! ARP（IPv4 over Ethernet，v0.9 实现）
//!
//! 只做 RFC 826 里「以太网 + IPv4」这一种组合（`htype=1`、`ptype=0x0800`、
//! `hlen=6`、`plen=4`），其余组合明确拒绝而不是猜。
//!
//! ```text
//! 偏移  长度  字段
//! 0     2    htype = 1（以太网）
//! 2     2    ptype = 0x0800（IPv4）
//! 4     1    hlen = 6
//! 5     1    plen = 4
//! 6     2    oper（1 = request，2 = reply）
//! 8     6    sender MAC
//! 14    4    sender IP
//! 18    6    target MAC（request 里全 0）
//! 24    4    target IP
//! ```
//!
//! 提供一张固定容量的 **IP → MAC 缓存**（[`Cache`]）：ARP 解析一次即可复用，
//! 缓存满了按「最久未使用」覆盖 —— v0.9 只有网关一个目标，命中率不是问题，
//! 但把「解析一次就记住」这件事写进接口，后面的 UDP 发送路径才不必每次重新 ARP。

use crate::driver::net::{self, NetworkDevice};
use crate::driver::serial;
use crate::network::ethernet;
use crate::network::ipv4::Address;
use crate::time::clock;

/// ARP 报文长度（以太网 + IPv4）。
pub const PACKET_LEN: usize = 28;

/// 操作码：请求。
pub const OPER_REQUEST: u16 = 1;
/// 操作码：应答。
pub const OPER_REPLY: u16 = 2;

/// 解析出的 ARP 报文。
#[derive(Clone, Copy, Debug)]
pub struct ArpPacket {
    /// 操作码（[`OPER_REQUEST`] / [`OPER_REPLY`]）
    pub oper: u16,
    /// 发送方 MAC
    pub sender_mac: [u8; 6],
    /// 发送方 IP
    pub sender_ip: Address,
    /// 目标 MAC
    pub target_mac: [u8; 6],
    /// 目标 IP
    pub target_ip: Address,
}

/// 写一个 ARP 报文（28 字节）。`target_mac` 在 request 里传 [`ZERO_MAC`]。
pub fn build(
    buf: &mut [u8; PACKET_LEN],
    oper: u16,
    sender_mac: [u8; 6],
    sender_ip: Address,
    target_mac: [u8; 6],
    target_ip: Address,
) {
    buf[0..2].copy_from_slice(&1u16.to_be_bytes()); // htype = Ethernet
    buf[2..4].copy_from_slice(&0x0800u16.to_be_bytes()); // ptype = IPv4
    buf[4] = 6; // hlen
    buf[5] = 4; // plen
    buf[6..8].copy_from_slice(&oper.to_be_bytes());
    buf[8..14].copy_from_slice(&sender_mac);
    buf[14..18].copy_from_slice(&sender_ip.octets());
    buf[18..24].copy_from_slice(&target_mac);
    buf[24..28].copy_from_slice(&target_ip.octets());
}

/// 解析一个 ARP 报文；不是「以太网 + IPv4」组合时返回 `None`。
pub fn parse(buf: &[u8]) -> Option<ArpPacket> {
    if buf.len() < PACKET_LEN {
        return None;
    }
    let htype = u16::from_be_bytes([buf[0], buf[1]]);
    let ptype = u16::from_be_bytes([buf[2], buf[3]]);
    if htype != 1 || ptype != 0x0800 || buf[4] != 6 || buf[5] != 4 {
        return None;
    }
    let mut sender_mac = [0u8; 6];
    let mut target_mac = [0u8; 6];
    sender_mac.copy_from_slice(&buf[8..14]);
    target_mac.copy_from_slice(&buf[18..24]);
    Some(ArpPacket {
        oper: u16::from_be_bytes([buf[6], buf[7]]),
        sender_mac,
        sender_ip: Address::from_octets([buf[14], buf[15], buf[16], buf[17]]),
        target_mac,
        target_ip: Address::from_octets([buf[24], buf[25], buf[26], buf[27]]),
    })
}

/// 固定容量的 IP → MAC 缓存（线性探测，容量小到不值得上哈希表）。
pub struct Cache {
    entries: [Option<(Address, [u8; 6])>; Cache::CAPACITY],
    /// 下一个要覆盖的槽位（简单的轮转替换）。
    next: usize,
}

impl Cache {
    /// 缓存容量（v0.9 只解析网关，4 条足够）。
    pub const CAPACITY: usize = 4;

    /// 空缓存（`const` 构造，可直接放静态量里）。
    pub const fn new() -> Self {
        Self {
            entries: [None; Self::CAPACITY],
            next: 0,
        }
    }

    /// 查表。
    pub fn lookup(&self, ip: Address) -> Option<[u8; 6]> {
        self.entries
            .iter()
            .flatten()
            .find(|(cached, _)| *cached == ip)
            .map(|(_, mac)| *mac)
    }

    /// 插入/更新一条。
    pub fn insert(&mut self, ip: Address, mac: [u8; 6]) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .flatten()
            .find(|(cached, _)| *cached == ip)
        {
            entry.1 = mac;
            return;
        }
        let slot = self.next % Self::CAPACITY;
        self.entries[slot] = Some((ip, mac));
        self.next = (slot + 1) % Self::CAPACITY;
    }

}

/// 读取 PIT tick（`clock::tick_count` 是 unsafe：调用方须处于内核态）。
fn ticks() -> u64 {
    // SAFETY: ARP 只在 ring0（启动自测、TCP 连接、syscall handler）里运行。
    unsafe { clock::tick_count() }
}

/// 发一次 ARP 请求并等应答（无缓存的单次解析）。
///
/// [`Cache`] 命中之后调的就是它 —— [`crate::network::Stack::arp_resolve`]
/// 先查缓存、未命中才到这里；TCP 连接（[`crate::network::tcp`]）与 socket 层
/// 手里只有 `&mut dyn NetworkDevice` 而没有 `Stack`，因此等待循环只有这一份实现。
///
/// 应答的合法性检查与原来 `Stack::arp_resolve` 里的一致：只收操作码为应答、
/// 发送方为 `target`、目的 IP/MAC 都是本机的报文（以太网目的为本机或广播）。
pub fn resolve(
    dev: &mut dyn NetworkDevice,
    src_mac: [u8; 6],
    src_ip: Address,
    target: Address,
    timeout_ticks: u64,
) -> Option<[u8; 6]> {
    let mut frame = [0u8; ethernet::HEADER_LEN + PACKET_LEN];
    let mut arp_bytes = [0u8; PACKET_LEN];
    build(
        &mut arp_bytes,
        OPER_REQUEST,
        src_mac,
        src_ip,
        ethernet::ZERO_MAC,
        target,
    );
    {
        let payload =
            ethernet::build(&mut frame, ethernet::BROADCAST_MAC, src_mac, ethernet::ETHERTYPE_ARP)?;
        payload[..PACKET_LEN].copy_from_slice(&arp_bytes);
    }
    dev.transmit(&frame).ok()?;

    let mut rx = [0u8; net::FRAME_MAX];
    let start = ticks();
    loop {
        if ticks().wrapping_sub(start) > timeout_ticks {
            return None;
        }
        let n = match dev.receive(&mut rx) {
            Ok(Some(n)) => n,
            Ok(None) => continue,
            Err(_) => return None,
        };
        let Some(eth) = ethernet::parse(&rx[..n]) else {
            continue;
        };
        if eth.ethertype != ethernet::ETHERTYPE_ARP {
            continue;
        }
        let Some(packet) = parse(eth.payload) else {
            continue;
        };
        if packet.oper != OPER_REPLY || packet.sender_ip != target {
            continue;
        }
        // 应答必须发给我们：以太网目的为本机（或广播），且 ARP 的目的 IP / MAC 都是本机。
        if !(eth.dst == src_mac || ethernet::is_broadcast(eth.dst)) {
            continue;
        }
        if packet.target_ip != src_ip || packet.target_mac != src_mac {
            continue;
        }
        // 交叉校验：帧头源 MAC 应与 ARP 报文的发送方 MAC 一致（不一致只提示，以 ARP 为准）。
        if eth.src != packet.sender_mac {
            serial::println("net: arp note: frame src != arp sender mac");
        }
        return Some(packet.sender_mac);
    }
}
