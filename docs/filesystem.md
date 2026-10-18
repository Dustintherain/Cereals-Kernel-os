# 文件系统（阶段文档）

- 阶段：第六阶段（v0.7）+ 第八阶段（v0.8 磁盘后端）
- 最后更新：2026-10-11
- 状态：✅ v0.7 核心已实现并验收（VFS + RAMFS + 挂载点路由 + fd 表 → `open`/`read`/`write`/`close`，
  `make test-fs` 20 项串口断言 PASS）；**v0.8 磁盘后端（FAT12/16/32，只读）已挂到同一套 VFS** 并验收
  （`make test-fat` 21 项），H-03 `mkfs`（`tools/mkfs/mkfs.py`）同步落地；
  **CSS-2（VFS 收口）亦已落地**：后端能力位/卷信息、挂载表带卷身份与多挂载点、卸载、挂载错误改 errno

> 本文档顶部保留阶段/最后更新/状态字段，与项目文档维护约定对齐。
- 依赖：syscall.md（open/read/write/close）、memory.md（内核堆：RAMFS 的数据就在堆上）
- 后续（规划）：磁盘/分区/卷/盘符、挂载点策略、文件系统探测与更多格式（exFAT/ext2/ext4/NTFS/ISO9660）
  归 **CSS 存储管理**（[storage.md](./storage.md)，提案 v1）；本文只描述**当前**的 VFS/RAMFS/FAT 实现

## 1. 概述

文件系统阶段分两步，两步均已完成：

1. **VFS 抽象 + RAMFS 后端**（v0.7）：把「文件」这件事的语义先立起来，
   并让 syscall 层的文件语义有真实落点；
2. **磁盘后端**（v0.8）：FAT12/16/32 只读后端 + 块设备挂到同一套 VFS 上，并补 `mkfs`（H-03）——
   验证方式是「**syscall 层一行都不改**，`open`/`read`/`close` 直接能读磁盘上的文件」。

非目标重申：不把具体文件系统写死进内核（ADR-004），`FileSystem` trait 是唯一入口；
`syscall` 层只认路径与 fd，不认「RAMFS 还是 FAT32」。

## 2. v0.7 实际实现（2026-10-11 落地）

### 2.1 模块划分（`kernel/src/fs/`）

| 模块 | 职责 | 关键类型/函数 |
| ---- | ---- | ---- |
| `inode.rs` | 与后端无关的节点形状 | `NodeKind`（File/Dir）、`Metadata`（kind/size/`perms()`）、`DirEntry` |
| `vfs.rs` | 抽象 + 路径工具 + 系统级入口 | `trait FileSystem`、`normalize/components/within`、`stat/read_all/read/write/create/mkdir/list/usage/backend_name`，**CSS-2**：`FsCaps`/`VolumeInfo`/`MountOptions`、`route/backend_caps/backend_volume_info`、`ENOTSUP` |
| `mount.rs` | 挂载表（**最长前缀路由**；CSS-2 拆成「后端 / 挂载点」两个表） | `MountTable::mount/mount_volume/alias_volume/umount/resolve/route/sync_counts`、`MountEntry { point, backend, vol_id, options }`、全局 `mount::table()` |
| `ramfs.rs` | 第一个后端（内存 FS） | `RamFs { root: Node }`、`Node { name, kind, data: Vec<u8>, children: Vec<Node> }` |
| `file.rs` | fd 表（`open` 的落点） | `FdTable`、`open/read/write/close/reset`、`O_RDONLY/O_WRONLY/O_RDWR/O_CREAT` |

`FileSystem` trait（v0.8 的形状；v0.7 曾用 `&self` + 借用返回，见下方说明）：

