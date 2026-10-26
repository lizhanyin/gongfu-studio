//! 整体 GLB 导出(所见即所得):可见 fskin 部件 + FSKEL 骨架蒙皮 + 动作动画
//! + FSM 帽子挂骨。与视口(viewer.ts CPU 蒙皮)的对应关系:
//!
//! - 引擎蒙皮语义(F3D.dll SkinStream_SkinVerts_LBS):pos = Σ w·M_bone·boneLocalPos,
//!   每影响自带骨骼局部坐标,原始权重不归一。glTF 标准 LBS 只能表达「归一权重 +
//!   统一绑定位置」,因此导出取:绑定姿态 = 首帧蒙皮位置、权重归一、
//!   IBM_j = 首帧骨骼世界矩阵的逆。单影响顶点任意帧与引擎逐位一致;混合顶点
//!   首帧精确,其余帧为标准 LBS 线性近似(与所有 glTF 蒙皮管线相同)。
//! - 无骨架时部件退化为静态网格(authored pos,无 skin/动画);部件引用的骨骼
//!   超出骨架范围(或权重和为 0)时该顶点挂「兜底骨骼」(identity 节点,
//!   IBM=I),保持 authored 位置不参与蒙皮——对应 viewer skinPart 的绑定姿势兜底。
//! - 帽子:挂到目标骨骼节点下,holder 矩阵 = 首帧骨骼世界矩阵的逆,再叠加
//!   用户偏移——与 viewer refreshBoneAttached 同口径,之后随骨骼刚性跟随。
//!   挂点骨名为空时自动选含 head / tou 的骨骼(同 autoHatBone)。
//! - 贴图文件路径由前端解析传入(与视口显示同源,Rust 不再猜目录),读字节后
//!   DDS/TGA/BMP 解码为 PNG 嵌入,png/jpg 原样嵌入;跨缝 UV 靠 REPEAT 寻址
//!   (无需 OBJ 导出那样的顶点拆分),UV v 翻转(引擎左下原点 → glTF 左上)。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;

use crate::fskel;
use crate::fsm;
use crate::fskin;
use crate::vfs;
use crate::dds::{self, TexImage};

/// 一个待导出的 fskin 部件:模型路径 + 每材质槽的贴图文件(None = 该槽无贴图)
pub struct GlbPart {
    pub path: String,
    pub textures: Vec<Option<String>>,
}

/// FSM 帽子:模型 + 挂点骨名(空 = 自动)+ 显示偏移
pub struct GlbHat {
    pub path: String,
    pub bone: String,
    pub offset: [f32; 3],
}

// ---------------------------------------------------------------- 矩阵工具
// 4x4 行主序 [[f32;4];4],点变换按齐次行向量 x·M

type M4 = [[f32; 4]; 4];

