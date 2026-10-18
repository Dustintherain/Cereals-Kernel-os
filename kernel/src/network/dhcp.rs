//! DHCP 客户端最小集（v0.9 实现）
//!
//! 为什么用 DHCP 做 UDP 的验收：QEMU 的 `-netdev user`（slirp）自带一个 DHCP
//! 服务器，**不需要主机侧跑任何服务**就能收到一个真实的 UDP 应答 —— 一次
//! `Ethernet → IPv4 → UDP` 的完整往返，而不是自己发给自己。报文本身是标准的
//! BOOTP + DHCP options，因此「收到了应答」等价于「整条接收路径的解析都对」。
//!
//! 只实现 **DISCOVER → OFFER** 这一步：拿到 `yiaddr`（offer 给的地址）与
//! server-id 就够了，不实现 REQUEST/ACK、不续租（v0.9 的边界：能用、可验证）。
//!
//! ```text
//! BOOTP 头（236 字节）：
//! 0   1  op（1=request / 2=reply）
//! 1   1  htype（1=以太网）      2  1  hlen（6）
//! 3   1  hops                   4  4  xid
//! 8   2  secs                   10 2  flags（bit15=1 请求广播应答）
//! 12  4  ciaddr                 16 4  yiaddr（offer 里是分配给我的地址）
//! 20  4  siaddr                 24 4  giaddr
//! 28  16 chaddr（客户端 MAC + 10 个 0）
//! 44  64 sname                  108 128 file
//! 236 4  magic cookie = 63 82 53 63
//! 240 ... options（TLV），255 = END
//! ```

use crate::network::ipv4::Address;

/// DHCP 服务器端口。
pub const SERVER_PORT: u16 = 67;
/// DHCP 客户端端口。
pub const CLIENT_PORT: u16 = 68;

/// BOOTP 头长度。
const BOOTP_LEN: usize = 236;
/// magic cookie 长度。
const COOKIE_LEN: usize = 4;
/// 选项区起始偏移。
const OPTIONS_OFF: usize = BOOTP_LEN + COOKIE_LEN;
/// DHCP 报文最小长度（RFC 2131：至少 300 字节）。
pub const MESSAGE_LEN: usize = 300;

/// magic cookie，标识后面的字节是 DHCP options。
const MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

/// op：BOOTREQUEST。
const OP_REQUEST: u8 = 1;
/// op：BOOTREPLY。
const OP_REPLY: u8 = 2;
/// htype：以太网。
const HTYPE_ETHERNET: u8 = 1;

// ---- option 码 ----
const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_MSG_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_END: u8 = 255;

/// DHCP 消息类型：DISCOVER。
const MSG_DISCOVER: u8 = 1;
/// DHCP 消息类型：OFFER。
const MSG_OFFER: u8 = 2;

/// 本客户端使用的 xid（固定值即可：一次启动只发一个 DISCOVER）。
const XID: u32 = 0x4345_524C; // "CERL"

/// 拼一个 DHCP DISCOVER，写入 `buf`，返回写入的字节数（= [`MESSAGE_LEN`]）。
///
/// 固定请求广播应答（flags bit15）：OFFER 会以广播帧回来，网卡一定能收到。
pub fn build_discover(buf: &mut [u8], mac: [u8; 6]) -> usize {
    // 下面的写入固定在 0..248 范围内（BOOTP 头 + cookie + 选项）；缓冲比最小报文
    // 还短时不能“写完再说”，直接返回 0 让调用方报错。
    if buf.len() < MESSAGE_LEN {
        return 0;
    }
    let len = MESSAGE_LEN;
    buf[..len].fill(0);
    buf[0] = OP_REQUEST;
    buf[1] = HTYPE_ETHERNET;
    buf[2] = 6; // hlen
    buf[3] = 0; // hops
    buf[4..8].copy_from_slice(&XID.to_be_bytes());
    // secs = 0
    buf[10..12].copy_from_slice(&0x8000u16.to_be_bytes()); // 请求广播应答
    // ciaddr/yiaddr/siaddr/giaddr 全 0，chaddr 从偏移 28 起
    if len >= 34 {
        buf[28..34].copy_from_slice(&mac);
    }
    buf[BOOTP_LEN..BOOTP_LEN + COOKIE_LEN].copy_from_slice(&MAGIC_COOKIE);

    let mut off = OPTIONS_OFF;
    // 53 / 1 / DISCOVER
    buf[off] = OPT_MSG_TYPE;
    buf[off + 1] = 1;
    buf[off + 2] = MSG_DISCOVER;
    off += 3;
    // 55 / 4 / 请求的参数列表：subnet mask, router, DNS, server-id
    buf[off] = 55;
    buf[off + 1] = 4;
    buf[off + 2] = OPT_SUBNET_MASK;
    buf[off + 3] = OPT_ROUTER;
    buf[off + 4] = 6; // domain-name-server
    buf[off + 5] = OPT_SERVER_ID;
    off += 6;
    // END
    buf[off] = OPT_END;
    off += 1;
    // 其余字节保持 0（填充到最小报文长度）
    let _ = off;
    len
}

/// 解析出的 DHCP OFFER。
#[derive(Clone, Copy, Debug)]
pub struct Offer {
    /// offer 分配的地址（BOOTP 的 `yiaddr`）
    pub yiaddr: Address,
    /// 服务器标识（option 54）
    pub server_id: Option<Address>,
    /// 子网掩码（option 1）
    pub subnet_mask: Option<Address>,
    /// 路由器/网关（option 3，取第一个）
    pub router: Option<Address>,
}

/// 解析一个 DHCP 应答；不是 OFFER 时返回 `None`。
pub fn parse_offer(payload: &[u8]) -> Option<Offer> {
    if payload.len() < OPTIONS_OFF {
        return None;
    }
    if payload[0] != OP_REPLY || payload[1] != HTYPE_ETHERNET {
        return None;
    }
    if payload[BOOTP_LEN..OPTIONS_OFF] != MAGIC_COOKIE {
        return None;
    }
    let yiaddr = Address::from_octets([
        payload[16],
        payload[17],
        payload[18],
        payload[19],
    ]);

    let mut offer = Offer {
        yiaddr,
        server_id: None,
        subnet_mask: None,
        router: None,
    };
    let mut is_offer = false;

    let mut i = OPTIONS_OFF;
    while i < payload.len() {
        let code = payload[i];
        if code == OPT_END {
            break;
        }
        if code == 0 {
            i += 1; // PAD
            continue;
        }
        if i + 1 >= payload.len() {
            break;
        }
        let len = payload[i + 1] as usize;
        let start = i + 2;
        let end = start.checked_add(len)?;
        if end > payload.len() {
            break;
        }
        let value = &payload[start..end];
        match code {
            OPT_MSG_TYPE if len == 1 => is_offer = value[0] == MSG_OFFER,
            OPT_SERVER_ID if len == 4 => {
                offer.server_id = Some(Address::from_octets([
                    value[0], value[1], value[2], value[3],
                ]));
            }
            OPT_SUBNET_MASK if len == 4 => {
                offer.subnet_mask = Some(Address::from_octets([
                    value[0], value[1], value[2], value[3],
                ]));
            }
            OPT_ROUTER if len >= 4 => {
                offer.router = Some(Address::from_octets([
                    value[0], value[1], value[2], value[3],
                ]));
            }
            _ => {}
        }
        i = end;
    }

    if is_offer {
        Some(offer)
    } else {
        None
    }
}
