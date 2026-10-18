# 纯内核化改造规划（Cerlesse Kernel / Cerlesse Userland）

- 阶段：第一阶段 / 边界规划（v0.9 之后、桌面路线之前执行）
- 最后更新：2026-10-12
- 状态：🟡 执行中（2026-10-12）——**阶段二 ✅、阶段三 ✅、阶段四 ✅、阶段五 🟡**；
  阶段一（基线 tag）、阶段六（接口冻结 / 移植文档）未做。执行记录与实测证据见第 8 节；
  本文档同时是**逐文件迁移清单**与**六阶段验收规格**

> 本文档顶部保留 阶段 / 最后更新 / 状态 字段，与项目文档维护约定对齐（见 [README.md](README.md)）。
> 关联：[architecture.md](architecture.md)（第 5 节非目标）、[ckm.md](ckm.md)（CKM 规划：`ckmd`/`ckm` 属用户态，
> 必须落在 `userland/`，内核只保留 CKM Core 机制）、[project-structure.md](project-structure.md)（目录与职责）、
> [roadmap.md](roadmap.md)（阶段划分）、[syscall.md](syscall.md)（ABI 规格）、[package-manager.md](package-manager.md)（`.ckc`）、
> [../DEVELOPMENT.md](../DEVELOPMENT.md)（第 3 节 Cargo 规划、第 5 节任务清单）。

## 0. 目标与边界

把 Cereals-Kernel-Cerlesse 收敛成 **Cerlesse Kernel**：使用 Rust 编写的 x86_64 独立宏内核，
提供**可被外部操作系统用户空间使用**的底层接口；Shell / init / libc / 基础命令 / 包管理器 / 应用
迁到独立的 **Cerlesse Userland** 项目。原则：**内核提供机制，外部系统提供功能。**

```text
外部用户空间（不属于内核）        Cerlesse Kernel（本仓库）            硬件 / 虚拟硬件
Shell · init · libc ·            系统调用 ABI · 进程/线程 · 调度 ·     x86_64 · UEFI · QEMU
命令(coreutils) · cpm            内存 · VFS/内核 FS · 网络栈 · 驱动 ·
                                 中断 · 同步 · IPC · 安全隔离
```

| 层 | 归属 | 内容 | 本仓库现状 |
| ---- | ---- | ---- | ---- |
| 内核 | **本仓库** | 系统调用 ABI、进程/线程、调度、内存、VFS 与内核文件系统、网络协议栈、驱动框架、中断、同步、IPC、安全隔离 | `kernel/`（83 文件：78 个 `.rs` + `link.ld` + `Cargo.toml` + 3 个 `interrupt/*.md`） |
| 引导 | **本仓库**（独立组件） | UEFI 引导、内存检测、装载内核、移交控制权 | `boot/`（独立 crate，`x86_64-unknown-uefi`） |
| 共享 ABI | **本仓库**（严格受限） | boot↔kernel 结构、系统调用 ABI 常量与记录布局 | `shared/`（22 行，仅 `BootInfo`）；**ABI 常量目前尚无归属**（见 C6） |
| 构建工具 | **本仓库**（不进内核运行时） | 镜像制作、内核测试编排、串口日志分析、FAT 镜像生成 | `tools/`（Python）、`tools-go/`（Go）、`tests/`（内核场景） |
| 用户态 | **Cerlesse-Userland** | Shell、init、libc、coreutils、cpm、示例、用户态 ABI 测试 | `libc/` + `user/*`（当前在本仓库 workspace 内，见 C1/C5） |

「纯内核」**不要求启动后什么都跑不起来**：要求内核**不负责提供完整用户空间**，
并在用户态缺失时仍能启动到**可诊断状态**（串口调试 / 内核自测模式）——这是验收标准 7。

## 1. 现状审计

### 1.1 已符合边界的部分

