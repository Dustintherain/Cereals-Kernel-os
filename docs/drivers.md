# 设备驱动 / PCI / 磁盘（阶段文档）

- 阶段：第八阶段（v0.8 驱动框架 / PCI / 块设备；v0.9 网络栈；v1.0 驱动兼容性；v0.4 中断控制时钟键盘已完成）
- 最后更新：2026-10-14
- 状态：✅ v0.8 已完成（**驱动注册框架 + PCI 枚举 + 块设备抽象 + virtio-blk（legacy）**，`make test-driver` 23 项 PASS；**磁盘文件系统 FAT12/16/32 已挂到 VFS**，`make test-fat` 21 项 PASS）；**v1.0 驱动兼容性：AHCI/SATA 驱动已落地**（`make test-ahci` 24 项 PASS，见 1.9；顺带修掉 PCI 枚举的真缺陷 B-18 —— 多功能设备的 function 1..7 从来没被枚举过）；**v0.9 简化网络栈已落地**（**virtio-net（legacy）+ Ethernet → ARP → IPv4 → UDP**，见 1.8）+
**TCP/socket 补齐已落地**（内核 TCP 客户端最小集 + `socket/bind/connect/sendto/recvfrom` +
`/bin/tcpecho`，`make test-network` 13 → **24 项** PASS，见 1.10）；v0.4（中断控制器 + PIT + PS/2 键盘 + 上下文切换原语 + H-04/H-05 Go 工具）已完成
- 依赖：interrupt（IRQ 分发）、memory（连续物理帧 → DMA / virtqueue）、arch::port（端口 I/O）

> 本文档顶部保留阶段/最后更新/状态字段，与项目文档维护约定对齐。

## 1. 当前状态（2026-10-14）

### 1.1 v0.8 已落地：驱动框架 + PCI 枚举 + 块设备

代码位置与职责：

| 文件 | 职责 |
| ---- | ---- |
| `kernel/src/driver/pci.rs` | PCI 枚举（配置机制 #1：`0xCF8`/`0xCFC`）、BAR 探测与窗口大小计算、设备表、`print_bdf` |
| `kernel/src/driver/driver.rs` | 驱动注册表 + `Driver` trait（`name`/`matches`/`attach`）+ `probe_all()` 匹配与挂接 + `ProbeStats` |
| `kernel/src/driver/block.rs` | 块设备抽象 `BlockDevice`（按 512 字节扇区读写）+ 设备注册表 + 块设备验收自测 |
| `kernel/src/driver/virtio_blk.rs` | legacy virtio-blk 0.9.5 驱动：virtqueue 布局、描述符链、notify、used ring 轮询、bounce buffer |
| `kernel/src/driver/ahci.rs` | **v1.0**：AHCI/SATA 驱动（命令列表 / 接收 FIS / 命令表 + PRDT、IDENTIFY DEVICE、LBA48 读写 DMA EXT、同步轮询、bounce buffer）—— 真实机器上的磁盘控制器，见 1.9 |
| `kernel/src/arch/x86_64/port.rs` | 端口 I/O 统一入口（`inb/inw/inl` / `outb/outw/outl`） |
| `kernel/src/memory/frame.rs` | `allocate_contiguous(count)` / `free_contiguous(...)`：virtqueue 需要的**物理连续**帧 |

启动顺序（`kernel/src/main.rs`，在 `Kernel started!` 之后、调度器/用户态之前）：

```text
pci::init()            → 枚举 256×32 个 function，填设备表并逐设备打印一行
find_by_id(1af4,1001)   → 断言启动盘所在的 legacy virtio-blk 确实在总线上
driver::register_builtin() → 注册表里放入 virtio-blk-pci / virtio-net-pci / ahci-pci
driver::probe_all()    → 逐设备匹配（注册顺序优先）→ attach → 统计 matched/attached/failed
block::selftest(disk)  → 引导扇区 / 分块读 / 写回读 / 负例（启动盘，virtio-blk0）
ahci::block_index()    → 若有 SATA 盘，对**同一套** block::selftest 再跑一遍（`ahci0`）
fs::mount_disks()      → 把 FAT 卷挂到 /disk（启动盘）与 /mnt/fat（mkfs 测试盘，最长前缀路由）
fs::disk_selftest()    → 磁盘文件系统验收（读内核自己的映像 / 跨簇公式 / LFN / 偏移读 / EROFS）
```

最后两行是 v0.8 收官部分：磁盘的文件系统层（`kernel/src/fs/fat.rs`）与驱动层 1.1 表格里的其它模块
同属一个启动链 —— 驱动只是把扇区搬进内存，**FAT 与 VFS 才是「文件」的落点**，
详见 [filesystem.md](./filesystem.md) 第 3 节。

### 1.2 关键设计决策（为什么是这样）

**PCI 用配置机制 #1（`0xCF8`/`0xCFC`）而不是 ECAM。** 机制 #1 每次访问两条端口指令，不依赖 ACPI；
ECAM 需要先解析 MCFG（ACPI）表，那是引入 ACPI 之后才划算的优化。代价（约 8k 次端口读扫描全总线）
在启动日志里可忽略，换来的是「枚举」这条路径自成一体、可被单测形态的串口断言完全覆盖。

**只实现 legacy virtio（0.9.5），不做 modern（virtio 1.0）。** 设备 ID 决定接口：`0x1001`（legacy，
BAR0 是 I/O 窗口，寄存器是简单偏移）/ `0x1042`（modern，寄存器在 capability 指向的 MMIO 里，
队列布局也不同）。legacy 的实现量大约只有现代接口的三分之一，而 QEMU 用
`disable-modern=on` 就能稳定提供 legacy 设备；真遇到 modern 设备时 `attach` 会**打印明确原因并返回错误**
（不静默失败），错误会计入 `ProbeStats::failed`。

