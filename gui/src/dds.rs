//! DDS 纹理解码（本游戏的 DDS 为 DXT1/DXT3 压缩，头部 pitch/mips 为 0 的变体）
//! 解码为自顶向下的 RGBA8。

pub struct TexImage {
    pub width: u32,
    pub height: u32,
    /// RGBA8，行序自顶向下
    pub rgba: Vec<u8>,
}

pub fn decode_dds(bytes: &[u8]) -> Result<TexImage, String> {
    if bytes.len() < 0x80 || &bytes[0..4] != b"DDS " {
        return Err("不是有效的 DDS 文件".into());
    }
    let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let height = rd(12);
    let width = rd(16);
    let fourcc = &bytes[0x54..0x58];
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(format!("异常尺寸 {width}x{height}"));
    }
    let data = &bytes[0x80..];
    let rgba = match fourcc {
        b"DXT1" => decode_dxt(data, width, height, Alpha::None),
        b"DXT3" => decode_dxt(data, width, height, Alpha::Explicit),
        b"DXT5" => decode_dxt(data, width, height, Alpha::Interpolated),
        _ => {
            return Err(format!(
                "不支持的 DDS 格式 {}",
                String::from_utf8_lossy(fourcc)
            ))
        }
    }?;
    Ok(TexImage {
        width,
        height,
        rgba,
    })
}

enum Alpha {
    None,
    Explicit,
    Interpolated,
}

fn expand565(c: u16) -> [u8; 3] {
    let r = (c >> 11) & 0x1f;
    let g = (c >> 5) & 0x3f;
    let b = c & 0x1f;
    [
        (r * 255 / 31) as u8,
        (g * 255 / 63) as u8,
        (b * 255 / 31) as u8,
    ]
}

fn lerp3(a: &[u8; 3], b: &[u8; 3], t: u32) -> [u8; 3] {
    [
        (a[0] as u32 * (3 - t) + b[0] as u32 * t) / 3,
        (a[1] as u32 * (3 - t) + b[1] as u32 * t) / 3,
        (a[2] as u32 * (3 - t) + b[2] as u32 * t) / 3,
    ]
    .map(|v| v as u8)
}

fn decode_dxt(data: &[u8], w: u32, h: u32, alpha_mode: Alpha) -> Result<Vec<u8>, String> {
    let block_bytes = match alpha_mode {
        Alpha::None => 8,
        _ => 16,
    };
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let need = (bw * bh) as usize * block_bytes;
    if data.len() < need {
        return Err(format!("DXT 数据不足: 需要 {need} 有 {}", data.len()));
    }
    let mut out = vec![0u8; (w * h * 4) as usize];

    for by in 0..bh {
        for bx in 0..bw {
            let bi = (by * bw + bx) as usize;
            let blk = &data[bi * block_bytes..bi * block_bytes + block_bytes];
            let (color_off, punchthrough) = match alpha_mode {
                Alpha::None => (0, true),
                _ => (8, false),
            };

            // 颜色块
            let c0 = u16::from_le_bytes([blk[color_off], blk[color_off + 1]]);
            let c1 = u16::from_le_bytes([blk[color_off + 2], blk[color_off + 3]]);
            let c0v = expand565(c0);
            let c1v = expand565(c1);
            let mut palette = [[0u8; 3]; 4];
            palette[0] = c0v;
            palette[1] = c1v;
            if punchthrough && c0 <= c1 {
                palette[2] = [
                    ((c0v[0] as u32 + c1v[0] as u32) / 2) as u8,
                    ((c0v[1] as u32 + c1v[1] as u32) / 2) as u8,
                    ((c0v[2] as u32 + c1v[2] as u32) / 2) as u8,
                ];
                palette[3] = [0, 0, 0];
            } else {
                palette[2] = lerp3(&c0v, &c1v, 1);
                palette[3] = lerp3(&c0v, &c1v, 2);
            }

            // alpha 块
            let mut alpha_px = [255u8; 16];
            match alpha_mode {
                Alpha::Explicit => {
                    for i in 0..8 {
                        let b = blk[i];
                        alpha_px[i * 2] = (b & 0x0f) * 17;
                        alpha_px[i * 2 + 1] = (b >> 4) * 17;
                    }
                }
                Alpha::Interpolated => {
                    let a0 = blk[0];
                    let a1 = blk[1];
                    let mut alphas = [0u8; 8];
                    alphas[0] = a0;
                    alphas[1] = a1;
                    if a0 > a1 {
                        for i in 0..6 {
                            alphas[i + 2] =
                                ((a0 as u32 * (6 - i as u32) + a1 as u32 * (i as u32)) / 6) as u8;
                        }
                    } else {
                        for i in 0..4 {
                            alphas[i + 2] =
                                ((a0 as u32 * (4 - i as u32) + a1 as u32 * (i as u32)) / 4) as u8;
                        }
                        alphas[6] = 0;
                        alphas[7] = 255;
                    }
                    let mut bits = u64::from_le_bytes(blk[2..8].try_into().unwrap());
                    for slot in alpha_px.iter_mut().take(16) {
                        *slot = alphas[(bits & 7) as usize];
                        bits >>= 3;
                    }
                }
                Alpha::None => {}
            }

            // 16 像素
            let idx_base = color_off + 4;
            for py in 0..4 {
                for px in 0..4 {
                    let x = bx * 4 + px;
                    let y = by * 4 + py;
                    if x >= w || y >= h {
                        continue;
                    }
                    let slot = (py * 4 + px) as usize;
                    let bits = (blk[idx_base + slot / 4] >> ((slot % 4) * 2)) & 0x3;
                    let (rgb, a) = if punchthrough && c0 <= c1 && bits == 3 {
                        ([0, 0, 0], 0u8)
                    } else {
                        (palette[bits as usize], alpha_px[slot])
                    };
                    let o = ((y * w + x) * 4) as usize;
                    out[o] = rgb[0];
                    out[o + 1] = rgb[1];
                    out[o + 2] = rgb[2];
                    out[o + 3] = a;
                }
            }
        }
    }
    Ok(out)
}

