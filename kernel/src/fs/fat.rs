//! FAT 磁盘后端（v0.8 实现，**只读**）
//!
//! 这是 [`crate::fs::vfs::FileSystem`] 的第二个实现，挂在块设备上（virtio-blk →
//! [`crate::driver::block::BlockDevice`]）。它让「磁盘上的文件」和 RAMFS 里的文件在
//! VFS 层看起来一样 —— syscall 层一行都不用改，`open`/`read`/`close` 直接可用。
//!
//! ## 为什么是 FAT（而不是 EXT2/自己的格式）
//!
//! 启动盘本身就是 FAT（UEFI 固件的 ESP 要求），所以「同一个 FAT 实现」既能读我们自己
//! 生成的测试盘，也能读固件写的真实盘（内核因此能打开自己的 `\KERNEL.ELF`）。
//! 换句话说：格式选择由**已有的事实**决定，而不是由喜好决定。
//!
//! ## 覆盖范围
//!
//! | 项 | 支持情况 |
//! | ---- | ---- |
//! | 类型 | **FAT12 / FAT16 / FAT32** 全部按 BPB 自动判定（不是靠扩展名或配置） |
//! | 目录 | 12/16 的固定根目录区、32 与子目录的簇链；跳过 `.`/`..`/卷标/删除项 |
//! | 名字 | 8.3 短名（含 NT 大小写标志）+ **LFN 长文件名**（带校验和验证） |
//! | 文件 | 任意偏移读（`read` 走簇链）、整文件读（`read_all`）；空文件（首簇 0）正确返回 0 |
//! | 写 | **不支持**：写接口一律 [`vfs::EROFS`]（FAT 的分配器 + 目录项改写是 v0.9 的事） |
//! | 缓存 | 没有：每次访问都读设备（块设备是轮询的，读一个扇区约微秒级）。加扇区缓存是 v0.9 的优化 |
//! | 一致性 | 簇链断/越界/出现空闲标记 → [`vfs::EUCLEAN`]，不静默返回半截数据 |
//!
//! 已知限制（写清楚，避免「看起来支持」）：
//!
//! - 只接受 **512 字节扇区**的卷（`BPB_BytsPerSec == 512`）：块设备本身是 512 字节粒度，
//!   更大的扇区要么切成多次读、要么浪费内存，而 QEMU/固件都用 512；
//! - 不做「分区表（MBR/GPT）内的 FAT」：v0.8 直接把整盘当一个卷（超级软盘布局），
//!   与启动盘一致；分区表解析要等 `mkfs`/分区工具一起做；
//! - 时间戳/属性/短名与 LFN 的一致性只读不写；
//! - 没有 `..` 路径语义（VFS 层统一拒绝 `.`/`..`，见 `vfs::normalize`）。

use alloc::string::String;
use alloc::vec::Vec;

use crate::driver::block::{BlockDevice, SECTOR_SIZE};
use crate::fs::inode::{DirEntry, Metadata};
use crate::fs::vfs::{self, FileSystem};

/// 目录项固定 32 字节。
const DIR_ENTRY_SIZE: usize = 32;

/// 目录项属性：只读 / 隐藏 / 系统 / 卷标 / 目录 / 归档。
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
/// 长文件名目录项的属性组合（只读|隐藏|系统|卷标）。
const ATTR_LONG_NAME: u8 = 0x0F;

/// 目录项首字节：`0xE5` = 已删除；`0x00` = 其后全空。
const ENTRY_DELETED: u8 = 0xE5;
const ENTRY_END: u8 = 0x00;

/// 单个目录最多收集的条目数（防御畸形镜像导致的无界分配）。
const MAX_DIR_ENTRIES: usize = 1024;
/// LFN 名字的 UTF-16 字节上限（255 个字符 = 510 字节）。
const LFN_CAPACITY: usize = 512;
/// LFN 目录项里 13 个 UTF-16 码元的字节位置。
const LFN_SLOTS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

/// FAT 类型（决定 FAT 表项宽度与簇链结束标记）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FatType {
    /// 12 位表项（1.5 字节/项）
    Fat12,
    /// 16 位表项
    Fat16,
    /// 32 位表项（实际只用低 28 位）
    Fat32,
}

