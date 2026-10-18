# Cerlesse OS 文档中心

- 阶段：第一阶段 / 文档中心
- 最后更新：2026-10-11
- 状态：🟡 持续更新（2026-10-11 同步 v0.9 第六步（**内核侧最小动态链接**：`PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`）实现记录；同日已同步第五步基础用户程序、第四步简化网络栈、第三步进程对象、第二步交互式 Shell、第一步用户堆、v0.8 收官与 H-06 包管理器 `.ckc` 提案 P0a；同日新增 **desktop.md**（v1.0 之后的桌面环境兼容路线，规划）；同日新增 **pure-kernel.md**（纯内核化改造规划：现状审计 + 逐文件迁移清单 + 六阶段 Cargo/构建/验收规格）；同日推进 **纯内核化阶段三**：新增 `abi/` crate（系统调用 ABI 单一事实源，内核 18 文件 + libc 45 常量改为引用它）；
同日**执行其中阶段二/四/五（部分）**：`libc/`+`user/` 移入 `userland/` 独立 workspace、内核构建按 `manifest.txt` 内嵌（无用户态也能构建与启动）、新增 `tools/check-boundary.py` 边界门禁；同日新增 **storage.md**（CSS 存储管理规划：卷模型 / 盘符与挂载 / VFS 收口 / 与 CKM 的存储接口））

> 本文档顶部保留阶段/最后更新/状态字段，与项目文档维护约定对齐。

> 项目代号：**Cerlesse**（仓库：Cereals-Kernel-Cerlesse）
> 定位：使用 Rust 从零开发的类 Linux 高性能宏内核，支持 Linux / Windows 主机交叉开发（x86_64 优先）。
> 语言策略：**Rust 编写内核核心**，Python 用于构建/测试工具链，Go 用于主机侧辅助服务。

## 文档索引

