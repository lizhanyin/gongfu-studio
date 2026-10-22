//! 过滤器系统（对应 C++ 源 `FilterMgr/FilterMgr.cpp`）。
//!
//! | 编号 | 算法      | 说明                                        |
//! |------|-----------|---------------------------------------------|
//! | 1    | Changeless| 直通存储（与 0x4 大文件标志搭配出现）        |
//! | 2    | LZO1X-1   | minilzo 1.04，默认压缩路径                   |
//! | 3    | ZLib      | 原版只有断言存根，未实现                     |
//! | 4    | LZSS      | Okumura 经典实现（核心库未默认使用）         |

use crate::error::Error;
use crate::{lzo, lzss};

/// 直通存储。
pub const FILTER_CHANGELESS: u32 = 1;
/// LZO1X-1（minilzo 兼容）。
pub const FILTER_MINILZO: u32 = 2;
/// Okumura LZSS。
pub const FILTER_LZSS: u32 = 4;

/// 按表项 flag 中的过滤器编号解压数据，返回原始内容。
pub fn decompress(filter: u32, input: &[u8], origin_size: u32) -> Result<Vec<u8>, Error> {
    match filter {
        FILTER_CHANGELESS => Ok(input.to_vec()),
        FILTER_MINILZO => {
            let mut dst = vec![0u8; origin_size as usize];
            let written = lzokay::decompress::decompress(input, &mut dst)
                .map_err(|_| Error::DecompressFailed(filter))?;
            dst.truncate(written);
            Ok(dst)
        }
        FILTER_LZSS => lzss::decompress(input, origin_size as usize),
        other => Err(Error::UnsupportedFilter(other as u16)),
    }
}

/// 按过滤器解出前 `max_len` 字节即停（`max_len` 自动钳到 origin_size）。
/// LZO/LZSS 均为流式编码，前缀解压不解码剩余数据，供大文件只读头部；
/// Changeless 直通截断。`input` 为整条压缩块。
pub fn decompress_prefix(
    filter: u32,
    input: &[u8],
    origin_size: u32,
    max_len: usize,
) -> Result<Vec<u8>, Error> {
    let want = max_len.min(origin_size as usize);
    match filter {
        FILTER_CHANGELESS => Ok(input.get(..want).unwrap_or(input).to_vec()),
        FILTER_MINILZO => {
            let mut dst = vec![0u8; want];
            let written = lzo::decompress_prefix(input, &mut dst)?;
            dst.truncate(written);
            Ok(dst)
        }
        FILTER_LZSS => lzss::decompress_prefix(input, origin_size as usize, want),
        other => Err(Error::UnsupportedFilter(other as u16)),
    }
}

/// LZO1X-1 压缩（输出与 minilzo 字节不同，但同为合法 LZO1X 流，游戏可解）。
pub fn compress_lzo(input: &[u8]) -> Result<Vec<u8>, Error> {
    lzokay::compress::compress(input).map_err(|_| Error::CompressFailed)
}

/// 附加信息（名字 + 2 字节结束符 + 56 字节）之外，数据记录的总长。
pub fn record_size(name_len: usize, packed_size: u32) -> u64 {
    name_len as u64 + crate::format::LEN_OF_STRING_END as u64
        + crate::format::EXTRA_INFO_SIZE as u64
        + packed_size as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzo_roundtrip() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i / 257) as u8).collect();
        let packed = compress_lzo(&data).unwrap();
        assert!(packed.len() < data.len());
        let unpacked = decompress(FILTER_MINILZO, &packed, data.len() as u32).unwrap();
        assert_eq!(unpacked, data);
    }

    #[test]
    fn changeless_passthrough() {
        let d = b"0123456789";
        assert_eq!(decompress(FILTER_CHANGELESS, d, 10).unwrap(), d);
        assert_eq!(decompress_prefix(FILTER_CHANGELESS, d, 10, 4).unwrap(), &d[..4]);
        // max_len 超过 origin_size 时钳到 origin_size
        assert_eq!(decompress_prefix(FILTER_CHANGELESS, d, 4, 100).unwrap(), &d[..4]);
    }

    /// 前缀解压与全量解压在各截断点逐字节一致（LZO 与 LZSS）
    #[test]
    fn prefix_matches_full_both_filters() {
        let data: Vec<u8> = (0..40_000u32)
            .map(|i| if i / 3000 % 2 == 0 { (i % 251) as u8 } else { (i / 257) as u8 })
            .collect();
        for filter in [FILTER_MINILZO, FILTER_LZSS] {
            let packed = match filter {
                FILTER_MINILZO => compress_lzo(&data).unwrap(),
                _ => lzss::compress(&data),
            };
            let full = decompress(filter, &packed, data.len() as u32).unwrap();
            assert_eq!(full, data, "全量解压不一致 filter={filter}");
            for k in [0usize, 1, 0x3C, 5000, data.len() - 1, data.len()] {
                let pre = decompress_prefix(filter, &packed, data.len() as u32, k).unwrap();
                assert_eq!(pre.len(), k, "前缀长度不符 filter={filter} k={k}");
                assert_eq!(pre, &full[..k], "前缀内容不符 filter={filter} k={k}");
            }
        }
    }
}