**块 I/O 走同步轮询，不用中断。** v0.8 不配置 MSI-X/INTx，每次请求提交后自旋等 used ring 的 `idx` 前进
（上限由 PIT tick 给出）。理由是：块 I/O 常在无法睡眠的路径上发生，而中断上下文里做块 I/O 是另一类
复杂度的开始。代价是等待期间忙等；v0.9 起再改为「中断 + 等待队列」。

**bounce buffer 换掉一整类 DMA 对齐问题。** 调用方缓冲长度只要整扇区对齐即可，驱动内部用一块
页对齐静态缓冲（8 扇区 = 4 KiB）分块搬运。因此不要求调用方缓冲物理连续、也不要求对齐；
一次性拷贝的成本换来「上层随便传缓冲」的可用性。

**驱动框架只做匹配与挂接，不做生命周期管理。** 注册表容量固定（`MAX_DRIVERS`），启动阶段写入、
此后只读；一个设备最多被一个驱动接管；`attach` 失败不阻断其它设备。上层（`main`）只需要看
`ProbeStats`，不需要知道总线上有多少种设备。

### 1.3 验收证据（`make test-driver`，23 项断言；以下是真实串口输出）

```text
pci: 00:03.0 vendor=0x1af4 device_id=0x1001 class=0x1 subclass=0x0 prog_if=0x0   ← legacy virtio-blk
pci: 00:03.0 bar0=io:0x6000 size=0x80                                            ← BAR0 是 I/O 窗口（legacy 的标志）
pci: 00:1f.2 vendor=0x8086 device_id=0x2922 class=0x1 subclass=0x6 prog_if=0x1   ← 芯片组的 AHCI（B-18 修好后才能看到）
pci: 00:1f.2 bar5=mmio:0xc10c4000 size=0x1000                                    ← ABAR 是 MMIO（0x1000 = 4 KiB）
pci: enumerated devices=7
pci: virtio-blk (legacy) at 00:03.0 class=0x1 subclass=0x0
driver: registered index=0 name=virtio-blk-pci
driver: registered index=1 name=virtio-net-pci
driver: registered index=2 name=ahci-pci
driver: match 00:03.0 -> virtio-blk-pci
virtio-blk: legacy BAR0=0x6000 size=0x80
virtio-blk: host features=0x71006ed4
virtio-blk: guest features=0x0
virtio-blk: queue0 size=256 region=0x100000 (3 pages contiguous)
virtio-blk: capacity=131072 sectors (64 MiB)
virtio-blk: status=0xf (DRIVER_OK)
virtio-blk: registered as block device 0 name=virtio-blk0
driver: attached 00:03.0 -> virtio-blk-pci
driver: match 00:1f.2 -> ahci-pci
driver: attached 00:1f.2 -> ahci-pci
driver: probe PASS (devices=7 matched=2 attached=2 failed=0)
block: registered devices=1
block: device=virtio-blk0 sector_size=512 sectors=131072 bytes=67108864
block: boot sector sig=0x55aa fstype=FAT16 oem=MTOO4032 sectors_per_cluster=2
block: chunked read PASS (512/4096/4608 bytes consistent at lba 0)
block: write/readback PASS (lba=131071 pattern=0xa5)
block: negative cases PASS (unaligned len, lba beyond capacity, overflow)
block: selftest PASS
```

这些断言的意义（「硬件事实」而不是日志措辞）：

- `pci: enumerated devices=7` 与 `matched=2 attached=2`：**B-18 修复**后 `00:1f.2`（AHCI）
  与 `00:1f.3`（SMBus）才被枚举到，芯片组的 AHCI 控制器也才被 `ahci-pci` 认领；
- `device_id=0x1001` + `bar0=io:`：QEMU 在 `disable-modern=on` 下暴露的**就是** legacy 接口，
  与驱动实现的前提一致（modern 是 `0x1042`）；
- `queue0 size=256 region=0x100000 (3 pages contiguous)`：队列深度来自设备，队列区来自
  `frame::allocate_contiguous`（描述符表 + avail + 4 KiB 对齐的 used 三段必须物理连续，
  因为 legacy 的队列地址寄存器只装一个 PFN）；
- `capacity=131072 sectors (64 MiB)`：容量由设备报出，与 `tools/image-builder/build_image.py`
  的镜像尺寸互相印证；
- `boot sector sig=0x55aa fstype=FAT16`：从磁盘真的读回了本次构建的 FAT 引导扇区
  （类型串取自 BPB 的根目录项数决定的位置，不是写死的字符串）；
- `chunked read PASS`：4608 字节的读**跨过**驱动内部 8 扇区（4 KiB）搬运边界，与 4096 / 512
  两次独立读的拼接结果逐字节一致 → 分块重组与 LBA 偏移换算正确；
- `write/readback PASS (lba=131071 ...)`：最后一个扇区写 → 读回 → **还原** → 再读回；
  用非 snapshot 方式启动后比对镜像的 FAT 区域，确认「还原」真的把盘恢复原样（见 1.5）；
  > 该扇区自 CKM-2 起落在**状态区**（镜像尾部 1 MiB）的末尾：它仍然被还原，
  > 而状态区的两个槽在区域**开头**（`ckm/store.rs`），两者不重叠；
