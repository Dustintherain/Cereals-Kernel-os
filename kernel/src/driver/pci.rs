//! PCI 枚举与设备匹配（v0.8 实现）
//!
//! ## 为什么用 `0xCF8`/`0xCFC`（配置机制 #1）
//!
//! x86 上有两条访问 PCI 配置空间的路径：
//!
//! | 方式 | 地址来源 | 代价 |
//! | ---- | ---- | ---- |
//! | 机制 #1（本模块） | I/O 端口 `0xCF8`（地址）/ `0xCFC`（数据） | 每次访问两条端口指令，全平台可用 |
//! | 机制 #2 / ECAM（MCFG） | ACPI 表里给出的 MMIO 基址 | 需要解析 ACPI（v0.9+），访问更快 |
//!
//! v0.8 选机制 #1：它不依赖 ACPI，QEMU（q35）与真实机器都支持，
//! 而且**枚举本身**就是要被验证的行为。ECAM 化留给解析 ACPI 时的性能优化。
//!
//! ## 编号空间与「一个驱动匹配一个设备」
//!
//! 配置地址里 bus/dev/func 各占 8/5/3 位，因此完整空间是 256 × 32 × 8。
//! 枚举时：
//!
//! - `vendor == 0xFFFF` 表示该 function 不存在（唯一可靠的「设备在不在」判据）；
//! - 只在 function 0 上读 `header_type`，其 bit7（`MULTI_FUNCTION`）为 0 时该设备的
//!   function 1..8 一定不存在 —— 这一条把 256 个 function 的探测砍到 1 个；
//! - 枚举**全总线 0..=255**：桥后面的设备（v0.8 的 QEMU 里都在 bus 0）不会漏掉，
//!   代价是约 8k 次端口读（实测启动日志里 multi 的时间可忽略）。
//!
//! ## BAR 探测
//!
//! `config_write32(bar, 0xFFFF_FFFF)` → 读回的掩码取反 +1 就是窗口大小（这是 PCI
//! 规范定义的「用全 1 写允许探测」的机制）。探测完必须把原值写回，否则设备地址
//! 窗口会在枚举后被改掉。64 位 MMIO BAR（`type == 0b10`）占两个 BAR 槽位。
//!
//! ## 已知限制（写在文档里，不藏着）
//!
//! - 不分配/重定位 BAR：直接用固件分配的地址（QEMU 下实测可用；真实平台若出现
//!   未分配窗口（全 0）会打日志而不是猜测）；
//! - 不解析 capability 链（virtio 的 MSI-X / modern 能力结构在 v0.9 用）；
//! - 不配置 IRQ（v0.8 的块设备走轮询，见 `virtio_blk`），因此没有 `enable_intx`。

use alloc::vec::Vec;

use crate::arch::x86_64::port;
use crate::driver::serial;

/// 配置地址端口（机制 #1）
const CONFIG_ADDRESS: u16 = 0x0CF8;
/// 配置数据端口（机制 #1）
const CONFIG_DATA: u16 = 0x0CFC;

/// PCI 配置空间里用到的偏移
const OFFSET_VENDOR: u8 = 0x00;
const OFFSET_COMMAND: u8 = 0x04; // command(16) | status(16)
const OFFSET_CLASS: u8 = 0x08; // revision(8) | prog_if(8) | subclass(8) | class(8)
/// 头部类型：**0x0E**。`0x0C` 是 cache line size，读它恒为 0 → 多功能设备
/// 的 function 1..7 永远不会被探测（B-18）。
const OFFSET_HEADER_TYPE: u8 = 0x0E;
const OFFSET_BAR0: u8 = 0x10;
/// BAR 槽位数（`0x10..0x28`）
pub const BAR_COUNT: usize = 6;

/// 不存在的 function 读回 `0xFFFF`
const NO_DEVICE: u16 = 0xFFFF;

/// `header_type` 的 `MULTI_FUNCTION` 位
const HEADER_MULTI_FUNCTION: u8 = 0x80;

/// 命令寄存器位：响应内存空间访问（MMIO BAR 必须先打开它）
pub const COMMAND_MEMORY_SPACE: u16 = 1 << 1;
/// 命令寄存器位：总线主控（设备可以发起 DMA）
pub const COMMAND_BUS_MASTER: u16 = 1 << 2;