| 文档 | 内容 | 状态 |
| ---- | ---- | ---- |
| [architecture.md](./architecture.md) | 第一阶段：内核架构、语言分工、目标与非目标 | ✅ 已完成（会随开发反复修订） |
| [lang-selection.md](./lang-selection.md) | 按功能领域汇总的性能最佳语言对照（辅助选型参考） | ✅ 新增（2026-10-07） |
| [package-manager.md](./package-manager.md) | 面向内核/系统生态的高效、安全、轻量、灵活包管理器设计提案（H-06） | 🟡 提案 v1 + **P0a/P0b 已落地**：包容器后缀 **`.ckc`**（CKC，64 字节头 + 确定性 ustar，身份 = payload 哈希，`userland/pkgmgr/ckc.py`）+ **事务引擎**（`userland/pkgmgr/cpm.py`：回执/事务日志/stage→commit/回滚/`recover`/`gc`/单写者锁/失败注入）；验收 `make test-pkg` = `test-ckc` 37 + `test-cpm` **63** 项；catalog 签名（阻塞于开放问题 13.1）与内核侧接入（P1）待做，见该文档 11.1 |
| [ckm.md](./ckm.md) | **Cerlesse Kernel Manager（CKM）设计提案**：权限模型（单管理员 + 能力位 + 判定点）、内核接口（`kernel/src/ckm/` + 扩展号段 + 定长记录）、扩展生命周期与覆盖层、事务/恢复/安全模式、CKM-1…CKM-7 分期与故障验收 | 🟡 **CKM-1/CKM-2/CKM-3 已落地**（内核：`kernel/src/ckm/` 七模块 + 覆盖层 + 双槽 A/B 状态区，syscall 257–259，`make test-ckm` 40 断言 + `make test-ckm-store` 跨重启；**CKM-3 事务引擎在主机侧**：`userland/pkgmgr/cpm.py`，`make test-cpm` 63 项）；CKM-4…CKM-7 规划中 |
| [storage.md](./storage.md) | **Cerlesse Storage System（CSS）设计提案**：卷模型（设备→分区→卷→盘符/挂载点 + 稳定 `vol_id`）、Windows 式盘符与 Linux 式挂载双视图、VFS 收口（`FileSystem` 增量 + 挂载表带卷身份 + 错误语义改 errno）、与 CKM 的接口（2 个能力位 + 判定点 8–10 + 号段 261–266）、文件系统支持路线与第三方依赖政策、后端装载政策（第一版编译内置，6.8）、CSS-1…CSS-6 分期与 11 条验收 | 🟡 **CSS-1/CSS-2 已落地，CSS-3 大部分落地**（`kernel/src/storage/` + `storage_volumes`(261)/`storage_rescan`(265) + `mkfs` GPT/MBR 夹具 + 挂载表带卷身份/多挂载点/卸载 + 挂载错误改 errno + 盘符表 `storage/letters.rs` + `storage_mount`(262)/`unmount`(263)/`assign_letter`(264) + `libc` 的 `C:\` 重写 + `make test-storage`），CSS-4…CSS-6 规划中 |
| [project-structure.md](./project-structure.md) | 目录结构与模块职责划分 | ✅ 已完成 |
| [../DEVELOPMENT.md](../DEVELOPMENT.md) | 主开发文档：需求、依赖、任务清单、测试、QEMU、Issue 规划 | ✅ 已完成 |
| [api.md](./api.md) | **API 参考**：系统调用 ABI / errno / `libc` API / 内核内部 trait / 主机工具 CLI / Make 目标 | 🟡 持续更新（2026-10-11 新建） |
| [software-development.md](./software-development.md) | **软件开发文档**：环境与工具链、构建/测试/调试、代码与文档规范、协作流程、质量门禁、扩展步骤 | 🟡 持续更新（2026-10-11 新建） |
| [boot.md](./boot.md) | 阶段：Bootloader / UEFI / Kernel Entry / GDT / IDT | ✅ v0.1 + v0.2 已完成 |
| [memory.md](./memory.md) | 阶段：物理/虚拟内存 / Heap / Allocator | ✅ v0.3 已完成 |
| [drivers.md](./drivers.md) | 阶段：驱动框架 / PCI / 磁盘（含 v0.4 中断/时钟/键盘/上下文切换） | ✅ v0.8 已完成（驱动注册框架 + PCI 枚举 + 块设备抽象 + **legacy virtio-blk**，`make test-driver` 23 项 PASS；启动盘已改为 virtio-blk）；**v1.0 驱动兼容性：AHCI/SATA 驱动已落地**（`make test-ahci` 24 项 PASS，见 1.9；并修 PCI 枚举缺陷 B-18）；v0.4（PIC + PIT 100Hz + PS/2 键盘 + 上下文切换原语）已完成；v0.9 键盘加**行缓冲 + 终端式回显**（`make test-keyboard` 断言同步）；**v0.9 第四步：简化网络栈（virtio-net + Ethernet → ARP → IPv4 → UDP）已落地**（`make test-network` 13 项，见 1.8）；**v1.0 TCP/socket 补齐**（内核 TCP 客户端最小集 + `socket`/`bind`/`connect`/`sendto`/`recvfrom` + `/bin/tcpecho`，`make test-network` 13 → **23 项**，见 1.10） |
| [process.md](./process.md) | 阶段：进程/线程 / 上下文切换 / 调度器 / IPC | ✅ v0.5 已完成（线程模型 + 上下文切换 + Round Robin 时间片调度 `make test-scheduler`；IPC pipe 骨架 `make test-ipc`）；channel / shared_memory 仍为占位；v0.6 追加了 `scheduler::suspend()`；**v0.9 第三步：进程对象与 `fork`/`execve`/`wait4` 已落地**（每进程地址空间 + 每进程 64 KiB 内核栈 + 完整用户帧，`make test-proc`，见 1.4） |
| [syscall.md](./syscall.md) | 阶段：系统调用 / 用户空间 / ELF | 🟡 v0.7 核心已完成（`syscall`/`sysret` 入口 + ring3 隔离 + `read/write/open/close`+ `write/exit/getpid/sleep` + **ELF Loader** 从 RAMFS `/init` 按段权限装载），v0.9 已落地六步：**用户堆**（3.5）、**`stdin` + 交互式 Shell**（3.6）、**`fork`/`execve`/`wait4` + `argc`/`argv`**（3.7，外部程序 `/bin/hello` 真的能跑）、**基础用户程序 + `getdents`/`getprocs`**（3.8，`/bin/{ls,cat,echo,ps}`）、**内核侧最小动态链接**（3.9，`ET_DYN` + `PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`，`/bin/dynhello` + `/lib/*`）；**v1.0 第一步：`mprotect`**（3.10，已映射页的保护属性运行期改写 + `PROT_NONE`，W^X 的「先写后换只执行」）；**v1.0 第三步：Socket/TCP 补齐**（3.13，`socket`/`bind`/`connect`/`sendto`/`recvfrom` 41/42/44/45/49 + `/bin/tcpecho`）；`make test-user` 96 项 / `make test-shell` 11 项 / `make test-proc` 8 项 + 4 正则 / `make test-programs` 33 项 + 7 正则 / `make test-dyn` 10 项 PASS；仍未做：环境变量/文件映射（`mmap` 加 offset）/**完整**动态链接（见 3.4/3.9/3.10） |
| [filesystem.md](./filesystem.md) | 阶段：VFS / RAMFS / 磁盘文件系统 | ✅ v0.7 + **v0.8 磁盘后端**已完成（VFS trait + 挂载表最长前缀路由 + RAMFS + fd 表；**FAT12/16/32 只读后端** + `tools/mkfs/mkfs.py`，`make test-fs` 21 项 / `make test-fat` 21 项 PASS） |
| [desktop.md](./desktop.md) | **桌面环境兼容路线（v1.0 之后，规划）**：现状审计、用户态/内核侧归属、D0–D5 分层里程碑与 QEMU 验收 | 📋 规划（2026-10-11 新增） |
| [pure-kernel.md](./pure-kernel.md) | **纯内核化改造规划与执行记录**：现状审计（C1–C9，含实测证据）、逐文件迁移清单、内核/用户态双仓库划分、六阶段（Cargo 配置 / 构建目标 / 验收）、七条验收标准对照、非目标 | 🟡 执行中（阶段二/三/四 ✅，2026-10-12 更新） |

## 阶段说明

- **第一阶段（已完成）**：确定架构方向、总体路线、里程碑、目录结构，并创建代码文件框架占位。
- **第二阶段（v0.1–v0.2）**：Boot ✅、GDT/IDT/异常 ✅（见 boot.md 实现记录）。
- **第三阶段（v0.3）**：物理/虚拟内存 + 内核堆 ✅（见 memory.md）。
- **第四阶段（v0.4）**：中断控制器 + Timer + 键盘 + 上下文切换原语 + H-04/H-05 Go 工具 ✅（见 drivers.md 第 1 节）。
- **第五阶段（v0.5）**：进程/线程 + Round Robin 调度 + IPC（pipe 骨架）✅（见 process.md）。
- **第六阶段（v0.6–v0.7）**🟡：v0.6（syscall 入口 + ring3 隔离 + 用户态脚手架）与
  v0.7（VFS/RAMFS/fd 表 + 文件 syscall + ELF Loader）均已实现并验收，见 syscall.md 与 filesystem.md。
- **第七阶段（v0.8）**✅：驱动注册框架 + PCI 枚举 + 块设备（legacy virtio-blk）已实现并验收（`make test-driver` 23 项），
  见 drivers.md；**v1.0 第二步再接上 AHCI/SATA 驱动**（真机磁盘控制器，`make test-ahci` 24 项，并修 PCI 枚举缺陷 B-18），磁盘文件系统（**FAT12/16/32 只读后端**）已挂到现有 VFS，H-03 `mkfs` 同步落地（`make test-fat` 21 项），
  见 filesystem.md。之后的 v0.9：Shell + 用户程序 + 网络。
- **第八阶段（v0.9 开局）**🟡：第一步 **用户堆**（`brk`/`mmap`/`munmap` + `libc` 的
  `#[global_allocator]`）已落地并验收（见 syscall.md 3.5）；第二步 **`stdin` + 交互式 Shell**
  已落地并验收 —— 键盘行缓冲 → `read(0)` 整行 → `/bin/sh`（`make test-shell` 11 项，见 syscall.md 3.6）；
  第三步 **进程对象 + `fork`/`execve`/`wait4` + `argc`/`argv`** 已落地并验收 ——
  Shell 能真的跑起外部程序（`/bin/hello`，`make test-proc`，见 syscall.md 3.7 与 process.md 1.4）；
  第四步 **简化网络栈**（virtio-net + Ethernet → ARP → IPv4 → UDP，`make test-network` 13 项，见 drivers.md 1.8）已落地；
  v1.0 **TCP/socket 补齐**（内核 TCP 客户端最小集 + 五个 socket 系调用 + `/bin/tcpecho` 回显往返，
  `make test-network` 13 → **23 项**，见 drivers.md 1.10 与 syscall.md 3.13）已落地；
  第五步 **基础用户程序**（`ls`/`ps`/`cat`/`echo` → `/bin/*`，含新增的目录枚举 `getdents` 与进程表 `getprocs`，
  `make test-programs` 33 项 + 7 正则，见 syscall.md 3.8）已落地；
  第六步 **内核侧最小动态链接**（`ET_DYN` + `PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`，
  `make test-dyn` 10 项 + `make dyn-hostcheck` 宿主 glibc 独立复核，见 syscall.md 3.9）已落地 —— ✅ 六步全部完成。
- **v1.0 第一步**✅（2026-10-11）：**`mprotect`** —— 已映射页的保护属性可在运行期改写
  （`PROT_NONE` 关页不丢帧、`RX` 转换后真的执行，见 syscall.md 3.10；`make test-user` 91 项）；
- **v1.0 第三步**✅（2026-10-14）：**Socket/TCP 补齐** —— 内核 TCP 客户端最小集
  （三次握手/收发/关闭）+ `socket`/`bind`/`connect`/`sendto`/`recvfrom`（41/42/44/45/49）+
  `/bin/tcpecho` 回显往返（见 syscall.md 3.13；`make test-network` 13 → **23 项**，
  `make test-user` 91 → **96 项**）；ICMP（ping）仍待落地。
  **完整**动态链接（用户态 `ld.so`）、文件映射（`mmap` 加 offset）、用户/组/权限等仍归后续
  （见 architecture.md 第 5 节）。
- **第九阶段（v1.0 之后）**📋：**桌面环境兼容**（内核侧地基与接口对齐，D0 地基 → D1 输入 →
  D2 显示 → D3 音频 → D4 会话/电源 → D5 收尾）—— 规划见 [desktop.md](./desktop.md)，未开工。
- **第十阶段（v0.9 之后；执行上排在第九阶段之前）**🟡：**纯内核化边界收敛** —— 内核只提供底层机制，
  Shell / init / libc / coreutils 已拆到独立 workspace [`userland/`](../userland/README.md)；
  六阶段中 **阶段二（边界门禁）✅、阶段三（`abi/` 单一事实源）✅、阶段四（用户空间迁移）✅、
  阶段五（构建/测试拆分）🟡**；阶段一（基线 tag）与阶段六（接口冻结 / 移植文档）未做。
  实测：无 `userland/` 的 checkout 里 `cargo build -p kernel` 退出码 0（改造前 101）、
  `make test-kernel-only` 7/7 PASS、`make abi-check` 7/7（内核与 libc 的调用号/errno/记录布局
  只有一个定义点，C6 消除）。
  规划与执行记录见 [pure-kernel.md](./pure-kernel.md)。
- **第十一阶段（v1.0 之后）**🔨：**CKM —— 内核级权限控制与扩展管理**（CKM Core 在内核 /
  CKM Service 与 Manager 在用户态 / 可插拔覆盖层 / 独立恢复环境），分 CKM-1…CKM-7；
  第一版不做动态内核模块装载与 GUI —— 见 [ckm.md](./ckm.md)。
  **CKM-1（内核接口）与 CKM-2（可写持久化 + 覆盖层机制）已落地**（2026-10-12）：能力位判定/审计环/组件状态查询、
  系统盘尾部 1 MiB 的双槽 A/B + CRC32 状态区、覆盖层注册表与路径改写钩子（`fs::mount::resolve`）、
  `make test-ckm`（40 断言）与 `make test-ckm-store`（写 → 重启 → 读回）。
- **第十二阶段（v1.0 之后，与第十一阶段同期）**🔨：**CSS —— Cerlesse 存储管理系统**（磁盘/分区识别 → 卷与稳定标识 → 盘符与多挂载点 → 可写文件系统 → `ckm storage` 管理程序），分 CSS-1…CSS-6；第一版不写分区表、不做 RAID/LVM/加密卷/GUI。**CSS-1（底层识别）与 CSS-2（VFS 收口）已落地，CSS-3（盘符）大部分落地**（GPT/MBR 只读扫描 + 卷记录 + `storage_volumes`/`storage_rescan` + 挂载表带卷身份/多挂载点/卸载 + 挂载错误改 errno + 盘符表/`storage_mount`/`unmount`/`assign_letter` + `libc` 的 `C:\` 重写），CSS-3 剩余（`/storage/**` 状态树、卷标写/改名）与 CSS-4…CSS-6 未开工 —— 见 [storage.md](./storage.md)。
- **后续**：每进入一个新阶段就编写/修订对应的主题文档；阶段内已落地部分也要及时补记当前状态（例如 v0.4 中断控制器）。
- 本套文档**会在开发过程中被反复修改**——修改前请在对应文档顶部记录修订日期与变更点。-  本次（2026-10-12，第二十轮）修订重点：**纯内核化阶段三：`abi/` 单一事实源落地** ——
  新增 `abi/` crate（`no_std`、零依赖、`forbid(unsafe_code)`）：`syscall`（号 + `SYS_MAX` + 号段规则 +
  `NAMED`/`UNIMPLEMENTED`）、`errno`、`flags`（`O_*`/`PROT_*`/`MAP_*`/fd）、`limits`（`PATH_MAX`/`IO_MAX`/`ARG_*`）、
  `dirent`/`procinfo`/`volume`/`ckm`（记录布局、能力位、op 编号、受保护前缀）与 `record`（编译期断言）；
  内核 18 个文件（`syscall/*` 9 + `fs/file.rs` + `security/capability.rs` + `ckm/{mod,audit,state,policy}.rs` +
  `storage/volume.rs` 等）与 `libc/src/syscall.rs` 的 45 个常量改为引用它（**对外名字与类型不变**，行为零变化）；
  新增 `make abi-check` 并并入 `make check`。实测：两处构建退出码 0、`abi-check` 7/7、`check-boundary` 6/6、
  14 个 QEMU 目标全 PASS；反例：改 `SYS_GETPROCS` → `abi-check` 红（退出码 2）、单侧改 `DIRENT_SIZE` →
  `test-programs` 红（`ls` 条目找不到）。同步 pure-kernel.md（C6 → ✅、新增 8.2）、syscall.md、
  project-structure.md、DEVELOPMENT.md（§3/§5 表）、README.md 与 Makefile。
-  本次（2026-10-12，第十九轮）修订重点：**CKM-2（可写持久化 + 覆盖层机制）落地** —— 新增
  `kernel/src/ckm/store.rs`（系统盘尾部 1 MiB 的双槽 A/B + CRC32 状态区，开放问题 1 定夺为 (b)）与
  `kernel/src/ckm/overlay.rs`（注册/撤销/最长前缀/优先级/冲突拒绝，钩子在 `fs::mount::resolve`），
  `policy::reject`（语义拒绝）与 `fs::overlay_selftest`；`tools/image-builder/build_image.py` 改为
  「先在 63 MiB 上 `mformat`、再追加 1 MiB 保留区」（设备仍 64 MiB，FAT 看不到状态区）；
  新增 `make test-ckm-store` + `tests/test_ckm_store.py`（两阶段启动：写 → 重启 → 读回）与编排场景
  `ckm-store`（共 19 个场景），`test-ckm` 扩到 40 断言，`test-fat`/`test-programs`/`test-storage` 的
  FAT 几何与根目录条目数断言同步；ckm.md 新增 9.2 落地清单与开放问题 1 的定夺。
-  本次（2026-10-11，第十七轮）修订重点：**新增 CSS（Cerlesse Storage System）存储管理提案（规划，不写代码）** —— 新建 `storage.md`：
  定稿**卷模型**（四层：设备→分区→卷→挂载点/盘符；`Volume` 记录；`vol_id` 三条确定性生成规则：
  文件系统 UUID/序列号 > `pt:<disk>-<part>-<lba>` > `raw:<disk>`；只读原因与健康状态各成枚举；卷表容量 16、每卷 ≤2 挂载点），
  **路径/盘符/挂载**（`C:\x` 与 `/volumes/x` 双视图指向同一文件；Windows 式路径由用户态 `libc` 查卷表后重写，
  **内核 syscall 只收 POSIX 路径**；默认卷 `/volumes/<label>`、可移动卷 `/media/<label>`；错误语义追加 `ENODEV`/`ENOTSUP`），
  **VFS 收口与 CKM 接口**（`FileSystem` 只加 `caps`/`volume_info`/`probe` 三个只读方法、挂载表带 `vol_id`/`options`、
  挂载失败从 `&'static str` 改 errno、`kernel/src/storage/` 六模块 + `syscall/storage.rs`；
  CKM 侧 2 个能力位 + 判定点 8–10 + 事件 12–16 + 号段 **261–266**（`SYS_MAX` → **267**）+ 卷 64 字节/分区 48 字节定长记录 + `/storage/**` 状态树 + 串口证据格式）；
  另有文件系统支持路线（FAT→exFAT→ext2 优先，ext4/NTFS 只读优先）与第三方依赖政策（现状零依赖且被 K4 门禁强制）、
  写入语义五条、CSS-1…CSS-6 分期与 11 条验收（含宿主工具实测：有 `blkid`/`mtools`/`mkfs.ext2`，**没有** `sfdisk`/`parted`/`mkfs.exfat`）、
  非目标与 9 个开放问题；同步 ckm.md（能力位表扩到 6 位、判定点 8–10、事件 12–16、号段说明与 CKM-2 硬约束）、
  DEVELOPMENT.md（H-08 行 + 5.13）、roadmap（第十二阶段）、architecture 第 5 节、project-structure（规划目录）、
  README（文档入口）、tools/README.md（`mkfs` 规划分区表生成）与 filesystem.md（交叉引用）；
  同日复核补：**6.8 文件系统后端装载政策**（第一版编译内置、不经 CKM 运行时装卸）、6.1 覆盖层/回滚/恢复归 CKM、
  6.7「兼容性 = `fs=`+`ro=`+`health=` 完整展示」、第 11 节「不做写回修复」（`fsck` 只读检查）；
  随后 **CSS-1（底层识别）落地**：新建 `kernel/src/storage/`（GPT/MBR 只读扫描 + 卷记录 + 解析自测）、
  `storage_volumes`(261)/`storage_rescan`(265)、`mkfs` GPT/MBR 夹具、`libc` 解析器与 `/bin/volumes`、
  `make test-storage` 系列与编排场景 `storage`/`storage-mbr`（15 → 17 场景），见 storage.md 第 9/10 节。
  随后 **CSS-2（VFS 收口）落地**：`FileSystem::caps`/`volume_info`（保守缺省；RAMFS 可写 / FAT 只读 +
  大小写不敏感 + 卷标）；FAT 探测收归 `fs/fat.rs`（`probe`/`probe_sector`，删掉 `storage/volume.rs` 的重复 BPB 判定）、
  `FatFs::open` → `OpenError { errno, reason }`；挂载表拆「后端 / 挂载点」并带 `vol_id`/`options`
  （`alias_volume`/`umount`，挂载计数写回卷记录偏移 56）；`fs: stat` 行尾 `vol=`；
  新增 `fs::storage_mount_selftest` 开机自测（只读原因来自能力位、`storage: volcheck` 后端与记录一致、
  同一卷多挂载点同字节、卸载后 `ENOENT` 且卷仍在表、`ENOENT`/`EINVAL`/`EEXIST` 反例）；
  `tests/test_boot.py` 增 `--expect-re`（反向引用绑同一 `vol_id`，因为启动盘序列号随镜像构建变化）；
  `make test-storage`（31 项 + 2 正则）、`test-storage-mbr`（14 项 + 1 正则），
  编排场景 `fs`/`fat`/`storage`/`storage-mbr`/`programs` 全 PASS。
-  本次（2026-10-11，第十四轮）修订重点：**纯内核化执行阶段二/四/五（部分）** —— 新增
  `tools/check-boundary.py`（6 项边界门禁 + `--selftest`，并入 `make test-all`）；`libc/`+`user/` 移入
  `userland/` 独立 workspace（自带 `Cargo.toml`/`.cargo/config.toml`/`Makefile`/`README.md`，产出
  `build/userland/images/manifest.txt`）；`kernel/build.rs` 改为按清单内嵌（内核不再需要 binutils），
  `syscall/user.rs` 收敛为一次 `include!` + 契约校验 + 空清单诊断分支，`fs::selftest` 根目录条目数改为推导；
  新增 `userland-images`/`image-kernel`/`kernel-nouserland`/`test-kernel-only`；纯内核 checkout 实测
  `cargo build -p kernel` 退出码 **0**（改造前 101）、`make test-kernel-only` 7/7 PASS；同步
  pure-kernel.md（第 8 节执行记录）、project-structure（目录树）、DEVELOPMENT（5.11）、roadmap 与 README。
-  本次（2026-10-11，第十三轮）修订重点：**新增纯内核化改造规划** —— 新建 `pure-kernel.md`
  （现状审计 C1–C9：内核构建期嵌套 cargo 构建用户程序、10 处 `include_bytes!` 把 203,280 字节（≈199 KiB）用户态内嵌进内核映像、
  `fs::init` 硬编码 `/bin`/`/lib`、启动末尾跑用户态自测；**逐文件迁移清单**（`kernel/` 83 个文件里只有 9 个要改，
  `user/` 33 + `libc/` 7 + `tools`/`tools-go`/`tests` 各文件逐项去向）；**六阶段**：冻结基线 / 边界门禁 /
  ABI 单一来源（新增 `abi/`）/ 用户空间迁移（feature `embed-userland`） / 构建与测试拆分 / 接口稳定；
  七条「纯内核化」验收标准对照表；实测证据：删掉 `libc/`+`user/` 后 `cargo build -p kernel` 退出码 **101**）。
  同步 roadmap.md（第十阶段）、DEVELOPMENT.md（5.11）、project-structure.md（第 4 节目标结构）与 README.md。
-  本次（2026-10-11，第十六轮）修订重点：**新增 CKM 设计提案（规划，不写代码）** —— 新建 `ckm.md`：
  定稿**权限模型**（单管理员模型 + 4 个能力位 + 7 个判定点；凭证**只能**由 CKM 组件表在 `execve` 时改变，
  因此没有 `su`/`setuid` 路径）、**内核接口**（`kernel/src/ckm/` 五模块分工、`CKM_ABI_VERSION = 1`、
  扩展号段 257–259 与 `SYS_MAX` 257 → 260、审计/组件状态两份定长记录、串口证据行格式、事件枚举）、
  **扩展生命周期**（四类扩展、八态状态机与合法转换表、确定性枚举与加载顺序）；另含与 H-06 的分工
  （`.ckc` 不重做、P0b = CKM 阶段 3、`cpm` 合并成 `ckm pkg`）、覆盖层与 KernelSU 元模块的对应关系、
  安全模式/恢复环境、CKM-1…CKM-7 分期、8 条故障场景落成断言、非目标与 7 个开放问题；
  同步 DEVELOPMENT.md（H-07 行 + 5.12）、roadmap（第十一阶段）、architecture 第 5 节、project-structure
  （规划目录）、README（文档入口）。
-  本次（2026-10-11，第十五轮）修订重点：**v1.0 第一步（`mprotect`）落地 + 补齐 `LICENSE`** ——
  `kernel/src/syscall/memory.rs::sys_mprotect`（页对齐/`len>0`/未知位/W^X 四类拒绝各留串口原因）、
  `address_space.rs::mprotect` + `UserWindow` + `prot_to_flags` + `accessible`（`PROT_NONE` = 清 `USER`
  留 `PRESENT`、每次改权限 `invlpg`）、堆窗口逐页 `heap_flags`（`fork`/`brk` 同步）、`mmap(PROT_NONE)` 合法化；
  `syscall_table` 注册 10 号并把 `UNIMPLEMENTED` 清空；`libc` 新增 `mprotect`/`PROT_NONE`，
  `userland/user/hello` 新增 `mprotect_selftest`（`PROT_NONE` → 内核拒收 `-EFAULT` → 内容原样 →
  `RX` 后真的执行机器码，另含 brk 堆窗口的 `PROT_NONE` 往返）；`make test-user` 61 → **91 项** PASS；`kernel/build.rs` 在「无清单」分支
  登记默认 `manifest.txt` 为构建输入（修掉「先编内核后出镜像 → 一直无用户态」的缓存陷阱）；
  仓库新增 **`LICENSE`（MIT）**（README 许可证一节同步）；同步 syscall.md（新增 3.10）、api.md
  （2.6 + 节号顺移 + `test-user` 84）、roadmap.md、DEVELOPMENT.md（§5.7）、userland/libc/README.md、Makefile。
-  本次（2026-10-11，第十二轮）修订重点：**新增桌面环境兼容路线（规划）** —— 新建 `desktop.md`
  （现状审计 + 用户态/内核侧归属纠正 + D0–D5 分层 + v1.1–v1.6 里程碑）；同步 architecture.md 第 5 节、
  roadmap.md（第九阶段 + 「禁止提前做」项修正）、DEVELOPMENT.md（5.10）、project-structure.md（规划新增目录）
  与 README.md。
-  本次（2026-10-11，第十一轮）修订重点：**v0.9 第六步（内核侧最小动态链接）落地** ——
  新增 `kernel/src/syscall/dynamic.rs`（`ET_DYN` 分流：`PT_INTERP` + `DT_NEEDED` → `/lib/<name>` 递归装载，
  应用 `R_X86_64_RELATIVE`/`GLOB_DAT`/`JUMP_SLOT`/`64`，`DT_PLTREL=REL` 明确拒绝）与
  `address_space.rs` 的共享库窗口（基址 + 1 MiB、32 页）；新增汇编夹具 `user/dynhello/`
  （`as`+`ld` 现编，`kernel/build.rs` 内嵌）与 RAMFS `/lib`（构建/内嵌清单 7 → 8）；
  新增 `make test-dyn`（10 项）与 `make dyn-hostcheck`（宿主机 glibc 独立复核）；
  `test-fs` 20 → 21 项、`test-programs` 条目数 5/7；同步 syscall.md（新增 3.9，修正 3.1/3.4/3.8）、
  architecture.md 第 5 节、roadmap/DEVELOPMENT/README/project-structure/api/filesystem。
-  本次（2026-10-11，第十轮）修订重点：**v0.9 第五步（基础用户程序与目录/进程枚举）落地** ——
  新增 `syscall/dir.rs`（`getdents`：VFS `list()` → 定长 88 字节记录，号 78 沿用 Linux 但格式不同）与
  `syscall/process.rs::sys_getprocs`（定长 40 字节记录，号 **256**）；`syscall_table.rs` 确立
  「`0..=255` Linux 语义 / `>= 256` Cerlesse 扩展」并把 `SYS_MAX` 改 257；`process/process.rs` 新增定长 `comm`
  （`execve` 改写为程序名）；`libc` 新增 `getdents`/`getprocs` + `dirent_*`/`procinfo_*` 解析器；
  新增 `user/{ls,cat,echo,ps}` → RAMFS `/bin/*`（构建/内嵌清单 3 → 7）；新增 `make test-programs`（17 项 + 5 正则）
  与编排 `programs` 场景（13 → **14 场景**）；`make test-proc` 负例由 `ls` 改 `foo`；
  同步 syscall.md（新增 3.8、3.6 未做项修正与修订记录）、project-structure.md、roadmap.md、DEVELOPMENT.md、README.md。
-  本次（2026-10-11，第九轮）修订重点：**v0.9 第四步（简化网络栈）落地** ——
  新增 `driver/net.rs`（`NetworkDevice` trait + 注册表）与 `driver/virtio_net.rs`（legacy virtio-net 0.9.5：
  RX/TX 双队列、10 字节 virtio-net 头、MAC 读自设备配置空间、同步轮询），`network/{ethernet,arp,ipv4,udp,dhcp}.rs`
  与 `network/mod.rs`（`Stack` + `selftest`：Ethernet II → ARP → IPv4 → UDP 的组帧/解析与校验和）；
  `driver::register_builtin` 追加 virtio-net、`main.rs` 在磁盘验收后接入网络自测（**无网卡时跳过**）；
  新增 `make test-network`（13 项，QEMU 加 legacy virtio-net-pci + `-netdev user` = slirp）与编排 `network` 场景
  （12 → **13 场景**）；`tests/test_boot.py` 增 `--nic`；同步 drivers.md（新增 1.8 节 + 状态行 + 驱动列表/限制）、
  roadmap.md、DEVELOPMENT.md、README.md、project-structure.md。
-  本次（2026-10-11，第八轮）修订重点：**v0.9 第三步（进程对象 + `fork`/`execve`/`wait4`）落地** ——
  进程表从「最小登记」换成真正的 PCB（每进程地址空间 + 每进程 64 KiB 内核栈 + 栈底哨兵
  + 停在 syscall 里的完整用户帧）、新增 `syscall/process.rs`（fork 复制地址空间与当前帧、
  execve 换映像并重建 argv 用户栈、wait4 延迟调度 + 回收）、`trampoline` 改为按进程切换；
  Shell 的外部命令改走 `fork`+`execve`+`wait4`，新增外部程序 `/bin/hello`（`user/hello-bin`）；
  新增 `make test-proc`（8 项 + 4 条正则，含 PID 一致性反向引用）与编排 `proc` 场景（11 → **12 场景**）；
  同步 syscall.md（新增 3.7、更新 3.2/3.3/3.4/3.6/4）、process.md（新增 1.4，1.3 阻塞点关闭）、
  libc/README（新增进程调用与 `argc`/`argv` 一节）、project-structure/roadmap/README/DEVELOPMENT。
-  本次（2026-10-11，第七轮）修订重点：**v0.9 第二步（`stdin` + 交互式 Shell）落地** ——
  键盘行缓冲 + 终端式回显、`read(0)` 整行交付（3 秒 `-EAGAIN`）、`user/shell` → RAMFS `/bin/sh`
  （提示符 `root@cerlesse C:>`、内建命令、外部命令真调 `fork()` 暴露 ENOSYS）、新增 `make test-shell`（12 项）
  与编排 `shell` 场景（11 场景）；同步 syscall.md（新增 3.6、3.2/3.3/3.4）、process.md（1.3 阻塞点）、
  drivers.md（键盘行缓冲）、project-structure/roadmap/README/DEVELOPMENT。
-  本次（2026-10-12，阶段五）修订重点：**构建与测试按责任拆分** —— 归属用户态的主机工具全部迁出内核仓库（`tools/pkgmgr/` + `tests/test_{ckc,cpm}.py` → `userland/pkgmgr/`，`tools/check_user_elf.py` → `userland/tools/`），用户态侧新增 `make -C userland test-pkg`/`test-all`；内核 `make test-all` 收敛为纯内核 18 目标，新增 `test-all-with-userland`/`test-orch-userland`，编排场景用 `tags` 分组（`testorch -tag`/`-exclude-tag`）；详见 [pure-kernel.md](./pure-kernel.md) 8.3 与 [../userland/pkgmgr/README.md](../userland/pkgmgr/README.md)。
-  本次（2026-10-11，第六轮）修订重点：**H-06 P0a（`.ckc` 包容器）落地** —— 内核包后缀定名为
  `.ckc`（CKC = Cerlesse Kernel Container）：64 字节头（magic/版本/头长/flags/长度/偏移/
  32 字节 payload_sha256/保留位）+ 确定性 ustar（归档名 = 目标路径去 `/`、`PKG.toml` 第一、
  按字节序、mtime/uid/gid 全 0、仅常规文件）；身份 = payload 的 SHA-256；
  实现 `userland/pkgmgr/ckc.py`（`pack`/`verify`/`info`；阶段五迁至用户态 workspace）、新增 `make test-pkg`（37 项，含系统 `tar` 独立复核）；
  同步 package-manager.md 3.2/11.1/12.1、DEVELOPMENT.md（H-06 行 + 测试方案）、tools/README.md、
  project-structure.md、README.md。
-  本次（2026-10-11，第五轮）修订重点：**v0.9 第一步（用户堆）落地** ——
  `kernel/src/syscall/memory.rs`（`brk`/`mmap`/`munmap`，W^X + 全零页 + `invlpg`）、
  `address_space.rs` 的堆窗口与匿名 mmap 窗口、`libc/heap.rs`（`#[global_allocator]`）、
  `user/hello` 堆自测与 `user.rs` 页帧泄漏检查；`make test-user` 断言 32 → **59 项**，
  同步 syscall.md（新增 3.5、2.7 表、地址布局）、roadmap/DEVELOPMENT/README/project-structure/libc README。
-  本次（2026-10-11，第四轮）修订重点：**H-06 包管理器提案 v1 完成** ——
  package-manager.md 从「设想」补成可实施规格（七个制品格式、内容哈希 + 三级校验、
  **纯文件安装**的安全模型与威胁对策、事务阶段与回滚、scope 路径白名单与到内核现状的映射、
  CLI 与退出码、MVP 三期、可脚本断言的验收条目），索引状态由「设计中」改为「提案 v1（待原型）」；
  DEVELOPMENT.md 的 H-06 行同步（仍为设计阶段，验收与实现属下一轮）。
-  本次（2026-10-11，第三轮）修订重点：**v0.8 收官** —— FAT12/16/32 只读后端（`fs/fat.rs`）挂到 VFS
  与 H-03 `mkfs`（`tools/mkfs/mkfs.py`）落地并通过 `make test-fat`（21 项）验收后，
  更新 filesystem.md（改为 v0.8 实现记录）、drivers.md（限制项移除）、roadmap/project-structure/DEVELOPMENT
  与索引状态（v0.8 标 ✅，编排场景 9 → 10）。
-  本次（2026-10-11，第二轮）修订重点：v0.8 前半（驱动框架 + PCI 枚举 + 块设备 + virtio-blk legacy）
  验收通过后，把 drivers.md 从「v0.8 尚未编写」更新为实现记录（设计取舍、日志证据、限制）
  并在索引/阶段说明里标明「v0.8 前半完成」。
-  本次（2026-10-11，第一轮）修订重点：v0.6（ring3 + syscall 入口 + 用户态脚手架）与
  v0.7（VFS/RAMFS/文件 syscall/ELF Loader）验收通过后，把 syscall.md 与 filesystem.md 从占位
  更新为实现记录并在索引里标明状态。
  更早（2026-10-08）修订重点：v0.4 与 v0.5 验收全部通过后，把索引状态更新为已完成——
  v0.4（内核侧 PIC / PIT 100Hz / PS/2 键盘 / 上下文切换原语 + 主机侧 H-04/H-05 Go 工具）、
  v0.5（线程模型 + Round Robin 时间片调度 + IPC pipe 骨架），与 DEVELOPMENT.md 任务清单一致。

## 文档维护约定

1. 每份文档顶部保留 `最后更新` / `阶段` / `状态` 三行，便于快速辨识（已有文档逐步对齐）。
2. 设计发生变更时，先改文档再改代码（文档即规格）。
3. 未开始的阶段文档保留占位大纲，不提前编写细节；但阶段内已落地部分应及时补记当前状态，避免文档与代码库状态脱节。
4. 本项目文档会在开发过程中被反复修改，修改前请在对应文档顶部记录修订日期与变更点。
