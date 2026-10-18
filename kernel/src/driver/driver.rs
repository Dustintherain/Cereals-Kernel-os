//! 驱动注册框架 / Driver Manager（v0.8 实现）
//!
//! 目标：把「总线上有什么设备」（枚举）与「谁来管这个设备」（驱动）解耦。
//!
//! 分层（自下而上）：
//!
//! ```text
//! pci.rs         枚举 function / 探测 BAR —— 只回答“总线上有什么”
//! driver.rs      驱动注册表 + 匹配 + 挂接（本文件）—— 回答“谁来管它”
//! block.rs       按扇区读写 + 设备注册表 —— 上层唯一可见的接口
//! virtio_blk.rs  具体的块驱动（同时实现 Driver 与 BlockDevice）
//! ```
//!
//! 匹配语义（v0.8，够用且可预测）：
//!
//! - 驱动按**注册顺序**参与匹配，第一个 `matches` 为真的驱动获得该设备；
//! - 一个设备最多挂接一次；没有驱动认领的设备只记录一行日志（不算失败）；
//! - 某个驱动 `attach` 失败**不会**中断其它设备的探测，只计入 `failed`。
//!
//! 为什么不让每个驱动各自遍历 PCI 设备表：那样每加一个驱动就要复制一遍遍历逻辑，
//! 而且没人能回答「这台机器上有多少设备真的被驱动接管了」。这里把遍历与统计收在一处，
//! `main` 只需要调用 [`probe_all`] 并检查返回的 [`ProbeStats`]。

use crate::driver::pci::{self, PciDevice};
use crate::driver::serial;

/// 一个可挂接到 PCI function 上的驱动。
pub trait Driver {
    /// 驱动名（日志与错误信息里用，例如 `virtio-blk-pci`）。
    fn name(&self) -> &'static str;

    /// 是否认领该设备。只允许查看 ID / 类别 / BAR，**不得产生副作用**
    /// （同一设备可能被多个驱动依次询问）。
    fn matches(&self, dev: &PciDevice) -> bool;

    /// 挂接：初始化硬件，并把设备注册到上层抽象（例如块设备表）。
    fn attach(&self, dev: &PciDevice) -> Result<(), &'static str>;
}

/// 驱动数量上限（v0.8 virtio-blk；v0.9 追加 virtio-net；v1.0 追加 ahci。
/// 固定容量数组让借用关系一眼看清）。
pub const MAX_DRIVERS: usize = 4;

/// 驱动注册表（`register` 只在启动阶段写一次，之后只读）。
static mut REGISTRY: [Option<&'static dyn Driver>; MAX_DRIVERS] = [None; MAX_DRIVERS];

/// 注册一个驱动，返回它的下标。
///
/// 失败：注册表已满（[`MAX_DRIVERS`]）。
pub fn register(driver: &'static dyn Driver) -> Result<usize, &'static str> {
    // SAFETY: 注册表是静态量；启动阶段单线程写入，此后只读。
    unsafe {
        let table = (&raw mut REGISTRY).as_mut().expect("REGISTRY 是静态量");
        for (index, slot) in table.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(driver);
                return Ok(index);
            }
        }
    }
    Err("driver registry full")
}

/// 已注册的驱动数量。
pub fn count() -> usize {
    // SAFETY: 只读访问注册表。
    unsafe {
        (&raw const REGISTRY)
            .as_ref()
            .map(|table| table.iter().filter(|slot| slot.is_some()).count())
            .unwrap_or(0)
    }
}

/// 取第 `index` 个已注册驱动。
pub fn get(index: usize) -> Option<&'static dyn Driver> {
    // SAFETY: 只读访问注册表。
    unsafe { (&raw const REGISTRY).as_ref()?.get(index)?.as_ref().copied() }
}

/// 探测统计（验收断言的数字来源）。
#[derive(Clone, Copy, Default)]
pub struct ProbeStats {
    /// 参与匹配的 PCI function 数
    pub devices: usize,
    /// 被某个驱动认领的设备数
    pub matched: usize,
    /// 成功挂接的设备数
    pub attached: usize,
    /// 认领了但挂接失败的设备数
    pub failed: usize,
}

impl ProbeStats {
    /// 所有被认领的设备都挂接成功了。
    pub fn all_attached(&self) -> bool {
        self.matched == self.attached && self.failed == 0
    }
}

/// 注册所有内置驱动（v0.8：virtio-blk；v0.9：virtio-net；v1.0：ahci）。
///
/// 注册顺序即匹配顺序：blk 在前、net 其次、ahci 最后；三者匹配的判据不重叠
///（legacy blk = 厂商 `0x1af4` / 设备 `0x1001`；legacy net = 同厂商 / `0x1000`；
/// ahci 只看类别 `0x01`/`0x06`/`0x01`），所以顺序不影响结果。
pub fn register_builtin() -> Result<(), &'static str> {
    register(&crate::driver::virtio_blk::VIRTIO_BLK_DRIVER)?;
    register(&crate::driver::virtio_net::VIRTIO_NET_DRIVER)?;
    register(&crate::driver::ahci::AHCI_DRIVER)?;
    Ok(())
}

/// 登记驱动表里每个驱动的名字（启动日志，测试用它证明「注册真的发生了」）。
pub fn report() {
    for index in 0..count() {
        if let Some(driver) = get(index) {
            serial::print("driver: registered index=");
            serial::print_dec(index as u64);
            serial::print(" name=");
            serial::println(driver.name());
        }
    }
}

/// 遍历 PCI 设备表：为每个设备找驱动并挂接，返回统计。
///
/// 逐设备打印一行 `driver: match bdf -> name`，成功挂接再打 `driver: attached ...`，
/// 失败打 `driver: attach FAILED bdf -> name: reason`。
pub fn probe_all() -> ProbeStats {
    let mut stats = ProbeStats::default();
    for dev in pci::devices() {
        stats.devices += 1;
        let Some(driver) = driver_for(dev) else {
            continue;
        };
        stats.matched += 1;
        serial::print("driver: match ");
        pci::print_bdf(dev.bus, dev.device, dev.function);
        serial::print(" -> ");
        serial::println(driver.name());

        match driver.attach(dev) {
            Ok(()) => {
                stats.attached += 1;
                serial::print("driver: attached ");
                pci::print_bdf(dev.bus, dev.device, dev.function);
                serial::print(" -> ");
                serial::println(driver.name());
            }
            Err(err) => {
                stats.failed += 1;
                serial::print("driver: attach FAILED ");
                pci::print_bdf(dev.bus, dev.device, dev.function);
                serial::print(" -> ");
                serial::print(driver.name());
                serial::print(": ");
                serial::println(err);
            }
        }
    }
    stats
}

/// 按 PCI 设备反查「哪个驱动管它」（诊断与自测用；纯匹配，不产生副作用）。
pub fn driver_for(dev: &PciDevice) -> Option<&'static dyn Driver> {
    (0..count()).find_map(|index| get(index).filter(|driver| driver.matches(dev)))
}
