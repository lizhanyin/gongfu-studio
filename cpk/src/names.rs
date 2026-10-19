//! 包内名字编码：磁盘 OsStr ↔ 包内 GBK 字节。
//!
//! 原工具是 MBCS(ANSI/GBK) 程序，落盘的名字与 CRC 计算都以 GBK 字节为准。
//! 为保证与游戏包逐字节一致，本库在 Windows 上经 UTF-16 中转做 GBK 转换；
//! 非 Windows 平台按原始字节透传（假定调用方已准备 GBK 字节）。

use std::ffi::{OsStr, OsString};

#[cfg(windows)]
pub fn os_to_gbk(s: &OsStr) -> Vec<u8> {
    let utf8 = s.to_string_lossy();
    let (bytes, _, _) = encoding_rs::GBK.encode(&utf8);
    bytes.into_owned()
}

#[cfg(windows)]
pub fn gbk_to_os(bytes: &[u8]) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    let (utf8, _, _) = encoding_rs::GBK.decode(bytes);
    let wide: Vec<u16> = utf8.encode_utf16().collect();
    OsString::from_wide(&wide)
}

#[cfg(unix)]
pub fn os_to_gbk(s: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_bytes().to_vec()
}

#[cfg(unix)]
pub fn gbk_to_os(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

/// GBK 字节 → 有损 UTF-8 字符串（仅用于显示与匹配）。
pub fn gbk_lossy(bytes: &[u8]) -> String {
    let (s, _, _) = encoding_rs::GBK.decode(bytes);
    s.into_owned()
}

/// 有损 UTF-8 字符串 → GBK 字节（用于把用户提供的过滤模式转成可比较形式）。
pub fn utf8_to_gbk(s: &str) -> Vec<u8> {
    let (bytes, _, _) = encoding_rs::GBK.encode(s);
    bytes.into_owned()
}
