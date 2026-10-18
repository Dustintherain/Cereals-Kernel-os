<div align="center">

# Cereals-Kernel-Cerlesse

**用 Rust 从零写起、`make run` 就能进交互式 Shell 的 x86_64 类 Linux 宏内核（代号 Cerlesse OS）**

<img src="assets/banner.svg" alt="Cerlesse OS — an x86_64 kernel written from scratch in Rust" width="100%">

[![Rust no_std](https://img.shields.io/badge/Rust-no__std%20%C2%B7%20no%20third--party%20crates-000000?logo=rust&logoColor=white)](#这是什么)
[![Target](https://img.shields.io/badge/target-x86__64%20%C2%B7%20UEFI%20%C2%B7%20QEMU-1c1c1c)](#快速开始)

</div>

---

## 这是什么

Cerlesse 是一个**从零实现**的 x86_64 宏内核（monolithic kernel，`no_std` Rust、不依赖任何第三方 crate）：UEFI 引导、4 级页表与物理帧分配器、中断与时钟、进程与 Round Robin 调度、`syscall`/`sysret` 快速路径与 ring3 隔离、VFS + RAMFS + FAT12/16/32、PCI 枚举与 legacy virtio / AHCI/SATA 驱动、用户态 Shell 与基础工具、Ethernet/ARP/IPv4/UDP 网络栈、内核侧动态链接、GPT/MBR 分区与卷识别 —— 每一层都是自己写的。

它不是 Linux 的替代品，也不是只会打印一行字的骨架：`make run` 之后，QEMU 串口（也就是你的终端）会给出一个**真能用的 Shell** —— `ls` / `ps` / `cat` / `echo` 都是独立用户程序，由 Shell 的 `fork` + `execve` 起来，`wait4` 回收退出码。

## 为什么需要它

读源码和教材能理解概念，但只有自己实现一遍才会撞上真正的细节 —— debug 构建下 16 KiB 内核栈静默溢出踩坏帧分配器、进程退出必须先恢复 CR3 再 `Drop` 地址空间、清页表项不 `invlpg` 就会留下幽灵映射。这些是读不出来的。

自研系统的常见死法也不是写不出来，而是**停在「能打印一行字」**：没有验收手段，就无法判断某一层是否真的正确。所以这个项目把三件事当成硬约束：

1. **文档即规格** —— 能力先写进 [`docs/`](docs/README.md)（含设计取舍与踩坑），再落代码；
2. **串口是唯一权威日志通道** —— 集成测试只断言串口输出，可以在 CI 里无人值守复现；
3. **关键实现交给独立一方复核** —— `mkfs` 生成的 FAT 盘由 **mtools** 读、内核链接的动态库夹具由**宿主机 glibc `ld.so`** 跑出相同输出与退出码、`.ckc` 包的 payload 由系统 **`tar`** 读。

## 快速开始

```bash
make build   # 引导器 + 内核 + 磁盘镜像（build/disk.img）
make run     # QEMU + OVMF 启动，串口输出到终端
```

启动后你会看到内核自测日志与提示符，然后就能像用命令行一样操作它：

```text
root@cerlesse C:> ls /bin
ls: /bin entries=9
root@cerlesse C:> cat /etc/motd
motd: hello from ramfs
root@cerlesse C:> hello world
hello-bin: Hello from an external program!
root@cerlesse C:>
```

## 安装

| 依赖 | 说明 |
| --- | --- |
| Rust（stable / `rustup`） | 需要目标 `x86_64-unknown-none` 与 `x86_64-unknown-uefi` |
| QEMU | `qemu-system-x86_64`（机器 `q35`，512 MiB） |
| OVMF | `OVMF_CODE.fd` / `OVMF_VARS.fd`（默认 `/usr/share/OVMF/`） |
| GNU Make + Python 3 | 构建入口与主机侧工具（仅标准库，无第三方包） |
| 可选：mtools | `make test-fat` 的独立复核（`mdir` / `mtype`） |
| 可选：Go | 并行测试编排与串口日志分析（`make go-test` / `make test-orch`） |
| 可选：binutils | 只影响用户态的动态链接夹具（`as` / `ld`）；内核构建不再依赖它 |

```bash
rustup target add x86_64-unknown-none x86_64-unknown-uefi
# Debian/Ubuntu 例：sudo apt-get install qemu-system-x86 ovmf mtools make python3 golang
```

工具链路径不一致时用 Makefile 变量覆盖：

```bash
make run OVMF_CODE=/path/OVMF_CODE.fd OVMF_VARS=/path/OVMF_VARS.fd QEMU=/path/qemu-system-x86_64
```

## 你会得到什么

以真实串口片段为准（下面每一行都是集成测试里被断言的输出）：

```text
Kernel started!
driver: attached 00:03.0 -> virtio-blk-pci
driver: attached 00:1f.2 -> ahci-pci
ahci: port 2 model=QEMU HARDDISK capacity=131072 sectors (64 MiB) lba48=1
block: device=virtio-blk0 sector_size=512 sectors=131072 bytes=67108864
block: device=ahci0 sector_size=512 sectors=131072 bytes=67108864
fs: mount point=/ fs=ramfs
fs: mount point=/disk fs=fat
user: program=/bin/sh source=elf
IRQ0_heartbeat tick=100
root@cerlesse C:> ls /bin
ls: /bin entries=9
root@cerlesse C:> ps
ps: pid=3 ppid=0 state=RUNNING comm=sh
root@cerlesse C:> hello world
hello-bin: Hello from an external program!
hello-bin: argv[1]=world
sh: hello: exit code=1
root@cerlesse C:>
```

- **一条命令进入可用系统**：`make run` 构建引导器 + 内核 + 磁盘镜像并启动 QEMU，串口就是控制台。
- **每一层都有可复现的验收入口**：`make test-all` 跑全部集成测试目标加 ABI 门禁（`make abi-check`）与边界门禁，覆盖引导 / 异常 / 内存 / 中断 / 调度 / IPC / 文件系统 / 存储（分区/卷/多挂载点/卸载） / 驱动 / 网络 / 用户态；`make test-orch` 再用 Go 编排器并行跑 20 个场景并二次复核串口日志。
- **机制与功能分层，而且边界有门禁**：内核只提供机制，Shell、`init`、`libc`、coreutils 位于独立 workspace [`userland/`](userland/README.md)。把 `userland/` 整个移出 checkout，`make image-kernel` 仍能产出可启动镜像（`make test-kernel-only` 通过），`make check-boundary` 会挡住把依赖加回来的提交。
- **可读的设计与踩坑记录**：每个子系统一份规格文档（含「为什么这样做」和真实故障复盘），入口见 [文档索引](docs/README.md)。

## 工作方式

按 **v0.1 → v1.0** 逐层推进，每个版本只交付一个可验证的能力：引导 → 异常诊断 → 内存与内核堆 → 中断/时钟/键盘 → 进程与调度 → syscall 与用户态 → 文件系统 → 驱动与磁盘 → Shell、用户程序与网络 → 动态链接。落地即写验收，验收证据进文档。内核与用户态的唯一接触面是构建产物 `build/userland/images/manifest.txt`，内核**不认识任何程序名**，因此用户态可以整体搬走。

## 常用命令

| 命令 | 作用 |
| --- | --- |
| `make test` | 冒烟测试：断言串口出现 `Kernel started!` |
| `make test-all` | 全部集成测试 + 边界门禁 |
| `make test-user` | 用户态全链路：ELF 装载 + 文件 syscall + 用户堆 + 隔离 + Shell |
| `make test-shell` / `make test-proc` | 注入真实按键跑交互式 Shell / `fork` + `execve` + `wait4` |
| `make test-programs` | 从 Shell 跑 `/bin/{ls,cat,echo,ps,volumes}`（`getdents` 列目录、`getprocs` 列进程、`storage_volumes` 列卷） |
| `make test-ckm` / `make test-ckm-store` | CKM 权限/审计/组件状态 + 覆盖层（注册/撤销改变 `open` 结果）/ 可写状态区的**跨重启**持久化（写 → 重启 → 读回） |
| `make test-fs` / `make test-fat` | VFS + RAMFS / FAT12/16/32 只读后端（双盘启动） |
| `make test-driver` / `make test-ahci` | PCI 枚举 + virtio-blk / **芯片组自带的 AHCI（SATA）控制器 + 盘上同一套块设备验收** |
| `make test-network` | virtio-net + ARP + DHCP + TCP（slirp；TCP 回显由测试桩在宿主侧起一次性回声服务 `--tcp-echo-port`） |
| `make test-dyn` / `make dyn-hostcheck` | 内核侧最小动态链接 / 同一批夹具交给宿主机 glibc `ld.so` 复核 |
| `make test-storage` / `make test-storage-mbr` | GPT / MBR 分区镜像的卷枚举（`storage_volumes`）+ VFS 收口（卷身份上挂载点、多挂载点、卸载）+ 主机侧夹具复核 |
| `make check` / `make abi-check` / `make check-boundary` | 主机侧门禁（**ABI 单一事实源** + 边界 + Go 单测）/ `abi` crate 自测（号/号段/记录尺寸/能力位）/ 内核不得依赖用户态 |
| `make test-orch` / `make test-pkg` | 内核组 14 个场景并行编排（`testorch -exclude-tag userland`；用户态组用 `make test-orch-userland`）/ 包管理器（主机侧，不启 QEMU）：`.ckc` 容器（`test-ckc` 37 项）+ **事务引擎 `cpm`（`test-cpm` 63 项）** |

其余目标（`image-kernel`、`userland-images`、`fat-image`、`go-test` 等）及其输出含义见 [DEVELOPMENT.md](DEVELOPMENT.md) 与 [docs/api.md](docs/api.md)。

## 项目结构

```text
boot/      UEFI 引导              kernel/    Rust 内核（内存/中断/调度/syscall/fs/驱动/进程）
shared/    引导与内核共享结构      abi/       系统调用 ABI 单一事实源（内核与用户态引用同一处）
userland/  用户态独立 workspace（libc + Shell + 用户程序）
tools/     Python 主机工具        tools-go/  Go 侧并行编排与串口日志分析
tests/     QEMU 集成测试与编排    docs/      开发文档（每个子系统一份规格）    assets/  README 视觉资产
```

内核只提供机制，用户态提供功能。系统调用 ABI（调用号 / errno / 标志位 / 记录布局）只有一个定义点：
`abi/` crate —— 内核与 `userland/libc` 都引用它，`make abi-check` 固化，任何一侧改坏都会构建或测试失败。
完整目录树与模块职责见 [docs/project-structure.md](docs/project-structure.md)，内核/用户态划分与迁移阶段见 [docs/pure-kernel.md](docs/pure-kernel.md)。

## 文档入口

| 从这里开始 | 内容 |
| --- | --- |
| [docs/README.md](docs/README.md) | 文档索引与各阶段状态（先看这个） |
| [DEVELOPMENT.md](DEVELOPMENT.md) | 主开发文档：需求、依赖、**逐版本任务清单与完成记录**、测试方案、ADR、缺陷汇总 |
| [docs/architecture.md](docs/architecture.md) | 架构、语言分工（Rust 内核 / Python 工具 / Go 服务）、目标与非目标 |
| [docs/syscall.md](docs/syscall.md) · [docs/process.md](docs/process.md) | 系统调用与用户态 / 进程、调度、IPC |
| [docs/filesystem.md](docs/filesystem.md) · [docs/drivers.md](docs/drivers.md) | VFS、RAMFS、FAT / 驱动框架、PCI、virtio、网络栈 |
| [docs/roadmap.md](docs/roadmap.md) · [docs/desktop.md](docs/desktop.md) | 路线图 / v1.0 之后的桌面环境兼容路线 |
| [docs/ckm.md](docs/ckm.md) · [docs/package-manager.md](docs/package-manager.md) | CKM（内核级权限控制与扩展管理）：权限模型、内核接口、扩展生命周期（CKM-1/2/3 已落地） / `.ckc` 包格式（P0a）与事务引擎 `cpm`（P0b = CKM-3） |
| [docs/storage.md](docs/storage.md) | CSS（存储管理）规划：卷模型与稳定标识、Windows 式盘符 + Linux 式挂载双视图、VFS 收口（**CSS-1/CSS-2 已落地**）、与 CKM 的权限/持久化对接 |

## 当前状态

v0.1 – v0.9 已全部实现并通过自动化验收：UEFI 引导 → 异常诊断 → 内存与内核堆 → 中断、时钟与键盘 → 进程与 Round Robin 调度 → `syscall` 与 ring3 隔离 → VFS/RAMFS/ELF 装载 → PCI、virtio-blk 与 FAT → 用户堆、交互式 Shell、`fork`/`execve`/`wait4`、简化网络栈、基础用户程序、内核侧最小动态链接。

v1.0 正在进行（第一步：运行期内存保护 `mprotect`，2026-10-11 落地；第二步：AHCI/SATA 驱动 + PCI 枚举修缺陷 B-18，2026-10-14 落地；第三步：Socket/TCP 补齐 + `/bin/tcpecho`，2026-10-14 落地，`make test-network` 23 项 / `make test-user` 96 项 PASS；其余为 ICMP、权限与凭证、完整启动链、发布）。

**v1.0 之后的目标定位**（[CKM 设计提案](docs/ckm.md)）：

```text
Cerlesse Kernel + CKM 内核控制核心 + CKM 用户态管理服务 + 可插拔扩展系统 + 独立恢复环境
```

内核只提供强制权限与组件状态机制（CKM Core），扩展安装/事务/修复由持权的用户态服务完成，
系统文件通过可撤销的覆盖层修改 —— 分层思路参考 KernelSU，但不照搬 Android 的 root 机制；
第一版**不做**动态内核模块装载与图形界面。

**已落地 CKM-1（内核接口 + 判定点 + syscall 257–259）、CKM-2（双槽 A/B 可写状态区 + 覆盖层机制）
与 CKM-3（事务引擎，主机侧 `userland/pkgmgr/cpm.py`）**：回执、事务日志、`stage → activate → commit`
可回滚、掉电后 `recover`、`gc` 与单写者锁都已可脚本断言（`make test-ckm` / `make test-cpm`）；
CKM-4（`ckmd`/`ckm`）起把事务接进内核侧用户态服务。

存储侧有一条并列的路线（[CSS 存储管理提案](docs/storage.md)）：**像 Windows 一样管理磁盘与卷**
（盘符、卷标、稳定卷标识、多挂载点），**像 Linux 一样通过统一 VFS 兼容多种文件系统**
（新格式只是再实现一个 `FileSystem`）。**已落地 CSS-1（底层识别）与 CSS-2（VFS 收口）**：
GPT/MBR 只读扫描 + 稳定 `vol_id` + `storage_volumes`/`storage_rescan`，挂载表带卷身份、一个卷可挂多个点、
卸载后卷仍在表里、挂载失败给 errno（`EEXIST`/`ENOSPC`/`ENOTSUP`/`EUCLEAN`…）；
盘符分配（CSS-3）与写能力（CSS-4：FAT → exFAT → ext2）按阶段开放，第一版不写分区表。

逐版本的实现记录、验收证据与踩坑见 [DEVELOPMENT.md](DEVELOPMENT.md) 第 5 节，路线图见 [docs/roadmap.md](docs/roadmap.md)。

## 关于作者

由 [@Dustintherain](https://github.com/Dustintherain) 开发与维护。问题与建议提到 [Issues](https://github.com/Dustintherain/Cereals-Kernel-Cerlesse/issues)。

## 许可证

本仓库以 **MIT** 许可发布，完整条款见 [`LICENSE`](LICENSE)：可自由使用、修改、分发（含商业用途），保留版权声明与许可声明即可。

```text
MIT License · Copyright (c) 2026 Dustintherain
```

若需更换许可（例如 Apache-2.0 或双许可），改 `LICENSE` 与本节的表述即可 —— 目前仓库里没有第三方 vendored 代码，替换许可不涉及兼容性排查。
