//! # cpk — 软星 CPK 资源包读写库
//!
//! 功夫世界等软星（Softstar）引擎游戏使用的 CPK 包格式（魔数 `RST\x1A`，
//! 2003 年 CTCRST 设计的 "game resource file system"）。
//! 本库是官方打包工具 Cpack2009（MFC，CPKCore 2.0）核心功能的 Rust 移植，
//! 与游戏端读取器（vEngine `vfs/CPKFast.cpp`，file system 1.0）保持兼容。
//!
//! 特性：
//! - 读：打开、目录树、解包（LZO1X-1 / LZSS / 直通存储）、MD5 与 CRC 校验
//! - 写：新建包、添加文件/目录（递归）、覆盖、删除（碎片化管理）、表扩容
//! - 中文文件名按 GBK 字节处理，保证 CRC 与游戏一致
//!
//! ```no_run
//! use cpk::Cpk;
//!
//! let mut cpk = Cpk::open("system.cpk")?;
//! for e in cpk.entries() {
//!     println!("{}", e.full_path);
//! }
//! # Ok::<(), cpk::Error>(())
//! ```
pub mod crc;
pub mod error;
pub mod filter;
pub mod format;
pub mod lzo;
pub mod lzss;
pub mod names;
pub mod package;

pub use error::{Error, Result};
pub use package::{crc_for_path, AddOpts, Cpk, CpkInfo, Entry, ExtractReport, VerifyReport};

/// CPK 文件魔数（小端 "RST\x1A"）
pub const MAGIC: [u8; 4] = [0x52, 0x53, 0x54, 0x1a];

/// 探测字节流是否为 CPK 包
pub fn is_cpk(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_matches_rst() {
        assert!(is_cpk(&MAGIC));
        assert!(!is_cpk(b"CPK "));
        assert!(!is_cpk(b""));
    }

    /// 端到端：新建 → 打包目录 → 重新打开 → 解包 → 逐字节比对。
    #[test]
    fn pack_unpack_roundtrip() {
        use std::fs;

        let dir = std::env::temp_dir().join("cpk_roundtrip_test");
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("src");
        fs::create_dir_all(src.join("子目录")).unwrap();

        // 可压缩数据 + 随机（不可压缩）数据 + 空文件 + 中文文件名
        let text = b"hello cpk, hello cpk, hello cpk!".repeat(100);
        fs::write(src.join("text.bin"), &text).unwrap();
        let mut x = 0x13579BDFu32;
        let rand: Vec<u8> = (0..50_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        fs::write(src.join("rand.bin"), &rand).unwrap();
        fs::write(src.join("empty.bin"), b"").unwrap();
        fs::write(src.join("子目录").join("中文 名字.txt"), b"GBK \xd6\xd0\xce\xc4").unwrap();

        let cpk_path = dir.join("test.cpk");
        let mut cpk = Cpk::create(&cpk_path, 64).unwrap();
        let n = cpk
            .add_path(&src, &AddOpts::default(), &mut |_, _, _| {})
            .unwrap();
        assert_eq!(n, 5); // text + rand + empty + 子目录 + 中文文件（根目录本身不进包）

        // 重新打开校验
        let mut cpk = Cpk::open(&cpk_path).unwrap();
        let report = cpk.verify(&mut |_, _, _| {}).unwrap();
        assert!(report.crc_bad.is_empty(), "CRC 不一致: {:?}", report.crc_bad);
        assert!(report.md5_bad.is_empty(), "MD5 不一致: {:?}", report.md5_bad);
        assert!(report.read_bad.is_empty(), "读取失败: {:?}", report.read_bad);

        // 解包并比对
        let out = dir.join("out");
        let r = cpk.extract(&out, &mut |_| true, &mut |_, _, _| {}).unwrap();
        assert_eq!(r.files, 4);
        assert_eq!(fs::read(out.join("text.bin")).unwrap(), text);
        assert_eq!(fs::read(out.join("rand.bin")).unwrap(), rand);
        assert_eq!(fs::read(out.join("empty.bin")).unwrap(), b"");
        assert_eq!(
            fs::read(out.join("子目录").join("中文 名字.txt")).unwrap(),
            b"GBK \xd6\xd0\xce\xc4"
        );

        // 删除 rand.bin 后条目数下降
        let removed = cpk
            .remove(&mut |e| e.full_path == "rand.bin", &mut |_, _, _| {})
            .unwrap();
        assert_eq!(removed, 1);
        let mut cpk = Cpk::open(&cpk_path).unwrap();
        assert!(cpk.entries().iter().all(|e| e.full_path != "rand.bin"));

        let _ = fs::remove_dir_all(&dir);
    }

    /// 前缀读取：read_file_prefix 各长度与 read_file 前缀一致；
    /// read_packed 缓存压缩块 + filter::decompress_prefix 多次解前缀同样一致。
    #[test]
    fn read_file_prefix_matches_full() {
        use std::fs;

        let dir = std::env::temp_dir().join("cpk_prefix_test");
        let _ = fs::remove_dir_all(&dir);
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();

        // 可压缩（LZO 有效）与不可压缩（大概率 Changeless/低压缩比）两种样本
        let text = b"prefix reading, prefix reading, prefix reading! ".repeat(500);
        let mut x = 0x2468ACE0u32;
        let rand: Vec<u8> = (0..20_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        fs::write(src.join("text.bin"), &text).unwrap();
        fs::write(src.join("rand.bin"), &rand).unwrap();

        let cpk_path = dir.join("test.cpk");
        let mut cpk = Cpk::create(&cpk_path, 64).unwrap();
        cpk.add_path(&src, &AddOpts::default(), &mut |_, _, _| {})
            .unwrap();
        let mut cpk = Cpk::open(&cpk_path).unwrap();

        for e in cpk.entries().iter().filter(|e| !e.is_dir) {
            let full = cpk.read_file(e).unwrap();
            assert_eq!(full.len(), e.origin_size as usize);
            for k in [0usize, 1, 0x3C, 100, full.len() / 2, full.len().saturating_sub(1), full.len()] {
                if k > full.len() {
                    continue;
                }
                let pre = cpk.read_file_prefix(e, k).unwrap();
                assert_eq!(pre.len(), k, "{} 前缀长度 {k}", e.full_path);
                assert_eq!(pre, &full[..k], "{} 前缀内容 {k}", e.full_path);
                // max_len 超过条目大小：钳到条目大小
                let clamped = cpk.read_file_prefix(e, full.len() + 12345).unwrap();
                assert_eq!(clamped, full, "{} 超长钳制", e.full_path);
            }
            // read_packed + decompress_prefix 组合（缓存压缩块多次解）
            let (packed, filter) = cpk.read_packed(e).unwrap();
            for k in [0x3Cusize, full.len()] {
                let want = k.min(full.len());
                let pre = filter::decompress_prefix(filter, &packed, e.origin_size, want).unwrap();
                assert_eq!(pre, &full[..want], "{} packed 组合前缀 {want}", e.full_path);
            }
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
