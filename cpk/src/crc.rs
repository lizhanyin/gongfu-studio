//! CPK 专用 CRC32（`CPK::Crc`，CPKCore.cpp:63）。
//!
//! 与标准 zlib CRC-32 不同：非反射（MSB-first）、多项式 0x04C11DB7、
//! 初值 = 前 ≤4 字节按大端装载后取反、最终再取反；空输入返回 0。
//! CRC 是文件身份标识（表键），按全路径小写 GBK 字节计算，不做数据校验。

const POLYNOMIAL: u32 = 0x04C1_1DB7;

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut j = 0;
        while j < 8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ POLYNOMIAL
            } else {
                c << 1
            };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static TABLE: [u32; 256] = build_table();

/// 计算 CPK CRC32。输入为包内全路径的小写字节串（不含结尾 NUL）。
pub fn crc32(data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0; // 根目录
    }
    // 前 4 字节（不足则有多少取多少）按大端拼入 result，随后取反作为初值
    let mut result: u32 = (data[0] as u32) << 24;
    let mut i = 1;
    if i < data.len() {
        result |= (data[1] as u32) << 16;
        i = 2;
        if i < data.len() {
            result |= (data[2] as u32) << 8;
            i = 3;
            if i < data.len() {
                result |= data[3] as u32;
                i = 4;
            }
        }
    }
    result = !result;

    while i < data.len() {
        let b = data[i] as u32;
        result = ((result << 8) | b) ^ TABLE[(result >> 24) as usize];
        i += 1;
    }
    !result
}

/// 仅做 ASCII 小写折叠。原工具 `_tcslwr` 作用于 ANSI(GBK) 字节串，
/// GBK 双字节区不发生变化，等价于仅折叠 ASCII 大写字母。
pub fn ascii_lower(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .map(|&b| if b.is_ascii_uppercase() { b + 32 } else { b })
        .collect()
}

/// 对包内全路径（任意大小写、`\` 分隔）计算 CRC。
pub fn crc_of_path(full_path_bytes: &[u8]) -> u32 {
    crc32(&ascii_lower(full_path_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn deterministic_and_sensitive() {
        let a = crc32(b"map");
        let b = crc32(b"MAP");
        let c = crc32(b"map\\abc.tga");
        assert_eq!(a, crc_of_path(b"MaP"));
        assert_ne!(a, b); // crc32 本身区分大小写，折叠由调用方负责
        assert_ne!(a, c);
    }

    #[test]
    fn short_inputs() {
        // 长度 1~4 的输入只走初值装载，不进入循环
        let _ = crc32(b"a");
        let _ = crc32(b"ab");
        let _ = crc32(b"abc");
        let _ = crc32(b"abcd");
        let _ = crc32(b"abcde");
    }
}
