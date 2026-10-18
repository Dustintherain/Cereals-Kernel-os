//! tcpecho —— socket/TCP 用户态验收程序（RAMFS 上的 `/bin/tcpecho`）
//!
//! 只在有网卡时由内核自测运行（`kernel/src/syscall/user.rs::selftest`
//! 检查 `driver::net::count()`，无网卡时跳过）：
//!
//! ```text
//! socket → connect(10.0.2.2:18080) → sendto → recvfrom → write → read → close
//! ```
//!
//! 两轮收发走**两套** syscall 形状：第一轮 `sendto`/`recvfrom`（号 44/45），
//! 第二轮 `write`/`read`（号 1/0 落到套接字 fd 的路由）——
//! 两套都通才说明「套接字 fd」真的融进了文件 fd 的读写路径，而不只是新号可用。
//! 第三轮发 3000 字节（> MSS，内核必须切 3 段）却用 512 字节小缓冲逐段读：
//! 回声可能在我们仍在发送时就回来，专门验证「未交付字节不被 ACK 吞掉」。
//! 回声服务由测试侧在宿主起（`tests/test_boot.py` 的 `--tcp-echo-port`），
//! 发什么回什么；三轮都逐字节比对，成功打印 `tcpecho: PASS` 并 `exit(0)`。

#![no_std]
#![no_main]

use libc::{
    close, connect, println, read, recvfrom, sendto, sockaddr_in, socket, write, AF_INET,
    IPPROTO_TCP, SOCK_STREAM,
};

/// 回声服务地址（slirp 网关即宿主，见 `kernel/src/network/mod.rs::TCP_ECHO_SERVER`）。
const SERVER: [u8; 4] = [10, 0, 2, 2];
/// 回声服务端口（与内核自测用同一个，测试侧在宿主 `0.0.0.0:18080` 起服务）。
const PORT: u16 = 18080;
/// 第一轮载荷（`sendto` 发、`recvfrom` 收）。
const PAYLOAD_A: &[u8] = b"tcpecho-user";
/// 第二轮载荷（`write` 发、`read` 收：同一连接上的第二段数据）。
const PAYLOAD_B: &[u8] = b"rw";
/// 第三轮载荷长度（> 2×MSS：内核要切三段；读侧只用 [`LARGE_CHUNK`] 字节缓冲）。
const LARGE_LEN: usize = 3000;
/// 第三轮的读缓冲大小（远小于 MSS：旧实现会把剩余字节连同 ACK 一起丢掉）。
const LARGE_CHUNK: usize = 512;

/// 发 `payload` 并收回等长字节（收不满/对不上即失败；`use_write` 切第二套形状）。
fn round_trip(fd: i32, payload: &[u8], use_write: bool) -> bool {
    if use_write {
        if write(fd, payload) != payload.len() as isize {
            println!("tcpecho: write failed");
            return false;
        }
    } else if sendto(fd, payload) != payload.len() as isize {
        println!("tcpecho: send failed");
        return false;
    }
    let mut got = [0u8; 64];
    let mut filled = 0;
    while filled < payload.len() {
        let n = if use_write {
            read(fd, &mut got[filled..payload.len()])
        } else {
            recvfrom(fd, &mut got[filled..payload.len()])
        };
        if n <= 0 {
            println!("tcpecho: recv failed n={}", n);
            return false;
        }
        filled += n as usize;
    }
    if got[..payload.len()] != payload[..] {
        println!("tcpecho: mismatch");
        return false;
    }
    true
}

/// 第三轮：一次 `sendto` 超 MSS 的载荷，再用 512 字节小缓冲分批 `read` 回来。
fn round_large(fd: i32) -> bool {
    let mut payload = [0u8; LARGE_LEN];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = b'a' + (i % 26) as u8;
    }
    let n = sendto(fd, &payload);
    if n != LARGE_LEN as isize {
        println!("tcpecho: large send failed n={}", n);
        return false;
    }
    let mut got = [0u8; LARGE_LEN];
    let mut filled = 0;
    while filled < LARGE_LEN {
        let want = (LARGE_LEN - filled).min(LARGE_CHUNK);
        let n = read(fd, &mut got[filled..filled + want]);
        if n <= 0 {
            println!("tcpecho: large recv failed n={}", n);
            return false;
        }
        filled += n as usize;
    }
    if got != payload {
        println!("tcpecho: large mismatch");
        return false;
    }
    true
}

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if fd < 0 {
        println!("tcpecho: socket failed errno={}", fd);
        return 1;
    }
    println!("tcpecho: socket fd={}", fd);

    let addr = sockaddr_in(SERVER, PORT);
    let rc = connect(fd as i32, &addr);
    if rc != 0 {
        println!("tcpecho: connect failed errno={}", rc);
        let _ = close(fd as i32);
        return 1;
    }
    println!("tcpecho: connected 10.0.2.2:18080");

    if !round_trip(fd as i32, PAYLOAD_A, false) {
        let _ = close(fd as i32);
        return 1;
    }
    println!("tcpecho: round 1 sendto/recvfrom PASS len={}", PAYLOAD_A.len());

    if !round_trip(fd as i32, PAYLOAD_B, true) {
        let _ = close(fd as i32);
        return 1;
    }
    println!("tcpecho: round 2 write/read PASS len={}", PAYLOAD_B.len());

    if !round_large(fd as i32) {
        let _ = close(fd as i32);
        return 1;
    }
    println!("tcpecho: round 3 large payload PASS len={}", LARGE_LEN);

    let rc = close(fd as i32);
    if rc != 0 {
        println!("tcpecho: close failed errno={}", rc);
        return 1;
    }
    println!("tcpecho: PASS");
    0
}