| 项 | 证据 |
| ---- | ---- |
| 内核不含 Python/Go/脚本 | `kernel/` 83 个文件 = 78 个 `.rs` + `link.ld` + `Cargo.toml` + `src/interrupt/` 下 3 个 `.md` 说明；Python/Go 只在 `tools/`、`tools-go/` |
| 引导器是独立 crate 与独立目标 | `boot/Cargo.toml`（`x86_64-unknown-uefi`），只依赖 `shared` |
| 共享面很窄 | `shared/src/lib.rs` 22 行，仅 `BOOT_MAGIC` + `BootInfo`，`no_std` |
| 内核没有「从 user/* 引入的 Rust 依赖」 | `kernel/Cargo.toml` 的 `[dependencies]` 只有 `shared = { path = "../shared" }` |
| 文件系统/网络/驱动是内核服务 | `fs/fat.rs`、`network/{ethernet,arp,ipv4,udp,dhcp}.rs`、`driver/virtio_{blk,net}.rs` 都在内核侧（Linux 式边界） |
| 多数内核测试不依赖用户程序 | `make test-memory` / `test-scheduler` / `test-ipc` / `test-driver` / `test-fat` / `test-network` 的断言只看内核自身行为（`test-network` 无网卡时跳过）；**例外**：`make test-fs` 断言 `fs: create /init`、`fs: ls / entries=5`、`/bin`、`/lib` 的落盘，因此依赖 C3（阶段四必须改写，见第 6 节风险表） |

### 1.2 违反边界的耦合（C1–C9，全部可复现）

**执行状态（2026-10-12）**：C1 ✅ · C2 ✅ · C3 ✅ · C4 ✅ · C5 ✅ · C6 ✅（阶段三落地）·
C7 🟡 部分 · C8 ✅ · C9 🟡 部分。下面每一条描述的是**改造前**的实测事实（保留为对照），
证据与命令见第 8 节。

**C1 内核构建期用嵌套 cargo 构建用户程序 —— 内核离不开用户态源码。**
`kernel/build.rs` 的 `USER_PROGRAMS`（7 个 Rust 用户程序）与 `build_dynamic_fixtures()`（3 个 as/ld 夹具）
在**内核构建时**执行 `cargo build -p user-*`，并用 `rerun-if-changed` 监视 `libc/`、`user/*`（共 27 条）。

实测（在 `/tmp` 的仓库副本里删掉 `libc/`、`user/` 并同步 workspace `members`，模拟「用户态不在本仓库」）：

```text
$ cargo build -p kernel --target x86_64-unknown-none --offline
error: failed to run custom build command for `kernel v0.1.0 (/tmp/pk/kernel)`
  --- stderr
  error: package ID specification `user-hello` did not match any packages
  thread 'main' panicked at kernel/build.rs:301:5:
  用户程序构建失败（user-hello，profile user-release）
EXIT=101
```

即 **验收标准 1 当前不成立**：没有 `libc/`、`user/` 就编不出内核。

**C2 用户程序二进制被内嵌进内核映像。** `kernel/src/syscall/user.rs:50-74` 有 **10 处** `include_bytes!`：
7 个 Rust 程序（`user_hello.elf` … `user_ps.elf`）+ 3 个动态夹具（`dyn/dynhello`、`dyn/libgreet.so`、`dyn/ld.so`）。
实测体积：内核 ELF `3,844,896` 字节内嵌用户态合计 **203,280 字节（≈199 KiB）**
（`user_hello`/`user_shell` 各 29,096；`ls`/`cat`/`echo`/`ps`/`hello-bin` 各 20,904；
`dynhello`/`ld.so`/`libgreet.so` 14,024 / 13,176 / 13,368）。

**C3 内核对用户空间布局有硬编码知识。** `kernel/src/fs/mod.rs:69` 的
`fs::init(programs, libraries)` 建 `/bin`、`/lib`、`/etc/motd`、`/tmp`，
并把用户程序 ELF 当普通文件写进 RAMFS；清单来自 `syscall::user::program_images()`。

**C4 内核启动流程里跑用户态验收。** `kernel/src/main.rs` 末尾调用 `syscall::user::selftest()`
（跑 `/init`、隔离探针、交互式 `/bin/sh`、动态链接闭环）——「用户态能不能用」被当成内核启动的一部分。

**C5 workspace 把内核与用户态绑在一起。** 根 `Cargo.toml` 的 `members` 同时包含 `kernel` 与
`libc`、`user/*`（共 11 个包）；`[profile.user-release]` 存在的**唯一理由**是内核要用它构建用户程序
（该文件注释原文）。

**C6 ABI 没有单一事实源（两处镜像）。**

| ABI 项 | 内核侧 | 用户态侧 |
| ---- | ---- | ---- |
| 调用号 | `kernel/src/syscall/syscall_table.rs`：`0/1/2/3/9/10/11/12/35/39/57/59/60/61/78/256`、`SYS_MAX = 257` | `libc/src/syscall.rs:45-58,283` |
| errno | `syscall/mod.rs:36`（`-38`）、`file.rs:43` 与 `memory.rs:35`（`-14`）、`dir.rs:49`（`-22`） | `libc/src/syscall.rs:79,94,98` |
| 记录布局 | `dir.rs:42` `DIRENT_SIZE = 88`、`process.rs:49` `PROCINFO_SIZE = 40` | `libc::{DIRENT_SIZE, PROCINFO_SIZE}`（注释写「必须一致」） |

**已消除（2026-10-12，阶段三）**：三类事实全部移入 `abi/` crate —— 内核侧与 libc 侧都只引用它，
不再各写一份。防漂移由两道防线保证：`make abi-check`（值固化：号唯一、号段不重叠、记录尺寸、
能力位/op 编号逐条断言）与 `abi/src/record.rs`（编译期断言：任一侧把数字改坏，**构建**即失败）。
上表保留为改造前的对照。CKM/CSS 落地后多出来的第三批 ABI（能力位、op 编号、CKM/存储记录尺寸、
受保护前缀）也一并在本轮进了 `abi`（规划时它们还不存在，故 3.1 只列了 4 个文件）。

**C7 测试责任混杂（同一份编排里两类场景）。** `tests/orchestrate.json` 15 个场景中，
`user` / `shell` / `proc` / `programs` / `dyn` 共 **5 个是用户态集成**；
`tests/test_ckc.py`（307 行，**阶段五已迁至 `userland/pkgmgr/tests/`**）**完全与内核无关**（`.ckc` 打包容器格式）；
`tests/fatroot/data` 是内核 FAT 测试盘内容（属于内核）。

**C8 构建配置不分家。** `.cargo/config.toml` 的 `relocation-model=static` 同时作用于内核与用户程序
（两者都用 `x86_64-unknown-none`）；`Makefile` 的 `USER_RUSTFLAGS` / `USER_PROFILE` 与
`kernel/build.rs` 的 `USER_*` 常量「成对维护，改一处要同步另一处」。

**C9 死物残留。** `user/init/src/main.rs` 只有 2 行占位、**不在 workspace `members` 里**
（无 `Cargo.toml` / `build.rs` / `user.ld`）；工作区里 `tools/dynexec/__pycache__/dynelf.cpython-310.pyc`
的源文件已不存在（且被 `.gitignore` 忽略，未入库）。

### 1.3 结论

- 边界违反集中在 **4 处代码**（C1–C4）、**2 处配置**（C5、C8）、**1 处 ABI 事实源**（C6）、**1 处测试划分**（C7）。
- 内核的**能力主体**（syscall / 进程 / 调度 / 内存 / VFS / FAT / 驱动 / 网络）已经是内核服务，**不需要搬动**；
  要搬的是 `libc/` + `user/*`（用户态产品功能）及其构建、测试、校验链路。
- 因此这是**边界收敛**，不是重写：`kernel/` 下 83 个文件中规划只有 **9 个需要改动**（见 3.1），
  其余文件连同 `fs/fat.rs`、`network/*`、`driver/*` 一个字都不用动。
- **实际执行下来更小**：阶段四只改了 3 个内核文件（`kernel/build.rs` 重写、
  `kernel/src/syscall/user.rs` 改镜像来源、`kernel/src/fs/mod.rs` 修一条写死的断言），
  加上根 `Cargo.toml` 与 `Makefile`；`kernel/src/main.rs` **不需要改**。

## 2. 目标仓库结构

```text
Cerlesse-Kernel-Cerlesse/          # 本仓库（内核 + 引导 + 构建工具 + 内核测试）
├── boot/                          # 引导器（独立组件，x86_64-unknown-uefi）
├── shared/                        # boot↔kernel 纯数据结构（BootInfo）
├── abi/                           # 系统调用 ABI 单一事实源（no_std、零依赖；✅ 阶段三）
├── kernel/
│   ├── src/{arch,memory,interrupt,time,process,scheduler,syscall,fs,driver,network,ipc,security}（沿用现有模块划分）
│   ├── link.ld
│   ├── build.rs                   # 只注入 link.ld（不再构建用户程序）
│   └── Cargo.toml                 # deps: shared + abi
├── tools/                         # image-builder / mkfs / check-boundary.py【新增】
├── tools-go/                      # testorch / serialmon（内核场景编排 + 串口分析）
├── tests/                         # 内核场景（orchestrate.json 中 tags 不含 userland 的部分 + fatroot/）
├── userland/                      # 【新增】用户态独立 workspace（阶段五后可拆到独立仓库）
├── docs/
├── Makefile
└── Cargo.toml                     # workspace: shared, boot, abi, kernel + exclude userland
```

> **执行上的一个变化**：用户态先以 `userland/` 子目录形式拆成**独立 workspace**
> （本轮已落地，见第 8 节），而不是立刻开新仓库 —— 这样二进制拆分可验证，
> 而 git 历史不碎；真正搬到 `Cerlesse-Userland` 仓库时用 `git subtree split -P userland`
> 即可（阶段五剩余项）。

```text
Cerlesse-Userland/                 # 新仓库（用户态 + 用户态测试）
├── libc/                          # C 运行时 / Rust 用户态运行时
├── init/                          # 第一个用户态进程（现 user/hello）
├── shell/                         # 命令解释器（现 user/shell）
├── coreutils/                     # ls / cat / echo / ps（现 user/{ls,cat,echo,ps}）
├── pkgmgr/                        # 包管理器（阶段五已落位：现 userland/pkgmgr/ckc.py + cpm.py）
├── examples/                      # 示例程序（现 user/hello-bin）
├── tests/                         # 用户态 ABI 测试 + 动态链接夹具（现 user/dynhello）
├── .cargo/config.toml             # code-model=large + relocation-model=static
├── Makefile
└── Cargo.toml                     # workspace: libc, init, shell, coreutils/*, examples/*, abi(路径依赖或 crates.io)
```

> `abi/` 先作为**本仓库内的 workspace 成员**（文档 + 常量 + 记录布局），暂不单独发布；
> 等接口稳定（阶段六）再决定是否抽成独立可发布的 crate。userland 侧通过
> `abi = { path = "../Cerlesse-Kernel-Cerlesse/abi" }`（开发期）或版本号依赖。

## 3. 逐文件迁移清单

列含义：**处置** = 保留 / 迁移 / 新增 / 删除；**阶段** 见第 4 节（一–六）。

### 3.1 `kernel/`（83 文件；阶段四实际改了 3 个，阶段三实际改了 18 个）

**执行状态**：`kernel/build.rs` ✅（重写为「读清单 → 复制镜像 → 生成 include 源码」，
删除约 300 行嵌套 cargo / as / ld 逻辑）、`kernel/src/syscall/user.rs` ✅（10 处 `include_bytes!`
→ 一次 `include!` + 清单契约校验 + 空清单诊断分支）、`kernel/src/fs/mod.rs` ✅（根目录条目数
由「`/init` 是否存在」**推导**，不再写死 5）。`kernel/src/main.rs` **未改**（空清单分支在
`selftest()` 里，`main.rs` 无需知道）。`kernel/Cargo.toml` 与 `syscall/{syscall_table,dir,process,mod}.rs`
属阶段三（`abi` 单一来源）——**已于 2026-10-12 完成，且实际改了 18 个内核文件**（CKM/CSS 的
常量也各有两份），见 8.2。

> 规划时写的 `embed-userland` **Cargo feature 没有采用**：实际用「环境变量 +
> 清单文件」表达「内嵌/不内嵌」，好处是 `cargo build -p kernel` 本身可用（不需要记住
> feature 组合），且清单不存在时自然退化为纯内核构建。

| 文件 | 现状 | 改造 | 阶段 |
| ---- | ---- | ---- | ---- |
| `kernel/Cargo.toml` | deps 只有 `shared` | 增加 `abi`（✅ 阶段三） | 三 |
| `kernel/build.rs` | 432 行：注入 `link.ld` + 构建 7 用户程序 + 3 动态夹具 + ELF 校验 | 删掉 `USER_PROGRAMS` / `build_user_program` / `build_dynamic_fixtures` / `validate_image` / `assert_alloc_free`（约 300 行），只留 `link.ld` 注入 | 四 |
| `kernel/src/syscall/user.rs` | 10 处 `include_bytes!(env!("OUT_DIR")/…)` + 用户态运行驱动 | 程序清单改为**可注入**（空清单合法）；`#[cfg(feature = "embed-userland")]` 之外不引用任何用户态字节 | 四 |
| `kernel/src/fs/mod.rs` | `init(programs, libraries)` 建 `/bin`、`/lib`、`/etc/motd`、`/tmp` | 空清单合法：只建 `/etc/motd` + `/tmp`；`/bin`、`/lib` 按需建立 | 四 |
| `kernel/src/main.rs` | 末尾无条件调用 `syscall::user::selftest()` | 门控到 feature；默认路径进入「无用户态 → 串口诊断模式」 | 四 |
| `kernel/src/syscall/syscall_table.rs` | 编号常量 + `SYS_MAX` + `UNIMPLEMENTED` 表 | 编号改由 `abi::syscall::*` re-export（`SYS_TABLE` 结构不变）（✅ 阶段三） | 三 |
| `kernel/src/syscall/dir.rs`、`process.rs` | `DIRENT_SIZE = 88` / `PROCINFO_SIZE = 40` + 记录布局 | 常量与布局移入 `abi::{dirent,procinfo}`，内核 re-export（✅ 阶段三） | 三 |
| `kernel/src/syscall/mod.rs` | `ENOSYS = -38` 等 errno 字面量 | 改用 `abi::errno::*`（✅ 阶段三） | 三 |

| 目录 | 文件（全部**保留、零改动**） |
| ---- | ---- |
| `kernel/src/arch/x86_64/` | `boot.rs`、`context.rs`、`cpu.rs`、`gdt.rs`、`idt.rs`、`interrupt.rs`、`mod.rs`、`paging.rs`、`port.rs` |
| `kernel/src/memory/` | `allocator.rs`、`frame.rs`、`heap.rs`、`mod.rs`、`page.rs` |
| `kernel/src/interrupt/` | `controller.rs`、`controller_test.rs`、`exception.rs`、`irq.rs`、`mod.rs` + `doc CONTRIBUTING_CHAIN.md`、`known_state.md`、`pic_policy.md` |
| `kernel/src/process/` | `context.rs`、`mod.rs`、`pid.rs`、`process.rs`、`thread.rs` |
| `kernel/src/scheduler/` | `mod.rs`、`queue.rs`、`round_robin.rs`、`scheduler.rs` |
| `kernel/src/fs/` | `fat.rs`、`file.rs`、`inode.rs`、`mount.rs`、`ramfs.rs`、`vfs.rs`（`mod.rs` 见上表） |
| `kernel/src/driver/` | `block.rs`、`driver.rs`、`framebuffer.rs`、`keyboard.rs`、`mod.rs`、`net.rs`、`network.rs`、`pci.rs`、`serial.rs`、`virtio_blk.rs`、`virtio_net.rs` |
| `kernel/src/network/` | `arp.rs`、`dhcp.rs`、`ethernet.rs`、`ipv4.rs`、`mod.rs`、`socket.rs`、`tcp.rs`、`udp.rs` |
| `kernel/src/ipc/` | `channel.rs`、`mod.rs`、`pipe.rs`、`shared_memory.rs` |
| `kernel/src/security/` | `capability.rs`、`credential.rs`、`mod.rs`、`permission.rs` |
| `kernel/src/time/` | `clock.rs`、`mod.rs`、`timer.rs` |
| `kernel/`（根） | `link.ld` |

> `kernel/src/syscall/` 其余文件（`address_space.rs`、`dynamic.rs`、`elf.rs`、`file.rs`、`memory.rs`、
> `trampoline.rs`）**保留**：它们是内核的 ELF 装载 / 地址空间 / 用户内存访问机制，
> 属于内核机制而不是用户态产品功能。

### 3.2 `user/`（33 文件，整体迁移到 Cerlesse-Userland）

| 文件 | 处置 | 去向 |
| ---- | ---- | ---- |
| `user/hello/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 + 改名 | `init/`（内核 RAMFS `/init` 的来源） |
| `user/shell/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 | `shell/`（RAMFS `/bin/sh`） |
| `user/ls/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 | `coreutils/ls/` |
| `user/cat/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 | `coreutils/cat/` |
| `user/echo/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 | `coreutils/echo/` |
| `user/ps/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 | `coreutils/ps/` |
| `user/hello-bin/{Cargo.toml,build.rs,user.ld,src/main.rs}` | 迁移 + 改名 | `examples/hello/`（RAMFS `/bin/hello`） |
| `user/dynhello/{README.md,dynhello.s,libgreet.s,ldso.s}` | 迁移 | `tests/dynlink/`（动态链接夹具，`as`+`ld` 现编） |
| `user/init/src/main.rs` | **删除** | 2 行占位、不在 workspace；职责由 `init/`（现 `user/hello`）承担 |
| `user/`（空目录） | 删除 | 迁移完成后本仓库不再有 `user/` |

### 3.3 `libc/`（7 文件）

| 文件 | 处置 | 去向 |
| ---- | ---- | ---- |
| `libc/src/lib.rs`（126）、`syscall.rs`（430）、`console.rs`（91）、`heap.rs`（119）、`args.rs`（95） | 迁移 | `Cerlesse-Userland/libc/src/`（`syscall.rs` 的编号/errno 改引 `abi`） |
| `libc/Cargo.toml` | 迁移 + 改依赖 | 增加 `abi` 依赖，去掉与内核 workspace 的耦合 |
| `libc/README.md`（259） | 迁移 | `Cerlesse-Userland/libc/README.md` |

### 3.4 `shared/` 与 `boot/`（6 文件，全部保留）

| 文件 | 处置 | 说明 |
| ---- | ---- | ---- |
| `shared/src/lib.rs`（22）、`shared/Cargo.toml`（6） | 保留 | 只放 boot↔kernel 纯数据；**不允许**把用户态类型塞进来（阶段二门禁） |
| `boot/src/main.rs`（378）、`serial.rs`（77）、`uefi.rs`（208）、`boot/Cargo.toml` | 保留 | 独立组件与独立目标，本改造不动它 |

### 3.5 `tools/` 与 `tools-go/`（15 文件，含 1 个未入库残留）

| 文件 | 处置 | 理由 / 阶段 |
| ---- | ---- | ---- |
| `tools/image-builder/build_image.py`（63） | 保留 | 内核镜像组装（内核职责） |
| `tools/mkfs/mkfs.py`（544） | 保留 | 内核 FAT 测试盘生成（内核测试） |
| `tools/check_user_elf.py`（128） | 迁移 ✅（2026-10-12） | 校验的是**用户程序** ELF 布局 → `userland/tools/check_user_elf.py`（与已有的 `check_dyn_fixture.py` 同处）；内核侧运行期仍有 `ELF rejected` 校验 |
| `tools/pkgmgr/ckc.py`（722） | 迁移 ✅（2026-10-12） | `.ckc` 打包/校验/事务 = 包管理器（`cpm`）→ `userland/pkgmgr/`（`cpm.py` 与两份套件同批）；格式**规范**留在本仓库 `docs/package-manager.md` |
| `tools/dynexec/__pycache__/dynelf.cpython-310.pyc` | 删除 | 源文件不存在（C9） |
| `tools/check-boundary.py` | **新增** | 边界门禁（阶段二） |
| `tools/README.md`（45） | 保留 + 修订 | 表格同步迁移去向 |
| `tools-go/go.mod`（3）、`cmd/testorch/main.go`（107）、`cmd/serialmon/main.go`（192）、`internal/orchestrator/{config.go,orchestrator.go,orchestrator_test.go}`、`internal/serialparse/{serialparse.go,serialparse_test.go}`、`tools-go/README.md` | 保留 | 内核场景编排与串口分析；阶段五新增 `-tag`/`-exclude-tag` 用法说明（内核侧 `-exclude-tag userland`，用户态侧 `-tag userland`） |

### 3.6 `tests/`（10 文件）

| 文件 | 处置 | 理由 / 阶段 |
| ---- | ---- | ---- |
| `tests/test_boot.py`（151） | 保留 | 内核 QEMU 启动 + 串口断言 |
| `tests/test_keyboard.py`（253） | 保留 | 内核中断/键盘/Shell 输入注入基建（Shell 场景迁出，基建留下） |
| `tests/orchestrate.json`（563，15 场景） | 拆分 ✅（2026-10-12，用标签） | 用户态集成场景标 `"tags": ["userland"]`（现为 6 个：`user`/`ckm`/`shell`/`proc`/`programs`/`dyn`）；`testorch -exclude-tag userland` = 内核组（13），`-tag userland` = 用户态组；物理拆文件待 userland 独立成仓库 |
| `tests/fatroot/README.md` + `data/`（5 文件） | 保留 | 内核 FAT 测试盘内容 |
| `tests/test_ckc.py`（307） | 迁移 ✅（2026-10-12） | `.ckc` 格式测试 → `userland/pkgmgr/tests/test_ckc.py`（与 `test_cpm.py` 一起） |
| `tests/orchestrate-userland.json` | **未采用** | 阶段五实测：用 `tags` 分组（`testorch -tag userland` / `-exclude-tag userland`）比拆两份清单更不易漏改（同一句期望只写一遍），故不新建该文件，见 8.3 偏离 1 |

### 3.7 仓库根与 `docs/`

| 文件 | 处置 | 改造 | 阶段 |
| ---- | ---- | ---- | ---- |
| `Cargo.toml` | 改 | `members = ["shared","boot","abi","kernel"]`；删 `[profile.user-release]` 与 `default-members` 里对用户态的存在意义 | 四 |
| `.cargo/config.toml` | 改（拆） | 内核侧保留 `relocation-model=static`；userland 仓库自建（含 `code-model=large`） | 四 |
| `Makefile` | 改 | 目标分层 + 新目标（阶段五详列） | 二·四·五 |
| `docs/pure-kernel.md` | **新增** | 本文档 | — |
| `docs/syscall.md` | 改 | 成为**正式 ABI 规格**（编号表由 `abi/` 校对，见阶段六） | 六 |
| `docs/architecture.md` | 改 | 第 5 节非目标补「不把内核变成用户态发行版」 | 二 |
| `docs/project-structure.md` | 改 | 第 3 节改为「目标结构（含 `abi/`、userland 外置）」 | — |
| `docs/roadmap.md` | 改 | 新增阶段（本改造） | — |
| `docs/README.md`、`README.md`、`DEVELOPMENT.md` | 改 | 索引、workspace 规划、任务清单同步 | — |
| `docs/package-manager.md` | 保留 | `.ckc` 格式规范留内核侧（P0b/P1 见 11.1） | 五 |

## 4. 六阶段改造计划

每阶段给出：**目标 / 改动 / Cargo 配置 / 构建与测试目标 / 验收**。原则是**渐进迁移**，不允许一次性重写。

| 阶段 | 状态 | 落地内容 |
| ---- | ---- | ---- |
| 一 冻结基线 | ⬜ 未做 | `make baseline` 与 `v0.9.0-baseline` tag 待补（QEMU 证据已在各 `build/serial-*.log`） |
| 二 边界门禁 | ✅ 完成 | `tools/check-boundary.py`（6 项 + `--selftest`）、`make check-boundary`、并入 `make test-all` |
| 三 ABI 单一来源 | ✅ 完成 | `abi/` crate（9 文件 + 7 项宿主自测）、内核 18 文件与 libc 45 常量改为引用它、`make abi-check` 并入 `make check` |
| 四 用户空间迁移 | ✅ 完成 | `libc/` + `user/` → `userland/`（独立 workspace）；内核侧改 3 个文件；`make image-kernel` / `make test-kernel-only` |
| 五 构建与测试拆分 | ✅ 大部分（2026-10-12） | 包管理器（`ckc.py`/`cpm.py` + 两份套件）与 `check_user_elf.py` 已迁入 `userland/`；`test-all` 只含内核场景，用户态集成走 `test-all-with-userland`；编排场景用 `tags` 分组（物理拆分待 userland 独立成仓库）；`git subtree` 未执行 |
| 六 接口稳定 | ⬜ 未做 | `abi` 版本号、`docs/porting.md`、`make abi-doc` 待做 |

### 阶段一：冻结当前基线

| 项 | 内容 |
| ---- | ---- |
| 目标 | 留下一个可回退的稳定版本，记录改造前的全部实测结果 |
| 改动 | 只加一个 `make baseline` 目标（构建 4 类镜像 + 跑 `test-all` + `test-orch` + 归档日志到 `build/baseline/`）；**不改任何代码** |
| Cargo 配置 | 不改 |
| 构建目标 | `make baseline` |
| 验收 | `make baseline` 退出码 0（`test-all` 16 个目标 + `test-orch` 15 场景全绿，日志落在 `build/baseline/`）；打标签 `git tag -a v0.9.0-baseline`（现状：仓库只有 `v0.4`、`v0.4-irq0-stack-fix` 两个 tag，v0.5–v0.9 均未打） |

### 阶段二：明确内核与用户态边界（门禁化）

| 项 | 内容 |
| ---- | ---- |
| 目标 | 把「哪些文件属于哪一层」变成**可执行检查**，防止边界再次被越过 |
| 改动 | 新增 `tools/check-boundary.py`：扫 `kernel/src/**/*.rs`、`kernel/Cargo.toml`、`shared/src/**/*.rs`，断言——① 不出现 `include_bytes!(… user_*)`（迁移期白名单见下）；② 不出现嵌套 `Command::new("cargo")`；③ 不出现 `user/`、`libc/` 路径字面量；④ `shared/` 只含纯数据（无 `fn` 定义之外的逻辑）；⑤ `crate::syscall::user::selftest` 只允许出现在 `main.rs` 的 feature 分支 |
| Cargo 配置 | 不改（门禁只读，不参与构建） |
| 构建目标 | `make check-boundary`（并入 `make check`，作为 CI 必过项） |
| 验收 | 正例：在当前树上**按迁移期白名单**通过（白名单显式列出 C1–C4 的 4 处，带「阶段四移除」标注）；负例：脚本自带 `--selftest` 注入 4 类违规样本，必须全部报错（退出码 1） |

### 阶段三：规范系统调用 ABI（单一事实源）

| 项 | 内容 |
| ---- | ---- |
| 目标 | 消除 C6 的双写：编号、errno、记录布局只有一处定义；**行为零变化** |
| 改动 | 新增 `abi/` crate（`#![no_std]`，无依赖）：`syscall.rs`（号 + `SYS_MAX` + 号段规则）、`errno.rs`、`dirent.rs`（88 字节记录布局）、`procinfo.rs`（40 字节）、`flags.rs`（`O_*`/`PROT_*`/`MAP_*`）、`record.rs`（记录尺寸/对齐的编译期断言）。内核 `syscall_table.rs`、`dir.rs`、`process.rs`、`mod.rs` 改为 re-export；`libc/src/syscall.rs` 同样 re-export（**`libc::SYS_READ` 等对外名字保持不变**，userland 源码无需改） |
| Cargo 配置 | 根 `Cargo.toml` `members` 增加 `"abi"`；`kernel/Cargo.toml`、`libc/Cargo.toml` 各加 `abi = { path = "../abi" }` |
| 构建目标 | `make abi-check`（`cargo test -p abi`，宿主运行）；既有 `make test-all` 不变 |
| 验收 | `make abi-check` 通过（断言：号唯一、`0..=255` 与 `>= 256` 号段不重叠、`SYS_MAX` 与当前真值相等、`DIRENT_SIZE == 88`、`PROCINFO_SIZE == 40`、记录尺寸页内对齐）；`make test-all` 与阶段一基线**逐项一致**（行为零变化）；反例验证——临时改 ABI 里的号，`make abi-check` 必须失败，且把记录尺寸单侧改坏时 QEMU 场景必须失败（证明 ABI 既被**固化**也被**测试覆盖**） |

