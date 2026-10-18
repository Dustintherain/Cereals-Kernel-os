//! IRQ 分发（v0.4 实现）
//!
//! 目标：建立最小可验证的 IRQ 分发语义，同时为后续所有 IRQ 提供注册表骨架。
//! - 汇编 IRQ stub（向量 32..47）统一跳入 `irq_dispatch(vector)`；
//! - `irq_dispatch` 根据向量号查找已注册的 handler 并调用；
//! - 处理完成后发送控制器 EOI（主片 + 必要时从片）；
//! - IRQ0（PIT）负责递增内核 tick，并按节流打印心跳；
//! - IRQ1（PS/2 键盘）负责读取扫描码并回显到串口。
//!
//! 重入约定：
//! - IDT 中 IRQ 使用中断门（type 0xE），进入时 CPU 自动清 IF，
//!   因此 IRQ handler 之间不会嵌套，串口输出也不会被另一个 IRQ 打断。
//! - 但主循环（`kernel_main`）与 IRQ handler 都会写串口，
//!   故 IRQ handler 打印时仍通过 `SERIAL_DEBUG_IN_PROGRESS` 做标记，
//!   主循环在打印前可用 `serial_debug_enter` 让出。

use crate::interrupt::controller;
use core::cell::UnsafeCell;

/// 向量号对应的 IRQ 处理回调类型。
///
/// 使用 `unsafe extern "C" fn(u8)` 以匹配中断上下文入口 ABI。
pub type IrqHandler = unsafe extern "C" fn(u8);

/// PIC 兼容映射下可用 IRQ 数量（IRQ0..IRQ15）。
pub const IRQ_COUNT: usize = 16;

/// IRQ 处理器注册表（单核、初始化阶段写入、中断上下文只读）。
pub static IRQ_REGISTRY: IrqRegistry = IrqRegistry::new();

/// IRQ 注册表：向量 `IRQ_BASE_VECTOR + n` 对应槽位 `n`。
pub struct IrqRegistry {
    slots: UnsafeCell<[Option<IrqHandler>; IRQ_COUNT]>,
}

// 开发期单核内核的最小可用写法：注册只在初始化阶段发生，分发只读。
unsafe impl Sync for IrqRegistry {}

impl IrqRegistry {
    const fn new() -> Self {
        Self {
            slots: UnsafeCell::new([None; IRQ_COUNT]),
        }
    }

    /// 为指定向量号注册 IRQ 回调。
    ///
    /// # Safety
    ///
    /// 仅在初始化阶段（单核、尚未开启对应 IRQ）调用。
    pub unsafe fn register(&self, vector: u8, handler: IrqHandler) {
        let index = match controller::vector_to_irq(vector) {
            Some(irq) => irq as usize,
            None => panic!("irq_registry: 向量 {vector} 不在 IRQ 范围 32..47"),
        };
        let slots = unsafe { &mut *self.slots.get() };
        if slots[index].is_some() {
            panic!("irq_registry: 向量 {vector} 已注册过");
        }
        slots[index] = Some(handler);
    }

    /// 将指定向量号分发到已注册的回调（若存在）。
    ///
    /// # Safety
    ///
    /// 仅由中断上下文（`irq_dispatch`）调用。
    pub unsafe fn dispatch(&self, vector: u8) {
        let Some(irq) = controller::vector_to_irq(vector) else {
            return;
        };
        let slots = unsafe { &*self.slots.get() };
        if let Some(handler) = slots[irq as usize] {
            handler(vector);
        }
    }

    /// 指定向量号是否已注册回调。
    #[allow(dead_code)]
    pub unsafe fn is_registered(&self, vector: u8) -> bool {
        match controller::vector_to_irq(vector) {
            Some(irq) => unsafe { (&*self.slots.get())[irq as usize].is_some() },
            None => false,
        }
    }

    /// 已注册的 IRQ 数量（调试/自检用）。
    #[allow(dead_code)]
    pub unsafe fn registered_count(&self) -> usize {
        let slots = unsafe { &*self.slots.get() };
        slots.iter().filter(|s| s.is_some()).count()
    }
}

/// 串口非重入保护标记（开发期辅助）。
///
/// 串口中断保持关闭，但主循环与 IRQ handler 都会写串口，
/// 故用一个标记协调，避免两者的输出互相穿插。
#[repr(transparent)]
pub struct SerialDebugCell(UnsafeCell<bool>);

unsafe impl Sync for SerialDebugCell {}

#[no_mangle]
pub static SERIAL_DEBUG_IN_PROGRESS: SerialDebugCell = SerialDebugCell(UnsafeCell::new(false));