impl FatType {
    /// 后端名字：挂在 VFS 上时就是 `fat12` / `fat16` / `fat32`（日志与路由证据用）。
    pub const fn name(self) -> &'static str {
        match self {
            FatType::Fat12 => "fat12",
            FatType::Fat16 => "fat16",
            FatType::Fat32 => "fat32",
        }
    }

    /// 簇链结束标记（>= 该值即结束）。
    const fn eoc_threshold(self) -> u32 {
        match self {
            FatType::Fat12 => 0x0FF8,
            FatType::Fat16 => 0xFFF8,
            FatType::Fat32 => 0x0FFF_FFF8,
        }
    }

    /// 是否为「链结束」标记。
    fn is_eoc(self, value: u32) -> bool {
        value >= self.eoc_threshold()
    }
}

/// 由引导扇区（BPB）解析出的卷几何。
#[derive(Clone, Copy)]
pub struct Geometry {
    /// FAT 类型
    pub fat_type: FatType,
    /// 每扇区字节数（恒为 [`SECTOR_SIZE`]）
    pub bytes_per_sector: u32,
    /// 每簇扇区数
    pub sectors_per_cluster: u32,
    /// 保留扇区数（第一个 FAT 的起始扇区）
    pub reserved_sectors: u32,
    /// 根目录项数（FAT32 为 0，其根目录是簇链）
    pub root_entries: u32,
    /// 卷总扇区数
    pub total_sectors: u32,
    /// 第一个数据扇区
    pub data_start: u32,
    /// 簇总数（数据区）
    pub cluster_count: u32,
    /// FAT32 根目录首簇（12/16 为 0）
    pub root_cluster: u32,
    /// 12/16 固定根目录区的首扇区（FAT32 为 0）
    pub root_dir_start: u32,
    /// 12/16 固定根目录区的扇区数（FAT32 为 0）
    pub root_dir_sectors: u32,
}

impl Geometry {
    /// 每簇字节数。
    fn cluster_bytes(&self) -> u64 {
        self.sectors_per_cluster as u64 * self.bytes_per_sector as u64
    }

    /// 解析 BPB。`device_bytes` 是块设备容量，用于拒绝「卷比设备还大」的镜像。
    fn parse(boot: &[u8], device_bytes: u64) -> Result<Self, &'static str> {
        let u16_at = |offset: usize| -> u32 {
            u16::from_le_bytes([boot[offset], boot[offset + 1]]) as u32
        };
        let u32_at = |offset: usize| -> u32 {
            u32::from_le_bytes([boot[offset], boot[offset + 1], boot[offset + 2], boot[offset + 3]])
        };

        let bytes_per_sector = u16_at(11);
        if bytes_per_sector != SECTOR_SIZE {
            return Err("fat: unsupported sector size (only 512-byte sectors)");
        }
        let sectors_per_cluster = boot[13] as u32;
        if sectors_per_cluster == 0 || !sectors_per_cluster.is_power_of_two() {
            return Err("fat: invalid sectors-per-cluster");
        }
        let reserved_sectors = u16_at(14);
        if reserved_sectors == 0 {
            return Err("fat: invalid reserved sector count");
        }
        let num_fats = boot[16] as u32;
        if num_fats == 0 {
            return Err("fat: invalid FAT count");
        }
        let root_entries = u16_at(17) as u32;

        let total16 = u16_at(19);
        let total32 = u32_at(32);
        let total_sectors = if total16 != 0 { total16 } else { total32 };
        if total_sectors == 0 {
            return Err("fat: zero total sector count");
        }
        if total_sectors as u64 * SECTOR_SIZE as u64 > device_bytes {
            return Err("fat: volume is larger than the device");
        }

        let fat16_size = u16_at(22);
        let fat32_size = u32_at(36);
        let fat_sectors = if fat16_size != 0 { fat16_size } else { fat32_size };
        if fat_sectors == 0 {
            return Err("fat: zero FAT size");
        }

        let root_dir_sectors = (root_entries * DIR_ENTRY_SIZE as u32).div_ceil(bytes_per_sector);
        let overhead = reserved_sectors + num_fats * fat_sectors + root_dir_sectors;
        let data_sectors = total_sectors
            .checked_sub(overhead)
            .ok_or("fat: geometry underflow (overhead exceeds volume size)")?;
        let cluster_count = data_sectors / sectors_per_cluster;
        if cluster_count == 0 {
            return Err("fat: no data clusters");
        }

        // 布局（有无固定根目录区）决定 32 位 FAT；其余由簇数决定（FAT 规范的做法）。
        let layout32 = root_entries == 0 && fat16_size == 0;
        let fat_type = if layout32 {
            FatType::Fat32
        } else if cluster_count < 4085 {
            FatType::Fat12
        } else if cluster_count < 65525 {
            FatType::Fat16
        } else {
            return Err("fat: FAT16 layout with FAT32-sized cluster count");
        };