**执行结果（2026-10-12，✅）**：`abi/` 已落地（9 文件 + 7 项宿主自测），C6 消除，
**行为零变化**（同一批数字，只换了定义点）；验收断言里的 `SYS_MAX == 257` 是**规划时的过期值**
（CKM/CSS 扩号段后实际为 267），`abi` 按当前真值固化。逐项证据、实际改动面（内核 18 文件、
libc 45 常量）与两处刻意偏离见 8.2。

### 阶段四：迁移用户空间（本阶段是唯一的行为性改动）

| 项 | 内容 |
| ---- | ---- |
| 目标 | `user/` + `libc/` 迁到 Cerlesse-Userland；内核在**没有**用户态时也能构建与启动 |
| 改动（内核侧） | ① `kernel/build.rs` 删用户程序构建/校验（约 300 行），只留 `link.ld`；② `kernel/src/syscall/user.rs` 的 10 处 `include_bytes!` 改为 feature 门控 + 外部注入（`CERLESSE_USERLAND_DIR` + `CERLESSE_USERLAND_MANIFEST`，默认空）；③ `fs::init` 空清单合法；④ `main.rs` 的 `selftest` 门控，默认打 `user: no userland image embedded; entering diagnostic idle loop` 后进入 `hlt` 空闲循环 |
| 改动（userland 侧） | 按 3.2/3.3 迁移（保留 git 历史：`git subtree split -P libc` / `-P user`，或 filter-repo）；新仓库自带 `.cargo/config.toml`（`code-model=large` + `relocation-model=static`）、`[profile.user-release]`、`user.ld`、`build.rs`、`Makefile`（构建 + `check_user_elf.py` 校验） |
| Cargo 配置 | 内核：`members = ["shared","boot","abi","kernel"]`，`default-members` 移除（不再需要），feature `embed-userland`（默认关）；userland：`members = ["libc","init","shell","coreutils/*","examples/*"]`，`abi` 路径依赖 |
| 构建目标 | 内核：`make kernel`、`make image`、`make test`、`make test-kernel-only`（新增，无用户态启动验收）；跨仓库：`make image-userland CERLESSE_USERLAND_DIR=<path>` |
| 验收 | ① 只含内核的 tarball（`git archive`）里 `make kernel && make image && make test` 通过；② `make test-kernel-only` 通过：内核启动、内存/调度/IPC/VFS/驱动自测全绿，并在无用户态时进入可诊断状态；③ `make image-userland` + `make test-all` 与阶段一基线逐项一致；④ `make check-boundary` 白名单清空后仍通过 |

