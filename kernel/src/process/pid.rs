//! PID 分配与回收（v0.5/v0.6）

#![allow(dead_code)]

/// PID 上限（超过后回绕）。
pub const PID_MAX: u64 = 4096;

/// 下一个待分配的 PID。0 号保留给启动上下文（`kernel_main`）。
static mut NEXT_PID: u64 = 0;

/// 被显式保留、分配器必须跳过的 PID 表容量。
const RESERVED_MAX: usize = 16;
/// 保留的 PID（顶层用户程序的号是内核↔用户态契约，不能被 `fork` 再发一次）。
static mut RESERVED: [u64; RESERVED_MAX] = [0; RESERVED_MAX];
/// `RESERVED` 里的有效项数。
static mut RESERVED_LEN: usize = 0;

/// 保留一个 PID：之后 [`alloc`] 永远不会再发出它。
///
/// 为什么需要：`user.rs` 的顶层程序用固定 PID 1/3/4/5 运行（测试与 `ps` 都按它
/// 断言），而 `fork` 走的是本分配器 —— 两套号源必须显式合并，否则分配器会把
/// 正在使用的 `3` 再发给第三个 `fork` 子进程。
pub fn reserve(pid: u64) {
    unsafe {
        let len = *(&raw const RESERVED_LEN);
        if len >= RESERVED_MAX || is_reserved(pid) {
            return;
        }
        let table = &mut *(&raw mut RESERVED);
        table[len] = pid;
        *(&raw mut RESERVED_LEN) = len + 1;
    }
}

/// `pid` 是否已被 [`reserve`] 保留。
fn is_reserved(pid: u64) -> bool {
    unsafe {
        let len = (*(&raw const RESERVED_LEN)).min(RESERVED_MAX);
        let table = &*(&raw const RESERVED);
        table[..len].contains(&pid)
    }
}

/// 分配一个 PID，跳过被 [`reserve`] 保留的号。
pub fn alloc() -> u64 {
    unsafe {
        // 上界 `PID_MAX` 只防“整段号段都被保留”时死循环；正常路径第一次就返回。
        for _ in 0..PID_MAX {
            let pid = NEXT_PID;
            NEXT_PID = if NEXT_PID + 1 >= PID_MAX { 1 } else { NEXT_PID + 1 };
            if !is_reserved(pid) {
                return pid;
            }
        }
        1 // 理论上不可达（只保留了 0..=5）
    }
}

/// 重置分配器（仅供自测/重启路径调用）。
pub fn reset() {
    unsafe { NEXT_PID = 0 }
}

/// 当前运行线程的 PID。v0.6 中内核线程与轻量进程一一对应，调度器在
/// 调度切换时把 `cur_pid` 置回；若无当前任务，返回 0。
pub fn current_task() -> u64 {
    unsafe {
        if !crate::process::thread::CURRENT_THREAD.is_null() {
            let t = &*crate::process::thread::CURRENT_THREAD;
            t.pid
        } else {
            0
        }
    }
}

/// 返回当前运行任务的 PID。若无当前任务，返回 `None`。
pub fn current() -> Option<u64> {
    Some(current_task())
}