        let root_cluster = if layout32 { u32_at(44) } else { 0 };
        if layout32 && root_cluster < 2 {
            return Err("fat: FAT32 root cluster is not set");
        }

        // `num_fats` / `fat_sectors` 只参与上面的布局计算（`data_start` / `root_dir_start`），
        // 解析完就不需要留着了 —— 只读后端不会再去改动 FAT 副本。
        Ok(Self {
            fat_type,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            root_entries,
            total_sectors,
            data_start: overhead,
            cluster_count,
            root_cluster,
            root_dir_start: reserved_sectors + num_fats * fat_sectors,
            root_dir_sectors,
        })
    }
}

// ---- CSS-2：探测入口（`docs/storage.md` 5.1 / 5.3）----
//
// 「这个偏移上有没有 FAT」**只能**由 FAT 后端自己回答（探测顺序由 `storage::volume` 固定，
// 不让后端互相争抢）。CSS-1 时这份 BPB 判断曾在 `storage/volume.rs` 里被抄写过一遍；
// CSS-2 把它收回到这里，于是「FAT 的规则」在内核里只有一处（`Geometry::parse` + 本文件）。

/// 探测结果（三种结局，调用方不需要猜）。
pub enum ProbeOutcome {
    /// 读引导扇区失败：设备消失 / I/O 错误（不是「不是 FAT」）。
    ReadFailed,
    /// 这个偏移上不是 FAT（没有 `0xAA55`，或 BPB 自相矛盾）。
    NotFat,
    /// 是 FAT。
    Fat(FatProbe),
}

/// 探测命中的 FAT 卷（几何 + BPB 里的自带标识）。
#[derive(Clone, Copy)]
pub struct FatProbe {
    /// 卷几何（含 FAT12/16/32 类型）。
    pub geometry: Geometry,
    /// 卷序列号（BPB `BS_VolID`，**未**加类型前缀）。
    pub serial: u32,
    /// 卷标原始 11 字节（未规范化；规范化由 `storage::volume` 做，记录里只放规范形式）。
    pub label: [u8; 11],
    /// 卷容量（取「BPB 说的扇区数 × 512」与「调用方给的容量」的较小值）。
    pub total_bytes: u64,
}

/// 探测 `start_lba` 起、`sectors` 个扇区的卷是不是 FAT（只读；不挂载、不修改任何字节）。
///
/// `sectors` 是**卷**的大小（无分区表的整盘，或分区表里的一个分区），因此
/// 「卷比它所在的分区还大」会被 `Geometry::parse` 拒绝 —— 这是分区探针能否工作的前提。
pub fn probe(dev: &mut dyn BlockDevice, start_lba: u64, sectors: u64) -> ProbeOutcome {
    let mut boot = [0u8; SECTOR_SIZE as usize];
    match dev.read(start_lba, &mut boot) {
        Ok(got) if got == boot.len() => {}
        _ => return ProbeOutcome::ReadFailed,
    }
    match probe_sector(&boot, sectors.saturating_mul(SECTOR_SIZE as u64)) {
        Some(hit) => ProbeOutcome::Fat(hit),
        None => ProbeOutcome::NotFat,
    }
}

/// 从一个已读出的引导扇区判断是不是 FAT（`probe` 的纯函数内核；没有设备依赖）。
///
/// 规则与 `Geometry::parse` 同源：扇区大小必须是 512、簇大小必须是 2 的幂、
/// 保留扇区/FAT 数非零、BPB 的卷不能大于设备容量 —— 任何一条不满足都答
/// [`ProbeOutcome::NotFat`]（由调用方继续试后面的格式）。
pub fn probe_sector(boot: &[u8], capacity_bytes: u64) -> Option<FatProbe> {
    if boot.len() < SECTOR_SIZE as usize {
        return None;
    }
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return None;
    }
    let geometry = Geometry::parse(boot, capacity_bytes).ok()?;
    let (serial, label) = bp_identity(boot, geometry.fat_type);
    let total_bytes = capacity_bytes.min(geometry.total_sectors as u64 * SECTOR_SIZE as u64);
    Some(FatProbe { geometry, serial, label, total_bytes })
}

