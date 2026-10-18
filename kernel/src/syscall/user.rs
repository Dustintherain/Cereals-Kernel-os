//! 用户态运行驱动（v0.6 建立，v0.7 改为「从 VFS 读 ELF 装载」，
//! v0.9 第三步改为「每个程序是一个进程对象」，纯内核化后镜像来源改为**构建期清单**）
//!
//! 加载链（v0.7 起从未改变，只是镜像的来源变了）：
//!
//! ```text
//! userland/ 构建产物 → manifest.txt → kernel/build.rs 生成 PROGRAM_IMAGES/LIBRARY_IMAGES
//!        │  fs::init（main.rs 调用）       ← 空清单也合法（纯内核启动）
//!        ▼
//! RAMFS /init、/bin/sh、/lib/*              ← VFS 上的普通文件
//!        │  vfs::read_all(path)           ← 走挂载表路由 + 文件读
//!        ▼
//! elf::load → UserSpace（按段权限逐页映射）
//!        │  process::create → 进程表登记（独立地址空间 + 独立内核栈 + 用户帧）
//!        ▼
//! trampoline::run（切 CR3 + 恢复用户帧 → ring3）
//!        │
//!        ├─ 内建命令 / 文件 syscall / 用户堆 / 退出码
//!        └─ v0.9：fork / execve / wait4（`/bin/hello` 是被它跑起来的外部程序）
//! ```
//!
//! [`selftest`] 覆盖的验收路径（路径常量在本模块，**程序实现属于 `userland/`**）：
//!
//! - [`INIT_PATH`]：写串口、读 `/etc/motd`、写 `/tmp/hello.txt`（内核随后读回并打印）、
//!   用户堆自测、`exit(42)`；
//! - `probe`：**隔离探针** —— 位置无关的手写机器码（`global_asm!`），
//!   直接读内核映像地址（0x200_0000），必须被 ring3 #PF 拦下（不经 RAMFS，不依赖用户态）；
//! - [`SHELL_PATH`]：**交互式 Shell** —— 命令从键盘行缓冲（`read(0)`）来，
//!   内建命令 + **外部程序**（走 `fork`/`execve`/`wait4`）；
//! - [`HELLO_BIN_PATH`]：**外部程序** —— 由 Shell `fork` + `execve` 起来，
//!   打印自己的 PID 与 `argv`，证明「新进程 + 参数传递」真的发生了；
//! - [`LS_PATH`] / [`CAT_PATH`] / [`ECHO_PATH`] / [`PS_PATH`]：**基础用户程序** —— 各自
//!   只需要一条内核能力（`getdents` / 文件 syscall / `write` / `getprocs`），
//!   因此同时是「用户态能不能真的用上 VFS/进程表」的验收。
//! - [`TCPECHO_PATH`]：**socket 验收程序** —— 有网卡时才运行（`socket`/`connect`/
//!   `sendto`/`recvfrom` + `write`/`read` 两轮回声收发），无网卡时跳过。
//!
//! 这些常量是**内核与用户态之间的契约**：非空清单必须提供这些路径（否则 selftest
//! 报 `ENOENT`）；空清单则走「纯内核诊断」分支，内核不依赖用户态存在。

use core::arch::global_asm;
use core::ptr::addr_of;

use crate::arch::x86_64::{cpu, gdt, paging};
use crate::driver::serial;
use crate::fs::{self, vfs};
use crate::memory::frame;
use crate::process::pid;
use crate::process::process::{self, NO_PROCESS};
use crate::syscall::address_space::{UserSpace, USER_STACK_TOP};
use crate::syscall::trampoline::{self, RunCtx, UserFrame, NO_PROCESS as NO_PROCESS_U64};

// ---- 用户态镜像清单（内核与用户态的**唯一**接触面）----
//
// `kernel/build.rs` 按 `build/userland/images/manifest.txt`（由 `make userland-images`
// 在 `userland/` 里产出）把镜像字节复制进 `$OUT_DIR/userland/`，并生成下面两个静态切片：
//
//     PROGRAM_IMAGES: &[(&str, &[u8])]   写进 RAMFS 的可运行程序（`/init`、`/bin/*`）
//     LIBRARY_IMAGES: &[(&str, &[u8])]   被装载的对象（`/lib/ld.so`、`/lib/libgreet.so`）
//
// **空清单是合法的**：没有用户态时内核照样构建、启动，然后进入串口诊断模式
// （见 [`selftest`]）—— 这是 docs/pure-kernel.md 的验收标准 1/2/7。
// 内核源码里没有任何一条具体程序名或字节：全部来自清单。
include!(concat!(env!("OUT_DIR"), "/userland_images.rs"));

