//! .fak (AKK v1.0) 顶点动画模型解析与序列化
//!
//! 布局（2026-10 对 res/system/Monster/G080/G080.FAK 全字节逆向验证；
//! 2026-10 对 system.cpk 全量 696 个 FAK 归纳修正）:
//!   0x00 "AKK\0" | 0x04 版本 | 0x08 顶点数 | 0x0C 子网格数
//!   0x10 子网格区偏移(恒0x13C) | 0x14 材质数 | 0x18 材质区偏移
//!   0x1C 帧数 | 0x20 帧区偏移 | 0x24 bbox max/min 各3个f32
//!   0x3C..0x13C 保留256字节
//!   子网格区 ×N: 与 FSM 同款记录 [u16 基地址][u16 序号][u16 ?0][u16 该节
//!                顶点数][bbox 6f32][u32 三角形数][u16 索引×3n][20字节0填充]
//!                序号多为材质序号且可共用（G09BDW022 节8-13 同为 8）；
//!                顶点数为共享池子集，索引为节内局部（加基地址成全局，
//!                bbox 判据：局部误差 0.00 / 全局误差 ~1500）
//!   帧区 ×N: [动作名4B][f32 帧时长][u32 顶点数][顶点记录 32B ×N]
//!            [u32 挂点数][挂点记录 36B ×N]
//!            每帧之后都恰好一个挂点块标记（G080 恒 0 = 无挂点；bs001
//!            每帧 4 个 eft1-4 特效挂点，位置/朝向逐帧动画——顶点动画
//!            无骨架，挂点只能逐帧烘焙）。挂点记录: [名4B][f32×7 前3位置
//!            后4四元数][4B 尾(多 00 CC CC CC)]，与 FSM 挂点同构
//!   材质区 ×N: 等分至文件尾（全量实测恒 0x46C/条，与 FSKIN/FSM 材质槽
//!            同宽），每条: [u32 序号][diffuse/ambient/emissive/specular
//!            各4f32][f32 高光强度][3字节前缀][NUL结尾路径][0填充]
//!   变体：N144 类纯挂点定位模型（顶点数 0、帧数据空、每帧挂点块非 0，
//!         bbox 为 FLT_MAX 空盒）；G036 类损坏样本（区域偏移与计数全 0
//!         但文件含数据）——按空模型解析，残留字节进 tail 原样保留。
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
    /// 记录头 [u16 基地址][u16 序号][u16 ?0][u16 顶点数] 中的序号，
    /// 通常为材质序号；多节可共用（G09BDW022 节8-13 同为 8）
    pub index: u16,
    /// 该节顶点段在共享池中的起始下标（文件内三角形索引的基地址）
    pub base_vertex: u16,
    /// 记录头里语义未定的 u16 0，按原样保留保证往返一致
    pub pad: u16,
    /// 该节顶点数：共享池的子集（单节文件等于全池）
    pub vertex_count: u16,
    pub bbox_max: [f32; 3],
    pub bbox_min: [f32; 3],
    /// 扁平 u16 索引，每 3 个为一个三角形。文件内为节内局部索引，
    /// 解析时已加 baseVertex 转为共享池全局索引；序列化时减回
    pub triangles: Vec<u16>,
    /// 记录尾填充（原样保留）
    pub tail: Vec<u8>,
}

