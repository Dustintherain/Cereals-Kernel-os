# 项目目录结构（第一阶段）

- 阶段：第一阶段 / 结构规划
- 最后更新：2026-10-11
- 状态：✅ 已完成（代码框架已按本结构创建占位文件；占位文件内标注了计划实现的版本号；2026-10-08 本次 docs 迭代统一元信息与修订记录；2026-10-11 补记 v0.9 六步（含动态链接夹具与 `/lib`）与 v0.8 收官）

> 本文档顶部保留阶段 / 最后更新 / 状态字段，与项目文档维护约定对齐。

## 1. 完整目录树

```text
Cerlesse/
│
├── shared/                      # boot/kernel 共享数据结构（v0.1 ✅）
│   └── src/lib.rs               # shared::BootInfo、BOOT_MAGIC
│
├── abi/                         # ✅ 纯内核化阶段三：系统调用 ABI 单一事实源
│   └── src/                     # syscall / errno / flags / limits / dirent / procinfo / volume / ckm / record
│
├── boot/                        # Bootloader crate（v0.1 ✅）
│   ├── src/                     # main.rs（UEFI 入口+ELF 装载）、serial.rs、uefi.rs
│   └── Cargo.toml
│
├── kernel/                      # 内核主体（Rust, no_std）
│   ├── src/
│   │   ├── arch/x86_64/         # 架构相关：boot/cpu/gdt/idt/interrupt/paging/context/port（v0.8 端口 I/O）
│   │   ├── memory/              # frame/page/heap/allocator
│   │   ├── interrupt/           # exception/irq/controller
│   │   ├── process/             # process/thread/context/pid
│   │   ├── scheduler/           # scheduler/queue/round_robin
│   │   ├── syscall/             # ✅ v0.6–v0.7：trampoline/address_space/elf/syscall_table/process/file/user
│   │   │                        #    ✅ v0.9 第一步：memory（brk/mmap/munmap，用户堆）
│   │   │                        #    ✅ v0.9 第二步：file::read_stdin（键盘行缓冲 → fd 0）
│   │   │                        #    ✅ v0.9 第三步：process 的 fork/execve/wait4 + argv 用户栈
│   │   │                        #    ✅ v0.9 第五步：dir（getdents）+ process::getprocs（目录/进程表枚举）
│   │   │                        #    ✅ v0.9 第六步：dynamic（ET_DYN 最小动态链接）+ elf 按 e_type 分流
│   │   ├── fs/                  # ✅ v0.7–v0.8：vfs/inode/mount/ramfs/file + fat（FAT12/16/32 只读后端）
│   │   ├── driver/              # ✅ v0.8：driver（注册框架）/pci/block/virtio_blk + serial（v0.1）
│   │   │                        #    ✅ v0.9：keyboard 加行缓冲与行编辑（v0.4 起为 IRQ1 回显）
│   │   │                        #    ✅ v0.9 第四步：net（NetworkDevice 抽象）+ virtio_net（legacy 网卡驱动）
│   │   │                        #    ✅ v1.0：ahci（AHCI/SATA 块驱动；pci 修 B-18：头部类型偏移 0x0E）
│   │   ├── time/                # timer/clock
│   │   ├── ipc/                 # pipe/channel/shared_memory
│   │   ├── network/             # ✅ v0.9 第四步：ethernet/arp/ipv4/udp/dhcp + mod（Stack + selftest）
│   │   │                        #    ✅ v1.0 第三步：tcp（三次握手/按 MSS 发送/累积 ACK/重传/按序接收/RST/FIN + echo 验收函数）
│   │   │                        #    + socket（8 槽套接字表，fd 16..=23）；ICMP 仍为占位（完整 TCP/IP 是第一版非目标）
│   │   ├── security/            # permission/credential/capability
│   │   └── main.rs              # 内核入口（v0.1 ✅：_start + kernel_main）
│   ├── link.ld                  # 链接脚本：装载到 32MiB（ADR-007）
│   ├── build.rs                 # 只注入链接脚本；用户态镜像按 manifest.txt 内嵌（清单缺失 = 空清单）
│   └── Cargo.toml
│
├── userland/                    # ✅ 纯内核化：**用户态独立 workspace**（自 2026-10-11 起，见 pure-kernel.md）
│   ├── Cargo.toml               # workspace：libc + user/*；含 user-release profile
│   ├── .cargo/config.toml       # code-model=large + relocation-model=static（**只对用户态**）
│   ├── Makefile                 # images / check / dyn / dyn-hostcheck / test-pkg / test-all → build/userland/images/*
│   ├── libc/                    # ✅ 最小用户态运行时（syscall 封装 / _start / print! / heap / args）
│   ├── pkgmgr/                  # ✅ 阶段五迁入：ckc.py（H-06 P0a 容器）+ cpm.py（P0b 事务引擎）+ tests/
│   ├── tools/                   # ✅ 用户态侧校验脚本：check_dyn_fixture.py、check_user_elf.py（阶段五迁入）
│   └── user/
│       ├── hello/               # ✅ Ring3 用户程序 → RAMFS /init（libc + user.ld + build.rs）
│       ├── shell/               # ✅ 交互式 Shell → /bin/sh（fork/execve/wait4 跑外部命令）
│       ├── hello-bin/           # ✅ 外部程序 → /bin/hello（由 Shell fork+execve 起来）
│       ├── ls/ cat/ echo/ ps/   # ✅ coreutils → /bin/*
│       ├── volumes/             # ✅ 存储卷枚举 → /bin/volumes（CSS-1 storage_volumes）
│       ├── tcpecho/             # ✅ TCP 回显验收 → /bin/tcpecho（v1.0 第三步 socket/TCP 补齐）
│       ├── dynhello/            # ✅ 汇编夹具（as+ld 现编）→ /bin/dynhello + /lib/{ld.so,libgreet.so}
│       └── init/                # v0.9 占位（内核实际跑的 /init 是 user/hello 的产物）
│
├── tools/                       # 主机侧工具（阶段五后：只留内核/镜像侧工具）
│   ├── image-builder/           # Python：磁盘镜像组装
│   ├── check-boundary.py        # ✅ 纯内核化：内核/用户态边界门禁（6 项 + --selftest）
│   └── mkfs/                    # ✅ v0.8：FAT12/16/32 镜像生成（mkfs.py，mtools 独立复核）
│                                #    （check_user_elf.py 与 pkgmgr/ 已随阶段五迁入 userland/）
│
├── tools-go/                    # Go：测试编排 / 串口日志分析等主机服务
│
├── tests/                       # 集成测试（Python 驱动 QEMU；**只包含内核场景** —— 用户态集成场景在 orchestrate.json 里标 `tags: [userland]`，由 test-orch-userland 跑）
│   └── fatroot/                 # ✅ v0.8：FAT 测试盘的内容（data/）与复核说明
│
├── docs/                        # 全部开发文档（见 docs/README.md）
│
├── Cargo.toml                   # Cargo Workspace（**只有 shared/boot/abi/kernel**；exclude userland）
├── Makefile                     # 构建/QEMU/测试入口（v0.1 ✅）
├── DEVELOPMENT.md               # 主开发文档
└── README.md
```

