//! 进程类系统调用（v0.6 首批，v0.9 第三步补上 `fork` / `execve` / `wait4`）
//!
//! v0.6/v0.7 的用户态是「一次跑一个用户程序，`exit` 即回到内核」。本模块把这一步
//! 补齐成真正的进程语义：每个用户程序是一个 [`Process`](crate::process::process::Process)
//! （独立地址空间 + 独立内核栈 + 停在 `syscall` 里的完整寄存器帧），于是
//!
//! | 调用 | 语义 |
//! | ---- | ---- |
//! | `fork()` | 复制当前地址空间与**当前用户帧**，子进程 `rax=0`；在父进程里返回子进程 PID |
//! | `execve(path, argv)` | 用 `path` 的 ELF 替换当前映像（新地址空间 + 新 `argv` 用户栈），**不返回** |
//! | `wait4(pid)` | 运行该子进程直到它退出，回收地址空间并返回其退出码 |
//! | `exit(code)` | 标记僵尸进程并回到「谁在等我」的内核上下文 |
//! | `getpid()` | 当前进程 PID |
//!
//! ## 与 Linux 的差异（刻意，且必须写在文档里）
//!
//! 1. **子进程不会与父进程并发运行**：单 CPU、用户态无抢占，`fork` 之后子进程停在
//!    那一帧里，直到父进程 `wait4` 才被恢复。对 `fork`/`execve`/`wait4` 的**调用方**
//!    来说这是透明的 —— Shell 的 `if pid == 0 { execve } else { wait4 }` 原样可用，
//!    只是「并发」变成了「父进程等着时，子进程在跑」。
//! 2. `fork` 不复制文件描述符表（fd 表仍是全局的），`execve` 前会 `fs::file::reset()`。
//! 3. 没有信号、进程组、`WNOHANG`；`wait4` 只认直接子进程。
//!
//! ## 用户帧是这里的核心数据结构
//!
//! `trampoline::current_frame()` 指向**当前 syscall 保存的完整用户寄存器帧**。
//! `fork` 直接复制它（只改 `rax`），`execve` 用 `UserFrame::new_user` 造一帧新的 ——
//! 两者都直接决定「子进程/新映像第一条指令是什么、寄存器是什么」。

use core::ptr;

use abi::errno::{EAGAIN, ECHILD, EFAULT, EINVAL, ENOENT, ENOSYS};

use crate::arch::x86_64::{cpu, gdt, paging};
use crate::driver::serial;
use crate::fs::vfs;
use crate::memory::page::{align_down, PAGE_SIZE};
use crate::process::pid;
use crate::process::process::{self, ProcState, NO_PROCESS};
use crate::syscall::address_space::{UserSpace, USER_STACK_TOP};
use crate::syscall::trampoline::{self, RunCtx, UserFrame, NO_PROCESS as NO_PROCESS_U64};
use crate::time::clock;

/// `execve` 路径长度上限（**定义在 `abi::limits`**，与 `open`/`getdents` 同一上限）。
const PATH_MAX: u64 = abi::limits::PATH_MAX;
/// `getprocs` 的记录尺寸（**再导出**，定义在 `abi::procinfo`）。
pub use abi::procinfo::PROCINFO_SIZE;
/// `execve` 的 `argv` 项数上限 / 单参数上限 / 用户栈占用上限
/// （**再导出**，定义在 `abi::limits`；用户栈 16 KiB，`argv` 只占一小块）。
pub use abi::limits::{ARG_MAX, ARG_MAX_BYTES, ARG_MAX_LEN};
/// `nanosleep` 的最大等待 tick 数；仅作为「PIT 停摆」时的兜底。
const SLEEP_MAX_SPINS: u64 = 10_000;

/// 系统调用 `exit(code)`：把当前进程标记为僵尸并回到「谁在等我」。
///
/// **不返回**（尾表达式类型为 `!`），声明 `-> i64` 只是为了与
/// [`crate::syscall::syscall_table::SyscallHandler`] 的类型一致。
///
/// 地址空间**不在这里释放**：当前 CR3 还指向它。释放由回收方负责
/// （子进程由 `wait4` 回收，顶层程序由 `syscall::user::run_process` 回收）——
/// 顺序错了就是「释放掉自己正在跑的页表」。
pub unsafe fn sys_exit(args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let code = args[0] as i64;
    serial::print("user: exit code=");
    serial::print_dec(code as u64);
    serial::println("");
    crate::syscall::user::set_exit_code(code);
    if let Some(p) = process::current_mut() {
        p.exit_code = code;
        p.state = ProcState::Zombie;
    }
    unsafe { finish_current() }
}