```rust
pub trait FileSystem {
    fn name(&self) -> &'static str;
    fn stat(&mut self, path: &str) -> Result<Metadata, i64>;
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i64>;
    fn read_all(&mut self, path: &str) -> Result<Vec<u8>, i64>;
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize, i64>;
    fn create(&mut self, path: &str) -> Result<(), i64>;
    fn mkdir(&mut self, path: &str) -> Result<(), i64>;
    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, i64>;
    fn usage(&self) -> (usize, usize) { (0, 0) } // 默认实现：磁盘后端不适用
    // CSS-2 追加的两个只读查询（都带保守缺省：只读、无卷标 → 猜错的代价是数据被写坏）
    fn caps(&self) -> FsCaps { FsCaps::read_only_unknown() }
    fn volume_info(&self) -> VolumeInfo { VolumeInfo::unknown() }
}
```

v0.8 把方法都收 `&mut self`、并把 `read_all` 从「借用后端内部缓冲」改成**返回 `Vec<u8>`**：
挂载表本就用 `&'static mut` **独占**持有后端，而磁盘后端的文件内容根本不在内存里 ——
要「借用」就得先把整个文件读进一个缓存，那还不如直接把这次读的结果交出去。
代价是比 RAMFS 多一次拷贝（小文件可忽略），收益是磁盘后端不需要任何内部可变性。

约定两条：

- 方法收的是**挂载点内的相对路径**（挂载点前缀的剥离由 `MountTable::resolve` 完成）；
- 错误一律是**负数 errno**（`-ENOENT`/`-EISDIR`/`-ENOTDIR`/`-EEXIST`/`-EINVAL`/`-ENOSPC`/`-EROFS`/`-EIO`/`-EUCLEAN`，
  CSS 又追加 `-ENODEV`（设备消失）与 `-ENOTSUP`（盘在、格式没后端）），
  syscall handler 因此可以把文件系统错误直接返回用户态，中间不需要翻译层；
- **挂载接口同口径**（CSS-2）：`mount`/`mount_volume`/`alias_volume`/`umount` 返回 `Result<_, i64>`
  （`EEXIST` 挂载点重复 / `ENOSPC` 表满 / `ENOENT` 未挂载 / `EINVAL` 卸载根），
  `FatFs::open` 返回 `OpenError { errno, reason }` —— 原因串只进证据行，不进错误码；
  这也把 v0.8 的「挂载失败返回 `&'static str`」修掉了（那种字符串不可断言）。

### 2.2 挂载表：路由是「可观测行为」而不是隐含逻辑

`MountTable::resolve(path)` 做**最长前缀匹配**，返回 `(后端, 挂载点内相对路径)`。
它存在的意义不只是「支持多个 FS」：**它是唯一把用户看到的路径变成某个后端内相对路径的地方**，
`vfs::stat/read/write/...` 每个入口都经过它，因此「请求落到了哪个文件系统」可以用串口断言验证：

```text
fs: mount point=/ fs=ramfs
fs: mount point=/mnt fs=ramfs-scratch
fs: route /mnt/note.txt fs=ramfs-scratch      ← 路由到 /mnt 的后端，而不是根后端
fs: readback /mnt/note.txt len=7 data=v0.7-ok ← 读回的确实是被路由过去的后端里的数据
```

后端用 `Box<dyn FileSystem>` **由挂载表独占持有**（不是 `static mut`）：所有访问都经
`resolve()` 取一次 `&mut`，不存在两个 `&mut` 同时指向同一后端的情形。

CSS-2 把「后端」与「挂载点」分成两张表（`backends` / `mounts`）：**一个后端可以被多个挂载点引用**
（同一个卷挂到 `/volumes/<label>` 与 `/media/<label>` 是同一份数据，不是两次打开），
`MountEntry` 里存的是**后端下标**而不是后端本身。这也让卸载变得可做：
`umount(point)` 只摘条目，后端不释放（可能还有别的挂载点引用它）；
挂载/卸载都会把「哪个卷挂了几个点」写回 `storage` 的卷记录（偏移 56），
`storage: mount point=… vol=…` / `storage: unmount point=… vol=…` 是它的证据行。

### 2.3 路径语义（刻意收窄）