## 2. 模块职责速查

| 目录 | 职责 | 计划版本 |
| ---- | ---- | ---- |
| `shared/` | BootInfo 等跨 boot/kernel 纯数据结构 | v0.1 ✅ |
| `abi/` | **系统调用 ABI 单一事实源**（纯内核化阶段三 ✅）：调用号/`SYS_MAX`/号段规则、errno、`O_*`/`PROT_*`/`MAP_*`/fd 编号、参数上限（`PATH_MAX`/`IO_MAX`/`ARG_*`）、`dirent`/`procinfo`/`volume`/`ckm` 记录布局与能力位/op 编号、编译期断言；内核与 `userland/libc` 都引用它（`make abi-check`） | 纯内核化 ✅ |
| `boot/` | UEFI 引导、内存检测、装载内核、传递 BootInfo | v0.1 ✅ |
| `kernel/src/arch/x86_64/` | GDT、IDT、控制寄存器（`cpu.rs`）、页表操作（`paging.rs`）、上下文切换 | v0.1–v0.3 ✅ |
| `kernel/src/memory/` | 物理帧分配、虚拟内存映射、内核堆、全局分配器 | v0.3 ✅ |
| `kernel/src/interrupt/` | 异常处理、IRQ 分发、PIC/APIC | v0.2 / v0.4 |
| `kernel/src/time/` | PIT/APIC Timer、系统时钟、sleep/yield | v0.4 |
| `kernel/src/process/` | 进程/线程模型、PID、状态机 | v0.5 |
| `kernel/src/scheduler/` | 调度队列、Round Robin | v0.5 |
| `kernel/src/syscall/` | 系统调用入口/分发、用户地址空间、ELF Loader、首批与文件 syscall、**用户堆（`memory.rs`：`brk`/`mmap`/`munmap`）**、**进程调用（`process.rs`：`fork`/`execve`/`wait4` + argv 用户栈 + `getprocs`）**、**目录枚举（`dir.rs`：`getdents`）**、**最小动态链接（`dynamic.rs`：`ET_DYN`/`PT_INTERP`/`DT_NEEDED`/重定位）** | v0.6–v0.7 ✅ / v0.9 第一·三·五·六步 ✅ |
| `kernel/src/fs/` | VFS 抽象（trait + 路径工具）、挂载表路由、RAMFS、fd 表、**FAT12/16/32 只读后端**；**CSS-2 收口：后端能力位/卷信息、挂载表带卷身份与多挂载点、卸载、挂载错误改 errno** | v0.7 ✅ / v0.8 ✅ / CSS-2 ✅ |
| `kernel/src/driver/` | 串口（v0.1）、键盘（v0.4）、**驱动注册框架 / PCI 枚举 / 块设备抽象 / virtio-blk**（v0.8 ✅）、virtio-net（v0.9 ✅）、**ahci（AHCI/SATA，v1.0 ✅）** | v0.4 – v1.0 |
| `kernel/src/driver/{pci,driver,block,virtio_blk,ahci}.rs` | `pci` 只回答「总线上有什么」（含命令寄存器；v1.0 修 B-18：头部类型偏移 `0x0E`）；`driver` 回答「谁来管它」（`Driver` trait + `probe_all`）；`block` 是上层唯一可见的按扇区接口；`virtio_blk`/`ahci` 是两个具体实现（各自同时实现 `Driver` 与 `BlockDevice`，上层零改动） | v0.8 / v1.0 ✅ |
| `kernel/src/fs/fat.rs` | 块设备上的 FAT12/16/32 **只读**后端（第二个 `FileSystem` 实现）：BPB 解析、固定根目录区/簇链目录、8.3 + NT 标志 + LFN、任意偏移读；写操作 `EROFS`；损坏结构 `EUCLEAN`；**CSS-2 起同时是 FAT 的探测唯一入口（`probe`/`probe_sector`）与 `caps`/`volume_info` 的提供方，`open` 返回 `OpenError{errno,reason}`** | v0.8 ✅ / CSS-2 ✅ |
| `kernel/src/memory/frame.rs` | 位图帧分配器；v0.8 追加 `allocate_contiguous` / `free_contiguous`（virtqueue 与 DMA 要求的物理连续缓冲） | v0.3 ✅ / v0.8 ✅ |
| `kernel/src/arch/x86_64/port.rs` | 端口 I/O 统一入口：`inb/inw/inl` + `outb/outw/outl`（PCI 配置空间与 legacy virtio 寄存器窗口依赖 32 位读写） | v0.8 ✅ |
| `kernel/src/ipc/` | 管道、通道、共享内存 | v0.5+ |
| `kernel/src/network/` | 以太网 → IP → TCP/UDP → Socket | v0.9 |
| `kernel/src/security/` | 权限、凭证（uid/gid）、文件 mode 判定；**能力位与 CKM 判定点见 `kernel/src/ckm/`**（规划） | v1.0 |
| `userland/` | **用户态独立 workspace**（不属于内核）：`libc/`（运行时）+ `user/*`（程序）+ `Makefile`（构建 + ELF 校验 + `manifest.txt`）+ `.cargo/config.toml`（large 代码模型只对用户态生效）；含 `pkgmgr/`（包管理器主机工具，阶段五迁入）与 `tools/`（`check_dyn_fixture.py`、`check_user_elf.py`）；与内核的唯一接触面是 `build/userland/images/` | 纯内核化 ✅ |
| `userland/libc/` | 最小用户态运行时：syscall 封装（含 `open/read/close`、`brk/mmap/munmap`、`fork/execve/wait4` 与 `socket/connect/bind/sendto/recvfrom` + `sockaddr_in`（v1.0 第三步））、`_start`（入口寄存器 `argc`/`argv`）、`print!/println!`、panic handler、`heap`（`#[global_allocator]`）、`args` | v0.6 ✅ / v0.7 ✅ / v0.9 ✅ |
| `userland/user/` | 用户程序（`hello` ✅ v0.6/v0.7 → `/init`；`shell` ✅ v0.9 → `/bin/sh`；`hello-bin` ✅ v0.9 第三步 → `/bin/hello`（外部程序）；**`ls` / `cat` / `echo` / `ps` ✅ v0.9 第五步 → `/bin/*`**；`volumes` ✅ CSS-1 → `/bin/volumes`；`tcpecho` ✅ v1.0 第三步 → `/bin/tcpecho`；**`dynhello/` ✅ v0.9 第六步 —— 不是 Rust crate，而是 `as`+`ld` 现编的动态链接夹具 → `/bin/dynhello` + `/lib/{ld.so,libgreet.so}`**；`init/` 仍为占位） | v0.6–v0.9 |
| `tools/`（Python） | 镜像构建、**`check-boundary.py`（内核/用户态边界门禁，纯内核化 ✅）**、`mkfs/` FAT 镜像生成（v0.8 ✅）；归属用户态的 `check_user_elf.py` 与 `pkgmgr/` 已于**阶段五**迁入 `userland/` | v0.1 起 / 阶段五 ✅ |
| `tools-go/`（Go） | 测试编排、串口日志服务 | v0.4 起 |
| `tests/` | QEMU 集成测试 | v0.1 起 |