### 阶段五：精简构建与测试（按责任拆目标）

| 项 | 内容 |
| ---- | ---- |
| 目标 | 内核构建、引导构建、镜像生成、用户态测试各自独立；任一方向单独可用 |
| 改动 | Makefile 分层：**内核** `help/build/boot/kernel/image/run/test/test-*（内核场景）/baseline/clean`；**跨仓库** `image-userland/test-all-with-userland`（未提供 `CERLESSE_USERLAND_DIR` 时明确报错而非静默跳过）；**移出** `user`、`test-user`、`test-shell`、`test-proc`、`test-programs`、`test-dyn`、`dyn-hostcheck`、`test-pkg`；`test_orch` 改用 `-exclude-tag userland`（19 个场景里的 13 个内核场景） |
| Cargo 配置 | 不改（阶段四已完成分家） |
| 构建目标 | `make test-all`（内核，10 场景 + 各 `test-*`）、`make test-orch`（内核）、userland 仓库 `make test-all` / `make test-orch`（5 场景） |
| 验收 | 内核仓库 `make test-all` + `make test-orch` 通过（内核场景）；userland 仓库 `make test-all`（含 `test_ckc.py` 迁过去的格式测试、`dyn-hostcheck`）通过；两侧分别可在**没有对方源码**时完成**构建**（userland 的 QEMU 运行测试需要内核镜像路径，该依赖在两边 README 明确写出） |