/// 用户程序在 RAMFS 上的路径。
pub const INIT_PATH: &str = "/init";
/// 交互式 Shell 在 RAMFS 上的路径（`fs::init` 写入）。
pub const SHELL_PATH: &str = "/bin/sh";
/// 外部程序在 RAMFS 上的路径（`fs::init` 写入）。
pub const HELLO_BIN_PATH: &str = "/bin/hello";
/// v0.9 第五步的基础用户程序路径（`ls` / `cat` / `echo` / `ps`）。
///
/// 与 `/bin/hello` 不同，它们**不由内核直接运行**，而是由 Shell 的
/// `fork` + `execve` 路径起来（`ls` / `ps` 没有内建命令；`cat` / `echo` 有内建
/// 版本，要走真实程序得用 `/bin/cat` / `/bin/echo`，见 `user/shell` 的帮助文本）。
pub const LS_PATH: &str = "/bin/ls";
/// `cat` 程序路径（内建 `cat` 会遮蔽裸命令名）。
pub const CAT_PATH: &str = "/bin/cat";
/// `echo` 程序路径（内建 `echo` 会遮蔽裸命令名）。
pub const ECHO_PATH: &str = "/bin/echo";
/// `ps` 程序路径。
pub const PS_PATH: &str = "/bin/ps";
/// 用户程序写文件的路径；内核在用户态结束后读回并打印。
pub const USER_FILE: &str = "/tmp/hello.txt";
/// v0.9 动态链接主程序路径（ET_DYN PIE，见 user/dynhello/）。
pub const DYNHELLO_PATH: &str = "/bin/dynhello";
/// socket/TCP 用户态验收程序路径（`user/tcpecho`：connect 回声服务，两轮收发）。
///
/// 只在有网卡时运行（`make test-network` 的 QEMU 带 virtio-net-pci + 回声服务）；
/// 无网卡时跳过（不是失败），因此纯内核/无网测试的串口只多一行 SKIP。
pub const TCPECHO_PATH: &str = "/bin/tcpecho";
/// 解释器路径（必须等于 `kernel/build.rs::DYN_INTERP` 与 `PT_INTERP`）。
pub const LDSO_PATH: &str = "/lib/ld.so";
/// 共享库路径（`DT_NEEDED=libgreet.so` → `/lib/libgreet.so`）。
pub const LIBGREET_PATH: &str = "/lib/libgreet.so";

global_asm!(
    ".align 16",
    // ---- probe：越权读取内核映像（0x200_0000）→ 应被 #PF 拦截 ----
    // 位置无关（只用立即数与寄存器间接寻址），因此整段字节可拷到任意用户代码页执行。
    ".global cer_user_probe_start",
    "cer_user_probe_start:",
    "mov rax, 0x2000000",                             // 内核链接地址
    "mov rbx, [rax]",                                 // ring3 读内核页 → #PF
    "mov rax, 60",                                    // 只有隔离失效时才会到这里
    "mov rdi, 7",
    "syscall",
    "ud2",
    ".global cer_user_probe_end",
    "cer_user_probe_end:",
);

extern "C" {
    static cer_user_probe_start: u8;
    static cer_user_probe_end: u8;
}

/// 最近一次用户态运行的退出码；`-1` 表示「没有走到 exit」（异常终止）。
///
/// 这是**验收用的镜像变量**：进程自己的退出码在进程表里（`Process::exit_code`），
/// 这里只是把「刚刚结束的那个进程」的结果留给 `selftest` 的断言读。
/// 非空清单必须提供的路径（内核 ↔ 用户态的**契约**，见模块文档）。
///
/// 缺任何一条都说明用户态构建侧与内核期望不一致 —— 在这里一次性报出来，
/// 而不是等到某条 `run_from_vfs` 给出一个难懂的 ENOENT。
const REQUIRED_PROGRAM_PATHS: [&str; 9] = [
    INIT_PATH, SHELL_PATH, HELLO_BIN_PATH, LS_PATH, CAT_PATH, ECHO_PATH, PS_PATH, DYNHELLO_PATH,
    TCPECHO_PATH,
];
const REQUIRED_LIBRARY_PATHS: [&str; 2] = [LDSO_PATH, LIBGREET_PATH];

static mut EXIT_CODE: i64 = -1;

