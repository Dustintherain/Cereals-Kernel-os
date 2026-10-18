//! # Cerlesse 系统调用 ABI（单一事实源）
//!
//! 这个 crate 是内核与用户态**共同引用**的 ABI 定义：调用号、`errno`、标志位、
//! 记录布局。它的存在消灭的是一类具体缺陷 —— 同一个事实在内核与 libc 里
//! 各写一份，靠注释「必须与对方一致」维持（`docs/pure-kernel.md` 的 C6）。
//!
//! ## 分层
//!
//! | 模块 | 内容 | 谁引用 |
//! | ---- | ---- | ---- |
//! | [`syscall`] | 调用号、`SYS_MAX`、号段规则、已命名未实现的号 | 内核 `syscall_table`、libc |
//! | [`errno`] | `-errno` 约定（`i64`） | 内核各 syscall 模块、libc |
//! | [`flags`] | `O_*` / `PROT_*` / `MAP_*` / fd 编号 | 内核 `fs::file`、`address_space`、`memory`、libc |
//! | [`limits`] | 参数上限（路径、IO、`argv`） | 内核 `file`/`dir`/`process`、libc |
//! | [`dirent`] | `getdents` 的 88 字节记录 | 内核 `syscall::dir`、libc |
//! | [`procinfo`] | `getprocs` 的 40 字节记录 | 内核 `syscall::process`、libc |
//! | [`volume`] | `storage_*` 的 64 字节卷记录与 `flags` | 内核 `storage::volume`、libc |
//! | [`ckm`] | CKM 记录布局、能力位、op 编号、受保护前缀 | 内核 `ckm::*`、libc |
//! | [`record`] | **编译期**断言：尺寸/对齐/号段不重叠 | 任何引用者（编译失败即证据） |
//! | [`sock`] | socket 域/类型/协议与 `sockaddr_in` 布局 | 内核 `network`/`syscall::net`、libc |
//!
//! ## 约定
//!
//! - **值只有一个定义点**：内核与 libc 一律 `pub use` 或 `as` 转换，不再重写数字；
//! - **类型按各侧需要保留**：同一事实在两侧的类型可能不同（例如 `PROT_*`
//!   内核侧是 `u64`、libc 侧是 `i32`），此时 abi 给寄存器语义的规范类型
//!   （`u64`/`i64`），另一侧做一次**显式**转换 —— 转换是编译期常量折叠，行为不变，
//!   而「值从哪来」仍然只有一个答案；
//! - **号段规则**：`0..=255` = Linux x86_64 语义（只增不改），`>= 256` = Cerlesse 扩展。
//!
//! ## 自测
//!
//! `cargo test -p abi`（`make abi-check`）在**宿主**上跑，断言号唯一、号段不重叠、
//! 记录尺寸，并打印实际值 —— 见本文件末尾的 `tests` 模块。
#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod ckm;
pub mod dirent;
pub mod errno;
pub mod flags;
pub mod limits;
pub mod procinfo;
pub mod record;
pub mod sock;
pub mod syscall;
pub mod volume;

#[cfg(test)]
mod tests {
    use crate::{ckm, dirent, errno, procinfo, record, sock, syscall, volume};

    /// 调用号唯一、落在 `SYS_MAX` 内、且排序后就是 [`syscall::NAMED`]。
    #[test]
    fn numbers_are_unique_and_in_range() {
        let mut sorted = syscall::NAMED;
        sorted.sort_unstable();
        for w in sorted.windows(2) {
            assert_ne!(w[0], w[1], "系统调用号重复: {}", w[0]);
        }
        for nr in sorted {
            assert!(nr < syscall::SYS_MAX, "号 {} 越界 (SYS_MAX={})", nr, syscall::SYS_MAX);
        }
        // 排序后应逐个相等（NAMED 本身按号升序维护）
        assert_eq!(sorted, syscall::NAMED, "NAMED 必须按号升序");
    }