**执行结果（2026-10-12，✅ 大部分）**：

| 项 | 实际做法 | 证据 |
| ---- | ---- | ---- |
| 包管理器迁出 | `tools/pkgmgr/{ckc.py,cpm.py}` + `tests/test_{ckc,cpm}.py` → `userland/pkgmgr/{,tests/}`（新增 `pkgmgr/README.md`）；内核 `tools/pkgmgr/` 不再存在 | `make -C userland test-pkg` 退出码 0（`test-ckc` 37 + `test-cpm` 63） |
| 用户程序 ELF 校验迁出 | `tools/check_user_elf.py` → `userland/tools/check_user_elf.py`（该目录已有 `check_dyn_fixture.py`）—— 用户态构建因此**不需要内核仓库的任何工具** | `rm -rf build/userland/images && make -C userland test-all` 退出码 0 |
| 跨仓库目标 | `CERLESSE_USERLAND_DIR ?= userland`；`test-pkg` 找不到目录时**报错退出 1**（不静默跳过）；新增 `test-all-with-userland`（= 内核 `test-all` + 用户态集成目标） | `make test-pkg` 退出码 0（转发）；`make -n` 可核命令 |
| 编排场景分组 | **用标签而不是拆文件**：`tests/orchestrate.json` 给 6 个用户态集成场景标 `"tags": ["userland"]`（`user`/`ckm`/`shell`/`proc`/`programs`/`dyn`），`testorch` 新增 `-tag` / `-exclude-tag`（词表 `KnownTags` 在加载期硬校验，拼错即报错）；`make test-orch` = `-exclude-tag userland`，`make test-orch-userland` = `-tag userland` | 内核组 13 场景全 PASS；新增 `cmd/testorch/main_test.go`（分组方向 + 未知标签拒绝）且 `go test ./...` 通过 |
| `test-all` 只含内核 | 从 `test-all` 移除 `test-user`/`test-shell`/`test-proc`/`test-programs`/`test-dyn`/`test-ckm`/`test-pkg`，加入 `test-ckm-store`；18 个内核目标逐个实跑全 PASS | 见第 8.3 节逐条结果 |

**与规划的刻意偏离**：

1. **编排场景用标签分组，而**不是**拆成 `orchestrate-kernel.json` + `orchestrate-userland.json`**：期望字符串只写一遍，改一处不会漏改另一处。物理拆分推迟到 `userland/` 真的用
   `git subtree split` 拆出去时（那时文件跟着用户态一起走才有意义）。
2. **目标留在内核 Makefile 里（但不在 `test-all` 中）**：`test-user`/`test-programs` 等仍需要内核侧的 QEMU 驱动
   （`tests/test_boot.py`）与镜像组装，而用户态侧目前只有 Python 工具。等用户态独立成仓库
   时，驱动与场景定义一起搬过去，内核侧只留转发。
3. **验收里的场景计数是扫的旧值**：写「10 场景」时 CKM/CSS 还没落地；现在内核组 **13** 个场景（多出`ckm-store`/
   `storage`/`storage-mbr`），用户态组 **6** 个（多出 `ckm`）。

**仍未做**：`git subtree split -P userland`（拆成独立仓库 —— 需要新仓库与你的决定）；
用户态侧自己的 `make test-orch`（需要内核的 QEMU 驱动与镜像路径，属拆仓后的事）。

### 阶段六：稳定内核接口