| 输入 | 结果 |
| ---- | ---- |
| `/etc/motd`、`etc/motd` | 都规范化为 `/etc/motd`（允许省略前导 `/`） |
| `//a//b/` | `/a/b`（折叠重复与结尾斜杠） |
| `.` / `..` / 空串 | `EINVAL`（没有 cwd、没有父目录链接 —— 明确拒绝而不是猜一个含义） |
| `/etc/motd/deeper`（路径中间是文件） | `ENOTDIR` |
| `/nope` | `ENOENT` |

v0.7 **没有**：软/硬链接、权限位（`perms()` 返回固定串 `-rw-r--r--` / `drwxr-xr-x`，权限在 v1.0 的
`security/` 里做）、`rename`/`unlink`/`truncate`、稀疏文件（`write` 的 `offset > len` 返回 `EINVAL`）、
页缓存/写回（数据直接在内核堆上）。

### 2.4 RAMFS

- 数据模型就是 `Node { name, kind, data: Vec<u8>, children: Vec<Node> }`，**没有 inode 号、没有块分配、
  没有日志**；文件内容直接放在内核堆里，因此 `fs::init()` 必须在 `memory::init`（堆就绪）之后调用；
- 单文件上限 `MAX_FILE_BYTES` = 256 KiB（挡住用户态一次 `write` 把内核堆写爆的路径，超出返回 `ENOSPC`）；
- 目录列举按**插入顺序**返回（不排序），`ls -l` 风格的权限串来自 `Metadata::perms()`。

### 2.5 初始文件树（由 `fs::init` 建立）

```text
/            ramfs
├── init     用户程序 ELF（内核内嵌，见 syscall.md 3.1）
├── bin/     全部外部程序（内核内嵌；由 Shell fork+execve 起来，见 syscall.md 3.7/3.8）
│   ├── sh   交互式 Shell ELF（v0.9 第二步）
│   ├── hello    外部程序：pid + argv（v0.9 第三步）
│   ├── ls / cat / echo / ps   基础用户程序（v0.9 第五步）
│   └── dynhello   动态链接主程序（ET_DYN PIE；v0.9 第六步，见 syscall.md 3.9）
├── lib/     v0.9 第六步：**被装载的对象**（不是被运行的程序）—— 内核按 PT_INTERP/DT_NEEDED 的名字在此查找
│   ├── ld.so         PT_INTERP 的落点（被装载、被重定位，但不被进入）
│   └── libgreet.so   DT_NEEDED=libgreet.so 的落点（导出 greet）
├── etc/
│   └── motd 文本文件（23 字节，用户程序用 open/read 读它）
└── tmp/     用户程序写文件的落点
```

启动日志（`make test-fs` 的断言来源；每条都是 `create` + `write` 各一行）：

```text
fs: mount point=/ fs=ramfs
fs: mkdir /bin
fs: create /init
fs: write /init bytes=29096
fs: create /bin/sh
fs: write /bin/sh bytes=29096
...             （/bin/hello、/bin/ls、/bin/cat、/bin/echo、/bin/ps、/bin/volumes、/bin/tcpecho、/bin/dynhello 同形：create + write bytes=<ELF 大小>）
fs: mkdir /lib
fs: create /lib/ld.so
fs: write /lib/ld.so bytes=<ELF 大小>
fs: create /lib/libgreet.so
fs: write /lib/libgreet.so bytes=<ELF 大小>
fs: mkdir /etc
fs: create /etc/motd
fs: write /etc/motd bytes=23
fs: mkdir /tmp
fs: ramfs files=13 bytes=<总计>    （init + 9 个程序 + motd + /lib 的 2 个对象；目录不计入文件数）
```

（ELF 字节数随构建变化，断言只写前缀；`files=13` 是**清单长度**的后果 —— 加一个程序/库就要同步
`syscall/user.rs::PROGRAM_IMAGES`/`LIBRARY_IMAGES`、`kernel/build.rs` 的构建清单与这里。）

### 2.6 fd 表与 syscall 对接

fd 编号（与 Linux 一致的形状，见 syscall.md 2.7）：