    /// 号段规则：`0..=255` 与 `>= 256` 不重叠，且边界值符合文档。
    #[test]
    fn segments_do_not_overlap() {
        assert_eq!(syscall::LINUX_SEGMENT_END, 256);
        assert_eq!(syscall::CERLESSE_SEGMENT_START, 256);
        assert!(syscall::LINUX_SEGMENT_END <= syscall::CERLESSE_SEGMENT_START);
        assert!(syscall::is_linux_segment(0));
        assert!(syscall::is_linux_segment(255));
        assert!(!syscall::is_linux_segment(256));
        assert!(syscall::is_cerlesse_segment(256));
        assert!(syscall::is_cerlesse_segment(syscall::SYS_MAX - 1));
        assert!(!syscall::is_cerlesse_segment(syscall::SYS_MAX));
        // Linux 号段里的号在本 ABI 里只能是我们自己用的那几个（不能顺手占用无关号）
        assert_eq!(syscall::SYS_READ, 0);
        assert_eq!(syscall::SYS_SOCKET, 41);
        assert_eq!(syscall::SYS_CONNECT, 42);
        assert_eq!(syscall::SYS_SENDTO, 44);
        assert_eq!(syscall::SYS_RECVFROM, 45);
        assert_eq!(syscall::SYS_BIND, 49);
        assert_eq!(syscall::SYS_GETDENTS, 78);
        assert_eq!(syscall::SYS_GETPROCS, 256);
        assert_eq!(syscall::SYS_MAX, 267);
    }

    /// 已命名但未实现的号必须**在号段内**、且逐个可查名字（串口证据依赖它）。
    #[test]
    fn unimplemented_table_is_consistent() {
        for (nr, name) in syscall::UNIMPLEMENTED {
            assert!(nr < syscall::SYS_MAX);
            assert!(!name.is_empty());
            assert_eq!(syscall::unimplemented_name(nr), Some(name));
        }
        assert_eq!(syscall::unimplemented_name(syscall::SYS_READ), None);
        assert_eq!(syscall::unimplemented_name(syscall::SYS_MAX), None);
    }

    /// 记录布局：尺寸与字段偏移必须自洽（数组/缓冲区按这些数字切分）。
    #[test]
    fn record_layouts_are_self_consistent() {
        assert_eq!(dirent::DIRENT_SIZE, 88);
        assert_eq!(dirent::NAME_OFFSET + dirent::NAME_MAX, dirent::DIRENT_SIZE);
        assert_eq!(procinfo::PROCINFO_SIZE, 40);
        assert_eq!(
            procinfo::NAME_OFFSET + procinfo::NAME_MAX,
            procinfo::PROCINFO_SIZE
        );
        assert_eq!(volume::RECORD_SIZE, 64);
        assert_eq!(ckm::AUDIT_RECORD_SIZE, 48);
        assert_eq!(ckm::COMPONENT_RECORD_SIZE, 64);
        assert_eq!(ckm::COMPONENT_ID_MAX + 1, 16); // 16 字节字段：15 字符 + NUL
        assert_eq!(volume::ID_MAX + 1, 16);
        assert_eq!(volume::LABEL_MAX, 11); // FAT 风格
        // 记录尺寸都是 8 的倍数且放得进一页（阶段六的“页内对齐”要求）
        for size in [
            dirent::DIRENT_SIZE,
            procinfo::PROCINFO_SIZE,
            volume::RECORD_SIZE,
            ckm::AUDIT_RECORD_SIZE,
            ckm::COMPONENT_RECORD_SIZE,
        ] {
            assert_eq!(size % 8, 0, "记录尺寸 {} 未按 8 字节对齐", size);
            assert!(size <= 4096, "记录尺寸 {} 超过一页", size);
        }
        // record.rs 的编译期断言被写进二进制，运行时也复核一次（防止有人把它们删掉）
        assert_eq!(record::MAX_RECORD_SIZE, 88);
    }

    /// errno：`-errno` 约定，值唯一、且都是负数。
    #[test]
    fn errnos_are_negative_and_unique() {
        let all = [
            errno::EPERM,
            errno::ENOENT,
            errno::EIO,
            errno::EBADF,
            errno::ECHILD,
            errno::EAGAIN,
            errno::ENOMEM,
            errno::EACCES,
            errno::EFAULT,
            errno::EISDIR,
            errno::EINVAL,
            errno::ENOSPC,
            errno::ENOSYS,
            errno::EAFNOSUPPORT,
            errno::ENOTCONN,
            errno::ETIMEDOUT,
            errno::ECONNREFUSED,
        ];
        let mut sorted = all;
        sorted.sort_unstable();
        for w in sorted.windows(2) {
            assert_ne!(w[0], w[1], "errno 重复: {}", w[0]);
        }
        assert!(all.iter().all(|v| *v < 0), "errno 必须是负数");
        assert_eq!(errno::ENOSYS, -38);
        assert_eq!(errno::EPERM, -1);
        assert_eq!(errno::EAFNOSUPPORT, -97);
        assert_eq!(errno::ENOTCONN, -107);
        assert_eq!(errno::ETIMEDOUT, -110);
        assert_eq!(errno::ECONNREFUSED, -111);
    }