fn mat_identity() -> M4 {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-10 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

/// T·R(四元数)复合为 4x4
fn mat_from_tr(t: [f32; 3], q: [f32; 4]) -> M4 {
    let [x, y, z, w] = quat_normalize(q);
    let m = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    [
        [m[0][0], m[0][1], m[0][2], t[0]],
        [m[1][0], m[1][1], m[1][2], t[1]],
        [m[2][0], m[2][1], m[2][2], t[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn mat_mul(a: &M4, b: &M4) -> M4 {
    let mut o = [[0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            o[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    o
}

/// 通用 4x4 求逆(余子式),奇异返回 None
fn mat_invert(m: &M4) -> Option<M4> {
    let mut cof = [[0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mut minor = [[0f32; 3]; 3];
            let mut mi = 0;
            for r in 0..4 {
                if r == i {
                    continue;
                }
                let mut mj = 0;
                for c in 0..4 {
                    if c == j {
                        continue;
                    }
                    minor[mi][mj] = m[r][c];
                    mj += 1;
                }
                mi += 1;
            }
            let det = |mm: &[[f32; 3]; 3]| {
                mm[0][0] * (mm[1][1] * mm[2][2] - mm[1][2] * mm[2][1])
                    - mm[0][1] * (mm[1][0] * mm[2][2] - mm[1][2] * mm[2][0])
                    + mm[0][2] * (mm[1][0] * mm[2][1] - mm[1][1] * mm[2][0])
            };
            cof[i][j] = det(&minor)
                * if (i + j) % 2 == 0 { 1.0 } else { -1.0 };
        }
    }
    // 按第 0 行展开(与 cof[i][j] = (-1)^(i+j)·minor 的定义配套)
    let det = m[0][0] * cof[0][0] + m[0][1] * cof[0][1] + m[0][2] * cof[0][2] + m[0][3] * cof[0][3];
    if det.abs() < 1e-12 {
        return None;
    }
    let mut inv = [[0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            inv[i][j] = cof[j][i] / det;
        }
    }
    Some(inv)
}

/// 齐次点变换
fn xf(m: &M4, p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

/// 列主序 16 元素(glTF node.matrix / IBM accessor 布局)
fn mat_col_major(m: &M4) -> [f32; 16] {
    let mut o = [0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            o[c * 4 + r] = m[r][c];
        }
    }
    o
}

fn f32_bytes(data: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(data.len() * 4);
    for v in data {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn u16_bytes(data: &[u16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(data.len() * 2);
    for v in data {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn u32_bytes(data: &[u32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(data.len() * 4);
    for v in data {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n < 1e-10 {
        return v;
    }
    [v[0] / n, v[1] / n, v[2] / n]
}

// ---------------------------------------------------------------- GLB 构建器

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;

/// bin 缓冲 + bufferView / accessor 收集
struct GlbBuilder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl GlbBuilder {
    fn new() -> Self {
        GlbBuilder {
            bin: Vec::new(),
            views: Vec::new(),
            accessors: Vec::new(),
        }
    }

    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let off = self.bin.len();
        self.bin.extend_from_slice(bytes);
        let mut v = json!({"buffer": 0, "byteOffset": off, "byteLength": bytes.len()});
        if let Some(t) = target {
            v["target"] = (t as u32).into();
        }
        self.views.push(v);
        self.views.len() - 1
    }

    /// 通用 accessor:bytes 为已编码好的原始分量数据
    fn accessor(
        &mut self,
        bytes: &[u8],
        component_type: u32,
        count: usize,
        ty: &str,
        target: Option<u32>,
        minmax: Option<(Vec<f32>, Vec<f32>)>,
    ) -> usize {
        let vi = self.view(bytes, target);
        let mut a = json!({
            "bufferView": vi, "componentType": component_type,
            "count": count, "type": ty,
        });
        if let Some((mn, mx)) = minmax {
            a["min"] = json!(mn);
            a["max"] = json!(mx);
        }
        self.accessors.push(a);
        self.accessors.len() - 1
    }

    fn f32s(&mut self, data: &[f32], ty: &str, target: Option<u32>) -> usize {
        let comps = match ty {
            "MAT4" => 16,
            "VEC4" => 4,
            "VEC3" => 3,
            "VEC2" => 2,
            _ => 1,
        };
        let count = data.len() / comps;
        self.accessor(&f32_bytes(data), 5126, count, ty, target, None)
    }

    /// POSITION 专用(带 min/max,规范建议部分查看器需要)
    fn positions(&mut self, data: &[f32]) -> usize {
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for c in data.chunks_exact(3) {
            for k in 0..3 {
                mn[k] = mn[k].min(c[k]);
                mx[k] = mx[k].max(c[k]);
            }
        }
        self.accessor(
            &f32_bytes(data),
            5126,
            data.len() / 3,
            "VEC3",
            Some(ARRAY_BUFFER),
            Some((mn.to_vec(), mx.to_vec())),
        )
    }

    fn u16s(&mut self, data: &[u16], ty: &str, target: Option<u32>) -> usize {
        // accessor 的 count 是元素数(VEC4 = 4 个 u16 一组)
        let comps = match ty {
            "VEC4" => 4,
            "VEC3" => 3,
            "VEC2" => 2,
            _ => 1,
        };
        self.accessor(&u16_bytes(data), 5123, data.len() / comps, ty, target, None)
    }

    /// SCALAR + min/max(动画输入 accessor 的规范要求)
    fn times(&mut self, data: &[f32]) -> usize {
        let mn = data.iter().cloned().fold(f32::MAX, f32::min);
        let mx = data.iter().cloned().fold(f32::MIN, f32::max);
        self.accessor(
            &f32_bytes(data),
            5126,
            data.len(),
            "SCALAR",
            None,
            Some((vec![mn], vec![mx])),
        )
    }

    fn u32s(&mut self, data: &[u32]) -> usize {
        self.accessor(&u32_bytes(data), 5125, data.len(), "SCALAR", Some(ELEMENT_ARRAY_BUFFER), None)
    }
}

// ---------------------------------------------------------------- 纹理:PNG / TGA / BMP

fn crc32(data: &[u8]) -> u32 {
    // PNG 用反射多项式 0xEDB88320,逐位计算(无表,编码尺寸小、速度足够)
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// zlib stored 块打包(不压缩;游戏贴图多为 256/512,DXT 解码后体积可控)
fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 65535 * 5 + 16);
    out.push(0x78);
    out.push(0x01);
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0, 0, 0xff, 0xff]); // 单个空终块
        out.extend_from_slice(&adler32(data).to_be_bytes());
        return out;
    }
    let mut chunks = data.chunks(65535).peekable();
    while let Some(c) = chunks.next() {
        // 只有最后一块 BFINAL=1,否则解码器会在中途停
        let bfinal = if chunks.peek().is_none() { 0x01 } else { 0x00 };
        out.push(bfinal); // BTYPE=00(stored)
        let len = c.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

/// RGBA8(自顶向下行序)→ PNG 字节。色型 6(真彩+alpha)、8bit、filter 全 0。
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = (width * 4) as usize;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in rgba.chunks_exact(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    out.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    out.extend_from_slice(&png_chunk(b"IDAT", &zlib_store(&raw)));
    out.extend_from_slice(&png_chunk(b"IEND", &[]));
    out
}

/// TGA 解码(与前端 tex.ts decodeTga 同口径):真彩 24/32bit、灰度 8bit,
/// 未压缩与 RLE;原点左下时翻转为自顶向下。色表 / 16bit 不支持。
pub fn decode_tga(bytes: &[u8]) -> Result<TexImage, String> {
    if bytes.len() < 18 {
        return Err("TGA 头不完整".into());
    }
    let cmap_type = bytes[1];
    let ty = bytes[2];
    let w = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let h = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let depth = bytes[16];
    if w == 0 || h == 0 {
        return Err(format!("TGA 尺寸无效 {w}x{h}"));
    }
    if cmap_type != 0 {
        return Err("不支持色表 TGA".into());
    }
    let rle = ty == 10 || ty == 11;
    if ty != 2 && ty != 3 && ty != 10 && ty != 11 {
        return Err(format!("不支持的 TGA 类型 {ty}"));
    }
    let bpp = (depth >> 3) as usize;
    if bpp != 1 && bpp != 3 && bpp != 4 {
        return Err(format!("不支持的 TGA 位深 {depth}"));
    }
    let mut p = 18 + bytes[0] as usize;
    let n = (w as usize) * (h as usize);
    let mut px = vec![0u8; n * 4];
    let mut i = 0usize;
    let px_at = |p: usize| -> Result<[u8; 4], String> {
        if p + bpp > bytes.len() {
            return Err("TGA 数据不完整".into());
        }
        if bpp == 1 {
            return Ok([bytes[p], bytes[p], bytes[p], 255]);
        }
        Ok([
            bytes[p + 2],
            bytes[p + 1],
            bytes[p],
            if bpp == 4 { bytes[p + 3] } else { 255 },
        ])
    };
    let mut put = |v: [u8; 4], i: &mut usize| {
        px[*i..*i + 4].copy_from_slice(&v);
        *i += 4;
    };
    if !rle {
        for k in 0..n {
            let v = px_at(p + k * bpp)?;
            put(v, &mut i);
        }
    } else {
        let mut k = 0usize;
        while k < n {
            if p >= bytes.len() {
                return Err("TGA 数据不完整".into());
            }
            let hdr = bytes[p];
            p += 1;
            let cnt = (hdr & 0x7f) as usize + 1;
            if hdr & 0x80 != 0 {
                let v = px_at(p)?;
                p += bpp;
                for _ in 0..cnt {
                    put(v, &mut i);
                }
            } else {
                for _ in 0..cnt {
                    let v = px_at(p)?;
                    p += bpp;
                    put(v, &mut i);
                }
            }
            k += cnt;
        }
    }
    if bytes[17] & 0x20 == 0 {
        let row = (w as usize) * 4;
        for y in 0..(h as usize / 2) {
            let a = y * row;
            let b = (h as usize - 1 - y) * row;
            for k in 0..row {
                px.swap(a + k, b + k);
            }
        }
    }
    Ok(TexImage {
        width: w,
        height: h,
        rgba: px,
    })
}

/// BMP 解码:未压缩(BI_RGB)24/32bit,top-down(负高)与 bottom-up 均支持。
/// 32bit 的 alpha 全 0 时按不透明处理(部分工具导出的 BI_RGB 不写 alpha)。
pub fn decode_bmp(bytes: &[u8]) -> Result<TexImage, String> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return Err("不是有效的 BMP 文件".into());
    }
    let rd_u32 = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let rd_i32 = |o: usize| rd_u32(o) as i32;
    let pixel_off = rd_u32(10) as usize;
    let dib = rd_u32(14) as usize;
    if dib < 40 {
        return Err("不支持的核心 BMP 头".into());
    }
    let w = rd_i32(18);
    let h_abs = rd_i32(22).abs();
    let bpp = u16::from_le_bytes([bytes[28], bytes[29]]) as u32;
    let compression = rd_u32(30);
    if w <= 0 || h_abs <= 0 {
        return Err(format!("BMP 尺寸无效 {w}x{h_abs}"));
    }
    if compression != 0 {
        return Err(format!("不支持压缩 BMP (biCompression={compression})"));
    }
    if bpp != 24 && bpp != 32 {
        return Err(format!("不支持的 BMP 位深 {bpp}"));
    }
    let (w, h) = (w as u32, h_abs as u32);
    let top_down = rd_i32(22) < 0;
    let stride = (((bpp as usize * w as usize) + 31) / 32) * 4;
    let need = pixel_off + stride * h as usize;
    if bytes.len() < need {
        return Err("BMP 像素数据不完整".into());
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for row in 0..h as usize {
        let src_y = if top_down { row } else { h as usize - 1 - row };
        let src = pixel_off + src_y * stride;
        let dst = row * (w as usize) * 4;
        for x in 0..w as usize {
            let s = src + x * (bpp as usize / 8);
            let d = dst + x * 4;
            out[d] = bytes[s + 2];
            out[d + 1] = bytes[s + 1];
            out[d + 2] = bytes[s];
            out[d + 3] = if bpp == 32 { bytes[s + 3] } else { 255 };
        }
    }
    if bpp == 32 {
        let all_zero = out.chunks_exact(4).all(|c| c[3] == 0);
        if all_zero {
            for c in out.chunks_exact_mut(4) {
                c[3] = 255;
            }
        }
    }
    Ok(TexImage {
        width: w,
        height: h,
        rgba: out,
    })
}

// ---------------------------------------------------------------- 图片嵌入

/// 读取贴图文件并转为可嵌入的 (字节, mimeType):DDS/TGA/BMP 解码后编 PNG,
/// png/jpg 原样嵌入。失败返回 None(材质以纯色导出,不中断整体导出)。
fn load_embedded_image(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let bytes = vfs::read_file(path).ok()?;
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or_default();
    match ext {
        "png" => Some((bytes, "image/png")),
        "jpg" | "jpeg" => Some((bytes, "image/jpeg")),
        "dds" => {
            let t = dds::decode_dds(&bytes).ok()?;
            Some((encode_png(t.width, t.height, &t.rgba), "image/png"))
        }
        "tga" => {
            let t = decode_tga(&bytes).ok()?;
            Some((encode_png(t.width, t.height, &t.rgba), "image/png"))
        }
        "bmp" => {
            let t = decode_bmp(&bytes).ok()?;
            Some((encode_png(t.width, t.height, &t.rgba), "image/png"))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- 顶点构建

/// 一个部件展开后的顶点缓冲(几何 + 可选蒙皮属性)
struct PartVerts {
    pos: Vec<f32>,
    nrm: Vec<f32>,
    uv: Vec<f32>,
    joints: Vec<u16>,
    weights: Vec<f32>,
    /// 任一顶点用到兜底骨骼(需要追加 identity 兜底骨)
    used_fallback: bool,
    /// 因影响数 >4 被截断的顶点数
    dropped_influences: usize,
}

const MAX_INFLUENCES: usize = 4;

/// fskin 顶点 → 绑定姿态(首帧蒙皮)几何 + joints/weights。
/// world0 = None 时为静态导出(authored pos,不产出蒙皮属性)。
fn build_fskin_verts(
    m: &fskin::FskinModel,
    world0: Option<&[M4]>,
    bone_count: usize,
    fallback_joint: usize,
) -> PartVerts {
    let n = m.vertices.len();
    let mut v = PartVerts {
        pos: Vec::with_capacity(n * 3),
        nrm: Vec::with_capacity(n * 3),
        uv: Vec::with_capacity(n * 2),
        joints: Vec::with_capacity(n * 4),
        weights: Vec::with_capacity(n * 4),
        used_fallback: false,
        dropped_influences: 0,
    };
    let skinned = world0.is_some();
    for vtx in &m.vertices {
        let mut p = vtx.pos;
        if skinned {
            let world = world0.unwrap();
            let valid: Vec<&fskin::Influence> = vtx
                .influences
                .iter()
                .filter(|i| (i.bone_index as usize) < bone_count)
                .collect();
            let wsum: f32 = valid.iter().map(|i| i.weight).sum();
            if valid.is_empty() || wsum.abs() < 1e-6 {
                // 无有效骨骼:保持 authored 位置,挂兜底骨(IBM=I ⇒ 不动)
                v.used_fallback = true;
                v.joints.extend_from_slice(&[
                    fallback_joint as u16,
                    0,
                    0,
                    0,
                ]);
                v.weights.extend_from_slice(&[1.0, 0.0, 0.0, 0.0]);
            } else {
                // 绑定位置 = 归一化权重的首帧蒙皮(与导出后静止姿态一致)
                let mut b = [0f32; 3];
                for inf in &valid {
                    let x = xf(&world[inf.bone_index as usize], inf.bone_local_pos);
                    let w = inf.weight / wsum;
                    b[0] += w * x[0];
                    b[1] += w * x[1];
                    b[2] += w * x[2];
                }
                p = b;
                // 同骨多影响合并(glTF 每顶点每骨至多一次;合并是引擎加权
                // 累加的等价重排),再按权重取前 4 归一
                let mut merged: Vec<(u32, f32)> = Vec::new();
                for inf in &valid {
                    match merged.iter_mut().find(|(bn, _)| *bn == inf.bone_index) {
                        Some(e) => e.1 += inf.weight,
                        None => merged.push((inf.bone_index, inf.weight)),
                    }
                }
                merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                if merged.len() > MAX_INFLUENCES {
                    v.dropped_influences += 1;
                    merged.truncate(MAX_INFLUENCES);
                }
                let sum: f32 = merged.iter().map(|(_, w)| *w).sum();
                for (bone, w) in &merged {
                    v.joints.push(*bone as u16);
                    v.weights.push(w / sum);
                }
                for _ in merged.len()..MAX_INFLUENCES {
                    v.joints.push(0);
                    v.weights.push(0.0);
                }
            }
        }
        let nrm = normalize3(vtx.normal);
        v.pos.extend_from_slice(&p);
        v.nrm.extend_from_slice(&nrm);
        v.uv.extend_from_slice(&[vtx.uv[0], 1.0 - vtx.uv[1]]);
    }
    v
}

// ---------------------------------------------------------------- 导出主体

/// 帽子挂点骨解析:与 viewer autoHatBone / refreshBoneAttached 同口径——
/// 指定名先忽略大小写全等、再包含;为空自动选含 head / tou 的骨骼。
fn resolve_hat_bone(name: &str, bone_names: &[String]) -> Option<usize> {
    let find = |pred: &dyn Fn(&str) -> bool| bone_names.iter().position(|n| pred(&n.to_lowercase()));
    if !name.is_empty() {
        let lower = name.to_lowercase();
        return find(&|n| n == &lower).or_else(|| find(&|n| n.contains(&lower)));
    }
    find(&|n| n.contains("head")).or_else(|| find(&|n| n.contains("tou")))
}

fn stem_of_path(path: &str) -> String {
    let base = path.rsplit(['\\', '/']).next().unwrap_or(path);
    base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base).to_string()
}

/// 导出整体 GLB。返回摘要文案。
pub fn export_full_glb(
    parts: &[GlbPart],
    fskel_path: Option<&str>,
    hat: Option<&GlbHat>,
    out_path: &str,
) -> Result<String, String> {
    if parts.is_empty() && hat.is_none() {
        return Err("没有可导出的部件".into());
    }

    // ---- 解析部件与骨架
    let mut models: Vec<(fskin::FskinModel, Vec<Option<String>>)> = Vec::new();
    for p in parts {
        let bytes = vfs::read_file(&p.path).map_err(|e| format!("读取 {}: {e}", p.path))?;
        let m = fskin::parse_fskin(&p.path, &bytes)?;
        models.push((m, p.textures.clone()));
    }
    let skel = match fskel_path {
        Some(s) => {
            let f = fskel::get_or_load(s).map_err(|e| format!("读取骨架 {s}: {e}"))?;
            if f.frame_count() == 0 {
                return Err("骨架没有帧数据,无法确定绑定姿态".into());
            }
            Some(f)
        }
        None => None,
    };

    // ---- 首帧各骨世界矩阵(绑定姿态)与 glTF 局部 TRS
    let bc = skel.as_ref().map(|f| f.bone_count() as usize).unwrap_or(0);
    let mut world0: Vec<M4> = Vec::new();
    let mut bone_nodes: Vec<Value> = Vec::new();
    let mut bone_children: Vec<Vec<usize>> = vec![Vec::new(); bc];
    let mut scene_roots: Vec<usize> = Vec::new();
    if let Some(f) = &skel {
        let f0 = f.frames_flat(0, 1); // 帧 × 骨 × (t3,q4)
        for b in 0..bc {
            let o = b * 7;
            let t = [f0[o], f0[o + 1], f0[o + 2]];
            let q = [f0[o + 3], f0[o + 4], f0[o + 5], f0[o + 6]];
            let local = mat_from_tr(t, q);
            let parent = f.parents[b];
            world0.push(if parent >= 0 {
                mat_mul(&world0[parent as usize], &local)
            } else {
                local
            });
            if parent >= 0 {
                bone_children[parent as usize].push(b);
            } else {
                scene_roots.push(b);
            }
            let qn = quat_normalize(q);
            bone_nodes.push(json!({
                "name": f.bone_names[b],
                "translation": t,
                "rotation": qn,
            }));
        }
    }

    let mut g = GlbBuilder::new();
    let mut nodes: Vec<Value> = Vec::new();
    for (b, mut node) in bone_nodes.drain(..).enumerate() {
        if !bone_children[b].is_empty() {
            node["children"] = json!(bone_children[b]);
        }
        nodes.push(node);
    }

    // 兜底骨骼(index = bc):无有效影响的顶点挂它(IBM=I ⇒ authored 位置不动)
    let fallback_joint = bc;
    let mut used_fallback = false;
    let mut dropped = 0usize;

    // ---- 贴图 / 材质缓存(跨部件共享)
    let mut images: Vec<Value> = Vec::new();
    let mut textures: Vec<Value> = Vec::new();
    let mut image_by_path: HashMap<String, usize> = HashMap::new();
    let mut tex_by_image: HashMap<usize, usize> = HashMap::new();
    let mut materials_json: Vec<Value> = Vec::new();

    let sampler_idx = 0usize; // 唯一 REPEAT sampler
    let mut get_texture = |file: &Option<String>, g: &mut GlbBuilder| -> Option<usize> {
        let file = file.as_ref()?;
        let key = file.to_lowercase();
        let image_idx = match image_by_path.get(&key) {
            Some(i) => *i,
            None => {
                let (bytes, mime) = load_embedded_image(file)?;
                let vi = g.view(&bytes, None);
                images.push(json!({"bufferView": vi, "mimeType": mime}));
                let i = images.len() - 1;
                image_by_path.insert(key, i);
                i
            }
        };
        Some(*tex_by_image.entry(image_idx).or_insert_with(|| {
            textures.push(json!({"sampler": sampler_idx, "source": image_idx}));
            textures.len() - 1
        }))
    };

    // 材质槽 → glTF material(viewer 口径:diffuse 底色、emissive 自发光、
    // roughness 0.75 / metallic 0、双面)
    let mut make_material = |diffuse: Option<&[f32; 4]>,
                             emissive: Option<&[f32; 4]>,
                             tex: Option<usize>|
     -> usize {
        let d = diffuse.copied().unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let e = emissive.copied().unwrap_or([0.0; 4]);
        let mut m = json!({
            "pbrMetallicRoughness": {
                "baseColorFactor": d,
                "metallicFactor": 0.0,
                "roughnessFactor": 0.75,
            },
            "emissiveFactor": [e[0], e[1], e[2]],
            "doubleSided": true,
        });
        if let Some(ti) = tex {
            m["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index": ti});
        }
        if d[3] < 0.999 {
            m["alphaMode"] = "BLEND".into();
        }
        materials_json.push(m);
        materials_json.len() - 1
    };

    // ---- fskin 部件网格
    let mut meshes_json: Vec<Value> = Vec::new();
    let mut mesh_nodes: Vec<Value> = Vec::new();
    for (m, tex_files) in &models {
        let verts = build_fskin_verts(m, skel.as_ref().map(|_| world0.as_slice()), bc, fallback_joint);
        used_fallback |= verts.used_fallback;
        dropped += verts.dropped_influences;

        let pos_acc = g.positions(&verts.pos);
        let nrm_acc = g.f32s(&verts.nrm, "VEC3", Some(ARRAY_BUFFER));
        let uv_acc = g.f32s(&verts.uv, "VEC2", Some(ARRAY_BUFFER));
        let use32 = verts.pos.len() / 3 > u16::MAX as usize;

        let mut attributes = json!({"POSITION": pos_acc, "NORMAL": nrm_acc, "TEXCOORD_0": uv_acc});
        if skinned_for_part(&verts) {
            let j_acc = g.u16s(&verts.joints, "VEC4", Some(ARRAY_BUFFER));
            let w_acc = g.f32s(&verts.weights, "VEC4", Some(ARRAY_BUFFER));
            attributes["JOINTS_0"] = j_acc.into();
            attributes["WEIGHTS_0"] = w_acc.into();
        }

        // 每子网格一个 primitive,材质槽与子网格逐槽对应(与 viewer 分组一致)
        let mut primitives: Vec<Value> = Vec::new();
        for (k, s) in m.submeshes.iter().enumerate() {
            if s.triangles.is_empty() {
                continue;
            }
            let idx_acc = if use32 {
                g.u32s(&s.triangles.iter().map(|&t| t as u32).collect::<Vec<_>>())
            } else {
                g.u16s(&s.triangles, "SCALAR", Some(ELEMENT_ARRAY_BUFFER))
            };
            let tex = get_texture(&tex_files.get(k).cloned().unwrap_or(None), &mut g);
            let mat = m.materials.get(k);
            let mi = make_material(
                mat.map(|x| &x.diffuse),
                mat.map(|x| &x.emissive),
                tex,
            );
            primitives.push(json!({"attributes": attributes, "indices": idx_acc, "material": mi}));
        }
        if primitives.is_empty() {
            continue; // 无三角形部件(纯特效挂点)跳过
        }
        meshes_json.push(json!({"primitives": primitives}));
        let mut node = json!({"mesh": meshes_json.len() - 1, "name": stem_of_path(&m.path)});
        if skinned_for_part(&verts) {
            node["skin"] = 0.into();
        }
        mesh_nodes.push(node);
    }

    // ---- skin(joints + IBM;所有蒙皮部件共享一个 skin)
    let mut skins_json: Vec<Value> = Vec::new();
    let has_skin = skel.is_some()
        && (mesh_nodes.iter().any(|n| n.get("skin").is_some()) || used_fallback);
    if has_skin {
        let mut joints: Vec<usize> = (0..bc).collect();
        if used_fallback {
            joints.push(fallback_joint);
            nodes.push(json!({"name": "BindFallback"}));
            scene_roots.push(nodes.len() - 1);
        }
        // 注意:fallback 节点已 push,下面 IBM 数组按下标对齐 joints
        let mut ibm: Vec<f32> = Vec::with_capacity(joints.len() * 16);
        for (j, &bone) in joints.iter().enumerate() {
            let m = if j < bc {
                mat_invert(&world0[bone]).unwrap_or_else(mat_identity)
            } else {
                mat_identity()
            };
            ibm.extend_from_slice(&mat_col_major(&m));
        }
        let ibm_acc = g.f32s(&ibm, "MAT4", None);
        skins_json.push(json!({"joints": joints, "inverseBindMatrices": ibm_acc, "name": "Skin"}));
    }

    // ---- 帽子(FSM)挂骨
    if let Some(h) = hat {
        let bytes = vfs::read_file(&h.path).map_err(|e| format!("读取 {}: {e}", h.path))?;
        let m = fsm::parse_fsm(&h.path, &bytes)?;
        let n = m.vertex_count as usize;
        let mut pos = Vec::with_capacity(n * 3);
        let mut nrm = Vec::with_capacity(n * 3);
        let mut uv = Vec::with_capacity(n * 2);
        for i in 0..n {
            let o = i * 8;
            pos.extend_from_slice(&m.vertices[o..o + 3]);
            let nn = normalize3([m.vertices[o + 3], m.vertices[o + 4], m.vertices[o + 5]]);
            nrm.extend_from_slice(&nn);
            uv.extend_from_slice(&[m.vertices[o + 6], 1.0 - m.vertices[o + 7]]);
        }
        let pos_acc = g.positions(&pos);
        let nrm_acc = g.f32s(&nrm, "VEC3", Some(ARRAY_BUFFER));
        let uv_acc = g.f32s(&uv, "VEC2", Some(ARRAY_BUFFER));
        let use32 = n > u16::MAX as usize;
        let mut primitives: Vec<Value> = Vec::new();
        for (k, s) in m.submeshes.iter().enumerate() {
            if s.triangles.is_empty() {
                continue;
            }
            let idx_acc = if use32 {
                g.u32s(&s.triangles.iter().map(|&t| t as u32).collect::<Vec<_>>())
            } else {
                g.u16s(&s.triangles, "SCALAR", Some(ELEMENT_ARRAY_BUFFER))
            };
            // 帽子部件无前端贴图列表:按材质 stem 在模型根目录树中查找(与 OBJ 导出同口径)
            let tex = m.materials
                .get(k)
                .map(|md| {
                    let stem = md
                        .texture_path
                        .rsplit(['\\', '/'])
                        .next()
                        .and_then(|b| b.rsplit_once('.').map(|(s, _)| s.to_string()));
                    stem
                })
                .flatten()
                .and_then(|stem| find_texture_stem(&model_root_of(&h.path)?, &stem))
                .and_then(|p| get_texture(&Some(p), &mut g));
            let mat = m.materials.get(k);
            let mi = make_material(mat.map(|x| &x.diffuse), mat.map(|x| &x.emissive), tex);
            primitives.push(json!({"attributes": {
                "POSITION": pos_acc, "NORMAL": nrm_acc, "TEXCOORD_0": uv_acc,
            }, "indices": idx_acc, "material": mi}));
        }
        if !primitives.is_empty() {
            meshes_json.push(json!({"primitives": primitives}));
            let mesh_idx = meshes_json.len() - 1;
            // 挂接链:骨骼 > holder(矩阵=挂接姿态骨骼世界矩阵的逆)> inner(偏移)> mesh
            let attach = skel.as_ref().and_then(|f| resolve_hat_bone(&h.bone, &f.bone_names));
            match attach {
                Some(b) => {
                    let inv = mat_invert(&world0[b]).unwrap_or_else(mat_identity);
                    nodes.push(json!({"name": "HatHolder", "matrix": mat_col_major(&inv)}));
                    let holder = nodes.len() - 1;
                    nodes.push(json!({"translation": h.offset, "mesh": mesh_idx, "name": "Hat"}));
                    let inner = nodes.len() - 1;
                    nodes[holder]["children"] = json!([inner]);
                    // 挂到骨骼节点下(骨骼节点已收集完毕,直接补 children)
                    let mut ch = nodes[b]["children"].as_array().cloned().unwrap_or_default();
                    ch.push(holder.into());
                    nodes[b]["children"] = json!(ch);
                }
                None => {
                    nodes.push(json!({
                        "translation": h.offset, "mesh": mesh_idx, "name": "Hat",
                    }));
                    scene_roots.push(nodes.len() - 1);
                }
            }
        }
    }

    // mesh 节点挂场景根(skinned mesh 的节点变换被 skin 语义忽略,挂根即可)
    let first_mesh_node = nodes.len();
    for n in mesh_nodes {
        nodes.push(n);
        scene_roots.push(nodes.len() - 1);
    }
    let _ = first_mesh_node;

    // ---- 动画:每个动作一条(骨骼 rotation + translation,LINEAR)
    let mut animations: Vec<Value> = Vec::new();
    if let Some(f) = &skel {
        for a in &f.actions {
            let from = a.first_frame as usize;
            let to = from + a.frame_count as usize;
            let times: Vec<f32> = (0..to - from).map(|k| k as f32 * f.frame_dt).collect();
            let ti = g.times(&times);
            let flat = f.frames_flat(from as u32, to as u32);
            let frames = to - from;
            let mut channels: Vec<Value> = Vec::new();
            let mut samplers: Vec<Value> = Vec::new();
            for b in 0..bc {
                let mut rot: Vec<f32> = Vec::with_capacity(frames * 4);
                let mut tr: Vec<f32> = Vec::with_capacity(frames * 3);
                for fr in 0..frames {
                    let o = (fr * bc + b) * 7;
                    rot.extend_from_slice(&quat_normalize([
                        flat[o + 3],
                        flat[o + 4],
                        flat[o + 5],
                        flat[o + 6],
                    ]));
                    tr.extend_from_slice(&flat[o..o + 3]);
                }
                let ri = g.f32s(&rot, "VEC4", None);
                samplers.push(json!({"input": ti, "output": ri, "interpolation": "LINEAR"}));
                channels.push(json!({"sampler": samplers.len() - 1, "target": {"node": b, "path": "rotation"}}));
                let tri = g.f32s(&tr, "VEC3", None);
                samplers.push(json!({"input": ti, "output": tri, "interpolation": "LINEAR"}));
                channels.push(json!({"sampler": samplers.len() - 1, "target": {"node": b, "path": "translation"}}));
            }
            animations.push(json!({
                "name": a.name, "channels": channels, "samplers": samplers,
            }));
        }
    }

    // ---- 组装 GLB
    while g.bin.len() % 4 != 0 {
        g.bin.push(0);
    }
    let mut doc = json!({
        "asset": {"version": "2.0", "generator": "Gongfu Studio"},
        "scene": 0,
        "scenes": [{"nodes": scene_roots}],
        "nodes": nodes,
        "meshes": meshes_json,
        "materials": materials_json,
        "samplers": [{"wrapS": 10497, "wrapT": 10497, "magFilter": 9729, "minFilter": 9987}],
    });
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
    }
    if !skins_json.is_empty() {
        doc["skins"] = json!(skins_json);
    }
    if !animations.is_empty() {
        doc["animations"] = json!(animations);
    }
    doc["accessors"] = json!(g.accessors);
    doc["bufferViews"] = json!(g.views);
    doc["buffers"] = json!([{"byteLength": g.bin.len()}]);

    let json_chunk = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    let mut json_chunk = json_chunk;
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    let total = 12 + 8 + json_chunk.len() + 8 + g.bin.len();
    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_chunk);
    out.extend_from_slice(&(g.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&g.bin);

    fs::write(out_path, out).map_err(|e| format!("写入失败: {e}"))?;

    let vert_total: usize = models.iter().map(|(m, _)| m.vertices.len()).sum();
    let action_count = skel.as_ref().map(|f| f.actions.len()).unwrap_or(0);
    let mut summary = format!(
        "已导出整体 GLB({} 部件 · {vert_total} 顶点 · {} 材质 · {} 贴图",
        models.len(),
        materials_json.len(),
        images.len(),
    );
    if has_skin {
        summary.push_str(&format!(
            " · {} 骨骼蒙皮 · {} 动作",
            bc + if used_fallback { 1 } else { 0 },
            action_count
        ));
    }
    if let Some(h) = hat {
        summary.push_str(&format!(" · 帽子 {}", stem_of_path(&h.path)));
    }
    if dropped > 0 {
        summary.push_str(&format!("(注意:{dropped} 个 >4 影响顶点已按权重截断)"));
    }
    summary.push_str(&format!("): {out_path}"));
    Ok(summary)
}

/// 部件是否产出蒙皮属性(joints/weights 非空 ⇒ build_fskin_verts 走了蒙皮分支)
fn skinned_for_part(v: &PartVerts) -> bool {
    !v.joints.is_empty()
}

/// 按 stem 在目录树查找贴图(与 obj.rs find_texture 同口径;
/// 帽子无前端解析路径、示例/测试自动导出时使用)
fn find_texture_stem(root: &str, stem: &str) -> Option<String> {
    vfs::walk_files(root, &["dds", "tga", "png", "jpg", "bmp"])
        .into_iter()
        .find(|e| {
            e.name
                .rsplit_once('.')
                .map(|(s, _)| s.eq_ignore_ascii_case(stem))
                .unwrap_or(false)
        })
        .map(|e| e.path)
}

/// 模型根目录(部件路径的祖父目录;游戏资源的标准布局)
fn model_root_of(path: &str) -> Option<String> {
    path.rsplit_once(['\\', '/'])
        .map(|(d, _)| d)
        .and_then(|d| d.rsplit_once(['\\', '/']).map(|(r, _)| r.to_string()))
}

/// 自动解析部件各材质槽贴图(模型根目录树内按 stem 匹配)。
/// 前端(视口同源)路径不可用时(示例 / 测试 / CLI)使用。
pub fn auto_part(path: &str) -> Result<GlbPart, String> {
    let bytes = vfs::read_file(path).map_err(|e| format!("读取 {path}: {e}"))?;
    let m = fskin::parse_fskin(path, &bytes)?;
    let root = model_root_of(path);
    let textures = m
        .materials
        .iter()
        .map(|md| {
            md.texture_path
                .rsplit(['\\', '/'])
                .next()
                .and_then(|b| b.rsplit_once('.').map(|(s, _)| s.to_string()))
                .and_then(|stem| root.as_deref().and_then(|r| find_texture_stem(r, &stem)))
        })
        .collect();
    Ok(GlbPart {
        path: path.to_string(),
        textures,
    })
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    const AG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/AvatarGirl");

    fn load_fskin(name: &str) -> fskin::FskinModel {
        let p = format!("{AG}/{name}");
        fskin::parse_fskin(&p, &fs::read(&p).unwrap()).unwrap()
    }

    /// 首帧各骨世界矩阵(与导出同口径)
    fn frame_worlds(f: &fskel::FskelFile, frame: usize) -> Vec<M4> {
        let bc = f.bone_count() as usize;
        let flat = f.frames_flat(frame as u32, frame as u32 + 1);
        let mut world = Vec::with_capacity(bc);
        for b in 0..bc {
            let o = b * 7;
            let local = mat_from_tr(
                [flat[o], flat[o + 1], flat[o + 2]],
                [flat[o + 3], flat[o + 4], flat[o + 5], flat[o + 6]],
            );
            let parent = f.parents[b];
            world.push(if parent >= 0 {
                mat_mul(&world[parent as usize], &local)
            } else {
                local
            });
        }
        world
    }

    /// PNG 编码 → 手工解 stored zlib → 像素逐字节还原(filter 0)
    #[test]
    fn png_stored_roundtrip() {
        let (w, h) = (7u32, 5u32);
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[x as u8 * 30, y as u8 * 40, 128, 255]);
            }
        }
        let png = encode_png(w, h, &rgba);
        assert_eq!(&png[0..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        // 收集 IDAT
        let mut idat = Vec::new();
        let mut p = 8usize;
        while p + 8 <= png.len() {
            let len = u32::from_be_bytes(png[p..p + 4].try_into().unwrap()) as usize;
            let kind = &png[p + 4..p + 8];
            if kind == b"IDAT" {
                idat.extend_from_slice(&png[p + 8..p + 8 + len]);
            }
            p += 12 + len;
        }
        // 解 stored 块(头 5 字节:BTYPE + LEN + NLEN)
        assert_eq!(&idat[0..2], &[0x78, 0x01]);
        let mut q = 2usize;
        let mut raw = Vec::new();
        loop {
            let bfinal = idat[q] & 1;
            assert_eq!(idat[q] >> 1, 0, "应为 stored 块");
            let len = u16::from_le_bytes(idat[q + 1..q + 3].try_into().unwrap()) as usize;
            raw.extend_from_slice(&idat[q + 5..q + 5 + len]);
            q += 5 + len;
            if bfinal == 1 {
                break;
            }
        }
        // 还原:去每行 filter 字节
        let stride = (w * 4) as usize;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in raw.chunks_exact(stride + 1) {
            out.extend_from_slice(&row[1..]);
        }
        assert_eq!(out, rgba, "PNG stored 像素应逐字节还原");
    }

    /// TGA 解码:手工构造 24bit 未压缩(含左下原点翻转)与 32bit RLE
    #[test]
    fn tga_decode_variants() {
        // 2x2 24bit,原点左下:字节序从底行开始,BGR
        let mut b = vec![0u8; 18];
        b[2] = 2; // 真彩未压缩
        b[12..14].copy_from_slice(&2u16.to_le_bytes());
        b[14..16].copy_from_slice(&2u16.to_le_bytes());
        b[16] = 24;
        b[17] = 0; // 左下原点
        let px: [([u8; 3], [u8; 3]); 2] = [
            ([255, 0, 0], [0, 255, 0]),   // 底行:red green
            ([0, 0, 255], [255, 255, 255]), // 顶行:blue white
        ];
        for row in px {
            for c in [row.0, row.1] {
                b.extend_from_slice(&[c[2], c[1], c[0]]); // BGR
            }
        }
        let t = decode_tga(&b).unwrap();
        assert_eq!((t.width, t.height), (2, 2));
        // 翻转后顶行是 blue white(自顶向下)
        assert_eq!(&t.rgba[0..8], &[0, 0, 255, 255, 255, 255, 255, 255]);
        assert_eq!(&t.rgba[8..16], &[255, 0, 0, 255, 0, 255, 0, 255]);

        // 2x1 32bit RLE:一个 raw 包(2 像素)
        let mut b = vec![0u8; 18];
        b[2] = 10; // 真彩 RLE
        b[12..14].copy_from_slice(&2u16.to_le_bytes());
        b[14..16].copy_from_slice(&1u16.to_le_bytes());
        b[16] = 32;
        b[17] = 0x20; // 顶左原点(不翻转)
        b.push(0x01); // raw 包 2 像素
        b.extend_from_slice(&[10, 20, 30, 40]);
        b.extend_from_slice(&[50, 60, 70, 80]);
        let t = decode_tga(&b).unwrap();
        assert_eq!(&t.rgba, &[30, 20, 10, 40, 70, 60, 50, 80]);
    }

    /// 蒙皮数学验证:glTF 公式(Σ w'·Mᶠ·M⁰⁻¹·v_bind)对引擎公式(Σ w·Mᶠ·lp)
    /// 的偏差——单影响顶点应逐帧精确,混合顶点统计分布;同时统计权重和 ≠1
    /// 与 >4 影响的数据占比(这两类是 glTF 标准表达力的固有限制)。
    #[test]
    fn skinning_matches_engine() {
        let skel_path = format!("{AG}/act/girl.FSKEL");
        let Ok(bytes) = fs::read(&skel_path) else {
            eprintln!("girl.FSKEL 不存在,跳过");
            return;
        };
        let f = fskel::parse_fskel(&skel_path, &bytes).unwrap();
        let bc = f.bone_count() as usize;
        let w0 = frame_worlds(&f, 0);
        let inv0: Vec<M4> = w0.iter().map(|m| mat_invert(m).unwrap()).collect();

        // 抽查帧:首帧 + 动作中后段几帧
        let mut frames = vec![0usize];
        for k in [
            f.frame_count() / 3,
            f.frame_count() / 2,
            f.frame_count() - 1,
        ] {
            let k = k as usize;
            if k > 0 && !frames.contains(&k) {
                frames.push(k);
            }
        }

        for name in ["girl_head.fskin", "girl_Upper.fskin", "girl_Hair01.fskin"] {
            let m = load_fskin(name);
            let n = m.vertices.len();
            let mut single = n;
            let mut mixed = 0usize;
            let mut nonone_sum = 0usize; // Σw 偏离 1 超过 1% 的顶点
            let mut over4 = 0usize;
            // v_bind(导出绑定位置,归一权重)
            let vbind: Vec<[f32; 3]> = m
                .vertices
                .iter()
                .map(|v| {
                    let wsum: f32 = v
                        .influences
                        .iter()
                        .filter(|i| (i.bone_index as usize) < bc)
                        .map(|i| i.weight)
                        .sum();
                    if (wsum - 1.0).abs() > 0.01 {
                        nonone_sum += 1;
                    }
                    if v.influences.len() > 4 {
                        over4 += 1;
                    }
                    if v.influences.len() == 1 {
                        mixed += 0;
                    } else {
                        single -= 1;
                        mixed += 1;
                    }
                    let mut o = [0f32; 3];
                    for inf in &v.influences {
                        let bi = inf.bone_index as usize;
                        if bi >= bc {
                            continue;
                        }
                        let x = xf(&w0[bi], inf.bone_local_pos);
                        for k in 0..3 {
                            o[k] += inf.weight / wsum * x[k];
                        }
                    }
                    o
                })
                .collect();

            let mut worst_single = 0f32;
            let mut worst_mixed = 0f32;
            let mut sum_mixed = 0f32;
            let mut cnt_mixed = 0usize;
            for &fr in &frames {
                let wt = frame_worlds(&f, fr);
                for (i, v) in m.vertices.iter().enumerate() {
                    let mut engine = [0f32; 3];
                    let mut gltf = [0f32; 3];
                    let wsum: f32 = v
                        .influences
                        .iter()
                        .filter(|i| (i.bone_index as usize) < bc)
                        .map(|i| i.weight)
                        .sum();
                    for inf in &v.influences {
                        let bi = inf.bone_index as usize;
                        if bi >= bc {
                            continue;
                        }
                        let e = xf(&wt[bi], inf.bone_local_pos);
                        for k in 0..3 {
                            engine[k] += inf.weight * e[k];
                        }
                        let g = xf(&wt[bi], xf(&inv0[bi], vbind[i]));
                        for k in 0..3 {
                            gltf[k] += inf.weight / wsum * g[k];
                        }
                    }
                    let d = ((engine[0] - gltf[0]).powi(2)
                        + (engine[1] - gltf[1]).powi(2)
                        + (engine[2] - gltf[2]).powi(2))
                    .sqrt();
                    if v.influences.len() == 1 {
                        worst_single = worst_single.max(d);
                    } else {
                        worst_mixed = worst_mixed.max(d);
                        sum_mixed += d;
                        cnt_mixed += 1;
                    }
                }
            }
            eprintln!(
                "{name}: {n} 顶点(单影响 {single} / 混合 {mixed} / Σw≠1 {nonone_sum} / >4影响 {over4})\n  \
                 单影响最大误差 {worst_single:.6}(期望 <0.01)· 混合最大 {worst_mixed:.4} 平均 {:.4}",
                sum_mixed / cnt_mixed.max(1) as f32
            );
            assert!(
                worst_single < 0.01,
                "{name} 单影响顶点 glTF 公式应与引擎一致(实测 {worst_single})"
            );
        }
    }

    /// 整体导出 → GLB 结构自检:容器、JSON 结构、skin、动画、贴图、帽子挂接
    #[test]
    fn full_glb_structure() {
        let skel = format!("{AG}/act/girl.FSKEL");
        let Ok(_) = fs::read(&skel) else {
            eprintln!("girl.FSKEL 不存在,跳过");
            return;
        };
        // 与前端 resolveTexture 同口径:目录内按材质贴图 stem 匹配
        let dir_tex: Vec<(String, String)> = vfs::walk_files(AG, &["dds", "tga", "png", "jpg"])
            .into_iter()
            .map(|e| {
                (
                    e.name
                        .rsplit_once('.')
                        .map(|(s, _)| s.to_lowercase())
                        .unwrap_or_default(),
                    e.path,
                )
            })
            .collect();
        let resolve = |tex_path: &str| -> Option<String> {
            let stem = tex_path
                .rsplit(['\\', '/'])
                .next()?
                .rsplit_once('.')
                .map(|(s, _)| s.to_lowercase())?;
            dir_tex.iter().find(|(s, _)| *s == stem).map(|(_, p)| p.clone())
        };
        let parts = ["girl_head.fskin", "girl_Hair01.fskin", "girl_Upper.fskin", "girl_Lower.fskin", "girl_Boot.fskin"]
            .iter()
            .map(|n| {
                let path = format!("{AG}/{n}");
                let m = load_fskin(n);
                let textures = m
                    .materials
                    .iter()
                    .map(|md| resolve(&md.texture_path))
                    .collect();
                GlbPart { path, textures }
            })
            .collect::<Vec<_>>();
        let hat = GlbHat {
            path: concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Hat/FM01.FSM").to_string(),
            bone: String::new(),
            offset: [0.0, 1.0, 0.0],
        };
        let out = std::env::temp_dir().join("gongfu_full_glb_test.glb");
        let summary = export_full_glb(&parts, Some(&skel), Some(&hat), out.to_str().unwrap());
        let summary = match summary {
            Ok(s) => s,
            Err(e) => panic!("{e}"),
        };
        eprintln!("{summary}");

        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
        let total = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(total, bytes.len());
        eprintln!("GLB 大小 {:.2} MB", bytes.len() as f64 / 1048576.0);
        let jl = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        assert_eq!(&bytes[16..20], b"JSON");
        let doc: Value = serde_json::from_slice(&bytes[20..20 + jl]).unwrap();

        let meshes = doc["meshes"].as_array().unwrap();
        assert_eq!(meshes.len(), 6, "5 部件 + 帽子");
        for m in meshes {
            for prim in m["primitives"].as_array().unwrap() {
                assert!(prim["material"].is_number(), "primitive 应带材质");
            }
        }
        let skin = &doc["skins"][0];
        let joints = skin["joints"].as_array().unwrap();
        assert_eq!(joints.len(), 135, "girl.FSKEL 骨数");
        // IBM accessor 存在且类型 MAT4
        let ibm = skin["inverseBindMatrices"].as_u64().unwrap() as usize;
        assert_eq!(doc["accessors"][ibm]["type"], "MAT4");
        assert_eq!(
            doc["accessors"][ibm]["count"].as_u64().unwrap() as usize,
            joints.len()
        );
        // 动画存在且通道目标都在骨骼节点范围
        let anims = doc["animations"].as_array().unwrap();
        assert!(!anims.is_empty());
        for a in anims {
            for ch in a["channels"].as_array().unwrap() {
                let n = ch["target"]["node"].as_u64().unwrap();
                assert!(n < 135);
            }
        }
        // 帽子挂接:某骨骼节点 children 里有 HatHolder
        let nodes = doc["nodes"].as_array().unwrap();
        let holder = nodes
            .iter()
            .position(|n| n["name"] == "HatHolder")
            .expect("帽子挂接节点");
        assert!(
            nodes.iter().any(|n| n["children"]
                .as_array()
                .is_some_and(|c| c.contains(&Value::from(holder)))),
            "HatHolder 应挂某骨骼下"
        );
        // 贴图:5 部件各自目录里都有 dds,帽子在 Hat 目录
        let images = doc["images"].as_array().unwrap();
        assert!(images.len() >= 5, "应嵌入贴图(实际 {})", images.len());
        // buffer 覆盖所有 bufferView
        let bin_len = doc["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
        for v in doc["bufferViews"].as_array().unwrap() {
            let off = v["byteOffset"].as_u64().unwrap() as usize;
            let len = v["byteLength"].as_u64().unwrap() as usize;
            assert!(off + len <= bin_len);
        }
        // 引用完整性:场景/children/skin joints/动画目标引用的节点都存在
        let node_count = nodes.len();
        let check_nodes = |vals: &Vec<Value>, what: &str| {
            for n in vals {
                let n = n.as_u64().unwrap() as usize;
                assert!(n < node_count, "{what} 引用了不存在的节点 {n}");
            }
        };
        check_nodes(doc["scenes"][0]["nodes"].as_array().unwrap(), "场景");
        for n in nodes {
            if let Some(c) = n["children"].as_array() {
                check_nodes(c, "children");
            }
        }
        check_nodes(joints, "skin joints");
        // mesh primitive:attributes 各 accessor count 一致,indices 界内
        let accs = doc["accessors"].as_array().unwrap();
        let acc_count = |i: u64| accs[i as usize]["count"].as_u64().unwrap();
        for m in meshes {
            for prim in m["primitives"].as_array().unwrap() {
                let pos = prim["attributes"]["POSITION"].as_u64().unwrap();
                let nrm = prim["attributes"]["NORMAL"].as_u64().unwrap();
                let uv = prim["attributes"]["TEXCOORD_0"].as_u64().unwrap();
                assert_eq!(acc_count(pos), acc_count(nrm));
                assert_eq!(acc_count(pos), acc_count(uv));
                let idx = prim["indices"].as_u64().unwrap();
                assert!(acc_count(idx) % 3 == 0, "索引数应为 3 的倍数");
                assert!(
                    acc_count(idx) > 0 && acc_count(pos) > 0,
                    "三角形与顶点数应为正"
                );
            }
        }
        let _ = fs::remove_file(&out);
    }
}
