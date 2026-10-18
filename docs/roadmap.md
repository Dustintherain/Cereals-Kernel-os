# 总体开发路线图（第一阶段）

- 阶段：第一阶段 / 路线规划
- 最后更新：2026-10-11
- 状态：🟡 持续更新（2026-10-11 补记 v0.6/v0.7 实际进度、v0.8 前半落地与 v0.8 收官、
  **v0.9 六步全部落地**：第一步（用户堆）、第二步（`stdin` + 交互式 Shell）、第三步
  （**进程对象 + `fork`/`execve`/`wait4`**）、第四步（**简化网络栈**：virtio-net + Ethernet → ARP → IPv4 → UDP）、
  第五步（**基础用户程序 `ls`/`cat`/`echo`/`ps`** + `getdents`/`getprocs`）、
  第六步（**内核侧最小动态链接**：`PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`）；
  v0.9 范围内的目标已全部完成；剩余项（`mprotect`、完整动态链接（用户态 `ld.so`）、
  TCP/ICMP 与 socket、更多用户程序）归 v1.0，其中 **`mprotect` 已于 2026-10-11（v1.0 第一步）落地**：
  已映射页的保护属性可在运行期改写（`PROT_NONE` 与 W^X 的「先写后换只执行」，见 syscall.md 3.10）；
  **Socket/TCP 已于 2026-10-14（v1.0 第三步）落地**：内核 TCP 客户端最小集 + `socket`/`bind`/`connect`/`sendto`/`recvfrom`
 （号 41/42/44/45/49）+ `/bin/tcpecho` 回显往返，`make test-network` 13 → **24 项**（见 syscall.md 3.13）；
  ICMP（ping）仍待落地；
  **v1.0 之后新增「桌面环境兼容路线」（规划）**：
  D0 地基 → D1 输入 → D2 显示 → D3 音频 → D4 会话/电源 → D5 收尾，见 [desktop.md](desktop.md)；
  同日再加 **「纯内核化改造规划」（v0.9 之后，规划；执行上在桌面路线之前）**：边界收敛 ——
  `libc/`+`user/*` 迁往独立 Cerlesse Userland，内核提供机制/用户态提供功能，见 [pure-kernel.md](pure-kernel.md)；
  同日再加 **「CKM 设计提案」（v1.0 之后，规划）**：内核级强制权限控制 + 特权用户态服务 + 可插拔扩展系统
  + 独立恢复环境（分层借鉴 KernelSU，但不照搬 Android root 机制），见 [ckm.md](ckm.md)；
  同日再加 **「CSS 存储管理系统」（v1.0 之后，与 CKM 同期）**：磁盘→分区→卷→盘符/挂载点，
  Windows 式盘符 + Linux 式统一 VFS，见 [storage.md](storage.md)；
  **CSS-1（底层识别）与 CSS-2（VFS 收口）已落地，CSS-3（盘符）也于 2026-10-12 大部分落地**
  （GPT/MBR 只读扫描 + `storage_volumes`/`storage_rescan` + 挂载表带卷身份/多挂载点/卸载 + 挂载错误改 errno
  + 盘符表/`storage_mount`/`unmount`/`assign_letter` + `libc` 的 `C:\` 重写 + `make test-storage`），
  CSS-4…CSS-6 与 CSS-3 剩余（状态树/卷标写）规划中）

> 本文档顶部保留阶段/最后更新/状态字段，与项目文档维护约定对齐。

## 1. 总体开发路线

```text
Boot / 启动（Bootloader → CPU初始化 → Kernel入口）
        ↓
Kernel 基础层（GDT / IDT / Exception / Panic）
        ↓
内存管理（Physical → Virtual → Heap → Allocator）
        ↓
中断与时钟（PIC/APIC → Timer → Keyboard）
        ↓
进程 / 线程（Task → Context → Scheduler → IPC）
        ↓
系统调用（syscall → Process → File → Memory）
        ↓
文件系统（VFS → FAT/EXT → File → Directory）
        ↓
驱动系统（PCI → Block → USB → Input → Display）
        ↓
用户空间（ELF → libc → init → shell）
        ↓
网络系统（Ethernet → IP → TCP/UDP → Socket）
        ↓