/// 一个 PCI function。
#[derive(Clone, Copy, Debug)]
pub struct PciDevice {
    /// 总线号（0..=255）
    pub bus: u8,
    /// 设备号（0..=31）
    pub device: u8,
    /// 功能号（0..=7）
    pub function: u8,
    /// 厂商 ID（`0xFFFF` 表示不存在）
    pub vendor: u16,
    /// 设备 ID
    pub device_id: u16,
    /// 类别码（0x01 = 大容量存储）
    pub class: u8,
    /// 子类别码（0x00 = SCSI/其他，0x06 = SATA/AHCI，0x80 = 其他）
    pub subclass: u8,
    /// 编程接口（0x01 = AHCI 1.0）
    pub prog_if: u8,
    /// 头部类型（bit7 = 多功能）
    pub header_type: u8,
    /// 已探测的 BAR（`None` 表示该槽位未实现）
    pub bars: [Option<Bar>; BAR_COUNT],
}

impl PciDevice {
    /// 设备位置（`bus, device, function`；日志里用 [`print_bdf`] 逐段输出）。
    pub fn location(&self) -> (u8, u8, u8) {
        (self.bus, self.device, self.function)
    }

    /// 是否为多功能设备的 function 0（此时需要继续探测 function 1..8）
    pub fn is_multi_function(&self) -> bool {
        self.header_type & HEADER_MULTI_FUNCTION != 0
    }

    /// 第 `index` 个 BAR。
    pub fn bar(&self, index: usize) -> Option<Bar> {
        self.bars.get(index).copied().flatten()
    }

    /// 读命令寄存器（低 16 位；高 16 位是只读的状态寄存器，不在这里返回）。
    pub fn command(&self) -> u16 {
        let (bus, device, function) = self.location();
        // SAFETY: 端口 I/O 在 ring0 使用；这里只读配置空间。
        (unsafe { config_read32(bus, device, function, OFFSET_COMMAND) } & 0xFFFF) as u16
    }

    /// 置位命令寄存器里的 `bits`，返回**旧值**。
    ///
    /// 为什么是「读改写」而不是写一个 16 位字：命令寄存器的高 16 位是状态寄存器，
    /// 用 `write32` 回写整字时保留它，比再引一次 `config_write16` 更不容易踩到
    /// 「写状态位」的边界（状态位本身是只读的，但保留原值最省心）。
    pub fn enable_command(&self, bits: u16) -> u16 {
        let (bus, device, function) = self.location();
        // SAFETY: 同 `command`；写命令寄存器是允许的（规范定义的可写寄存器）。
        let old = unsafe { config_read32(bus, device, function, OFFSET_COMMAND) };
        let command = (old as u16) | bits;
        unsafe {
            config_write32(
                bus,
                device,
                function,
                OFFSET_COMMAND,
                (old & 0xFFFF_0000) | command as u32,
            )
        };
        (old & 0xFFFF) as u16
    }
}

/// 一个已探测的 BAR 窗口。
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    /// 槽位下标（0..6）
    pub index: usize,
    /// 是否为 I/O 窗口（否则是 MMIO）
    pub io: bool,
    /// 是否 64 位 MMIO（占两个槽位）
    pub is_64bit: bool,
    /// 基址（已按窗口类型对齐；I/O 窗口已去掉低 2 位的类型标志）
    pub address: u64,
    /// 窗口大小（字节；探测失败为 0）
    pub size: u64,
}

// ---- 配置空间访问（机制 #1） ----

/// 拼一个配置地址：bit31 = enable，位段 bus/dev/func/offset。
fn config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((function & 0x07) as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// 读 32 位配置寄存器。
///
/// # Safety
/// 端口 I/O 是特权操作；调用方必须处于 ring0（内核态）。
pub unsafe fn config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    unsafe {
        port::outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        port::inl(CONFIG_DATA)
    }
}

/// 写 32 位配置寄存器。
///
/// # Safety
/// 同 [`config_read32`]；写错了会改变设备行为（BAR 探测会写全 1，见 `read_bar`）。
pub unsafe fn config_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    unsafe {
        port::outl(CONFIG_ADDRESS, config_address(bus, device, function, offset));
        port::outl(CONFIG_DATA, value);
    }
}

/// 读 8 位配置寄存器（半字节/字节由 `offset` 的低 2 位决定）。
///
/// # Safety
/// 同 [`config_read32`]。
pub unsafe fn config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let value = unsafe { config_read32(bus, device, function, offset) };
    (value >> ((offset & 3) * 8)) as u8
}

