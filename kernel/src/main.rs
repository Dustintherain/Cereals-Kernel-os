//! Cerlesse OS 内核入口（v0.1 实现；v0.2 GDT/IDT；v0.3 内存管理；v0.4 中断/Timer/键盘）
//!
//! 启动链：UEFI Bootloader 加载内核 ELF → ExitBootServices → 跳转 `_start`，
//! RDI 指向 `shared::BootInfo`。本文件建立自己的栈并进入 `kernel_main`。
//!
//! v0.4 验收（串口观测）：
//! - `PIC initialized` / `keyboard initialized` / `KBIRQ enabled`：中断与外设初始化完成；
//! - `IRQ0_heartbeat tick=<n>`：PIT（100Hz）每 100 tick（1 秒）打印一次，tick 递增；
//! - `KBIRQ ENTRY` / `KB a` / `KBIRQ EXIT`：按键经 IRQ1 → 扫描码解码 → 串口回显。
//!
//! 初始化完成后进入 `hlt` 空闲循环，中断由硬件持续递交（不再是阻塞式观测窗口）。

#![no_std]
#![no_main]

extern crate alloc;

mod arch;
mod ckm;
mod driver;
mod fs;
mod interrupt;
mod ipc;
mod memory;
mod network;
mod process;
mod scheduler;
mod security;
mod storage;
mod time;

type KernelTaskEntry = extern "C" fn() -> !;

extern "C" fn kernel_task_a() -> ! {
    driver::serial::println("task_a: entered");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

extern "C" fn kernel_task_b() -> ! {
    driver::serial::println("task_b: entered");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

extern "C" fn kernel_task_c() -> ! {
    driver::serial::println("task_c: entered");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack, preserves_flags)); }
    }
}

/// v0.6 系统调用子系统（syscall 入口、编号分发、首批 syscall handler）
mod syscall;

use core::arch::global_asm;
use shared::BootInfo;

// 栈与入口：bootloader 以 RDI = &BootInfo 跳入 _start。
global_asm!(
    ".section .bss",
    ".align 16",
    ".global stack_bottom",
    "stack_bottom:",
    ".space 65536",
    ".global stack_top",
    "stack_top:",
    ".text",
    ".global _start",
    "_start:",
    "cli", // B-01修复：入口即关中断，关闭固件 IDT 下的中断窗口
    "lea rsp, [rip + stack_top]",
    "xor rbp, rbp",
    "call kernel_main",
    "hlt_loop:",
    "hlt",
    "jmp hlt_loop",
);