| fd | 含义 | v0.7 行为 |
| ---- | ---- | ---- |
| 0 | stdin | **未接通**：`read` 返回 0（EOF），不让用户程序卡住 |
| 1 / 2 | stdout / stderr | 写串口（v0.6 行为不变，`user: write fd=1 len=N`） |
| ≥ 3 | RAMFS 文件 | 由 `fs::file` 的 fd 表管理（`FileHandle { path, offset, mode }`） |

- `open` 取**最小空闲槽位**，所以 `close(3)` 之后 `open` 会再次拿到 fd 3（实测：
  `user: close fd=3` → `user: open path=/tmp/hello.txt flags=0x41 fd=3`）；
- 偏移量由 `read`/`write` 推进，这是 v0.7 唯一的位置状态（没有 `lseek`、没有 `O_APPEND`）；
- 每次进入用户态前 `fs::file::reset()` 清空动态槽位（一次只跑一个用户程序，
  「每进程 fd 表」等 v0.8 的进程对象）。

### 2.7 验收（v0.7，`make test-fs` 19 项串口断言 PASS）

- [x] 挂载与初始文件树：`fs: mount point=/ fs=ramfs` / `fs: create /init` / `fs: write /etc/motd bytes=23` / `fs: mkdir /tmp`
- [x] 元数据与内容：`fs: stat /etc/motd kind=file size=23 perms=-rw-r--r--`、`fs: read /etc/motd len=23 data=motd: hello from ramfs`
- [x] **挂载点路由**：`fs: mount point=/mnt fs=ramfs-scratch` + `fs: route /mnt/note.txt fs=ramfs-scratch` + `fs: readback /mnt/note.txt len=7 data=v0.7-ok`
- [x] 目录列举（`ls` 语义）：`fs: ls / entries=5` + 每项 `name/kind/size/perms`
      （v0.7 为 3；v0.9 第二步新增 `/bin` → 4，第六步新增 `/lib` → 5——这是个精确断言：
      根条目变化必须同步改 `fs::selftest` 与测试期望）
      > **CKM-2（2026-10-12）之后有两条边界要分清**：`fs::selftest` 跑在 `/ckm` 建立**之前**，
      > 所以这里仍是 5；用户进程看到的 `/` 是 **6**（多了内核的 `/ckm`，覆盖区 `/ckm/overlay` 的家，
      > 见 `docs/ckm.md` 6.1/9.2）——`make test-programs` 断言的就是后者（`ls: / entries=6`）。
- [x] 负例 errno：`fs: negative cases PASS (ENOENT/ENOTDIR/EISDIR/EEXIST/EINVAL)`
- [x] 文件 syscall 全链路（随 `make test-user`，v0.7 时 32 项、v0.9 起 **61 项**断言）：`user: open path=/etc/motd flags=0x0 fd=3`
      → `user: read fd=3 want=64 got=23` → 内容经 `write` 打到串口 → `user: read fd=3 want=64 got=0`（EOF）
      → `user: close fd=3` → `user: open path=/tmp/hello.txt flags=0x41 fd=3` → `user: write fd=3 len=24`
      → 内核读回 `fs: readback /tmp/hello.txt len=24` + `fs: user data: written by user program`
- [ ] Shell 里 `cd`/`ls`/`cat` RAMFS 内文件 → 属 v0.9（需要 `init`/`sh` 与 `argc/argv`；v0.7 在**内核侧**
      与**一个用户程序**里验证了同样的语义，见 `user/hello`）
- [x] 磁盘后端（FAT）+ `mkfs`（H-03）→ v0.8 已落地，见第 3 节（`make test-fat` 21 项 PASS）

## 3. v0.8 实际实现：FAT 磁盘后端（`kernel/src/fs/fat.rs`）

### 3.1 为什么是 FAT，而不是 EXT2 / 自创格式

启动盘本身就是 FAT（UEFI 固件的 ESP 要求）。所以「同一个 FAT 实现」既能读我们自己生成的测试盘，
也能读**固件写的真实盘** —— 内核因此能打开自己的 `/disk/…KERNEL.ELF`。
格式选择由**已有的事实**决定，而不是由喜好决定（也避免「自己造格式自己写解析器」这种自证）。