/// BPB 里的身份字段：序列号与卷标的偏移**按 FAT 类型**取。
///
/// FAT12/16 在 `0x27` / `0x2B`，FAT32 在 `0x43` / `0x47`（都在扩展引导签名 `0x29` 之后）。
/// 不按类型区分就会把 FAT12/16 的序列号读成 0，于是所有 FAT12/16 卷的 `vol_id` 撞在一起。
fn bp_identity(boot: &[u8], fat_type: FatType) -> (u32, [u8; 11]) {
    let (serial_off, label_off) = if fat_type == FatType::Fat32 { (0x43, 0x47) } else { (0x27, 0x2B) };
    let serial = u32::from_le_bytes([
        boot[serial_off],
        boot[serial_off + 1],
        boot[serial_off + 2],
        boot[serial_off + 3],
    ]);
    let mut label = [0u8; 11];
    label.copy_from_slice(&boot[label_off..label_off + 11]);
    (serial, label)
}

/// 目录的形态：12/16 的根目录是**固定区域**，其余（含 FAT32 根）是**簇链**。
#[derive(Clone, Copy)]
enum DirRef {
    /// 固定根目录区（`first_sector` 起 `sectors` 个扇区）
    Fixed { first_sector: u64, sectors: u64 },
    /// 簇链起点
    Cluster(u32),
}

/// 一个原始目录项（名字已按 LFN/短名规则解出）。
#[derive(Clone)]
struct RawEntry {
    /// 显示名（LFN 优先，否则 8.3 短名 + NT 大小写标志）
    name: String,
    /// 首簇（空文件为 0）
    cluster: u32,
    /// 字节数（目录为 0）
    size: u32,
    /// 是否目录
    is_dir: bool,
}

/// LFN 目录项的拼装状态。
///
/// 规范：LFN 项排在短名项**之前**，且按序号**降序**出现（第一项带 `0x40` 标志），
/// 每项 13 个 UTF-16 码元，序号 1 对应名字的**开头** —— 因此可以直接按 `seq` 定位写入位置，
/// 不需要「先收集再反转」。
struct Lfn {
    /// UTF-16LE 字节（按名字顺序排列）
    bytes: [u8; LFN_CAPACITY],
    /// 已写入的有效字节数
    len: usize,
    /// 短名校验和（首项里带来的）
    checksum: u8,
    /// 期望的下一个序号
    expected: u8,
    /// 是否处在有效的 LFN 序列里
    valid: bool,
}

impl Lfn {
    const fn new() -> Self {
        Self {
            bytes: [0; LFN_CAPACITY],
            len: 0,
            checksum: 0,
            expected: 0,
            valid: false,
        }
    }

    fn reset(&mut self) {
        self.len = 0;
        self.checksum = 0;
        self.expected = 0;
        self.valid = false;
    }

    /// 处理一个 LFN 目录项（非法序号会中止本序列，而不是拼出乱码）。
    fn push(&mut self, entry: &[u8]) {
        let seq = entry[0] & 0x1F;
        if entry[0] & 0x40 != 0 {
            self.reset();
            self.checksum = entry[13];
            self.expected = seq;
            self.valid = seq != 0;
        }
        if !self.valid || seq == 0 || seq != self.expected {
            self.valid = false;
            return;
        }
        let offset = (seq as usize - 1) * 13 * 2;
        if offset + 26 > LFN_CAPACITY {
            self.valid = false;
            return;
        }
        for (i, slot) in LFN_SLOTS.iter().enumerate() {
            self.bytes[offset + i * 2] = entry[*slot];
            self.bytes[offset + i * 2 + 1] = entry[*slot + 1];
        }
        self.len = (offset + 26).max(self.len);
        self.expected = seq - 1;
    }

    /// 短名项到来：若序列完整且校验和匹配，返回长名字符串。
    fn take(&mut self, short: &[u8]) -> Option<String> {
        let complete = self.valid && self.len > 0 && self.expected == 0;
        let checksum_ok = complete && (self.expected == 0) && self.checksum == lfn_checksum(short);
        let name = if checksum_ok { decode_utf16(&self.bytes[..self.len]) } else { None };
        self.reset();
        name
    }
}

/// LFN 校验和（对 11 字节短名做循环右移累加，规范原文的算法）。
fn lfn_checksum(short: &[u8]) -> u8 {
    let mut sum = 0u8;
    for &byte in short.iter().take(11) {
        sum = (((sum & 1) << 7) + (sum >> 1)).wrapping_add(byte);
    }
    sum
}