/// ring3 异常终止路径（由 `interrupt::exception` 的 #PF/#GP handler 调用）。
///
/// 与 `exit` 的区别只在退出码：这里是「被异常杀死」，退出码记 `-1`
/// （v0.6 起 `user::last_exit_code()` 就是用 `-1` 表示「没走到 exit」，
/// `make test-user` 的探针断言依赖它）。
pub unsafe fn terminate_from_fault() -> ! {
    serial::println("user: process terminated by ring3 fault");
    crate::syscall::user::set_exit_code(-1);
    if let Some(p) = process::current_mut() {
        p.exit_code = -1;
        p.state = ProcState::Zombie;
    }
    unsafe { finish_current() }
}

/// 系统调用 `getpid`：返回当前进程的 PID。
pub unsafe fn sys_getpid(_args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let pid = process::current().map(|p| p.pid).unwrap_or(0);
    serial::print("user: getpid -> ");
    serial::print_dec(pid);
    serial::println("");
    pid as i64
}

/// 系统调用 `getprocs(buf_ptr, buf_len)`：把进程表快照写进用户缓冲。
///
/// v0.9 第五步新增（`user/ps` 的落点）。Linux 没有这个调用（`ps` 读 `/proc`），
/// 而本内核的 VFS 还没有虚拟文件 / `/proc`，所以这里直接把进程表交出去 ——
/// 记录格式见下，**改它就是改 ABI**（`libc::PROCINFO_SIZE` 必须一致）：
///
/// ```text
/// 偏移  长度  字段        说明
/// 0     8     pid         LE u64
/// 8     8     ppid        LE u64（0 = 内核启动上下文）
/// 16    1     state       0 = READY，1 = RUNNING，2 = ZOMBIE
/// 17    7     reserved    内核写 0
/// 24    16    comm        `Process::comm`（定长，补 0；无终止符语义）
/// ```
///
/// 返回写入的记录数（`0` = 进程表为空），缓冲装不下时截断；负数 = `-errno`。
/// 枚举的是**槽位顺序**（不是 PID 顺序）：`ps` 因此不需要排序逻辑，内核也不承诺
/// 排序 —— 这就是「进程表长什么样」的如实转发。
pub unsafe fn sys_getprocs(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let (buf_ptr, buf_len) = (args[0], args[1]);
    if (buf_len as usize) < PROCINFO_SIZE {
        return EINVAL;
    }
    let out = match unsafe {
        crate::syscall::file::user_bytes_mut(space, buf_ptr, buf_len, "getprocs")
    } {
        Ok(out) => out,
        Err(errno) => return errno,
    };

    let cap = out.len() / PROCINFO_SIZE;
    let mut count = 0usize;
    for slot in 0..process::MAX_PROC {
        let ptr = process::process_ptr(slot);
        if ptr.is_null() {
            continue;
        }
        if count == cap {
            break;
        }
        let p = unsafe { &*ptr };
        let rec = &mut out[count * PROCINFO_SIZE..(count + 1) * PROCINFO_SIZE];
        rec.fill(0);
        rec[0..8].copy_from_slice(&p.pid.to_le_bytes());
        rec[8..16].copy_from_slice(&p.parent_pid.to_le_bytes());
        rec[16] = match p.state {
            ProcState::Ready => 0,
            ProcState::Running => 1,
            ProcState::Zombie => 2,
        };
        rec[24..24 + process::PROC_COMM_LEN].copy_from_slice(&p.comm);
        count += 1;
    }

    serial::print("user: getprocs count=");
    serial::print_dec(count as u64);
    serial::println("");
    count as i64
}