覆盖范围：

| 项 | 支持情况 |
| ---- | ---- |
| 类型 | **FAT12 / FAT16 / FAT32** 全部按 BPB 自动判定（由**簇数**与布局决定，不靠扩展名或配置） |
| 目录 | 12/16 的**固定根目录区**、32 与所有子目录的**簇链**；跳过 `.`/`..`/卷标/已删除项 |
| 名字 | 8.3 短名（应用 NT 大小写标志）+ **LFN 长文件名**（按序号定位拼装 + 校验和验证） |
| 文件 | 任意偏移读（沿簇链走）、整文件读；空文件（首簇 0）**正确返回 0 字节**而不是去解引用簇 0 |
| 写 | **不支持**：`write`/`create`/`mkdir` 一律 `EROFS`（分配器 + 目录项改写属后续版本） |
| 缓存 | 没有：每次访问都真读设备（扇区缓冲在栈上），加扇区缓存是后续优化 |
| 一致性 | 簇链断/越界/出现空闲标记 → `EUCLEAN`，**不静默返回半截数据** |

已知限制（写清楚，避免「看起来支持」）：

- 只接受 **512 字节扇区**的卷（`BPB_BytsPerSec == 512`）—— 块设备本身就是 512 字节粒度，
  而 QEMU/固件都用 512；
- 不解析分区表（MBR/GPT）：v0.8 把**整盘当一个卷**（超级软盘布局），与启动盘一致；
- 时间戳/属性只读不写；没有 `..` 路径语义（VFS 层统一拒绝 `.`/`..`）。

### 3.2 挂载：两个卷，同一套路由

`fs::mount_disks()`（在 `Kernel started!` 之后、调度器之前）把块设备注册表里的卷挂到 VFS：

| 块设备 | 挂载点 | 内容 |
| ---- | ---- | ---- |
| 0（启动盘） | `/disk` | 固件要用的 ESP：`EFI/BOOT/BOOTX64.EFI` + `KERNEL.ELF` |
| 1（测试盘） | `/mnt/fat` | `tools/mkfs/mkfs.py` 生成，内容见 `tests/fatroot/README.md` |

`/mnt/fat` 恰好挂在 `/mnt`（自测用的第二个 RAMFS）**下面**，所以顺带验证了挂载表的**最长前缀路由**：

```text
fs: mount point=/disk fs=fat16 sectors=129024 cluster_bytes=1024 clusters=64244
storage: mount point=/disk vol=fs:a2ff2e36 ro=ro-backend     ← CSS-2：卷身份 + 只读原因
fs: mount point=/mnt/fat fs=fat32 sectors=131072 cluster_bytes=512 clusters=128992
storage: mount point=/mnt/fat vol=fs:11102620 ro=ro-backend
fs: route /mnt/note.txt fs=ramfs-scratch /mnt/fat/hello.txt fs=fat32
```

> **启动盘的 `sectors=129024`（63 MiB）不是设备的容量**：设备仍然报 `sectors=131072`（64 MiB，
> 见 `block: device=virtio-blk0 … bytes=67108864`），但 CKM-2 起镜像尾部 1 MiB 是**状态区**，
> FAT 只覆盖前面 63 MiB（`tools/image-builder/build_image.py` 先在 63 MiB 上 `mformat`，
> 再把保留区追加回去）。第二块盘（`/mnt/fat`，`mkfs.py` 生成）的 131072 是它**自己的**容量，两者无关。

`ro=` 不是写死的字面量：它来自后端自己的能力位（`FatFs::caps().read_only` → `ro-backend`；
RAMFS 是可写后端）。CSS-2 的挂载自测（`fs::storage_mount_selftest`，每次启动都跑）还会：
把后端报的序列号/卷标与 `storage_volumes` 记录逐字段比对（两条读同一张 BPB 的路径必须同一个答案，
证据行 `storage: volcheck`）、给启动卷加一个别名挂载点 `/volumes/<卷标>` 并断言两个路径读到**同一段字节**
且 `vol_id` 相同、卸载后路径变 `ENOENT` 而卷仍在表里（`mounts=1`）。