/// 校验清单覆盖了契约里的每条路径（[`selftest`] 的前置条件）。
fn check_manifest_contract() -> Result<(), &'static str> {
    for want in REQUIRED_PROGRAM_PATHS {
        if !program_images().iter().any(|(path, _)| *path == want) {
            serial::print("user: manifest missing program ");
            serial::println(want);
            return Err("userland manifest is missing a required program path");
        }
    }
    for want in REQUIRED_LIBRARY_PATHS {
        if !library_images().iter().any(|(path, _)| *path == want) {
            serial::print("user: manifest missing library ");
            serial::println(want);
            return Err("userland manifest is missing a required library path");
        }
    }
    Ok(())
}

/// 要写进 RAMFS 的用户程序清单（由构建期清单生成，见 [`PROGRAM_IMAGES`]）。
pub fn program_images() -> &'static [(&'static str, &'static [u8])] {
    PROGRAM_IMAGES
}

/// 要写进 RAMFS `/lib` 的解释器与共享库清单（见 [`LIBRARY_IMAGES`]）。
pub fn library_images() -> &'static [(&'static str, &'static [u8])] {
    LIBRARY_IMAGES
}

/// 记录退出码（由 `syscall::process::sys_exit` / 异常终止路径调用）。
pub fn set_exit_code(code: i64) {
    unsafe {
        EXIT_CODE = code;
    }
}

/// 最近一次用户程序的退出码（未到达 exit 时为 `-1`）。
pub fn last_exit_code() -> i64 {
    unsafe { EXIT_CODE }
}

/// 取一个内嵌汇编程序的字节区间 `[start, end)`。
fn asm_program(start: *const u8, end: *const u8) -> &'static [u8] {
    let len = end as usize - start as usize;
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// 运行一个已装载的进程：登记进程对象、切 CR3/TSS/栈、恢复用户帧。
///
/// 返回时用户程序已经结束（`sys_exit` 或 ring3 异常），进程的地址空间已归还。
///
/// # Safety
/// `space` 必须已经装载好映像；`frame` 必须是同一个 `space` 里的合法入口。
unsafe fn run_process(
    name: &'static str,
    pid: u64,
    source: &'static str,
    cred: crate::security::Cred,
    space: UserSpace,
    frame: UserFrame,
) -> Result<(), &'static str> {
    let Some(slot) = process::free_slot() else {
        return Err("user: process table full");
    };
    let root = space.root();
    let p = process::create(slot, pid, name, 0, NO_PROCESS).ok_or("user: process slot busy")?;
    p.frame = frame;
    p.space = Some(space);
    // 顶层程序的初始凭证（CKM-1）：组件表登记过的（`/init`、`/bin/sh`）用组件声明的
    // uid/gid/能力，其余用最小权限。这一行证据说明「内核直接运行的程序是什么身份」，
    // 它与 `execve` 路径上的 `exec_component` 判定行是两条互补的证据。
    p.cred = cred;
    serial::print("ckm: program path=");
    serial::print(name);
    serial::print(" uid=");
    serial::print_dec(cred.uid as u64);
    serial::print(" caps=");
    serial::print_hex(cred.caps as u64);
    serial::println("");
    // 顶层程序一旦被运行就是 RUNNING（`create` 给的是 READY）——
    // `user/ps` 读的就是这个字段，不撒谎。
    p.state = crate::process::process::ProcState::Running;
    // 「谁在等我」= 内核启动上下文（顶层程序退出后回到 `run_process` 的调用者）
    p.return_to = RunCtx {
        krsp: 0, // 由 run_user_frame 的汇编写入
        cr3: paging::current_root(),
        rsp0: gdt::kernel_stack(),
        syscall_top: trampoline::syscall_stack_top(),
        space: 0,
        current: NO_PROCESS_U64,
    };

    let space_ptr = p.space.as_ref().expect("刚写入的地址空间") as *const UserSpace as u64;
    serial::print("user: enter name=");
    serial::print(name);
    serial::print(" pid=");
    serial::print_dec(pid);
    serial::print(" entry=");
    serial::print_hex(frame.rip);
    serial::print(" source=");
    serial::print(source);
    serial::print(" rsp0=");
    serial::print_hex(p.kstack_top());
    serial::println("");

    fs::file::reset();
    crate::network::socket::reset();
    unsafe {
        EXIT_CODE = -1;
        process::set_current_index(slot);
        cpu::write_cr3(root);
        gdt::set_kernel_stack(p.kstack_top());
        trampoline::set_syscall_stack_top(p.kstack_top());
        trampoline::set_user_space(space_ptr);
        trampoline::run(
            &p.frame as *const UserFrame,
            &mut p.return_to as *mut RunCtx,
        );
    }

    // 用户态结束：内核侧已由 exit/异常路径恢复成 `return_to` 的快照
    let code = process::process_mut(slot).map(|p| p.exit_code).unwrap_or(-1);
    set_exit_code(code);
    process::release(slot); // 归还地址空间（含堆/mmap 页）
    Ok(())
}

