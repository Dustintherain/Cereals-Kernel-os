//! AHCI / SATA 块设备驱动（驱动兼容性：真实机器上的磁盘控制器）
//!
//! ## 为什么加它（与 virtio-blk 的关系）
//!
//! v0.8 的块设备只有一个后端：legacy virtio-blk（[`crate::driver::virtio_blk`]）。
//! 它够用，但**只在虚拟化环境下存在** —— 真实机器的内置盘挂在 AHCI（SATA）控制器上，
//! 而 q35 的芯片组本身就带一个 ICH9 AHCI 控制器（`00:1f.2`，`8086:2922`，
//! class `0x01` / subclass `0x06` / prog_if `0x01`）。换句话说：在这之前，
//! 内核**看不到**这台机器上唯一真实存在的磁盘控制器。
//!
//! 这个驱动因此是「第二条块设备路径」，而不是 virtio-blk 的替代品：
//! 两者实现同一个 [`BlockDevice`] trait（[`crate::driver::block`]），
//! 上层（FAT 后端、VFS、CSS 存储层）一行都不用改 —— 这正是 v0.8 把
//! 「后端可换、接口不变」写进 `block.rs` 模块头的理由。
//!
//! > 注意到 `00:1f.2` 之前，先修掉了 PCI 枚举的一个真缺陷（B-18）：
//! > 头部类型读的是偏移 `0x0C`（cache line size，恒为 0）而不是 `0x0E`，
//! > 于是**多功能设备的 function 1..7 从来没被枚举过**。SATA 与 SMBus
//! > 都是 `00:1f.0`（ISA 桥）的后续 function —— 固件（OVMF）看得见它们
//! > （引导日志里有 `Pci(0x1F,0x2)/Sata(...)`），内核却看不见。
//!
//! ## 数据结构（AHCI 1.3 的「三大件」）
//!
//! ```text
//! 命令列表  PxCLB → 32 × 32 B = 1 KiB，1 KiB 对齐（本驱动只用槽位 0）
//! 接收 FIS  PxFB  → 256 B，256 B 对齐（设备 → 主机的 FIS 落点）
//! 命令表    CTBA  → 128 B 固定部分 + 16 B × PRDTL，128 B 对齐
//!                  0x00 CFIS[64]  0x40 ACMD[16]  0x50 保留[48]  0x80 PRDT[]
//! ```
//!
//! 三块内存都来自 [`crate::memory::frame::allocate_contiguous`]（4 KiB 帧，
//! 恒等映射 → 帧地址就是设备要的物理地址），一次请求只用槽位 0 —— 与
//! virtio-blk 一样是「同一时刻一个请求在飞」的同步模型。
//!
//! ## 同步轮询而不是中断
//!
//! 提交（写 `PxCI` 的槽位位）之后自旋等它清零，超时上限由 PIT tick 给出，
//! 与 [`crate::driver::virtio_blk`] 的取舍一致（不配置 MSI-X/INTx，见
//! `docs/drivers.md` 1.2 与 1.4）。代价是等待期间忙等，换来的是驱动不依赖
//! IRQ 分发、也不在中断上下文里做块 I/O。
//!
//! ## 缓冲策略
//!
//! 与 virtio-blk 相同：调用方缓冲只要整扇区对齐，驱动内部用一块页对齐的静态
//! bounce buffer（[`BOUNCE_BYTES`] = 8 扇区 = 4 KiB）分块搬运，因此不要求
//! 调用方缓冲物理连续、也不要求对齐 —— 一次拷贝换掉一整类 DMA 对齐问题。
//!
//! ## 已知限制（明确没做，不要读成已完成）
//!
//! - 只用端口 0 号命令槽，且**不用**端口乘法器（PMP）与 FIS-based switching；
//! - 不做端口 COMRESET：端口必须由固件（或前一级引导）把 PHY 带起来
//!   （`PxSSTS.DET == 3`），否则视为「没有盘」。理由：对**没有盘**的端口做
//!   COMRESET 要等完整超时，6 个端口就是 30 秒，启动日志会被拖垮；
//! - 不配置中断（与块设备/网卡一样走轮询）、不做 NCQ（不用 `PxSACT`）；
//! - 不区分 ATAPI（光驱）：`PxSIG` 不是 SATA 磁盘的端口直接跳过。