#[no_mangle]
pub extern "C" fn kernel_main(boot_info: *const BootInfo) -> ! {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }
    driver::serial::init();
    driver::serial::println("Cerlesse kernel v0.4");

    // 自建 GDT（含 TSS）+ IDT，接管异常处理
    arch::x86_64::init();
    driver::serial::println("GDT/TSS + IDT loaded");

    // v0.4：中断控制器 → PIT/IRQ0 → 键盘/IRQ1 → 打开 IF。
    // 注意：所有 IRQ 回调必须在 `enable_irqs()` 之前注册完毕，
    // 否则第一次中断会分发给空槽位（当前实现会安全丢弃，但属于配置错误）。
    {
        let mut controller = interrupt::controller::Controller::new(
            interrupt::controller::ControllerKind::Pic,
        );
        controller.init_pic();
        driver::serial::println("PIC initialized");

        // PIT 通道 0：100Hz（10ms/tick），供 tick 计数与心跳。
        unsafe { time::timer::set_channel0_reload(time::timer::TICK_RELOAD_100HZ) };
        controller.enable_irq(0);

        // 键盘控制器初始化（排空输出缓冲 + 打开 IRQ1 使能位），
        // 必须在启用 PIC 的 IRQ1 之前完成，否则残留字节会产生伪中断。
        unsafe { driver::keyboard::init() };
        driver::serial::println("keyboard initialized");

        controller.enable_irq(1);
        driver::serial::println("KBIRQ enabled");

        // 注册回调：IRQ0（PIT tick + 心跳）、IRQ1（键盘回显）。
        interrupt::irq::register_irq0_pit_callback();
        interrupt::irq::register_keyboard_callback();

        // 打开处理器中断标志（IF），PIT 与键盘中断开始递交。
        unsafe { arch::x86_64::enable_irqs() };
        driver::serial::println("PIT 100Hz + PIC IRQ0/IRQ1 unmasked; IF enabled");
    }

    if boot_info.is_null() {
        driver::serial::println("panic: null BootInfo");
        halt();
    }
    // SAFETY: 非空指针，由 bootloader 按 shared::BootInfo 布局构造。
    let bi = unsafe { &*boot_info };
    if bi.magic != shared::BOOT_MAGIC {
        driver::serial::println("panic: bad BootInfo magic");
        halt();
    }

    driver::serial::print("memory map: ");
    driver::serial::print_hex(bi.memory_map_size as u64);
    driver::serial::print(" bytes, desc size ");
    driver::serial::print_hex(bi.memory_descriptor_size as u64);
    driver::serial::println("");

    // v0.3：帧分配器 → 内核堆 → 自测（heap/frame/paging）
    if let Err(err) = memory::init(bi) {
        driver::serial::print("mem: FAIL: ");
        driver::serial::println(err);
        halt();
    }


    // v0.4：内核级上下文切换原语自测（为 v0.5 的任务/调度铺路）。
    if let Err(err) = arch::x86_64::context::selftest() {
        driver::serial::print("context: FAIL: ");
        driver::serial::println(err);
        halt();
    }
    driver::serial::println("context: switch PASS");

    // v0.7：VFS + RAMFS（RAMFS 用内核堆保存文件内容，因此必须在 memory::init 之后）。
    // `/init` 与 `/bin/*` 的内容都是内嵌的用户程序 ELF，`syscall::user` 稍后
    // 从 VFS 读它们并交给 ELF Loader（v0.9 的 Shell 走的也是这条普通路径）。
    // `library_images()` 是 v0.9 动态链接的解释器与共享库（落在 `/lib`）。
    if let Err(errno) = fs::init(
        syscall::user::program_images(),
        syscall::user::library_images(),
    ) {
        driver::serial::print("fs: FAIL errno=");
        driver::serial::print_dec(errno.unsigned_abs());
        driver::serial::println("");
        halt();
    }
    if let Err(err) = fs::selftest() {
        driver::serial::print("fs: FAIL: ");
        driver::serial::println(err);
        halt();
    }

    // CKM-1：凭证 / 能力位 / 判定点 / 审计环就位（`docs/ckm.md`）。
    //
    // 位置在 `fs::init` 之后、任何用户进程之前：
    // - 之后才能保证「启动早期的 open/execve 也经过判定」（否则存在一段无判定窗口）；
    // - `fs::init` 之前不行：`ckm::init` 会打印证据行，而串口与 RAMFS 都已就绪更便于定位。
    // 自测里的「非法转换」「审计回绕」必须真跑过一遍（它们是机制而非展示）。
    ckm::api::init();
    if let Err(err) = ckm::api::selftest() {
        driver::serial::print("ckm: FAIL: ");
        driver::serial::println(err);
        halt();
    }

    // CKM-2（第一步）：覆盖层机制 —— 注册/撤销真的改变 `open` 的结果。
    //
    // 位置在 `ckm::api::init` 之后（覆盖表的判定点 7/8 会写审计环，环必须先建好）、
    // 用户进程之前（用户看到的必须是**空表 + 基线视图**，见 `docs/ckm.md` 6.4）。
    // 自测的**文件动作**在 `fs/` 侧发起：`ckm/` 不依赖 `fs/`，方向是 `fs/ ──▶ ckm/`。
    if let Err(err) = fs::overlay_selftest() {
        driver::serial::print("fs: overlay FAIL: ");
        driver::serial::println(err);
        halt();
    }

    driver::serial::println("Kernel started!");

    // v0.4：观测 IF 状态，确认中断使能路径已接通。
    {
        let enabled = arch::x86_64::irqs_enabled();
        driver::serial::print("IF=");
        driver::serial::print_dec(if enabled { 1u64 } else { 0u64 });
        driver::serial::println("");
    }

    // v0.8：PCI 枚举 → 驱动注册 → 探测/挂接 → 块设备验收。
    //
    // 位置在 `Kernel started!` 之后、调度器/用户态之前：块设备是 v0.8 磁盘文件系统
    //（FAT32）的地基，而它与调度器、用户态都没有相互依赖 —— 放在这里失败原因也最好定位。
    //
    // 启动盘本身就是这块 virtio-blk（见 Makefile / tests/*.py 的 QEMU 命令行），
    // 所以「内核能启动」与「驱动能读写这块盘」验证的是同一台设备，不是两条并行路径。
    {
        let devices = driver::pci::init();
        driver::serial::print("pci: enumerated devices=");
        driver::serial::print_dec(devices as u64);
        driver::serial::println("");

        // 启动盘所在的 legacy virtio-blk 必须出现在总线上（否则后面的探测没什么可挂）。
        let Some(blk) = driver::pci::find_by_id(
            driver::virtio_blk::VIRTIO_VENDOR,
            driver::virtio_blk::VIRTIO_BLK_LEGACY,
        ) else {
            driver::serial::println("pci: FAIL: no legacy virtio-blk device on the bus");
            halt();
        };
        let (bus, device, function) = blk.location();
        driver::serial::print("pci: virtio-blk (legacy) at ");
        driver::pci::print_bdf(bus, device, function);
        driver::serial::print(" class=");
        driver::serial::print_hex(blk.class as u64);
        driver::serial::print(" subclass=");
        driver::serial::print_hex(blk.subclass as u64);
        driver::serial::println("");

        if let Err(err) = driver::driver::register_builtin() {
            driver::serial::print("driver: FAIL: ");
            driver::serial::println(err);
            halt();
        }
        driver::driver::report();

        let stats = driver::driver::probe_all();
        let ok = stats.attached > 0 && stats.all_attached();
        driver::serial::print(if ok {
            "driver: probe PASS (devices="
        } else {
            "driver: probe FAIL (devices="
        });
        driver::serial::print_dec(stats.devices as u64);
        driver::serial::print(" matched=");
        driver::serial::print_dec(stats.matched as u64);
        driver::serial::print(" attached=");
        driver::serial::print_dec(stats.attached as u64);
        driver::serial::print(" failed=");
        driver::serial::print_dec(stats.failed as u64);
        driver::serial::println(")");
        if !ok {
            halt();
        }

        driver::serial::print("block: registered devices=");
        driver::serial::print_dec(driver::block::count() as u64);
        driver::serial::println("");
        let Some(disk) = driver::block::first() else {
            driver::serial::println("block: FAIL: no block device registered");
            halt();
        };
        if let Err(err) = driver::block::selftest(disk) {
            driver::serial::print("block: FAIL: ");
            driver::serial::println(err);
            halt();
        }
    }

    // 驱动兼容性（v1.0）：AHCI/SATA —— 真实机器上的磁盘控制器。
    //
    // 端口上真的有盘时，跑的是 `driver::block::selftest` —— **与 virtio-blk
    // 同一套验收**（引导扇区 / 跨搬运边界的分块读 / 末扇区写回读后还原 / 负例），
    // 因此「SATA 这条路也能读写」不是日志措辞，而是同一批断言在另一条路径上成立。
    // 没有盘时不跑（总线上有控制器、没插盘是常态），并明确打印出来。
    if let Some(index) = driver::ahci::block_index() {
        driver::serial::print("ahci: running the block selftest on the SATA disk (block index=");
        driver::serial::print_dec(index as u64);
        driver::serial::println(")");
        let Some(sata) = driver::block::get(index) else {
            driver::serial::println("ahci: FAIL: the SATA disk is missing from the block registry");
            halt();
        };
        if let Err(err) = driver::block::selftest(sata) {
            driver::serial::print("ahci: FAIL: ");
            driver::serial::println(err);
            halt();
        }
    } else {
        driver::serial::println("ahci: no SATA drive attached; skipping the SATA block selftest");
    }

    // CKM-2（第二步）：可写持久化状态区 —— 系统盘尾部 1 MiB 的双槽 A/B 区。
    //
    // 位置必须在块设备注册（驱动探测）之后：状态区是**扇区级**的原始区，不在任何
    // 文件系统里（定夺与布局见 `docs/ckm.md` 第 12 节与 `ckm/store.rs`）。
    // 因此它比文件系统挂载更早可用，也不需要先有可写文件系统（那是 CSS-4）。
    //
    // **CSS-3 起它比卷扫描更早**：盘符（`storage/letters`）等存储偏好要在
    // `storage::init` 枚举卷时就已知，否则 `storage: vol … letter=` 证据行会
    // 先打出一组缺省字母、再被盘上的映射推翻 —— 启动日志就成了两份真相。
    ckm::store::init();

    // CSS-1（H-08）：块设备之上的分区/卷只读识别 —— 先看清「有哪些卷」，再谈挂载。
    // 位置在块设备验收之后、`fs::mount_disks` 之前：扫描是只读的，失败不会残留在盘上。
    // `storage::init` 内部先装载 `storage/letters`（CSS-3），因此依赖上面的状态区。
    if let Err(err) = storage::init() {
        driver::serial::print("storage: FAIL: ");
        driver::serial::println(err);
        halt();
    }
    if let Err(err) = storage::selftest() {
        driver::serial::print("storage: FAIL: ");
        driver::serial::println(err);
        halt();
    }

    if let Err(err) = ckm::store::selftest() {
        driver::serial::print("ckm: store FAIL: ");
        driver::serial::println(err);
        halt();
    }
    // CSS-3：盘符缺省规则 + 自定义分配 + 持久化往返（证据见 `make test-storage`）。
    // 必须在状态区自测之后：它会把自定义映射写进同一个区（`storage/letters`）——
    // 「同一个区被第二个消费者使用」这件事本身因此是可断言的。
    if let Err(err) = storage::letters::selftest() {
        driver::serial::print("storage: letters FAIL: ");
        driver::serial::println(err);
        halt();
    }

    // v0.8：把块设备上的 FAT 卷挂进 VFS（`/disk` = 启动盘、`/mnt/fat` = mkfs 生成的测试盘），
    // 然后在**文件系统层**做一次验收 —— syscall 层不需要任何改动就能 open/read 磁盘上的文件。
    if let Err(errno) = fs::mount_disks() {
        driver::serial::print("fs: disk mount FAIL errno=");
        driver::serial::print_dec(errno.unsigned_abs());
        driver::serial::println("");
        halt();
    }
    if let Err(err) = fs::disk_selftest() {
        driver::serial::print("fs: disk FAIL: ");
        driver::serial::println(err);
        halt();
    }
    // CSS-2：挂载收口自测 —— 只读原因可读、记录被消费者读对、同一卷多挂载点、卸载。
    // 位置在 `disk_selftest` 之后（它已经证明 `/disk` 上有真文件可读）。
    if let Err(err) = fs::storage_mount_selftest() {
        driver::serial::print("fs: storage mount FAIL: ");
        driver::serial::println(err);
        halt();
    }

    // v0.9：简化网络栈验收（Ethernet → ARP → IPv4 → UDP）。
    //
    // 只有在总线上真的挂了一块网卡（`make test-network` 的 QEMU 加了 virtio-net-pci）
    // 时才跑；其它测试不带网卡，走到这里就跳过 —— 不把「没有网卡」当成失败。
    // 断言的是网络里回来的真实内容（ARP 应答的 MAC、DHCP OFFER 的 yiaddr/server），
    // 见 `network::selftest`。
    if driver::net::count() == 0 {
        driver::serial::println("net: no network device present; skipping network selftest");
    } else if let Some(dev) = driver::net::first() {
        driver::net::report();
        if let Err(err) = network::selftest(dev) {
            driver::serial::print("net: FAIL: ");
            driver::serial::println(err);
            halt();
        }
    }


    // v0.2 验收：异常注入测试（feature 门控，默认构建不触发）
    #[cfg(feature = "exception-test")]
    {
        driver::serial::println("test: triggering divide error (#DE)");
        unsafe {
            core::arch::asm!(
                "xor eax, eax",
                "div eax",
                lateout("eax") _,
                lateout("edx") _,
                options(nostack),
            );
        }
    }

    // v0.3 验收：页错误诊断测试（feature 门控，默认构建不触发）
    #[cfg(feature = "pagefault-test")]
    {
        const UNMAPPED: u64 = 0x0000_5000_0000_0000;
        driver::serial::println("test: touching unmapped address");
        unsafe {
            core::ptr::write_volatile(UNMAPPED as *mut u8, 0xAA);
        }
    }

    // v0.5：内核线程 + Round Robin 调度（PIT/IRQ0 驱动时间片抢占）。
    {
        use crate::scheduler::scheduler as sched;

        sched::init();
        // v0.5 内核线程仍然使用旧入口名称 `task_a/task_b/task_c`；
        // 它们在本版本保留为占位，由下一阶段接入用户态隔离后逐步替换。
        sched::spawn("task_a", kernel_task_a as KernelTaskEntry);
        sched::spawn("task_b", kernel_task_b as KernelTaskEntry);
        sched::spawn("task_c", kernel_task_c as KernelTaskEntry);
        driver::serial::println("sched: tasks spawned; starting round robin");

        // 切出到第一个内核任务；本轮 Round Robin 绕回 pid 0 时从这里继续。
        sched::start();

        let stats = sched::stats();
        sched::report();
        if stats.started && stats.switches >= 4 {
            driver::serial::println("sched: round-robin wrap PASS");
        } else {
            driver::serial::println("sched: round-robin wrap FAIL");
            halt();
        }
    }

    // v0.5：IPC（pipe 骨架）验收自测——在 Round Robin 巡回之后执行，
    // 同时验证管道语义与「不给调度器引入新崩溃面」。
    if let Err(err) = ipc::pipe::selftest() {
        driver::serial::print("pipe: FAIL: ");
        driver::serial::println(err);
        halt();
    }
    driver::serial::println("pipe: selftest PASS");

    // v0.6：syscall + 用户态隔离验收。
    //
    // 进入用户态前必须挂起调度器：用户程序不在调度器任务表里，
    // 若 PIT 中断在用户态上下文里切到内核任务，那个任务会在
    // 「用户页表 + 用户栈」的中间状态上恢复执行。
    {
        use crate::scheduler::scheduler as sched;
        sched::suspend();
        driver::serial::println("sched: suspended for user-mode run");
    }
    if let Err(err) = syscall::user::selftest() {
        driver::serial::print("user: FAIL: ");
        driver::serial::println(err);
        halt();
    }

    // 之后的执行由调度器接管：每个任务在 hlt 中等待下一个时间片。
    halt()
}