## 3. v1.0 之后规划新增（桌面路线，尚未创建）

桌面环境兼容路线（见 [desktop.md](desktop.md)）预计新开以下模块；**当前都还不存在**，
进入对应里程碑（v1.1–v1.5）时按本表的职责边界创建并回填：

| 规划目录 | 职责 | 里程碑 |
| ---- | ---- | ---- |
| `kernel/src/dev/`（或并入 `driver/`） | 设备模型：`/dev` 节点、设备号、fd→驱动路由、`uevent` | D0 |
| `kernel/src/procfs/`、`kernel/src/sysfs/` | `/proc`、`/sys` 最小只读视图 | D0 |
| `kernel/src/input/` | 输入核心 + evdev + uinput | D1 |
| `kernel/src/display/` | fbdev + DRM/KMS 对象模型与 uapi 子集 | D2 |
| `kernel/src/sound/` | ALSA 用户接口子集 + 驱动 | D3 |
| `kernel/src/acpi/` | FADT/MADT 解析、S5、热区/电池桩 | D4 |
| `kernel/src/ipc/{unix,eventfd,memfd}.rs` | AF_UNIX/`SO_PEERCRED`、eventfd/timerfd、memfd | D0/D4 |

### 3.1 CKM 新增（第十一阶段；**CKM-1/CKM-2 已落地**）