use core::sync::atomic::{fence, Ordering};

use crate::arch::x86_64::paging;
use crate::driver::block::{self, is_sector_aligned, BlockDevice, SECTOR_SIZE};
use crate::driver::driver::Driver;
use crate::driver::pci::{self, PciDevice};
use crate::driver::serial;
use crate::memory::frame;
use crate::memory::page::PAGE_SIZE;
use crate::time::clock;

/// PCI：大容量存储控制器
const CLASS_STORAGE: u8 = 0x01;
/// PCI：SATA（AHCI 1.0）
const SUBCLASS_SATA: u8 = 0x06;
/// PCI：编程接口 = AHCI 1.0
const PROG_IF_AHCI: u8 = 0x01;

// ---- 通用主机控制寄存器（相对 ABAR）----
const CAP: u64 = 0x00;
const GHC: u64 = 0x04;
const PI: u64 = 0x0C;
const VS: u64 = 0x10;

/// `GHC`：AHCI 使能
const GHC_AE: u32 = 1 << 31;

// ---- 端口寄存器（相对 `0x100 + port * 0x80`）----
const PORT_BASE: u64 = 0x100;
const PORT_STRIDE: u64 = 0x80;

const PX_CLB: u64 = 0x00; // 命令列表基址（低/高）
const PX_CLBU: u64 = 0x04;
const PX_FB: u64 = 0x08; // 接收 FIS 基址（低/高）
const PX_FBU: u64 = 0x0C;
const PX_IS: u64 = 0x10; // 端口中断状态（写 1 清零）
const PX_CMD: u64 = 0x18; // 端口命令与状态
const PX_TFD: u64 = 0x20; // 任务文件数据
const PX_SIG: u64 = 0x24; // 设备签名
const PX_SSTS: u64 = 0x28; // SATA 状态
const PX_SERR: u64 = 0x30; // SATA 错误（写 1 清零）
const PX_CI: u64 = 0x38; // 命令发布（bit N = 槽位 N）

/// `PxCMD`：启动命令列表处理
const PXCMD_ST: u32 = 1 << 0;
/// `PxCMD`：FIS 接收使能
const PXCMD_FRE: u32 = 1 << 4;
/// `PxCMD`：命令列表运行中（只读）
const PXCMD_FR: u32 = 1 << 14;
/// `PxCMD`：命令列表正在取指（只读）
const PXCMD_CR: u32 = 1 << 15;

/// `PxTFD`：忙
const TFD_BSY: u32 = 1 << 7;
/// `PxTFD`：数据请求
const TFD_DRQ: u32 = 1 << 3;
/// `PxTFD`：错误（bit 0 是 ERR，7:1 是具体错误位）
const TFD_ERR: u32 = 1 << 0;

/// `PxSSTS.DET` 字段：设备已就位且 PHY 通信已建立
const SSTS_DET_MASK: u32 = 0xF;
const SSTS_DET_PRESENT: u32 = 3;
/// `PxSIG`：SATA 磁盘（ATA）
const SIG_SATA: u32 = 0x0000_0101;

/// `PxIS`：任务文件错误（Task File Error Status）
const PXIS_TFES: u32 = 1 << 30;

// ---- ATA 命令 ----
const CMD_IDENTIFY: u8 = 0xEC; // IDENTIFY DEVICE
const CMD_READ_DMA_EXT: u8 = 0x25; // READ DMA EXT（LBA48，设备 → 主机）
const CMD_WRITE_DMA_EXT: u8 = 0x35; // WRITE DMA EXT（LBA48，主机 → 设备）

// ---- FIS ----
/// H2D（主机 → 设备）寄存器 FIS
const FIS_TYPE_H2D: u8 = 0x27;
/// H2D FIS 的 C 位：这是一条命令（而不是控制更新）
const H2D_FLAG_C: u8 = 1 << 7;
/// H2D FIS 的 Device 字节：LBA 模式（bit 6）
const ATA_DEVICE_LBA: u8 = 0x40;
/// H2D FIS 长度：20 字节 = 5 个 dword（命令头的 CFL 字段）
const CMD_HDR_CFL_FIS_DWORDS: u32 = 5;

