//! .fscn (SCN v1) 场景静态网格解析与序列化
//!
//! 布局（2026-10 对 system.cpk 全量 1454 个 .FSCN 逆向验证，解析 0 失败）:
//!   0x00 "SCN\0" | 0x04 版本(恒1) | 0x08 ?(恒1) | 0x0C 顶点数 | 0x10 顶点区偏移(恒0x148)
//!   0x14 子网格数 | 0x18 子网格区偏移 | 0x1C 材质数 | 0x20 材质区偏移
//!   0x24 挂点数 | 0x28 挂点区偏移 | 0x2C bbox min/max 各3个f32
//!   0x44..0x144 保留256字节0 | 0x144 u32 BVH段偏移 = 挂点区偏移+4+挂点数×36
//!   （FSM 到此为止，顶点从 0x144 起；SCN 多出这个树偏移字段，顶点后移到 0x148）
//!   顶点区 ×N×32B: [pos3][normal3][uv2]，法线未归一化——与 FSM 同款；
//!                无贴图顶点 UV 原生写 0xFFFFFFFF/NaN（G06DW07-A 有 88 个）
//!   子网格区 ×N: 与 FSM/FAK 完全同款变长记录（61/1454 样本 base≠0 分段池、
//!                47 样本序号≠位置共用材质，语义同 FSM 多节武器）
//!   材质区 ×N: 等分至挂点区（全量实测恒 0x46C/条，与 FSM/FSKIN 材质槽同宽）
//!   挂点区: [u32 数量][记录×36B]。全量 1454 样本数量恒 0，仅剩数量字段
//!   —— SCN 特有尾部: 包围体层次（BVH），从 0x144 指向的段偏移开始 ——
//!   [u32 记录总数][记录×N]，记录判型: 第 3 个 u32 为 0 是内部节点，否则是叶子
//!     内部节点 36B: [u32 左子][u32 右子][u32 0][bbox max3][bbox min3]
//!       左右子为记录数组全局下标，0xFFFFFFFF 表示空侧（289/1454 样本出现）
//!     叶子: [u32 -1][u32 -1][u32 三角形数][u16 顶点索引×3n][bbox 6f32]
//!       索引为顶点池全局下标（无子网格基地址叠加）
//!   根为记录 0；满二叉时全部内部节点在前叶子在后（G01DW01: 262+263=525），
//!   全部叶子三角形数之和 == 子网格三角形总数（全量样本逐字节核对）
//!
//! 与 FSM 的关系：SCN = FSM 头部多一个 BVH 段偏移字段 + 尾部多一棵 BVH，
//! 顶点/子网格/材质/挂点四区逐字节同构；BVH 疑似用于场景剔除/拾取，语义待验证。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 头部(0x44) + 保留256字节 + BVH段偏移(4B) 之后的顶点区固定偏移
pub const VERTEX_OFFSET: usize = 0x148;
/// BVH 段偏移字段在文件中的位置（保留区末尾）
const TREE_OFFSET_FIELD: usize = 0x144;
/// 子网格记录索引区之后的固定 0 填充长度（与 FSM/FAK 一致）
const SUBMESH_TAIL: usize = 20;
/// 挂点记录定长（FSM 同款：名 4B + f32×7 + 尾 4B）
const DUMMY_RECORD: usize = 36;
const HEADER_LEN: usize = 0x44;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FscnSubmesh {
    /// 记录头 [u16 基地址][u16 序号][u16 ?][u16 顶点数] 中的序号，
    /// 通常逐节递增；47/1454 样本多节共用同一材质序号
    pub index: u16,
    /// 该节顶点段在共享池中的起始下标（文件内三角形索引的基地址）
    pub base_vertex: u16,
    /// 记录头里语义未定的 u16（9/1454 样本非 0），按原样保留保证往返一致
    pub pad: u16,
    /// 该节顶点数：共享池的子集（61/1454 样本分段池，其余等于全池）
    pub vertex_count: u16,
    pub bbox_max: [f32; 3],
    pub bbox_min: [f32; 3],
    /// 扁平 u16 索引，每 3 个为一个三角形。文件内为节内局部索引，
    /// 解析时已加 baseVertex 转为共享池全局索引；序列化时减回
    pub triangles: Vec<u16>,
    /// 记录尾填充（原样保留）
    pub tail: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FscnMaterial {
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

/// 挂点记录（结构与 FSM 同款；全量样本挂点数恒 0，仅作格式完备性保留）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FscnDummy {
    /// 定长 4 字节字段，去尾部 NUL
    pub name: String,
    /// 前 3 个为位置，后 4 个疑似四元数
    pub values: [f32; 7],
    /// 记录尾填充（原样保留）
    pub tail: Vec<u8>,
}

/// BVH 记录。判型依据第 3 个 u32：0 为内部节点，>0 为叶子三角形数；
/// 叶子前两个 u32 恒 0xFFFFFFFF（全量样本验证）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FscnTreeRecord {
    /// 内部节点：左右子为记录数组全局下标，0xFFFFFFFF 表示空侧
    Node {
        left: u32,
        right: u32,
        bbox_max: [f32; 3],
        bbox_min: [f32; 3],
    },
    /// 叶子：扁平 u16 顶点池全局索引，每 3 个为一个三角形
    Leaf {
        triangles: Vec<u16>,
        bbox_max: [f32; 3],
        bbox_min: [f32; 3],
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FscnModel {
    pub path: String,
    pub version: u32,
    /// 头部 0x08 的 u32（全量恒 1），语义未定，按原样保留
    pub unknown: u32,
    pub vertex_count: u32,
    /// 头部包围盒：min 在前 max 在后（子网格记录相反）
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub reserved256: Vec<u8>,
    /// 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2]
    pub vertices: Vec<f32>,
    pub submeshes: Vec<FscnSubmesh>,
    pub materials: Vec<FscnMaterial>,
    pub dummies: Vec<FscnDummy>,
    /// 尾部 BVH：根为记录 0
    pub tree: Vec<FscnTreeRecord>,
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

pub fn parse_fscn(path: &str, bytes: &[u8]) -> Result<FscnModel, String> {
    if bytes.len() < HEADER_LEN {
        return Err("文件太小".into());
    }
    if &bytes[0..4] != b"SCN\0" {
        return Err("魔数不是 SCN".into());
    }
    let mut rh = Reader::new(bytes);
    rh.pos = 4;
    let version = rh.u32()?;
    let unknown = rh.u32()?;
    let vertex_count = rh.u32()?;
    let vertex_off = rh.u32()? as usize;
    let submesh_count = rh.u32()? as usize;
    let submesh_off = rh.u32()? as usize;
    let material_count = rh.u32()? as usize;
    let material_off = rh.u32()? as usize;
    let dummy_count = rh.u32()? as usize;
    let dummy_off = rh.u32()? as usize;
    let mut bbox_min = [0f32; 3];
    let mut bbox_max = [0f32; 3];
    for v in &mut bbox_min {
        *v = rh.f32()?;
    }
    for v in &mut bbox_max {
        *v = rh.f32()?;
    }
    if vertex_off != VERTEX_OFFSET {
        return Err(format!("异常的顶点区偏移 0x{vertex_off:x}"));
    }
    if vertex_count as usize > u16::MAX as usize {
        return Err(format!("顶点数 {vertex_count} 超 u16 索引范围"));
    }
    if !(VERTEX_OFFSET < submesh_off
        && submesh_off <= material_off
        && material_off <= dummy_off
        && dummy_off + 4 <= bytes.len())
    {
        return Err(format!(
            "区偏移异常: 子网格 0x{submesh_off:x} 材质 0x{material_off:x} 挂点 0x{dummy_off:x}"
        ));
    }
    let reserved256 = bytes[HEADER_LEN..TREE_OFFSET_FIELD].to_vec();
    let tree_off = u32::from_le_bytes([
        bytes[TREE_OFFSET_FIELD],
        bytes[TREE_OFFSET_FIELD + 1],
        bytes[TREE_OFFSET_FIELD + 2],
        bytes[TREE_OFFSET_FIELD + 3],
    ]) as usize;
    let tree_expect = dummy_off + 4 + dummy_count * DUMMY_RECORD;
    if tree_off != tree_expect {
        return Err(format!(
            "BVH段偏移 0x{tree_off:x} 与挂点区推算 0x{tree_expect:x} 不符"
        ));
    }

    // ---- 顶点区（与 FSM 同款 [pos3][normal3][uv2]）----
    let vlen = submesh_off - VERTEX_OFFSET;
    if vlen % 32 != 0 || vlen / 32 != vertex_count as usize {
        return Err(format!(
            "顶点区 {vlen} 字节与头部顶点数 {vertex_count} 不符（每顶点 32B）"
        ));
    }
    let mut r = Reader::new(bytes);
    r.pos = VERTEX_OFFSET;
    let mut vertices = Vec::with_capacity(vertex_count as usize * 8);
    for i in 0..vertex_count as usize * 8 {
        let v = r.f32()?;
        // 位置/法线（每顶点前 6 个 f32）要求有限值；UV 允许 NaN——
        // G06DW07-A 有 88 个顶点 UV 原生写 0xFFFFFFFF（无贴图顶点），非损坏
        if i % 8 < 6 && !v.is_finite() {
            return Err(format!("顶点含非有限浮点 @0x{:x}", r.pos - 4));
        }
        vertices.push(v);
    }

    // ---- 子网格区：与 FSM/FAK 同款记录，索引为节内局部（加基地址成全局）----
    let mut submeshes = Vec::with_capacity(submesh_count);
    let mut mesh_tris = 0u64;
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
        mesh_tris += tri_count as u64;
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
        submeshes.push(FscnSubmesh {
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
    if r.pos != material_off {
        return Err(format!(
            "子网格区结束于 0x{:x}，与材质区偏移 0x{material_off:x} 不吻合（尾部填充长度假设失效）",
            r.pos
        ));
    }

    // ---- 材质区：material_count 条记录等分到挂点区（FSM 同款）----
    let region = dummy_off - material_off;
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
        let texture_path = crate::fskin::decode_path(&bytes[r.pos..p_end]);
        r.pos = p_end + 1;
        let tail = bytes[r.pos..rec_end].to_vec();
        r.pos = rec_end;
        materials.push(FscnMaterial {
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
    if r.pos != dummy_off {
        return Err(format!(
            "材质区结束于 0x{:x}，与挂点区偏移 0x{dummy_off:x} 不吻合",
            r.pos
        ));
    }

    // ---- 挂点区：[u32 数量][记录×36B]。全量样本数量恒 0 ----
    let cnt = r.u32()?;
    if cnt as usize != dummy_count {
        return Err(format!(
            "挂点区数量 {cnt} 与头部 {dummy_count} 不符 @0x{:x}",
            r.pos - 4
        ));
    }
    let mut dummies = Vec::with_capacity(dummy_count);
    for _ in 0..dummy_count {
        let name_raw = r.take(4)?;
        let end = name_raw.iter().position(|&b| b == 0).unwrap_or(name_raw.len());
        let name = String::from_utf8_lossy(&name_raw[..end]).into_owned();
        let mut values = [0f32; 7];
        for v in &mut values {
            *v = r.f32()?;
        }
        let tail = r.take(DUMMY_RECORD - 32)?.to_vec();
        dummies.push(FscnDummy { name, values, tail });
    }
    if r.pos != tree_off {
        return Err(format!(
            "挂点区结束于 0x{:x}，与BVH段偏移 0x{tree_off:x} 不吻合",
            r.pos
        ));
    }

    // ---- BVH 段（SCN 特有）----
    let record_count = r.u32()? as usize;
    if record_count == 0 || r.pos + 12 > bytes.len() {
        return Err(format!("BVH记录数异常: {record_count} @0x{:x}", r.pos - 4));
    }
    let mut tree = Vec::with_capacity(record_count);
    let mut leaf_tris = 0u64;
    for i in 0..record_count {
        let a = r.u32()?;
        let b = r.u32()?;
        let c = r.u32()?;
        let read_bbox = |r: &mut Reader| -> Result<([f32; 3], [f32; 3]), String> {
            let mut mx = [0f32; 3];
            let mut mn = [0f32; 3];
            for v in &mut mx {
                *v = r.f32()?;
            }
            for v in &mut mn {
                *v = r.f32()?;
            }
            Ok((mx, mn))
        };
        if c == 0 {
            let check = |v: u32| -> Result<(), String> {
                if v != u32::MAX && v as usize >= record_count {
                    Err(format!("BVH节点 {i} 子引用 {v} 越界（共 {record_count} 条记录）"))
                } else {
                    Ok(())
                }
            };
            check(a)?;
            check(b)?;
            let (bbox_max, bbox_min) = read_bbox(&mut r)?;
            tree.push(FscnTreeRecord::Node {
                left: a,
                right: b,
                bbox_max,
                bbox_min,
            });
        } else {
            if a != u32::MAX || b != u32::MAX {
                return Err(format!(
                    "BVH叶子 {i} 前缀异常: 0x{a:x} 0x{b:x}（应为 FFFFFFFF）"
                ));
            }
            let n = c as usize;
            let mut triangles = Vec::with_capacity(n * 3);
            for _ in 0..n * 3 {
                let v = r.u16()? as u32;
                if v >= vertex_count {
                    return Err(format!(
                        "BVH叶子 {i} 索引 {v} 越界（顶点池 {vertex_count}）"
                    ));
                }
                triangles.push(v as u16);
            }
            leaf_tris += n as u64;
            let (bbox_max, bbox_min) = read_bbox(&mut r)?;
            tree.push(FscnTreeRecord::Leaf {
                triangles,
                bbox_max,
                bbox_min,
            });
        }
    }
    if r.pos != bytes.len() {
        return Err(format!(
            "BVH段结束于 0x{:x}，文件末尾 0x{:x}（判定记录数 {record_count} 失效）",
            r.pos,
            bytes.len()
        ));
    }
    if leaf_tris != mesh_tris {
        return Err(format!(
            "BVH叶子三角形总数 {leaf_tris} 与子网格三角形总数 {mesh_tris} 不符"
        ));
    }

    Ok(FscnModel {
        path: path.to_string(),
        version,
        unknown,
        vertex_count,
        bbox_min,
        bbox_max,
        reserved256,
        vertices,
        submeshes,
        materials,
        dummies,
        tree,
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
pub fn serialize_fscn(m: &FscnModel) -> Result<Vec<u8>, String> {
    if m.reserved256.len() != TREE_OFFSET_FIELD - HEADER_LEN {
        return Err("保留区长度异常".into());
    }
    if m.vertices.len() != m.vertex_count as usize * 8 {
        return Err("顶点数据长度与顶点数不符".into());
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
    let material_off = VERTEX_OFFSET + m.vertices.len() * 4 + sbuf.len();

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
    let dummy_off = material_off + mbuf.len();

    // 挂点区
    let mut dbuf = Vec::new();
    push_u32(&mut dbuf, m.dummies.len() as u32);
    for d in &m.dummies {
        if d.tail.len() != DUMMY_RECORD - 32 {
            return Err("挂点尾部填充长度异常".into());
        }
        let name = d.name.as_bytes();
        if name.len() > 4 || name.contains(&0u8) {
            return Err(format!("挂点名 {name:?} 超出 4 字节定长字段"));
        }
        let mut field = [0u8; 4];
        field[..name.len()].copy_from_slice(name);
        dbuf.extend_from_slice(&field);
        for x in &d.values {
            push_f32(&mut dbuf, *x);
        }
        dbuf.extend_from_slice(&d.tail);
    }
    let tree_off = dummy_off + dbuf.len();

    // BVH 段：内部节点在前叶子在后（解析序即记录序，保持原文件排列）
    let mut tbuf = Vec::new();
    push_u32(&mut tbuf, m.tree.len() as u32);
    for rec in &m.tree {
        match rec {
            FscnTreeRecord::Node {
                left,
                right,
                bbox_max,
                bbox_min,
            } => {
                push_u32(&mut tbuf, *left);
                push_u32(&mut tbuf, *right);
                push_u32(&mut tbuf, 0);
                for x in bbox_max.iter().chain(bbox_min.iter()) {
                    push_f32(&mut tbuf, *x);
                }
            }
            FscnTreeRecord::Leaf {
                triangles,
                bbox_max,
                bbox_min,
            } => {
                if triangles.len() % 3 != 0 {
                    return Err("BVH叶子索引数不是 3 的倍数".into());
                }
                push_u32(&mut tbuf, u32::MAX);
                push_u32(&mut tbuf, u32::MAX);
                push_u32(&mut tbuf, (triangles.len() / 3) as u32);
                for t in triangles {
                    push_u16(&mut tbuf, *t);
                }
                for x in bbox_max.iter().chain(bbox_min.iter()) {
                    push_f32(&mut tbuf, *x);
                }
            }
        }
    }

    // 头部
    let mut out = Vec::with_capacity(tree_off + tbuf.len());
    out.extend_from_slice(b"SCN\0");
    push_u32(&mut out, m.version);
    push_u32(&mut out, m.unknown);
    push_u32(&mut out, m.vertex_count);
    push_u32(&mut out, VERTEX_OFFSET as u32);
    push_u32(&mut out, m.submeshes.len() as u32);
    push_u32(&mut out, VERTEX_OFFSET as u32 + m.vertices.len() as u32 * 4);
    push_u32(&mut out, m.materials.len() as u32);
    push_u32(&mut out, material_off as u32);
    push_u32(&mut out, m.dummies.len() as u32);
    push_u32(&mut out, dummy_off as u32);
    for x in m.bbox_min.iter().chain(m.bbox_max.iter()) {
        push_f32(&mut out, *x);
    }
    out.extend_from_slice(&m.reserved256);
    push_u32(&mut out, tree_off as u32);
    debug_assert_eq!(out.len(), VERTEX_OFFSET);
    for v in &m.vertices {
        push_f32(&mut out, *v);
    }
    out.extend_from_slice(&sbuf);
    out.extend_from_slice(&mbuf);
    out.extend_from_slice(&dbuf);
    out.extend_from_slice(&tbuf);
    Ok(out)
}

/// 目录列举用的轻量统计：(顶点数, 三角形数)。
/// 只读头部 + 子网格区（跳过索引数组），不解析顶点、材质与 BVH。
/// 泛型 Reader：磁盘 File 与 CPK 内存读取器共用。
/// 注意：不能断言子网格序号==位置或顶点数==全池
/// （corpus 分别有 47/61 个反例），只做结构游走。
pub fn light_meta<R: std::io::Read>(r: &mut R) -> Option<(u32, u32)> {
    let mut header = [0u8; 0x2C];
    r.read_exact(&mut header).ok()?;
    if &header[0..4] != b"SCN\0" {
        return None;
    }
    let rd = |o: usize| u32::from_le_bytes([header[o], header[o + 1], header[o + 2], header[o + 3]]);
    let vertex_count = rd(0x0C);
    let submesh_count = rd(0x14) as usize;
    let submesh_off = rd(0x18) as usize;
    let material_off = rd(0x20) as usize;
    if rd(0x10) as usize != VERTEX_OFFSET
        || submesh_off <= VERTEX_OFFSET
        || material_off < submesh_off
        || material_off > 1 << 24
    {
        return None;
    }
    // 游标已在 0x2C，补读到 material_off；buf 下标 = 绝对偏移 - 0x2C
    let mut buf = vec![0u8; material_off - 0x2C];
    if r.read_exact(&mut buf).is_err() {
        return None;
    }
    let u32at = |o: usize| {
        u32::from_le_bytes([
            buf[o - 0x2C],
            buf[o + 1 - 0x2C],
            buf[o + 2 - 0x2C],
            buf[o + 3 - 0x2C],
        ])
    };
    // 逐记录走子网格区：8B 头 + 24B bbox + 4B 三角形数 + 索引 + 20B 尾
    let mut pos = submesh_off;
    let mut tris = 0u32;
    for _ in 0..submesh_count {
        if pos + 36 > material_off {
            return None;
        }
        let tc = u32at(pos + 32);
        if tc > 1 << 24 {
            return None;
        }
        tris += tc;
        pos += 36 + tc as usize * 6 + SUBMESH_TAIL;
    }
    if pos != material_off {
        return None;
    }
    Some((vertex_count, tris))
}

/// 磁盘文件版：只读文件头与子网格区，不整读顶点与材质（gui 内部走 vfs，供测试与外部调用）
#[cfg_attr(not(test), allow(dead_code))]
pub fn light_meta_from_file(path: &Path) -> Option<(u32, u32)> {
    let mut f = fs::File::open(path).ok()?;
    light_meta(&mut f)
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // 项目 res 内的实测样本（system.cpk 解包）
    const G01: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Building/g01/G01DW01.FSCN");
    const WK11: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/ItemSys/ItemMdl/WK11.FSCN");

    fn assert_roundtrip(path: &str) -> FscnModel {
        let bytes = fs::read(path).unwrap_or_else(|_| panic!("读取样本失败: {path}"));
        let m = parse_fscn(path, &bytes).expect("解析");
        let out = serialize_fscn(&m).unwrap();
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
        m
    }

    /// 建筑网格 G01DW01：4 子网格 4 材质，262 内部节点 + 263 叶子的满二叉 BVH
    #[test]
    fn roundtrip_g01dw01() {
        let m = assert_roundtrip(G01);
        assert_eq!(m.version, 1);
        assert_eq!(m.unknown, 1);
        assert_eq!(m.vertex_count, 2439);
        assert_eq!(m.reserved256.len(), 256);
        assert!(m.reserved256.iter().all(|&b| b == 0));
        assert_eq!(m.submeshes.len(), 4);
        assert_eq!(m.materials.len(), 4);
        assert_eq!(m.materials[0].texture_path, r"system\building\texture\G01-086-A.tga");
        assert_eq!(m.materials[3].texture_path, r"system\building\texture\G01-080-F.tga");
        // 首顶点位置 (631.698, 6.693, 941.361)，法线 x=-1.701（未归一化墙面法线）
        assert_eq!(m.vertices[0].to_bits(), 0x441deca8);
        assert!((m.vertices[3] - -1.7009).abs() < 1e-3);
        assert_eq!(m.vertices.len(), 2439 * 8);
        // 头部包围盒 = 顶点实测范围
        assert!((m.bbox_min[0] - -782.6902).abs() < 1e-2);
        assert!((m.bbox_max[1] - 1384.9436).abs() < 1e-2);
        // BVH：根为记录 0，孩子 1/2；满二叉 262+263=525
        assert_eq!(m.tree.len(), 525);
        match &m.tree[0] {
            FscnTreeRecord::Node { left, right, .. } => assert_eq!((*left, *right), (1, 2)),
            _ => panic!("记录 0 应为内部节点"),
        }
        let nodes = m
            .tree
            .iter()
            .filter(|r| matches!(r, FscnTreeRecord::Node { .. }))
            .count();
        let leaves = m.tree.len() - nodes;
        assert_eq!((nodes, leaves), (262, 263));
        // 叶子三角形总数 == 子网格三角形总数 1559
        let leaf_tris: usize = m
            .tree
            .iter()
            .map(|r| match r {
                FscnTreeRecord::Leaf { triangles, .. } => triangles.len() / 3,
                _ => 0,
            })
            .sum();
        let mesh_tris: usize = m.submeshes.iter().map(|s| s.triangles.len() / 3).sum();
        assert_eq!((leaf_tris, mesh_tris), (1559, 1559));
    }

    /// 道具模型 WK11：35 顶点小网格，7+8=15 条 BVH 记录，单一材质
    #[test]
    fn roundtrip_wk11() {
        let m = assert_roundtrip(WK11);
        assert_eq!(m.vertex_count, 35);
        assert_eq!(m.submeshes.len(), 1);
        assert_eq!(m.materials.len(), 1);
        assert_eq!(m.materials[0].texture_path, r"system\itemmdl\WK11.tga");
        assert_eq!(m.tree.len(), 15);
        // 首顶点位精确采样（逆向时由 hex 直接读出）
        assert_eq!(m.vertices[0].to_bits(), 0x3fc23212);
        assert_eq!(m.vertices[1].to_bits(), 0x411670bd); // y = 9.403
        assert_eq!(m.vertices[7].to_bits(), 0x3ebb3e44); // uv.v
    }

    #[test]
    fn light_meta_matches_full_parse() {
        for (p, expect) in [(G01, (2439, 1559)), (WK11, (35, 44))] {
            let bytes = fs::read(p).unwrap();
            let m = parse_fscn(p, &bytes).unwrap();
            let tris: usize = m.submeshes.iter().map(|s| s.triangles.len() / 3).sum();
            assert_eq!((m.vertex_count, tris as u32), expect);
            assert_eq!(light_meta_from_file(Path::new(p)), Some(expect));
        }
    }

    /// 无贴图顶点的 UV 原生写 0xFFFFFFFF（NaN）。全量 corpus 仅
    /// Building/g06/G06DW07-A.FSCN 出现（88 个顶点），此处用改写样本
    /// 覆盖同一路径：解析应接受且往返保持字节一致。
    #[test]
    fn nan_uv_accepted() {
        let mut bytes = fs::read(WK11).unwrap();
        let vertex_count =
            u32::from_le_bytes(bytes[0xC..0x10].try_into().unwrap()) as usize;
        // 顶点 3 的 uv 两个字段（偏移 0x148 + 3*32 + 24）
        for k in 0..2 {
            let o = VERTEX_OFFSET + 3 * 32 + 24 + k * 4;
            bytes[o..o + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        assert!(vertex_count > 3);
        let m = parse_fscn(WK11, &bytes).expect("NaN UV 应可解析");
        assert!(m.vertices[3 * 8 + 6].is_nan());
        assert!(m.vertices[3 * 8 + 7].is_nan());
        let out = serialize_fscn(&m).unwrap();
        assert_eq!(bytes, out, "往返不一致");
    }
}