- `negative cases PASS`：长度非整扇区、LBA 越过容量、LBA 溢出（`u64::MAX-1`）三种输入都必须
  报错 —— 最后一条曾经真的漏过（`lba + sectors` 环绕后看起来在容量内），见 B-16/B-17。

### 1.4 已知限制（明确未做，不要读成已完成）

- **磁盘文件系统是只读的**：FAT12/16/32 后端已挂到 VFS（`fs/fat.rs`），但 `write`/`create`/`mkdir`
  一律返回 `EROFS` —— 分配器与目录项改写属后续版本；也没有扇区缓存（每次访问都真读设备）。
  详情与限制清单见 [filesystem.md](./filesystem.md) 3.1。
- **不分配/重定位 BAR**：直接使用固件分配的地址。真实平台上若出现未分配窗口（全 0）会打日志而不是猜测
  （v0.8 的实现里 `read_bar` 对全 0 返回「未实现」）。
- **不解析 capability 链、不配置中断**：MSI-X/INTx 与设备特定能力结构仍未做；v0.9 的网络驱动同样走
  **同步轮询**（不配置 MSI-X/INTx），因此这条限制对块设备与网卡都成立（见 1.8）。
- **最多两块 virtio-blk 盘**（`block` 注册表容量 2，每块盘各自的队列状态在实例里、请求缓冲共用）：
  启动盘 + 测试盘刚好够用，更多盘位留给后续版本。
- **不实现 modern（virtio 1.0）接口**：见 1.2；遇到 modern 设备时明确报错。
- **queue 只支持 0 号队列**：块设备只有一个 virtqueue，够用；多队列留给 v0.9+。
- **AHCI 只做「已由固件带起来的端口」**：不做 COMRESET（对没有盘的端口做它要等完整超时，
  6 个端口就是 30 秒）；只用 0 号命令槽；不配置中断、不做 NCQ（不用 `PxSACT`）；
  不认 ATAPI（光驱）：`PxSIG` 不是 `0x101` 的端口明确跳过（见 1.9）。
- 端口 I/O 的新统一入口（`arch/x86_64/port.rs`）只被 PCI / virtio 使用；早期驱动（serial/keyboard/PIC）
  仍各自保留私有副本 —— 改造它们只会扩大 diff，与 v0.8 目标无关。

### 1.5 磁盘写入的「无损」验证

`block: selftest` 会写最后一个扇区再做还原。为确认还原有效（而不只是「读回自身缓存」），
用**不带 `snapshot=on`** 的方式启动一次并比对镜像的 **FAT 区域（前 63 MiB）**：

```text
before=26a5e771b617734a3ac211caff0fa189
after =26a5e771b617734a3ac211caff0fa189
IMAGE UNCHANGED (write test restored the scratch sector)
```

> **CKM-2（2026-10-12）起只比前 63 MiB**：镜像尾部 1 MiB 是 **CKM 状态区**
> （`kernel/src/ckm/store.rs` 的双槽 A/B + CRC32），启动时**按设计会被写入**
> （首次开机建立状态区，之后 `put` 只写非活动槽）。这是两个区域的两条不同性质，不是「无损」被破坏：
> 扇区 131071（scratch）的内容被还原，状态区按原子写更新 —— 第二次启动读回 `state=loaded` 就是
> 「读到的是盘上的字节」的直接证据（`make test-ckm-store`）。同理，`make run` 之后整盘 md5 会变。

测试套件里则统一用 `snapshot=on`（临时写时复制），既让并行场景共享同一镜像，也不污染原始文件；
只有 CKM-2 的持久化自测（`tests/test_ckm_store.py` + `--persist`）会真的落盘，且它用的是**独占副本**。

### 1.6 v0.4 已落地部分（中断控制器 + PIT 100Hz + PS/2 键盘 + 上下文切换原语）

- [x] 中断控制器骨架与 PIC 第一版（初始化、IRQ→向量映射、mask/EOI 原语）
- [x] `kernel/src/interrupt/` 绑定为 `controller` / `exception` / `irq` 三子模块
- [x] `kernel_main` 中接入 PIC 初始化（默认全屏蔽，不使能外设 IRQ）
- [x] 选型策略记录：`kernel/src/interrupt/pic_policy.md`
- [x] PIT/Timer 子系统（`kernel/src/time/`：通道 0 重装载值、tick 计数）
- [x] IRQ 分发入口与 16 槽 IRQ 注册表（`kernel/src/interrupt/irq.rs`）
- [x] IRQ0（PIT）回调：100Hz，每 100 tick 打印一次 `IRQ0_heartbeat tick=<n>`
- [x] PS/2 键盘中断输入（`kernel/src/driver/keyboard.rs`）：8042 初始化、IRQ1、set 1 解码
- [x] **行缓冲与终端式回显（v0.9）**：128 字节行缓冲 + 行编辑（退格、回显），`Enter` 把整行标为可取
      并打印 `kbd: line ready len=N`；`stdin`（fd=0）由 `take_line` 取走（`syscall/file.rs::read_stdin`）。
      回显协议由 v0.4 的 `KB <字符>` 改为**原样回显**（理由：终端语义；`make test-keyboard` 断言同步改为
      `kbd: line ready len=3`，`KBIRQ ENTRY/EXIT` 不变）
- [x] 上下文切换原语（`kernel/src/arch/x86_64/context.rs`）：`switch_to` / `init_context` + 内核自测
- [x] `make test-keyboard`：QEMU monitor `sendkey` 注入真实按键并断言回显

### 1.7 中断/时钟侧的已知状态与限制

