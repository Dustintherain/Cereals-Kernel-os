//! 系统调用号与号段规则（`docs/syscall.md` 3 节是文字规格，本模块是**唯一数字来源**）。
//!
//! 编号沿用 **Linux x86_64** 语义（便于以后直接跑现成的用户态程序）：
//! `read=0, write=1, open=2, close=3, mmap=9, mprotect=10, munmap=11, brk=12,
//! nanosleep=35, getpid=39, socket=41, connect=42, sendto=44, recvfrom=45, bind=49,
//! fork=57, execve=59, exit=60, wait4=61`，
//! 目录列举沿用 Linux 的 `getdents=78`（**记录格式不同**，见 [`crate::dirent`]）。
//!
//! **号段规则（阶段六冻结，现已生效）**：
//!
//! | 号段 | 语义 | 规矩 |
//! | ---- | ---- | ---- |
//! | `0..=255` | Linux x86_64 | 只按 Linux 语义**新增**，不复用、不改语义 |
//! | `>= 256` | Cerlesse 扩展 | 号一旦发布**不再回收**（`docs/ckm.md` 5.3 同一原则） |
//!
//! 扩展号段内的分配：`256` = `getprocs`，`257..=260` = CKM，
//! `261..=266` = 存储（CSS），`267` = [`SYS_MAX`]（不含）。
//!
//! 表按 `SYS_MAX` 排满：未单独实现的号落在公共 ENOSYS 桩上，因此
//! 「号合法但未实现」与「号越界」是两种可区分的失败 —— 前者在
//! [`UNIMPLEMENTED`] 里有名字（串口打出 `syscall: ENOSYS nr=… name=…`），
//! 后者连号都不认识。

/// `read(fd, buf, count)`。
pub const SYS_READ: u64 = 0;
/// `write(fd, buf, count)`。
pub const SYS_WRITE: u64 = 1;
/// `open(path_ptr, path_len, flags)`（路径是**指针 + 长度**，不是 NUL 结尾）。
pub const SYS_OPEN: u64 = 2;
/// `close(fd)`。
pub const SYS_CLOSE: u64 = 3;
/// `mmap(len, prot, flags)`（私有匿名，**没有 offset 参数**）。
pub const SYS_MMAP: u64 = 9;
/// `mprotect(addr, len, prot)`。
pub const SYS_MPROTECT: u64 = 10;
/// `munmap(addr, len)`。
pub const SYS_MUNMAP: u64 = 11;
/// `brk(addr)`（沿用 Linux 语义：失败返回原值，不返回错误）。
pub const SYS_BRK: u64 = 12;
/// `nanosleep(ticks)`（Linux 名 `nanosleep`，单位是内核 tick 而不是 `timespec`）。
pub const SYS_SLEEP: u64 = 35;
/// `getpid()`。
pub const SYS_GETPID: u64 = 39;
/// `fork()`。
pub const SYS_FORK: u64 = 57;
/// `execve(path_ptr, path_len, argv_ptr, argc, 0)`（`argv` 是「指针 + 长度」对数组）。
pub const SYS_EXEC: u64 = 59;
/// `exit(code)`。
pub const SYS_EXIT: u64 = 60;
/// `wait4(pid)`。
pub const SYS_WAIT: u64 = 61;
/// `getdents(path_ptr, path_len, buf_ptr, buf_len)`（Linux 同名号，记录格式见 [`crate::dirent`]）。
pub const SYS_GETDENTS: u64 = 78;
/// `socket(domain, ty, protocol)`（Linux x86_64 号；只认 `AF_INET` + `SOCK_STREAM` + `IPPROTO_TCP`）。
pub const SYS_SOCKET: u64 = 41;
/// `connect(fd, addr_ptr, addr_len)`（Linux 号；`addr` 是 16 字节 `sockaddr_in`，见 [`crate::sock`]）。
pub const SYS_CONNECT: u64 = 42;
/// `sendto(fd, buf_ptr, len, addr_ptr, addr_len)`（Linux 号；已连接套接字上传 `0, 0` 地址）。
pub const SYS_SENDTO: u64 = 44;
/// `recvfrom(fd, buf_ptr, len, 0, 0)`（Linux 号；地址回填未实现，`addr` 必须传 0）。
pub const SYS_RECVFROM: u64 = 45;
/// `bind(fd, addr_ptr, addr_len)`（Linux 号；只固定本地端口，地址须是本机）。
pub const SYS_BIND: u64 = 49;
/// `getprocs(buf_ptr, buf_len)`（**Cerlesse 扩展号**：Linux 没有「列举进程」的系统调用，
/// `ps` 走 `/proc`，而本内核的 VFS 还没有虚拟文件）。
pub const SYS_GETPROCS: u64 = 256;
/// `ckm_policy_query(op, arg, out_ptr, out_len, 0)`：返回某 op 在**当前凭证**下的判定。
/// 无能力要求（只能查自己，没有参数能指定别的主体）。
pub const SYS_CKM_POLICY_QUERY: u64 = 257;
/// `ckm_audit_read(out_ptr, out_len, since_seq, 0, 0)`：需要 [`crate::ckm::CAP_AUDIT_READ`]。
pub const SYS_CKM_AUDIT_READ: u64 = 258;
/// `ckm_component_state(buf_ptr, buf_len)`：只读导出组件状态记录。
pub const SYS_CKM_COMPONENT_STATE: u64 = 259;
/// `ckm_policy_set(...)`（阶段 4 才定义行为）：需要 [`crate::ckm::CAP_CKM_ADMIN`]。
pub const SYS_CKM_POLICY_SET: u64 = 260;
/// `storage_volumes(buf_ptr, buf_len, out_count_ptr, 0, 0)`：卷表快照。
pub const SYS_STORAGE_VOLUMES: u64 = 261;
/// `storage_mount(vol_id_ptr, vol_id_len, point_ptr, point_len, flags)`。
pub const SYS_STORAGE_MOUNT: u64 = 262;
/// `storage_unmount(point_ptr, point_len, 0, 0, 0)`。
pub const SYS_STORAGE_UNMOUNT: u64 = 263;
/// `storage_assign_letter(vol_id_ptr, vol_id_len, letter, 0, 0)`。
pub const SYS_STORAGE_ASSIGN_LETTER: u64 = 264;
/// `storage_rescan(disk_index, 0, 0, 0, 0)`：`u32::MAX` = 全部设备（[`crate::volume::RESCAN_ALL`]）。
pub const SYS_STORAGE_RESCAN: u64 = 265;
/// `storage_format(...)`（CSS-4 才实现）。
pub const SYS_STORAGE_FORMAT: u64 = 266;

