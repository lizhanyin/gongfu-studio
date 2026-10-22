//! .fak (AKK v1.0) 顶点动画模型解析与序列化
//!
//! 布局（2026-10 对 res/system/Monster/G080/G080.FAK 全字节逆向验证）:
//!   0x00 "AKK\0" | 0x04 版本 | 0x08 顶点数 | 0x0C 子网格数
//!   0x10 子网格区偏移(恒0x13C) | 0x14 材质数 | 0x18 材质区偏移
//!   0x1C 帧数 | 0x20 帧区偏移 | 0x24 bbox max/min 各3个f32
//!   0x3C..0x13C 保留256字节
//!   子网格区 ×N: [u16 0][u16 序号][u16 0][u16 顶点数][bbox 6f32][u32 三角形数]
//!                [u16 索引×3n][20字节0填充]
//!   帧区 ×N: 首帧直接开始，其后每帧前有 [u32 0] 分隔符，区末 [u32 0] 终止:
//!            [动作名4B][f32 帧时长][u32 顶点数][顶点记录 32B ×N]
//!            顶点记录: [pos3][normal3][uv2]，法线未归一化（长度2.1~3.0）
//!   材质区 ×N: 等分至文件尾，每条: [u32 序号][diffuse/ambient/emissive/specular
//!            各4f32][f32 高光强度][3字节前缀][NUL结尾路径][0填充]
//!
//! 帧区为逐帧完整顶点快照（vertex morph，无骨骼）。同名连续帧合并为动作
//! （G080: atck20/spck28/bek7/std20/walk12/dead46 = 133 帧）。UV 经全部帧
//! 验证恒定且无跨缝三角形，渲染端可作静态属性。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 头部(0x3C) + 保留256字节之后的子网格区固定偏移
pub const SUBMESH_OFFSET: usize = 0x13C;
/// 子网格记录索引区之后的固定 0 填充长度
const SUBMESH_TAIL: usize = 20;
const HEADER_LEN: usize = 0x3C;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakSubmesh {
    /// 记录头 [u16 0][u16 序号] 中的序号
    pub index: u16,
    /// 记录头里语义未定的两个 u16 0，按原样保留保证往返一致
    pub pad: [u16; 2],
    pub bbox_max: [f32; 3],
    pub bbox_min: [f32; 3],
    /// 扁平 u16 索引，每 3 个为一个三角形，引用共享顶点池
    pub triangles: Vec<u16>,
    /// 记录尾填充（原样保留）
    pub tail: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakFrame {
    /// 动作名（定长 4 字节字段，去尾部 NUL）
    pub name: String,
    /// 该帧的持续时长（秒）；dead 为 1/30，其余多为 0.125
    pub frame_dt: f32,
    /// 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2]
    pub data: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakMaterial {
    /// 材质序号（fskin 里同位置字段被当作 f32，此处确认为整数索引）
    pub index: u32,
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub emissive: [f32; 4],
    pub specular: [f32; 4],
    pub spec_power: f32,
    pub pre: [u8; 3],
    pub texture_path: String,
    pub tail: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakAction {
    pub name: String,
    pub first_frame: u32,
    pub frame_count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakModel {
    pub path: String,
    pub version: u32,
    pub vertex_count: u32,
    pub bbox_max: [f32; 3],
    pub bbox_min: [f32; 3],
    pub reserved256: Vec<u8>,
    pub submeshes: Vec<FakSubmesh>,
    pub frames: Vec<FakFrame>,
    pub materials: Vec<FakMaterial>,
    /// 同名连续帧合并的动作分段（解析时生成，序列化时重算）
    pub actions: Vec<FakAction>,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }
    fn u32(&mut self) -> Result<u32, String> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u16(&mut self) -> Result<u16, String> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.b.len() {
            return Err(format!("文件越界: 需要 {} 字节 @0x{:x}", n, self.pos));
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
}

/// 同名连续帧合并为动作分段
fn split_actions(frames: &[FakFrame]) -> Vec<FakAction> {
    let mut out: Vec<FakAction> = Vec::new();
    for (i, f) in frames.iter().enumerate() {
        match out.last_mut() {
            Some(a) if a.name == f.name => a.frame_count += 1,
            _ => out.push(FakAction {
                name: f.name.clone(),
                first_frame: i as u32,
                frame_count: 1,
            }),
        }
    }
    out
}

pub fn parse_fak(path: &str, bytes: &[u8]) -> Result<FakModel, String> {
    if bytes.len() < HEADER_LEN {
        return Err("文件太小".into());
    }
    if &bytes[0..4] != b"AKK\0" {
        return Err("魔数不是 AKK".into());
    }
    let mut rh = Reader::new(bytes);
    rh.pos = 4;
    let version = rh.u32()?;
    let vertex_count = rh.u32()?;
    let submesh_count = rh.u32()? as usize;
    let submesh_off = rh.u32()? as usize;
    let material_count = rh.u32()? as usize;
    let material_off = rh.u32()? as usize;
    let frame_count = rh.u32()? as usize;
    let frame_off = rh.u32()? as usize;
    let mut bbox_max = [0f32; 3];
    let mut bbox_min = [0f32; 3];
    for v in &mut bbox_max {
        *v = rh.f32()?;
    }
    for v in &mut bbox_min {
        *v = rh.f32()?;
    }
    if submesh_off != SUBMESH_OFFSET {
        return Err(format!("异常的子网格区偏移 0x{submesh_off:x}"));
    }
    if vertex_count as usize > u16::MAX as usize {
        return Err(format!("顶点数 {vertex_count} 超 u16 索引范围"));
    }
    if !(submesh_off < frame_off && frame_off <= material_off && material_off <= bytes.len()) {
        return Err(format!(
            "区偏移异常: 子网格 0x{submesh_off:x} 帧 0x{frame_off:x} 材质 0x{material_off:x}"
        ));
    }
    let reserved256 = bytes[HEADER_LEN..SUBMESH_OFFSET].to_vec();

    // ---- 子网格区 ----
    let mut r = Reader::new(bytes);
    r.pos = submesh_off;
    let mut submeshes = Vec::with_capacity(submesh_count);
    for i in 0..submesh_count {
        let pad_a = r.u16()?;
        let index = r.u16()?;
        let pad_b = r.u16()?;
        let vc = r.u16()?;
        if index != i as u16 {
            return Err(format!("子网格序号跳变: 期望 {i} 实际 {index} @0x{:x}", r.pos - 6));
        }
        if vc as u32 != vertex_count {
            return Err(format!(
                "子网格顶点数 {vc} 与头部 {vertex_count} 不符 @0x{:x}",
                r.pos - 2
            ));
        }
        let mut bbox_max = [0f32; 3];
        let mut bbox_min = [0f32; 3];
        for v in &mut bbox_max {
            *v = r.f32()?;
        }
        for v in &mut bbox_min {
            *v = r.f32()?;
        }
        let tri_count = r.u32()? as usize;
        let mut triangles = Vec::with_capacity(tri_count * 3);
        for _ in 0..tri_count * 3 {
            let idx = r.u16()?;
            if idx as u32 >= vertex_count {
                return Err(format!("子网格 {i} 索引 {idx} 越界（顶点数 {vertex_count}）"));
            }
            triangles.push(idx);
        }
        let tail = r.take(SUBMESH_TAIL)?.to_vec();
        submeshes.push(FakSubmesh {
            index,
            pad: [pad_a, pad_b],
            bbox_max,
            bbox_min,
            triangles,
            tail,
        });
    }
    if r.pos != frame_off {
        return Err(format!(
            "子网格区结束于 0x{:x}，与帧区偏移 0x{frame_off:x} 不吻合（尾部填充长度假设失效）",
            r.pos
        ));
    }

    // ---- 帧区 ----
    let mut frames = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
        if i > 0 {
            let sep = r.u32()?;
            if sep != 0 {
                return Err(format!("帧分隔符非 0: {sep} @0x{:x}", r.pos - 4));
            }
        }
        let name_raw = r.take(4)?;
        let end = name_raw.iter().position(|&b| b == 0).unwrap_or(name_raw.len());
        let name = String::from_utf8_lossy(&name_raw[..end]).into_owned();
        let frame_dt = r.f32()?;
        if !frame_dt.is_finite() || frame_dt <= 0.0 {
            return Err(format!("帧时长异常 {frame_dt}（帧 {i}）"));
        }
        let vc = r.u32()?;
        if vc != vertex_count {
            return Err(format!("帧 {i} 顶点数 {vc} 与头部 {vertex_count} 不符"));
        }
        let n = vc as usize * 8;
        let mut data = Vec::with_capacity(n);
        for _ in 0..n {
            let v = r.f32()?;
            if !v.is_finite() {
                return Err(format!("帧 {i} 含非有限浮点 @0x{:x}", r.pos - 4));
            }
            data.push(v);
        }
        frames.push(FakFrame {
            name,
            frame_dt,
            data,
        });
    }
    let term = r.u32()?;
    if term != 0 {
        return Err(format!("帧区终止符非 0: {term}"));
    }
    if r.pos != material_off {
        return Err(format!(
            "帧区结束于 0x{:x}，与材质区偏移 0x{material_off:x} 不吻合",
            r.pos
        ));
    }

    // ---- 材质区：material_count 条记录等分到文件尾 ----
    let region = bytes.len() - material_off;
    if material_count == 0 {
        if region != 0 {
            return Err("材质数为 0 但材质区非空".into());
        }
    } else if region % material_count != 0 {
        return Err(format!(
            "材质区 {region} 字节不能被 {material_count} 条记录整除"
        ));
    }
    let rec = if material_count == 0 {
        0
    } else {
        region / material_count
    };
    let mut materials = Vec::with_capacity(material_count);
    for i in 0..material_count {
        let rec_end = material_off + (i + 1) * rec;
        let index = r.u32()?;
        let read4 = |r: &mut Reader| -> Result<[f32; 4], String> {
            let mut a = [0f32; 4];
            for v in a.iter_mut() {
                *v = r.f32()?;
            }
            Ok(a)
        };
        let diffuse = read4(&mut r)?;
        let ambient = read4(&mut r)?;
        let emissive = read4(&mut r)?;
        let specular = read4(&mut r)?;
        let spec_power = r.f32()?;
        let pre: [u8; 3] = [r.take(1)?[0], r.take(1)?[0], r.take(1)?[0]];
        let p_end = bytes[r.pos..rec_end]
            .iter()
            .position(|&c| c == 0)
            .map(|p| r.pos + p)
            .ok_or("纹理路径未终止")?;
        let texture_path = String::from_utf8_lossy(&bytes[r.pos..p_end]).into_owned();
        r.pos = p_end + 1;
        let tail = bytes[r.pos..rec_end].to_vec();
        // 对齐到记录末尾（等长记录的固定填充）
        r.pos = rec_end;
        materials.push(FakMaterial {
            index,
            diffuse,
            ambient,
            emissive,
            specular,
            spec_power,
            pre,
            texture_path,
            tail,
        });
    }

    let actions = split_actions(&frames);
    Ok(FakModel {
        path: path.to_string(),
        version,
        vertex_count,
        bbox_max,
        bbox_min,
        reserved256,
        submeshes,
        frames,
        materials,
        actions,
    })
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn push_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

/// 重新序列化。未修改的模型可字节级还原原文件（见 roundtrip 测试）。
pub fn serialize_fak(m: &FakModel) -> Result<Vec<u8>, String> {
    if m.reserved256.len() != SUBMESH_OFFSET - HEADER_LEN {
        return Err("保留区长度异常".into());
    }
    // 子网格区
    let mut sbuf = Vec::new();
    for s in &m.submeshes {
        if s.triangles.len() % 3 != 0 {
            return Err("三角形索引数不是 3 的倍数".into());
        }
        if s.tail.len() != SUBMESH_TAIL {
            return Err("子网格尾部填充长度异常".into());
        }
        push_u16(&mut sbuf, s.pad[0]);
        push_u16(&mut sbuf, s.index);
        push_u16(&mut sbuf, s.pad[1]);
        push_u16(&mut sbuf, m.vertex_count.min(u16::MAX as u32) as u16);
        for x in s.bbox_max.iter().chain(s.bbox_min.iter()) {
            push_f32(&mut sbuf, *x);
        }
        push_u32(&mut sbuf, (s.triangles.len() / 3) as u32);
        for t in &s.triangles {
            push_u16(&mut sbuf, *t);
        }
        sbuf.extend_from_slice(&s.tail);
    }
    let frame_off = SUBMESH_OFFSET + sbuf.len();

    // 帧区
    let mut fbuf = Vec::new();
    for (i, f) in m.frames.iter().enumerate() {
        if i > 0 {
            push_u32(&mut fbuf, 0);
        }
        let name = f.name.as_bytes();
        if name.len() > 4 || name.contains(&0u8) {
            return Err(format!("帧名 {name:?} 超出 4 字节定长字段"));
        }
        let mut field = [0u8; 4];
        field[..name.len()].copy_from_slice(name);
        fbuf.extend_from_slice(&field);
        push_f32(&mut fbuf, f.frame_dt);
        push_u32(&mut fbuf, m.vertex_count);
        if f.data.len() != m.vertex_count as usize * 8 {
            return Err(format!("帧 {i} 顶点数据长度异常"));
        }
        for v in &f.data {
            push_f32(&mut fbuf, *v);
        }
    }
    push_u32(&mut fbuf, 0);
    let material_off = frame_off + fbuf.len();

    // 材质区
    let mut mbuf = Vec::new();
    for mat in &m.materials {
        push_u32(&mut mbuf, mat.index);
        for arr in [mat.diffuse, mat.ambient, mat.emissive, mat.specular] {
            for x in arr {
                push_f32(&mut mbuf, x);
            }
        }
        push_f32(&mut mbuf, mat.spec_power);
        mbuf.extend_from_slice(&mat.pre);
        let path_bytes = mat.texture_path.as_bytes();
        if path_bytes.contains(&0u8) {
            return Err("纹理路径含非法字符".into());
        }
        mbuf.extend_from_slice(path_bytes);
        mbuf.push(0);
        mbuf.extend_from_slice(&mat.tail);
    }

    // 头部
    let mut out = Vec::with_capacity(material_off + mbuf.len());
    out.extend_from_slice(b"AKK\0");
    push_u32(&mut out, m.version);
    push_u32(&mut out, m.vertex_count);
    push_u32(&mut out, m.submeshes.len() as u32);
    push_u32(&mut out, SUBMESH_OFFSET as u32);
    push_u32(&mut out, m.materials.len() as u32);
    push_u32(&mut out, material_off as u32);
    push_u32(&mut out, m.frames.len() as u32);
    push_u32(&mut out, frame_off as u32);
    for x in m.bbox_max.iter() {
        push_f32(&mut out, *x);
    }
    for x in m.bbox_min.iter() {
        push_f32(&mut out, *x);
    }
    out.extend_from_slice(&m.reserved256);
    debug_assert_eq!(out.len(), SUBMESH_OFFSET);
    out.extend_from_slice(&sbuf);
    out.extend_from_slice(&fbuf);
    out.extend_from_slice(&mbuf);
    Ok(out)
}

/// 目录列举用的轻量统计：(顶点数, 三角形数, 帧数)。
/// 只读头部 + 子网格区（跳过索引数组），不解析帧数据。
/// 泛型 Reader：磁盘 File 与 CPK 内存读取器共用。
pub fn light_meta<R: std::io::Read>(r: &mut R) -> Option<(u32, u32, u32)> {
    let mut header = [0u8; HEADER_LEN];
    r.read_exact(&mut header).ok()?;
    if &header[0..4] != b"AKK\0" {
        return None;
    }
    let rd = |o: usize| u32::from_le_bytes([header[o], header[o + 1], header[o + 2], header[o + 3]]);
    let vertex_count = rd(0x08);
    let submesh_count = rd(0x0C) as usize;
    let frame_count = rd(0x1C);
    let frame_off = rd(0x20) as usize;
    if rd(0x10) as usize != SUBMESH_OFFSET || frame_off <= SUBMESH_OFFSET || frame_off > 1 << 24 {
        return None;
    }
    // 游标已在 0x3C，补读到 frame_off；buf 下标 = 绝对偏移 - HEADER_LEN
    let mut buf = vec![0u8; frame_off - HEADER_LEN];
    if r.read_exact(&mut buf).is_err() {
        return None;
    }
    let u16at = |o: usize| u16::from_le_bytes([buf[o - HEADER_LEN], buf[o + 1 - HEADER_LEN]]);
    let u32at = |o: usize| {
        u32::from_le_bytes([
            buf[o - HEADER_LEN],
            buf[o + 1 - HEADER_LEN],
            buf[o + 2 - HEADER_LEN],
            buf[o + 3 - HEADER_LEN],
        ])
    };
    // 逐记录走子网格区：8B 头 + 24B bbox + 4B 三角形数 + 索引 + 20B 尾
    let mut pos = SUBMESH_OFFSET;
    let mut tris = 0u32;
    for i in 0..submesh_count {
        if pos + 36 > frame_off {
            return None;
        }
        if u16at(pos + 2) != i as u16 || u16at(pos + 6) as u32 != vertex_count {
            return None;
        }
        let tc = u32at(pos + 32);
        if tc > 1 << 24 {
            return None;
        }
        tris += tc;
        pos += 36 + tc as usize * 6 + SUBMESH_TAIL;
    }
    if pos != frame_off {
        return None;
    }
    Some((vertex_count, tris, frame_count))
}

/// 磁盘文件版：只读文件头与子网格区，不整读帧数据（gui 内部走 vfs，供测试与外部调用）
#[cfg_attr(not(test), allow(dead_code))]
pub fn light_meta_from_file(path: &Path) -> Option<(u32, u32, u32)> {
    let mut f = fs::File::open(path).ok()?;
    light_meta(&mut f)
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // 项目 res 内的实测样本
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Monster/G080");
    const FILE: &str = "G080.FAK";

    fn read_sample() -> Option<Vec<u8>> {
        let p = format!("{ROOT}/{FILE}");
        fs::read(&p).ok()
    }

    /// 临时诊断：直接解析用户文件
    #[test]
    fn tmp_parse_user_fak() {
        let bytes = fs::read(r"E:\Workspace\res\bs001\bs001.fak").unwrap();
        match parse_fak("bs001.fak", &bytes) {
            Ok(m) => {
                eprintln!(
                    "OK: v={} verts={} submeshes={} frames={} materials={} actions={:?}",
                    m.version,
                    m.vertex_count,
                    m.submeshes.len(),
                    m.frames.len(),
                    m.materials.len(),
                    m.actions
                        .iter()
                        .map(|a| format!("{}×{}", a.name, a.frame_count))
                        .collect::<Vec<_>>()
                );
                let out = serialize_fak(&m).unwrap();
                eprintln!("roundtrip byte-equal: {}", bytes == out);
            }
            Err(e) => panic!("解析失败: {e}"),
        }
    }

    #[test]
    fn roundtrip_g080() {
        let Some(bytes) = read_sample() else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        let m = parse_fak(FILE, &bytes).expect("解析 G080.FAK");
        let out = serialize_fak(&m).unwrap();
        let diff = bytes
            .iter()
            .zip(out.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(bytes.len().min(out.len()));
        assert!(
            bytes.len() == out.len() && diff == bytes.len(),
            "往返不一致: 首差异 @0x{diff:x} (len {} vs {})",
            bytes.len(),
            out.len()
        );
    }

    #[test]
    fn parse_g080_structure() {
        let Some(bytes) = read_sample() else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        let m = parse_fak(FILE, &bytes).unwrap();
        assert_eq!(m.version, 0);
        assert_eq!(m.vertex_count, 821);
        assert_eq!(
            m.submeshes.iter().map(|s| s.triangles.len() / 3).collect::<Vec<_>>(),
            vec![56, 24, 181, 664]
        );
        assert_eq!(m.submeshes.iter().map(|s| s.index).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        // 四个子网格的索引构成 821 顶点的不相交划分
        let uni: BTreeSet<u16> = m
            .submeshes
            .iter()
            .flat_map(|s| s.triangles.iter().copied())
            .collect();
        assert_eq!(uni.len(), 821);

        assert_eq!(m.frames.len(), 133);
        let acts: Vec<(&str, u32, u32)> = m
            .actions
            .iter()
            .map(|a| (a.name.as_str(), a.first_frame, a.frame_count))
            .collect();
        assert_eq!(
            acts,
            vec![
                ("atck", 0, 20),
                ("spck", 20, 28),
                ("bek", 48, 7),
                ("std", 55, 20),
                ("walk", 75, 12),
                ("dead", 87, 46),
            ]
        );
        // 帧数据长度与首顶点采样（位精确，逆向时由 hex 直接读出）
        assert_eq!(m.frames[0].data.len(), 821 * 8);
        assert_eq!(m.frames[0].data[0].to_bits(), 0xc2f9d0e2);
        assert_eq!(m.frames[0].data[1].to_bits(), 0x4386d759);
        assert_eq!(m.frames[0].data[2].to_bits(), 0xc14ff44c);
        assert!((m.frames[0].data[6] - 0.0902).abs() < 0.001); // uv.u
        assert!((m.frames[0].data[7] - 0.4402).abs() < 0.001); // uv.v
        assert!((m.frames[0].frame_dt - 0.125).abs() < 1e-6);
        assert!((m.frames[87].frame_dt - 1.0 / 30.0).abs() < 1e-6); // dead 首帧
        // 头部包围盒与第 0 帧顶点一致（另行全帧验证于 python 逆向）
        assert!((m.bbox_max[0] - 283.24).abs() < 0.01);
        assert!((m.bbox_min[1] - 72.11).abs() < 0.01);

        assert_eq!(m.materials.len(), 4);
        let paths: Vec<&str> = m.materials.iter().map(|x| x.texture_path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                r"system\monster\g080\G080+.tga",
                r"system\monster\g080\G080+.tga",
                r"system\monster\g080\G080.tga",
                r"system\monster\g080\G080.tga",
            ]
        );
        assert_eq!(
            m.materials.iter().map(|x| x.index).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert!((m.materials[0].spec_power - 12.8).abs() < 0.01);
    }

    #[test]
    fn light_meta_matches_full_parse() {
        let meta = light_meta_from_file(Path::new(&format!("{ROOT}/{FILE}")));
        let Some(meta) = meta else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        assert_eq!(meta, (821, 925, 133));
    }
}