pub(crate) fn halt() -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    driver::serial::println("KERNEL PANIC");
    if let Some(loc) = info.location() {
        driver::serial::print(loc.file());
        driver::serial::print(":");
        driver::serial::print_hex(loc.line() as u64);
        driver::serial::println("");
    }
    // 消息必须打出来：只报位置的话，「索引越界」与「unwrap 到 None」
    // 在串口上长得一模一样，定位要多花一整轮重启。
    struct SerialText {
        budget: usize,
    }
    impl core::fmt::Write for SerialText {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            for b in s.bytes() {
                if self.budget == 0 {
                    return Err(core::fmt::Error);
                }
                self.budget -= 1;
                if (0x20..0x7f).contains(&b) {
                    driver::serial::putc(b);
                } else {
                    // 不可打印字节用 `\xNN` 转义：长度异常的 `&str` 就落在这里，
                    // 直接 `print` 会把终端搞乱。
                    driver::serial::print("\\x");
                    driver::serial::print_hex_pad(b as u64, 2);
                }
            }
            Ok(())
        }
    }
    driver::serial::print("panic: ");
    // 消息要打出来，但**限长**：一个长度异常的 `&str` 会把串口灌爆
    // （实测教训：几 MB 的消息比没有消息更难定位）。
    let mut sink = SerialText { budget: 160 };
    let complete = core::fmt::Write::write_fmt(&mut sink, format_args!("{}", info.message())).is_ok();
    if !complete {
        driver::serial::print("...");
    }
    driver::serial::println("");
    halt()
}
