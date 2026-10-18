//! VFS 虚拟文件系统抽象（v0.7 实现）
//!
//! 这一层定义「文件系统长什么样」，具体存储交给后端实现（当前只有
//! [`crate::fs::ramfs::RamFs`]）：
//!
//! - [`FileSystem`]：后端要实现的 trait（stat / read / write / create / mkdir / list）；
//! - 路径工具：[`normalize`] / [`components`] / [`within`] / [`split_parent`]；
//! - 系统级入口：[`stat`] / [`read_all`] / [`write`] / [`create`] / [`list`] ……
//!   它们先经 [`crate::fs::mount::table`] 做**挂载点路由**，再把挂载点内的相对
//!   路径交给对应后端。
//!
//! 错误约定：与系统调用一致，返回**负数 errno**（`-ENOENT` 等），
//! 这样 `fs` 的错误可以直接从 syscall handler 返回给用户态，中间不需要翻译。
//!
//! v0.7 的路径语义（刻意收窄，避免「看起来支持其实没实现」）：
//! - 只支持绝对路径与相对根路径（`/a/b`、`a/b` 等价），统一规范化为 `/a/b`；
//! - `.` 与 `..` 一律 [`EINVAL`]（没有 cwd、没有父目录链接）；
//! - 重复 `/` 与结尾 `/` 被折叠（`//a//b/` → `/a/b`）。

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::fs::inode::{DirEntry, Metadata};

// ---- errno（与 syscall 约定一致：负数） ----
//
// **全部再导出 `abi::errno`**（阶段三的单一事实源）：这里不再各写一份数字。
// `vfs::E*` 的名字保持不变，已有调用点不用改；数值只在内核与 libc 共享的 ABI 里定义一次。
pub use abi::errno::{
    EBADF, EEXIST, EIO, EISDIR, EINVAL, ENOENT, ENOEXEC, ENODEV, ENOMEM, ENOSPC, ENOTDIR,
    ENOTSUP, EROFS, EUCLEAN,
};

/// 后端能力位（CSS-2，见 `docs/storage.md` 5.1）。
///
/// 路径解析层与卷表**查询**它，而不是各自再实现一遍「大小写怎么办」「能不能写」：
/// 「FAT 读 NT 大小写标志」这类规则因此只写在后端里一次。
#[derive(Clone, Copy)]
pub struct FsCaps {
    /// 后端当前只实现了只读（写接口返回 [`EROFS`]）。
    pub read_only: bool,
    /// 路径匹配不区分大小写（NTFS/FAT 保留原名但匹配时不区分；ext2/4 区分）。
    pub case_insensitive: bool,
    /// 后端支持的最大文件名长度（字节）。
    pub max_name_len: usize,
    /// 卷是否自带卷标（FAT/exFAT/ext 在超级块里有；RAMFS 没有）。
    pub has_label: bool,
}

impl FsCaps {
    /// 保守缺省：只读、区分大小写、没有卷标 —— 后端不声明能力时按最弱的算。
    pub const fn read_only_unknown() -> Self {
        Self { read_only: true, case_insensitive: false, max_name_len: 255, has_label: false }
    }

    /// 可写后端的能力位（RAMFS；CSS-4 之后的 FAT/exFAT/ext2 亦同）。
    pub const fn writable() -> Self {
        Self { read_only: false, case_insensitive: false, max_name_len: 255, has_label: false }
    }

    /// 只读原因的短名（记录与证据行共用；可写时为 `rw`）。
    pub fn read_only_reason(self) -> &'static str {
        if self.read_only {
            "ro-backend"
        } else {
            "rw"
        }
    }
}

/// 卷自带信息（探测的产物；取不到的字段留空而不是编造）。
#[derive(Clone, Copy)]
pub struct VolumeInfo {
    /// 文件系统自带序列号/UUID 前缀（无则 `None`）。
    pub serial: Option<u32>,
    /// 卷标原始字节（**未规范化**；记录里放的是 `storage::volume` 的规范形式）。
    pub label: [u8; 11],
    /// 卷标有效长度（已去尾部空格/NUL）。
    pub label_len: usize,
    /// 卷容量（字节）。
    pub total_bytes: u64,
}