/// 探测一个 BAR：返回类型、基址与窗口大小。
///
/// 探测过程会临时把该 BAR 写成全 1（规范定义的「允许探测」窗口），读回掩码，
/// 再把原值写回。写回是必须的 —— 否则设备在枚举之后会指向错误地址。
unsafe fn read_bar(bus: u8, device: u8, function: u8, index: usize) -> Option<Bar> {
    let offset = OFFSET_BAR0 + (index as u8) * 4;
    let original = unsafe { config_read32(bus, device, function, offset) };
    if original == 0 {
        return None; // 未实现该 BAR（全 0 表示「不使用」，与未分配的窗口无法区分，见模块头注释）
    }

    let io = original & 0x1 != 0;
    let type_bits = (original >> 1) & 0x3;
    let is_64bit = !io && type_bits == 0b10;
    let high_original = if is_64bit {
        unsafe { config_read32(bus, device, function, offset + 4) }
    } else {
        0
    };

    // 写全 1 探测窗口大小
    unsafe { config_write32(bus, device, function, offset, 0xFFFF_FFFF) };
    let low_mask = unsafe { config_read32(bus, device, function, offset) };
    let high_mask = if is_64bit {
        unsafe { config_write32(bus, device, function, offset + 4, 0xFFFF_FFFF) };
        unsafe { config_read32(bus, device, function, offset + 4) }
    } else {
        0
    };

    // 恢复原值（顺序：先高后低，避免中途出现非法组合）
    if is_64bit {
        unsafe { config_write32(bus, device, function, offset + 4, high_original) };
    }
    unsafe { config_write32(bus, device, function, offset, original) };

    // 大小只从掩码推出来：`size = ~mask + 1`，且**必须在该字自己的位宽里取反**
    // （低字是 u32；把它当 u64 取反会得到 0xFFFFFFFF00000080 这种值）。
    //
    // 另外两个边界：
    // - 掩码全 1（`u32::MAX`）→ 该 BAR 未实现（与 Linux `pci_size` 的判据一致）；
    // - 64 位 BAR 的低字掩码为 0 → 窗口 ≥ 4 GiB，大小由高字决定。
    let size = if is_64bit {
        match probe_size(low_mask) {
            Some(low) if low == 0x1_0000_0000 => probe_size(high_mask).map_or(0, |high| high << 32),
            Some(low) => low,
            None => 0,
        }
    } else {
        probe_size(low_mask).unwrap_or(0)
    };

    let address_raw = if is_64bit {
        ((high_original as u64) << 32) | (original as u64)
    } else {
        original as u64
    };
    let address = if io {
        address_raw & !0x3
    } else {
        address_raw & !0xF
    };

    Some(Bar {
        index,
        io,
        is_64bit,
        address,
        size,
    })
}

/// 由「写全 1 探测」读回的掩码推窗口大小：`size = ~mask + 1`。
///
/// 语义：掩码里为 1 的位是设备**可写**的地址位，为 0 的位是大小位。因此低 4 位
/// （类型/预取标志）要屏蔽掉；掩码全 1 表示该 BAR 未实现（返回 `None`）。
///
/// 注意 `0x1_0000_0000 - aligned` 而不是 `!aligned + 1`：低字是 u32，必须在 32 位里
/// 取反，否则 0xFFFFFF80 会算出 0xFFFFFFFF00000080。
fn probe_size(mask: u32) -> Option<u64> {
    if mask == u32::MAX {
        return None;
    }
    Some(0x1_0000_0000 - ((mask & !0xFu32) as u64))
}

/// 读一个 function 的基础信息（不存在返回 `None`）。
unsafe fn read_device(bus: u8, device: u8, function: u8) -> Option<PciDevice> {
    let id = unsafe { config_read32(bus, device, function, OFFSET_VENDOR) };
    let vendor = (id & 0xFFFF) as u16;
    if vendor == NO_DEVICE {
        return None;
    }
    let device_id = (id >> 16) as u16;
    let class_reg = unsafe { config_read32(bus, device, function, OFFSET_CLASS) };
    let revision = (class_reg & 0xFF) as u8;
    let prog_if = ((class_reg >> 8) & 0xFF) as u8;
    let subclass = ((class_reg >> 16) & 0xFF) as u8;
    let class = ((class_reg >> 24) & 0xFF) as u8;
    let header_type = unsafe { config_read8(bus, device, function, OFFSET_HEADER_TYPE) };

    let mut bars: [Option<Bar>; BAR_COUNT] = [None; BAR_COUNT];
    let mut index = 0;
    while index < BAR_COUNT {
        let bar = unsafe { read_bar(bus, device, function, index) };
        if let Some(info) = bar {
            bars[index] = Some(info);
            index += if info.is_64bit { 2 } else { 1 };
        } else {
            index += 1;
        }
    }

    let _ = revision;
    Some(PciDevice {
        bus,
        device,
        function,
        vendor,
        device_id,
        class,
        subclass,
        prog_if,
        header_type,
        bars,
    })
}

