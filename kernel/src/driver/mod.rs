//! 驱动模块
//!
//! | 模块 | 内容 | 版本 |
//! | ---- | ---- | ---- |
//! | [`serial`] | 16550 UART（COM1）：开发期唯一日志通道 | v0.1 |
//! | [`keyboard`] | PS/2 键盘：IRQ1 扫描码 → set 1 解码 → 串口回显 | v0.4 |
//! | [`pci`] | PCI 枚举（`0xCF8`/`0xCFC` 机制 #1 + BAR 探测） | v0.8 |
//! | [`driver`] | 驱动注册框架：`Driver` trait + 匹配 + 挂接统计 | v0.8 |
//! | [`block`] | 块设备抽象（按扇区读写）与设备注册表 | v0.8 |
//! | [`virtio_blk`] | legacy virtio-blk 0.9.5 驱动（同步轮询 + bounce buffer） | v0.8 |
//! | [`net`] | 网络设备抽象（以太网帧收发）与注册表 | v0.9 |
//! | [`virtio_net`] | legacy virtio-net 网卡驱动（RX/TX 双队列 + 同步轮询） | v0.9 |
//! | [`ahci`] | AHCI/SATA 块设备驱动（真实机器上的磁盘控制器；命令列表 + FIS + PRDT） | v1.0 |
//!
//! 启动顺序（见 `main.rs`）：内存/堆 → PCI 枚举 → 注册内置驱动 → `probe_all()` 挂接
//! → 块设备自测（含 AHCI 盘）→ 网络自测。驱动只在启动阶段注册，之后驱动表与设备表都是只读的。
pub mod ahci;
pub mod block;
pub mod driver;
pub mod keyboard;
pub mod net;
pub mod pci;
pub mod serial;
pub mod virtio_blk;
pub mod virtio_net;