CKM（内核级权限控制与扩展管理，见 [ckm.md](ckm.md)）的目录；已落地的部分是
`kernel/src/ckm/*`（CKM-1 五模块 + CKM-2 的 `overlay.rs`/`store.rs`），其余按 CKM-3…CKM-7 分期创建并回填：

| 目录 | 职责 | 分期 |
| ---- | ---- | ---- |
| `kernel/src/ckm/{mod,cred,policy,state,audit,api}.rs` | CKM Core：能力位与凭证、判定点、组件状态表、审计环、给 `syscall/` 的唯一入口 ✅ | CKM-1 |
| `kernel/src/ckm/overlay.rs` | 覆盖层注册表（注册/撤销/最长前缀/优先级/冲突拒绝）+ 判定点 7/8 ✅ | CKM-2 |
| `kernel/src/ckm/store.rs` | 可写持久化：系统盘尾部 1 MiB 的双槽 A/B + CRC32 状态区 ✅ | CKM-2 |
| `kernel/src/security/{credential,permission}.rs` | 凭证（uid/gid）与文件 mode 判定（v1.0 先落地，CKM 的**下层**，不重复实现） | v1.0 |
| `userland/ckmd/` | CKM Service：事务、包操作、签名校验、修复回滚（持 `CAP_CKM_SERVICE`） | CKM-4 |
| `userland/ckm/` | CKM Manager：CLI（`ckm state`/`ckm audit`/`ckm pkg …`/`ckm enter-safe-mode`） | CKM-4 |
| `tools/recovery/`（或并入 `tools/image-builder`） | 恢复镜像与启动失败计数的构建期支持 | CKM-6 |