impl VolumeInfo {
    /// 一无所知（后端不提供时）。
    pub const fn unknown() -> Self {
        Self { serial: None, label: [0u8; 11], label_len: 0, total_bytes: 0 }
    }

    /// 去尾部空格/NUL 后的长度（`label_len` 的计算口径，与 `storage::volume` 一致）。
    pub fn trimmed_len(raw: &[u8; 11]) -> usize {
        let mut end = raw.len();
        while end > 0 && (raw[end - 1] == b' ' || raw[end - 1] == 0) {
            end -= 1;
        }
        end
    }

}

/// 挂载选项（CSS-2，见 `docs/storage.md` 5.2）。
#[derive(Clone, Copy)]
pub struct MountOptions {
    /// 以只读挂载（用户或 CKM 指定）→ 只读原因 `ro-mount`。
    pub read_only: bool,
}

impl MountOptions {
    /// 默认（可写）。
    pub const RW: Self = Self { read_only: false };
    /// 只读挂载。
    pub const RO: Self = Self { read_only: true };
}

/// 后端文件系统必须实现的操作。
///
/// 方法都收**挂载点内的相对路径**（`/a/b`；挂载点本身是 `/`），
/// 挂载点前缀的剥离由 [`crate::fs::mount::MountTable::resolve`] 完成。
///
/// v0.8 起所有路径操作收 `&mut self`：挂载表本来就用 `&'static mut` **独占**持有后端
/// （[`crate::fs::mount::MountTable::resolve`]），而磁盘后端需要修改自己的扇区缓冲、
/// 甚至直接访问块设备。用 `&self` + 内部可变性只是把这件事实藏起来。
pub trait FileSystem {
    /// 后端名字（出现在 `fs: mount point=... fs=<name>` 与路由日志里）。
    fn name(&self) -> &'static str;

    /// 取元数据。
    fn stat(&mut self, path: &str) -> Result<Metadata, i64>;

    /// 从 `offset` 起读最多 `buf.len()` 字节，返回实际读到字节数（EOF 为 0）。
    fn read(&mut self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i64>;

    /// 读整个文件（**所有权版本**）。
    ///
    /// v0.7 曾是「零拷贝借用后端内部缓冲」（`&'a [u8]`），v0.8 改成返回 `Vec<u8>`：
    /// 磁盘后端的文件内容根本不在内存里，要「借用」就得先把整个文件读进一个缓存 ——
    /// 那还不如直接把这次读的结果交出去。代价是比 RAMFS 多一次拷贝（小文件可忽略）。
    fn read_all(&mut self, path: &str) -> Result<Vec<u8>, i64>;

    /// 从 `offset` 起写 `data`；`offset == len` 表示追加。
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<usize, i64>;

    /// 创建空文件（父目录必须存在）。
    fn create(&mut self, path: &str) -> Result<(), i64>;

    /// 创建目录（父目录必须存在）。
    fn mkdir(&mut self, path: &str) -> Result<(), i64>;

    /// 列举目录（后端自己的顺序；v0.7 的 RAMFS 是插入顺序，FAT 是磁盘上的顺序）。
    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, i64>;

    /// 后端统计 `(文件数, 占用字节数)`。
    ///
    /// 只有内存后端有「占用字节」的概念（磁盘后端要等 v0.8 的块分配），
    /// 因此默认实现是 `(0, 0)`；RAMFS 覆盖它，让启动日志能报出内核有多少
    /// 文件与字节进了内存。
    fn usage(&self) -> (usize, usize) {
        (0, 0)
    }

    /// 后端能力位（CSS-2 新增；缺省按最弱的能力算，见 [`FsCaps::read_only_unknown`]）。
    ///
    /// 默认实现是**保守**的：没声明的后端一律当「只读、区分大小写、没有卷标」，
    /// 而不是当「什么都可以」—— 猜错的代价是数据被写坏。
    fn caps(&self) -> FsCaps {
        FsCaps::read_only_unknown()
    }

    /// 卷自带信息（CSS-2 新增；探测阶段得到，缺省为空）。
    fn volume_info(&self) -> VolumeInfo {
        VolumeInfo::unknown()
    }
}

// ---- 路径工具 ----

/// 规范化路径为 `/a/b` 形式。
///
/// 拒绝空路径、`.`、`..`（[`EINVAL`]）；折叠重复 `/`、结尾 `/`，并允许省略前导 `/`。
pub fn normalize(path: &str) -> Result<String, i64> {
    if path.is_empty() {
        return Err(EINVAL);
    }
    let mut out = String::from("/");
    let mut first = true;
    for comp in path.split('/') {
        if comp.is_empty() {
            continue; // 折叠重复斜杠与结尾斜杠
        }
        if comp == "." || comp == ".." {
            // v0.7 没有 cwd，也没有 `..` 链接：明确拒绝而不是悄悄猜一个含义
            return Err(EINVAL);
        }
        if !first {
            out.push('/');
        }
        out.push_str(comp);
        first = false;
    }
    Ok(out)
}

/// 规范化之后的路径分量（`/a/b` → `["a", "b"]`；`/` → `[]`）。
pub fn components(path: &str) -> Vec<&str> {
    path.split('/').filter(|c| !c.is_empty()).collect()
}

/// `path` 是否等于 `point` 或位于其下（挂载点路由用）。
pub fn within(path: &str, point: &str) -> bool {
    if point == "/" {
        return true;
    }
    path == point || path.starts_with(&format!("{point}/"))
}

// ---- 系统级入口（经挂载表路由） ----

/// 取元数据。
pub fn stat(path: &str) -> Result<Metadata, i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.stat(&rel)
}