| 项 | 内容 |
| ---- | ---- |
| 目标 | 把 ABI 冻结成有版本、有文档、可移植、可独立演进的接口 |
| 改动 | ① `abi` 增加 `VERSION` 与号段规则（`0..=255` 只增不改、新能力一律 `>= 256`）；② `docs/syscall.md` 的编号表改为「由 `abi/` 校对」（`make abi-doc` 生成并校验 diff）；③ 新增 `docs/porting.md`（架构移植规范：`arch/` 需实现的最小集合）；④ `docs/drivers.md` 增「驱动接口规范」（`Driver`/`BlockDevice`/`NetworkDevice` trait 的稳定性承诺）；⑤ `docs/software-development.md` 增「测试指南」（哪些测试属内核、哪些属 userland、如何单跑一个场景）；⑥ 决策点：`abi/` 是否抽成独立可发布 crate |
| Cargo 配置 | 若抽 crate：`abi` 独立版本号 + `abi = "0.1"`（两侧同时引用发布版）；否则保持路径依赖 |
| 构建目标 | `make abi-doc`、`make check`（boundary + abi-check + abi-doc + go-test） |
| 验收 | 第 5 节七条验收标准全部成立（逐条给出命令与输出）；两侧各自打 tag（内核 `v1.0-kernel`、userland `v0.1`），tag 后的 tarball 各自可构建 |

## 5. 「纯内核化」验收标准对照

| # | 标准 | 现状 | 达成阶段 | 验证命令 |
| ---- | ---- | ---- | ---- | ---- |
| 1 | 不依赖 Shell、init、libc、包管理器即可编译内核 | ✅ **成立**（改造前实测 `EXIT=101`，C1） | 四 | 把整个 `userland/` 目录排除出 checkout 后：`cargo build -p kernel --target x86_64-unknown-none`（实测 EXIT=0） |
| 2 | 内核能单独生成可启动镜像 | ✅ **成立** | 四 | 同上 checkout：`make image`（提示不内嵌用户态）或显式 `make image-kernel`（实测 EXIT=0） |
| 3 | QEMU 可验证启动、异常、内存、调度 | ✅ 已有（`test`/`test-exception`/`test-memory`/`test-scheduler`） | 一（冻结） | `make test test-exception test-memory test-scheduler` |
| 4 | syscall ABI 有文档，用户态不依赖内核私有实现 | ✅ 常量单一事实源（`abi/`，C6 消除）；文档有（`docs/syscall.md`），编号表尚未由 `make abi-doc` 自动校对 | 三 ✅ · 六 | `make abi-check` + `make abi-doc` |
| 5 | ELF 加载、进程创建、文件访问由独立测试程序验证 | 🟡→✅（程序现在由 `userland/` 自己构建，内核只按清单内嵌） | 四·五 | `make -C userland images` + `make test-user`(91) / `test-proc`(8+4 正则) / `test-programs`(17+5) / `test-dyn`(10)，均实测 PASS |
| 6 | 内核与用户态分别构建、分别测试、独立演进 | 🟡 独立 workspace ✅；独立 **Git 仓库** ⬜ | 五 | 内核侧 `make image` / `make test-*`；用户态侧 `make -C userland images` 与 `dyn-hostcheck`（**不需要内核源码**）；`git subtree split -P userland` 待执行 |
| 7 | 用户态缺失时内核仍能启动并进入可诊断状态 | ✅ **成立** | 四 | `make test-kernel-only`（7 项断言，实测 PASS；在无 `userland/` 的 checkout 里同一条命令也 PASS） |

## 6. 风险与回退

| 风险 | 影响 | 处置 |
| ---- | ---- | ---- |
| 迁移期 ABI 双写漂移 | 内核与 libc 对同一号/errno 理解不一致，症状是运行期怪异返回值 | ✅ **已处置（阶段三，2026-10-12）**：常量移入 `abi/`，两道防线 —— `abi/src/record.rs` 的编译期断言（改坏即构建失败）+ `make abi-check` 的值固化（已进 `make check`）；反例已验证（8.2 的 #7/#8） |
| 构建期 ELF 校验丢失 | `user.ld` 改坏后要到 QEMU 里才发现 | 校验**两处都留**：构建期校验移到 userland CI（`check_user_elf.py`），内核 `syscall/elf.rs` 的运行期 `ELF rejected: …` 校验保持不动 |
| `make test-fs` / `test-user` 断言写死用户程序文件 | 无用户态时断言失败（`fs: ramfs files=N`、`fs: ls / entries=5`） | ✅ 已处理：内核 **`fs::selftest` 的根目录条目数改为推导**（`4 + /init 是否存在`）——这正是纯内核启动第一次跑时真正拦住的地方（`fs: FAIL: fs: root directory entry count mismatch`，见第 8 节）；`make test-fs`（有用户态）继续断言 5 项，纯内核路径由 `make test-kernel-only` 覆盖 |
| **构建顺序**（执行中新发现） | `make image` 可能先编内核再出用户态镜像，于是「有用户态的完整镜像」静默变成纯内核镜像 | ✅ `image: boot userland-images` + 在 recipe 里子 make 调 `kernel`（「先出镜像 → 再编内核 → 最后组装」）；改写后在**全新 checkout** 实测：`warning: embedded userland: 8 programs + 2 libraries` |
| 嵌套 cargo 与外层 target 锁 | 构建期互相等锁 / 复用错误 rustflag（历史上踩过 `CARGO_ENCODED_RUSTFLAGS`） | 阶段四整段删除嵌套构建，问题自然消失；userland 自己 `cargo build` |
| `.cargo/config.toml` 拆开后用户程序丢了 `code-model=large` | 链接期 `R_X86_64_32S out of range` | 新仓库自带 `.cargo/config.toml`，并在 userland `make` 里断言 `check_user_elf.py` 通过（含基址/页对齐/W^X/无重装载重定位） |
| git 历史与 CI 断档 | 迁移后 blame 丢失 / PR 流水线改两处 | 用 `git subtree split`（或 filter-repo）保历史；CI 按仓库各自配置，跨仓库用 `CERLESSE_USERLAND_DIR` 环境变量做集成回归 |
| 阶段四一次改动面大 | 内核启动链被破坏 | ✅ 无用户态分支放在 `selftest()` 开头，两条路径共用后面的代码；改完逐项重跑：`test-all` 的 15 个场景断言全部满足（13 个来自编排日志 + `test-scheduler`/`test-network` 单跑） |
| 内核「不认识程序名」退化 | 清单与内核期望不一致时报错难懂 | ✅ 在 `syscall/user.rs` 保留契约常量（8 程序路径 + 2 库路径），`selftest` 开头逐条校验清单，缺失时打印具体路径再报错 |

## 7. 非目标

- 不重写内核、不改 Rust 技术路线、不改为微内核/微服务化拆分。
- 不删除 `fs/`、`network/`、`driver/` 里的实现代码 —— 它们是内核服务，不是用户态产品功能。
- 不为「跑 Linux 二进制」引入兼容层；Linux 号段语义只是本内核 ABI 的**选型参考**（见 `docs/syscall.md`）。
- 不把 `tools/`、`tools-go/` 搬出本仓库（它们是构建/测试工具，不进入内核运行时）。
  例外：`pkgmgr/`（现 `userland/pkgmgr/`）与 `check_user_elf.py`（现 `userland/tools/`）是**用户态侧**的工具，已随阶段五迁入 `userland/` ——
  这不是「把构建工具搬走」，而是把用户态产品功能搬回用户态。
- 不要求两侧在**没有任何对方**的情况下都能跑完整集成测试：userland 的 QEMU 测试需要内核镜像路径，这是**显式依赖**而不是边界违反。
- 本文档不排期；顺序即依赖顺序（一 → 六）。原「阶段三之前不要动用户态源码」的约束**已解除**（阶段三 2026-10-12 完成）。

## 8. 执行记录

### 8.1 阶段二/四/五（2026-10-11）

本轮做了**阶段二（边界门禁）、阶段四（用户空间迁移）、阶段五的一部分**，未动阶段一/三/六。
全部结论都有本机实测支撑（命令 + 结果）：

