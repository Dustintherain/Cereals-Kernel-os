//! 串口驱动：16550 UART @ COM1（v0.1 实现）
//!

const COM1: u16 = 0x3F8;

#[inline]
unsafe fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") val,
            options(nomem, nostack, preserves_flags)
        );
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") val,
            options(nomem, nostack, preserves_flags)
        );
    }
    val
}

/// 初始化 COM1：38400 波特、8N1、启用 FIFO，串口中断保持关闭。
///
/// 当前阶段串口是唯一日志通道，且中断上下文中不复用串口打印，
/// 因此串口中断保持关闭可避免串口 IRQ 干扰 PIT IRQ0 调试。
/// 此外，不允许串口中断在输出过程中被意外重新使能。
pub fn init() {
    unsafe {
        outb(COM1 + 1, 0x00); // 关闭中断
        outb(COM1 + 3, 0x80); // DLAB=1
        outb(COM1 + 0, 0x03); // 波特率除数低位（38400）
        outb(COM1 + 1, 0x00); // 除数高位
        outb(COM1 + 3, 0x03); // 8N1，DLAB=0
        outb(COM1 + 2, 0xC7); // 启用 FIFO，清空，14 字节阈值
        outb(COM1 + 4, 0x0B); // DTR|RTS|OUT2
        // 再次确保串口中断关闭（防止后续操作意外使能）
        outb(COM1 + 1, 0x00);
    }
}

/// 当前是否有一行日志还没输出完（`LINE_OPEN`）。
///
/// 用途：IRQ0 的心跳只在一行的边界上输出。一条日志由多次 `print` 拼成
///（`print("user: program=")` + `print(path)` + …），如果心跳在中间插入，
/// 串口上就会出现 `user: prIRQ0_heartbeat…ogram=/init` —— 验收脚本断言的子串
/// 被劈成两半（这是实测到过的 flake）。
static mut LINE_OPEN: bool = false;

/// 是否处于一行日志的中间（心跳/异步输出应让路）。
pub fn line_open() -> bool {
    unsafe { *core::ptr::addr_of!(LINE_OPEN) }
}

/// 根据刚写出的字节更新行状态（`\n` 结束一行，`\r` 不改变状态）。
fn mark_line(c: u8) {
    unsafe {
        if c == b'\n' {
            *core::ptr::addr_of_mut!(LINE_OPEN) = false;
        } else if c != b'\r' {
            *core::ptr::addr_of_mut!(LINE_OPEN) = true;
        }
    }
}

/// 输出单个字节（供需要按字节回显的驱动使用，如键盘回显）。
pub fn putc(c: u8) {
    unsafe {
        // 等待发送保持寄存器空
        while inb(COM1 + 5) & 0x20 == 0 {}
        outb(COM1, c);
    }
    mark_line(c);
}

/// 批量输出 `buf` 的前 `count` 字节，返回实际写出的字节数。
/// 供 `syscall::file::sys_write` 等需要按长度写出的调用方复用。
pub fn put_bytes(buf: &[u8], count: usize) -> usize {
    let mut written = 0;
    for b in buf.iter().take(count) {
        putc(*b);
        written += 1;
    }
    written
}

pub fn print(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

pub fn println(s: &str) {
    print(s);
    putc(b'\r');
    putc(b'\n');
}

pub fn print_dec(val: u64) {
    if val == 0 {
        putc(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0;
    let mut v = val;
    while v > 0 {
        buf[len] = b'0' + (v % 10) as u8;
        v /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        putc(buf[len]);
    }
}

/// 输出固定位数的十六进制（**无 `0x` 前缀**，高位补 0）。
///
/// 用于把 PCI 的 bus/dev/func 打成人类可读的 `00:05.0` 形式：
/// `print_hex_pad(bus, 2)` / `print_hex_pad(dev, 2)`。`digits` 最大 16
/// （超过时按 16 处理，与 `u64` 位宽一致）。
pub fn print_hex_pad(val: u64, digits: usize) {
    let digits = digits.clamp(1, 16);
    for i in (0..digits).rev() {
        let nib = ((val >> (i * 4)) & 0xF) as u8;
        putc(if nib < 10 { b'0' + nib } else { b'a' + nib - 10 });
    }
}

pub fn print_hex(val: u64) {
    print("0x");
    let mut started = false;
    for i in (0..16).rev() {
        let nib = ((val >> (i * 4)) & 0xF) as u8;
        if nib != 0 {
            started = true;
        }
        if started {
            putc(if nib < 10 { b'0' + nib } else { b'a' + nib - 10 });
        }
    }
    if !started {
        putc(b'0');
    }
}
