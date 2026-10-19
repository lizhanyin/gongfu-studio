//! 经典 Okumura LZSS（内存版），对应 C++ 源根目录 `lzss.cpp`（过滤器编号 4）。
//!
//! 参数 N=4096、F=18、THRESHOLD=2，标志字节高位计数 8 组。
//! 原版压缩函数结尾刷写剩余编码时误把输出位置归零（`dwOutSize` 未随写推进），
//! 会覆盖已输出的数据；此处已修正，解压侧与原版逐字节兼容。

use crate::Error;

const N: usize = 4096; // 窗口大小（2 的幂）
const F: usize = 18; // 最长匹配
const THRESHOLD: usize = 2; // 匹配长度下限（含）
const NIL: usize = N;

struct Lzss {
    text_buf: Vec<u8>, // N + F - 1
    match_position: usize,
    match_length: usize,
    lson: Vec<i32>,
    rson: Vec<i32>,
    dad: Vec<i32>,
}

impl Lzss {
    fn new() -> Lzss {
        Lzss {
            text_buf: vec![0; N + F - 1],
            match_position: 0,
            match_length: 0,
            lson: vec![0; N + 1],
            rson: vec![0; N + 257],
            dad: vec![0; N + 1],
        }
    }

    fn init_tree(&mut self) {
        for i in N + 1..=N + 256 {
            self.rson[i] = NIL as i32;
        }
        for i in 0..N {
            self.dad[i] = NIL as i32;
        }
    }

    /// 插入 text_buf[r..r+F]，同时更新最长匹配（match_position/match_length）。
    fn insert_node(&mut self, r: usize) {
        let mut cmp: i32 = 1;
        let mut p = N + 1 + self.text_buf[r] as usize;
        self.rson[r] = NIL as i32;
        self.lson[r] = NIL as i32;
        self.match_length = 0;
        loop {
            if cmp >= 0 {
                if self.rson[p] as usize != NIL {
                    p = self.rson[p] as usize;
                } else {
                    self.rson[p] = r as i32;
                    self.dad[r] = p as i32;
                    return;
                }
            } else {
                if self.lson[p] as usize != NIL {
                    p = self.lson[p] as usize;
                } else {
                    self.lson[p] = r as i32;
                    self.dad[r] = p as i32;
                    return;
                }
            }
            let mut i = 1;
            while i < F {
                cmp = self.text_buf[r + i] as i32 - self.text_buf[p + i] as i32;
                if cmp != 0 {
                    break;
                }
                i += 1;
            }
            if i > self.match_length {
                self.match_position = p;
                self.match_length = i;
                if self.match_length >= F {
                    break;
                }
            }
        }
        // 用新节点 r 顶替旧节点 p
        let dp = self.dad[p] as usize;
        self.dad[r] = dp as i32;
        self.lson[r] = self.lson[p];
        self.rson[r] = self.rson[p];
        self.dad[self.lson[p] as usize] = r as i32;
        self.dad[self.rson[p] as usize] = r as i32;
        if self.rson[dp] as usize == p {
            self.rson[dp] = r as i32;
        } else {
            self.lson[dp] = r as i32;
        }
        self.dad[p] = NIL as i32;
    }

    fn delete_node(&mut self, p: usize) {
        if self.dad[p] as usize == NIL {
            return; // 不在树中
        }
        let q: usize;
        if self.rson[p] as usize == NIL {
            q = self.lson[p] as usize;
        } else if self.lson[p] as usize == NIL {
            q = self.rson[p] as usize;
        } else {
            // 两个孩子在：取左子树最右节点 q 顶替 p
            let mut q2 = self.lson[p] as usize;
            if self.rson[q2] as usize != NIL {
                while self.rson[q2] as usize != NIL {
                    q2 = self.rson[q2] as usize;
                }
                let dq2 = self.dad[q2] as usize;
                self.rson[dq2] = self.lson[q2];
                self.dad[self.lson[q2] as usize] = self.dad[q2];
                self.lson[q2] = self.lson[p];
                self.dad[self.lson[p] as usize] = q2 as i32;
            }
            self.rson[q2] = self.rson[p];
            self.dad[self.rson[p] as usize] = q2 as i32;
            let dp = self.dad[p] as usize;
            self.dad[q2] = dp as i32;
            if self.rson[dp] as usize == p {
                self.rson[dp] = q2 as i32;
            } else {
                self.lson[dp] = q2 as i32;
            }
            self.dad[p] = NIL as i32;
            return;
        }
        let dp = self.dad[p] as usize;
        self.dad[q] = dp as i32;
        if self.rson[dp] as usize == p {
            self.rson[dp] = q as i32;
        } else {
            self.lson[dp] = q as i32;
        }
        self.dad[p] = NIL as i32;
    }
}