- PIC 映射固定为 master/slave + IRQ0..15 → 向量 32..47；APIC（LAPIC/IOAPIC/redirection）仍为占位。
- IRQ 汇编入口（`irq_common`）保存/恢复 15 个通用寄存器，**不保存** XMM/FPU 状态（内核不用浮点）。
- 键盘行缓冲是**单行**（一行的容量 128、含行尾 `\n`）：还没有输入队列，上一行没被 `read` 取走时
  新按键被丢弃（`driver/keyboard.rs::LineBuffer` 文档说明了这个互斥约定）。多行队列/终端抽象等 Shell 之后的控制台阶段。
- 回显在**中断上下文直接打印**（cooked 模式）：同一串口通道上还有 `KBIRQ ENTRY/EXIT` 标记，
  可读性靠「一次击键一段标记」的约定，终端抽象尚未引入。

### 1.8 v0.9 已落地：简化网络栈（virtio-net → Ethernet → ARP → IPv4 → UDP）

代码位置与职责：

| 文件 | 职责 |
| ---- | ---- |
| `kernel/src/driver/net.rs` | 网络设备抽象 `NetworkDevice`（以太网帧收发）+ 注册表 + MAC 打印 |
| `kernel/src/driver/virtio_net.rs` | legacy virtio-net 0.9.5：两个 virtqueue（0 = 接收 / 1 = 发送）、10 字节 virtio-net 头、MAC 读自设备配置空间、同步轮询 |
| `kernel/src/network/ethernet.rs` | Ethernet II 组帧 / 解析（纯函数，不依赖设备） |
| `kernel/src/network/arp.rs` | ARP 请求 / 应答 + 固定容量 IP→MAC 缓存 + 无缓存 `resolve` 自由函数（TCP/socket 复用与 `Stack` 共用的唯一等待循环） |
| `kernel/src/network/ipv4.rs` | IPv4 地址、头组帧 / 解析、互联网校验和（`sum16` / `finish`） |
| `kernel/src/network/udp.rs` | UDP 组帧 / 解析（发送算 IPv4 伪头校验和；接收校验非零校验和） |
| `kernel/src/network/dhcp.rs` | DHCP DISCOVER 组包 + OFFER 解析（UDP 往返的验收载体） |
| `kernel/src/network/mod.rs` | `Stack`（设备 + 地址 + ARP 缓存）+ `selftest` 验收编排（ARP + DHCP + TCP 回声，见 1.10） |
| `kernel/src/network/tcp.rs` | **TCP 补齐**：客户端最小集（三次握手/按序收发/FIN 关闭，见 1.10） |
| `kernel/src/network/socket.rs` | **TCP 补齐**：AF_INET/SOCK_STREAM 套接字表（fd 16..=23，syscall 后端，见 1.10） |

启动顺序（挂载磁盘文件系统之后、调度器/用户态之前）：

```text
net::count()/first()      → 只有总线上挂了网卡才跑（无网卡时打印一行并跳过 —— 不把「没有网卡」当失败）
net::report()             → 打印设备名与 MAC
network::selftest(dev)    → ARP 解析网关 + DHCP DISCOVER → OFFER + TCP 回声往返（见 1.10）
```

**验收（`make test-network`，13 → 24 项串口断言）** —— QEMU 加一块 legacy virtio-net-pci，
`-netdev user`（slirp）自带 DHCP 服务器与虚拟网关；TCP 步另需测试侧在宿主起回声服务
（`--tcp-echo-port`，见 1.10 —— 这是 TCP 验收唯一需要宿主配合的一处）：

- `vendor=0x1af4 device_id=0x1000`：legacy 网卡被 PCI 枚举到（modern 0x1041 明确拒绝）；
- `virtio-net: mac=52:54:00:12:34:56`：MAC **从设备配置空间读到**（驱动协商了 `VIRTIO_NET_F_MAC`；
  兜底地址是 `02:00:00:00:00:01`，因此看到 QEMU 的默认值就只可能是「真的读了配置空间」）；
- `net: arp 10.0.2.2 -> 52:55:0a:00:02:02 (reply)`：ARP 广播请求 → 单播应答，MAC 正是 slirp 网关；
- `net: dhcp offer yiaddr=10.0.2.15 server=10.0.2.2 mask=255.255.255.0 router=10.0.2.2`：
  UDP 68→67 的 DISCOVER → OFFER 往返，报文里的地址 / 掩码 / 网关都被解析出来
  —— 这是 Ethernet → IPv4 → UDP 收发两条路径都通了的证据。

**边界（与 `docs/architecture.md` 第 5 节一致，别读成已完成）**：TCP 只做客户端最小集
（细节与不做项见 1.10）；不做 IP 分片重组（收到 `MF` 置位或 offset ≠ 0 的 IPv4 报文直接丢弃）；
不做 ICMP（`ping` 仍无）。中断仍不配置 —— 与块设备一样走同步轮询。

### 1.9 v1.0 已落地：AHCI/SATA —— 真实机器上的磁盘控制器

virtio-blk 只在虚拟化环境下存在；真实机器的内置盘挂在 **AHCI（SATA）** 控制器上。
q35 的芯片组本身就带一个 ICH9 AHCI 控制器（`00:1f.2`，`8086:2922`，class `0x01` /
subclass `0x06` / prog_if `0x01`）—— 也就是说，在这之前，内核**看不见这台机器上
唯一真实存在的磁盘控制器**。这一节记录补上它与修掉的那个真缺陷。

代码位置与职责：