// ---- 命令头 / 命令表 ----
/// 命令头：W 位（1 = 数据从主机内存流向设备 = 写盘）
const CMD_HDR_WRITE: u32 = 1 << 6;
/// 命令头：PRDTL 在 bit 31:16
const CMD_HDR_PRDTL_SHIFT: u32 = 16;
/// 命令表里 PRDT 的起始偏移
const CMD_TBL_PRDT: u64 = 0x80;
/// 命令表用到的字节数（固定 128 B + 1 个 PRDT 条目 16 B）
const CMD_TBL_BYTES: u64 = CMD_TBL_PRDT + 16;
/// PRDT：DBC 的 bit 31 = 完成时中断
const PRDT_IOC: u32 = 1 << 31;

/// 单次请求最多搬运的扇区数（bounce buffer 大小 / 扇区大小）。
const BOUNCE_SECTORS: usize = 8;
/// bounce buffer 字节数（页对齐，正好 4 KiB）。
const BOUNCE_BYTES: usize = BOUNCE_SECTORS * SECTOR_SIZE as usize;
/// 一次 I/O 的等待上限（PIT tick；100Hz → 500 tick = 5 秒）。
const IO_TIMEOUT_TICKS: u64 = 500;
/// 自旋上限（tick 不推进时的兜底，避免内核永久挂住）。
const IO_SPIN_LIMIT: u64 = 200_000_000;

/// 页对齐的 bounce buffer（静态 `.bss`，内核恒等映射 → 虚拟地址即物理地址）。
#[repr(align(4096))]
struct Bounce([u8; BOUNCE_BYTES]);

static mut BOUNCE: Bounce = Bounce([0; BOUNCE_BYTES]);

/// bounce buffer 的地址（`addr_of_mut!` 只取地址、不建引用）。
fn bounce_ptr() -> *mut u8 {
    // SAFETY: 仅计算地址。
    unsafe { core::ptr::addr_of_mut!(BOUNCE.0) as *mut u8 }
}

/// 一块挂在某个 AHCI 端口上的盘（`attach` 期间填好，之后经 `block` 注册表访问）。
#[derive(Clone, Copy)]
struct AhciDisk {
    name: &'static str,
    /// ABAR（MMIO 基址，恒等映射 → 直接当指针用）
    abar: u64,
    /// 端口号
    port: u32,
    /// 命令列表物理地址（1 KiB 对齐）
    clb: u64,
    /// 命令表物理地址（128 B 对齐）
    ctba: u64,
    /// 容量（扇区）
    capacity: u64,
}

impl AhciDisk {
    const EMPTY: Self = Self {
        name: "ahci0",
        abar: 0,
        port: 0,
        clb: 0,
        ctba: 0,
        capacity: 0,
    };

    fn port_base(&self) -> u64 {
        port_addr(self.abar, self.port, 0)
    }
}

/// 驱动能带的盘数上限（= [`crate::driver::block::MAX_BLOCK_DEVICES`]）。
pub const MAX_DEVICES: usize = crate::driver::block::MAX_BLOCK_DEVICES;

/// 实例名表（`BlockDevice::name` 要 `&'static str`，所以名字得来自静态表）。
static NAMES: [&str; MAX_DEVICES] = [
    "ahci0", "ahci1", "ahci2", "ahci3", "ahci4", "ahci5", "ahci6", "ahci7",
];

/// 驱动实例数组：`attach` 按顺序占用，槽位号就是块设备号。
static mut DISKS: [AhciDisk; MAX_DEVICES] = [AhciDisk::EMPTY; MAX_DEVICES];
/// 已被 `attach` 占用的实例数。
static mut ATTACHED: usize = 0;
/// 第一块 AHCI 盘在块设备表里的下标（`main` 用它跑块设备自测）。
static mut FIRST_BLOCK_INDEX: Option<usize> = None;

/// 第一块 AHCI 盘的块设备下标（总线上没有 SATA 盘时为 `None`）。
pub fn block_index() -> Option<usize> {
    // SAFETY: 只读访问静态量；`probe` 结束后不再变化。
    unsafe { *(&raw const FIRST_BLOCK_INDEX) }
}

/// 驱动对象：由 [`crate::driver::driver`] 的注册表引用（零大小，状态在 [`DISKS`] 里）。
pub struct AhciDriver;

/// `driver` 注册表里的一项。
pub static AHCI_DRIVER: AhciDriver = AhciDriver;