/// LZSS 压缩，输出为合法 Okumura 字节流。
pub fn compress(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() / 8 + 16);
    if input.is_empty() {
        return out;
    }
    let mut s = Lzss::new();
    s.init_tree();

    let mut code_buf = [0u8; 17];
    let mut code_buf_ptr = 1;
    let mut mask: u8 = 1;
    let mut in_pos = 0usize;
    let mut sp = 0usize; // 滑动窗口读位
    let mut r = N - F;

    for i in 0..r {
        s.text_buf[i] = b' ';
    }
    let mut len = 0usize;
    while len < F && in_pos < input.len() {
        s.text_buf[r + len] = input[in_pos];
        in_pos += 1;
        len += 1;
    }
    for i in 1..=F {
        s.insert_node(r - i);
    }
    s.insert_node(r);

    loop {
        if s.match_length > len {
            s.match_length = len; // 文本末尾附近匹配长度可能虚高
        }
        if s.match_length <= THRESHOLD {
            s.match_length = 1;
            code_buf[0] |= mask;
            code_buf[code_buf_ptr] = s.text_buf[r];
            code_buf_ptr += 1;
        } else {
            code_buf[code_buf_ptr] = s.match_position as u8;
            code_buf[code_buf_ptr + 1] =
                (((s.match_position >> 4) & 0xF0) | (s.match_length - (THRESHOLD + 1))) as u8;
            code_buf_ptr += 2;
        }
        if (mask << 1) == 0 {
            out.extend_from_slice(&code_buf[..code_buf_ptr]);
            code_buf[0] = 0;
            code_buf_ptr = 1;
            mask = 1;
        } else {
            mask <<= 1;
        }

        let last_match_length = s.match_length;
        let mut i = 0usize;
        while i < last_match_length && in_pos < input.len() {
            let c = input[in_pos];
            in_pos += 1;
            s.delete_node(sp);
            s.text_buf[sp] = c;
            if sp < F - 1 {
                s.text_buf[sp + N] = c; // 窗口尾部镜像，便于比较
            }
            sp = (sp + 1) & (N - 1);
            r = (r + 1) & (N - 1);
            s.insert_node(r);
            i += 1;
        }
        while i < last_match_length {
            i += 1;
            s.delete_node(sp);
            sp = (sp + 1) & (N - 1);
            r = (r + 1) & (N - 1);
            if len > 0 {
                len -= 1;
                if len > 0 {
                    s.insert_node(r);
                }
            }
        }
        if len == 0 {
            break;
        }
    }
    if code_buf_ptr > 1 {
        // 原版在此误用未推进的输出位置（写回 0 处），已修正为顺序追加
        out.extend_from_slice(&code_buf[..code_buf_ptr]);
    }
    out
}

/// LZSS 解压。`expected` 为原始长度，用于预分配并防越界。
pub fn decompress(input: &[u8], expected: usize) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(expected);
    let mut text_buf = vec![0u8; N + F - 1];
    for i in 0..N - F {
        text_buf[i] = b' ';
    }
    let mut r = N - F;
    let mut flags: u32 = 0;
    let mut in_pos = 0usize;

    loop {
        flags >>= 1;
        if flags & 0x100 == 0 {
            if in_pos >= input.len() {
                break;
            }
            flags = (input[in_pos] as u32) | 0xFF00; // 高字节充当 8 次计数器
            in_pos += 1;
        }
        if flags & 1 != 0 {
            // 字面字节
            if in_pos >= input.len() {
                break;
            }
            let c = input[in_pos];
            in_pos += 1;
            if out.len() >= expected + N {
                return Err(Error::LzssOverflow);
            }
            out.push(c);
            text_buf[r] = c;
            r = (r + 1) & (N - 1);
        } else {
            // (位置, 长度) 对
            if in_pos >= input.len() {
                break;
            }
            let i0 = input[in_pos] as usize;
            in_pos += 1;
            if in_pos >= input.len() {
                break;
            }
            let j0 = input[in_pos] as usize;
            in_pos += 1;
            let pos = i0 | ((j0 & 0xF0) << 4);
            let count = (j0 & 0x0F) + THRESHOLD;
            for k in 0..=count {
                if out.len() >= expected + N {
                    return Err(Error::LzssOverflow);
                }
                let c = text_buf[(pos + k) & (N - 1)];
                out.push(c);
                text_buf[r] = c;
                r = (r + 1) & (N - 1);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let packed = compress(data);
        let unpacked = decompress(&packed, data.len()).unwrap();
        assert_eq!(unpacked, data, "roundtrip failed for {} bytes", data.len());
    }

    #[test]
    fn roundtrip_various() {
        roundtrip(b"");
        roundtrip(b"a");
        roundtrip(b"Hello, LZSS world! Hello, LZSS world! Hello, LZSS world!");
        roundtrip(&vec![0u8; 100_000]);
        roundtrip(&(0..50_000u32).map(|i| (i * 7 + (i >> 3)) as u8).collect::<Vec<_>>());
        // 随机数据（不可压缩）
        let mut x = 0x12345678u32;
        let rand: Vec<u8> = (0..30_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        roundtrip(&rand);
    }
}