/// 尝试进入串口输出临界区；返回 `false` 表示已有持锁者，调用方应放弃本次输出。
///
/// # Safety
///
/// 仅在单核、无嵌套串口输出的前提下调用，且必须与 `serial_debug_exit` 配对。
pub unsafe fn serial_debug_enter() -> bool {
    let cell = unsafe { &mut *SERIAL_DEBUG_IN_PROGRESS.0.get() };
    if *cell {
        return false;
    }
    *cell = true;
    true
}

/// 退出串口输出临界区。
///
/// # Safety
///
/// 必须与 `serial_debug_enter` 成对调用。
pub unsafe fn serial_debug_exit() {
    let cell = unsafe { &mut *SERIAL_DEBUG_IN_PROGRESS.0.get() };
    *cell = false;
}

/// IRQ 分发入口（由汇编 IRQ stub 跳入，`irq_common` 已保存全部通用寄存器）。
///
/// 参数是 `irq_common` 构造的完整中断帧指针（布局同 [`ExceptionFrame`]），
/// 向量号取自 `frame.vector`。
///
/// [`ExceptionFrame`]: crate::arch::x86_64::interrupt::ExceptionFrame
///
/// # Safety
///
/// 仅由中断上下文（`irq_common`）调用，且 `frame` 必须指向合法的中断帧。
#[no_mangle]
pub unsafe extern "C" fn irq_dispatch(frame: *mut crate::arch::x86_64::interrupt::ExceptionFrame) {
    let vector = unsafe { (*frame).vector } as u8;

    // EOI 必须在分发**之前**发出。
    // 原因：IRQ0 回调会驱动 v0.5 调度器，可能在中断上下文内切换到其它任务；
    // 该调用栈要到很晚才恢复，若 EOI 放在切换之后，PIC 将不再递交后续 IRQ0，
    // 调度与心跳都会停摆。中断门已清 IF，故 EOI 提前不会有嵌套风险。
    unsafe {
        controller::eoi_with_vector_static(vector);
    }

    // 分发到已注册的 IRQ 回调。
    IRQ_REGISTRY.dispatch(vector);
}

/// 注册 IRQ0（PIT）回调。
pub fn register_irq0_pit_callback() {
    unsafe {
        IRQ_REGISTRY.register(controller::IRQ_BASE_VECTOR, irq0_callback);
    }
}

/// 注册键盘（IRQ1）回调。
///
/// 若 IRQ1 向量已被注册，本函数会触发开发期 panic（显式失败优于静默覆盖）。
pub fn register_keyboard_callback() {
    unsafe {
        let kb_vector = controller::irq_to_vector(1).expect("IRQ1 应在 IRQ 范围内");
        IRQ_REGISTRY.register(kb_vector, crate::driver::keyboard::keyboard_irq1_handler);
    }
}

/// IRQ0 回调：递增内核 tick，并按节流打印心跳。
///
/// 心跳节流：每 `HEARTBEAT_INTERVAL_TICKS` 个 tick 打印一次，避免刷屏。
/// 串口输出在 `serial_debug_enter` 失败时直接跳过，避免与主循环输出穿插。
unsafe extern "C" fn irq0_callback(vector: u8) {
    let _ = vector;

    let tick = unsafe {
        crate::time::clock::inc_tick();
        crate::time::clock::tick_count()
    };

    // 心跳只做节流打印，不能提前 return —— 每个 tick 都必须进入调度记账。
    // 只在行边界输出：前台日志是多次 `print` 拼出来的一行，插在中间会把
    // 串口上的断言子串劈断（`user: prIRQ0_heartbeat…ogram=/init`）。
    if tick % HEARTBEAT_INTERVAL_TICKS == 0 && !crate::driver::serial::line_open() {
        if unsafe { serial_debug_enter() } {
            crate::driver::serial::print("IRQ0_heartbeat tick=");
            crate::driver::serial::print_dec(tick);
            crate::driver::serial::print("\r\n");
            unsafe { serial_debug_exit() };
        }
    }

    // v0.5：时间片记账与轮转（可能切换到其它任务）。
    // IRQ0 的 EOI 已由 `irq_dispatch` 在分发前发出，因此这里切换不会丢中断。
    unsafe { crate::scheduler::scheduler::on_tick() };
}

/// PIT 心跳打印间隔（tick 数）：100Hz 下 100 tick = 1 秒。
const HEARTBEAT_INTERVAL_TICKS: u64 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    // 说明：kernel crate 是 `no_std` 二进制目标，本体无法直接 `cargo test`。
    // 这里的断言仅用于文档化分发语义；实际验证走 QEMU 串口集成测试。
    #[test]
    fn irq_count_matches_pic_mapping() {
        assert_eq!(IRQ_COUNT, 16);
    }
}
