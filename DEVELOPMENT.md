# Cerlesse OS 主开发文档（DEVELOPMENT）

- 阶段：第八阶段（v0.9 进行中）
- 最后更新：2026-10-11
- 状态：v0.1–v0.5 全部完成（v0.4：中断/Timer/键盘/上下文切换 + H-04/H-05 Go 工具，`make go-test` / `make test-orch` 通过；v0.5：线程模型 + Round Robin 时间片调度 + IPC pipe 骨架，`make test-scheduler` / `make test-ipc` 通过）；
  v0.6 核心已落地并通过 `make test-user` 验收（syscall/sysret 入口 + ring3 + 用户地址空间隔离），
  并且**用户态脚手架已可用**（`libc/` 最小运行时 + `user/hello` 真实 Rust 用户程序 + `kernel/build.rs` 构建期摊平内嵌，`make user` 可单独验证）；
  **v0.7 已落地并通过 `make test-fs`（20 项）+ `make test-user`（32 项）验收**：
  VFS（trait + 挂载表最长前缀路由）、RAMFS、fd 表、`read`/`write`/`open`/`close`，
  以及 **ELF Loader**（用户程序不再是摊平映像，而是 RAMFS 上的 `/init` 经 `elf.rs` 按段权限装载，
  `RW` 段对 ring3 可写）；`make user` 追加 `tools/check_user_elf.py` 独立校验段布局；
  **v0.8 已完成并通过 `make test-driver`（22 项，v1.0 起为 23 项）+ `make test-fat`（21 项）验收**：驱动注册框架（`Driver` trait + 注册表 + `probe_all`）、
  PCI 枚举（配置机制 #1 + BAR 探测 + 窗口大小）、块设备抽象（按 512 字节扇区读写）、
  **legacy virtio-blk 0.9.5**（virtqueue + 同步轮询 + bounce buffer）、`frame::allocate_contiguous`（DMA 前置条件）、
  **FAT12/16/32 只读后端挂到 VFS**（`/disk` = 启动盘、`/mnt/fat` = 测试盘，syscall 层零改动）与 H-03 `mkfs`
  （`tools/mkfs/mkfs.py`，mtools 独立复核）；启动盘已改为 virtio-blk（`disable-modern=on`）；
  **v0.9 已开局**（2026-10-11，运行时前置条件第一步：**用户堆**）—— `brk`/`mmap`/`munmap`
  （W^X、全零页、`invlpg`、Linux 式 `brk` 语义）+ `libc` 的 `#[global_allocator]`（`Vec` 可用）；
  同日**第二步：交互式 Shell 与 `stdin`** —— 键盘行缓冲（行编辑 + 终端式回显）→
  `read(0)` 整行交付 → `user/shell`（提示符 `root@cerlesse C:>`、内建 `help`/`echo`/`cat`/`pid`/`exit`），
  内嵌为 RAMFS 上的 `/bin/sh`，新增 `make test-shell`（11 项）；
  同日**第三步：进程对象 + `fork`/`execve`/`wait4`** —— 每进程地址空间 + 每进程 64 KiB 内核栈
  （栈底哨兵）+ 停在 syscall 里的完整用户帧；Shell 的外部命令真的跑起来了
  （`/bin/hello`，新增 `make test-proc`，8 项 + 4 条正则）；
  `make test-user` 由 32 项增至 **61 项**；`argc/argv` 一并可用；
  同日**第四步：简化网络栈**（virtio-net + Ethernet → ARP → IPv4 → UDP，ARP 解析网关 + DHCP 往返，
  `make test-network` 13 项；v1.0 又补齐 TCP/socket：内核 TCP 客户端最小集 + `socket`/`bind`/`connect`/`sendto`/`recvfrom` +
  `/bin/tcpecho` 回显往返，`make test-network` 13 → **23 项**，见 syscall.md 3.13）；同日**第五步：基础用户程序（`ls`/`cat`/`echo`/`ps`）** —— 新增
  `getdents`（目录枚举）与 `getprocs`（进程表快照，Cerlesse 扩展号段 256），`/bin` 下四个程序全部可跑
  （`make test-programs`，17 项 + 5 条正则）；同日**第六步：内核侧最小动态链接** ——
  `ET_DYN` 分流 + `PT_INTERP`/`DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`（`make test-dyn` 10 项 +
  `make dyn-hostcheck` 宿主机 glibc 独立复核），v0.9 六步全部落地
- 关联文档：[docs/architecture.md](docs/architecture.md) · [docs/roadmap.md](docs/roadmap.md) · [docs/project-structure.md](docs/project-structure.md) · [docs/drivers.md](docs/drivers.md) · [docs/syscall.md](docs/syscall.md) · [docs/filesystem.md](docs/filesystem.md) · [docs/process.md](docs/process.md)

---

## 1. 功能需求

### 1.1 核心需求（Rust 内核）

| 编号 | 需求 | 版本 | 状态 |
| ---- | ---- | ---- | ---- |
| R-01 | UEFI 引导进入 Rust 内核入口，串口输出 | v0.1 | ✅ |
| R-02 | GDT/IDT 与异常处理（#PF/#GP/#DF 等） | v0.2 | ✅ |
| R-03 | 物理帧分配 + 虚拟内存映射 + 内核堆分配器 | v0.3 | ✅ |
| R-04 | 中断控制器（PIC/APIC）、Timer、键盘输入 | v0.4 | ✅（APIC 未做） |
| R-05 | 进程/线程模型、上下文切换、Round Robin 调度 | v0.5 | ✅ |
| R-06 | 系统调用入口、首批 syscall、用户态隔离 | v0.6 | ✅ |
| R-07 | VFS + RAMFS + ELF Loader | v0.7 | ✅ |
| R-08 | 驱动框架、PCI 枚举、磁盘驱动 | v0.8 | ✅ |
| R-09 | Shell、基础用户程序、简化网络栈、最小动态链接 | v0.9 | ✅（六步全部落地，见第 5 节 v0.9 清单） |
| R-10 | 权限/凭证、可完整启动的类 Linux 小型 OS | v1.0 | ⬜ |

### 1.2 主机侧需求（Python / Go）