/// 系统调用 `fork`：复制当前进程。
///
/// 父进程拿到子进程 PID；子进程拿到 **0**（恢复时 `rax` 被写成 0），
/// 且从**同一条用户指令**继续执行 —— 因为子进程的初始帧就是父进程停在
/// `fork` 系统调用里的那一帧的副本（含 `rbx/rbp/r12..r15`，见
/// `trampoline` 模块文档）。
///
/// 失败（地址空间复制失败 / 进程表满）返回负 errno，且**不改变进程表**。
pub unsafe fn sys_fork(_args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let Some(parent) = process::current() else {
        return ENOSYS; // 内核上下文里没有「进程」可复制
    };
    let parent_slot = parent.slot;
    let parent_pid = parent.pid;
    // 凭证沿袭：`fork` 不增加能力（`docs/ckm.md` 3.2 的「能力不可继承性提升」）。
    // 这里先拷一份到局部量，避免在 `process::create` 读写进程表时继续持有父进程引用。
    let parent_cred = parent.cred;
    let child_space = {
        let Some(space) = parent.space.as_ref() else {
            return ENOSYS;
        };
        match space.clone_for_fork() {
            Ok(space) => space,
            Err(err) => {
                serial::print("user: fork address space copy failed errno=");
                serial::print_dec(err.unsigned_abs());
                serial::println("");
                return err;
            }
        }
    };
    let Some(slot) = process::free_slot() else {
        serial::println("user: fork FAIL (process table full)");
        return EAGAIN;
    };

    let pid = pid::alloc();
    // 克隆「停在 syscall 里的父进程帧」，把返回值改成 0。
    let mut frame = unsafe { ptr::read(trampoline::current_frame()) };
    frame.rax = 0;

    let child = process::create(slot, pid, "fork-child", parent_pid, parent_slot)
        .expect("free_slot 返回的槽位必然空闲");
    child.frame = frame;
    child.space = Some(child_space);
    child.cred = parent_cred;

    serial::print("user: fork parent=");
    serial::print_dec(parent_pid);
    serial::print(" -> child pid=");
    serial::print_dec(pid);
    serial::print(" image_pages=");
    serial::print_dec(child.space.as_ref().map_or(0, |s| s.image_pages()) as u64);
    serial::println("");
    pid as i64
}