> 边界：CKM Core 是内核机制（`tools/check-boundary.py` 的门禁继续生效），
> `ckmd`/`ckm` 必须进 `userland/`；内核不认识 `.ckc`，也不解析扩展清单。

### 3.2 CSS 规划新增（第十二阶段，尚未创建）

CSS（存储管理，见 [storage.md](storage.md)）新开/扩展以下目录，按 CSS-1…CSS-6 分期创建并回填
（**CSS-1/CSS-2 已落地，CSS-3 大部分落地**；仍标「规划」的行尚未开工）：

| 规划目录 | 职责 | 分期 |
| ---- | ---- | ---- |
| `kernel/src/storage/{mod,registry,partition,gpt,mbr,volume,selftest}.rs` | CSS：设备/分区/卷登记、GPT/MBR **只读**解析、稳定卷标识 `vol_id`、卷标/健康状态/只读原因（✅ CSS-1 已落地） | CSS-1 |
| `kernel/src/storage/letters.rs` | ✅ **CSS-3 已落地**：盘符表（确定规则 `C`/`D`/… + 自定义映射持久化到 CKM 状态区 `storage/letters`；`assign`/`clear`/`load_from_store`/`default_for`/`selftest`） | CSS-3 ✅ |
| `kernel/src/syscall/storage.rs` | CSS 的 syscall 入口（号 **261–266**：卷枚举/挂载/卸载/分配字母/重扫/格式化；✅ 261/262/263/264/265 已实现，**只余 266 桩**） | CSS-1 / CSS-3 |
| `kernel/src/fs/{vfs,mount,inode,file,ramfs,fat}.rs` | ✅ **CSS-2 已落地**：加 `caps`/`volume_info`（及 `FsCaps`/`VolumeInfo`/`MountOptions`）、FAT 探测收归 `fs/fat.rs`、挂载表拆「后端 / 挂载点」并带 `vol_id`/`options`（`alias_volume`/`umount`/`sync_counts`）、挂载错误改 errno、`stat` 输出 `vol=` | CSS-2 ✅ |
| `kernel/src/fs/{exfat,ext2,ext4,ntfs,iso9660}/` | 新文件系统后端（各自实现 `FileSystem`，VFS 不变） | CSS-4 / CSS-6 |
| `userland/libc/src/path.rs` | ✅ **CSS-3 已落地**：Windows 式路径（`C:\x`）→ POSIX 路径重写（查 `storage_volumes`，不硬编码字母；`is_windows`/`resolve`/`resolve_str`） | CSS-3 ✅ |
| `userland/ckm/`（`ckm storage …`） | CSS Manager：列出磁盘/分区/卷、挂载/卸载、分配盘符、显示健康与只读原因 | CSS-5 |
| `tools/mkfs/` | ✅ **GPT/MBR 分区表镜像生成**（CSS-1 已落地，`make test-storage` 的夹具），沿用「自己生成 + 宿主独立复核」的做法 | CSS-1 |