| 编号 | 需求 | 语言 | 状态 |
| ---- | ---- | ---- | ---- |
| H-01 | 镜像构建脚本（组装 boot + kernel + FS 镜像） | Python | ✅ v0.1 |
| H-02 | QEMU 自动化：启动、超时、串口日志断言 | Python | ✅ v0.1 |
| H-03 | `mkfs` 工具（FAT12/16/32 镜像生成） | Python | ✅ v0.8（`tools/mkfs/mkfs.py`，mtools 复核） |
| H-04 | 测试编排服务（并行多场景跑 QEMU 测试） | Go | ✅ v0.4（`tools-go/cmd/testorch`） |
| H-05 | 串口日志实时收集与分析 | Go | ✅ v0.4（`tools-go/cmd/serialmon`） |
| H-06 | 包管理器设计与主机侧原型（高效/安全/轻量、Windows 式灵活性参考） | Python（纯标准库） | 🟡 提案 v1 ✅ + **P0a/P0b 已落地**（`.ckc` 容器 `userland/pkgmgr/ckc.py` + **事务引擎 `userland/pkgmgr/cpm.py`**；`make test-pkg` = `test-ckc` 37 + `test-cpm` **63** 项）；catalog 签名（开放问题 13.1）与内核侧接入（P1）待做 |
| H-07 | **CKM（Cerlesse Kernel Manager）：内核级权限控制与扩展管理**（强制权限模型、受控系统接口、扩展状态与覆盖层、事务/回滚、安全模式与恢复环境） | Rust 内核 + 用户态（`ckmd`/`ckm`）+ Python（主机侧工具） | 🟡 **提案 v1 + CKM-1/CKM-2/CKM-3 已落地**：CKM-1（`kernel/src/ckm/` + 判定点 1/3/5/6 + syscall 257–259）、CKM-2（双槽 A/B 状态区 + 覆盖层机制）、**CKM-3（事务引擎，主机侧 `userland/pkgmgr/cpm.py`）**；CKM-4…CKM-7 规划中 —— 见 [docs/ckm.md](docs/ckm.md) |
| H-08 | **CSS（Cerlesse Storage System）：存储管理**（磁盘/分区识别、卷与稳定标识、盘符与卷标、多挂载点、文件系统探测与格式化、健康/只读状态，与 CKM 的权限/持久化对接） | Rust 内核（`kernel/src/storage/` + `fs/*`）+ 用户态（`ckm storage`，CSS-5）+ Python（`tools/mkfs` 生成分区表夹具） | 🟡 **CSS-1/CSS-2 已落地，CSS-3 大部分落地**：GPT/MBR 只读扫描 + 卷记录 + `storage_volumes`(261)/`storage_rescan`(265) + `mkfs` 夹具；VFS 收口（`caps`/`volume_info`、挂载表带 `vol_id`/`options`、多挂载点、卸载、挂载错误改 errno）；盘符表 `storage/letters.rs` + `storage_mount`(262)/`storage_unmount`(263)/`storage_assign_letter`(264) + `libc` 的 `C:\` 重写；仍缺 `/storage/**` 状态树、卷标写/改名、可写文件系统与 `ckm storage` 界面 —— 见 [docs/storage.md](docs/storage.md) |

### 1.3 非目标（见 architecture.md 第 5 节）

SMP、USB、Wi-Fi、GPU（**3D 加速**）、完整 POSIX、完整动态链接 ELF、完整 TCP/IP、多架构。

> v1.0 之后另设**桌面环境兼容路线**（内核侧地基与接口对齐：设备模型/`ioctl`/`poll`/evdev/
> fbdev→KMS/ALSA 子集/ACPI 子集等），规划与里程碑见 [docs/desktop.md](docs/desktop.md)；
> 它不改变本文档 v1.0 的范围。
>
> v1.0 之后还设 **CKM（内核级权限控制与扩展管理）**：内核提供强制权限与组件状态机制，
> 事务/包操作/修复由特权用户态服务完成，**不把包管理器放进内核**；第一版不做动态内核模块装载与 GUI。
> 需求行 H-07，规格见 [docs/ckm.md](docs/ckm.md)（执行上排在桌面路线之前，与它互不阻塞）。

---

## 2. 模块依赖关系

```text
boot ──→ arch(x86_64) ──→ interrupt ──→ time
              │               │
              ↓               ↓
           memory ──────→ process ──→ scheduler
              │               │
              ↓               ↓
            driver ─────→ syscall ──→ fs ──→ user(shell/apps)
                              │
                              ↓
                          network（最后）
```

依赖规则：

- 上游模块未验收，下游模块不得开工。
- `network` 依赖 `driver` + `syscall`，排在最后。
- `security` 依赖 `process` + `fs`，v1.0 才实现。

---

## 3. Cargo Workspace 规划

根 `Cargo.toml` 的 `members` 是 **`shared`/`boot`/`abi`/`kernel`**（用户态已拆到 `userland/`，见下），
因此 `cargo build`（不带 `--target`）只构建内核侧；用户程序只能在 `userland/` workspace 里
为目标 `x86_64-unknown-none` 构建（链接在 2^39）。

约定：

- `shared`：boot 与 kernel 共享的 `BootInfo` 等纯数据结构（`no_std`）。
- `abi`：**系统调用 ABI 的单一事实源**（阶段三，`no_std`、零依赖、`forbid(unsafe_code)`）：调用号/`SYS_MAX`/号段规则、
  errno、`O_*`/`PROT_*`/`MAP_*`/fd 编号、参数上限、`dirent`/`procinfo`/`volume`/`ckm` 记录布局与编译期断言；内核与 libc 都引用它。
  宿主自测：`make abi-check`。
- `kernel`：`#![no_std]`，目标 `x86_64-unknown-none`，链接脚本 `kernel/link.ld`（装载 32MiB）。
- `boot`：UEFI 引导 crate，目标 `x86_64-unknown-uefi`；与内核边界调用必须 `extern "sysv64"`（ADR-006）。
- `tools/`（Python）与 `tools-go/`（Go）**不进 Cargo workspace**，各自独立管理（仅标准库）。
- 交叉编译在 Linux 与 Windows 主机上均通过 `rustup target add` 相同目标完成（见 docs/boot.md）。

用户态（`libc` + `user/*`）自纯内核化起是**另一个 workspace**（`userland/Cargo.toml`，
根 `Cargo.toml` 用 `exclude = ["userland"]` 排除），其中定义用户程序专用 profile
（large 代码模型与 LTO 不兼容）：

```toml
[profile.user-release]
inherits = "dev"
opt-level = 2
lto = false
debug = false
debug-assertions = false
overflow-checks = false
strip = true
panic = "abort"
```

约定：

- `shared`：boot 与 kernel 共享的 `BootInfo` 等纯数据结构（no_std）。
- `kernel`：`#![no_std]`，目标 `x86_64-unknown-none`，链接脚本 `kernel/link.ld`（32MiB）。
- `boot`：UEFI 引导 crate，目标 `x86_64-unknown-uefi`；与内核边界调用必须 `extern "sysv64"`（ADR-006）。
- `userland/libc`：最小用户态运行时（syscall 封装 + `_start` + `print!/println!` + 堆 + `args`），
  属于用户态 workspace（根 `Cargo.toml` 的 `exclude`）。
- `userland/user/*`：`no_std` 用户态程序，产出静态 ELF；`hello`（→ RAMFS `/init`）、
  `shell`（→ `/bin/sh`）、`hello-bin`（→ `/bin/hello`，外部程序）、`ls`/`cat`/`echo`/`ps`（→ `/bin/*`）
  均已落地；`user/init` 仍是占位（内核实际跑的 `/init` 是 `user/hello` 的产物）。
- `userland/user/dynhello/`：**不是 Rust crate**，而是动态链接的汇编夹具（`dynhello.s`/`libgreet.s`/`ldso.s`）；
  由 `userland/Makefile` 用 binutils（`as`+`ld`）现编到 `build/userland/images/`，再由清单交给内核内嵌
  （裸目标 `x86_64-unknown-none` 不支持 `cdylib`，所以这一组工件不走 cargo）。
- `tools/`（Python）与 `tools-go/`（Go）**不进 Cargo workspace**，各自独立管理。
- 交叉编译在 Linux 与 Windows 主机上均通过 `rustup target add` 相同目标完成（见 docs/boot.md）。

> **纯内核化（已执行，见 5.11 与 [docs/pure-kernel.md](docs/pure-kernel.md)）**：本仓库的 workspace
> 已收敛为 `members = ["shared", "boot", "abi", "kernel"]`（`abi` = 系统调用 ABI 单一事实源，阶段三 ✅）；`libc` 与 `user/*`
> 连同 `user-release` profile、`.cargo/config.toml`（`code-model=large`）已迁入 `userland/` 独立
> workspace，内核通过 `build/userland/images/manifest.txt` 按清单内嵌。
> 因此 `cargo build -p kernel` **在没有 `userland/` 时也能成功**，启动后进入串口诊断模式。

---

## 4. 目录结构

见 [docs/project-structure.md](docs/project-structure.md)（完整目录树与模块职责表）。

---

## 5. 版本任务清单（v0.1 – v1.0）

### 5.1 里程碑总览

| 版本 | 目标 | 状态 |
| ---- | ---- | ---- |
| v0.1 | Bootloader + Rust Kernel | ✅ 2026-10-06 |
| v0.2 | GDT + IDT + Exception | ✅ 2026-10-06 |
| v0.3 | Physical/Virtual Memory + Heap | ✅ 2026-10-07 |
| v0.4 | Interrupt + Timer + Keyboard + H-04/H-05 | ✅ 2026-10-08 |
| v0.5 | Process + Thread + Scheduler + IPC（pipe） | ✅ 2026-10-08 |
| v0.6 | Syscall + User Space + 用户态脚手架 | ✅ 2026-10-10 |
| v0.7 | VFS + RAMFS + ELF Loader | ✅ 2026-10-11 |
| v0.8 | Driver + PCI + Disk（FAT 只读 + mkfs） | ✅ 2026-10-11 |
| v0.9 | Shell + User Programs + Network | ✅ 2026-10-11（六步：用户堆 / 交互式 Shell / 进程对象 / 简化网络栈 / 基础用户程序 / 最小动态链接） |
| v1.0 | 完整可启动类 Linux 小型 OS | ⬜ |

### 5.2 v0.1 – v0.5 完成记录（要点）

- **v0.1**：Cargo Workspace + 双目标（uefi/none）；UEFI 引导解析内存映射、装载内核 ELF、
  传 `BootInfo`、`ExitBootServices`；`kernel_main` + 串口；H-01 镜像构建 + H-02 QEMU 测试。
  验收：串口打印 `Kernel started!`。
- **v0.2**：自建 GDT + TSS（IST1 双错栈 / `RSP0`）+ ltr；IDT 32 个异常向量 + 汇编 stub
  （区分有/无错误码）+ 统一保存/恢复；异常诊断输出（向量名/错误码/RIP/CS/RFLAGS/RSP/SS/通用寄存器，
  #PF 带 CR2）。验收：`make test` 回归 + `make test-exception`（#DE）。
- **v0.3**：位图帧分配器（1 bit/帧，128KiB 覆盖 4GiB）、4 级页表（translate/map_4k/unmap_4k/TLB/`AddressSpace`）、
  内核堆（映像末尾预留 1MiB + 首次适配空闲链表 + 合并）。
  验收：`make test-memory`（含 `Box`/`Vec`/`String`、1024 帧压力、页表自测）+ `make test-pagefault`（#PF + CR2）。
- **v0.4**：PIC（8259A，IRQ→向量 32..47）+ IRQ 分发注册表；PIT 通道 0 频率 **100Hz（10ms/tick）** +
  心跳；PS/2 键盘（8042 初始化 + IRQ1 + set 1 解码）；上下文切换原语（callee-saved + 栈切换 + RFLAGS）；
  H-04/H-05 Go 工具（`go-test` / `test-orch`）。
  验收：`make test-keyboard`（QEMU monitor 注入真实按键 + PIT 心跳）。
- **v0.5**：线程控制块与状态机、PID 分配、定长环形就绪队列、Round Robin（200ms 时间片）、
  PIT/IRQ0 抢占式调度（pid 1/2/3 严格轮转并绕回 pid 0）；IPC 单向 pipe 骨架（64B 环形缓冲，非阻塞语义）。
  验收：`make test-scheduler` / `make test-ipc`。

### 5.3 v0.6 — Syscall + User Space ✅（2026-10-10 核心，2026-10-11 脚手架收尾）

- [x] 入口选型：`syscall`/`sysret` 快速路径（`EFER.SCE` + `IA32_STAR/LSTAR/FMASK`，GDT 按 STAR 布局）；入口切专用 syscall 栈
- [x] 寄存器约定：号 `rax`；参数 `rdi/rsi/rdx/r10/r8`；返回 `rax`；负数 = `-errno`
- [x] `syscall/mod.rs` 分发层 + `syscall_table.rs` 全号段表（未实现号落 `-ENOSYS` 桩）
- [x] 首批 syscall：`write`（串口，先校验用户缓冲指针）/ `getpid` / `nanosleep` / `exit`
- [x] 用户地址空间隔离：独立 PML4（用户映射固定下标 1 = `0x8000000000`，每级 `USER`），
      代码页 RO+X、栈页 RW+NX，`Drop` 整棵释放
- [x] ring3 异常终止语义：`exception_dispatch` 按 `cs & 3` 识别，用户态异常不 panic
- [x] 用户态执行期收敛：`scheduler::suspend()` + `TSS.RSP0` 切专用陷阱栈
- [x] 用户态脚手架：`libc/`（syscall 封装 + `_start` + `print!/println!` + `#[panic_handler]`）
      + `user/hello` 真实 Rust 用户程序；`kernel/build.rs` 构建期构建/校验/内嵌其 ELF；
      `UserSpace::load_program` 支持多代码页（1..=8）
- [x] 验收：`make test-user`（19 项 → v0.7 增至 32 → v0.9 增至 61 → v1.0 第一步增至 91）；`make user` 单独验证脚手架

### 5.4 v0.7 — VFS + RAMFS + ELF ✅（2026-10-11）

- [x] VFS（`fs/vfs.rs`）：`FileSystem` trait + 路径工具（`normalize`/`components`/`within`）+ 系统级入口
- [x] 挂载管理（`fs/mount.rs`）：**最长前缀路由**（`/mnt/*` 落到 `/mnt` 后端），日志 `fs: route ... fs=...`
- [x] Inode/元数据（`fs/inode.rs`）：`NodeKind`、`Metadata`、`DirEntry`
- [x] RAMFS（`fs/ramfs.rs`）：`Node { name, kind, data, children }`，单文件 ≤ 256 KiB；
      `fs::init` 建初始树 `/init` + `/etc/motd` + `/tmp`（v0.9 加 `/bin`）
- [x] fd 表与文件 syscall（`fs/file.rs` + `syscall/file.rs`）：0/1/2 保留、动态 fd ≥ 3（最小空闲槽位）、
      `open`（指针 + 长度、`O_CREAT`）/`read`/`write`/`close`，用户缓冲按页校验（`-EFAULT`）
- [x] **ELF Loader**（`syscall/elf.rs`）：ELF64/`ET_EXEC`/`EM_X86_64` 解析 → 按段权限逐页映射
      （`W` → 可写、非 `X` → `NX`）→ 拷贝 `p_filesz`、`p_memsz` 余下置 0 → 入口取 `e_entry`；
      拒绝畸形 ELF 时打印 `user: ELF rejected: <reason>`
- [x] 用户程序改为从文件加载：`kernel/build.rs` 内嵌 ELF → RAMFS `/init` → `vfs::read_all` → `elf::load`
      （v0.6 的「构建期摊平」已移除）；`user.ld` 四个段分别 `ALIGN(0x1000)`，`W^X` 不退化
- [x] 隔离探针保留为 `map_raw` 路径（探针不依赖被测的 ELF Loader）
- [x] 验收：`make test-fs` 19 项 PASS（挂载/初始文件树/stat/内容读回/挂载点路由/`ls`/负例 errno）；
      `make test-user` 32 项 PASS（`source=elf` + 段数/页数/可写页 + `open/read/close/write` 全链路 +
      用户写入被内核读回 + `static counter=1`（`RW` 段可写）+ `EFAULT` + 退出码/隔离回归）
- [x] `make user` 追加 `tools/check_user_elf.py`：不启 QEMU 即可校验基址/段页对齐/`W^X`/映像窗口/无重定位
- [x] Shell 里 `cat` 已可用（v0.9 第二步，内建命令走同一套文件 syscall）；`ls`/`cd` 等
      **外部程序**在 v0.9 第三步打通（`fork`/`execve`/`wait4`）—— 见下方 v0.9 小节
- [x] 磁盘文件系统后端（FAT）+ Python mkfs（H-03）→ **v0.8 已落地**（见 v0.8 小节的最后两条）
- [x] 用户堆 / `brk` / `mmap`（v0.9 第一步）、`argc/argv` 与 `fork`/`execve`/`wait4`（v0.9 第三步）；
      **剩余**：动态链接、`mprotect` —— 两者均已在后续版本补齐（v0.9 第六步 / v1.0 第一步，
      见 docs/syscall.md 3.9 / 3.10）

### v0.8 — Driver + PCI + Disk（已实现并通过验收，2026-10-11）
- [x] Driver Manager 与驱动注册框架（`kernel/src/driver/driver.rs`）：`Driver` trait（`name`/`matches`/`attach`）、
      固定容量注册表、`probe_all()` 按注册顺序匹配 + 挂接、`ProbeStats { devices, matched, attached, failed }`
- [x] PCI 枚举 + Vendor/Device 匹配（`kernel/src/driver/pci.rs`）：配置机制 #1（`0xCF8`/`0xCFC`）、
      全总线扫描（多功能位修剪）、BAR 探测（写全 1 读掩码）与**窗口大小**计算、类别三元组解码、`print_bdf`
- [x] 块设备抽象 + 注册表（`kernel/src/driver/block.rs`）：`BlockDevice`（`read`/`write`/`sectors`/`sector_size`）、
      512 字节扇区粒度、长度非整扇区直接报错（不静默取整）
- [x] 块设备驱动：**legacy virtio-blk（0.9.5）**（`kernel/src/driver/virtio_blk.rs`）：
      virtqueue（描述符表 + avail + 4 KiB 对齐 used）→ notify → 轮询 used ring（PIT tick 超时），
      描述符链「请求头 → 数据 → 状态字节」，页对齐 bounce buffer 分块搬运（不要求调用方缓冲对齐/连续）；
      status 机 `ACKNOWLEDGE → DRIVER → FEATURES_OK → DRIVER_OK`，modern（0x1042）明确拒绝并给出原因
- [x] 基础设施：`arch/x86_64/port.rs`（`inb/inw/inl`+`outb/outw/outl`）、`frame::allocate_contiguous` / `free_contiguous`
      （virtqueue 必须物理连续，legacy 的队列地址只装一个 PFN）
- [x] 硬件路线改变：QEMU 启动盘由 IDE/AHCI 改为 **virtio-blk（legacy）** —— 「镜像能启动」与
      「内核驱动能读写这块盘」变成同一台设备上的同一条路径（Makefile `run` / `tests/test_boot.py` / `tests/test_keyboard.py`）
- [x] 验收：`make test-driver` 22 项 PASS（`vendor=0x1af4 device_id=0x1001`、`bar0=io:`、
      `queue0 size=256 ... 3 pages contiguous`、`capacity=131072 sectors (64 MiB)`、
      `boot sector sig=0x55aa fstype=FAT16`、分块读一致性 512/4096/4608、写回读 + 还原、三类负例）；
      编排场景由 8 个增至 **9 个**（`driver`），`make test-orch` 9 passed / 0 failed（127.2s）
- [x] 磁盘 FS 后端（**FAT12/16/32，只读**）挂载到 VFS（`kernel/src/fs/fat.rs` + `fs::mount_disks`）：
      类型由 BPB 的**簇数**判定（不靠扩展名）、12/16 的固定根目录区与 32 的子目录簇链、
      8.3 短名 + NT 大小写标志 + **LFN 长文件名**（含校验和）、任意偏移读、空文件（首簇 0）正确返回 0；
      写操作一律 `EROFS`；簇链断/越界 → `EUCLEAN`（不静默返回半截数据）。
      `FileSystem` trait 随之收窄为 `&mut self` + `read_all -> Vec<u8>`（磁盘文件内容不在内存里，
      原来那种「借用后端缓冲」的形状不再成立）；**syscall 层一行未改** —— 这正是 VFS 收口的目的
- [x] Python `mkfs` 工具（H-03，`tools/mkfs/mkfs.py`）：自己实现 FAT12/16/32 镜像生成（引导扇区/FSInfo/
      FAT 副本/LFN + 8.3 目录项/簇链），写完回读校验；正确性由 **mtools 独立复核**（`mdir`/`mtype`
      读同一张盘，见 `tests/fatroot/README.md`）—— 两套独立实现互相印证，而不是自己验证自己
- [x] 验收：`make test-fat` **21 项** PASS（双盘启动）：`/disk` 是固件写的 FAT16 启动盘，
      从它读回**内核自己的映像**（`/disk/KERNEL.ELF` 前 4 字节 = ELF 魔数 `0x7f454c46`）与
      引导程序（`BOOTX64.EFI` = `MZ`）；`/mnt/fat` 是 `mkfs` 生成的 FAT32 测试盘，
      跨簇文件（3000 字节 = 6 个簇）**逐字节对公式**、偏移 1024 的 8 字节读、空文件、LFN 长文件名、
      二级子目录、最长前缀路由（`/mnt/note.txt` → RAMFS、`/mnt/fat/*` → FAT）、负例 `ENOENT/ENOTDIR/EISDIR/EROFS`；
      编排场景由 9 个增至 **10 个**（`fat`）

#### v0.8 缺陷与设计记录

| 编号 | 等级 | 现象 | 处理 |
| ---- | ---- | ---- | ---- |
| B-16 | 中 | BAR 窗口大小算错：低字掩码（u32）被当成 u64 取反 → 类型正确的 I/O BAR 报出 `0xFFFFFFFF00000080` 这种尺寸（真实值 0x80） | 大小改为在该字**自己的位宽**里求 `~mask + 1`（`0x1_0000_0000 - (mask & !0xF)`）；另补两条边界：掩码全 1 = BAR 未实现，64 位 BAR 低字掩码为 0 → 大小由高字决定 |
| B-17 | 高 | `lba + sectors` 用 `u64` 加法：`lba = u64::MAX - 1` 时环绕后的和为 0，「看起来」在容量内 → 越界请求会发给设备 | 改用 `checked_add`，溢出即报错；负例测试加入 `lba=u64::MAX-1` |

设计取舍（与 ADR 同级，理由见 docs/drivers.md 1.2）：

- PCI 走配置机制 #1（不依赖 ACPI）而不是 ECAM；
- 只实现 legacy virtio，modern 遇到就报错（QEMU 用 `disable-modern=on` 提供 legacy）；
- 块 I/O 同步轮询（自旋 + PIT tick 超时），不配置 MSI-X/INTx；
- 驱动注册表容量固定、启动期写入、之后只读（`main` 只看 `ProbeStats`）。

### v0.9 — Shell + User Programs + Network（已实现并通过验收，2026-10-11）
- [x] **运行时前置条件第一步：用户堆**（2026-10-11 落地）
    - [x] `brk`（256 KiB 窗口，页对齐、增长全成或全不成；“失败返回原值”的 Linux 语义）
    - [x] `mmap`（只收 `MAP_PRIVATE|MAP_ANONYMOUS`，5 参数无 offset；窗口 256 KiB、first-fit；**新页全零**；`PROT_WRITE|PROT_EXEC` → `EINVAL`（W^X））
    - [x] `munmap`（整段归还；清页表项 + `invlpg`；未对齐/未映射 → `EINVAL`）
    - [x] `libc::heap`：`#[global_allocator]`（bump over `mmap`，64 KiB 块，`dealloc` 不回收 —— 由 `UserSpace::Drop` 在运行结束时整棵释放）
    - [x] 参考用户程序加入堆自测（零页、模式读回、堆缓冲 `write`、6 个负例、`Vec`/`format!`）
    - [x] 验收：`make test-user` 由 32 项增至 **59 项**（含 `user: heap frame reclamation PASS`）；
          `orchestrate.json` 的 `user` 场景同步；`mprotect` 列入「号已定义但未实现」
          （已于 v1.0 第一步实现，见 5.7）
- [x] **运行时前置条件第二步：`stdin` + 交互式 Shell**（2026-10-11 落地）
    - [x] 键盘行缓冲（`driver/keyboard.rs`）：128 字节单行缓冲、行编辑（退格/回显）、`Enter` 交付整行 + `kbd: line ready len=N`
    - [x] `read(0)` 行缓冲语义（`syscall/file.rs::read_stdin`）：整行交付（行尾 `\n`）、3 秒无整行 → `-EAGAIN`(-11)（无进程模型时的权宜之计）
    - [x] `user/shell`（→ `/bin/sh`）：提示符 `root@cerlesse C:>`（用户名@主机名 盘符>）、参数切分、内建 `help`/`echo`/`cat`/`pid`/`exit`
    - [x] 外部命令路径：真的调 `fork()` → 当时内核回 `syscall: ENOSYS nr=57 name=fork`（依赖关系的实测证据；
          **v0.9 第三步已实现**，该 ENOSYS 留痕自然消失，`make test-shell` 的断言同时改为外部程序路径）
    - [x] `kernel/build.rs` 由「单个用户程序」改为「按列表构建+校验+内嵌」（`/init` + `/bin/sh` 同路数）
    - [x] 验收：新增 `make test-shell`（**11 项**，QEMU monitor 注入真实按键）+ 编排 `shell` 场景（**11 场景**，后续第三步又加为 12）；
          `make test-user` 由 59 项增至 **61 项**；`make test-fs` 根目录条目数 3 → 4（新增 `/bin`）
- [x] **运行时前置条件第三步：进程对象 + `fork`/`execve`/`wait4`**（2026-10-11 落地）
    - [x] 进程对象（`process/process.rs`）：真正的 PCB（地址空间 / 用户帧 / 每进程 64 KiB 内核栈
          + 栈底哨兵 / `return_to` / `Ready|Running|Zombie`）；旧入口假设「一次只跑一个用户程序」的问题
          在 docs/syscall.md 3.6 / process.md 1.4 记下了根因
    - [x] `syscall/process.rs`：`fork`（复制地址空间 + 克隆当前用户帧）、`execve`（换映像 + 重建 argv 用户栈，
          先切 CR3 再 Drop 旧地址空间）、`wait4`（延迟调度 + 回收）
    - [x] `trampoline` 改为按进程切换：CR3 / `TSS.RSP0` / syscall 栈顶 / 当前进程下标；`suspend`/`resume` 语义
    - [x] `argc/argv` 传递（`libc/src/args.rs`，入口寄存器 `rdi`/`rsi`）
    - [x] 外部程序 `user/hello-bin` → RAMFS `/bin/hello`；Shell 的 `cmd_external` 改走 `fork`+`execve`+`wait4`
          （`ls` → `/bin/ls` 不存在 → `execve` 返 `-ENOENT` → 子进程 `exit(127)`）
    - [x] 验收：新增 `make test-proc`（**8 项 + 4 条正则**；`--expect-re` 用反向引用断言
          fork/execve/wait4 三处 PID 相同、并断言 `frames free before=N after=N` 无页帧泄漏）；
          编排新增 `proc` 场景（11 → **12 场景**）；`make test-shell` 12 → 11 项
- [x] **运行时前置条件第四步：简化网络栈**（2026-10-11 落地）
    - [x] `driver/net.rs`：网络设备抽象 `NetworkDevice`（以太网帧收发）+ 注册表
    - [x] `driver/virtio_net.rs`：legacy virtio-net 0.9.5（RX/TX 双队列、10 字节 virtio-net 头、
          MAC 读自设备配置空间、同步轮询；modern 明确拒绝）
    - [x] `network/{ethernet,arp,ipv4,udp,dhcp}.rs`：Ethernet II → ARP（带 IP→MAC 缓存）→
          IPv4（互联网校验和）→ UDP（伪头校验和）的纯函数组帧/解析
    - [x] `network/mod.rs`：`Stack`（设备 + 地址 + ARP 缓存）+ `selftest`（ARP 解析网关 + DHCP DISCOVER → OFFER）
    - [x] `main.rs`：磁盘验收后跑网络自测，**无网卡时打印一行并跳过**（不把「没有网卡」当失败）
    - [x] 验收：新增 `make test-network`（**13 项**，QEMU 加 legacy virtio-net-pci + `-netdev user`）；
          编排新增 `network` 场景（12 → **13 场景**）；`tests/test_boot.py` 增 `--nic`
- [x] **运行时前置条件第五步：基础用户程序（`ls`/`cat`/`echo`/`ps`）**（2026-10-11 落地）
    - [x] `syscall/dir.rs`：`getdents`（号 78）—— VFS `list()` → 定长 88 字节记录（`kind`/`size`/`name_len`/`name`）
          写进用户缓冲；装不下就**截断**（每条完整），缓冲小于一条记录返回 `-EINVAL`
    - [x] `syscall/process.rs::sys_getprocs`（号 **256**）—— 定长 40 字节记录（`pid`/`ppid`/`state`/`comm`），
          按**槽位顺序**枚举；`syscall_table.rs` 由此确立「`0..=255` Linux 语义 / `>= 256` Cerlesse 扩展」并改 `SYS_MAX` 257
    - [x] `process/process.rs`：新增定长 `comm: [u8; 16]`（`create` 填充调试名，`execve` 成功后改写为程序名 ——
          否则 `ps` 看到的永远是 `"exec"`）；顶层程序运行前置为 `RUNNING`
    - [x] `libc`：`getdents`/`getprocs` 封装 + `dirent_name`/`dirent_is_dir`/`dirent_size` 与
          `procinfo_pid`/`procinfo_ppid`/`procinfo_state`/`procinfo_name` 解析器（偏移只有内核侧一个权威定义）
    - [x] `user/{ls,cat,echo,ps}` → RAMFS `/bin/*`（`kernel/build.rs`、workspace、`fs::init` 清单 3 → 7）；
          Shell 帮助文本同步，并说明「内建命令遮蔽同名程序」（`cat`/`echo` 要走程序得写 `/bin/...`）
    - [x] 验收：新增 `make test-programs`（**17 项 + 5 条正则**，QEMU monitor 注入
          `ls` / `ls /bin` / `ps` / `/bin/echo …` / `/bin/cat /etc/motd`）；编排新增 `programs` 场景（13 → **14 场景**）；
          `make test-proc` 的“找不到的程序”负例由 `ls` 改为 `foo`
- [x] **运行时前置条件第六步：内核侧最小动态链接**（2026-10-11 落地）
    - [x] `syscall/elf.rs::load` 按 `e_type` 分流：`ET_DYN` → 新增 `syscall/dynamic.rs`（`ET_EXEC` 路径一行未改）
    - [x] `dynamic.rs`：解析 `PT_INTERP`（→ VFS `/lib/ld.so`）、`PT_DYNAMIC`/`DT_NEEDED`（→ `/lib/<name>`，递归，
          `MAX_OBJECTS = 8` 挡住 `DT_NEEDED` 环）与 `DT_*` 表项；`DT_PLTREL=REL` **明确拒绝**
    - [x] 共享库窗口（`address_space.rs`）：`USER_DYN_BASE` = 基址 + 1 MiB、32 页；`map_dyn_page`（幂等、按需、
          新页全零、同一套 W^X）+ `dyn_page_count` + `phys_of`（内核 CR3 下写重定位）；`fork` 复制窗口、
          `page_flags` 认可窗口、`Drop` 整树释放
    - [x] 重定位：`R_X86_64_RELATIVE` 与 `GLOB_DAT`/`JUMP_SLOT`/`R_X86_64_64`（在所有对象的 `.dynsym` 里
          按名字线性扫描，只认已定义符号）；未知类型跳过并留日志，解析不到符号 → `-ENOEXEC`
    - [x] 夹具 `user/dynhello/`（`as` + `ld` 现编，`kernel/build.rs` 内嵌）与 RAMFS `/lib`
          （`fs::init` 新增 `libraries` 参数；`PROGRAM_IMAGES` 7 → 8，与 `LIBRARY_IMAGES` 分开）
    - [x] 验收：新增 `make test-dyn`（**10 项**：`PT_INTERP`/`NEEDED`/两个装载基址/`JUMP_SLOT` 重定位/
          `link PASS objects=3 relocs=2`/两条 ring3 输出/`exit code=42`/无帧泄漏）与
          `make dyn-hostcheck`（**宿主机 glibc `ld.so` 独立复核**同一批 `.s`，两套装载器必须一致）；
          `make test-user` 的帧回收检查覆盖共享库窗口；`test-fs` 20 → **21 项**（新增 `/lib`）、
          `test-programs` 条目数断言 `/`=5、`/bin`=7；`make test-all` 加入 `test-dyn`
- [x] `mprotect`（W^X 转换）→ **2026-10-11 落地（v1.0 第一步，见 5.7）**；
      `mmap` 加 offset（文件映射）与固定地址放置仍未做
      （动态链接的**完整**形态 —— 用户态 `ld.so`、版本符号、TLS —— 仍不做，见 architecture.md 第 5 节）
- [ ] 其余用户程序（cp/mv/rm/mkdir/kill）与 Shell 增强（`cd`、管道、重定向）
      （`ls`/`ps`/`cat`/`echo` 已于 2026-10-11 落地，见上）
- [x] TCP + Socket syscall 最小集（2026-10-14 落地：`network/tcp.rs` 三次握手/收发/关闭 + `network/socket.rs` 8 槽表 + `socket`/`connect`/`send`/`recv`/`close`；`user/tcpecho` 回显往返；ICMP 仍为占位，见下）
- [x] 验收：Shell 能启动外部程序（`make test-proc`，2026-10-11）；基础用户程序可跑（`make test-programs`，2026-10-11）；
     网络最小闭环（`make test-network`，2026-10-11）；动态链接最小闭环（`make test-dyn` + `make dyn-hostcheck`，2026-10-11）；
     ping（ICMP）仍待落地

### 5.7 v1.0 — 完整可启动类 Linux 小型 OS

- [x] **内存保护属性可在运行期改写：`mprotect`**（2026-10-11 落地，第一步）
    - [x] ABI 层（`syscall/memory.rs::sys_mprotect`）：页对齐、`len > 0`、未知保护位、
          `PROT_WRITE|PROT_EXEC`（W^X）—— 每类拒绝都留 `user: mprotect rejected (<reason>)`
    - [x] 地址空间层（`address_space.rs::mprotect` + `prot_to_flags`/`accessible`）：范围必须落在
          **同一个**窗口（映像 / brk 堆 / 匿名 mmap / 共享库）且每页已映射；
          `PROT_NONE` = 清 `USER` 留 `PRESENT`（ring3 #PF、帧与内容保留、内核也不当它用户缓冲）；
          每次改权限都 `invlpg`；`mmap(PROT_NONE)` 随之变为合法
    - [x] 堆窗口新增逐页标志位 `heap_flags`（`brk` 增长/收缩与 `fork` 同步维护），
          `page_flags` 改为「`PRESENT` + `USER` 齐备才算可访问」
    - [x] 用户态：`libc::mprotect` + `PROT_NONE`；`user/hello` 新增 `mprotect_selftest` ——
          `PROT_NONE` 关页 → 内核拒收该缓冲（`-EFAULT`）→ 改回 `RW` 内容原样 →
          换 `RX` 并**真的执行** `mov eax,42; ret`（W^X 的「先写后换只执行」）
    - [x] 验收：`make test-user` 61 → **91 项** PASS（新增 30 项：4 条 `user: mprotect addr=… -> pages=N`（mmap 窗口
          `PROT_NONE`/`RW`/`RX` + brk 堆窗口 `PROT_NONE`/`RW`）、两处 `PROT_NONE` 的内核拒收双向证据（`-EFAULT`）、
          `data kept=1`、`exec ret=42`、6 类拒绝的 errno 与串口原因、`munmap` 照常归还）
    - [x] 副作用：`syscall_table::UNIMPLEMENTED` 变**空表**（v0.9 起唯一剩下的号是 `mprotect`），
          `syscall: ENOSYS nr=… name=…` 机制与「号越界」仍保留可区分
    - [x] 构建侧顺带修复：`kernel/build.rs` 在「清单不存在」分支也把默认 `manifest.txt` 登记为
          构建输入，避免「先编内核、后出用户态镜像」时内核被永久钉在「无用户态」的旧产物上
- [ ] 用户/组/权限（`-rwxr-xr-x`、UID/GID）
- [ ] 进程隔离与 syscall 参数校验强化
- [ ] 启动链：Kernel → init → Shell → User Programs
- [x] **驱动兼容性：AHCI/SATA 驱动 + PCI 枚举修缺陷（B-18）**（2026-10-14 落地）
    - [x] `driver/pci.rs`：头部类型偏移 `0x0C`（cache line size，恒为 0）→ `0x0E` ——
          多功能设备的 function 1..7 以前**从未被枚举**（q35 上 `enumerated devices=5` → 7，
          芯片组的 SATA `00:1f.2` / SMBus `00:1f.3` 才可见）；新增命令寄存器读写
          （`command`/`enable_command`：内存空间 + 总线主控）
    - [x] `driver/ahci.rs`：AHCI 1.3（命令列表 / 接收 FIS / 命令表 + PRDT、IDENTIFY DEVICE
          取型号与容量、LBA48 READ/WRITE DMA EXT、`PxCI` 同步轮询、4 KiB bounce buffer）；
          **按类别匹配**（`0x01`/`0x06`/`0x01`，不看厂商）；没插盘时 `attach` 照样成功并打印 `drives=0`；
          ATAPI（光驱）按 `PxSIG` 跳过
    - [x] `main.rs`：有 SATA 盘时对 `ahci0` 再跑一次 **同一个** `block::selftest`
          （上层 FAT/VFS/CSS 零改动 —— 第二条块设备路径与 virtio-blk 共用一个 trait）
    - [x] 验收：新增 `make test-ahci`（**24 项**，QEMU 把 `build/fat.img` 插到 ICH9 的 `ide.2`）
          并入 `test-all`（**19 目标**）；编排场景 `ahci`（**20 场景**）；`test-driver` 23 项
          （`matched/attached` 由 1 改 2：AHCI 控制器也被认领）；实测 SATA 上挂载的
          `fs: mount point=/mnt/fat fs=fat32 …` + `fs: disk selftest PASS` 与 virtio 路径结论一致
- [ ] 文档全量修订、发布 v1.0 tag
- [ ] 验收：QEMU 一键启动到可交互 Shell

### 5.8 缺陷与修复汇总

| 编号 | 版本 | 级别 | 问题 | 修复 |
| ---- | ---- | ---- | ---- | ---- |
| B-01 | v0.1 | 高 | `_start`→`kernel_main` 内 `cli` 前存在中断窗口（IDT 仍是固件遗留） | `_start` 第一行 `cli`，入口即刻关中断 |
| B-02 | v0.1 | 中 | 内核无自有 GDT/IDT，异常直接 triple fault | v0.2 自建 GDT/IDT + 异常诊断 |
| B-03 | v0.1 | 低 | 异常处理无自动化回归 | 新增 `make test-exception` |
| B-04 | v0.1 | 低 | 启动日志含一次性低地址内存诊断 | 保留（<64MiB，用于装载冲突诊断） |
| B-05 | v0.2 | 高 | 内核默认链接为 PIE，绝对数据依赖 `.rela.dyn`；bootloader 不处理动态重定位 → handler 地址错误 | 强制 `relocation-model=static`（ADR-008） |
| B-06 | v0.3 | 中 | 异常名表自向量 9 起错位一位 | 按 Intel SDM Table 6-1 重建 32 项名表 + `test-pagefault` |
| B-07 | v0.3 | 中 | 堆分配器把对齐前导 padding 计入块大小 → 越界覆盖空洞头 | 块大小改为 `want - pad` + 填充模式校验 |
| B-08 | v0.4 | 高 | IRQ 向量 33..47 走入异常分发 → IRQ1 触发即 panic | 16 个 IRQ stub 统一 `jmp irq_common` |
| B-09 | v0.4 | 高 | IRQ 路径未保存通用寄存器 → 被打断代码带破坏寄存器继续，随机 #GP | `irq_common` 保存/恢复 15 个通用寄存器 |
| B-10 | v0.5 | 高 | 上下文切换未保存 `RFLAGS`（切出后 IF 永久为 0） | `context_switch` 加 `pushfq`/`popfq`；新上下文 `RFLAGS=0x202` |
| B-11 | v0.5 | 高 | IRQ EOI 在回调之后（IRQ0 回调触发切换后 PIC 停摆） | 改为「先 EOI，后分发」 |
| B-12 | v0.5 | 中 | 启动上下文切出后未回就绪队列，Round Robin 绕不回 pid 0 | `start()` 切出前把 pid 0 压回队尾 |
| B-13 | v0.6 | 高 | 创建用户地址空间时无条件重写整棵内核页表清 `USER`，OVMF 页表页只读 → 启动 #PF | `scrub_user_bits` 惰性：仅当 `USER` 确为 1 时写回 |
| B-14 | v0.6 | 高 | `syscall` 返回路径栈算术错误（`add rsp,48`）→ `sysretq` 跳飞 `RIP=0x3` | 改为 `sub rsp,16` 后 `pop r11`/`pop rcx` |
| B-15 | v0.7 | 高 | syscall 入口未恢复 caller-saved 寄存器 → 用户态 `r9` 被破坏，#PF | 入口保存/恢复 `rdi/rsi/rdx/r10/r8/r9` |
| B-16 | v0.8 | 中 | BAR 窗口大小按 64 位取反，I/O BAR 报出错误尺寸 | 大小在该字**自身位宽**里求 `~mask + 1` |
| B-17 | v0.8 | 高 | `lba + sectors` 用 `u64` 加法，`lba = MAX-1` 环绕后被误判在容量内 | 改用 `checked_add`，溢出即报错 |
| B-18 | v1.0 | 高 | PCI 头部类型读在偏移 `0x0C`（cache line size，恒为 0）而非 `0x0E` → `MULTI_FUNCTION` 判据永远为假，**多功能设备的 function 1..7 从未被枚举**：q35 上 `00:1f.0` 的 function 2（SATA/AHCI）与 function 3（SMBus）不在设备表里（固件的引导日志里却有 `Pci(0x1F,0x2)/Sata(...)`） | 偏移改为 `0x0E`；`enumerated devices=5` → 7；新增 `make test-ahci` 把这个 function 变成被断言的设备 |

v0.9 记录的两条踩坑（设计记录，非编号缺陷）：debug 构建下 16 KiB 内核栈静默溢出踩坏 `.bss` 帧分配器；
子进程退出时必须**先恢复 CR3 再 Drop 地址空间**。详见 [docs/process.md](docs/process.md) 1.4 与
[docs/syscall.md](docs/syscall.md) 3.7。

### 5.9 关键设计决策（ADR）

见 [docs/architecture.md](docs/architecture.md) 第 6 节（ADR-001 … ADR-011）。新增重大决策时追加编号并同步本文。

### 5.10 桌面兼容路线（v1.0 之后，规划）

见 [docs/desktop.md](docs/desktop.md)（现状审计 + 用户态/内核侧归属纠正 + D0–D5 分层 + QEMU 可断言验收）。
里程碑建议：

| 版本 | 目标 | 验收抓手 |
| ---- | ---- | ---- |
| v1.1 | D0 地基：设备模型 + `ioctl` + `poll` + `/proc`/`/sys` 最小视图 | `make test-devfs` / `make test-procfs` |
| v1.2 | D1 输入：输入核心 + evdev + uinput | `make test-input` |
| v1.3 | D2a fbdev + `screendump` 像素断言 | `make test-fb` |
| v1.4 | D2b KMS 子集 + `virtio-gpu` | `make test-kms` |
| v1.5 | D3 音频 + D4 电源/会话地基 | `make test-snd` / `make test-power` |
| v1.6+ | D5 网络/文件系统/权限收尾 | 逐项 |

> 优先级修正（见 desktop.md 第 5 节）：**先 D0 地基，再 D1 输入，再 D2 显示**；
> systemd-logind / D-Bus / PipeWire 是用户态组件，KVM 与桌面前置无关（它是「当宿主」的方向）。

### 5.11 纯内核化改造（v0.9 之后，规划；执行上在桌面路线之前）

见 [docs/pure-kernel.md](docs/pure-kernel.md)（现状审计 C1–C9 + **逐文件迁移清单** + 六阶段 Cargo 配置/构建目标/验收 + 第 8 节执行记录）。

**执行状态（2026-10-12）**：阶段二 ✅（`tools/check-boundary.py`：6 项 + `--selftest`，已并入 `make test-all`）、
阶段三 ✅（**`abi/` 落地**：调用号/errno/标志位/记录布局/CKM 与存储字面量移到单一事实源，内核 18 文件、
libc 45 常量改为引用它；新增 `make abi-check` 并并入 `make check`；两个反例已验证 —— 改号 → `abi-check` 红，
单侧改记录尺寸 → `test-programs` 红）、
阶段四 ✅（`libc/`+`user/` → `userland/` 独立 workspace；内核 `build.rs` 改写为「按 `manifest.txt` 内嵌」；
新增 `userland-images`/`image-kernel`/`kernel-nouserland`/`test-kernel-only` 目标）、
阶段五 🟡（Makefile 已按责任分层；`orchestrate.json` 拆分、`.ckc` 测试迁出、`git subtree split` 未做）、
阶段一/六 ⬜。实测：无 `userland/` 的 checkout 里 `cargo build -p kernel` 退出码 **0**（改造前 **101**）、
`make image` 能单独出镜像、`make test-kernel-only` **7/7** PASS；有用户态时 `make image` 内嵌
**8 程序 + 2 库**，15 个集成场景与 `test-user`(91)/`test-shell`(11)/`test-proc`(8+4)/`test-programs`(17+5)/
`test-dyn`(10)/`test-fs`(21) 全部 PASS。

| 阶段 | 目标 | Cargo 配置 / 构建目标 | 验收抓手 |
| ---- | ---- | ---- | ---- |
| 一 | 冻结当前基线 | 不改 Cargo；新增 `make baseline` | `make baseline` 退出码 0 + `git tag v0.9.0-baseline` |
| 二 | 边界门禁（内核不得依赖 Shell/libc/cpm） | 不改 Cargo；新增 `tools/check-boundary.py` | `make check-boundary`（含 `--selftest` 负例） |
| 三 ✅ | syscall ABI 单一事实源 | workspace 新增 `abi`；kernel/libc 均依赖它（**已落地 2026-10-12**） | `make abi-check`（`cargo test -p abi`，7 项）+ `make test-all` 与基线逐项一致 |
| 四 | 迁移用户空间 | 内核 `members` 收敛为 `shared,boot,abi,kernel`；新增 feature `embed-userland`（默认关）；userland 自带 `.cargo/config.toml` 与 `user-release` profile | 只含内核的 tarball 里 `make kernel && make image` 通过；`make test-kernel-only`；`make image-userland` |
| 五 | 构建与测试拆分 | 不改 Cargo；Make 目标分层 | 内核 `make test-all` + `make test-orch`（15 → **10 场景**）；userland 侧 5 场景 |
| 六 | 稳定内核接口 | `abi` 增 `VERSION`（决策：是否抽独立 crate） | `make abi-doc`；`docs/porting.md`；内核 tag `v1.0-kernel` |

> 关键事实（实测，见 pure-kernel.md 第 1 节）：当前删掉 `libc/` 与 `user/` 后 `cargo build -p kernel` **直接失败**
> （`kernel/build.rs:301` 报「用户程序构建失败」，退出码 **101**），内核映像内嵌用户态 **203,280 字节（≈199 KiB）**；
> 因此验收标准 1/2/7（不依赖用户态即可编译/启动）现状不成立。改造顺序上，**阶段三（ABI 单一来源）先于
> 阶段四（动用户态源码）**，避免迁移期 ABI 双写漂移。

---

### 5.12 CKM —— 内核级权限控制与扩展管理（H-07，v1.0 之后；**CKM-1/CKM-2 已落地**）

**来源**：希望 Cerlesse 有「内核级强制权限 + 可插拔扩展 + 故障恢复」的管理体系（分层思想参考 KernelSU，
但不照搬 Android 的 `su`/SELinux/root 隐藏机制）。**本文档不复制规格**，只记需求、依赖与口径：

见 [docs/ckm.md](docs/ckm.md)（权限模型 / 内核接口 / 扩展生命周期三项已定稿 + CKM-1…CKM-7 分期 + 8 条故障验收）。

- [x] **CKM-1 内核接口**（2026-10-12）：`kernel/src/ckm/`（`cred`/`policy`/`state`/`audit`/`api`）+ 判定点
      （写受保护前缀 / 写状态树 / 读审计 / 策略查询）+ `ckm_policy_query`(257)/`ckm_audit_read`(258)/
      `ckm_component_state`(259) + 串口证据；前置 v1.0 的 `security/`（uid/gid + mode）已就位。
      验收 `make test-ckm`（22 断言 + 正则）与 `test-proc` 的 uid 2000 对照，见 [ckm.md](docs/ckm.md) 9.1
- [x] **CKM-2 可写持久化 + 覆盖层机制**（2026-10-12）：开放问题 1 定夺为 **(b) 双槽 A/B + CRC32 的极小日志式区**
      —— 落在系统盘尾部 1 MiB（在 FAT 之外；镜像工具改为「先在 63 MiB 上 `mformat`、再追加保留区」），
      `kernel/src/ckm/store.rs` 提供 `get`/`put`（无活动指针：整槽写非活动槽 + CRC 校验）+
      `kernel/src/ckm/overlay.rs` 提供 `overlay_register/unregister`（最长前缀 + 优先级 + 同优先级冲突拒绝，
      钩子在 `fs::mount::resolve`）；验收 `make test-ckm`（40 断言：注册/撤销改变 `open` 结果）+ 新增
      **`make test-ckm-store`**（两阶段启动：写 → 重启 → 读回逐字节一致且第二次不重写）+ 编排场景 `ckm-store`，
      见 [ckm.md](docs/ckm.md) 9.2
- [x] **CKM-3 事务引擎**（= H-06 P0b，**主机侧已落地 2026-10-12**）：`userland/pkgmgr/cpm.py` 的回执/事务日志/`recover`/失败注入/`gc`/单写者锁（**签名/catalog 除外**，阻塞于 package-manager 开放问题 13.1）；验收 `make test-cpm`（**63 项**，与 package-manager 12.1 对齐）；把事务引擎接进内核侧（`ckmd`/`ckm`）属 CKM-4
- [ ] **CKM-4 特权服务与 CLI**：`userland/ckmd`（`CAP_CKM_SERVICE`）+ `userland/ckm`（CLI）+ 内核状态树 `/ckm/**`
- [ ] **CKM-5 系统修改模块**：`kind=3` 覆盖层全量（优先级/冲突/开机顺序/基线保护；停用即撤销）
- [ ] **CKM-6 恢复环境**：恢复镜像 + 引导菜单/命令行 + 启动失败计数 + 安全模式四件事
- [ ] **CKM-7 可加载内核模块**：仅登记状态；装载需独立提案（`docs/ckm-kmod.md`）先评审
- [ ] 验收（第一版必须过）：`make test-ckm`（权限拒绝/覆盖撤销/组件状态确定性）+ `make test-recovery`
      （连续 3 次启动失败进安全模式 + 恢复环境修复 `ckmd`）+ H-06 的离线回滚/原子失败条目

边界（必须保持）：内核不认识「包」（不解析 `.ckc`、不解压、不比较版本、不做依赖求解、不做界面）；
`ckmd`/`ckm` 属于 `userland/`（`tools/check-boundary.py` 继续把内核与用户态分开）；
`.ckc` 包格式不重做（H-06 P0a 直接复用）。

### 5.13 CSS —— Cerlesse Storage System（H-08，v1.0 之后；CSS-1/CSS-2 已落地）

**来源**：希望 Cerlesse **像 Windows 一样管理磁盘与卷**（盘符、卷标、卷 GUID/稳定标识）
**像 Linux 一样通过统一 VFS 兼容多种文件系统**（挂载点、多后端、统一文件接口）。
**本文档不复制规格**，只记需求、依赖与口径：见 [docs/storage.md](docs/storage.md)。

- [x] **CSS-1 底层识别**（2026-10-11 落地）：`kernel/src/storage/`（`mod`/`registry`/`partition`/`gpt`/`mbr`/`volume`/`selftest`）+
      `BlockDevice` 注册表 2 → 8 + `tools/mkfs/mkfs.py` 增加 GPT/MBR 镜像生成 +
      `storage_volumes`(261)/`storage_rescan`(265) + 卷记录（64 字节）；
      **无分区表的镜像（现有 `fat.img`/`disk.img`）继续识别为整盘一卷**；
      证据：内核解析自测 11 例 / 45 断言（含主表头损坏回退备用表头）、`make test-storage`（26 项）、
      `make test-storage-fixtures`、`make test-storage-mbr`、用户程序 `/bin/volumes`（`test-programs` 23 项 + 6 正则）
- [x] **CSS-2 VFS 收口**（2026-10-11 落地）：`FileSystem` 追加 `caps`/`volume_info`（带保守缺省，不重写 trait）、
      FAT 探测收归 `fs/fat.rs`（`probe`/`probe_sector`，删掉 `storage/volume.rs` 里重复的 BPB 判定）、
      挂载表拆 `backends`/`mounts` 并带 `vol_id`/`options`（新增 `alias_volume`/`umount`，挂载计数写回卷记录）、
      **挂载失败由 `&'static str` 改 errno**（`EEXIST`/`ENOSPC`/`ENOENT`/`EINVAL`/`ENOTSUP`/`EUCLEAN`）、`stat` 行尾 `vol=`；
      证据：`fs::storage_mount_selftest`（开机自测：只读原因来自能力位、后端与卷记录一致、
      同一卷多挂载点读到同字节、卸载后 `ENOENT` 且卷仍在表、`ENOENT`/`EINVAL`/`EEXIST` 反例）、
      `make test-storage`（31 项 + 2 正则）、`make test-storage-mbr`（14 项 + 1 正则）、
      `tests/test_boot.py` 新增 `--expect-re`（反向引用绑同一 `vol_id`）；
      不回归：`test-fs`/`test-fat`/`test-user`(91)/`test-programs`(23+6)/`test-driver`(22)/`test-kernel-only`(7)
      + 编排场景 `fs`/`fat`/`storage`/`storage-mbr`/`programs`
- [x] **CSS-3 Windows 式卷管理**（2026-10-12 落地，盘符部分）：盘符表 `kernel/src/storage/letters.rs`
      （确定规则 `C`/`D`/… + 自定义映射持久化到 CKM 状态区键 `storage/letters`，`assign`/`clear`/`selftest`）+
      `storage_mount`(262)/`storage_unmount`(263)/`storage_assign_letter`(264)（判定点 8 `CAP_STORAGE_ADMIN`）+
      `libc/src/path.rs` 的 `C:\` → POSIX 重写 + `/bin/volumes` 子命令；
      前置 **CKM-2 可写区已就绪**（字母表真的落盘，不假装持久化）；
      证据：`make test-storage`（`letter=` 字段 + `storage: letters selftest PASS (checks=36)`）、
      `make test-storage-mbr`（`checks=22`）、`make test-programs`（`volumes demo` 双入口同字节 + 卸载后 `ENOENT`）、
      `make test-ckm`（零能力进程 `-> -1` need=0x10）、`make test-ckm-store`（`storage/letters` 跨重启读回）；
      **仍缺**：卷标写/改名、`/storage/{volumes,partitions,letters}` 只读状态树、Shell 提示符改用盘符表
- [ ] **CSS-4 可写存储**：FAT 写 → exFAT 读写 → ext2 读写（所有写入走 VFS；掉电一致性二选一先定夺）
- [ ] **CSS-5 管理程序**：`ckm storage list/vol/part/mount/umount/letter/fsck`（`fsck` 只读检查；写回修复为第一版非目标；随后 TUI）
- [ ] **CSS-6 兼容性扩展**：ext4 只读 → NTFS 只读 → ISO9660 只读；XFS/Btrfs 研究
- [ ] 验收（第一版必须过）：`make test-storage`（分区/卷枚举确定性、无分区表退化为整盘一卷、
      双路径同一文件、字母不漂移、只读原因、`degraded` 健康、卸载、`ENOTSUP` 与 `ENOENT` 可区分、
      未授权挂载被拒）+ **不回归**（`make test-fs`/`test-fat`/`test-user` 全 PASS）

边界（必须保持）：**盘符不属于文件系统驱动**（`fs/*` 不认识 `C:`，只实现 `FileSystem`）；
**CSS 不重写 VFS**（新格式 = 再实现一个 `FileSystem`）；**第一版不写分区表**（GPT/MBR 只读扫描）；
**文件系统后端编译内置**（第一版不经 CKM 扩展体系运行时装卸；可加载内核模块属 CKM 阶段 7）；
卷/分区/盘符的归属：内核机制在 `kernel/src/storage/`，权限走 CKM 判定点 8–10，
界面在 `userland/`（`tools/check-boundary.py` 继续生效）。

## 6. 测试方案

| 层级 | 内容 | 工具 | 版本 |
| ---- | ---- | ---- | ---- |
| 单元测试 | 纯逻辑模块（分配器算法、ELF 解析、路径解析）host 上 `cargo test`（**计划**，暂以内核内自测覆盖） | Rust | v0.4 起规划 |
| 集成测试 | QEMU 启动 → 断言串口日志包含预期输出 | Python (H-02) | v0.1 |
| 压力测试 | 帧分配、任务轮转、fork 风暴 | Python + Go 编排 | v0.3/v0.5 |
| 回归测试 | 每个里程碑的验收标准固化为测试用例 | Go 编排服务 | v0.4 |
| 异常测试 | 故意触发 #DE/#PF，校验诊断输出 | Python | v0.2/v0.3 |
| 驱动/块设备 | PCI 枚举 → 驱动挂接 → 扇区读写/分块一致性/负例 | Python | v0.8 |
| 磁盘文件系统 | 双盘启动 → FAT16 启动盘读内核映像 / FAT32 测试盘读·LFN·跨簇·偏移·负例 | Python | v0.8 |
| 包容器（不启 QEMU） | `.ckc` 确定性、头部逐字段、payload 篡改、ustar 规则、打包期规则 + `tar` 独立复核 | Python | H-06 P0a |
| 键盘/交互/进程 | QEMU monitor `sendkey` 注入真实按键 → 行缓冲/Shell/进程调用全流程（`--expect-re` 支持正则 + 反向引用） | Python | v0.4，v0.9 扩展 |
| 动态链接 | 内核装载器（`make test-dyn`）与宿主 glibc `ld.so`（`make dyn-hostcheck`）跑同一份 `.s` 夹具，输出与退出码一致 | Python + binutils | v0.9 |

约定：

- 每个版本任务清单里的「验收」必须落成自动化测试。
- 串口是唯一权威日志通道，测试断言只依赖串口输出。
- 目标与项数见 [docs/api.md](docs/api.md) 第 7 节。

---

## 7. QEMU 调试方案

```bash
make run
# 底层等价于：
qemu-system-x86_64 -machine q35 -m 512M -serial stdio -display none \
  -drive if=pflash,format=raw,readonly=on,file=$OVMF_CODE \
  -drive if=pflash,format=raw,file=build/OVMF_VARS.fd \
  -drive if=none,id=blk0,format=raw,file=build/disk.img \
  -device virtio-blk-pci,drive=blk0,disable-modern=on
```

- `disable-modern=on` 必需：内核只实现 legacy virtio（见 docs/drivers.md 1.2）。
- 测试脚本额外加 `snapshot=on`（临时写时复制），使并行场景共享镜像且不污染原文件。
- 调试：`-s -S` + `gdb target remote :1234`；崩溃用 `-no-reboot -d int,cpu_reset`；
  内核 ELF 保留符号，`add-symbol-file kernel.elf` 后可按函数名下断点。
- 更完整的开发/调试流程见 [docs/software-development.md](docs/software-development.md) 第 5 节。

---

## 8. GitHub Issue / 里程碑规划

里程碑与版本一一对应（Milestone 标签 `v0.1` … `v1.0`），Issue 标签分类：

| 标签 | 用途 |
| ---- | ---- |
| `boot` / `memory` / `interrupt` / `process` / `syscall` / `fs` / `driver` / `network` / `security` / `ckm` | 模块归属 |
| `phase:bug` / `phase:feature` / `phase:refactor` / `phase:doc` | 类型 |
| `host:python` / `host:go` | 主机侧工具 |
| `blocked:upstream` | 被依赖项阻塞 |

规划规则：

1. 每个里程碑开工时，把第 5 节对应版本的 checklist 拆成 Issues（一个 checklist 项 ≈ 1 个 Issue）。
2. Issue 标题格式：`[v0.x] 模块: 简述`，例如 `[v0.3] memory: 实现 bitmap FrameAllocator`。
3. 里程碑关闭条件：任务清单全部勾选 + 验收测试通过 + 对应阶段文档更新。

---

## 9. 文档维护约定

1. 每份文档顶部保留 `阶段` / `最后更新` / `状态` 三行。
2. **设计变更先改文档再改代码**（文档即规格）；未开始的阶段保留占位大纲，
   但阶段内已落地部分应及时补记，避免文档与代码库状态脱节。
3. 新增/调整文档时同步 [docs/README.md](docs/README.md) 索引与相关交叉链接；
   **对外接口变化必须同步 [docs/api.md](docs/api.md)**。
4. 文档会被反复修改：每次修订在该文档的「修订记录」补一行（日期 + 变更点）。
5. 面向开发者的流程/规范统一收敛到 [docs/software-development.md](docs/software-development.md)。

---

## 10. 修订记录

> 本表按日期从旧到新排列；每轮迭代的变更点在此追加（不删除历史）。

| 日期 | 变更 |
| 2026-10-12 | **纯内核化阶段三：系统调用 ABI 单一事实源（`abi` crate）** —— 新增 `abi/`（`no_std`、零依赖）：调用号/`SYS_MAX`/号段规则（含 `NAMED`/`UNIMPLEMENTED`）、`errno`、`O_*`/`PROT_*`/`MAP_*`/fd 编号、`PATH_MAX`/`IO_MAX`/`ARG_*`、`dirent`(88)/`procinfo`(40)/`volume`(64)/`ckm`(48/64) 记录布局与能力位/op 编号/受保护前缀、编译期断言（`record.rs`）；根 `Cargo.toml` 变为 `shared/boot/abi/kernel`，`kernel` 与 `userland/libc` 各加路径依赖；**内核 18 个文件、libc 45 个常量改为引用它**（对外名字与类型不变，行为零变化）；新增 `make abi-check`（`cargo test -p abi`，7 项）并并入 `make check`。实测：两处构建退出码 0、`make image` 内嵌 9 程序 + 2 库、`make abi-check` 7/7、`make check-boundary` 6/6、14 个 QEMU 目标（test/test-user/test-programs/test-proc/test-ckm/test-storage/test-storage-mbr/test-fs/test-dyn/test-shell/test-memory/test-scheduler/test-ipc/test-kernel-only）全 PASS；两个反例（改 `SYS_GETPROCS` → `abi-check` 退出码 2；单侧改 libc 的 `DIRENT_SIZE` → `test-programs` 退出码 2 且 `ls` 报错）证明 ABI 既被固化也被集成测试覆盖。同步 docs/pure-kernel.md（C6 → ✅、3.1/3.7/4/5/6 节、新增 8.2）、docs/syscall.md、docs/project-structure.md、README.md |
| 2026-10-12 | **CKM-2（可写持久化 + 覆盖层机制）落地** —— 新增 `kernel/src/ckm/store.rs`（系统盘尾部 1 MiB 的**双槽 A/B + CRC32** 状态区：`init`/`get`/`put`/`key_count`/自测；`kernel_main` 在 `storage::init` 之后调用，不需要文件系统）与 `kernel/src/ckm/overlay.rs`（覆盖层注册表 `register`/`unregister`/`lookup`/`rewrite` + 最长前缀/优先级/同优先级冲突拒绝；判定点 7/8 的落点）；`fs::mount::resolve`/`route` 在查表之前接入路径改写（空表快路径），`fs::overlay_selftest` 在 `fs/` 侧建立 `/ckm/overlay/**` 并断言「注册/撤销改变 `open` 结果」（`ckm/` 不反向依赖 `fs/`）；新增 `policy::reject`（语义拒绝，`need=0x0`，与能力不足可区分）；`tools/image-builder/build_image.py` 改为「先在 63 MiB 上 `mformat`、再追加 1 MiB 保留区」（设备仍 64 MiB、`table=none partitions=1`，FAT 看不到状态区）；新增 `make test-ckm-store` + `tests/test_ckm_store.py`（**两阶段启动**：清零独占副本 → 写（`state=fresh`，槽 A/B 交替）→ 同一镜像重启 → `state=loaded` 读回逐字节相同且不重写）与 `tests/test_boot.py --persist`、编排场景 `ckm-store`（共 **19 个场景**）；`test-ckm` 22 → **40 断言**，`test-fat`（启动盘 FAT 64 → 63 MiB：`sectors=129024 clusters=64244`）、`test-programs`（`/` 5 → 6 项、`ls: entry name=ckm`、`bytes=66060288`）、`test-storage` 的断言同步；全量回归 PASS（test-ckm / test-ckm-store / test-kernel-only / test-fs / test-fat / test-storage / test-storage-mbr / test-user 96 / test-driver 22 / test-proc / test-programs 24+6 / test-storage-fixtures / test-pkg 37 / check-boundary / 编排场景 ckm·ckm-store·programs·storage·storage-mbr·fs） |
| ---- | ---- |
| 2026-10-06 | 第一阶段初稿：需求、依赖、workspace 规划、v0.1–v1.0 任务清单、测试/QEMU/Issue 规划 |
| 2026-10-07 | v0.3 完成：内存管理（帧分配器/页表 Mapper/内核堆）；新增 test-memory、test-pagefault 验收目标；补记 B-06/B-07 |
| 2026-10-08 | v0.4 完成：PIC/PIT 100Hz/PS-2 键盘/上下文切换原语；新增 test-keyboard 验收目标；补记 B-08/B-09；H-04/H-05 主机侧 Go 工具仍待开工 |
| 2026-10-08 | v0.5 内核侧落地：线程模型/PID/就绪队列/Round Robin 时间片调度接入 PIT/IRQ0；新增 test-scheduler；补记 B-10/B-11/B-12；IPC 仍待做 |
| 2026-10-08 | 本次 docs 迭代统一文档与代码库状态不一致点：v0.4 收官前，部分文档一度记载 H-04/H-05 仍待编写；但工具代码已实际落地；随后复核确认 `make go-test`（vet + test）与 `make test-orch`（6 场景并行，6 passed / 0 failed）均通过，v0.4 最终标记为整体完成 |
| 2026-10-08 | 继续 v0.5 后续开发前，统一当前未完成项表述：v0.5 仅剩 IPC(pipe/channel/shared_memory) 未落地；syscall.md 与 filesystem.md 仍为占位文档，尚未进入 v0.6/v0.7 编码 |
| 2026-10-08 | 按开发流程复查后更新：v0.4 与 v0.5 都尚未在 DEVELOPMENT 里标记为整体“以完成”，因为当前尚未完成项尚未通过验收/尚未实现 |
| 2026-10-08 | 本次 docs 迭代统一各阶段文档顶部元信息（阶段 / 最后更新 / 状态）、修订记录与本文件描述一致，保持索引与主开发文档一致 |
| 2026-10-10 | **v0.6 核心落地**：`syscall`/`sysret` 入口 + 真实用户页表（PML4 下标 1，每级 `USER`）+ ring3 进入/退出 + `write/exit/getpid/sleep` + ring3 异常终止（不再 panic）；新增 `make test-user` 18 项断言与 orchestrate.json `user` 场景（7 场景）；补记 B-13（固件页表只读 → 惰性清 `USER`）、B-14（`syscall` 返回栈算术）；同步 docs/syscall.md 从占位改为实现记录 |
| 2026-10-11 | **v0.6 用户态脚手架可用**：`libc/` 从占位变为实现（syscall 封装 + `_start` + `print!/println!` + `panic_handler`）；新增真实用户程序 `user/hello` 并入 workspace（含 `default-members` 与 `user-release` profile，large 代码模型 + `lto = false`）；`kernel/build.rs` 构建期构建用户程序并摊平成连续映像内嵌（断言装载基址 / 入口在映像第 0 字节 / 无可装载重定位）；`UserSpace::load_program` 支持多代码页（1..=8）；新增 `make user`，`make test-user` 增至 **19 项断言**（含 `user: program=hello source=libc`）；全量回归（test / test-memory / test-scheduler / test-ipc / test-exception / test-pagefault / test-keyboard / go-test / test-orch 7 场景）PASS |
| 2026-10-11 | **v0.7 落地（VFS + RAMFS + ELF Loader）**：`fs/` 从占位变为实现（`vfs` trait + 路径工具 + 系统级入口、`mount` 最长前缀路由、`inode` 元数据、`ramfs` 内存后端、`file` fd 表）；`read`/`write`/`open`/`close` 接入 `syscall_table`（fd 0/1/2 保留、动态 fd ≥ 3、用户缓冲按页校验）；新增 `syscall/elf.rs`：用户程序改为「RAMFS `/init` → ELF Loader 按段权限逐页映射」（`build.rs` 不再摊平，`user.ld` 四个段 `ALIGN(0x1000)`，`.lbss.*` 必须归入 `.bss`），`RW` 段对 ring3 可写；新增 `make test-fs`（19 项）与 `tools/check_user_elf.py`（`make user` 验段布局），`make test-user` 增至 **32 项断言**；修 B-15（syscall 入口未恢复 caller-saved 寄存器 → 用户态 `r9` 被破坏，#PF）；全量回归（9 个串口测试目标 + `go-test` + `test-orch` **8 场景**）PASS |
| 2026-10-11 | **v0.8 前半落地（驱动框架 + PCI + 块设备）**：新增 `driver/driver.rs`（`Driver` trait + 注册表 + `probe_all` + `ProbeStats`）、`driver/pci.rs`（配置机制 #1 + BAR 探测/窗口大小）、`driver/block.rs`（`BlockDevice` + 注册表 + 验收自测）、`driver/virtio_blk.rs`（legacy virtio 0.9.5：virtqueue/描述符链/轮询/bounce buffer）、`arch/x86_64/port.rs`（端口 I/O 32 位入口）、`frame::allocate_contiguous`（DMA/virtqueue 前置）；启动盘改为 **virtio-blk（`disable-modern=on`）**，`main.rs` 在 `Kernel started!` 后接入「枚举 → 注册 → 探测/挂接 → 块设备自测」；新增 `make test-driver`（22 项）与编排场景 `driver`（**9 场景**，127.2s 全 PASS），全量回归 PASS（含非 snapshot 启动后镜像 md5 不变，验证写测试真的还原了扇区）；修 B-16（BAR 大小 32→64 位宽错误 → I/O BAR 尺寸报错）、B-17（`lba + sectors` 环绕 → 越界请求漏拦）；同步 docs/drivers.md（v0.8 实现记录 + 设计取舍 + 限制） |
| 2026-10-11 | **v0.8 收官（磁盘文件系统 + `mkfs`）**：`fs/fat.rs`（FAT12/16/32 只读：BPB 按簇数定类型、固定根目录区/簇链目录、8.3 + NT 标志 + LFN 校验和、偏移读、`EROFS`/`EUCLEAN`）挂到 VFS（`fs::mount_disks` → `/disk` 启动盘、`/mnt/fat` 测试盘，验证最长前缀路由）；`FileSystem` trait 改为 `&mut self` + `read_all -> Vec<u8>`；新增 `vfs::is_mount_point` 并修「单盘启动误判 `/mnt/fat` 已挂 → 启动被 halt」；`tools/mkfs/mkfs.py`（H-03）自己生成 FAT12/16/32 镜像并由 mtools 独立复核，`tests/fatroot/` 提供测试盘内容与复核说明；`tests/test_boot.py` 新增 `--disk2`；新增 `make fat-image` / `make test-fat`（**21 项**，双盘启动）与编排场景 `fat`（**10 场景**，150.7s 全 PASS）；全量回归 PASS（`test-driver` 22 项 / `test-fs` 19 项 / `test-user` 32 项 / `make go-test`）；同步 filesystem.md（第 3 节由计划改写为实现记录）、drivers.md、roadmap/README/project-structure/tools README |
| 2026-10-11 | **H-06 P0a 落地：`.ckc` 包容器（CKC）** —— 内核包后缀定为 **`.ckc`**（Cerlesse Kernel Container），64 字节头 + 确定性 ustar payload（身份 = payload 的 SHA-256）；新增 `userland/pkgmgr/ckc.py`（纯标准库 `pack`/`verify`/`info`，含 `PKG.toml` 子集解析、scope 白名单、setuid/hooks/超长路径等打包期规则）与验收 `userland/pkgmgr/tests/test_ckc.py`（新增 `make test-pkg`，**37 项**，含系统 `tar` 独立复核）；docs/package-manager.md 3.2 由「`.cpk` 裸 tar 设想」改写为容器规格，11.1 拆成 P0a（已落地）/P0b（事务/catalog/receipt）；同步 docs/README.md 索引、tools/README.md、project-structure.md、README.md |
| 2026-10-11 | **v0.9 第四步：简化网络栈（virtio-net → Ethernet → ARP → IPv4 → UDP）** —— 新增 `driver/net.rs`（`NetworkDevice` + 注册表）、`driver/virtio_net.rs`（legacy virtio-net 0.9.5：RX/TX 双队列、10 字节 virtio-net 头、MAC 读自设备配置空间、同步轮询）、`network/{ethernet,arp,ipv4,udp,dhcp}.rs` 与 `network/mod.rs`（`Stack` + `selftest`）；`network/{tcp,socket}.rs` 占位说明更新；`driver::register_builtin` 追加 virtio-net、`main.rs` 在磁盘验收后接入网络自测（**无网卡时跳过**）；新增 `make test-network`（**13 项**，QEMU 加 legacy virtio-net-pci + `-netdev user` = slirp，主机侧不用跑任何服务）与编排场景 `network`（12 → **13 场景**）；`tests/test_boot.py` 增 `--nic`；全量回归 PASS（boot/ memory/ driver 22/ fs 20/ user 61/ shell 11/ proc 8+4/ scheduler/ ipc/ keyboard/ fat 21/ exception/ pagefault 均通过）；同步 docs/drivers.md（新增 1.8 节 + 驱动列表/限制/状态行）、docs/roadmap.md、README.md |
| 2026-10-11 | **v0.9 第五步：基础用户程序（`ls`/`cat`/`echo`/`ps`）与目录/进程枚举** —— 新增 `syscall/dir.rs`（`getdents`：VFS `list()` → 定长 88 字节记录；号 78 沿用 Linux 但格式不同；截断语义）与 `syscall/process.rs::sys_getprocs`（定长 40 字节记录；号 **256**）；`syscall_table.rs` 确立「`0..=255` Linux 语义 / `>= 256` Cerlesse 扩展」并改 `SYS_MAX` 256 → 257；`process/process.rs` 新增定长 `comm`（`execve` 改写为程序名）；`libc` 新增 `getdents`/`getprocs` + `dirent_*`/`procinfo_*` 解析器；新增 `user/{ls,cat,echo,ps}` → RAMFS `/bin/*`（`kernel/build.rs`/workspace/`fs::init` 清单 3 → 7），Shell 帮助文本同步；新增 `make test-programs`（**17 项 + 5 条正则**）与编排场景 `programs`（13 → **14 场景**）；`make test-proc` 的负例由 `ls` 改 `foo`；`make user` 改为构建/校验 7 个用户程序；全量回归 PASS（test / fs 20 / user 61 / shell 11 / proc 8+4 / programs 17+5 / memory / keyboard / scheduler / ipc / driver 22 / fat 21 / network 13 / exception / pagefault / `make test-pkg` 37 / `make go-test`） |
| 2026-10-11 | **v0.9 第三步：进程对象 + `fork`/`execve`/`wait4`** —— `process/process.rs` 换成真正的 PCB（地址空间 / 用户帧 / 每进程 64 KiB 内核栈 + 栈底哨兵 / `return_to` / `Ready\|Running\|Zombie`）；新增 `kernel/src/syscall/process.rs`（fork 复制地址空间与当前帧、execve 换映像并重建 argv 用户栈、wait4 延迟调度 + 回收）；`trampoline` 改为按进程切 CR3/`TSS.RSP0`/syscall 栈顶/当前进程下标； `libc` 新增 `args`（入口寄存器 ABI）并去掉 `fork`/`execve`/`wait4` 的 ENOSYS 说法；新增外部程序 `user/hello-bin` → RAMFS `/bin/hello`，Shell 的 `cmd_external` 改走 `fork`+`execve`+`wait4`；`tests/test_keyboard.py` 新增 `--expect-re`（正则 + 反向引用）；新增 `make test-proc`（8 项 + 4 正则）并加入 `test-all`，编排场景 11 → **12**（新增 `proc`）；`make test-shell` 12 → 11 项；记录两条踩坑（debug 构建下 16 KiB 内核栈静默溢出踩坏 `.bss` 帧分配器；子进程退出时必须先恢复父进程 CR3 再 Drop 地址空间）；同步 syscall/process/libc-README/project-structure/roadmap/README |
| 2026-10-11 | **v0.9 第二步：交互式 Shell 与 `stdin`** —— 键盘行缓冲（128 字节 + 行编辑，回显由 `KB <字符>` 改为终端式原样回显，`kbd: line ready len=N`）、`read(0)` 行缓冲语义（3 秒 `-EAGAIN`）、新增 `user/shell`（提示符 `root@cerlesse C:>`、内建 `help`/`echo`/`cat`/`pid`/`exit`、外部命令真调 `fork()`）、`kernel/build.rs` 改为多用户程序构建（`/init` + `/bin/sh`）、`syscall/mod.rs` 对已命名未实现的号打印 `syscall: ENOSYS nr=… name=…`；新增 `make test-shell`（12 项）与编排 `shell` 场景（11 场景）；`make test-user` 59 → **61 项**，`test-fs` 根目录 3 → 4；同步 syscall/drivers/process/project-structure/roadmap/README |
| 2026-10-11 | **v0.9 第一步：用户堆**（运行时前置条件）—— 新增 `kernel/src/syscall/memory.rs`（`mmap`/`munmap`/`brk`：W^X、`MAP_*`/`PROT_*` 白名单、5 参数无 offset 的取舍、逐条错误码）、`address_space.rs` 新增 brk 窗口（256 KiB）与匿名 mmap 窗口（256 KiB，first-fit，清页表项时 `invlpg`）、`libc/heap.rs`（`#[global_allocator]`，bump over mmap）与 `brk/mmap/munmap` 封装、`user/hello` 堆自测、`user.rs` 页帧泄漏检查；`make test-user` 断言 32 → **59 项**，`orchestrate.json` 的 `user` 场景同步；全量回归 PASS（`test-fs` 19 / `test-driver` 22 / `test-fat` 21 / `make go-test` / `make test-orch` 10 场景） |
| 2026-10-11 | **v0.9 第六步（收官）：内核侧最小动态链接** —— 新增 `kernel/src/syscall/dynamic.rs`（`ET_DYN` 分流：`PT_INTERP` + `DT_NEEDED` → `/lib/<name>` 递归装载，应用 `R_X86_64_RELATIVE`/`GLOB_DAT`/`JUMP_SLOT`/`64`，`DT_PLTREL=REL` 明确拒绝）；`syscall/elf.rs` 按 `e_type` 分流（`ET_EXEC` 路径不变）；`address_space.rs` 新增共享库窗口（基址 + 1 MiB、32 页）与 `map_dyn_page`/`dyn_page_count`/`phys_of`（内核 CR3 下写重定位），`fork`/`page_flags`/`Drop` 同步；新增夹具 `user/dynhello/`（`as`+`ld` 现编，`kernel/build.rs` 内嵌）与 RAMFS `/lib`（`fs::init` 新增 `libraries` 参数，程序清单 7 → 8）；新增 `make test-dyn`（**10 项**）与 `make dyn-hostcheck`（宿主机 glibc 独立复核）并加入 `test-all`；`test-fs` 20 → **21 项**、`test-programs` 条目数断言 5/7；同步 syscall.md（新增 3.9 + 3.1/3.4/3.8 修订）、architecture.md 第 5 节、roadmap/README/project-structure/api |
| 2026-10-11 | **H-06 包管理器提案 v1 完成（设计阶段，不进内核主线）**：docs/package-manager.md 从「设想」补成可实施规格 —— 术语字段级定义、七个制品格式（`PKG.toml`/确定性 `.cpk`/`catalog.toml(+sig)`/`cpkg.toml`/receipt/journal/内容寻址 store）、内容哈希 + 三级校验（签名 → 绑定 → 落盘后逐文件）、安全模型（显式信任根、scope 域边界与路径白名单、**纯文件安装/无安装脚本**、setuid 拒绝、威胁—对策表）、事务阶段与原子性边界（文件级 + 可恢复）、效率预算（幂等零写入、文件级去重、`gc`）、CLI 与稳定退出码、MVP 三期（P0 主机侧 `userland/pkgmgr/` → P1 与镜像构建/基线对接 → P2 增量补丁等）、可脚本断言的验收条目（`make test-pkg`）；同步 docs/README.md 索引状态（设计中 → 提案 v1）与 H-06 行 |
| 2026-10-11 | **纯内核化：阶段二/四/五（部分）执行完成** —— 新增 `tools/check-boundary.py`（内核不得调用外部命令/内嵌用户态字节/依赖用户态包，6 项 + `--selftest` 反向自检）与 `make check-boundary`，并入 `make test-all`；`libc/`(7 文件) + `user/`(33 文件) 整体移入 `userland/` 独立 workspace（自带 `Cargo.toml`/`.cargo/config.toml`/`Makefile`，产出 `build/userland/images/{manifest.txt,*.elf}`；动态链接夹具与 `dyn-hostcheck` 随之搬走）；`kernel/build.rs` 重写为「读清单 → 复制 → 生成 include 源码」（删掉约 300 行嵌套 cargo/as/ld，**内核不再需要 binutils**），`syscall/user.rs` 的 10 处 `include_bytes!` 收敛为一次 `include!` + 清单契约校验 + 空清单诊断分支，`fs::selftest` 的根目录条目数改为「`4 + /init 是否存在`」推导；根 `Cargo.toml` 收敛为 `shared/boot/kernel` + `exclude userland`；新增 `userland-images`/`image-kernel`/`kernel-nouserland`/`test-kernel-only`。实测：无 `userland/` 的 checkout 里 `cargo build -p kernel` 退出码 **0**（改造前 **101**）、`make image` + `make test-kernel-only` 7/7 PASS（验收标准 1/2/7 成立）；全新 checkout `make image` 正确内嵌 8 程序 + 2 库；15 个编排场景与全部用户态测试 PASS；执行中发现并修掉「`image` 可能先编内核再出用户态镜像」的顺序缺陷 |
| 2026-10-11 | **新增「纯内核化改造规划」（v0.9 之后，规划）**：新建 docs/pure-kernel.md —— 现状审计 C1–C9（内核构建期嵌套 cargo 构建用户程序、10 处 `include_bytes!` 把 203,280 字节（≈199 KiB）用户态内嵌进内核映像、`fs::init` 硬编码 `/bin`/`/lib`、启动末尾跑用户态自测、ABI 两处镜像、workspace/测试责任混杂、`.cargo/config.toml` 不分家）、实测「删掉 `libc/`+`user/` 后 `cargo build -p kernel` 退出码 **101**」、逐文件迁移清单（内核 83 文件仅 9 个要改；`user/` 33 + `libc/` 7 + `tools`/`tools-go`/`tests` 逐项去向）、目标双仓库结构与 Cargo 配置前后对照、六阶段（冻结基线/边界门禁/ABI 单一来源/用户空间迁移/构建测试拆分/接口稳定）与七条验收标准对照表；本文档新增 5.11 与第 3 节注记，同步 roadmap/project-structure/docs-README/README |
| 2026-10-11 | **README 重写（对外首页，按 readme-generator 创作规范）**：README.md 从 466 行的开发日志改为 157 行的作品级首页 —— 首屏（H1 + 一句定位 + 封面 + 2 个徽章）、这是什么 / 为什么需要它 / 快速开始 / 安装 / 你会得到什么（真实串口片段）/ 工作方式 / 常用命令 / 项目结构 / 文档入口 / 当前状态 / 关于作者 / 许可证；新增 `assets/banner.svg`（1920×1080 深黑 + 暖金名片图，无外部字体与脚本）；逐版本实现记录不再在本文件复述，改由 DEVELOPMENT.md 第 5 节、docs/roadmap.md 与 git 历史承载；同时去掉各测试的「项数」（Makefile 实际断言数与部分文档记载不一致，见 README「常用命令」改为只列命令与断言内容） |
| 2026-10-11 | **新增「桌面环境兼容路线」（v1.0 之后，规划）**：新建 docs/desktop.md —— 现状审计（逐项有据：显示无/输入无 evdev/无 ioctl-poll/无设备模型与 `/proc`/无音频/无 ACPI/security 占位/fs 只读）、用户态与内核侧归属纠正（logind/D-Bus/PipeWire 是用户态；KVM 与桌面前置无关）、被遗漏的地基（设备模型/`ioctl`/`poll`/file-backed mmap/futex/信号/AF_UNIX/`/proc`+`/sys`/memfd）、D0–D5 分层与 QEMU 可断言验收（monitor `sendkey`/`screendump` 实测可用）、里程碑 v1.1–v1.6；本文档新增 5.10 与 1.3 注记，同步 roadmap/architecture/project-structure/docs-README/README |
| 2026-10-11 | **v1.0 第一步：`mprotect`**（同时补齐仓库缺失的 `LICENSE`，MIT）—— `syscall/memory.rs` 新增 `sys_mprotect`（页对齐/`len>0`/未知位/W^X 四类拒绝各留一行串口原因），`address_space.rs` 新增 `mprotect` + `UserWindow` + `prot_to_flags` + `accessible`（`PROT_NONE` = 清 `USER` 留 `PRESENT`，每次改权限 `invlpg`）、堆窗口逐页 `heap_flags`（含 `fork`/`brk` 同步）、`mmap(PROT_NONE)` 由拒绝改为合法；`syscall_table` 注册 10 号并把 `UNIMPLEMENTED` 清空；`libc` 新增 `mprotect`/`PROT_NONE`（含 `lib.rs` re-export），`userland/user/hello` 新增 `mprotect_selftest`（`PROT_NONE` → `-EFAULT` → 内容原样 → `RX` 后真的执行机器码；另覆盖 brk 堆窗口）；`make test-user` 61 → **91 项** PASS；`kernel/build.rs` 在无清单分支登记默认 `manifest.txt` 为构建输入（修掉「先编内核后出镜像 → 一直无用户态」的缓存陷阱）；同步 docs/syscall.md（新增 3.10）、docs/api.md（2.6 + 节号顺移 + `test-user` 84）、docs/README.md、docs/roadmap.md、userland/libc/README.md、Makefile 与本文件 §5.7 |
| 2026-10-11 | **新增「CKM 设计提案」（H-07，v1.0 之后，规划）** —— 新建 docs/ckm.md：定稿**权限模型**（单管理员模型 + 4 个能力位 + 7 个判定点；凭证只能由 CKM 组件表在 `execve` 时改变，因此没有 `su`/`setuid` 路径）、**内核接口**（`kernel/src/ckm/` 五模块分工、`CKM_ABI_VERSION = 1`、扩展号段 257–259 与 `SYS_MAX` 257 → 260、审计 48 字节 / 组件状态 64 字节定长记录、串口证据行格式与事件枚举）、**扩展生命周期**（四类扩展、八态状态机 + 合法转换表、确定性枚举与开机加载顺序）；另含与 H-06 的分工（`.ckc` 不重做、P0b = CKM-3、`cpm` 合并进 `ckm pkg`）、覆盖层与 KernelSU 元模块的对应关系（第一版内置、可插拔为后续）、事务/回滚与恢复环境（安全模式四件事）、CKM-1…CKM-7 分期、8 条故障场景落成断言、非目标与 7 个开放问题；本文档新增 H-07 需求行与 5.12，同步 docs/README、roadmap（第十一阶段）、architecture 第 5 节、project-structure（规划目录）、README（文档入口） |
| 2026-10-11 | **新增「CSS 存储管理系统提案」（H-08，v1.0 之后，规划）** —— 新建 docs/storage.md：定稿**卷模型**（设备→分区→卷→挂载点/盘符四层；`Volume` 记录；`vol_id` 三条确定性生成规则：文件系统 UUID/序列号 > `pt:<disk>-<part>-<lba>` > `raw:<disk>`；只读原因 5 值、健康状态 4 值；卷表 16 槽 / 每卷 ≤2 挂载点）、**路径与盘符**（`C:\x` ≡ `/volumes/x` 同一文件；Windows 式路径由用户态 libc 查卷表后重写，**内核 syscall 只收 POSIX 路径**；默认卷 `/volumes/<label>`、可移动卷 `/media/<label>`；错误语义追加 `ENODEV`/`ENOTSUP`，并修掉现状「挂载失败返回字符串」）、**VFS 收口与 CKM 接口**（`FileSystem` 只加 `caps`/`volume_info`/`probe` 三个只读方法、挂载表带 `vol_id`/`options`、`kernel/src/storage/` 六模块 + `syscall/storage.rs`；CKM 侧能力位扩到 6 位、判定点 8–10、事件 12–16、号段 **261–266**（`SYS_MAX` → **267**）、卷 64 字节/分区 48 字节定长记录、`/storage/**` 只读状态树与串口证据格式）；另含文件系统支持路线（FAT→exFAT→ext2 优先；ext4/NTFS 只读优先）、第三方依赖政策（现状零依赖且被 `check-boundary.py` K4 强制）、写入语义五条、CSS-1…CSS-6 分期、11 条可脚本验收（含宿主工具实测：有 `blkid`/`mtools`/`mkfs.ext2`，**没有** `sfdisk`/`parted`/`mkfs.exfat`）、非目标与 9 个开放问题；本文档新增 H-08 需求行与 5.13，同步 ckm.md（能力位/判定点/事件/号段/CKM-2 硬约束）、docs/README（索引 + 第十二阶段 + 修订）、roadmap（第十二阶段）、architecture 第 5 节、project-structure（规划目录）、README（文档入口）、tools/README.md 与 filesystem.md |
| 2026-10-11 | **CSS 提案边界补充（docs/storage.md）**：新增 **6.8 文件系统后端装载政策**（第一版全部编译内置、不经 CKM 运行时装卸、可独立更新驱动不在第一版承诺内）；6.1 增「覆盖层/回滚/恢复归 CKM」行；6.7 明确「兼容性 = `fs=`+`ro=`+`health=` 完整展示、不得压成布尔」；第 11 节增「不做写回修复」（`fsck` 只读检查），第 9 节 CSS-5 同步标注 |
| 2026-10-11 | **CSS-1 落地（底层识别）** —— 新建 `kernel/src/storage/{mod,registry,partition,gpt,mbr,volume,selftest}.rs`：GPT（主/备表头 + 两份 CRC32 + 128 项表项）与 MBR（4 主分区 + EBR 链）**只读**扫描，**无分区表退化为整盘一卷**，FAT/exFAT/ext 探测；`driver/block.rs` 注册表 2 → 8；`syscall/storage.rs` 新增 `storage_volumes`(261)/`storage_rescan`(265)（64 字节定长卷记录、只读、确定性枚举，`storage_volumes` 无能力位）；`tools/mkfs/mkfs.py` 增 GPT/MBR/主表损坏/无表四类夹具；`libc` 增 `storage_volumes` 解析器，新增用户程序 `/bin/volumes`（`userland`）；新增 `make storage-images`/`test-storage-fixtures`/`test-storage`/`test-storage-mbr` 与编排场景 `storage`/`storage-mbr`（15 → **17 场景**）；内核解析自测 11 例 / 45 断言（主表头损坏回退备用表头、CRC 失败 → `degraded`）；同步 docs/storage.md（第 2/9/10 节）、docs/README.md、docs/roadmap.md 与本文件 §5.13 |
| 2026-10-11 | **CSS-2 落地（VFS 收口）** —— `fs::mount::MountTable` 拆 `backends`/`mounts`（一个后端可被多个挂载点引用，新增 `mount_volume`/`alias_volume`/`umount`/`route`/`sync_counts`），挂载点带 `vol_id`/`options`，挂载/卸载把挂载计数写回卷记录偏移 56；`FileSystem` 增 `caps`/`volume_info`（保守缺省；RAMFS 可写、FAT 只读+大小写不敏感+卷标）；FAT 探测收归 `fs/fat.rs`（`probe`/`probe_sector`/`bp_identity`，删掉 `storage/volume.rs` 里重复的 `probe_fat`），`FatFs::open` 改返回 `OpenError { errno, reason }`；`fs::mount_disks` 按卷表挂载并保留 `/disk`、`/mnt/fat` 兼容点；`fs: stat` 行尾带 `vol=`；新增 `fs::storage_mount_selftest` 开机自测（只读原因来自能力位、后端与记录一致、同一卷多挂载点同字节、卸载后 `ENOENT` 且卷仍在表、`ENOENT`/`EINVAL`/`EEXIST` 反例）与 `storage: mount`/`volcheck`/`unmount` 证据行；`tests/test_boot.py` 增 `--expect-re`（反向引用绑同一 `vol_id`，因为启动盘卷序列号随镜像构建变化）；`make test-storage`（31 项 + 2 正则）、`make test-storage-mbr`（14 项 + 1 正则）、`make test-programs` 的 `mounts=0` → `mounts=1`（记录真的反映挂载表）；不回归：`test-fs`/`test-fat`/`test-user`(91)/`test-driver`(22)/`test-kernel-only`(7)/`check-boundary`/`go-test` 与 17 个编排场景；同步本文件 §5.13/H-08 行、docs/storage.md（2/5/9/10/13 节）、docs/filesystem.md、docs/roadmap.md、docs/README.md、docs/api.md、docs/project-structure.md 与 README |
| 2026-10-12 | **纯内核化阶段五落地（构建与测试按责任拆分）** —— 归属**用户态**的主机工具全部迁出内核仓库：`tools/pkgmgr/{ckc.py,cpm.py}` + `tests/test_{ckc,cpm}.py` → `userland/pkgmgr/`（新增 `README.md`），`tools/check_user_elf.py` → `userland/tools/`；`tests/orchestrate.json` 用 `tags` 分组（`user`/`ckm`/`shell`/`proc`/`programs`/`dyn` 共 6 个用户态集成场景标 `userland`，其余 13 个为内核场景），`testorch` 新增 `-tag`/`-exclude-tag`（加载期词表校验 + `main_test.go`）；内核 `make test-all` 收敛为**纯内核 18 个目标**（实测全 PASS），新增 `test-all-with-userland`/`test-orch-userland`，`make test-pkg` 改为跨仓库转发（`CERLESSE_USERLAND_DIR` 缺失时明确报错）；用户态侧新增 `make -C userland test-pkg`/`test-all`（实测退出码 0，不需要内核源码）；详见 docs/pure-kernel.md 8.3 与 [userland/pkgmgr/README.md](userland/pkgmgr/README.md) |
| 2026-10-12 | **CKM-3 落地（事务引擎，主机侧 = H-06 P0b）** —— 新增 `userland/pkgmgr/cpm.py`（纯标准库：目录骨架 + 回执（当前 + `.prev.toml`）+ 事务日志 `txn/<utc>-<pid>.jsonl` + stage/verify/activate/commit + 逆序回滚 + `recover` + `gc` + 单写者锁；**备份与 `activate` 记录都在写目标之前落日志**，因此「写到一半掉电」可恢复；`--fail-after`/`--crash-after` 两个失败注入开关）；`userland/pkgmgr/ckc.py` 拆出 `manifest_from_data`，使**安装期重跑域边界/scope 路径白名单**（`make test-pkg` 既有 `test-ckc` 37 项不变）；新增 `userland/pkgmgr/tests/test_cpm.py` 与 `make test-cpm`（**63 项**，含系统 `tar` 独立复核、逐字节回滚比对、手工拼的越界包零写入、并发锁、去重、`apply` 幂等），`make test-pkg` 改为 `test-ckc + test-cpm`；同步 docs/package-manager.md（11.1 由「待做」改为落地清单 + 12.1 勾选 + 14）、docs/ckm.md（9.3 + 状态 + 第 10 节覆盖情况 + 13）、docs/README.md、tools/README.md、本文件 H-06/H-07 行与 CKM 分期清单的 CKM-3 勾选 |
| 2026-10-14 | **v1.0 第二步：驱动兼容性（AHCI/SATA + PCI 枚举修缺陷 B-18）** —— `driver/pci.rs` 的头部类型偏移由 `0x0C`（cache line size，恒为 0）改为 `0x0E`：`MULTI_FUNCTION` 判据以前永远为假，**多功能设备的 function 1..7 从未被枚举**（q35 上 `pci: enumerated devices=5` → **7**，芯片组的 SATA `00:1f.2` / SMBus `00:1f.3` 才可见；固件引导日志里一直有 `Pci(0x1F,0x2)/Sata(...)`，两边不一致就是缺陷本身）；新增 `driver/ahci.rs`（AHCI 1.3：命令列表 / 接收 FIS / 命令表 + PRDT、IDENTIFY DEVICE 取型号与容量、LBA48 READ/WRITE DMA EXT、`PxCI` 同步轮询 + 4 KiB bounce buffer、**按类别匹配**不看厂商、ATAPI 按 `PxSIG` 跳过、没插盘时 `attach` 照样成功并打印 `drives=0`）并注册为 `index=2`；`pci.rs` 新增命令寄存器读写（内存空间 + 总线主控），MMIO BAR 第一次被真正用起来；`main.rs` 有 SATA 盘时对 `ahci0` 再跑一次**同一个** `block::selftest`（上层 FAT/VFS/CSS 零改动 —— 第二条块设备路径与 virtio-blk 共用一个 trait）；`tests/test_boot.py` 新增 `--ahci`/`--ahci-disk`（把盘插到 ICH9 的 `ide.2`）；新增 `make test-ahci`（**24 项**）并并入 `test-all`（18 → **19 目标**），编排场景 19 → **20**（新增 `ahci`；`driver` 场景的 `matched/attached` 由 1 改 2）。实测：`make test-ahci` 退出码 0（24/24）、`make test-driver` 23 项 PASS、`testorch driver,ahci` 两场景 PASS（30.1s）；同步 docs/drivers.md（新增 1.9 节 + 2.3 路线 + 驱动列表/限制/证据）、docs/api.md、docs/roadmap.md、docs/README.md、docs/project-structure.md、README.md |
| 2026-10-12 | **CSS-3 落地（盘符部分）** —— 新增 `kernel/src/storage/letters.rs`（固定容量盘符表：缺省 `C` 给系统卷、其余按 6.5 顺序给 `D`/`E`/…，**自定义**映射持久化到 CKM 状态区键 `storage/letters`，`letter = 0` 取消回到规则；非法 `EINVAL`/占用 `EEXIST`/超容量 `ENOSPC`/未知卷 `ENOENT`）；`storage/registry.rs` 增 `letter_of`/`slot_of`/`assign_letter`/`reassign`（盘符写进卷记录偏移 24）；`syscall/storage.rs` 把 **262 `storage_mount`**（已挂载卷只加别名、flags bit0 只读）/ **263 `storage_unmount`**（卸别名、卸 `/` → `EINVAL`）/ **264 `storage_assign_letter`** 从 ENOSYS 桩上拿下（均走 CKM 判定点 8 `CAP_STORAGE_ADMIN`，参数校验先于能力判定）；`fs::mount_volume_by_id`/`fs::unmount_point`（`fs/mod.rs`）；`userland/libc/src/path.rs` 新增 `is_windows`/`resolve`/`resolve_str`（查 `storage_volumes` 把 `C:\x` 重写为 POSIX）+ 三个 syscall 包装；`userland/user/volumes` 扩为 `list`/`mount`/`unmount`/`letter`/`demo`；内核自测 `letters::selftest`（默认规则/冲突拒绝/持久化往返/取消回退）；`make test-storage` 增 `letter=` 与盘符断言（`letters selftest checks=36`）、`test-storage-mbr`（`checks=22`）、`test-programs` 增 `volumes demo`、`test-ckm` 增零能力拒绝；同步本文件 H-08 行与 §5.13、docs/storage.md（2/4/6/9/10/13 节）、docs/ckm.md、docs/syscall.md（新增 3.12）、docs/api.md、docs/project-structure.md |


