//! LZO1X 前缀解压：解出流的前 `dst.len()` 字节即停。
//!
//! 状态机移植自 lzokay 2.0.1 的 `decompress`（LZO1X opcode 语义），
//! 唯一改动是输出上限：所有拷贝截断到 dst 末尾、写满立即返回，不再
//! 解码剩余数据。供 CPK 条目"只取头部/子网格区"的轻量解析使用，
//! 避免为几 KB 统计信息整条目解压大文件（如 NPC 下数百个 2MB 的 FAK）。

use crate::error::Error;
use crate::filter::FILTER_MINILZO;

/// 见 lzokay：零字节计数的长度上限
const MAX255_COUNT: usize = usize::MAX / 255 - 2;
/// 中程匹配（M3）操作码标记
const M3_MARKER: u8 = 0x20;
/// 远程匹配（M4）与流终止操作码标记
const M4_MARKER: u8 = 0x10;

fn fail<T>() -> Result<T, Error> {
    Err(Error::DecompressFailed(FILTER_MINILZO))
}

/// 把 `src` 解压为输出的前 `dst.len()` 字节，返回实际写入长度。
/// 写满即停：`dst` 尾部之后的数据不解码；输入消费到哪里无所谓。
pub fn decompress_prefix(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    if src.len() < 3 {
        return fail();
    }

    let mut inp = 0usize;
    let mut outp = 0usize;
    let mut state = 0usize;
    let mut nstate: usize;
    let mut lblen: usize;
    let mut lbcur: usize;

    let mut inst = input_byte(src, &mut inp)?;
    // LZO 首字节为字面量引导：>=22 直接拷 (t-17) 个字面量；18..=21 计入 state
    if inst >= 22 {
        let len = (inst as usize) - 17;
        copy_literals(src, &mut inp, dst, &mut outp, len)?;
        state = 4;
    } else if inst >= 18 {
        nstate = (inst as usize) - 17;
        state = nstate;
        copy_literals(src, &mut inp, dst, &mut outp, nstate)?;
    }

    loop {
        if outp == dst.len() {
            return Ok(outp); // 前缀已写满，剩余流不解码
        }
        if inp > 1 || state > 0 {
            inst = input_byte(src, &mut inp)?;
        }
        if inst & 0xC0 != 0 {
            // [M2] 2kB 内匹配：长度 3-8，后随 1 字节距离
            let next = input_byte(src, &mut inp)?;
            let distance = ((next as usize) << 3) + (((inst as usize) >> 2) & 0x7) + 1;
            lbcur = outp.checked_sub(distance).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
            lblen = ((inst as usize) >> 5) + 1;
            nstate = (inst as usize) & 0x3;
        } else if inst & M3_MARKER != 0 {
            // [M3] 16kB 内匹配：后随 LE16 = (距离<<2 | state)
            lblen = ((inst as usize) & 0x1F) + 2;
            if lblen == 2 {
                let offset = consume_zero_byte_length(src, &mut inp)?;
                let tail = input_byte(src, &mut inp)?;
                lblen += offset * 255 + 31 + tail as usize;
            }
            let raw = read_le16(src, &mut inp)?;
            let distance = ((raw as usize) >> 2) + 1;
            lbcur = outp.checked_sub(distance).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
            nstate = (raw as usize) & 0x3;
        } else if inst & M4_MARKER != 0 {
            // [M4] 16..48kB 匹配；距离 0 = 流终止
            lblen = ((inst as usize) & 0x7) + 2;
            if lblen == 2 {
                let offset = consume_zero_byte_length(src, &mut inp)?;
                let tail = input_byte(src, &mut inp)?;
                lblen += offset * 255 + 7 + tail as usize;
            }
            let raw = read_le16(src, &mut inp)?;
            let base_dist = ((inst as usize & 0x8) << 11) + ((raw as usize) >> 2);
            if base_dist == 0 {
                break; // 流结束
            }
            let distance = base_dist + 16384;
            lbcur = outp.checked_sub(distance).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
            nstate = (raw as usize) & 0x3;
        } else if state == 0 {
            // [Literal] 长字面量串：长度 3 + 扩展
            let mut len = inst as usize + 3;
            if len == 3 {
                let offset = consume_zero_byte_length(src, &mut inp)?;
                let tail = input_byte(src, &mut inp)?;
                len += offset * 255 + 15 + tail as usize;
            }
            copy_literals(src, &mut inp, dst, &mut outp, len)?;
            state = 4;
            continue;
        } else if state != 4 {
            // [M1 短] state=1..3：1kB 内拷 2 字节
            let tail = input_byte(src, &mut inp)?;
            let distance = ((inst as usize) >> 2) + ((tail as usize) << 2) + 1;
            lbcur = outp.checked_sub(distance).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
            lblen = 2;
            nstate = (inst as usize) & 0x3;
        } else {
            // [M1 长] state=4：2..3kB 内拷 3 字节
            let tail = input_byte(src, &mut inp)?;
            let distance = ((inst as usize) >> 2) + ((tail as usize) << 2) + 2049;
            lbcur = outp.checked_sub(distance).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
            lblen = 3;
            nstate = (inst as usize) & 0x3;
        }

        // 匹配拷贝（源与目标可重叠，逐字节）；写满即截断
        if lblen > 0 {
            let n = lblen.min(dst.len() - outp);
            for i in 0..n {
                dst[outp + i] = dst[lbcur + i];
            }
            outp += n;
        }

        copy_literals(src, &mut inp, dst, &mut outp, nstate)?;

        state = nstate;
    }

    // 到达终止符但前缀未写满：流确实结束了（正常情况 want <= origin_size，
    // 走不到这里；走到说明 origin_size 与流实际输出不符）。
    // 前缀模式不校验输入是否恰好消费完（尾随字节不影响已解出的前缀）。
    if lblen != 3 {
        return fail();
    }
    Ok(outp)
}