> 边界：**盘符不属于文件系统驱动**（`fs/*` 不认识 `C:`）；CSS 不重写 VFS（新格式 = 再实现一个
> `FileSystem`）；第一版不写分区表（GPT/MBR 只读扫描）；权限判定走 CKM，界面在 `userland/`。
> 见 [storage.md](storage.md) 第 0/6/11 节。

## 4. 纯内核化（已执行大部分，见 [pure-kernel.md](pure-kernel.md) 第 8 节）

内核只保留机制；用户态已拆成**独立 workspace** `userland/`（本轮），后续用
`git subtree split -P userland` 可整体搬到 `Cerlesse-Userland` 仓库。

```text
Cerlesse-Kernel-Cerlesse/（内核）      userland/（用户态独立 workspace）
├── boot/       引导器（独立组件）      ├── libc/        用户态运行时
├── shared/     boot↔kernel 纯数据      ├── user/hello   → RAMFS /init
├── kernel/     内核主体                ├── user/shell   → /bin/sh
├── tools/      构建工具 / 边界门禁      ├── user/{ls,cat,echo,ps}  coreutils
├── tools-go/   内核场景编排/串口        ├── user/hello-bin  → /bin/hello
├── tests/      内核场景 + fatroot      ├── user/dynhello  动态链接夹具
├── docs/                              └── Makefile / Cargo.toml / .cargo
└── userland/   用户态（上面一列）
```

唯一接触面是构建产物 `build/userland/images/manifest.txt`：内核构建脚本按它把镜像
内嵌进内核映像，**清单不存在时内核照样构建与启动**（进入串口诊断模式，
`make test-kernel-only`）。本轮实际只改了 3 个内核文件（`kernel/build.rs`、
`kernel/src/syscall/user.rs`、`kernel/src/fs/mod.rs`）；`kernel/src/{arch,memory,interrupt,
time,process,scheduler,fs,driver,network,ipc,security}`、`boot/`、`shared/` 未动。
目标结构里 `kernel/src/` 仍沿用现有模块划分，不新增 `boot/`、`block/`、`sync/`、`lib/` 等目录（非目标）。
`abi/`（syscall ABI 单一事实源）**已于阶段三创建**（`make abi-check`；目标结构里的
`abi/` 就是它）；`tests/orchestrate-kernel.json` 的物理拆分**未采用** —— 阶段五改用 `tests/orchestrate.json` 的 `tags` 分组（内核侧 `-exclude-tag userland` 的 13 场景 / 用户态侧 `-tag userland` 的 6 场景），见 [pure-kernel.md](./pure-kernel.md) 8.3。