/// 读整个文件（所有权版本：调用方拿到 `Vec`）。
pub fn read_all(path: &str) -> Result<Vec<u8>, i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.read_all(&rel)
}

/// 从 `offset` 起读入 `buf`。
pub fn read(path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.read(&rel, offset, buf)
}

/// 从 `offset` 起写 `data`。
pub fn write(path: &str, offset: u64, data: &[u8]) -> Result<usize, i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.write(&rel, offset, data)
}

/// 创建空文件。
pub fn create(path: &str) -> Result<(), i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.create(&rel)
}

/// 创建目录。
pub fn mkdir(path: &str) -> Result<(), i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.mkdir(&rel)
}

/// 列举目录。
pub fn list(path: &str) -> Result<Vec<DirEntry>, i64> {
    let (fs, rel) = crate::fs::mount::table().resolve(path)?;
    fs.list(&rel)
}

/// 后端统计 `(文件数, 占用字节数)`。
pub fn usage(path: &str) -> Result<(usize, usize), i64> {
    let (fs, _) = crate::fs::mount::table().resolve(path)?;
    Ok(fs.usage())
}

/// 某个路径是否是**挂载点本身**。
///
/// 与 [`backend_name`] 的区别很关键：`backend_name("/mnt/fat")` 在没有第二块盘时
/// 也会返回 `Some("ramfs-scratch")`（最长前缀落到挂在 `/mnt` 的后端），
/// 因此不能用它当作「FAT 卷已挂载」的证据。
pub fn is_mount_point(path: &str) -> bool {
    crate::fs::mount::table().is_mount_point(path)
}

/// 后端名字（日志用；路由到哪个文件系统是挂载表的可观测证据）。
pub fn backend_name(path: &str) -> Option<&'static str> {
    let (fs, _) = crate::fs::mount::table().resolve(path).ok()?;
    Some(fs.name())
}

// ---- CSS-2：卷身份与能力位的查询面 ----

/// 路径解析到的挂载点信息（挂载点 + 后端 + 卷身份 + 选项）。
///
/// 「路径 → 卷」在串口上可断言（`docs/storage.md` 5.2）：同一条路径解析到的
/// `vol_id` 必须与 `storage_volumes` 记录里的相等，这就是「两个入口同一份数据」的证据。
pub fn route(path: &str) -> Option<crate::fs::mount::Route> {
    crate::fs::mount::table().route(path)
}

/// 某条路径所在后端的能力位。
pub fn backend_caps(path: &str) -> Option<FsCaps> {
    let (fs, _) = crate::fs::mount::table().resolve(path).ok()?;
    Some(fs.caps())
}

/// 某条路径所在后端的卷自带信息。
pub fn backend_volume_info(path: &str) -> Option<VolumeInfo> {
    let (fs, _) = crate::fs::mount::table().resolve(path).ok()?;
    Some(fs.volume_info())
}