/// 系统调用号上限（**不含**）：`parse()` 对 `nr >= SYS_MAX` 返回「号越界」。
///
/// 号段 `256..SYS_MAX` 是 Cerlesse 扩展段（见模块头）。改这个数字等于扩号段，
/// 必须同时扩 [`UNIMPLEMENTED`]/表长（内核的 `SYS_TABLE` 以它为长度）。
pub const SYS_MAX: u64 = 267;

/// Linux 号段的结束（不含）：`0..LINUX_SEGMENT_END` 是 Linux x86_64 语义。
pub const LINUX_SEGMENT_END: u64 = 256;
/// Cerlesse 扩展号段的起点（含）。
pub const CERLESSE_SEGMENT_START: u64 = 256;

/// 号是否落在 Linux 号段（`0..=255`）。
pub const fn is_linux_segment(nr: u64) -> bool {
    nr < LINUX_SEGMENT_END
}

/// 号是否落在 Cerlesse 扩展号段（`256..SYS_MAX`）。
pub const fn is_cerlesse_segment(nr: u64) -> bool {
    nr >= CERLESSE_SEGMENT_START && nr < SYS_MAX
}

/// 所有**有名字**的调用号（按号升序；[`crate::record`] 与自测用它断言唯一性）。
///
/// 这张表是「已发布的号」清单：加号时同时加到这里，测试会拦住重复与越界。
pub const NAMED: [u64; 31] = [
    SYS_READ,
    SYS_WRITE,
    SYS_OPEN,
    SYS_CLOSE,
    SYS_MMAP,
    SYS_MPROTECT,
    SYS_MUNMAP,
    SYS_BRK,
    SYS_SLEEP,
    SYS_GETPID,
    SYS_SOCKET,
    SYS_CONNECT,
    SYS_SENDTO,
    SYS_RECVFROM,
    SYS_BIND,
    SYS_FORK,
    SYS_EXEC,
    SYS_EXIT,
    SYS_WAIT,
    SYS_GETDENTS,
    SYS_GETPROCS,
    SYS_CKM_POLICY_QUERY,
    SYS_CKM_AUDIT_READ,
    SYS_CKM_COMPONENT_STATE,
    SYS_CKM_POLICY_SET,
    SYS_STORAGE_VOLUMES,
    SYS_STORAGE_MOUNT,
    SYS_STORAGE_UNMOUNT,
    SYS_STORAGE_ASSIGN_LETTER,
    SYS_STORAGE_RESCAN,
    SYS_STORAGE_FORMAT,
];

/// 已定义但尚未实现的系统调用号：`(号, 名字)`。
///
/// 显式列出而不是「不写」：查表时它们落在 ENOSYS 桩上，与「号越界」是两种不同的
/// 失败原因，调试时不会被混为一谈。内核 `syscall_dispatch` 命中时打印
/// `syscall: ENOSYS nr=<n> name=<名字>`，于是「谁在依赖未实现的调用」在串口上可断言。
pub const UNIMPLEMENTED: [(u64, &str); 2] = [
    (SYS_CKM_POLICY_SET, "ckm_policy_set"),
    (SYS_STORAGE_FORMAT, "storage_format"),
];

/// 查「已命名但未实现」的号对应的名字（[`UNIMPLEMENTED`] 的查询接口）。
pub fn unimplemented_name(nid: u64) -> Option<&'static str> {
    let mut i = 0;
    while i < UNIMPLEMENTED.len() {
        if UNIMPLEMENTED[i].0 == nid {
            return Some(UNIMPLEMENTED[i].1);
        }
        i += 1;
    }
    None
}