这里有个容易写错的地方（实际踩过）：判断「测试盘是否已挂」不能用 `backend_name("/mnt/fat").is_some()`——
没有第二块盘时它依然返回 `ramfs-scratch`（最长前缀落到挂在 `/mnt` 的后端），于是单盘启动会在
「检查测试卷」上失败并 halt。正确判断是 `vfs::is_mount_point`（路径是否**就是**挂载点本身，而不是「落在别人挂载点下面」的普通路径）。

### 3.3 验收（`make test-fat`，21 项串口断言 PASS，双盘启动）

```text
fs: ls /disk entries=2
fs: ls /disk name=EFI kind=dir size=0 perms=drwxr-xr-x
fs: ls /disk name=KERNEL.ELF kind=file size=3005280 perms=-rw-r--r--
fs: read /disk/KERNEL.ELF magic=0x7f454c46 size=3005280 (our own kernel image, read from the boot volume)
fs: read /disk/EFI/BOOT/BOOTX64.EFI magic=0x4d5a route=fat16
fs: ls /mnt/fat name=dir kind=dir size=0
fs: ls /mnt/fat name=a long file name.txt kind=file size=13
fs: ls /mnt/fat name=empty.txt kind=file size=0
fs: ls /mnt/fat name=hello.txt kind=file size=17
fs: content check PASS /mnt/fat/dir/nested.bin len=3000 (formula verified byte by byte across 6 clusters)
fs: offset read PASS /mnt/fat/dir/nested.bin offset=1024 len=8 data=0x3 (matches the formula)
fs: empty file PASS /mnt/fat/empty.txt len=0
fs: long file name PASS /mnt/fat/a long file name.txt (LFN entry)
fs: nested directory PASS /mnt/fat/dir/deep/marker.txt
fs: disk negative cases PASS (ENOENT/ENOTDIR/EISDIR/EROFS)
fs: disk selftest PASS
```

这些断言的意义（都是「磁盘上的事实」而不是日志措辞）：

- `magic=0x7f454c46` + `size=3005280`：从**启动盘**读回了**内核自己的映像**（前 4 字节是 ELF 魔数，
  大小是 FAT 目录项里记录的真实字节数）—— VFS → FAT → 块设备 → virtio 的完整往返；
- `BOOTX64.EFI magic=0x4d5a`（`MZ`）：固件要用的 PE 引导文件在同一张盘上也能读到；
- `content check PASS … len=3000`：跨簇文件（512 字节簇 → **6 个簇**）**逐字节对公式**
  （`bytes((i*7+3)&0xFF)`，内核自己算期望值，不依赖宿主预先算好的校验和）；
- `offset read PASS … offset=1024`：任意偏移读 —— `read`/`open` 走的正是这条路径；
- `empty file PASS len=0`：空文件首簇为 0，必须返回 0 字节；
- `long file name PASS`：带空格的**长文件名**（LFN 目录项）路径可解析；
- `nested directory PASS`：二级子目录（簇链 + `.`/`..`）；
- `negative cases PASS (ENOENT/ENOTDIR/EISDIR/EROFS)`：错误各自具体，只读后端不能用「写失败」笼统表示。

单盘启动（如 `make test-driver`）时内核打印 `fs: no second volume (single-disk boot; ...)`（提示语里点名 `make test-fat`），
启动链继续走到调度器/用户态 —— 即「不挂测试盘也能正常启动」同样被覆盖。

### 3.4 mkfs（H-03，`tools/mkfs/mkfs.py`）

- 自己实现 FAT12/16/32 镜像生成（引导扇区/FSInfo/FAT 副本/LFN + 8.3 目录项/簇链），
  **不调 `mformat`** —— H-03 要的就是「我们能生成 FAT 镜像」这件事；