| # | 验证 | 结果 |
| ---- | ---- | ---- |
| 1 | 改造前：删掉 `libc/`+`user/` 后 `cargo build -p kernel` | ❌ `EXIT=101`（`kernel/build.rs:301`）—— 这是本文档的起点 |
| 2 | 改造后：无 `userland/` 的 checkout 里 `cargo build -p kernel --target x86_64-unknown-none` | ✅ EXIT=0 |
| 3 | 同上 checkout：`make image`（提示不内嵌用户态）+ `make test-kernel-only` | ✅ 7/7 断言（`Kernel started!` → `fs: disk selftest PASS` → `sched: round-robin wrap PASS` → `pipe: selftest PASS` → `user: no userland images embedded; entering diagnostic idle loop` → `user: kernel-only boot PASS`） |
| 4 | 全新 checkout（含 userland）：`make image` | ✅ `embedded userland: 8 programs + 2 libraries`（验证「先出镜像再编内核」的顺序） |
| 5 | 用户态独立构建：`make -C userland images` | ✅ 7 个程序（每个过 `check_user_elf.py` 段布局校验）+ 3 个动态链接对象（`check_dyn_fixture.py`：ET_DYN + `PT_INTERP` + `DT_NEEDED`）+ `manifest.txt` |
| 6 | `make -C userland dyn-hostcheck`（宿主 glibc） | ✅ `dyn-hostcheck PASS (glibc ld.so, exit=42)` |
| 7 | 边界门禁：`make check-boundary` | ✅ 6/6 通过（改造前 5/6 违规）；`--selftest` 6/6 违规样本全部被抓到 |
| 8 | 全量回归：15 个编排场景 + `test-user`(91) / `test-shell`(11) / `test-proc`(8+4) / `test-programs`(17+5) / `test-dyn`(10) / `test-fs`(21) / `test`/`test-memory`/`test-scheduler`/`test-network` | ✅ 全部 PASS（编排日志 13 个场景逐条核对 + `scheduler`/`network` 单跑） |

**改动面**：内核侧 3 个文件（`kernel/build.rs` 重写、`kernel/src/syscall/user.rs`、`kernel/src/fs/mod.rs`）
+ 根 `Cargo.toml`/`Makefile`；`libc/`(7 文件) 与 `user/`(33 文件) 整体移入 `userland/`（git 记录为 rename，
历史不丢）+ 新增 `userland/{Cargo.toml,.cargo/config.toml,Makefile,README.md,tools/check_dyn_fixture.py}`
与 `tools/check-boundary.py`。

**下一轮建议顺序**（2026-10-12 起）：阶段一（`make baseline` + `v0.9.0-baseline` tag）→
阶段六（`abi` 版本号与 `make abi-doc`、`docs/porting.md`、驱动接口规范）→
`git subtree split -P userland` 开新仓库（阶段五的唯一剩余项；阶段三已于 2026-10-12 完成，
阶段五已大部分完成，见 8.2/8.3）。

### 8.2 阶段三：ABI 单一事实源（2026-10-12）

`abi/` crate 落地，C6 消除；**行为零变化**（同一批数字，只换了定义点）。全部结论都有本机实测支撑：

| # | 验证 | 结果 |
| ---- | ---- | ---- |
| 1 | `cargo build -p kernel --target x86_64-unknown-none`（含 `abi` 依赖） | ✅ EXIT=0，0 warning |
| 2 | `cd userland && cargo build -p libc --target x86_64-unknown-none` | ✅ EXIT=0 |
| 3 | `make abi-check`（`cargo test -p abi`） | ✅ 7/7（号唯一 + 号段不重叠 + `SYS_MAX`/记录尺寸 + CKM/存储字面量 + 标志位组合） |
| 4 | `make check-boundary`（K4 允许 `abi` 依赖、K5 允许 `abi` 成员） | ✅ 6/6；`--selftest` 6/6 违规样本全部被抓到 |
| 5 | `make image` | ✅ `embedded userland: 9 programs + 2 libraries` |
| 6 | QEMU 回归：`test`(1) · `test-user`(96) · `test-programs`(32+7) · `test-proc`(20+4) · `test-ckm`(44+3 / 8+2 / 6+3) · `test-storage`(40+3) · `test-storage-mbr`(15+1) · `test-fs`(21) · `test-dyn`(10) · `test-shell`(11) · `test-memory`(5) · `test-scheduler`(7) · `test-ipc`(6) · `test-kernel-only`(7) | ✅ 全部 PASS（未跑到的：`test-exception`/`test-pagefault`/`test-keyboard`/`test-driver`/`test-fat`/`test-network`/`test-orch`/`test-pkg`/`go-test`，它们不涉及本次改动面） |
| 7 | **反例 1**：把 `abi::syscall::SYS_GETPROCS` 改成 255 | ✅ `make abi-check` 退出码 2，`segments_do_not_overlap` 断言失败 —— 号被固化，改号必红 |
| 8 | **反例 2**：只把 libc 的 `DIRENT_SIZE` 改 `abi::dirent::DIRENT_SIZE + 8`（单侧漂移，即改造前的"两处镜像"毛病） | ✅ `make test-programs` 退出码 2，报 `FAIL: 'ls: entry name=init …' not found`；回滚后 PASS —— 记录布局 ABI 真的被 QEMU 场景覆盖，不只是被常量断言覆盖 |

**实际改动面**（规划 4 个内核文件，实际 18 个 —— 规划写于 CKM/CSS 落地之前，那两批 ABI 常量当时还不存在）：

| 位置 | 改动 |
| ---- | ---- |
| `abi/`（新，10 文件） | `syscall.rs`（号 + `SYS_MAX` + 号段规则 + `NAMED`/`UNIMPLEMENTED`）、`errno.rs`、`flags.rs`（`O_*`/`PROT_*`/`MAP_*`/fd 编号）、`limits.rs`（`PATH_MAX`/`IO_MAX`/`ARG_*`）、`dirent.rs`、`procinfo.rs`、`volume.rs`、`ckm.rs`（记录尺寸 + 能力位 + op 编号 + 受保护前缀）、`record.rs`（编译期断言）、`lib.rs`（7 项宿主自测） |
| `kernel/src/syscall/`（9） | `syscall_table.rs`（编号 + `UNIMPLEMENTED` 再导出）、`mod.rs`（`ENOSYS`）、`dir.rs`、`process.rs`、`file.rs`、`memory.rs`、`storage.rs`、`address_space.rs`（`PROT_*`）、`ckm.rs`（经 `ckm::*` 间接引用） |
| `kernel/src/fs/file.rs` | `FD_FIRST_DYNAMIC`/`O_*`/`O_ACCMODE`/`O_ALLOWED` 再导出（该文件在阶段四的"零改动"清单里，阶段三只改这 7 行） |
| `kernel/src/security/capability.rs` | 6 个能力位再导出 + `ALL/ADMIN/INIT/SERVICE` 改为由 `abi::ckm::CAP_ALL` 推导，并加 3 条编译期断言 |
| `kernel/src/ckm/{mod,audit,state,policy}.rs` | `CKM_ABI_VERSION`/`STATE_SLOTS`/记录尺寸再导出；`COMPONENT_ID_LEN` 由 `COMPONENT_ID_MAX + 1` 推导；`Op::code`/`from_code` 的 32 处数字改 `abi::ckm::OP_*`；`PROTECTED_PREFIXES` 再导出 |
| `kernel/src/storage/volume.rs` | 记录尺寸 / 字段宽度 / `FREE_UNKNOWN` 再导出 |
| `userland/libc/src/syscall.rs` | 45 个常量改为 `abi::…` 值绑定（**对外名字与类型全不变**：`isize`/`i32` 保留） |
| 根 `Cargo.toml` / `kernel/Cargo.toml` / `userland/libc/Cargo.toml` / `Makefile` | 成员与路径依赖 + `make abi-check` 并入 `make check` |

**与规划的两处刻意偏离**：

