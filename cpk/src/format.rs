//! CPK 磁盘格式：常量、文件头、表项、附加信息结构。
//!
//! 对应 C++ 源 `Cpack2009/CPKCore/CPKDefine.h`（"game resource file system 2.0"，
//! 软星 CTCRST 2003）。全部结构为小端、无填充布局，字段宽度 4 字节对齐。

/// 文件类型标志，落盘小端字节序为 `52 53 54 1A`（"RST\x1A"）。
pub const CPK_LABEL: u32 = 0x1A54_5352;
pub const CPK_VERSION: u32 = 1;
/// 文件头大小：11 个 DWORD + 20 个保留 DWORD = 124 字节。
pub const HEADER_SIZE: usize = 124;
/// 单个表项大小。
pub const TABLE_ENTRY_SIZE: usize = 28;
/// 附加信息结构大小：5 个 FILETIME(40) + MD5(16)。
pub const EXTRA_INFO_SIZE: usize = 56;
/// 简单文件名之后的 0x00 结束符字节数（为与 Unicode 兼容保留 2 字节）。
pub const LEN_OF_STRING_END: usize = 2;
/// 新建包的初始表项数（`GT_CPK_DEFAULT_FILE`）。
pub const DEFAULT_TABLE_NUM: u32 = 5000;
/// 表扩容步进（`GT_CPK_TABLE_ADD`）。
pub const TABLE_ADD: u32 = 5000;
/// 超过此大小（50MB）的文件不压缩，直接存储。
pub const COMPRESS_SIZE: u64 = 50 * 1024 * 1024;

// ---- dwFlag 位定义 ----
/// 0x1：有效（删除后仍保留该位，但会被 0x10 排除出查找）
pub const FLAG_VALID: u32 = 0x0000_0001;
/// 0x2：目录
pub const FLAG_DIR: u32 = 0x0000_0002;
/// 0x4：大文件（未压缩存储）
pub const FLAG_LARGE: u32 = 0x0000_0004;
/// 0x10：已删除（表项变成空间碎片）
pub const FLAG_REMOVED: u32 = 0x0000_0010;
/// 0x8000：CRC 有重复
pub const FLAG_CRC_REPEAT: u32 = 0x0000_8000;

/// 表项是否代表有效文件或目录（而非碎片）：`IS_FILE_OR_DIR_VALID`。
pub const fn is_live(flag: u32) -> bool {
    (flag & FLAG_VALID != 0) && (flag & FLAG_REMOVED == 0)
}
/// 高 16 位为过滤器编号：`GET_FILTER`。
pub const fn filter_id(flag: u32) -> u32 {
    (flag >> 16) & 0xFFFF
}
/// 由低位标志与过滤器编号合成 dwFlag。
pub const fn make_flag(low: u32, filter: u32) -> u32 {
    (low & 0xFFFF) | (filter << 16)
}

/// CPK 文件头（124 字节）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub label: u32,
    pub version: u32,
    /// 文件表起始绝对偏移。
    pub table_start: u32,
    /// 数据区起始（建包时 = 头大小，之后不再更新）。
    pub data_start: u32,
    /// 当前有效文件 + 目录个数。
    pub file_num: u32,
    /// 是否整理过（原版从未使用）。
    pub is_formatted: u32,
    /// 头大小，恒 124。
    pub size_of_header: u32,
    /// 有效表项数（含碎片）。
    pub valid_table_num: u32,
    /// 表容量。
    pub max_table_num: u32,
    /// 碎片数 = valid_table_num - file_num。
    pub fragment_num: u32,
    /// 包当前总大小。
    pub package_size: u32,
    pub reserved: [u32; 20],
}