## 5. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-12 | **纯内核化阶段三：`abi/` 单一事实源** —— 目录树新增 `abi/`（`syscall`/`errno`/`flags`/`limits`/`dirent`/`procinfo`/`volume`/`ckm`/`record`），根 `Cargo.toml` 变为 `shared/boot/abi/kernel`，模块表新增 `abi/` 行；第 4 节由「`abi/` 尚未创建」改为「已落地」；内核 18 文件与 libc 45 常量改为引用它（`make abi-check`） |
| 2026-10-12 | **CKM-2**：`kernel/src/ckm/` 新增 `overlay.rs`（覆盖层注册表）与 `store.rs`（状态区）；3.1 节由「尚未创建」改为「已落地/待建」；`tools/image-builder/build_image.py` 的镜像布局（FAT 63 MiB + 状态区 1 MiB）与新增 `tests/test_ckm_store.py` |
| 2026-10-11 | v1.0 之后规划：新增第 3.2 节（**CSS 存储管理**预计新开/扩展的目录：`kernel/src/storage/`、`syscall/storage.rs`、`fs/{exfat,ext2,ext4,ntfs,iso9660}/`、libc 路径模块、`ckm storage`、`mkfs` 分区表生成 —— 均未创建，进入 CSS-1…CSS-6 时回填） |
| 2026-10-11 | 纯内核化**执行阶段二/四/五（部分）**：`libc/`(7) 与 `user/`(33) 移入 `userland/` 独立 workspace（目录树重写为本节的 userland/ 子树，含 `Cargo.toml`/`.cargo/config.toml`/`Makefile`/`tools/check_dyn_fixture.py`）；`kernel/build.rs` 改为「按 `manifest.txt` 内嵌」（不再构建用户程序）；新增 `tools/check-boundary.py`；根 `Cargo.toml` 收敛为 `shared/boot/kernel` + `exclude userland`；第 4 节由「规划」改为「已执行大部分」（实测：无 userland 的 checkout 仍能 `make image` + `make test-kernel-only`） |
| 2026-10-11 | 纯内核化规划：新增第 4 节（目标结构：内核/用户态双仓库、新增 `abi/`、9 个内核文件改动清单）；修订记录顺延为第 5 节 |
| 2026-10-11 | v1.0 之后规划：新增第 3 节（桌面路线预计新开的模块：`dev`/`procfs`/`sysfs`/`input`/`display`/`sound`/`acpi`/`ipc` 扩展 —— 均未创建，进入里程碑时回填） |
| 2026-10-11 | v0.9 第六步（最小动态链接）：`kernel/src/syscall/dynamic.rs` 与 `address_space.rs` 的共享库窗口（基址 + 1 MiB、32 页）落地，`elf.rs` 按 `e_type` 分流；新增汇编夹具 `user/dynhello/`（`as`+`ld` 现编）与 RAMFS `/lib`，`kernel/build.rs` 的程序清单 7 → 8 —— 目录树、模块表与 `user/` 描述同步 |
| 2026-10-14 | v1.0 第三步（Socket/TCP 补齐）：`kernel/src/network/tcp.rs` 从占位重写为真实实现（三次握手/按 MSS 切块发送/累积 ACK/重传/按序接收/RST/FIN + `echo` 验收函数），`socket.rs` 重写为 8 槽套接字表（fd 16..=23）；新增 `kernel/src/syscall/net.rs`（五个 handler）；`user/tcpecho` 从占位变为实现（RAMFS `/bin/tcpecho`）；`libc/` 新增 socket 系封装与地址构造器 —— 目录树、模块表同步 |
| 2026-10-11 | v0.9 第五步（基础用户程序）：`kernel/src/syscall/dir.rs`（`getdents`）与 `process.rs::sys_getprocs` 落地；`user/{ls,cat,echo,ps}` 从占位变为实现（RAMFS `/bin/*`），`kernel/build.rs` 的程序清单 3 → 7；`libc/` 新增 `getdents`/`getprocs` 与 `dirent_*`/`procinfo_*` 解析器 —— 目录树、模块表与 `libc/` 描述同步 |
| 2026-10-11 | v0.9 第三步（进程对象与外部程序）：`kernel/src/syscall/process.rs` 的 `fork`/`execve`/`wait4` 落地，`process/process.rs` 换成真正的 PCB（每进程内核栈），`trampoline` 改为按进程切换；`libc/` 新增 `args.rs`；新增 `user/hello-bin`（RAMFS `/bin/hello`）—— 目录树、模块表与 `libc/` 描述同步 |
| 2026-10-11 | v0.9 第二步（交互式 Shell）：`user/shell` 从占位变为实现（首个 `/init` 之外的用户程序，RAMFS 上的 `/bin/sh`），`kernel/build.rs` 由「单个用户程序」改为「按列表构建/校验/内嵌」，`fs::init` 增加 `/bin`；`driver/keyboard.rs` 加行缓冲（目录树注释同步） |
| 2026-10-06 | 第一阶段初稿，同步创建代码文件框架占位 |
| 2026-10-06 | v0.1 落地：新增 `shared/` crate、`link.ld`、`build.rs`，boot/kernel 标记完成 |
| 2026-10-07 | v0.3 落地：`memory/` 与 `arch/x86_64/{paging,cpu}.rs` 实现完成；link.ld 增加 `__kernel_start/__kernel_end` |
| 2026-10-08 | 本次 docs 迭代统一文档顶部元信息（阶段 / 最后更新 / 状态）与修订记录，保持与 DEVELOPMENT.md 与索引文档描述一致 |
| 2026-10-08 | 按开发文档流程继续规划/占位更新：v0.5 IPC 模块（pipe/channel/shared_memory）位置已保留，pipe 优先；v0.6 syscall 模块细化路径已补充至 syscall.md，让后续可直接开工 |
| 2026-10-11 | v0.6 用户态脚手架落地：`libc/` 与 `user/hello` 从占位变为实现并加入 workspace；`kernel/build.rs` 新增构建期用户程序构建与 ELF 摊平；workspace 新增 `user-release` profile 与 `default-members` |
| 2026-10-11 | v0.7 落地：`kernel/src/fs/` 从占位变为实现（`vfs`/`inode`/`mount`/`ramfs`/`file`），`kernel/src/syscall/` 新增 `elf.rs` 与文件 syscall；`kernel/build.rs` 由「摊平内嵌」改为「校验 ELF 后原样内嵌」（RAMFS `/init`）；新增 `tools/check_user_elf.py`（`make user` 验段布局） |
| 2026-10-11 | v0.8 前半落地：`kernel/src/driver/` 新增 `pci.rs` / `driver.rs` / `block.rs` / `virtio_blk.rs`（模块表里补上分层职责），`kernel/src/arch/x86_64/port.rs`（端口 I/O），`memory/frame.rs` 追加连续帧分配；`user/{init,shell,ls,...}` 仍为占位 |
| 2026-10-14 | **v1.0 驱动兼容性**：`driver/` 新增 `ahci.rs`（AHCI/SATA 块驱动，第二条 `BlockDevice` 实现），`pci.rs` 修 B-18（头部类型偏移 `0x0C` → `0x0E`，多功能设备的 function 1..7 才被枚举）；新增 `make test-ahci`（并入 `test-all` → 19 目标）与编排场景 `ahci`（20 场景）；目录树与模块表同步 |
| 2026-10-12 | **阶段五：构建与测试按责任拆分** —— `tools/pkgmgr/`（`ckc.py`+`cpm.py`）与 `tests/test_{ckc,cpm}.py` → `userland/pkgmgr/`，`tools/check_user_elf.py` → `userland/tools/`；`tests/orchestrate.json` 用 `tags` 分组 + `testorch -tag`/`-exclude-tag`；内核 `make test-all` 收敛为纯内核（18 目标），新增 `test-all-with-userland`/`test-orch-userland`，用户态侧 `make -C userland test-all`；目录树与模块表同步标注 |
| 2026-10-11 | H-06 P0a：`tools/` 新增 `pkgmgr/ckc.py`（CKC 包容器，后缀 `.ckc`；**阶段五迁至 `userland/pkgmgr/`**）；目录树与模块表同步标注 |
| 2026-10-11 | v0.9 第一步：`kernel/src/syscall/` 新增 `memory.rs`（用户堆 syscall），`libc/` 新增 `heap.rs`（`#[global_allocator]`）；目录树与模块表同步标注 |
| 2026-10-11 | v0.8 收官：`kernel/src/fs/` 新增 `fat.rs`（块设备上的 FAT 只读后端，模块表补一行）；`tools/mkfs/` 从占位变为实现（`mkfs.py`）；`tests/fatroot/`（测试盘内容 + mtools 复核说明）；`tests/test_boot.py` 支持 `--disk2`（双盘启动） |