/// 逐帧挂点记录（与 FSM 挂点同构；顶点动画无骨架，挂点逐帧烘焙）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FakDummy {
    /// 定长 4 字节字段，去尾部 NUL
    pub name: String,
    /// 前 3 个为位置，后 4 个为朝向四元数
    pub values: [f32; 7],
    /// 记录尾 4B（多为 00 CC CC CC，按原样保留）
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
    /// 该帧之后的挂点块（可为空）
    pub dummies: Vec<FakDummy>,
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
    /// 各区之后的残留字节（G036 类损坏样本区域计数全 0 但含数据；
    /// 正常文件为空）
    pub tail: Vec<u8>,
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
    if submesh_count > 0 && submesh_off != SUBMESH_OFFSET {
        return Err(format!("异常的子网格区偏移 0x{submesh_off:x}"));
    }
    if vertex_count as usize > u16::MAX as usize {
        return Err(format!("顶点数 {vertex_count} 超 u16 索引范围"));
    }
    // N144 类纯挂点模型子网格区与帧区同偏移；G036 类损坏样本偏移全 0
    if !(submesh_off <= frame_off && frame_off <= material_off && material_off <= bytes.len()) {
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
        let base_vertex = r.u16()?;
        let index = r.u16()?;
        let pad = r.u16()?;
        let vc = r.u16()?;
        if base_vertex as u32 + vc as u32 > vertex_count {
            return Err(format!(
                "子网格 {i} 顶点段 [{base_vertex}, {}) 超出共享池 {vertex_count} @0x{:x}",
                base_vertex as u32 + vc as u32,
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
            let local = r.u16()? as u32;
            let global = base_vertex as u32 + local;
            if global >= vertex_count {
                return Err(format!(
                    "子网格 {i} 局部索引 {local}+{base_vertex} 越界（顶点池 {vertex_count}）"
                ));
            }
            triangles.push(global as u16);
        }
        let tail = r.take(SUBMESH_TAIL)?.to_vec();
        submeshes.push(FakSubmesh {
            index,
            base_vertex,
            pad,
            vertex_count: vc,
            bbox_max,
            bbox_min,
            triangles,
            tail,
        });
    }
    if submesh_count > 0 && r.pos != frame_off {
        return Err(format!(
            "子网格区结束于 0x{:x}，与帧区偏移 0x{frame_off:x} 不吻合（尾部填充长度假设失效）",
            r.pos
        ));
    }

    // ---- 帧区：每帧 [名][时长][顶点数][数据] 后跟 [u32 挂点数][挂点 ×N] ----
    r.pos = frame_off;
    let mut frames = Vec::with_capacity(frame_count);
    for i in 0..frame_count {
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
            // N149 等资产帧数据含 NaN（导出时未初始化内存，引擎照用），
            // 按比特原样保留；走位错误由帧数与区偏移落点检查兜底
            data.push(r.f32()?);
        }
        let dc = r.u32()? as usize;
        if dc > 64 {
            return Err(format!("帧 {i} 挂点数 {dc} 异常 @0x{:x}", r.pos - 4));
        }
        let mut dummies = Vec::with_capacity(dc);
        for _ in 0..dc {
            let name_raw = r.take(4)?;
            let end = name_raw
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_raw.len());
            let name = String::from_utf8_lossy(&name_raw[..end]).into_owned();
            let mut values = [0f32; 7];
            for v in values.iter_mut() {
                *v = r.f32()?;
            }
            let tail = r.take(4)?.to_vec();
            dummies.push(FakDummy { name, values, tail });
        }
        frames.push(FakFrame {
            name,
            frame_dt,
            data,
            dummies,
        });
    }
    if r.pos != material_off {
        return Err(format!(
            "帧区结束于 0x{:x}，与材质区偏移 0x{material_off:x} 不吻合",
            r.pos
        ));
    }

    // ---- 材质区：material_count 条记录等分到文件尾 ----
    let region = bytes.len() - material_off;
    if material_count > 0 && region % material_count != 0 {
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
        let texture_path = crate::fskin::decode_path(&bytes[r.pos..p_end]);
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

    // 区域之后的残留（G036 类损坏样本；正常文件为空）
    let content_end = r.pos.max(SUBMESH_OFFSET);
    let tail = bytes[content_end..].to_vec();

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
        tail,
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
        push_u16(&mut sbuf, s.base_vertex);
        push_u16(&mut sbuf, s.index);
        push_u16(&mut sbuf, s.pad);
        push_u16(&mut sbuf, s.vertex_count);
        for x in s.bbox_max.iter().chain(s.bbox_min.iter()) {
            push_f32(&mut sbuf, *x);
        }
        push_u32(&mut sbuf, (s.triangles.len() / 3) as u32);
        for t in &s.triangles {
            // 全局索引减回基地址还原为节内局部索引
            let local = t.checked_sub(s.base_vertex).ok_or_else(|| {
                format!("索引 {t} 低于子网格基地址 {}，无法还原局部索引", s.base_vertex)
            })?;
            push_u16(&mut sbuf, local);
        }
        sbuf.extend_from_slice(&s.tail);
    }
    let frame_off = SUBMESH_OFFSET + sbuf.len();

    // 帧区：每帧 [名][时长][顶点数][数据][u32 挂点数][挂点记录]
    let mut fbuf = Vec::new();
    for (i, f) in m.frames.iter().enumerate() {
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
        push_u32(&mut fbuf, f.dummies.len() as u32);
        for d in &f.dummies {
            let name = d.name.as_bytes();
            if name.len() > 4 || name.contains(&0u8) {
                return Err(format!("挂点名 {name:?} 超出 4 字节定长字段"));
            }
            let mut field = [0u8; 4];
            field[..name.len()].copy_from_slice(name);
            fbuf.extend_from_slice(&field);
            for x in d.values.iter() {
                push_f32(&mut fbuf, *x);
            }
            if d.tail.len() != 4 {
                return Err("挂点尾部填充长度异常".into());
            }
            fbuf.extend_from_slice(&d.tail);
        }
    }
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
        let path_bytes = crate::fskin::encode_path(&mat.texture_path);
        if path_bytes.contains(&0u8) {
            return Err("纹理路径含非法字符".into());
        }
        mbuf.extend_from_slice(&path_bytes);
        mbuf.push(0);
        mbuf.extend_from_slice(&mat.tail);
    }

    // G036 类损坏样本：区域全空但带残留 → 保留原始全零偏移
    let degenerate = m.submeshes.is_empty()
        && m.frames.is_empty()
        && m.materials.is_empty()
        && !m.tail.is_empty();

    // 头部
    let mut out = Vec::with_capacity(material_off + mbuf.len() + m.tail.len());
    out.extend_from_slice(b"AKK\0");
    push_u32(&mut out, m.version);
    push_u32(&mut out, m.vertex_count);
    push_u32(&mut out, m.submeshes.len() as u32);
    push_u32(
        &mut out,
        if degenerate { 0 } else { SUBMESH_OFFSET as u32 },
    );
    push_u32(&mut out, m.materials.len() as u32);
    push_u32(&mut out, if degenerate { 0 } else { material_off as u32 });
    push_u32(&mut out, m.frames.len() as u32);
    push_u32(&mut out, if degenerate { 0 } else { frame_off as u32 });
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
    out.extend_from_slice(&m.tail);
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

    /// 临时全量回归已通过（CPK 695 个 FAK 全部解析+字节往返，2026-10），
    /// 以下为固定样本回归：分段顶点池 / 逐帧动画挂点 / 纯挂点定位模型。
    #[test]
    fn segmented_and_dummy_variants() {
        let base_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system");
        // (相对路径, 顶点数, 子网格数, 材质数, 帧数)
        let cases = [
            ("Building/g09/G09DW17.FAK", 8u32, 2usize, 1usize, 55usize),
            ("NPC/N128.FAK", 472, 1, 1, 9),
            ("NPC/N144.FAK", 0, 0, 0, 9),
        ];
        for (rel, vc, sc, mc, fc) in cases {
            let p = format!("{base_dir}/{rel}");
            let Ok(bytes) = fs::read(&p) else {
                eprintln!("跳过: {p} 不存在");
                continue;
            };
            let m = parse_fak(rel, &bytes).unwrap();
            assert_eq!(m.vertex_count, vc, "{rel}");
            assert_eq!(m.submeshes.len(), sc, "{rel}");
            assert_eq!(m.materials.len(), mc, "{rel}");
            assert_eq!(m.frames.len(), fc, "{rel}");
            let out = serialize_fak(&m).unwrap();
            assert_eq!(bytes, out, "{rel} 往返不一致");
        }
        // G09DW17：8 顶点池拆成两节 [0,4)+[4,8)，索引为节内局部
        let p = format!("{base_dir}/Building/g09/G09DW17.FAK");
        if let Ok(bytes) = fs::read(&p) {
            let m = parse_fak("G09DW17", &bytes).unwrap();
            assert_eq!(
                (m.submeshes[0].base_vertex, m.submeshes[0].vertex_count),
                (0, 4)
            );
            assert_eq!(
                (m.submeshes[1].base_vertex, m.submeshes[1].vertex_count),
                (4, 4)
            );
            // 全局索引应落在各自顶点段内
            assert!(m.submeshes[0].triangles.iter().all(|&t| t < 4));
            assert!(m.submeshes[1].triangles.iter().all(|&t| (4..8).contains(&t)));
        }
        // N128：每帧挂点 ride 的位置逐帧变化（顶点动画无骨架，挂点逐帧烘焙）
        let p = format!("{base_dir}/NPC/N128.FAK");
        if let Ok(bytes) = fs::read(&p) {
            let m = parse_fak("N128", &bytes).unwrap();
            let y0 = m.frames[0].dummies[0].values[1];
            let y1 = m.frames[1].dummies[0].values[1];
            assert_eq!(m.frames[0].dummies[0].name, "ride");
            assert!((y0 - 249.4).abs() < 0.1 && (y1 - 254.1).abs() < 0.1);
            assert!((y1 - y0).abs() > 1.0, "挂点位置应逐帧变化");
        }
        // N144：0 顶点纯挂点定位模型，每帧 2 个挂点（ride + eft1）
        let p = format!("{base_dir}/NPC/N144.FAK");
        if let Ok(bytes) = fs::read(&p) {
            let m = parse_fak("N144", &bytes).unwrap();
            assert!(m.frames.iter().all(|f| f.data.is_empty()));
            assert!(m
                .frames
                .iter()
                .all(|f| f.dummies.len() == 2
                    && f.dummies[0].name == "ride"
                    && f.dummies[1].name == "eft1"));
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