impl Header {
    pub fn from_bytes(b: &[u8]) -> Header {
        let u32at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let mut reserved = [0u32; 20];
        for (k, r) in reserved.iter_mut().enumerate() {
            *r = u32at(44 + k * 4);
        }
        Header {
            label: u32at(0),
            version: u32at(4),
            table_start: u32at(8),
            data_start: u32at(12),
            file_num: u32at(16),
            is_formatted: u32at(20),
            size_of_header: u32at(24),
            valid_table_num: u32at(28),
            max_table_num: u32at(32),
            fragment_num: u32at(36),
            package_size: u32at(40),
            reserved,
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut b = [0u8; HEADER_SIZE];
        let mut put = |i: usize, v: u32| b[i..i + 4].copy_from_slice(&v.to_le_bytes());
        put(0, self.label);
        put(4, self.version);
        put(8, self.table_start);
        put(12, self.data_start);
        put(16, self.file_num);
        put(20, self.is_formatted);
        put(24, self.size_of_header);
        put(28, self.valid_table_num);
        put(32, self.max_table_num);
        put(36, self.fragment_num);
        put(40, self.package_size);
        for (k, r) in self.reserved.iter().enumerate() {
            put(44 + k * 4, *r);
        }
        b
    }
}

/// CPK 文件表项（28 字节），整表按 `crc` 升序存放。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableEntry {
    pub crc: u32,
    pub flag: u32,
    pub father_crc: u32,
    pub start_pos: u32,
    pub packed_size: u32,
    pub origin_size: u32,
    pub extra_info_size: u32,
}

impl TableEntry {
    pub fn from_bytes(b: &[u8]) -> TableEntry {
        let u32at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        TableEntry {
            crc: u32at(0),
            flag: u32at(4),
            father_crc: u32at(8),
            start_pos: u32at(12),
            packed_size: u32at(16),
            origin_size: u32at(20),
            extra_info_size: u32at(24),
        }
    }

    pub fn to_bytes(&self) -> [u8; TABLE_ENTRY_SIZE] {
        let mut b = [0u8; TABLE_ENTRY_SIZE];
        let mut put = |i: usize, v: u32| b[i..i + 4].copy_from_slice(&v.to_le_bytes());
        put(0, self.crc);
        put(4, self.flag);
        put(8, self.father_crc);
        put(12, self.start_pos);
        put(16, self.packed_size);
        put(20, self.origin_size);
        put(24, self.extra_info_size);
        b
    }
}

/// 文件数据之后的附加信息（56 字节）。
///
/// 原版 C++ 工具写入的是未初始化栈数据（调试版表现为 0xCC 填充），
/// 时间戳字段游戏端并不读取；本实现写入真实时间。
/// 时间戳为 Windows FILETIME：自 1601-01-01 UTC 起、100ns 单位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileExtraInfo {
    pub create_time: u64,
    pub last_access_time: u64,
    pub last_write_time: u64,
    pub add_to_package_time: u64,
    pub del_from_package_time: u64,
    pub md5: [u8; 16],
}

/// Unix 时间戳（秒）→ FILETIME。
pub fn unix_to_filetime(secs: u64) -> u64 {
    const EPOCH_DELTA: u64 = 11_644_473_600; // 1601-01-01 → 1970-01-01
    (secs + EPOCH_DELTA) * 10_000_000
}

impl FileExtraInfo {
    pub fn to_bytes(&self) -> [u8; EXTRA_INFO_SIZE] {
        let mut b = [0u8; EXTRA_INFO_SIZE];
        let mut put = |i: usize, v: u64| b[i..i + 8].copy_from_slice(&v.to_le_bytes());
        put(0, self.create_time);
        put(8, self.last_access_time);
        put(16, self.last_write_time);
        put(24, self.add_to_package_time);
        put(32, self.del_from_package_time);
        b[40..56].copy_from_slice(&self.md5);
        b
    }

    pub fn from_bytes(b: &[u8]) -> FileExtraInfo {
        let u64at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let mut md5 = [0u8; 16];
        md5.copy_from_slice(&b[40..56]);
        FileExtraInfo {
            create_time: u64at(0),
            last_access_time: u64at(8),
            last_write_time: u64at(16),
            add_to_package_time: u64at(24),
            del_from_package_time: u64at(32),
            md5,
        }
    }
}