/// UTF-16LE → `String`（遇到 0x0000 结束；非 BMP/非法码元退化成 `?`，v0.8 只处理可打印 ASCII）。
fn decode_utf16(bytes: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let unit = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        i += 2;
        if unit == 0 {
            break;
        }
        out.push(char::from_u32(unit as u32).unwrap_or('?'));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 8.3 短名 → 显示名（应用 NT 大小写标志；`0x05` 开头的首字节是 `0xE5` 的转义）。
fn short_name(entry: &[u8]) -> String {
    let flags = entry[12];
    let lower_base = flags & 0x08 != 0;
    let lower_ext = flags & 0x10 != 0;

    let mut base_len = 8;
    while base_len > 0 && (entry[base_len - 1] == b' ' || entry[base_len - 1] == 0) {
        base_len -= 1;
    }
    let mut ext_len = 3;
    while ext_len > 0 && (entry[8 + ext_len - 1] == b' ' || entry[8 + ext_len - 1] == 0) {
        ext_len -= 1;
    }

    let mut out = String::new();
    for i in 0..base_len {
        let mut byte = entry[i];
        if i == 0 && byte == 0x05 {
            byte = 0xE5; // 规范里的转义（真实首字节 0xE5 与「已删除」冲突）
        }
        let ch = (byte as char).to_ascii_uppercase();
        out.push(if lower_base { ch.to_ascii_lowercase() } else { ch });
    }
    if ext_len > 0 {
        out.push('.');
        for i in 0..ext_len {
            let ch = (entry[8 + i] as char).to_ascii_uppercase();
            out.push(if lower_ext { ch.to_ascii_lowercase() } else { ch });
        }
    }
    out
}

/// 扫描一个扇区里的目录项；遇到「结束」项返回 `false`。
///
/// 是自由函数（不碰 [`FatFs`]）的原因：它只处理字节，调用方负责把扇区取下来 ——
/// 这样借用关系简单，也便于单测形态的复现（`mkfs.py` 与 mtools 是它的外部对照）。
fn scan_dir_sector(sector: &[u8], out: &mut Vec<RawEntry>, lfn: &mut Lfn) -> bool {
    let mut offset = 0;
    while offset + DIR_ENTRY_SIZE <= sector.len() {
        let entry = &sector[offset..offset + DIR_ENTRY_SIZE];
        offset += DIR_ENTRY_SIZE;

        if entry[0] == ENTRY_END {
            return false; // 其后全空
        }
        if entry[0] == ENTRY_DELETED {
            lfn.reset();
            continue;
        }
        let attr = entry[11];
        if attr & ATTR_LONG_NAME == ATTR_LONG_NAME {
            lfn.push(entry);
            continue;
        }
        if attr & ATTR_VOLUME_ID != 0 {
            lfn.reset(); // 卷标（或卷标+目录）条目
            continue;
        }
        if entry[0] == b'.' {
            lfn.reset(); // `.` / `..`
            continue;
        }
        // 到这里是一个普通文件/目录的短名项
        let name = lfn.take(entry).unwrap_or_else(|| short_name(entry));
        out.push(RawEntry {
            name,
            cluster: (u32::from_le_bytes([entry[20], entry[21], 0, 0]) << 16)
                | u16::from_le_bytes([entry[26], entry[27]]) as u32,
            size: u32::from_le_bytes([entry[28], entry[29], entry[30], entry[31]]),
            is_dir: attr & ATTR_DIRECTORY != 0,
        });
        if out.len() > MAX_DIR_ENTRIES {
            return false;
        }
    }
    true
}

/// FAT 文件系统实例。
/// 打开失败的原因：**errno**（给调用方做错误语义，CSS-2 起挂载失败不再只是一句话）
/// + 一句话（给人看的证据行）。
pub struct OpenError {
    /// `-EIO`（读失败）/ `-ENOTSUP`（不是 FAT）/ `-EUCLEAN`（是 FAT 但 BPB 自相矛盾）。
    pub errno: i64,
    /// 人读的原因（`fs: mount point=… failed errno=… : <reason>`）。
    pub reason: &'static str,
}

impl OpenError {
    /// 设备读失败。
    fn io(reason: &'static str) -> Self {
        Self { errno: vfs::EIO, reason }
    }

    /// 不是这个格式（例如整盘上带着 MBR 分区表）。
    fn not_fat(reason: &'static str) -> Self {
        Self { errno: vfs::ENOTSUP, reason }
    }

    /// 是 FAT，但结构自相矛盾（BPB 校验不过）。
    fn corrupt(reason: &'static str) -> Self {
        Self { errno: vfs::EUCLEAN, reason }
    }
}

/// FAT 文件系统实例。
pub struct FatFs {
    /// 底层块设备（独占借用；挂载表持有本后端）
    dev: &'static mut dyn BlockDevice,
    /// 后端名字（解析出 FAT 类型后填成 `fat12`/`fat16`/`fat32`）
    name: &'static str,
    /// 卷几何
    geometry: Geometry,
    /// 卷序列号（BPB `BS_VolID`；CSS-2 的 `volume_info` 用）
    serial: u32,
    /// 卷标原始 11 字节（未规范化；CSS-2 的 `volume_info` 用）
    volume_label: [u8; 11],
}

impl FatFs {
    /// 打开块设备上的 FAT 卷（读引导扇区 + 解析 BPB）。
    ///
    /// 失败原因（errno 见 [`OpenError`]）：
    /// - 设备读失败（[`vfs::EIO`]）；
    /// - 没有 `0xAA55` 引导签名 —— 说明这不是一个 FAT 卷（例如带着 MBR 分区表），
    ///   这与「盘不在」不同，因此是 [`vfs::ENOTSUP`]；
    /// - BPB 自相矛盾（扇区大小/簇大小/卷比设备大），见 [`Geometry::parse`] → [`vfs::EUCLEAN`]。
    pub fn open(dev: &'static mut dyn BlockDevice) -> Result<Self, OpenError> {
        let mut fs = Self {
            dev,
            name: "fat",
            serial: 0,
            volume_label: [0u8; 11],
            geometry: Geometry {
                fat_type: FatType::Fat12,
                bytes_per_sector: 0,
                sectors_per_cluster: 0,
                reserved_sectors: 0,
                root_entries: 0,
                total_sectors: 0,
                data_start: 0,
                cluster_count: 0,
                root_cluster: 0,
                root_dir_start: 0,
                root_dir_sectors: 0,
            },
        };

        let mut boot = [0u8; SECTOR_SIZE as usize];
        fs.read_bytes(0, &mut boot)
            .map_err(|_| OpenError::io("fat: cannot read the boot sector"))?;
        if boot[510] != 0x55 || boot[511] != 0xAA {
            return Err(OpenError::not_fat(
                "fat: boot signature 0xAA55 missing (not a FAT volume?)",
            ));
        }
        let capacity = fs.dev.capacity_bytes();
        fs.geometry = Geometry::parse(&boot, capacity).map_err(OpenError::corrupt)?;
        fs.name = fs.geometry.fat_type.name();
        let (serial, label) = bp_identity(&boot, fs.geometry.fat_type);
        fs.serial = serial;
        fs.volume_label = label;
        Ok(fs)
    }

    /// 卷几何（自测日志用）。
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    // ---- 底层读取 ----

    /// 从绝对字节偏移读 `out.len()` 字节（可跨扇区）。
    ///
    /// 每次访问都真读设备（无缓存，见模块头注释）；扇区缓冲放在**栈**上而不是结构体字段里，
    /// 这样不会出现「同时借用 `self.dev` 与 `self.sector`」的绕弯写法。
    fn read_bytes(&mut self, offset: u64, out: &mut [u8]) -> Result<(), i64> {
        let mut sector = [0u8; SECTOR_SIZE as usize];
        let mut done = 0usize;
        while done < out.len() {
            let pos = offset + done as u64;
            let lba = pos / SECTOR_SIZE as u64;
            let inner = (pos % SECTOR_SIZE as u64) as usize;
            self.dev.read(lba, &mut sector).map_err(|_| vfs::EIO)?;
            let take = (SECTOR_SIZE as usize - inner).min(out.len() - done);
            out[done..done + take].copy_from_slice(&sector[inner..inner + take]);
            done += take;
        }
        Ok(())
    }

    /// 簇号 → 首扇区（越界报 [`vfs::EUCLEAN`]）。
    fn cluster_lba(&self, cluster: u32) -> Result<u64, i64> {
        if cluster < 2 || cluster as u64 >= self.geometry.cluster_count as u64 + 2 {
            return Err(vfs::EUCLEAN);
        }
        Ok(self.geometry.data_start as u64
            + (cluster as u64 - 2) * self.geometry.sectors_per_cluster as u64)
    }

    /// 读 FAT 里的第 `cluster` 项。
    fn fat_entry(&mut self, cluster: u32) -> Result<u32, i64> {
        let fat_base = self.geometry.reserved_sectors as u64 * SECTOR_SIZE as u64;
        match self.geometry.fat_type {
            FatType::Fat12 => {
                let offset = cluster as u64 * 3 / 2;
                let mut raw = [0u8; 2];
                self.read_bytes(fat_base + offset, &mut raw)?;
                let value = u16::from_le_bytes(raw);
                Ok(if cluster % 2 == 0 {
                    (value & 0x0FFF) as u32
                } else {
                    (value >> 4) as u32
                })
            }
            FatType::Fat16 => {
                let mut raw = [0u8; 2];
                self.read_bytes(fat_base + cluster as u64 * 2, &mut raw)?;
                Ok(u16::from_le_bytes(raw) as u32)
            }
            FatType::Fat32 => {
                let mut raw = [0u8; 4];
                self.read_bytes(fat_base + cluster as u64 * 4, &mut raw)?;
                Ok(u32::from_le_bytes(raw) & 0x0FFF_FFFF)
            }
        }
    }

    /// 下一个簇：`None` = 链结束；`Err(EUCLEAN)` = 镜像损坏。
    fn next_cluster(&mut self, cluster: u32) -> Result<Option<u32>, i64> {
        let value = self.fat_entry(cluster)?;
        if self.geometry.fat_type.is_eoc(value) {
            return Ok(None);
        }
        if value < 2 || value as u64 >= self.geometry.cluster_count as u64 + 2 {
            // 0/1（空闲/保留）、越界、或坏簇标记 —— 都说明这张盘不可信
            return Err(vfs::EUCLEAN);
        }
        Ok(Some(value))
    }

    // ---- 目录 ----

    /// 根目录的形态。
    fn root_dir(&self) -> DirRef {
        if self.geometry.root_entries == 0 {
            DirRef::Cluster(self.geometry.root_cluster)
        } else {
            DirRef::Fixed {
                first_sector: self.geometry.root_dir_start as u64,
                sectors: self.geometry.root_dir_sectors as u64,
            }
        }
    }

    /// 收集目录里的全部条目（顺序 = 磁盘上的顺序）。
    fn collect_dir(&mut self, dir: DirRef) -> Result<Vec<RawEntry>, i64> {
        let mut out = Vec::new();
        let mut lfn = Lfn::new();
        let mut sector = [0u8; SECTOR_SIZE as usize];

        match dir {
            DirRef::Fixed { first_sector, sectors } => {
                for i in 0..sectors {
                    self.read_bytes((first_sector + i) * SECTOR_SIZE as u64, &mut sector)?;
                    if !scan_dir_sector(&sector, &mut out, &mut lfn) {
                        return Ok(out);
                    }
                }
            }
            DirRef::Cluster(first) => {
                let mut cluster = first;
                loop {
                    let lba = self.cluster_lba(cluster)?;
                    for i in 0..self.geometry.sectors_per_cluster as u64 {
                        self.read_bytes((lba + i) * SECTOR_SIZE as u64, &mut sector)?;
                        if !scan_dir_sector(&sector, &mut out, &mut lfn) {
                            return Ok(out);
                        }
                        if out.len() > MAX_DIR_ENTRIES {
                            return Err(vfs::EUCLEAN);
                        }
                    }
                    match self.next_cluster(cluster)? {
                        Some(next) => cluster = next,
                        None => break,
                    }
                }
            }
        }
        Ok(out)
    }

    /// 解析挂载点内的相对路径。
    fn resolve(&mut self, path: &str) -> Result<RawEntry, i64> {
        let comps = vfs::components(path);
        if comps.is_empty() {
            return Ok(RawEntry {
                name: String::from("/"),
                cluster: self.geometry.root_cluster,
                size: 0,
                is_dir: true,
            });
        }
        let mut dir = self.root_dir();
        for (i, comp) in comps.iter().enumerate() {
            let entries = self.collect_dir(dir)?;
            let found = entries
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(comp))
                .ok_or(vfs::ENOENT)?;
            if i + 1 == comps.len() {
                return Ok(found);
            }
            if !found.is_dir {
                return Err(vfs::ENOTDIR);
            }
            dir = DirRef::Cluster(found.cluster);
        }
        unreachable!("循环在最后一个分量上必然返回")
    }

    /// 从文件的第 `offset` 字节开始读最多 `out.len()` 字节（沿簇链走）。
    fn read_file(&mut self, entry: &RawEntry, offset: u64, out: &mut [u8]) -> Result<usize, i64> {
        let size = entry.size as u64;
        if offset >= size {
            return Ok(0); // EOF（含空文件：首簇 0 时也不会去解引用簇 0）
        }
        if entry.cluster < 2 {
            return Err(vfs::EUCLEAN); // 有内容却没有首簇
        }
        let want = out.len().min((size - offset) as usize);
        let cluster_bytes = self.geometry.cluster_bytes();

        // 跳过 offset 之前的簇：没有 inode 缓存，每次读都从链头走（v0.9 再补缓存）
        let mut cluster = entry.cluster;
        let mut skip = offset / cluster_bytes;
        while skip > 0 {
            cluster = self.next_cluster(cluster)?.ok_or(vfs::EUCLEAN)?;
            skip -= 1;
        }

        let mut done = 0usize;
        let mut inner = (offset % cluster_bytes) as usize;
        while done < want {
            let lba = self.cluster_lba(cluster)?;
            while done < want && inner < cluster_bytes as usize {
                let take = (cluster_bytes as usize - inner).min(want - done);
                self.read_bytes(lba * SECTOR_SIZE as u64 + inner as u64, &mut out[done..done + take])?;
                done += take;
                inner += take;
            }
            if done < want {
                cluster = self.next_cluster(cluster)?.ok_or(vfs::EUCLEAN)?;
                inner = 0;
            }
        }
        Ok(done)
    }
}

