//! Socket 域 / 类型 / 协议与 `sockaddr_in` 布局（TCP/socket 补齐）。
//!
//! 值沿用 Linux x86_64（`AF_INET=2`、`SOCK_STREAM=1`、`socket=41` …
//! 见 [`crate::syscall`]），记录布局只有一个权威定义（阶段三单一事实源）。
//!
//! ```text
//! sockaddr_in（16 字节）：
//! 偏移  长度  字段        说明
//! 0     2     family      LE u16，必须 AF_INET（x86 小端存 0x0002 即 `02 00`）
//! 2     2     port        BE u16（网络字节序，如 18080 = 0x46A0 存 `46 A0`）
//! 4     4     addr        IPv4 四字节（如 10.0.2.2 存 `0A 00 02 02`）
//! 8     8     zero        必须为 0（内核不校验内容，只要求长度）
//! ```
//!
//! 只实现 **IPv4 字节流（TCP）**：`SOCK_DGRAM` / `IPPROTO_UDP` 在这里具名，
//! 是为了让「传了 UDP 参数」返回明确的 `-EAFNOSUPPORT`，而不是 `-EINVAL` ——
//! 调用方能区分「参数写错」与「本内核不支持 UDP 套接字」。

/// 地址族：IPv4（本内核唯一支持的族）。
pub const AF_INET: u16 = 2;
/// 套接字类型：字节流（TCP）。
pub const SOCK_STREAM: i32 = 1;
/// 套接字类型：数据报（已命名、明确拒绝：UDP 套接字未实现）。
pub const SOCK_DGRAM: i32 = 2;
/// 协议：TCP。
pub const IPPROTO_TCP: i32 = 6;
/// 协议：UDP（已命名、明确拒绝）。
pub const IPPROTO_UDP: i32 = 17;

/// `sockaddr_in` 长度：`connect` / `bind` / `sendto` 的 `addr_len` 必须等于它。
pub const SOCKADDR_IN_LEN: usize = 16;
/// `family` 字段偏移（LE u16）。
pub const FAMILY_OFFSET: usize = 0;
/// `port` 字段偏移（BE u16）。
pub const PORT_OFFSET: usize = 2;
/// `addr` 字段偏移（4 字节 IPv4）。
pub const ADDR_OFFSET: usize = 4;