| 文件 | 职责 |
| ---- | ---- |
| `kernel/src/driver/ahci.rs` | `Driver`（按**类别**匹配，不看厂商）+ `BlockDevice`（第二条块设备路径）：命令列表 / 接收 FIS / 命令表 + PRDT / IDENTIFY DEVICE / LBA48 读写 DMA EXT / 同步轮询 |
| `kernel/src/driver/pci.rs` | **B-18 修复**：头部类型读 `0x0E` 而不是 `0x0C`；新增命令寄存器读写（内存空间 + 总线主控）；MMIO BAR 第一次被真正用起来（`00:1f.2` 的 BAR5） |
| `kernel/src/driver/block.rs` | **不变** —— AHCI 与 virtio-blk 实现同一个 trait，FAT 后端 / VFS / CSS 存储层一行未改 |
| `kernel/src/main.rs` | 有 SATA 盘时对 `ahci0` 再跑一次 `block::selftest`（与启动盘同一套断言） |
| `tests/test_boot.py` | 新增 `--ahci` / `--ahci-disk`：把盘插到 ICH9 的 2 号端口（QEMU 里叫 `ide.2`；`sata.2` 不存在） |

**B-18（高）**：`pci.rs` 把「头部类型」读在偏移 `0x0C`（cache line size，恒为 0）
而不是 `0x0E`，于是 `header_type & 0x80` 永远为 0 —— `MULTI_FUNCTION` 判据永远为假，
**多功能设备的 function 1..7 从来没被枚举过**。q35 上的后果很具体：`00:1f.0`（ISA 桥）
的 function 2（SATA）与 function 3（SMBus）不在内核的设备表里，而固件（OVMF）看得见
（引导日志里有 `Pci(0x1F,0x2)/Sata(0x2,0xFFFF,0x0)`）。修好后 `pci: enumerated devices=5`
→ **7**。这一条以前不可能被现有断言拦住：`test-driver` 只看 virtio-blk（一个单功能设备），
而「设备数变多」既不是失败、也没有断言绑定它。

验收（`make test-ahci`，24 项断言）—— q35 自带控制器，因此 QEMU 命令行只多一块盘
（`-device ide-hd,drive=ahci0,bus=ide.2`），**不需要**加控制器：

```text
pci: 00:1f.2 vendor=0x8086 device_id=0x2922 class=0x1 subclass=0x6 prog_if=0x1
driver: registered index=2 name=ahci-pci
ahci: controller 00:1f.2 abar=0xc10c4000 size=0x1000
ahci: pci command=0x7 -> 0x7                          ← 固件已打开内存空间 + 总线主控
ahci: version=0x10000 cap=0xc0141f05 slots=6 implemented=0x3f (6 ports)
ahci: port 2 det=3 sig=0x101 (SATA disk)
ahci: port 2 model=QEMU HARDDISK capacity=131072 sectors (64 MiB) lba48=1
ahci: registered as block device 1 name=ahci0
ahci: controller 00:1f.2 drives=1 failures=0
driver: attached 00:1f.2 -> ahci-pci
driver: probe PASS (devices=7 matched=2 attached=2 failed=0)
block: registered devices=2
ahci: running the block selftest on the SATA disk (block index=1)
block: device=ahci0 sector_size=512 sectors=131072 bytes=67108864
block: boot sector sig=0x55aa fstype=FAT32 oem=CERLESSE sectors_per_cluster=1
block: chunked read PASS (512/4096/4608 bytes consistent at lba 0)
block: write/readback PASS (lba=131071 pattern=0xa5)
block: negative cases PASS (unaligned len, lba beyond capacity, overflow)
block: selftest PASS
storage: disk index=1 dev=ahci0 sectors=131072 table=none partitions=1
fs: mount point=/mnt/fat fs=fat32 sectors=131072 cluster_bytes=512 clusters=128992
fs: disk selftest PASS
```

这些断言的意义（同样是「硬件事实」而不是日志措辞）：

- `pci: 00:1f.2 ... prog_if=0x1`：**B-18 的直接证据** —— 这个 function 以前根本不在
  设备表里（`0x01`/`0x06`/`0x01` 是驱动匹配的全部依据：只看类别，不看厂商）；
- `pci command=0x7 -> 0x7`：内存空间 + 总线主控（DMA）在挂接时已打开（7 =
  `IO_SPACE|MEMORY_SPACE|BUS_MASTER`）—— 固件自己用这个控制器引导过；驱动仍然读改写
  一遍并打印前后值，真到没打开的真实平台上能看出是哪一步补上的；
- `model=QEMU HARDDISK capacity=131072 sectors (64 MiB) lba48=1`：型号与容量来自
  **IDENTIFY DEVICE（`0xEC`）的返回**（word 27..46 的型号、word 100..103 的 LBA48 容量），
  `lba48=1` 来自 word 83 的 bit 10 —— 全部是设备报的，不是常量；
- `port 2 det=3 sig=0x101 (SATA disk)`：`PxSSTS.DET == 3` 且 `PxSIG == 0x101`。
  同一台机器上默认的 **ATAPI 光驱**报的是 `sig=0xeb140101`，被明确跳过并打印
  `(not a SATA disk; skipped)` —— 「有设备但不是磁盘」不算失败，也不算成功；
- `block: device=ahci0 ...` 起的一整套 `block::selftest`：**与 virtio-blk 同一套断言**
  （引导扇区 / 跨 4 KiB 搬运边界的分块读 / 末扇区写回读后还原 / 负例）。
  「SATA 这条路也能读写」因此不是另写一份测试，而是同一批事实在另一条路径上成立；