/// 系统调用 `execve(path, path_len, argv, argc)`：用新映像替换当前进程。
///
/// `argv` 指向用户内存里 `argc` 个 `(ptr, len)` 对（每对 16 字节）—— 与
/// `open`/`write` 一样用「指针 + 长度」而不是 NUL 结尾，内核不需要在不可信
/// 内存里找终止符，且每一段都在写入前经过 [`UserSpace::valid_user_range`] 校验。
///
/// 成功的路径是**不返回**的：换 CR3、换映像、然后 `iretq` 到新入口。
/// 失败返回负 errno，**当前映像不受影响**（新地址空间在局部量里，失败即 `Drop`）。
pub unsafe fn sys_exec(args: &[u64; 5], space: *const UserSpace) -> i64 {
    let slot = process::current_index();
    if slot == NO_PROCESS {
        return ENOSYS; // execve 必须由某个进程发起
    }
    let Some(space) = (unsafe { space.as_ref() }) else {
        return EFAULT;
    };

    let path_ptr = args[0];
    let path_len = args[1];
    let argv_ptr = args[2];
    let argc = args[3];

    // 1. 路径：长度 + 区间校验 → UTF-8 → 规范化
    if path_len == 0 || path_len > PATH_MAX {
        return EINVAL;
    }
    if !space.valid_user_range(path_ptr, path_len) {
        return EFAULT;
    }
    let raw_path = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len as usize) };
    let Ok(path) = core::str::from_utf8(raw_path) else {
        return EINVAL;
    };
    let Ok(path) = vfs::normalize(path) else {
        return EINVAL;
    };

    // 2. argv：数组本身与每个字符串都必须在用户可读区间内
    if argc > ARG_MAX {
        return EINVAL;
    }
    let mut args_list: [&[u8]; ARG_MAX as usize] = [&[]; ARG_MAX as usize];
    if argc > 0 {
        if !space.valid_user_range(argv_ptr, argc * 16) {
            return EFAULT;
        }
        for i in 0..argc {
            let pair = (argv_ptr + i * 16) as *const u64;
            let (ptr, len) = unsafe { (ptr::read(pair), ptr::read(pair.add(1))) };
            if len > ARG_MAX_LEN || !space.valid_user_range(ptr, len) {
                return EFAULT;
            }
            args_list[i as usize] =
                unsafe { core::slice::from_raw_parts(ptr as *const u8, len as usize) };
        }
    }

    // 2.5 判定点 7（`docs/ckm.md` 3.4）：程序文件必须**可读**（mode → `EACCES`）。
    //     读程序文件不需要任何能力位，因此这里**不**调能力判定；
    //     "`execve` 到受保护前缀的程序是否有额外限制" 的答案是“没有”（受保护前缀约束的是**写**）。
    //     先拿一份当前凭证：凭证切换要在“旧身份发起”的语义下记录（见下）。
    let parent_cred = crate::ckm::api::current_cred();
    if let Ok(meta) = vfs::stat(&path) {
        if let Err(errno) = crate::security::permission::check(
            meta.mode,
            meta.uid,
            meta.gid,
            &parent_cred,
            crate::security::permission::Want::Read,
        ) {
            serial::print("fs: EACCES path=");
            serial::print(&path);
            serial::print(" uid=");
            serial::print_dec(parent_cred.uid as u64);
            serial::println(" want=read (execve)");
            return errno;
        }
    }

    // 3. 读 ELF 字节（还没动当前映像）
    let Ok(elf) = vfs::read_all(&path) else {
        serial::print("user: exec ");
        serial::print(&path);
        serial::println(" -> ENOENT (no such program)");
        return ENOENT;
    };

    // 4. 新地址空间：装载 + 构建 argv 用户栈 + 新帧
    let mut new_space = match UserSpace::new() {
        Ok(space) => space,
        Err(err) => return err,
    };
    let stats = match new_space.load_elf(&elf) {
        Ok(stats) => stats,
        Err(err) => return err,
    };
    let frame = match build_user_frame(&new_space, stats.entry, &args_list[..argc as usize]) {
        Ok(frame) => frame,
        Err(err) => return err,
    };

    // 5. 替换映像。**先切 CR3 再丢弃旧地址空间** —— 顺序反了就是释放掉自己
    //    正在使用的页表。
    let new_root = new_space.root();
    // 凭证：命中 CKM 组件表（`/init`、`/bin/sh`、`/bin/hello`）则换成该组件声明的
    // uid/gid/能力，其余完全沿袭父进程（`docs/ckm.md` 3.1）。
    // 在 `process_mut` 之前算：`exec_cred` 内部要读进程表（pid）与写审计/证据。
    let new_cred = crate::ckm::api::exec_cred(&path, parent_cred);
    if new_cred != parent_cred {
        serial::print("ckm: credential switch pid=");
        serial::print_dec(process::process_mut(slot).map(|p| p.pid).unwrap_or(0));
        serial::print(" uid=");
        serial::print_dec(parent_cred.uid as u64);
        serial::print(" -> ");
        serial::print_dec(new_cred.uid as u64);
        serial::print(" caps=");
        serial::print_hex(new_cred.caps as u64);
        serial::println("");
    }
    let old_space;
    {
        let p = process::process_mut(slot).expect("当前进程存在");
        p.frame = frame;
        p.name = "exec";
        // `ps` 看到的是程序名（`ls`），不是 `"exec"` —— `comm` 是定长字段，
        // 因此这里能安全地指向用户给的路径（不需要把 String 泄漏成 `&'static str`）。
        p.set_comm(path.rsplit('/').next().unwrap_or(&path));
        p.cred = new_cred;
        old_space = p.space.replace(new_space);
    }
    unsafe { cpu::write_cr3(new_root) };
    drop(old_space);
    // fd 表与套接字表都是全局的：换映像等于换进程，继承的 fd 全部失效（见模块文档）。
    // 套接字必须一起清：否则新映像能按 fd 16..23 读写旧程序的已连接 TCP 连接，
    // 槽位也会一轮轮泄漏（8 个之后 `socket()` 报 ENOSPC）。
    crate::fs::file::reset();
    crate::network::socket::reset();

    serial::print("user: exec pid=");
    serial::print_dec(process::process_mut(slot).map(|p| p.pid).unwrap_or(0));
    serial::print(" path=");
    serial::print(&path);
    serial::print(" segments=");
    serial::print_dec(stats.segments as u64);
    serial::print(" image_pages=");
    serial::print_dec(stats.pages as u64);
    serial::print(" entry=");
    serial::print_hex(stats.entry);
    serial::println("");

    // 6. 进入新映像（不更新 return_to：返回点仍是父进程的 wait4）
    let p = process::process_mut(slot).expect("当前进程存在");
    let space_ptr = p.space.as_ref().expect("刚写入的地址空间") as *const UserSpace as u64;
    unsafe {
        trampoline::set_user_space(space_ptr);
        trampoline::enter(&p.frame as *const UserFrame)
    }
}