// ---- 枚举与全局设备表 ----

/// 枚举到的设备表（`init` 之后只读）。
static mut DEVICES: Option<Vec<PciDevice>> = None;

/// 已枚举的设备表（未初始化时为空表）。
pub fn devices() -> &'static [PciDevice] {
    unsafe {
        let slot = (&raw const DEVICES).as_ref();
        match slot.and_then(|v| v.as_ref()) {
            Some(list) => list.as_slice(),
            None => &[],
        }
    }
}

/// 设备数。
pub fn device_count() -> usize {
    devices().len()
}

/// 扫描全部总线上的 PCI function 并填入设备表（可重复调用，第二次起复用结果）。
///
/// 返回枚举到的设备数。每个设备会打印一行串口日志（`vendor`/`device`/`class`/BAR），
/// 这些日志就是「枚举真的发生了」的断言来源。
pub fn init() -> usize {
    if unsafe { (&raw const DEVICES).as_ref().is_some_and(|v| v.is_some()) } {
        return device_count();
    }
    let mut list = Vec::new();
    for bus in 0..=255u8 {
        for device in 0..=31u8 {
            let Some(first) = (unsafe { read_device(bus, device, 0) }) else {
                continue;
            };
            let multi = first.is_multi_function();
            list.push(first);
            log_device(&first);
            if multi {
                for function in 1..=7u8 {
                    if let Some(extra) = unsafe { read_device(bus, device, function) } {
                        list.push(extra);
                        log_device(&extra);
                    }
                }
            }
        }
    }
    let count = list.len();
    unsafe {
        let slot = (&raw mut DEVICES).as_mut().expect("DEVICES 是静态量");
        *slot = Some(list);
    }
    count
}

/// 按厂商/设备 ID 查找（第一个匹配）。
pub fn find_by_id(vendor: u16, device_id: u16) -> Option<PciDevice> {
    devices()
        .iter()
        .find(|dev| dev.vendor == vendor && dev.device_id == device_id)
        .copied()
}

/// `pci: bus=0x00 device=0x05 function=0x00 vendor=0x1af4 device_id=0x1001 class=0x01 subclass=0x00 prog_if=0x00`
///
/// 另起一行打 BAR 列表（只打已实现的槽位）。BDF 同时打印成紧凑的 `00:05.0` 形式，
/// 让人一眼能对上 `lspci` 的输出。
pub fn log_device(dev: &PciDevice) {
    serial::print("pci: ");
    print_bdf(dev.bus, dev.device, dev.function);
    serial::print(" vendor=");
    serial::print_hex(dev.vendor as u64);
    serial::print(" device_id=");
    serial::print_hex(dev.device_id as u64);
    serial::print(" class=");
    serial::print_hex(dev.class as u64);
    serial::print(" subclass=");
    serial::print_hex(dev.subclass as u64);
    serial::print(" prog_if=");
    serial::print_hex(dev.prog_if as u64);
    serial::println("");

    for bar in dev.bars.iter().flatten() {
        serial::print("pci: ");
        print_bdf(dev.bus, dev.device, dev.function);
        serial::print(" bar");
        serial::print_dec(bar.index as u64);
        serial::print(if bar.io { "=io:" } else { "=mmio:" });
        serial::print_hex(bar.address);
        serial::print(" size=");
        serial::print_hex(bar.size);
        serial::println("");
    }
}

/// 打印紧凑的 `bus:device.function`（`00:05.0`）。
pub fn print_bdf(bus: u8, device: u8, function: u8) {
    serial::print_hex_pad(bus as u64, 2);
    serial::print(":");
    serial::print_hex_pad(device as u64, 2);
    serial::print(".");
    serial::print_dec(function as u64);
}