- `storage: disk index=1 dev=ahci0 ...` + `fs: mount point=/mnt/fat fs=fat32 ...`：
  **同一张 FAT32 镜像**（`build/fat.img`）经 SATA 读出来，被 CSS-1 的卷扫描认出来、
  挂进 VFS；`fs: disk selftest PASS` 说明跨簇文件的逐字节公式也成立 ——
  与 virtio 那条路径给出的是同一个结论。

没有插盘时（其余每个测试场景都如此）：控制器照常被认领，日志给出
`ahci: port N det=0 sig=0xffff0101 (no device)` 与 `drives=0 failures=0`，
并打印 `ahci: no SATA drive attached; skipping the SATA block selftest` ——
**「总线上有控制器、没插盘」是常态，不是失败**（与 `net:` 无网卡时跳过同一约定）。

边界（不要读成已完成）：不做端口 COMRESET（端口必须由固件/前一级引导把 PHY 带起来，
否则视为没有盘）、只用 0 号命令槽、不配置中断、不做 NCQ；同样是「同一时刻一个请求在飞」
的同步模型，与 virtio-blk 的取舍一致。

### 1.10 TCP/socket 补齐（2026-10-14 落地）

v0.9 的网络只到 UDP，`network/tcp.rs` 与 `network/socket.rs` 是占位（见 1.8 当时的边界）。
这里补的是 **socket 可用的最小 TCP** —— 用户态五个 syscall + 内核 TCP 客户端最小集：

代码位置与职责：

| 文件 | 职责 |
| ---- | ---- |
| `kernel/src/network/tcp.rs` | 三次握手（SYN → SYN-ACK → ACK，5 次 × 1 秒重传）/ 按 MSS(1460) 切块发送 +
  累积 ACK + 重传（4 次 × 2 秒）/ 按序接收（`seq == rcv_nxt` 才收，乱序丢弃并重发 ACK）/
  RST（→ `Refused`）/ FIN 关闭（→ 完整 FIN-ACK 交换）/ `echo` 回声验收函数 |
| `kernel/src/network/socket.rs` | AF_INET/SOCK_STREAM 套接字表（fd 16..=23，与文件 fd 不重叠；`create`/`bind`/
  `connect`/`send`/`recv`/`close`；全局表 + 每次进用户态前清空） |
| `kernel/src/network/arp.rs` | 新增无缓存 `resolve` 自由函数（`Stack::arp_resolve` 改为调它，只留缓存查/填；
  TCP 与 socket 复用同一份等待循环，不另写第二份） |
| `kernel/src/syscall/net.rs` | 五个 handler（`socket/bind/connect/sendto/recvfrom` = 号 41/49/42/44/45）+
  `read`/`write`/`close` 落到套接字 fd 的路由（`read` = 首段即返，`write` = 全发） |
| `abi/src/sock.rs` | `AF_INET`/`SOCK_STREAM`/`IPPROTO_TCP` + `sockaddr_in` 布局（阶段三单一事实源） |
| `userland/user/tcpecho` | `/bin/tcpecho`：`connect(10.0.2.2:18080)` 后三轮收发（`sendto`/`recvfrom` +
  `write`/`read` + 3000 B 超 MSS 小缓冲读，同一连接，逐字节比对） |
| `tests/test_boot.py` | 新增 `--tcp-echo-port`：宿主回声服务（发什么回什么；内核经 slirp 网关连它） |

验收（`make test-network`，13 → **24 项**）—— 前 13 项（ARP/DHCP）不变，新增：

```text
net: tcp connect 10.0.2.2:18080 ESTABLISHED   ← SYN 的确认号对上 iss+1，第三次 ACK 已发
net: tcp send len=12 acked                   ← 12 字节被累积 ACK 覆盖（`cerlesse-tcp` 一段）
net: tcp recv len=12 match=PASS              ← 回声逐字节一致（分段回来也能拼合）
net: tcp close FIN-ACK PASS                  ← FIN 被确认 + 对端 FIN 已到 + 最终 ACK 已发
net: tcp PASS
tcpecho: round 1 sendto/recvfrom PASS        ← 用户态 44/45 号路径通
tcpecho: round 2 write/read PASS             ← 用户态 1/0 号落到套接字 fd 的路由通
tcpecho: round 3 large payload PASS len=3000 ← 超 MSS 分段 + 512 B 小缓冲读不丢字节
tcpecho: PASS
user: tcpecho PASS                           ← 内核自测跑 `/bin/tcpecho`，`exit(0)`
```

这些断言的意义（同样是「网络上回来的真实内容」）：SYN-ACK 的确认号是 slirp 的 TCP 栈
算出来的，回声字节是宿主回声服务原样送回来的，FIN 交换是两端各收各发 ——
任何一侧「假装通了」都会在字节比对或序号检查上露馅。内核自测与 `/bin/tcpecho`
调的是**同一套** `network::tcp` 代码（两条证据互相印证，不是两套实现各自为政）。

边界（不要读成已完成）：只做**客户端主动打开**（无 `listen`/`accept`）；无窗口通告与流控
（恒 65535）；无 SACK/时间戳/窗口扩大（发出的段无选项，收到的段选项跳过）；无乱序重组；
无 RTO 估算（固定超时 + 固定次数）；无 UDP 套接字（`SOCK_DGRAM` 具名拒绝）；无 ICMP。
无网卡时 TCP 与 `tcpecho` 都跳过（`net: no network device present` / `user: tcpecho SKIP`），
不是失败 —— 与无网卡跳过整个网络自测同一约定。

## 2. 开发路线

### 2.1 v0.8（Driver Manager → PCI → 块设备 → 磁盘 FS 挂载）

