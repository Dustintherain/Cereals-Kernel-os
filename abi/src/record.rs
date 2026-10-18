//! 记录布局与号段的**编译期**断言（`docs/pure-kernel.md` 阶段三验收）。
//!
//! 为什么用编译期断言而不是测试：这个 crate 的引用者是内核与用户程序，它们
//! **不在宿主上跑**。测试只在 `cargo test -p abi` 里跑；把「记录尺寸 / 号段边界 /
//! 掩码自洽」写成 `const _: () = assert!(...)`，则任何一侧只要把数字改坏，
//! **构建就失败** —— 这正是「单一事实源」要比「两份注释互相提醒」强的地方。
//!
//! 断言覆盖：
//!
//! | # | 断言 | 为什么 |
//! | ---- | ---- | ---- |
//! | R1 | 四条记录尺寸都是 8 的倍数 | 记录按 LE 整数切分，未对齐就会读到未对齐字段 |
//! | R2 | 记录尺寸 + 字段偏移自洽（`NAME_OFFSET + NAME_MAX == SIZE`） | 解析器按这个关系算切片 |
//! | R3 | 最大记录放得进一页 | 「记录尺寸页内对齐」是 ABI 承诺（阶段六） |
//! | R4 | `LINUX_SEGMENT_END <= CERLESSE_SEGMENT_START` | 号段不重叠 |
//! | R5 | `SYS_MAX` 大于最大已命名号 | 否则最后一个号会被判「越界」 |
//! | R6 | 掩码自洽（`PROT_MASK` / `MAP_SUPPORTED` / `O_ALLOWED`） | 内核用掩码拒绝未知位 |
//! | R7 | 盘符/容量哨兵不与合法值冲突 | `LETTER_NONE` 必须不是 A–Z |

use crate::{ckm, dirent, flags, procinfo, sock, syscall, volume};

/// 所有记录里最大的一条（自测与文档用；页内对齐的下界检查见 R3）。
pub const MAX_RECORD_SIZE: usize = {
    let mut max = ckm::AUDIT_RECORD_SIZE;
    if ckm::COMPONENT_RECORD_SIZE > max {
        max = ckm::COMPONENT_RECORD_SIZE;
    }
    if dirent::DIRENT_SIZE > max {
        max = dirent::DIRENT_SIZE;
    }
    if procinfo::PROCINFO_SIZE > max {
        max = procinfo::PROCINFO_SIZE;
    }
    if volume::RECORD_SIZE > max {
        max = volume::RECORD_SIZE;
    }
    max
};

/// 全部记录尺寸（把「有哪些记录」集中一次，便于逐条断言）。
pub const RECORD_SIZES: [usize; 5] = [
    dirent::DIRENT_SIZE,
    procinfo::PROCINFO_SIZE,
    volume::RECORD_SIZE,
    ckm::AUDIT_RECORD_SIZE,
    ckm::COMPONENT_RECORD_SIZE,
];

// R1：8 字节对齐（记录里的 u64 字段必须落在 8 的倍数偏移上）。
const _: () = assert!(dirent::DIRENT_SIZE % 8 == 0);
const _: () = assert!(procinfo::PROCINFO_SIZE % 8 == 0);
const _: () = assert!(volume::RECORD_SIZE % 8 == 0);
const _: () = assert!(ckm::AUDIT_RECORD_SIZE % 8 == 0);
const _: () = assert!(ckm::COMPONENT_RECORD_SIZE % 8 == 0);

// R2：字段偏移与尺寸自洽。
const _: () = assert!(dirent::NAME_OFFSET + dirent::NAME_MAX == dirent::DIRENT_SIZE);
const _: () = assert!(procinfo::NAME_OFFSET + procinfo::NAME_MAX == procinfo::PROCINFO_SIZE);
const _: () = assert!(dirent::NAME_OFFSET % 8 == 0);
const _: () = assert!(procinfo::NAME_OFFSET % 8 == 0);
// 16 字节标识字段 = 15 字符 + NUL（`ID_MAX`/`COMPONENT_ID_MAX` 是字符上限）
const _: () = assert!(volume::ID_MAX + 1 == 16);
const _: () = assert!(ckm::COMPONENT_ID_MAX + 1 == 16);

// R3：记录尺寸页内对齐（最大的一条不得跨页）。
const _: () = assert!(MAX_RECORD_SIZE <= 4096);

// R4：号段不重叠。
const _: () = assert!(syscall::LINUX_SEGMENT_END <= syscall::CERLESSE_SEGMENT_START);
const _: () = assert!(syscall::LINUX_SEGMENT_END == 256);
const _: () = assert!(syscall::CERLESSE_SEGMENT_START == 256);

// R5：`SYS_MAX` 覆盖全部已命名号（含已命名未实现的号）。
const _: () = assert!(syscall::SYS_MAX > syscall::SYS_STORAGE_FORMAT);
const _: () = assert!(syscall::SYS_MAX > syscall::SYS_CKM_POLICY_SET);
const _: () = assert!(syscall::SYS_MAX == 267);
const _: () = assert!(syscall::NAMED.len() == 31);
// R8：`sockaddr_in` 定长 16 字节（family 2 + port 2 + addr 4 + zero 8）。
const _: () = assert!(sock::SOCKADDR_IN_LEN == 16);
const _: () = assert!(sock::ADDR_OFFSET + 4 + 8 == sock::SOCKADDR_IN_LEN);

// R6：掩码自洽（内核按掩码拒未知位，掩码少一位就会静默接受）。
const _: () = assert!(flags::PROT_MASK == flags::PROT_READ | flags::PROT_WRITE | flags::PROT_EXEC);
const _: () = assert!(flags::MAP_SUPPORTED == flags::MAP_PRIVATE | flags::MAP_ANONYMOUS);
const _: () = assert!(flags::O_ALLOWED == flags::O_ACCMODE | flags::O_CREAT);
const _: () = assert!(flags::O_ACCMODE == 0o3);
const _: () = assert!(flags::FD_FIRST_DYNAMIC == 3);

// R7：哨兵与合法取值不冲突（`LETTER_NONE` 不能是 A–Z；`FREE_UNKNOWN` 不能是 0）。
const _: () = assert!(volume::LETTER_NONE < b'A');
const _: () = assert!(volume::FREE_UNKNOWN != 0);
const _: () = assert!(volume::MOUNT_RDONLY != 0);
// 受保护前缀的下标必须与数组顺序一致（编译期能查的部分）。
const _: () = assert!(ckm::PREFIX_ETC == 0);
const _: () = assert!(ckm::PREFIX_INIT == 3);
const _: () = assert!(ckm::PROTECTED_PREFIXES.len() == 4);