/// 系统调用 `wait4(pid)`：运行并回收直接子进程，返回其退出码。
///
/// 三种情况：
/// - 子进程已经是僵尸（之前 `wait4` 过、或异常退出过）→ 直接取退出码；
/// - 子进程还没跑过（刚 `fork`）→ 现在运行它，结束后取退出码；
/// - 找不到该 pid 的直接子进程 → `-ECHILD`（含 PID 复用/非子进程）。
pub unsafe fn sys_wait(args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let want = args[0];
    let Some(me) = process::current() else {
        return ECHILD;
    };
    let my_pid = me.pid;
    let Some(slot) = process::find_by_pid(want, Some(my_pid)) else {
        serial::print("user: wait pid=");
        serial::print_dec(want);
        serial::println(" -> ECHILD (not a direct child)");
        return ECHILD;
    };

    if process::process_mut(slot).map(|p| p.state) == Some(ProcState::Zombie) {
        return unsafe { reap(slot) };
    }
    // 「延迟调度」：子进程在父进程等待时才真正运行（见模块文档差异 1）
    unsafe { run_child(slot) }
}

/// 系统调用 `nanosleep`：以 PIT tick（100Hz → 10ms）为单位等待 `args[0]` 个 tick。
///
/// `syscall` 入口用 IA32_FMASK 清了 IF，因此等待期间显式 `sti; hlt` 让 PIT 推进。
pub unsafe fn sys_sleep(args: &[u64; 5], _space: *const UserSpace) -> i64 {
    let ticks = args[0].max(1);
    let start = unsafe { clock::tick_count() };
    let deadline = start + ticks;

    serial::print("user: sleep ticks=");
    serial::print_dec(ticks);
    serial::println("");

    let mut spins = 0u64;
    while unsafe { clock::tick_count() } < deadline {
        unsafe {
            core::arch::asm!("sti", "hlt", options(nomem, nostack, preserves_flags));
        }
        spins += 1;
        if spins > SLEEP_MAX_SPINS {
            // 见下方：返回前必须重新关中断（handler 的入口约定是 IF=0）。
            unsafe {
                core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
            }
            serial::println("user: sleep aborted (timer not advancing)");
            return abi::errno::EINTR;
        }
    }
    // 等待循环开过中断；恢复 handler 的「关中断」约定之后再打印/返回。
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }

    let elapsed = unsafe { clock::tick_count() } - start;
    serial::print("user: sleep done ticks=");
    serial::print_dec(elapsed);
    serial::println("");
    0
}

/// 构建 `argv` 用户栈并返回初始用户帧。
///
/// 布局（从 [`USER_STACK_TOP`] 向下）：
///
/// ```text
/// USER_STACK_TOP ─┬─ 保留 128 字节
///                 ├─ 参数字符串（每个以 NUL 结尾，内核不依赖终止符，NUL 只是给用户态看着方便）
///                 ├─ 对齐到 16
///                 └─ argv 指针数组（argc 个 + 末尾 NULL）← 用户 RSP
/// ```
///
/// `argc`/`argv` 通过入口寄存器 `rdi`/`rsi` 交给 `libc` 的 `_start`。
/// `argv` 原样传递（内核**不**把路径塞进 `argv[0]`）：调用方写什么就是什么，
/// 这也让 Shell 能把命令名本身当 `argv[0]`。
fn build_user_frame(space: &UserSpace, entry: u64, args: &[&[u8]]) -> Result<UserFrame, i64> {
    let total: u64 = args.iter().map(|a| a.len() as u64 + 1).sum();
    if total > ARG_MAX_BYTES {
        return Err(abi::errno::E2BIG);
    }
    let mut cursor = USER_STACK_TOP - 128;
    let mut pointers = [0u64; ARG_MAX as usize + 1];
    for (i, arg) in args.iter().enumerate() {
        cursor -= arg.len() as u64 + 1;
        if cursor < USER_STACK_TOP - ARG_MAX_BYTES {
            return Err(abi::errno::E2BIG);
        }
        space.stack_write(cursor, arg);
        space.stack_write(cursor + arg.len() as u64, &[0]);
        pointers[i] = cursor;
    }
    let argv_top = align_down(cursor, 16);
    let count = args.len() + 1;
    let argv_base = argv_top - (count as u64) * 8;
    if argv_base < USER_STACK_TOP - ARG_MAX_BYTES {
        return Err(abi::errno::E2BIG);
    }
    let mut bytes = [0u8; (ARG_MAX as usize + 1) * 8];
    for (i, ptr) in pointers[..count].iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&ptr.to_le_bytes());
    }
    space.stack_write(argv_base, &bytes[..count * 8]);
    Ok(UserFrame::new_user(
        entry,
        argv_base,
        args.len() as u64,
        argv_base,
    ))
}