1. [x] 驱动框架：`Driver` trait + 注册表 + 匹配/挂接统计（`driver/driver.rs`）。
2. [x] PCI 枚举：遍历总线、解析类别/BAR、按 Vendor/Device ID 匹配。
3. [x] 块设备抽象（`block.rs`）与 virtio-blk（legacy）驱动（`virtio_blk.rs`）。
4. [x] **磁盘文件系统后端：FAT12/16/32 只读挂载进 VFS**（`fs/fat.rs` + `fs::mount_disks`，见 `docs/filesystem.md`）。
5. [x] 验收补齐：`make test-fat` 从磁盘读到文件（内核自己的 `/disk/…KERNEL.ELF` 与测试盘上的各文件）。
     注：Shell 里的 `ls`/`cat` 属 v0.9；VFS 层的磁盘读取已在 v0.8 验收。

验收（v0.8）：

- [x] PCI 列表输出，包含 Vendor/Device ID（`make test-driver`）
- [x] 磁盘读写可验证（引导扇区读 + 最后扇区写回读 + 分块读一致性 + 负例）
- [x] FAT 挂载完成，可在 VFS 层访问磁盘文件（`make test-fat` 21 项：读内核映像 / 跨簇逐字节公式 / LFN / 空文件 / 偏移读 / `EROFS` 负例）

### 2.2 v0.9（网络栈顺序 + Socket syscall 最小集）

1. [x] Network Driver：`virtio-net`（legacy，复用 v0.8 的 `Driver` 框架与 PCI 枚举）—— `driver/virtio_net.rs`
2. [x] Ethernet → ARP → IPv4 → UDP（`network/{ethernet,arp,ipv4,udp,dhcp}.rs`，验收见 1.8）
3. [x] TCP 客户端最小集 + Socket syscall 最小集（`network/tcp.rs` + `network/socket.rs` +
   `syscall/net.rs` + `/bin/tcpecho`，验收见 1.10；ICMP 仍未做）

验收（v0.9）：

- [x] Shell 交互可用（`make test-shell`，v0.9 第二步）
- [x] 网络最小闭环：ARP 解析网关 + DHCP 的 UDP 往返（`make test-network`）
- [x] TCP 回声往返：内核自测 + 用户态 socket 三轮收发（`make test-network` 13 → 24 项）
- [ ] ping 简例（ICMP 仍未做）；HTTP 简例可用 TCP 手写（未立项）

### 2.3 v1.0（驱动兼容性：从「虚拟化够用」走向「真机也能起来」）

1. [x] PCI 枚举修缺陷：多功能设备的 function 1..7 真的被枚举（**B-18**）——
      q35 上 `enumerated devices=5` → 7，芯片组的 SATA 与 SMBus 才看得见。
2. [x] AHCI/SATA 块设备驱动（`driver/ahci.rs`）：命令列表 / 接收 FIS / 命令表 + PRDT、
      IDENTIFY DEVICE、LBA48 读写 DMA EXT、同步轮询 + bounce buffer；与 virtio-blk
      实现同一个 `BlockDevice`（上层零改动）。
3. [x] 验收：`make test-ahci`（24 项，QEMU 把 `build/fat.img` 插到 ICH9 的 2 号端口）+ 编排场景 `ahci`。
4. [ ] AHCI 先进特性（NCQ / MSI-X / 多命令槽）与 NVMe、USB：按需补（见 3.2）。

验收（v1.0 驱动兼容性）：

- [x] 芯片组的 AHCI 控制器被认领并有明确证据（`ahci: controller 00:1f.2 drives=… failures=0`）；
- [x] SATA 盘跑通与 virtio-blk **同一套**块设备验收（`block::selftest`）；
- [x] SATA 上的 FAT32 镜像能被 CSS-1 识别并挂进 VFS，文件内容逐字节比对通过；
- [x] 没插盘时明确跳过（不是失败）：`drives=0` + `skipping the SATA block selftest`。

## 3. 驱动列表

### 3.1 已实现

| 驱动 | 版本 | 说明 |
| ---- | ---- | ---- |
| Serial（串口） | v0.1 | 开发期唯一日志通道（`print`/`println`/`print_dec`/`print_hex`/`print_hex_pad`/`put_bytes`） |
| Keyboard（PS/2） | v0.4 | IRQ1 扫描码解码与串口回显 |
| Timer（PIT） | v0.4 | 100Hz tick：调度时间片与块 I/O 超时基准 |
| PCI | v0.8 / v1.0 | 枚举与 BAR 探测（配置机制 #1 + 命令寄存器读写）；v1.0 修 B-18（头部类型偏移 `0x0C` → `0x0E`，多功能设备才被枚举） |
| Disk（virtio-blk legacy） | v0.8 | 块设备：`BlockDevice` trait 的实现者（虚拟化环境） |
| Disk（AHCI/SATA） | v1.0 | 块设备：`BlockDevice` 的第二个实现者 —— **真实机器上的磁盘控制器**（见 1.9） |
| Net（virtio-net legacy） | v0.9 | 网卡：`NetworkDevice` trait 的实现者（RX/TX 双队列） |
| Net（TCP 客户端） | TCP 补齐 | `network/tcp.rs`：三次握手 + 按 MSS 切块发送 + 累积 ACK + 重传 + 按序接收 + RST/FIN（见 1.10） |
| Net（Socket） | TCP 补齐 | `network/socket.rs` + `syscall/net.rs`：AF_INET/SOCK_STREAM 套接字表（fd 16..=23）+ `socket/bind/connect/sendto/recvfrom`（号 41/49/42/44/45），`read`/`write`/`close` 按号段路由（见 1.10 与 syscall.md 3.13） |
| Disk FS（FAT12/16/32） | v0.8 | 不是 PCI 驱动，而是 `FileSystem` 后端（`fs/fat.rs`）：消费 `BlockDevice` 提供的扇区 |