#[inline(always)]
fn input_byte(src: &[u8], idx: &mut usize) -> Result<u8, Error> {
    let n = *src.get(*idx).ok_or(Error::DecompressFailed(FILTER_MINILZO))?;
    *idx += 1;
    Ok(n)
}

/// 从 `src` 拷 `len` 个字面量到 dst；两端各自截断——src 不足为错（流损坏），
/// dst 不足只拷放得下的部分（前缀写满，剩余丢弃）。
#[inline(always)]
fn copy_literals(
    src: &[u8],
    src_start: &mut usize,
    dst: &mut [u8],
    dst_start: &mut usize,
    len: usize,
) -> Result<(), Error> {
    let n = len.min(dst.len() - *dst_start);
    if *src_start + n > src.len() {
        return fail();
    }
    dst[*dst_start..*dst_start + n].copy_from_slice(&src[*src_start..*src_start + n]);
    *src_start += n;
    *dst_start += n;
    Ok(())
}

#[inline(always)]
fn read_le16(bytes: &[u8], pos: &mut usize) -> Result<u16, Error> {
    let lo = input_byte(bytes, pos)?;
    let hi = input_byte(bytes, pos)?;
    Ok(u16::from_le_bytes([lo, hi]))
}

#[inline(always)]
fn consume_zero_byte_length(src: &[u8], inp: &mut usize) -> Result<usize, Error> {
    let start = *inp;
    while src.get(*inp).copied() == Some(0) {
        *inp += 1;
    }
    let offset = *inp - start;
    if offset > MAX255_COUNT {
        return fail();
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 任意流、任意截断点：前缀解码必须与全量解码的前缀逐字节一致
    fn check_prefixes(data: &[u8], name: &str) {
        let packed = crate::filter::compress_lzo(data).unwrap();
        let mut full = vec![0u8; data.len()];
        let n = lzokay::decompress::decompress(&packed, &mut full).unwrap();
        assert_eq!(n, data.len(), "{name}: 全量解压长度不符");
        full.truncate(n);

        let mut cuts = vec![
            0usize,
            1,
            2,
            3,
            4,
            0x3C,
            64,
            100,
            1000,
            data.len() / 2,
            data.len().saturating_sub(1),
            data.len(),
        ];
        cuts.retain(|&k| k <= data.len());
        cuts.sort_unstable();
        cuts.dedup();
        for k in cuts {
            let mut dst = vec![0u8; k];
            let written = decompress_prefix(&packed, &mut dst)
                .unwrap_or_else(|e| panic!("{name}: 前缀 {k} 解压失败: {e}"));
            assert_eq!(written, k, "{name}: 前缀 {k} 写入长度不符");
            assert_eq!(&dst[..k], &full[..k], "{name}: 前缀 {k} 内容与全量不符");
        }
    }

    #[test]
    fn prefix_matches_full_decode() {
        // 高度可压缩
        check_prefixes(&vec![0u8; 100_000], "zeros");
        check_prefixes(&vec![b'A'; 50_000], "runs");
        // 文本型
        let text: Vec<u8> = (0..30_000u32)
            .map(|i| b"Hello, LZO world! "[(i % 18) as usize])
            .collect();
        check_prefixes(&text, "text");
        // 伪随机（几乎不可压缩）
        let mut x = 0x1234_5678u32;
        let rand: Vec<u8> = (0..30_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        check_prefixes(&rand, "random");
        // 混合：长零段夹随机段（触发各种匹配指令）
        let mut mixed = Vec::new();
        for i in 0..20 {
            mixed.extend(std::iter::repeat((i % 7) as u8).take(2000));
            mixed.extend_from_slice(&rand[..500]);
        }
        check_prefixes(&mixed, "mixed");
        // 极小样本
        check_prefixes(b"a", "one");
        check_prefixes(b"ab", "two");
        check_prefixes(b"abcabcabcabc", "small");
    }

    #[test]
    fn empty_prefix_ok() {
        let packed = crate::filter::compress_lzo(b"some data here").unwrap();
        let mut dst = vec![0u8; 0];
        assert_eq!(decompress_prefix(&packed, &mut dst).unwrap(), 0);
    }

    /// 真实样本：res/system 下 FSKEL/FSKIN 的 LZO 流（若由本库压缩则先经
    /// compress_lzo；res 内样本是解包后的原始字节，用本库压缩验证编码面）
    #[test]
    fn prefix_matches_full_real_sample() {
        let p = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Monster/G080/G080.FAK"));
        if !p.is_file() {
            return;
        }
        let data = std::fs::read(p).unwrap();
        check_prefixes(&data, "G080.FAK");
    }
}