impl Driver for AhciDriver {
    fn name(&self) -> &'static str {
        "ahci-pci"
    }

    /// 按**类别**匹配而不是厂商/设备 ID：任何符合 AHCI 1.0 的 SATA 控制器都认
    ///（Intel / AMD / 各家芯片组），因此这里刻意不看 `vendor`。
    fn matches(&self, dev: &PciDevice) -> bool {
        dev.class == CLASS_STORAGE && dev.subclass == SUBCLASS_SATA && dev.prog_if == PROG_IF_AHCI
    }

    fn attach(&self, dev: &PciDevice) -> Result<(), &'static str> {
        attach(dev)
    }
}

// ---- MMIO / 内存访问（volatile：这些内存同时被设备读写） ----

#[inline]
unsafe fn reg_read(addr: u64) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline]
unsafe fn reg_write(addr: u64, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

#[inline]
unsafe fn mem_u8(addr: u64, value: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}

#[inline]
unsafe fn mem_u32(addr: u64, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

#[inline]
unsafe fn mem_u64(addr: u64, value: u64) {
    unsafe { core::ptr::write_volatile(addr as *mut u64, value) }
}

/// 端口寄存器地址。
fn port_addr(abar: u64, port: u32, offset: u64) -> u64 {
    abar + PORT_BASE + (port as u64) * PORT_STRIDE + offset
}

/// 自旋等待 `cond` 为真，上限由 PIT tick（[`IO_TIMEOUT_TICKS`]）与自旋次数给出。
///
/// `what` 只用于超时日志：块设备超时最难定位的就是「卡在哪一步」。
fn wait_for(what: &'static str, mut cond: impl FnMut() -> bool) -> Result<(), &'static str> {
    let start = unsafe { clock::tick_count() };
    let mut spins = 0u64;
    loop {
        if cond() {
            return Ok(());
        }
        spins += 1;
        if spins > IO_SPIN_LIMIT || unsafe { clock::tick_count() } - start > IO_TIMEOUT_TICKS {
            serial::print("ahci: timeout waiting for ");
            serial::println(what);
            return Err("ahci: device timed out");
        }
        core::hint::spin_loop();
    }
}

/// 挂接一个 AHCI 控制器：使能 AHCI → 逐个端口找 SATA 盘 → 每块盘注册成块设备。
///
/// 端口上没有盘**不算失败**（总线上有控制器、没插盘是常态，`--ahci` 之外的
/// 每个测试场景都走这条路），日志里会明确写出 `drives=0`。
fn attach(dev: &PciDevice) -> Result<(), &'static str> {
    let bar = dev.bar(5).ok_or("ahci: BAR5 (ABAR) is not implemented")?;
    if bar.io {
        return Err("ahci: BAR5 is an I/O window, expected MMIO");
    }
    if bar.address == 0 || bar.size == 0 {
        return Err("ahci: ABAR is unassigned (firmware left the window at zero)");
    }
    let abar = bar.address;

    serial::print("ahci: controller ");
    let (bus, device, function) = dev.location();
    pci::print_bdf(bus, device, function);
    serial::print(" abar=");
    serial::print_hex(abar);
    serial::print(" size=");
    serial::print_hex(bar.size);
    serial::println("");

    // ABAR 落在 MMIO 区（不在 RAM 里），因此先确保这段恒等映射存在。
    // 内核跑在固件建立的恒等映射上，固件自己用过这个控制器，通常已经映射好了
    //（这时 `ensure_identity_range` 什么都不做，也不会打印）。
    paging::ensure_identity_range(abar, abar + bar.size.max(PAGE_SIZE))?;

    // 内存空间 + 总线主控（DMA）。固件通常已经打开，这里显式确认并打印前后值 ——
    // 真实平台上若没打开，日志上能看到是哪一步补上的。
    let before = dev.command();
    dev.enable_command(pci::COMMAND_MEMORY_SPACE | pci::COMMAND_BUS_MASTER);
    let after = dev.command();
    serial::print("ahci: pci command=");
    serial::print_hex(before as u64);
    serial::print(" -> ");
    serial::print_hex(after as u64);
    serial::println("");
    if after & pci::COMMAND_MEMORY_SPACE == 0 {
        return Err("ahci: enabling the memory space did not stick");
    }

    // AHCI 使能（GHC.AE）。固件一般已经置位。
    let ghc = unsafe { reg_read(abar + GHC) };
    if ghc & GHC_AE == 0 {
        unsafe { reg_write(abar + GHC, ghc | GHC_AE) };
        wait_for("GHC.AE", || unsafe { reg_read(abar + GHC) } & GHC_AE != 0)?;
    }

    let cap = unsafe { reg_read(abar + CAP) };
    let version = unsafe { reg_read(abar + VS) };
    let pi = unsafe { reg_read(abar + PI) };
    // CAP：bit 4:0 = NCS（0 基的命令槽数）。
    // 端口数取 `PI`（Ports Implemented）的位数，而不是 CAP 的 NP：QEMU 的 ich9-ahci
    // 报的 NP 是 31（按 AHCI_MAX_PORTS=32 算），而真正实现的只有 6 个 —— `PI` 才是
    //「这台控制器上有几个端口可用」的权威来源（枚举也只看它）。
    let slots = (cap & 0x1F) + 1;
    serial::print("ahci: version=");
    serial::print_hex(version as u64);
    serial::print(" cap=");
    serial::print_hex(cap as u64);
    serial::print(" slots=");
    serial::print_dec(slots as u64);
    serial::print(" implemented=");
    serial::print_hex(pi as u64);
    serial::print(" (");
    serial::print_dec(pi.count_ones() as u64);
    serial::println(" ports)");
    if slots == 0 {
        return Err("ahci: controller reports zero command slots");
    }

    let mut drives = 0usize;
    let mut failures = 0usize;
    for port in 0..32u32 {
        if pi & (1 << port) == 0 {
            continue;
        }
        let ssts = unsafe { reg_read(port_addr(abar, port, PX_SSTS)) };
        let sig = unsafe { reg_read(port_addr(abar, port, PX_SIG)) };
        let det = ssts & SSTS_DET_MASK;
        serial::print("ahci: port ");
        serial::print_dec(port as u64);
        serial::print(" det=");
        serial::print_dec(det as u64);
        serial::print(" sig=");
        serial::print_hex(sig as u64);
        if det != SSTS_DET_PRESENT {
            serial::println(" (no device)");
            continue;
        }
        if sig != SIG_SATA {
            serial::println(" (not a SATA disk; skipped)");
            continue;
        }
        serial::println(" (SATA disk)");
        match bring_up_drive(abar, port) {
            Ok(_) => drives += 1,
            Err(err) => {
                failures += 1;
                serial::print("ahci: port ");
                serial::print_dec(port as u64);
                serial::print(" FAILED: ");
                serial::println(err);
            }
        }
    }
    serial::print("ahci: controller ");
    pci::print_bdf(bus, device, function);
    serial::print(" drives=");
    serial::print_dec(drives as u64);
    serial::print(" failures=");
    serial::print_dec(failures as u64);
    serial::println("");

    if failures > 0 {
        return Err("ahci: at least one SATA port failed to come up");
    }
    Ok(())
}

/// 把一个端口带起来、读 IDENTIFY 并把盘注册成块设备；返回块设备下标。
fn bring_up_drive(abar: u64, port: u32) -> Result<usize, &'static str> {
    let cmd_addr = port_addr(abar, port, PX_CMD);

    // 1. 停端口：ST=0 → 等 CR=0；FRE=0 → 等 FR=0（改动基址寄存器前必须静止）。
    let cmd = unsafe { reg_read(cmd_addr) };
    if cmd & PXCMD_ST != 0 {
        unsafe { reg_write(cmd_addr, cmd & !PXCMD_ST) };
    }
    wait_for("PxCMD.CR to clear", || unsafe { reg_read(cmd_addr) } & PXCMD_CR == 0)?;
    let cmd = unsafe { reg_read(cmd_addr) };
    if cmd & PXCMD_FRE != 0 {
        unsafe { reg_write(cmd_addr, cmd & !PXCMD_FRE) };
    }
    wait_for("PxCMD.FR to clear", || unsafe { reg_read(cmd_addr) } & PXCMD_FR == 0)?;

    // 2. 三张表（见模块头）。接收 FIS 区的帧必须一直有效，因此不做实例字段
    //    （帧不释放即保持有效，这里也不需要再读它）。
    let clb = frame::allocate_contiguous(1)
        .ok_or("ahci: no frame for the command list")?
        .start_address();
    let fb = frame::allocate_contiguous(1)
        .ok_or("ahci: no frame for the received-FIS area")?
        .start_address();
    let ctba = frame::allocate_contiguous(1)
        .ok_or("ahci: no frame for the command table")?
        .start_address();
    // SAFETY: 三块都是刚分配的 4 KiB 帧，恒等映射可写。
    unsafe {
        core::ptr::write_bytes(clb as *mut u8, 0, PAGE_SIZE as usize);
        core::ptr::write_bytes(fb as *mut u8, 0, PAGE_SIZE as usize);
        core::ptr::write_bytes(ctba as *mut u8, 0, PAGE_SIZE as usize);
        reg_write(port_addr(abar, port, PX_CLB), clb as u32);
        reg_write(port_addr(abar, port, PX_CLBU), (clb >> 32) as u32);
        reg_write(port_addr(abar, port, PX_FB), fb as u32);
        reg_write(port_addr(abar, port, PX_FBU), (fb >> 32) as u32);
        // 3. 清中断与 SATA 错误状态（两个寄存器都是写 1 清零）。
        reg_write(port_addr(abar, port, PX_IS), u32::MAX);
        reg_write(port_addr(abar, port, PX_SERR), u32::MAX);
    }

    // 4. FRE=1（等 FR 置位）→ ST=1（等 CR 置位）：端口开始处理命令列表。
    let cmd = unsafe { reg_read(cmd_addr) } | PXCMD_FRE;
    unsafe { reg_write(cmd_addr, cmd) };
    wait_for("PxCMD.FR to set", || unsafe { reg_read(cmd_addr) } & PXCMD_FR != 0)?;
    let cmd = unsafe { reg_read(cmd_addr) } | PXCMD_ST;
    unsafe { reg_write(cmd_addr, cmd) };
    wait_for("PxCMD.CR to set", || unsafe { reg_read(cmd_addr) } & PXCMD_CR != 0)?;

    // 5. 占一个实例槽位（槽位号 = 块设备号），再 IDENTIFY 拿容量与型号。
    let slot = unsafe {
        let used = *(&raw const ATTACHED);
        if used >= MAX_DEVICES {
            return Err("ahci: no free device slot");
        }
        let state = &mut *(&raw mut DISKS[used]);
        state.name = NAMES[used];
        state.abar = abar;
        state.port = port;
        state.clb = clb;
        state.ctba = ctba;
        state.capacity = 0;
        *(&raw mut ATTACHED) = used + 1;
        used
    };
    // SAFETY: 槽位刚填好且未被注册，独占访问。
    unsafe { (&mut *(&raw mut DISKS[slot])).identify()? };

    let index = block::register(unsafe { &mut *(&raw mut DISKS[slot]) })?;
    serial::print("ahci: registered as block device ");
    serial::print_dec(index as u64);
    serial::print(" name=");
    serial::println(NAMES[slot]);
    // SAFETY: 只写一次（第一块盘），此后只读。
    unsafe {
        if (*(&raw const FIRST_BLOCK_INDEX)).is_none() {
            *(&raw mut FIRST_BLOCK_INDEX) = Some(index);
        }
    }
    Ok(index)
}

impl AhciDisk {
    /// IDENTIFY DEVICE（`0xEC`）：512 字节进 bounce buffer，容量与型号都来自设备本身
    ///（不是写死的常量）。
    fn identify(&mut self) -> Result<(), &'static str> {
        // SAFETY: bounce buffer 是 4 KiB 对齐的静态数组，长度足够。
        unsafe { core::ptr::write_bytes(bounce_ptr(), 0, SECTOR_SIZE as usize) };
        self.issue(CMD_IDENTIFY, 0, 1, false, SECTOR_SIZE as usize)?;

        // SAFETY: 上面的 `issue` 已把 512 字节写进 bounce buffer。
        let id = unsafe { core::slice::from_raw_parts(bounce_ptr() as *const u8, SECTOR_SIZE as usize) };
        let word = |index: usize| u16::from_le_bytes([id[index * 2], id[index * 2 + 1]]);
        // word 83 bit 10 = 支持 LBA48；容量在 word 100..104（LBA48）或 60..62（LBA28）。
        let lba48 = word(83) & (1 << 10) != 0;
        let capacity = if lba48 {
            (word(100) as u64)
                | ((word(101) as u64) << 16)
                | ((word(102) as u64) << 32)
                | ((word(103) as u64) << 48)
        } else {
            (word(60) as u64) | ((word(61) as u64) << 16)
        };
        if capacity == 0 {
            return Err("ahci: device reports zero capacity");
        }
        self.capacity = capacity;

        serial::print("ahci: port ");
        serial::print_dec(self.port as u64);
        serial::print(" model=");
        print_model(id);
        serial::print(" capacity=");
        serial::print_dec(capacity);
        serial::print(" sectors (");
        serial::print_dec(capacity * SECTOR_SIZE as u64 / (1024 * 1024));
        serial::print(" MiB) lba48=");
        serial::print_dec(u64::from(lba48));
        serial::println("");
        Ok(())
    }

    /// 提交一条命令（槽位 0）并自旋等它完成。
    ///
    /// `lba`/`count` 只对读写命令有意义；`len` 是本次数据交换的字节数
    ///（必须是整扇区且不超过 [`BOUNCE_BYTES`]）。
    fn issue(
        &mut self,
        command: u8,
        lba: u64,
        count: u16,
        write: bool,
        len: usize,
    ) -> Result<(), &'static str> {
        if len == 0 || len > BOUNCE_BYTES || len % SECTOR_SIZE as usize != 0 {
            return Err("ahci: transfer length out of range");
        }
        let pb = self.port_base();
        // 端口必须空闲：任务文件没有在忙/等数据，且没有在飞的命令。
        wait_for("PxTFD idle", || unsafe { reg_read(pb + PX_TFD) } & (TFD_BSY | TFD_DRQ) == 0)?;
        wait_for("PxCI empty", || unsafe { reg_read(pb + PX_CI) } == 0)?;

        // SAFETY: 命令列表 / 命令表 / bounce buffer 都已分配并恒等映射；
        // 环上的读写用 volatile（设备也读写它们）。
        unsafe {
            reg_write(pb + PX_IS, u32::MAX);

            // 命令表：CFIS（20 字节的 H2D 寄存器 FIS）
            let ct = self.ctba;
            core::ptr::write_bytes(ct as *mut u8, 0, CMD_TBL_BYTES as usize);
            mem_u8(ct, FIS_TYPE_H2D);
            mem_u8(ct + 1, H2D_FLAG_C); // C=1：命令；PM 端口 0
            mem_u8(ct + 2, command);
            mem_u8(ct + 4, lba as u8);
            mem_u8(ct + 5, (lba >> 8) as u8);
            mem_u8(ct + 6, (lba >> 16) as u8);
            mem_u8(
                ct + 7,
                if command == CMD_IDENTIFY { 0 } else { ATA_DEVICE_LBA },
            );
            mem_u8(ct + 8, (lba >> 24) as u8);
            mem_u8(ct + 9, (lba >> 32) as u8);
            mem_u8(ct + 10, (lba >> 40) as u8);
            mem_u8(ct + 12, count as u8);
            mem_u8(ct + 13, (count >> 8) as u8);

            // PRDT：一块 bounce buffer，DBC = 字节数 - 1，完成时中断。
            let prdt = ct + CMD_TBL_PRDT;
            mem_u64(prdt, bounce_ptr() as u64);
            mem_u32(prdt + 8, 0);
            mem_u32(prdt + 12, (len as u32 - 1) | PRDT_IOC);

            // 命令头（槽位 0）：CFL = 5 dword；W 位表示数据流向
            //（1 = 主机内存 → 设备，即写盘，见 AHCI 1.3 的 W 位定义）。
            let mut opts = CMD_HDR_CFL_FIS_DWORDS;
            if write {
                opts |= CMD_HDR_WRITE;
            }
            opts |= 1 << CMD_HDR_PRDTL_SHIFT; // PRDTL = 1
            mem_u32(self.clb, opts);
            mem_u32(self.clb + 4, 0); // PRDBC：由 HBA 回填
            mem_u64(self.clb + 8, ct); // CTBA
        }

        // 命令列表与命令表必须在设备看到 PxCI 之前落内存。
        fence(Ordering::SeqCst);
        unsafe { reg_write(pb + PX_CI, 1) };

        wait_for("PxCI to clear", || unsafe { reg_read(pb + PX_CI) } & 1 == 0)?;
        let interrupts = unsafe { reg_read(pb + PX_IS) };
        let tfd = unsafe { reg_read(pb + PX_TFD) };
        if interrupts & PXIS_TFES != 0 || tfd & TFD_ERR != 0 {
            serial::print("ahci: task file error is=");
            serial::print_hex(interrupts as u64);
            serial::print(" tfd=");
            serial::print_hex(tfd as u64);
            serial::println("");
            return Err("ahci: device reported a task file error");
        }
        Ok(())
    }

    /// 一次数据交换（带容量与溢出检查，与 virtio-blk 的 `transfer` 语义一致）。
    fn transfer(
        &mut self,
        command: u8,
        lba: u64,
        sectors: u16,
        write: bool,
        len: usize,
    ) -> Result<(), &'static str> {
        if sectors == 0 || sectors as usize * SECTOR_SIZE as usize > BOUNCE_BYTES {
            return Err("ahci: request size out of range");
        }
        // `checked_add` 而不是 `lba + sectors`：调用方给的 lba 可能接近 u64::MAX，
        // 环绕后的和会「看起来」在容量内，于是把越界请求发给设备（同 B-17）。
        let end = lba
            .checked_add(sectors as u64)
            .ok_or("ahci: sector address overflow")?;
        if end > self.capacity {
            return Err("ahci: request beyond the device capacity");
        }
        self.issue(command, lba, sectors, write, len)
    }
}