### 3.2 后续（按需补）

- NVMe（AHCI/SATA 已于 v1.0 落地，见 1.9）、USB、Audio、GPU、Framebuffer（视需要）。
- 块设备侧进阶：AHCI 的中断（MSI-X）+ NCQ（`PxSACT`）+ 多命令槽（当前是同步单请求）。

## 4. 非目标 / 后续

- SMP 多核下的中断均衡与高级 APIC 语义：v1.0 之后。
- USB、Wi-Fi、GPU、完整 TCP/IP 工业栈：非第一版目标。
- DMA 缓冲管理细节：v0.8 只做了「连续物理帧」这一最小的 DMA 前置条件；IOMMU/散列（scatter-gather）
  留给真实硬件需求出现时。

## 5. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-06 | 第一阶段创建占位 |
| 2026-10-07 | 补记当前状态：PIC 第一版已接入 kernel_main；v0.4 剩余（Timer/键盘/切换原语）与 v0.8/v0.9 待编写 |
| 2026-10-08 | v0.4 完成：IRQ stub 修正（B-08）、IRQ 入口保存寄存器（B-09）、PIT 100Hz、PS/2 键盘回显、上下文切换原语；新增 `make test-keyboard` |
| 2026-10-08 | v0.4 最后一项 H-04/H-05 落地：Go 编排器 + 串口分析器；`make go-test` / `make test-orch` 通过 |
| 2026-10-08 | 统一文档顶部元信息与修订记录（阶段 / 最后更新 / 状态） |
| 2026-10-11 | **v0.9 输入侧**：键盘驱动加行缓冲（行编辑 + 整行交付 + `kbd: line ready len=N`），回显由 `KB <字符>` 改为终端式原样回显；1.6 补记、1.7 限制项由「无环形缓冲」改为「单行缓冲 + cooked 回显」；`make test-keyboard` 断言同步 |
| 2026-10-11 | **v0.8 收官**：磁盘文件系统 FAT12/16/32（`fs/fat.rs`，只读）挂到 VFS，`fs::mount_disks` 接入启动链（`/disk` + `/mnt/fat`）；H-03 `tools/mkfs/mkfs.py` 落地（mtools 独立复核）；新增 `make test-fat`（21 项，双盘）与编排场景 `fat`（10 场景）；状态行改为「v0.8 已完成」，限制项从「FAT32 未做」改为「FAT 只读、无扇区缓存」 |
| 2026-10-11 | **v0.8 前半落地**：新增 `driver/pci.rs`（机制 #1 + BAR 探测 + 窗口大小）、`driver/driver.rs`（注册框架 + `probe_all`）、`driver/block.rs`（按扇区读写 + 验收自测）、`driver/virtio_blk.rs`（legacy virtio 0.9.5，同步轮询 + bounce buffer）、`arch/x86_64/port.rs`（端口 I/O）、`frame::allocate_contiguous`；启动盘改为 virtio-blk（Makefile / `tests/test_boot.py` / `tests/test_keyboard.py`，`disable-modern=on`）；新增 `make test-driver`（22 项）与编排场景 `driver`（9 场景）；修 B-16（BAR 大小 32→64 位宽错误）、B-17（LBA 溢出未拦）；状态行由「v0.8 尚未编写」改为「v0.8 前半完成，FAT32 待做」 |
| 2026-10-11 | **v0.9 简化网络栈落地**：新增 `driver/net.rs`（`NetworkDevice` trait + 注册表）、`driver/virtio_net.rs`（legacy virtio-net 0.9.5：RX/TX 双队列、10 字节 virtio-net 头、MAC 读自设备配置空间、同步轮询）、`network/{ethernet,arp,ipv4,udp,dhcp}.rs` 与 `network/mod.rs`（`Stack` + `selftest`）；`network/{tcp,socket}.rs` 占位说明更新；`driver/driver.rs::register_builtin` 追加 virtio-net；`main.rs` 在磁盘验收后跑网络自测（无网卡时跳过）；新增 `make test-network`（13 项，QEMU 加 legacy virtio-net-pci + `-netdev user`）与编排场景 `network`（12 → **13 场景**）；`tests/test_boot.py` 增 `--nic`；新增 1.8 节与状态行、驱动列表更新 |
| 2026-10-14 | **TCP/socket 补齐**：`network/tcp.rs` 重写为客户端最小集（三次握手 + 按 MSS 切块 + 累积 ACK + 重传 + 按序接收 + RST/FIN + `echo`）、`network/socket.rs` 重写为套接字表（fd 16..=23）、`network/arp.rs` 新增无缓存 `resolve`（`Stack::arp_resolve` 改调它）、新增 `syscall/net.rs`（41/49/42/44/45 五个 handler + `read`/`write`/`close` 套接字路由）与 `abi/src/sock.rs`（`NAMED` 26 → 31）+ 4 个 errno、新增 `/bin/tcpecho`（两轮收发）与 `tests/test_boot.py --tcp-echo-port`（宿主回声服务）；`make test-network` 13 → **23 项**，编排 `network` 场景同步；1.8 边界改写、新增 1.10 节、2.2 第三项打勾、驱动列表加两行；回归 `test-user`/`test-programs`/`test-driver`/`test-fs` 全 PASS |