/// 从 VFS 读一个程序、装载、运行（v0.7 起的常规路径）。
fn run_from_vfs(name: &'static str, path: &'static str, pid: u64) -> Result<(), &'static str> {
    let elf = vfs::read_all(path).map_err(|_| "user: cannot read program from VFS")?;
    let mut space = UserSpace::new().map_err(|_| "user: address space allocation failed")?;
    let stats = space.load_elf(&elf).map_err(|_| "user: ELF load failed")?;

    serial::print("user: program=");
    serial::print(path);
    serial::print(" source=elf bytes=");
    serial::print_dec(stats.bytes);
    serial::print(" segments=");
    serial::print_dec(stats.segments as u64);
    serial::print(" image_pages=");
    serial::print_dec(stats.pages as u64);
    serial::print(" writable_pages=");
    serial::print_dec(stats.writable_pages as u64);
    serial::print(" entry=");
    serial::print_hex(stats.entry);
    serial::println("");

    // 顶层程序的 argv 就是它自己（`argv[0] = path`）—— 与外部程序走的是同一条路
    let frame = crate::syscall::process::user_frame_for(&space, stats.entry, &[path.as_bytes()])
        .map_err(|_| "user: argv stack build failed")?;
    // 初始凭证来自 CKM 组件表（`docs/ckm.md` 3.1 的固定主体划分）。
    let cred = crate::ckm::api::program_cred(path);
    unsafe { run_process(name, pid, "elf", cred, space, frame) }
}

/// 运行内嵌的手写机器码（隔离探针）。
fn run_raw(name: &'static str, code: &'static [u8], pid: u64) -> Result<(), &'static str> {
    let mut space = UserSpace::new().map_err(|_| "user: address space allocation failed")?;
    let entry = space.map_raw(code).map_err(|_| "user: raw program mapping failed")?;

    serial::print("user: program=");
    serial::print(name);
    serial::print(" source=asm bytes=");
    serial::print_dec(code.len() as u64);
    serial::print(" image_pages=");
    serial::print_dec(space.image_pages() as u64);
    serial::print(" entry=");
    serial::print_hex(entry);
    serial::println("");

    let frame = UserFrame::new_user(entry, USER_STACK_TOP, 0, 0);
    // 手写机器码（隔离探针）没有组件表项：按最小权限（普通用户）启动。
    unsafe { run_process(name, pid, "asm", crate::security::Cred::plain(), space, frame) }
}

/// 读回用户程序写的文件（`/tmp/hello.txt`），证明用户态 `write` 真的落到了 RAMFS。
fn verify_user_file() -> Result<(), &'static str> {
    let data = vfs::read_all(USER_FILE)
        .map_err(|_| "user: /tmp/hello.txt missing (user write did not reach RAMFS)")?;
    if data.is_empty() {
        return Err("user: /tmp/hello.txt is empty");
    }
    serial::print("fs: readback ");
    serial::print(USER_FILE);
    serial::print(" len=");
    serial::print_dec(data.len() as u64);
    serial::println("");
    serial::print("fs: user data: ");
    serial::put_bytes(&data, data.len());
    serial::println("");
    Ok(())
}