完整系统（Shell + Utilities + Services + Apps）
```

## 2. 实际开发顺序（严格按依赖关系）

```text
① Bootloader → ② Kernel Entry → ③ Serial Console → ④ GDT/IDT → ⑤ Exception
→ ⑥ Physical Memory → ⑦ Virtual Memory → ⑧ Kernel Heap → ⑨ Interrupt
→ ⑩ Timer → ⑪ Keyboard → ⑫ Task → ⑬ Context Switch → ⑭ Scheduler
→ ⑮ Process → ⑯ Syscall → ⑰ User Mode → ⑱ ELF Loader → ⑲ VFS
→ ⑳ RAMFS → ㉑ Disk Driver → ㉒ PCI → ㉓ Network → ㉔ Shell → ㉕ User Applications
```

**禁止提前做的内容**：GUI / GPU（其地基在桌面路线的 D2 之后，见 [desktop.md](desktop.md)）、USB ——
这些都建立在前面的内核基础设施之上（网络已按计划在 v0.9 落地）。

## 3. 里程碑（v0.1 – v1.0）

| 版本 | 目标 | 对应文档 |
| ---- | ---- | ---- |
| **v0.1 ✅** | Bootloader + Rust Kernel（2026-10-06 QEMU 验收通过） | boot.md |
| **v0.2 ✅** | GDT + IDT + Exception（2026-10-06 验收通过） | boot.md / memory.md |
| **v0.3 ✅** | Physical/Virtual Memory + Heap（2026-10-07 验收通过；高半区映射移至 v0.6，ADR-009） | memory.md |
| **v0.4 ✅** | Interrupt + Timer + Keyboard（2026-10-08 验收通过）；含上下文切换原语 + H-04/H-05 Go 工具（`make go-test` / `make test-orch` 通过） | drivers.md |
| **v0.5 ✅** | Process + Thread + Scheduler + IPC（2026-10-08 验收通过：线程模型 + 上下文切换 + Round Robin 时间片调度 + pipe 骨架，`make test-scheduler` / `make test-ipc` 通过） | process.md |
| **v0.6 🟡** | Syscall + User Space（2026-10-10 核心已验收：`syscall`/`sysret` 入口 + ring3 + 用户地址空间隔离 + `write/exit/getpid/sleep`；2026-10-11 用户态脚手架可用：`libc/` + `user/hello` 真实用户程序跑在 ring3，`make user` / `make test-user` PASS）；ELF Loader / VFS 文件语义 / 进程对象留到 v0.7 | syscall.md |
| **v0.7 🟡** | VFS + RAMFS + ELF（2026-10-11 核心已验收：`FileSystem` trait + 挂载表最长前缀路由 + RAMFS 后端 + fd 表 + `open/read/write/close` + **ELF Loader**（RAMFS `/init` 按段权限逐页装载，`RW` 段对 ring3 可写）；`make test-fs` 19 项 / `make test-user` 32 项 PASS）。Shell 里 `ls/cat/cd` 属 v0.9、磁盘 FS（FAT32）与 `mkfs` 属 v0.8 | filesystem.md / syscall.md |
| **v0.8 ✅** | Driver + PCI + Disk（2026-10-11 验收：驱动注册框架 + PCI 枚举 + 块设备 + **legacy virtio-blk**（`make test-driver` 22 项）；**FAT12/16/32 只读后端挂到 VFS** + H-03 `mkfs`（`make test-fat` 21 项），双盘启动读 `/disk`（启动盘）与 `/mnt/fat`（测试盘）） | drivers.md / filesystem.md |
| **v0.9 ✅** | Shell + User Programs + Network（**六步全部落地**：第一步 **用户堆**（`brk`/`mmap`/`munmap` + `libc` 全局分配器）；第二步 **`stdin` + 交互式 Shell**（键盘行缓冲 → `read(0)` 整行 → `/bin/sh`）；第三步 **进程对象 + `fork`/`execve`/`wait4` + `argc`/`argv`** —— Shell 的外部程序 `/bin/hello` 真的能跑起来（`make test-shell` 11 项 / `make test-proc` 8 项 + 4 正则 / `make test-user` 61 项）；第四步 **简化网络栈**（virtio-net + Ethernet/ARP/IPv4/UDP，`make test-network` 13 项）；第五步 **基础用户程序**（`ls`/`cat`/`echo`/`ps` → `/bin/*`，含新增的目录枚举 `getdents` 与进程表 `getprocs`，`make test-programs` 17 项 + 5 正则）；第六步 **内核侧最小动态链接**（`PT_INTERP` + `DT_NEEDED` + `RELATIVE`/`JUMP_SLOT`，`/bin/dynhello` + `/lib/{ld.so,libgreet.so}`，`make test-dyn` 10 项 + `make dyn-hostcheck` glibc 独立复核）。**仍不做（归 v1.0）：`mprotect`、完整动态链接（用户态 `ld.so`）、TCP/ICMP 与 socket、更多用户程序**） | syscall.md / process.md / drivers.md / filesystem.md |
| **v1.0** | 完整可启动的类 Linux 小型 OS（第一步 ✅ 2026-10-11：**`mprotect`**；第二步 ✅ 2026-10-14：**驱动兼容性 —— AHCI/SATA 驱动 + PCI 枚举修缺陷 B-18**，`make test-ahci` 24 项，见 drivers.md 1.9；第三步 ✅ 2026-10-14：**Socket/TCP 补齐** —— 内核 TCP 客户端最小集 + 五个 socket 系调用 + `/bin/tcpecho`，`make test-network` 13 → **23 项**，见 syscall.md 3.13） | syscall.md / drivers.md |

> **v1.0 之后（规划，执行上排在桌面路线之前）**：CKM（内核级权限与扩展管理）——
> CKM Core（`kernel/src/ckm/`：强制权限、组件状态、审计）→ CKM Service（`ckmd`：事务/包操作/修复）
> → CKM Manager（`ckm` CLI）→ 覆盖层 → 恢复环境；分 CKM-1…CKM-7 七期，第一版不做动态内核模块装载与 GUI，
> 依赖关系：CKM-1 依赖 v1.0 的凭证/权限，CKM-2 依赖可写持久化；与桌面路线互不阻塞，见 [ckm.md](ckm.md)。
>
> **v1.0 之后（与 CKM 同期）**：**CSS（Cerlesse Storage System）存储管理** ——
> 磁盘/分区识别（GPT/MBR **只读**扫描）→ 卷与稳定标识（`vol_id`：文件系统 UUID > 分区位置 > 整盘）→
> Windows 式盘符（`C:`/`D:`）与 Linux 式挂载（`/volumes/<label>`、`/media/<label>`）双视图 →
> 可写文件系统（FAT → exFAT → ext2；ext4/NTFS 先只读）→ `ckm storage` 管理程序；分 CSS-1…CSS-6，
> 第一版不写分区表、不做 RAID/LVM/加密卷/GUI，权限与持久化复用 CKM（能力位 + 判定点 + 可写区），
> 见 [storage.md](storage.md)。
>
> **v1.0 之后（规划）**：桌面环境兼容路线（内核侧地基与接口对齐）——
> D0 地基（设备模型 + `ioctl` + `poll`）→ D1 输入（evdev/uinput）→ D2 显示（fbdev → DRM/KMS）
> → D3 音频（ALSA 子集）→ D4 会话/电源（ACPI 子集 + cgroup）→ D5 收尾；
> 里程碑 v1.1–v1.6 与逐项验收见 [desktop.md](desktop.md)。

每个版本的详细任务清单见 [../DEVELOPMENT.md](../DEVELOPMENT.md)。

## 4. 阶段划分（与文档对应）

1. **第一阶段（已完成）**：架构 + 路线 + 结构 + 文件框架。
2. **第二阶段（v0.1–v0.2）**：Boot、GDT/IDT、异常处理 —— 编写 boot.md（✅）。
3. **第三阶段（v0.3）**：物理/虚拟内存 + 内核堆 —— 编写 memory.md（✅）。
4. **第四阶段（v0.4）**：中断控制器 + Timer + 键盘 + 上下文切换原语（含 H-04/H-05 Go 工具） —— 见 drivers.md（✅）。
5. **第五阶段（v0.5）**：进程/线程/调度 + IPC（pipe 骨架） —— 见 process.md（✅）。
6. **第六阶段（v0.6–v0.7）**：系统调用/用户态/文件系统 —— v0.6（syscall 入口 + ring3 + 隔离 + 用户态脚手架）与 v0.7（VFS/RAMFS/fd 表/文件 syscall/ELF Loader）均已落地并验收，见 syscall.md 与 filesystem.md（均 🟡）；剩余的 `fork/exec/wait`、进程对象、用户堆、磁盘 FS 归入 v0.8+。
7. **第七阶段（v0.8–v0.9）**：v0.8 已完成（驱动注册框架 / PCI 枚举 / 块设备 / legacy virtio-blk + **FAT 磁盘后端挂到 VFS** + `mkfs`），
   见 drivers.md 与 filesystem.md（均 ✅）；v0.9 进行中：用户堆、**`stdin` + 交互式 Shell（`/bin/sh`）**、
   **进程对象 + `fork`/`execve`/`wait4`**（外部程序 `/bin/hello`）与 **基础用户程序**
   （`ls`/`ps`/`cat`/`echo` → `/bin/*`，含 `getdents`/`getprocs`）均已落地
   （见 syscall.md 3.5/3.6/3.7/3.8 与 process.md 1.4）；**简化网络栈**（virtio-net + Ethernet → ARP → IPv4 → UDP，
   见 drivers.md 1.8）亦已落地；最后一块**动态链接**（内核侧最小闭环：`PT_INTERP` + `DT_NEEDED` +
   `RELATIVE`/`JUMP_SLOT`，见 syscall.md 3.9）于 2026-10-11 落地 —— v0.9 至此收官。
8. **第八阶段（v1.0，进行中）**：整合、安全机制、发布 —— 第一步 **`mprotect`**（内存保护属性运行期改写）
   已于 2026-10-11 落地并验收（`make test-user` 91 项，见 syscall.md 3.10 与 DEVELOPMENT.md 5.7）；
   第二步 **AHCI/SATA 驱动 + B-18** 已于 2026-10-14 落地（`make test-ahci` 24 项，见 drivers.md 1.9）；
   第三步 **Socket/TCP 补齐** 已于 2026-10-14 落地（`make test-network` 13 → **23 项**，
   `make test-user` 91 → **96 项**，见 syscall.md 3.13）；ICMP（ping）、用户/组/权限、
   完整启动链、发布准备仍待做。
9. **第九阶段（v1.0 之后，规划）**：桌面环境兼容 —— 内核侧地基与接口对齐（D0–D5），见 [desktop.md](desktop.md)。
10. **第十阶段（v0.9 之后，进行中；执行上排在第九阶段之前）**：纯内核化边界收敛 —— 内核只提供机制，
    用户态已拆到独立 workspace `userland/`（阶段二/三/四 ✅、阶段五 🟡；阶段一/六 未做）；
    调用号/errno/记录布局自阶段三起只有一处定义（`abi/`，`make abi-check`），
    见 [pure-kernel.md](pure-kernel.md)（含第 8 节执行记录）。
11. **第十一阶段（v1.0 之后，进行中）**：CKM 内核级权限控制与扩展管理 —— CKM Core（内核）/
    CKM Service（`ckmd`）/ CKM Manager（`ckm`）/ 覆盖层 / 恢复环境，分 CKM-1…CKM-7；
    第一版不做动态内核模块装载与 GUI，见 [ckm.md](ckm.md)。
    **CKM-1（内核接口）已于 2026-10-12 落地**：`kernel/src/ckm/`（`cred`/`policy`/`state`/`audit`/`api`）+
    v1.0 `security/`（uid/gid + mode）+ syscall `257`/`258`/`259` + 判定点 1/3/5/6 的落点与证据。
    **CKM-2（可写持久化 + 覆盖层机制）同日落地**：系统盘尾部 1 MiB 的**双槽 A/B + CRC32 状态区**
    （`ckm/store.rs`，开放问题 1 取 (b)）与覆盖层注册表 + 路径改写钩子（`ckm/overlay.rs` + `fs::mount::resolve`），
    验收为 `make test-ckm`（40 断言）与 `make test-ckm-store`（跨重启：写 → 重启 → 读回）。
12. **第十二阶段（v1.0 之后，进行中；与第十一阶段同期）**：CSS 存储管理 —— 分区扫描 / 卷与稳定标识 /
    盘符与多挂载点 / 可写文件系统 / `ckm storage` 界面，分 CSS-1…CSS-6；第一版不写分区表、
    **文件系统后端编译内置**（不经 CKM 运行时装卸），见 [storage.md](storage.md)。
    **CSS-1（底层识别）已于 2026-10-11 落地**：`kernel/src/storage/`（GPT/MBR 只读扫描 + 卷记录 +
    解析自测）、`storage_volumes`(261)/`storage_rescan`(265)、`mkfs` 的 GPT/MBR 夹具、`libc` 解析器
    与 `/bin/volumes`、`make test-storage` 系列。
    **CSS-2（VFS 收口）同日落地**：`FileSystem::caps`/`volume_info`、FAT 探测收归 `fs/fat.rs`、
    挂载表拆「后端 / 挂载点」并带 `vol_id`/`options`（`alias_volume`/`umount`，挂载计数写回卷记录）、
    挂载错误改 errno、`stat` 行尾 `vol=`、`fs::storage_mount_selftest` 开机自测；
    `tests/test_boot.py` 增 `--expect-re`（反向引用绑同一 `vol_id`，因为启动盘序列号随镜像构建变化）。
    **CSS-3（盘符）于 2026-10-12 大部分落地**：盘符表 `storage/letters.rs`（确定规则 + 自定义映射落
    CKM 状态区键 `storage/letters`）+ `storage_mount`(262)/`storage_unmount`(263)/`storage_assign_letter`(264)
    （判定点 8 `CAP_STORAGE_ADMIN`）+ `libc` 的 `C:\` 重写 + `/bin/volumes` 子命令；仍缺 `/storage/**`
    只读状态树与卷标写/改名。

## 5. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-12 | **CKM-2（可写持久化 + 覆盖层机制）落地** —— 开放问题 1 定夺为 **(b) 双槽 A/B + CRC32 的极小日志式区**（系统盘尾部 1 MiB，在 FAT 之外；`tools/image-builder/build_image.py` 改为「先在 63 MiB 上 `mformat`、再追加保留区」）、`kernel/src/ckm/store.rs`（`get`/`put`/自测）、`kernel/src/ckm/overlay.rs`（注册/撤销/最长前缀/优先级/冲突拒绝，钩子在 `fs::mount::resolve`）、`policy::reject`（语义拒绝，`need=0x0`）、`fs::overlay_selftest`；新增 `make test-ckm-store` + `tests/test_ckm_store.py`（两阶段启动）与编排场景 `ckm-store`，`test-ckm` 扩到 40 断言，`test-fat`/`test-programs`/`test-storage` 的 FAT 几何与根目录条目断言同步；第十一阶段改为「进行中」 |
| 2026-10-12 | **CKM-1（内核接口）落地**：`kernel/src/ckm/`（`cred`/`policy`/`state`/`audit`/`api`）+ v1.0 `security/` + syscall `257`/`258`/`259` + 判定点 1/3/5/6；新增 `make test-ckm` 与编排场景 `ckm`，`test-user`/`test-proc` 同步扩展；同批修掉一处内核栈对齐 UB（`process::kstack_top` 16 字节对齐） |
| 2026-10-11 | **CSS-2（VFS 收口）落地** —— `fs::mount` 拆 `backends`/`mounts`（一个后端可被多挂载点引用）+ `vol_id`/`options`/`alias_volume`/`umount`/`route`/`sync_counts`；`FileSystem` 增 `caps`/`volume_info`（保守缺省；RAMFS 可写、FAT 只读+大小写不敏感+卷标）；FAT 探测唯一入口 `fs::fat::probe`/`probe_sector`（删掉 `storage/volume.rs` 里重复的 BPB 判定）、`FatFs::open` → `OpenError{errno,reason}`；`fs::mount_disks` 按卷表挂载并保留 `/disk`/`/mnt/fat` 兼容点；`fs: stat` 行尾 `vol=`；新增 `fs::storage_mount_selftest` 开机自测与 `storage: mount`/`volcheck`/`unmount`/`fs: route` 证据行；`tests/test_boot.py` 增 `--expect-re`；`make test-storage`（31 项 + 2 正则）/`test-storage-mbr`（14 项 + 1 正则）与编排场景 `fs`/`fat`/`storage`/`storage-mbr`/`programs` 全 PASS；第十二阶段与 storage.md 第 2/5/9/10 节同步 |
| 2026-10-11 | **新增「CSS 存储管理系统规划」（v1.0 之后）** —— 新建 [storage.md](storage.md)：卷模型（设备→分区→卷→盘符/挂载点 + 稳定 `vol_id`）、Windows 式盘符与 Linux 式挂载双视图、VFS 收口与挂载表带卷身份、与 CKM 的接口（能力位 / 判定点 / 号段 261–266）、文件系统支持路线、CSS-1…CSS-6 分期与 11 条验收；本文件标第十二阶段并补 v1.0 之后规划段 |
| 2026-10-06 | 第一阶段初稿：10 个里程碑 + 25 步依赖顺序 |
| 2026-10-06 | v0.1 完成，里程碑表标记 ✅ |
| 2026-10-06 | v0.2 完成（GDT/IDT/异常 + B-01/B-05 修复），里程碑表标记 ✅ |
| 2026-10-07 | v0.3 完成（帧分配器/页表 Mapper/内核堆 + B-06/B-07 修复），里程碑表标记 ✅ |
| 2026-10-07 | 补完 v0.3 后尚未整理的文档：完善 docs/drivers.md / docs/syscall.md / docs/filesystem.md / docs/process.md，补记 DEVELOPMENT.md v0.4 验收项与 docs/README.md 索引状态 |
| 2026-10-08 | v0.4 完成（PIC/PIT 100Hz/PS-2 键盘/上下文切换原语 + 自动化验收），里程碑表标记 ✅ |
| 2026-10-08 | v0.5 内核侧核心完成（线程/PID/就绪队列/Round Robin 时间片抢占 + `make test-scheduler`），里程碑表标记 🟡（IPC 待做）；本次 docs 迭代统一各阶段文档顶部元信息、修订记录与主开发文档（DEVELOPMENT.md）描述一致 |
| 2026-10-08 | v0.4/v0.5 收官复核：`make go-test` / `make test-orch` 实测通过（H-04/H-05 落地），IPC pipe 骨架落代码并通过 `make test-ipc`；v0.4、v0.5 里程碑均标记 ✅ |
| 2026-10-10 | v0.6 核心落地：syscall/sysret 入口 + ring3 + 用户地址空间隔离 + 首批 4 个 syscall，`make test-user` PASS；里程碑标记 🟡（ELF Loader 完整化 / 进程对象留到 v0.7） |
| 2026-10-11 | v0.6 用户态脚手架可用：`libc/` 落地（syscall 封装 + `_start` + `print!/println!`）、真实用户程序 `user/hello` 并入 workspace、`kernel/build.rs` 构建期摊平内嵌、`load_program` 支持多代码页；新增 `make user`，`make test-user` 增至 19 项断言 |
| 2026-10-11 | **v0.8 前半落地**（驱动注册框架 + PCI 枚举 + 块设备 + legacy virtio-blk）：新增 `driver/{driver,pci,block,virtio_blk}.rs` 与 `arch/x86_64/port.rs`、`frame::allocate_contiguous`；启动盘改为 virtio-blk（`disable-modern=on`）；新增 `make test-driver`（22 项）与编排第 9 个场景；该阶段里程碑当时记为 🟡（FAT/`mkfs` 未做，三日内的同一版本内补齐后标 ✅） |
| 2026-10-11 | **v0.7 核心落地**：VFS + RAMFS + fd 表 + `read/open/write/close` + **ELF Loader**（用户程序改为从 RAMFS `/init` 装载，按段权限逐页映射；`build.rs` 不再摊平）；`user.ld` 四段 `ALIGN(0x1000)`；新增 `make test-fs` 与 `tools/check_user_elf.py`，`make test-user` 增至 32 项；v0.7 里程碑标记 🟡（Shell `ls/cat/cd` → v0.9，磁盘 FS + `mkfs` → v0.8） |
| 2026-10-11 | **v0.9 第四步：简化网络栈（virtio-net → Ethernet → ARP → IPv4 → UDP）** —— 新增 `driver/net.rs`（`NetworkDevice` + 注册表）、`driver/virtio_net.rs`（legacy virtio-net 0.9.5：RX/TX 双队列、10 字节 virtio-net 头、MAC 读自设备配置空间、同步轮询）、`network/{ethernet,arp,ipv4,udp,dhcp}.rs` 与 `network/mod.rs`（`Stack` + `selftest`）；`network/{tcp,socket}.rs` 占位说明更新；`driver::register_builtin` 追加 virtio-net、`main.rs` 磁盘验收后跑网络自测（无网卡时跳过）；新增 `make test-network`（13 项，QEMU 加 legacy virtio-net-pci + `-netdev user` slirp）与编排场景 `network`（12 → **13 场景**）；`tests/test_boot.py` 增 `--nic` |
| 2026-10-11 | **v0.9 第三步：进程对象 + `fork`/`execve`/`wait4`**：`process/process.rs` 换成真正的 PCB（每进程 64 KiB 内核栈 + 栈底哨兵、`Ready/Running/Zombie`、`return_to`、停在 syscall 里的完整用户帧），新增 `syscall/process.rs`（fork 复制地址空间与当前帧、execve 换映像并重建 argv 用户栈、wait4 延迟调度 + 回收），`trampoline` 改为按进程切 CR3/TSS.RSP0/栈顶；`libc` 补上 `args`（入口寄存器 ABI）与去桩的 `fork`/`execve`/`wait4`；新增外部程序 `user/hello-bin` → RAMFS `/bin/hello`，Shell 的 `cmd_external` 改走 `fork`+`execve`+`wait4`；`tests/test_keyboard.py` 新增 `--expect-re`（正则 + 反向引用）并新增 `make test-proc`（8 项 + 4 正则），编排场景 11 → **12**；`make test-shell` 12 → **11 项**；记录两条踩坑（debug 构建下 16 KiB 内核栈静默溢出踩坏 `.bss` 帧分配器；退出时先恢复 CR3 再 Drop 地址空间）；v0.9 里程碑仍 🟡（动态链接/网络未做） |
| 2026-10-11 | **v0.9 第二步：`stdin` + 交互式 Shell**：键盘行缓冲（行编辑 + 终端式回显 + `kbd: line ready len=N`）、`read(0)` 整行交付（3 秒 `-EAGAIN`）、新增 `user/shell` → RAMFS `/bin/sh`（提示符 `root@cerlesse C:>`、内建 `help`/`echo`/`cat`/`pid`/`exit`、外部命令真调 `fork()` 并暴露 ENOSYS）、`kernel/build.rs` 改为多用户程序构建与内嵌、`fs::init` 加 `/bin`、`syscall/mod.rs` 打印 `syscall: ENOSYS nr=… name=…`；新增 `make test-shell`（12 项）与编排 `shell` 场景（10 → **11 场景**）；`make test-user` 59 → **61 项**、`test-fs` 根目录 3 → 4；v0.9 里程碑仍 🟡（进程对象/外部程序/网络未做，见 process.md 1.3） |
| 2026-10-11 | **v0.9 开局（运行时前置条件第一步：用户堆）**：`syscall/memory.rs` 实现 `brk`/`mmap`/`munmap`（256 KiB 堆窗口 + 256 KiB 匿名 mmap 窗口、按需映射、新页全零、W^X、`invlpg`），`libc` 提供 `#[global_allocator]`（bump over mmap）与封装，`user/hello` 加入堆自测，`user.rs` 新增页帧泄漏检查；`make test-user` 32 → **59 项** PASS，`make test-orch` 10 场景 PASS；v0.9 里程碑标记 🟡（Shell/网络仍未做） |
| 2026-10-11 | **v0.8 收官**（FAT 磁盘后端 + H-03 `mkfs`）：`fs/fat.rs`（FAT12/16/32 只读：BPB 按簇数定类型、固定根目录区/簇链目录、8.3 + NT 标志 + LFN 校验和、任意偏移读、`EROFS`/`EUCLEAN`）挂到 VFS（`fs::mount_disks` → `/disk` 启动盘、`/mnt/fat` 测试盘，验证最长前缀路由）；`FileSystem` trait 改为 `&mut self` + `read_all -> Vec<u8>`；`tools/mkfs/mkfs.py` 自己生成 FAT12/16/32 镜像并由 mtools 独立复核；新增 `make fat-image` / `make test-fat`（21 项，双盘）与编排第 10 个场景；v0.8 里程碑标记 ✅ |
| 2026-10-11 | **v0.9 第五步：基础用户程序（`ls`/`cat`/`echo`/`ps`）与目录/进程枚举** —— 新增 `syscall/dir.rs`（`getdents`：VFS `list()` → 定长 88 字节记录；号 78 沿用 Linux 但格式不同）与 `syscall/process.rs::sys_getprocs`（定长 40 字节记录；号 **256**，`syscall_table.rs` 确立「`0..=255` Linux 语义 / `>= 256` Cerlesse 扩展」并把 `SYS_MAX` 改为 257）；`process/process.rs` 新增定长 `comm`（`create` 填充、`execve` 改写为程序名）；`libc` 新增 `getdents`/`getprocs` + `dirent_*`/`procinfo_*` 解析器；新增 `user/{ls,cat,echo,ps}`（`kernel/build.rs`、workspace、`fs::init` 清单 3 → 7），Shell 帮助文本同步；新增 `make test-programs`（17 项 + 5 正则）与编排场景 `programs`（13 → **14 场景**）；`make test-proc` 的负例由 `ls` 改 `foo` |
| 2026-10-11 | **v0.9 第六步（收官）：内核侧最小动态链接** —— 新增 `syscall/dynamic.rs`（`ET_DYN` 分流：`PT_INTERP` + `DT_NEEDED` → `/lib/<name>` 递归装载，应用 `R_X86_64_RELATIVE`/`GLOB_DAT`/`JUMP_SLOT`/`64`；`DT_PLTREL=REL` 明确拒绝）；`address_space.rs` 新增共享库窗口（基址 + 1 MiB、32 页）与 `map_dyn_page`/`phys_of`；新增夹具 `user/dynhello/`（`as`+`ld` 现编）与 RAMFS `/lib`（清单 7 → 8）；新增 `make test-dyn`（10 项）与 `make dyn-hostcheck`（宿主机 glibc 独立复核）；`test-fs` 20 → 21、`test-programs` 条目数 5/7；v0.9 里程碑标记 ✅（见 syscall.md 3.9） |
| 2026-10-11 | **v1.0 第一步：`mprotect` 落地** —— `syscall/memory.rs::sys_mprotect`（页对齐/`len>0`/未知位/W^X 四类拒绝各留串口原因）+ `address_space.rs::mprotect`（范围限同一窗口、每页已映射；`PROT_NONE` = 清 `USER` 留 `PRESENT`；改权限即 `invlpg`）+ 堆窗口逐页 `heap_flags`；`mmap(PROT_NONE)` 由拒绝改为合法，`syscall_table::UNIMPLEMENTED` 因此**清空**；`libc` 新增 `mprotect`/`PROT_NONE`，`user/hello` 新增 `mprotect_selftest`（`RX` 后真的执行机器码 + brk 堆窗口的 `PROT_NONE` 往返）；`make test-user` 61 → **91 项** PASS；同时补齐仓库缺失的 **`LICENSE`（MIT）**；`kernel/build.rs` 在无清单分支登记默认 `manifest.txt` 为构建输入（修掉「先编内核后出镜像 → 一直无用户态」的缓存陷阱）；里程碑表 v1.0 行标记第一步 ✅ |
| 2026-10-11 | **新增「CKM 设计提案」（v1.0 之后，规划）** —— 新建 docs/ckm.md：定稿权限模型（单管理员 + 4 能力位 + 7 判定点，凭证只由组件表在 `execve` 时改变）、内核接口（`kernel/src/ckm/` 五模块 + `CKM_ABI_VERSION=1` + 扩展号段 257–259、`SYS_MAX` → 260 + 审计 48 字节/组件状态 64 字节定长记录 + 串口证据格式）、扩展生命周期（四类扩展、八态状态机 + 合法转换表、确定性枚举/加载顺序）；写清与 H-06 的分工（`.ckc` 不重做、P0b = CKM-3、`cpm` → `ckm pkg`）、覆盖层与 KernelSU 元模块的对应、安全模式/恢复环境、CKM-1…CKM-7 分期、8 条故障场景落成断言、非目标与 7 个开放问题；里程碑表新增第十一阶段 |
| 2026-10-11 | **纯内核化执行（阶段二/四/五部分）**：`libc/`+`user/` 迁入 `userland/` 独立 workspace，内核构建改为「按 `manifest.txt` 内嵌」（无用户态也能构建 + 启动到串口诊断模式），新增 `tools/check-boundary.py` 边界门禁与 `image-kernel`/`test-kernel-only`/`userland-images` 目标；第十阶段由「规划」改为「进行中」，详见 pure-kernel.md 第 8 节 |
| 2026-10-11 | **新增「纯内核化改造规划」（v0.9 之后，规划）**：新建 docs/pure-kernel.md —— 现状审计 C1–C9（内核构建期嵌套 cargo 构建用户程序、10 处 `include_bytes!` 把 203,280 字节（≈199 KiB）用户态内嵌进内核映像、`fs::init` 硬编码 `/bin`/`/lib`、启动末尾跑用户态自测、ABI 两处镜像、workspace 与测试责任混杂），实测删掉 `libc/`+`user/` 后 `cargo build -p kernel` 退出码 **101**；逐文件迁移清单（内核 83 文件仅 9 个要改；`user/` 33 + `libc/` 7 + `tools`/`tools-go`/`tests` 逐项去向）；六阶段与七条验收标准对照；同步 docs/README、DEVELOPMENT（5.11）、project-structure（第 4 节）与 README |
| 2026-10-11 | **新增「桌面环境兼容路线」（v1.0 之后，规划）**：新建 docs/desktop.md —— 现状审计（显示/输入/ioctl/设备模型/`/proc`/futex/音频/ACPI 等逐项有据）、用户态与内核侧归属纠正（logind/D-Bus/PipeWire 是用户态，KVM 与桌面前置无关）、被遗漏的地基清单、D0–D5 分层与 QEMU 可断言验收（monitor `sendkey`/`screendump` 实测可用）、里程碑 v1.1–v1.6；同步 roadmap（第九阶段 + 「禁止提前做」项修正）、DEVELOPMENT（5.10）、architecture 第 5 节、project-structure（规划新增目录）与 README |
| 2026-10-11 | **CSS 提案评审补边界**：[storage.md](storage.md) 增 6.8「文件系统后端装载政策」（第一版全部编译内置、不经 CKM 运行时装卸）、6.1「覆盖层/回滚/恢复归 CKM」、6.7「兼容性完整展示」、第 11 节「不做写回修复」（`fsck` 只读检查）；第十二阶段段落同步 |
| 2026-10-11 | **CSS-1 落地**：`kernel/src/storage/`（GPT 主/备表头 + CRC32、MBR + EBR 链只读扫描，无分区表退化为整盘一卷，FAT 探测）、块设备注册表 2 → 8、`storage_volumes`(261)/`storage_rescan`(265)（64 字节定长卷记录、确定性枚举）、`mkfs.py` 的 GPT/MBR/主表受损/无表四类夹具、`libc` 解析器 + 用户程序 `/bin/volumes`、`make storage-images`/`test-storage-fixtures`/`test-storage`/`test-storage-mbr` 与编排场景 `storage`/`storage-mbr`（15 → **17 场景**）、内核解析自测 11 例 / 45 断言；第十二阶段由「规划」改「进行中」；同步 [storage.md](storage.md) 第 2/9/10 节、docs/README.md、DEVELOPMENT.md 5.13 |
| 2026-10-12 | **纯内核化阶段三落地（ABI 单一事实源）**：新增 `abi/` crate（调用号/`SYS_MAX`/号段规则、errno、`O_*`/`PROT_*`/`MAP_*`/fd、`PATH_MAX`/`IO_MAX`/`ARG_*`、`dirent`/`procinfo`/`volume`/`ckm` 记录布局与能力位/op 编号 + 编译期断言）；内核 18 个文件与 libc 45 个常量改为引用它（对外名字与类型不变、行为零变化），新增 `make abi-check` 并并入 `make check`；第十阶段由「阶段二/四 ✅」更新为「阶段二/三/四 ✅」；详见 [pure-kernel.md](pure-kernel.md) 8.2 |
| 2026-10-12 | **CSS-3（盘符）大部分落地**：盘符表 `kernel/src/storage/letters.rs`（固定容量：缺省 `C` 给系统卷、其余按枚举顺序给 `D`/`E`/…；自定义映射持久化到 CKM 状态区键 `storage/letters`，`letter = 0` 取消回到规则）、`storage/registry.rs` 的 `letter_of`/`slot_of`/`assign_letter`（盘符写入卷记录偏移 24）、`syscall/storage.rs` 把 262 `storage_mount`/263 `storage_unmount`/264 `storage_assign_letter` 从 ENOSYS 桩上拿下（判定点 8 `CAP_STORAGE_ADMIN`）、`fs::mount_volume_by_id`/`fs::unmount_point`、`userland/libc/src/path.rs`（`C:\` → POSIX，查卷表不硬编码字母）+ 三个 syscall 包装、`/bin/volumes` 的 `list`/`mount`/`unmount`/`letter`/`demo` 子命令；`make test-storage`（`letter=` + `letters selftest checks=36`）/`test-storage-mbr`（`checks=22`）/`test-programs`（`volumes demo` 双入口）/`test-ckm`（零能力拒绝）/`test-ckm-store`（`storage/letters` 跨重启）全 PASS；第十二阶段与 [storage.md](storage.md) 第 2/4/6/9/10/13 节同步 |