impl BlockDevice for AhciDisk {
    fn name(&self) -> &'static str {
        self.name
    }

    fn sector_size(&self) -> u32 {
        SECTOR_SIZE
    }

    fn sectors(&self) -> u64 {
        self.capacity
    }

    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<usize, &'static str> {
        if !is_sector_aligned(buf.len()) {
            return Err("ahci: read length not a multiple of the sector size");
        }
        let mut done = 0usize;
        while done < buf.len() {
            let chunk = (buf.len() - done).min(BOUNCE_BYTES);
            let sectors = (chunk / SECTOR_SIZE as usize) as u16;
            self.transfer(
                CMD_READ_DMA_EXT,
                lba + (done / SECTOR_SIZE as usize) as u64,
                sectors,
                false,
                chunk,
            )?;
            // SAFETY: bounce buffer 是 `BOUNCE_BYTES` 大小的静态数组，且上面的
            // `transfer` 已经把它填好；`chunk <= BOUNCE_BYTES`。
            let bounce = unsafe { core::slice::from_raw_parts(bounce_ptr() as *const u8, chunk) };
            buf[done..done + chunk].copy_from_slice(bounce);
            done += chunk;
        }
        Ok(done)
    }

    fn write(&mut self, lba: u64, data: &[u8]) -> Result<usize, &'static str> {
        if !is_sector_aligned(data.len()) {
            return Err("ahci: write length not a multiple of the sector size");
        }
        let mut done = 0usize;
        while done < data.len() {
            let chunk = (data.len() - done).min(BOUNCE_BYTES);
            let sectors = (chunk / SECTOR_SIZE as usize) as u16;
            // SAFETY: 同 `read`；`chunk <= BOUNCE_BYTES`。
            let to_bounce = unsafe { core::slice::from_raw_parts_mut(bounce_ptr(), chunk) };
            to_bounce.copy_from_slice(&data[done..done + chunk]);
            self.transfer(
                CMD_WRITE_DMA_EXT,
                lba + (done / SECTOR_SIZE as usize) as u64,
                sectors,
                true,
                chunk,
            )?;
            done += chunk;
        }
        Ok(done)
    }
}

/// 打印 ATA 型号字符串（IDENTIFY 的 word 27..46，每个 word 的两字节是**交换**的）。
fn print_model(id: &[u8]) {
    let mut bytes = [0u8; 40];
    for index in 0..20 {
        bytes[index * 2] = id[(27 + index) * 2 + 1];
        bytes[index * 2 + 1] = id[(27 + index) * 2];
    }
    let end = bytes
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(0, |index| index + 1);
    for &byte in &bytes[..end] {
        serial::putc(if (0x20..0x7F).contains(&byte) { byte } else { b'.' });
    }
}