impl FileSystem for FatFs {
    fn name(&self) -> &'static str {
        self.name
    }

    /// CSS-2：FAT 后端的能力位（只读、大小写不敏感、8.3 与 LFN）。
    ///
    /// `read_only = true` 就是「只读的原因」的数据来源：挂载行里的 `ro=ro-backend`
    /// 不是写死的字面量，而是这里问出来的（写支持见 `docs/storage.md` CSS-4）。
    fn caps(&self) -> vfs::FsCaps {
        vfs::FsCaps {
            read_only: true,
            case_insensitive: true,
            max_name_len: 255,
            has_label: true,
        }
    }

    /// CSS-2：卷自带标识（序列号 + BPB 卷标原始字节）。
    ///
    /// 挂载后会与 `storage_volumes` 的记录逐字段比对（`fs::storage_mount_selftest`）：
    /// 两边认的是同一张 BPB，这就是「记录布局这个 ABI 被消费者读对了」的另一半证据。
    fn volume_info(&self) -> vfs::VolumeInfo {
        vfs::VolumeInfo {
            serial: Some(self.serial),
            label: self.volume_label,
            label_len: vfs::VolumeInfo::trimmed_len(&self.volume_label),
            total_bytes: self.geometry.total_sectors as u64 * SECTOR_SIZE as u64,
        }
    }

    fn stat(&mut self, path: &str) -> Result<Metadata, i64> {
        let entry = self.resolve(path)?;
        Ok(if entry.is_dir {
            Metadata::dir()
        } else {
            Metadata::file(entry.size as u64)
        })
    }

    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i64> {
        let entry = self.resolve(path)?;
        if entry.is_dir {
            return Err(vfs::EISDIR);
        }
        self.read_file(&entry, offset, buf)
    }

    fn read_all(&mut self, path: &str) -> Result<Vec<u8>, i64> {
        let entry = self.resolve(path)?;
        if entry.is_dir {
            return Err(vfs::EISDIR);
        }
        // 目录项里的 size 是**磁盘上的字节**，不可信：伪造的 0xFFFF_FFFF 会让内核
        // 先做一次 4 GiB 分配（分配失败 = abort），还没读到任何数据就死了。
        // 两个边界：不超过卷容量；分配用 `try_reserve`，OOM 归一为 -ENOMEM。
        let volume_bytes = self.geometry.total_sectors as u64 * SECTOR_SIZE as u64;
        let size = (entry.size as u64).min(volume_bytes) as usize;
        let mut out = Vec::new();
        out.try_reserve_exact(size).map_err(|_| vfs::ENOMEM)?;
        out.resize(size, 0);
        let read = self.read_file(&entry, 0, &mut out)?;
        out.truncate(read);
        Ok(out)
    }

    // ---- 写操作：只读后端 ----

    fn write(&mut self, _path: &str, _offset: u64, _data: &[u8]) -> Result<usize, i64> {
        Err(vfs::EROFS)
    }

    fn create(&mut self, _path: &str) -> Result<(), i64> {
        Err(vfs::EROFS)
    }

    fn mkdir(&mut self, _path: &str) -> Result<(), i64> {
        Err(vfs::EROFS)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, i64> {
        let dir = if vfs::components(path).is_empty() {
            self.root_dir()
        } else {
            let entry = self.resolve(path)?;
            if !entry.is_dir {
                return Err(vfs::ENOTDIR);
            }
            DirRef::Cluster(entry.cluster)
        };
        Ok(self
            .collect_dir(dir)?
            .into_iter()
            .map(|entry| DirEntry {
                name: entry.name,
                meta: if entry.is_dir {
                    Metadata::dir()
                } else {
                    Metadata::file(entry.size as u64)
                },
            })
            .collect())
    }
}