    /// CKM / 存储的字面量（能力位、op 编号、盘符哨兵）逐条固化。
    #[test]
    fn ckm_and_storage_literals() {
        assert_eq!(ckm::ABI_VERSION, 1);
        assert_eq!(ckm::MAX_COMPONENTS, 32);
        assert_eq!(ckm::CAP_CKM_ADMIN, 0x1);
        assert_eq!(ckm::CAP_CKM_SERVICE, 0x2);
        assert_eq!(ckm::CAP_FS_SYSTEM_WRITE, 0x4);
        assert_eq!(ckm::CAP_AUDIT_READ, 0x8);
        assert_eq!(ckm::CAP_STORAGE_ADMIN, 0x10);
        assert_eq!(ckm::CAP_STORAGE_FORMAT, 0x20);
        assert_eq!(ckm::CAP_ALL, 0x3f);
        let ops = [
            ckm::OP_FS_WRITE,
            ckm::OP_STATE_WRITE,
            ckm::OP_POLICY_SET,
            ckm::OP_AUDIT_READ,
            ckm::OP_POLICY_QUERY,
            ckm::OP_EXEC_COMPONENT,
            ckm::OP_OVERLAY_REGISTER,
            ckm::OP_OVERLAY_CONFLICT,
            ckm::OP_SAFE_MODE_ENTER,
            ckm::OP_TXN_COMMIT,
            ckm::OP_TXN_ROLLBACK,
            ckm::OP_STORAGE_MOUNT,
            ckm::OP_STORAGE_UNMOUNT,
            ckm::OP_STORAGE_LETTER,
            ckm::OP_STORAGE_FORMAT,
            ckm::OP_STORAGE_RESCAN,
        ];
        for (i, op) in ops.iter().enumerate() {
            assert_eq!(*op as usize, i + 1, "op 编号必须连续从 1 起");
        }
        assert_eq!(ckm::PREFIX_ETC, 0);
        assert_eq!(ckm::PREFIX_INIT, 3);
        assert_eq!(ckm::PROTECTED_PREFIXES, ["/etc", "/bin", "/lib", "/init"]);
        assert_eq!(volume::MOUNT_RDONLY, 1);
        assert_eq!(volume::LETTER_NONE, 0);
        assert_eq!(volume::RESCAN_ALL, u32::MAX);
        assert_eq!(volume::FREE_UNKNOWN, u64::MAX);
    }

    /// 标志位：`O_*` 的组合、`PROT_*` 的掩码、`MAP_*` 的最小可用组合。
    #[test]
    fn flag_sets() {
        use crate::flags as f;
        assert_eq!(f::O_ACCMODE, f::O_RDONLY | f::O_WRONLY | f::O_RDWR);
        assert_eq!(f::O_ALLOWED, f::O_ACCMODE | f::O_CREAT);
        assert_eq!(f::PROT_MASK, f::PROT_READ | f::PROT_WRITE | f::PROT_EXEC);
        assert_eq!(f::PROT_NONE, 0);
        assert_eq!(f::MAP_SUPPORTED, f::MAP_PRIVATE | f::MAP_ANONYMOUS);
        assert_eq!(f::FD_FIRST_DYNAMIC, 3);
        assert_eq!(f::STDIN, 0);
        assert_eq!(f::STDOUT, 1);
        assert_eq!(f::STDERR, 2);
        // W^X 的判定用的是两个**独立**的位：`prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0`
        assert_eq!(f::PROT_WRITE & f::PROT_EXEC, 0, "写位与执行位必须独立");
        assert_ne!(f::PROT_WRITE, f::PROT_EXEC);
        // 受保护前缀的路径长度上限与限制模块一致
        assert_eq!(crate::limits::PATH_MAX, 256);
    }

    /// `sockaddr_in`：16 字节定长，字段偏移自洽（ family 2 + port 2 + addr 4 + zero 8）。
    #[test]
    fn sockaddr_in_layout() {
        assert_eq!(sock::AF_INET, 2);
        assert_eq!(sock::SOCK_STREAM, 1);
        assert_eq!(sock::IPPROTO_TCP, 6);
        assert_eq!(sock::SOCKADDR_IN_LEN, 16);
        assert_eq!(sock::FAMILY_OFFSET, 0);
        assert_eq!(sock::PORT_OFFSET, 2);
        assert_eq!(sock::ADDR_OFFSET, 4);
        assert_eq!(sock::ADDR_OFFSET + 4 + 8, sock::SOCKADDR_IN_LEN);
    }
}