/// 验收自测：ELF 从 VFS 装载、ring3 syscall 往返（含用户堆）、用户写入落盘、地址空间隔离、
/// 交互式 Shell 与**外部程序**（v0.9 第三步的 fork/execve/wait4）。
///
/// 最后一项是**页帧泄漏检查**：四次运行的 `UserSpace` 都被释放，`fork` 出来的子进程
/// 在 `wait4` 里回收 —— 帧分配器的 `free_frames` 必须回到运行前的值，否则说明
/// 「某个进程的地址空间没有被归还」。
pub fn selftest() -> Result<(), &'static str> {
    // 纯内核路径：本次构建没有内嵌用户态镜像（清单为空）——
    // 内核**从不**依赖用户态才能启动。这里给出可断言的诊断行后正常返回，
    // 由 `kernel_main` 进入 `hlt` 空闲循环（`make test-kernel-only` 断言这条轨迹）。
    if program_images().is_empty() {
        serial::println("user: no userland images embedded; entering diagnostic idle loop");
        serial::println("user: kernel-only boot PASS (build images with `make userland-images`)");
        return Ok(());
    }
    // 有用户态：清单必须与内核期望的路径一致（缺一条就明确报错，见契约常量）。
    check_manifest_contract()?;
    crate::syscall::init();
    // 0 号保留给 `kernel_main`（启动上下文），用户程序从 1 开始编号 ——
    // `fork` 继续往下取号，因此 PID 不会与顶层程序撞车。
    let _kernel_main = pid::alloc();
    serial::println("user: selftest begin");
    let free_before = frame::stats().free_frames;

    // 1) 正常路径：/init（ELF 来自 RAMFS）完成一轮完整 syscall 往返并以 exit(42) 结束
    run_from_vfs("init", INIT_PATH, 1)?;
    let code = last_exit_code();
    if code != 42 {
        serial::print("user: init FAIL exit=");
        serial::print_dec(code as u64);
        serial::println("");
        return Err("init did not exit(42)");
    }
    serial::println("user: init PASS (ELF from RAMFS; write/open/read/close/exit code=42)");

    // 2) 用户态写入落到 RAMFS：内核读回同一文件并打印内容
    verify_user_file()?;

    // 3) 隔离路径：用户程序读内核页，必须收到 ring3 #PF 并被终止
    let probe = asm_program(addr_of!(cer_user_probe_start), addr_of!(cer_user_probe_end));
    run_raw("probe", probe, 2)?;
    let code = last_exit_code();
    if code != -1 {
        serial::print("user: probe FAIL exit=");
        serial::print_dec(code as u64);
        serial::println("");
        return Err("probe reached sys_exit: ring3 isolation not enforced");
    }
    serial::println("user: probe PASS (kernel page not reachable from ring3)");

    // 4) 交互式 Shell（v0.9）：`/bin/sh` 从 VFS 装载，命令从键盘行缓冲（stdin）来；
    //    它的外部命令路径会真正跑起 `/bin/hello`（见其源码的 `cmd_external`）。
    run_from_vfs("sh", SHELL_PATH, 3)?;
    let code = last_exit_code();
    if code != 0 {
        serial::print("user: shell FAIL exit=");
        serial::print_dec(code as u64);
        serial::println("");
        return Err("shell did not exit(0)");
    }
    serial::println("user: shell PASS (keyboard line buffer -> stdin -> builtins + external program)");

    // 5) v0.9 动态链接：`/bin/dynhello` 是 ET_DYN PIE —— `PT_INTERP=/lib/ld.so`、
    //    `DT_NEEDED=libgreet.so`。内核解析这两个路径、把解释器与共享库装进共享库窗口、
    //    应用 `R_X86_64_RELATIVE`（数据指针）与 `R_X86_64_JUMP_SLOT`（`greet` 的 GOT），
    //    然后进入程序。退出码 = 共享库 `greet()` 的返回值（42）。
    run_from_vfs("dynhello", DYNHELLO_PATH, 4)?;
    let code = last_exit_code();
    if code != 42 {
        serial::print("user: dynhello FAIL exit=");
        serial::print_dec(code as u64);
        serial::println("");
        return Err("dynhello did not exit(42)");
    }
    serial::println("user: dynamic link PASS (PT_INTERP + libgreet.so + RELATIVE/JUMP_SLOT)");

    // 5.5) socket/TCP 用户态验收：`/bin/tcpecho` 走 `socket/connect/sendto/recvfrom`
    //    与 `write/read` 两套形状与回声服务做两轮收发。无网卡时跳过（回声服务只在
    //    `--nic` 启动里存在，见 `network::TCP_ECHO_PORT`），不把「没有网卡」当失败。
    if crate::driver::net::count() == 0 {
        serial::println("user: tcpecho SKIP (no network device)");
    } else {
        run_from_vfs("tcpecho", TCPECHO_PATH, 5)?;
        let code = last_exit_code();
        if code != 0 {
            serial::print("user: tcpecho FAIL exit=");
            serial::print_dec(code as u64);
            serial::println("");
            return Err("tcpecho did not exit(0)");
        }
        serial::println("user: tcpecho PASS (socket/connect/send/recv/close over TCP)");
    }

    // 6) 每个进程的地址空间（含用户堆、共享库与 fork 出的子进程）都必须整棵归还
    let free_after = frame::stats().free_frames;
    serial::print("user: frames free before=");
    serial::print_dec(free_before as u64);
    serial::print(" after=");
    serial::print_dec(free_after as u64);
    serial::println("");
    if free_before != free_after {
        serial::println("user: process frame reclamation FAIL (an address space leaked frames)");
        return Err("user address space leaked frames");
    }
    serial::println("user: heap frame reclamation PASS");

    serial::println("user: selftest PASS");
    Ok(())
}
