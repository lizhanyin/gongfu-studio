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
}