1. **`SYS_MAX` 的规划值是过期的**：验收写 `SYS_MAX == 257`，实际 **267**（CKM 257–260、存储 261–266
   之后扩过号段）。`abi` 按当前真值固化，并断言 `LINUX_SEGMENT_END == CERLESSE_SEGMENT_START == 256`。
2. **libc 侧用"值绑定"而不是逐条 `pub use`**：libc 的每个常量都带面向用户态的文档
   （`-EAGAIN` 只由 `read(0)` 返回、`DIRENT_SIZE` 与记录布局的关系…），且类型与内核不同
   （`isize`/`i32` vs `i64`/`u64`）。逐条 `pub use` 会丢掉这些文档并改对外类型，
   因此保留 `pub const X: T = abi::… as T;` —— "值只有一个定义点"这个目标不受影响，
   类型与文档也零变化。

**仍未做**：阶段一（`make baseline` + `v0.9.0-baseline` tag）、
阶段六（`abi` 版本号与 `make abi-doc`、`docs/porting.md`、驱动接口规范）。
阶段区五的剩余项（包管理器/ELF 校验迁出、编排分组、`test-all` 只含内核）已于同日完成，见 8.3。

### 8.3 阶段五：构建与测试按责任拆分（2026-10-12）

目标：**内核仓库的测试只测内核**，用户态那一半单独可跑；构建工具按归属摆放。

| # | 验证 | 结果 |
| ---- | ---- | ---- |
| 1 | `make -C userland test-pkg`（`ckc.py`/`cpm.py` 与两份套件迁到 `userland/pkgmgr/` 后） | ✅ 退出码 0（`test-ckc` 37 + `test-cpm` 63） |
| 2 | `make test-pkg`（内核侧转发，`CERLESSE_USERLAND_DIR ?= userland`） | ✅ 退出码 0；目录缺失时报错退出 1（不静默跳过） |
| 3 | `rm -rf build/userland/images && make -C userland test-all`（images + check + dyn-hostcheck + test-pkg，`check_user_elf.py` 也已迁入） | ✅ 退出码 0 —— 用户态侧**不需要内核仓库的工具**就能构建与测试 |
| 4 | `go vet ./... && go test ./...`（含新增 `cmd/testorch/main_test.go`：分组两个方向 + 未知标签拒绝） | ✅ 全部通过 |
| 5 | `make test-orch` 命令（`-exclude-tag userland`，13 个内核场景） | ✅ 13/13 PASS（boot/memory/exception/pagefault/ckm-store/fs/driver/fat/storage/storage-mbr/keyboard/scheduler/network） |
| 6 | `make test-all` 的 18 个内核目标逐个单跑 | ✅ 全 PASS（abi-check、check-boundary、test、test-kernel-only、test-exception、test-memory、test-pagefault、test-keyboard、test-scheduler、test-ipc、test-fs、test-ckm-store、test-driver、test-fat、test-storage-fixtures、test-storage、test-storage-mbr、test-network）；**v1.0 起 `test-all` 为 19 目标**（新增 `test-ahci`，见 [drivers.md](drivers.md) 1.9） |
| 7 | `make test-all-with-userland` 的用户态那半（`test-user`/`test-shell`/`test-proc`/`test-programs`/`test-dyn`/`dyn-hostcheck`/`test-ckm`） | ✅ 全部 PASS（与上一轮相同命令；本轮只重跑了 `dyn-hostcheck` 与新到位置的 `test-pkg`） |

**一项能力实测提醒**：本机内存 1983 MiB，`testorch -parallel 6` 会把 6 个 512 MiB 的 QEMU 顶爆（实测 `ckm-store` 的串口日志 0 字节、场景失败）；默认 `concurrency=2` 下
一切正常。这不是代码问题，但使用 `-parallel` 覆盖并发度时要看内存。

**仍未做**：`git subtree split -P userland`（开新仓库；需要你决定），
以及用户态侧自己的 `make test-orch`（它需要内核的 QEMU 驱动与镜像路径，属拆仓后的事）。

## 9. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-12 | **执行阶段五（构建与测试按责任拆分）**：`tools/pkgmgr/{ckc.py,cpm.py}` + `tests/test_{ckc,cpm}.py` → `userland/pkgmgr/`（新增 `README.md`；内核侧 `test-pkg` 改为**跨仓库转发**，`CERLESSE_USERLAND_DIR` 缺失时明确报错）、`tools/check_user_elf.py` → `userland/tools/`（用户态构建不再需要内核工具）、`tests/orchestrate.json` 用 `tags` 分组（6 个用户态集成场景标 `userland`）+ `testorch` 新增 `-tag`/`-exclude-tag`（词表 `KnownTags` 加载期校验）与 `main_test.go`；内核 `make test-all` 改为**纯内核**（19 目标，逐条实跑全 PASS）、新增 `test-all-with-userland`/`test-orch-userland`；实测：`make -C userland test-all` 与 `make -C userland test-pkg` 退出码 0，内核组 13 场景全 PASS。详见 8.3 |
| 2026-10-12 | **执行阶段三（ABI 单一事实源）**：新增 `abi/` crate（调用号/`SYS_MAX`/号段规则、errno、`O_*`/`PROT_*`/`MAP_*`/fd、`PATH_MAX`/`IO_MAX`/`ARG_*`、`dirent`/`procinfo`/`volume`/`ckm` 记录布局与字面量、编译期断言）；根 `Cargo.toml` 增加成员，`kernel` 与 `userland/libc` 各加路径依赖；内核 18 个文件、libc 45 个常量改为引用 `abi`（对外名字与类型不变、行为零变化）；新增 `make abi-check` 并并入 `make check`。实测：两处构建 EXIT=0、`make abi-check` 7/7、`make check-boundary` 6/6、14 个 QEMU 目标全 PASS；两个反例（改 `SYS_GETPROCS` → `abi-check` 退出码 2；只改 libc 的 `DIRENT_SIZE` → `test-programs` 退出码 2 且 `ls` 报错）证明 ABI 既被固化也被测试覆盖。C6 消除 |
| 2026-10-11 | **执行阶段二/四/五（部分）**：新增 `tools/check-boundary.py`（6 项边界门禁 + `--selftest` 反向自检）与 `make check-boundary`/`make check`，并入 `make test-all`；`libc/`+`user/` 移入 `userland/`（独立 workspace：自带 `Cargo.toml`/`.cargo/config.toml`/`Makefile`，产出 `build/userland/images/{manifest.txt,*.elf}`，`dyn`/`dyn-hostcheck`/`check_dyn_fixture.py` 随迁）；`kernel/build.rs` 重写为「读清单 → 复制 → 生成 `include` 源码」（删除约 300 行嵌套 cargo/as/ld，内核不再需要 binutils），`syscall/user.rs` 的 10 处 `include_bytes!` 收敛为一次 `include!` + 契约校验 + 空清单诊断分支，`fs::selftest` 根目录条目数改为推导；根 `Cargo.toml` 收敛为 `shared/boot/kernel` + `exclude userland`，新增 `userland-images`/`image-kernel`/`kernel-nouserland`/`test-kernel-only` 目标。实测：无 `userland/` 的 checkout 里 `make image`+`make test-kernel-only` 通过（验收标准 1/2/7 成立）、全新 checkout `make image` 正确内嵌 8 程序 + 2 库、15 个场景与全部用户态测试 PASS；执行中发现并修掉「`image` 可能先编内核再出镜像」的顺序缺陷 |
| 2026-10-11 | 创建本文档：现状审计（C1–C9，含「删掉 `libc/`+`user/` 后内核构建 `EXIT=101`」与内核内含 203,280 字节（≈199 KiB）用户态的实测）、目标结构与两仓库划分、逐文件迁移清单（`kernel/` 83 文件逐项 + `user/`33 + `libc/`7 + `shared`/`boot`6 + `tools`/`tools-go`15 + `tests`/10 + 仓库根与 docs）、六阶段（冻结基线 / 边界门禁 / ABI 单一来源 / 用户空间迁移 / 构建与测试拆分 / 接口稳定）、七条验收标准对照表、风险与回退、非目标 |