- 写完做一次**回读校验**（几何 → FAT 副本 → 簇链 → 目录项 → 长度），失败就非零退出，
  免得把坏镜像交给 QEMU 再回头怀疑内核；
- 正确性由 **mtools 独立复核**：`mdir -i build/fat.img ::`、`mtype -i build/fat.img ::hello.txt`
  —— 两套**互相独立**的 FAT 实现都认同一张盘，才能说明「镜像本身是对的」（见 `tests/fatroot/README.md`）；
- 目录树取自 `tests/fatroot/data/`（`make fat-image`），每个文件对应一类断言：跨簇、空文件、LFN、二级子目录。

## 4. 非目标 / 后续

- 页缓存/写回/一致性策略、`fsync`：非第一版目标；
- 完整权限语义（UID/GID/ACL）与**卷标写/改名、`/storage/**` 状态树**：分别归 v1.0 的 `security/` 与 CSS-3 剩余；
  CSS-2 已给出 `MountOptions { read_only }`（只读挂载的机制），CSS-3 已接上 CKM 判定点 8（“谁能挂/谁能卸”）
  与盘符表 `storage/letters.rs`（`C:\` → POSIX 重写由 `libc/src/path.rs` 做，mount 由 `fs::mount_volume_by_id`/`unmount_point` 落）；
- 超大文件、高并发 IO、高性能目录索引：非第一版目标。

## 5. 修订记录

| 日期 | 变更 |
| ---- | ---- |
| 2026-10-14 | **v1.0 第三步（Socket/TCP 补齐）影响**：RAMFS `/bin` 加 `tcpecho` → `fs: ramfs files` 12 → **13**（2.5 的树与启动日志同步；`make test-programs` 断言 `ls: entry name=tcpecho`）；`fs/` 与 syscall 文件层无改动（套接字 fd 走独立号段，不进 fd 表） |
| 2026-10-12 | **CSS-3 影响**：`fs/mod.rs` 新增 `mount_volume_by_id`/`unmount_point`（`storage_mount`(262)/`storage_unmount`(263) 的底层落点，复用 CSS-2 的 `MountTable::mount_volume`/`alias_volume`/`umount`）；`fs/vfs.rs` 与已挂载后端的读写路径**无改动**（`C:\x` 是 `userland/libc/src/path.rs` 在发 syscall 前重写为 POSIX，内核仍只认绝对 POSIX 路径，见 storage.md 4.2） |
| 2026-10-12 | **CKM-2 影响**：`fs::mount` 的 `resolve`/`route` 在查表之前接入覆盖层路径改写（`ckm::overlay`，空表为快路径），因此「路径 → 后端」的每一种入口都看到同一份视图；新增 `fs::overlay_selftest`（在 `fs/` 侧建立 `/ckm/overlay/**`、注册/撤销、断言 `open` 结果变化）→ 用户进程看到的 `/` 从 5 变 **6**（`ls: / entries=6`，而 `fs::selftest` 的 5 不变，它在 `/ckm` 建立之前）；启动盘 FAT 由 64 MiB 收为 **63 MiB**（尾部 1 MiB 是 CKM 状态区），`sectors=129024 clusters=64244`（`make test-fat` 与 `test-programs` 的 `bytes=66060288` 同步）；`fs/vfs.rs` 与 syscall 层无改动 |
| 2026-10-11 | **CSS-2 落地（VFS 收口）**：`FileSystem` 增 `caps`/`volume_info`（保守缺省；RAMFS 可写、FAT 只读+大小写不敏感+卷标）；`fs/fat.rs` 增 FAT 探测唯一入口 `probe`/`probe_sector`/`bp_identity` 并把 `FatFs::open` 改返回 `OpenError { errno, reason }`；`fs/mount.rs` 拆「后端 / 挂载点」两张表（一个后端可被多挂载点引用），条目带 `vol_id`/`options`，新增 `alias_volume`/`umount`/`route`/`sync_counts`（挂载计数写回卷记录）；`fs::mount_disks` 按卷表挂载并保留 `/disk`、`/mnt/fat` 兼容点；`fs: stat` 行尾带 `vol=`；新增 `fs::storage_mount_selftest`（只读原因来自能力位、后端与记录一致、同一卷多挂载点同字节、卸载后 `ENOENT` 且卷仍在表、`ENOENT`/`EINVAL`/`EEXIST` 反例）；本文第 2.1/2.2/3.2/4 节同步标注现状 |
| 2026-10-06 | 第一阶段创建占位 |
| 2026-10-07 | 补记路线细化：明确 VFS 先行/RAMFS 后端/文件描述符表与 syscall 对接顺序、补充 ELF Loader 对接关系、补充 FAT32/mkfs 后续位置 |
| 2026-10-08 | 本次 docs 迭代统一文档顶部元信息（阶段 / 最后更新 / 状态）与修订记录，保持与 DEVELOPMENT.md 与索引文档描述一致 |
| 2026-10-11 | 备注：v0.8 的块设备层已落地（`block.rs` + `virtio_blk.rs`，`make test-driver`）；当时 FAT 后端与 `mkfs` 仍未开始 |
| 2026-10-11 | **v0.9 第六步影响**：RAMFS 新增 `/lib`（`ld.so` + `libgreet.so`，作为**被装载的对象**，见 syscall.md 3.9），`/bin` 再加 `dynhello` → `fs: ramfs files` 8 → **11**，根目录条目数 4 → **5**（2.5 的树/启动日志与 2.7 的 `entries=5` 同步；`make test-fs` 20 → **21 项**）；`fs::init` 新增 `libraries` 参数，`FileSystem` 与 syscall 层无改动（动态链接读库走的就是普通 `vfs::read_all`） |
| 2026-10-11 | **v0.9 第五步影响**：RAMFS 初始文件树里的 `/bin` 由 1 个程序（`sh`）扩到 6 个（`sh` + `hello` + `ls`/`cat`/`echo`/`ps`），`fs: ramfs files` 2 → **8**（2.5 的树与启动日志同步；根目录条目数仍为 4）；`ls` 从「`/bin/ls` 不存在 → execve ENOENT」变成真实程序（`make test-programs` 断言 `getdents` 真的读了 VFS 目录项） |
| 2026-10-11 | **v0.9 第二步影响**：RAMFS 初始文件树新增 `/bin`（`/bin/sh` = 交互式 Shell），根目录条目数 3 → 4（`fs::selftest` 与 `make test-fs` 期望同步）；`stdin`（fd 0）改接键盘行缓冲，fd 表描述更新 |
| 2026-10-11 | **v0.8 磁盘后端落地（收官）**：新增 `fs/fat.rs`（FAT12/16/32 只读：BPB 解析与类型判定、固定根目录区/簇链目录、8.3 + NT 标志 + LFN、偏移读、`EROFS`/`EUCLEAN`）与 `fs::mount_disks`（`/disk` + `/mnt/fat`，验证最长前缀路由）；`FileSystem` trait 收 `&mut self` 且 `read_all` 返回 `Vec<u8>`（磁盘内容不在内存里）；修「单盘启动误判 `/mnt/fat` 已挂」（改用 `vfs::is_mount_point`）；新增 `vfs::is_mount_point`；`make test-fat`（21 项，双盘启动，编排第 10 个场景）；`tools/mkfs/mkfs.py` + `tests/fatroot/`；第 3 节由「后续计划」改写为实现记录 |
| 2026-10-11 | **v0.7 核心落地**：从占位改为实现记录 —— 模块划分与 `FileSystem` trait、挂载表最长前缀路由（含路由证据日志）、路径语义收窄项、RAMFS 数据模型与单文件上限、初始文件树与启动日志、fd 表与 syscall 对接（fd 0/1/2 与 ≥3 的语义、最小空闲槽位、偏移量推进）、`make test-fs` 19 项验收；明确 Shell `cd/ls/cat` 属 v0.9、磁盘后端与 `mkfs` 属 v0.8 |