/// 供 `syscall::user` 复用的入口构建：`argv` 由调用方给（顶层程序给 `[path]`）。
pub fn user_frame_for(
    space: &UserSpace,
    entry: u64,
    args: &[&[u8]],
) -> Result<UserFrame, i64> {
    build_user_frame(space, entry, args)
}

/// 结束当前进程的内核侧：恢复「谁在等我」并跳回去。**不返回**。
unsafe fn finish_current() -> ! {
    let p = process::current_mut().expect("finish_current 必须在进程上下文里");
    // 先把「谁在等我」的快照取出来，再恢复硬件/全局量，最后跳回去。
    let ctx = p.return_to;
    unsafe { resume_kernel_side(&ctx) };
    unsafe { trampoline::return_to_suspended(&p.return_to as *const RunCtx) }
}

/// 把 `ctx` 描述的内核侧状态写回 CR3 / `TSS.RSP0` / 系统调用栈顶 / 地址空间 / 当前进程。
///
/// 顺序有讲究：**先恢复 CR3**（当前 CR3 指向即将被释放的地址空间），
/// 之后才允许任何人 `Drop` 那个地址空间。
unsafe fn resume_kernel_side(ctx: &RunCtx) {
    process::set_current_index(if ctx.current == NO_PROCESS_U64 {
        NO_PROCESS
    } else {
        ctx.current as usize
    });
    unsafe {
        cpu::write_cr3(ctx.cr3);
        gdt::set_kernel_stack(ctx.rsp0);
        trampoline::set_syscall_stack_top(ctx.syscall_top);
        trampoline::set_user_space(ctx.space);
    }
}

/// 运行子进程直到它退出（或异常终止），然后回收它并返回退出码。
unsafe fn run_child(slot: usize) -> i64 {
    let caller_slot = process::current_index();
    // 1. 「谁在等我」= 当前内核现场；子进程退出时据此恢复
    let (frame_ptr, ctx_ptr, child_cr3, child_rsp0, child_top, child_space, child_pid, child_name) = {
        let child = process::process_mut(slot).expect("子进程存在");
        child.return_to = RunCtx {
            krsp: 0, // 由 run_user_frame 的汇编写入
            cr3: paging::current_root(),
            rsp0: gdt::kernel_stack(),
            syscall_top: trampoline::syscall_stack_top(),
            space: trampoline::user_space_ptr(),
            current: caller_slot as u64,
        };
        child.state = ProcState::Running;
        let Some(space) = child.space.as_ref() else {
            serial::println("user: wait FAIL (child has no address space)");
            return ENOSYS;
        };
        (
            &child.frame as *const UserFrame,
            &mut child.return_to as *mut RunCtx,
            space.root(),
            child.kstack_top(), // 同一条内核栈：TSS.RSP0 与 syscall 栈顶一致
            child.kstack_top(),
            space as *const UserSpace as u64,
            child.pid,
            child.name,
        )
    };

    serial::print("user: run child pid=");
    serial::print_dec(child_pid);
    serial::print(" name=");
    serial::println(child_name);

    // 2. 切到子进程的地址空间/内核栈并恢复它的用户帧
    process::set_current_index(slot);
    unsafe {
        cpu::write_cr3(child_cr3);
        gdt::set_kernel_stack(child_rsp0);
        trampoline::set_syscall_stack_top(child_top);
        trampoline::set_user_space(child_space);
        trampoline::run(frame_ptr, ctx_ptr);
    }

    // 3. 子进程已结束；内核侧（CR3/TSS/栈/当前进程）由它的 exit 路径恢复成第 1 步的快照
    unsafe { reap(slot) }
}

/// 取走退出码并释放进程（地址空间整棵归还）。
unsafe fn reap(slot: usize) -> i64 {
    let (pid, code) = match process::process_mut(slot) {
        Some(p) => (p.pid, p.exit_code),
        None => return -ECHILD,
    };
    process::release(slot);
    serial::print("user: reaped pid=");
    serial::print_dec(pid);
    serial::print(" exit=");
    serial::print_dec(code as u64);
    serial::println("");
    code
}

/// `execve` 的页对齐辅助已在 `memory::page` 提供；这里用它保证 `argv` 数组 16 字节对齐。
const _: () = assert!(PAGE_SIZE == 4096);
