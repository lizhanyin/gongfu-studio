//! .fsm (SMM v1) 静态武器网格解析与序列化
//!
//! 布局（2026-10 对 res/system/Weapon/Jian/J01.FSM 全字节逆向验证）:
//!   0x00 "SMM\0" | 0x04 版本 | 0x08 ?(恒1) | 0x0C 顶点数 | 0x10 顶点区偏移(恒0x144)
//!   0x14 子网格数 | 0x18 子网格区偏移 | 0x1C 材质数 | 0x20 材质区偏移
//!   0x24 挂点数 | 0x28 挂点区偏移 | 0x2C bbox min/max 各3个f32（注意与子网格
//!   记录相反，头部 min 在前）| 0x44..0x144 保留256字节
//!   顶点区 ×N×32B: [pos3][normal3][uv2]，法线未归一化（0.86~1.68）
//!   子网格区 ×N: 与 FAK 完全同款 [u16 0][u16 序号][u16 0][u16 顶点数]
//!                [bbox max3][bbox min3][u32 三角形数][u16 索引×3n][20字节0填充]
//!   材质区 ×N: 等分至挂点区，每条: [u32 序号][diffuse/ambient/emissive/specular
//!            各4f32][f32 高光强度][3字节前缀][NUL结尾路径][0填充]
//!   挂点区: [u32 数量][记录×36B]: [名4B NUL结尾][f32×7][00 CC CC CC 尾]
//!   挂点 f32×7 前 3 个为位置（J01: GE2(0,-20,0)柄尾 / F02(0,11.06,0)护手 /
//!   F01(0,104.3,0)近剑尖 / GE1(0,150,0)超出剑尖的特效点），后 4 个疑似
//!   四元数，但 GE1 第 4 分量为 30 不合法，语义待游戏内验证
//!
//! 与 FAK 的关系：子网格/材质记录逐字节同构，仅头部多了挂点区、顶点改为
//! 静态共享池（FAK 顶点在逐帧快照里）。

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 头部(0x44) + 保留256字节之后的顶点区固定偏移
pub const VERTEX_OFFSET: usize = 0x144;
/// 子网格记录索引区之后的固定 0 填充长度
const SUBMESH_TAIL: usize = 20;
const HEADER_LEN: usize = 0x44;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FsmSubmesh {
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
pub struct FsmMaterial {
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

/// 特效/挂点记录（GE2/GE1/F02/F01 这类命名）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FsmDummy {
    /// 定长 4 字节字段，去尾部 NUL
    pub name: String,
    /// 前 3 个为位置，后 4 个疑似四元数（GE1 的第 4 分量 30 不合法）
    pub values: [f32; 7],
    /// 记录尾填充（J01 为 00 CC CC CC，原样保留）
    pub tail: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FsmModel {
    pub path: String,
    pub version: u32,
    /// 头部 0x08 的 u32（J01=1），语义未定，按原样保留
    pub unknown: u32,
    pub vertex_count: u32,
    /// 头部包围盒：min 在前 max 在后（子网格记录相反）
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub reserved256: Vec<u8>,
    /// 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2]
    pub vertices: Vec<f32>,
    pub submeshes: Vec<FsmSubmesh>,
    pub materials: Vec<FsmMaterial>,
    pub dummies: Vec<FsmDummy>,
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

pub fn parse_fsm(path: &str, bytes: &[u8]) -> Result<FsmModel, String> {
    if bytes.len() < HEADER_LEN {
        return Err("文件太小".into());
    }
    if &bytes[0..4] != b"SMM\0" {
        return Err("魔数不是 SMM".into());
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
        && dummy_off < bytes.len())
    {
        return Err(format!(
            "区偏移异常: 子网格 0x{submesh_off:x} 材质 0x{material_off:x} 挂点 0x{dummy_off:x}"
        ));
    }
    let reserved256 = bytes[HEADER_LEN..VERTEX_OFFSET].to_vec();

    // ---- 顶点区 ----
    let vlen = submesh_off - VERTEX_OFFSET;
    if vlen % 32 != 0 || vlen / 32 != vertex_count as usize {
        return Err(format!(
            "顶点区 {vlen} 字节与头部顶点数 {vertex_count} 不符（每顶点 32B）"
        ));
    }
    let mut r = Reader::new(bytes);
    r.pos = VERTEX_OFFSET;
    let mut vertices = Vec::with_capacity(vertex_count as usize * 8);
    for _ in 0..vertex_count as usize * 8 {
        let v = r.f32()?;
        if !v.is_finite() {
            return Err(format!("顶点含非有限浮点 @0x{:x}", r.pos - 4));
        }
        vertices.push(v);
    }

    // ---- 子网格区：与 FAK 同款记录 ----
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
        submeshes.push(FsmSubmesh {
            index,
            pad: [pad_a, pad_b],
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

    // ---- 材质区：material_count 条记录等分到挂点区 ----
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
        let texture_path = String::from_utf8_lossy(&bytes[r.pos..p_end]).into_owned();
        r.pos = p_end + 1;
        let tail = bytes[r.pos..rec_end].to_vec();
        // 对齐到记录末尾（等长记录的固定填充）
        r.pos = rec_end;
        materials.push(FsmMaterial {
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

    // ---- 挂点区：[u32 数量][记录 ×N] ----
    let cnt = r.u32()?;
    if cnt as usize != dummy_count {
        return Err(format!(
            "挂点区数量 {cnt} 与头部 {dummy_count} 不符 @0x{:x}",
            r.pos - 4
        ));
    }
    let rest = bytes.len() - r.pos;
    if rest % dummy_count != 0 || rest / dummy_count < 32 {
        return Err(format!(
            "挂点区 {rest} 字节不能被 {dummy_count} 条合法记录整除（最少 32B/条）"
        ));
    }
    let drec = rest / dummy_count;
    let mut dummies = Vec::with_capacity(dummy_count);
    for _ in 0..dummy_count {
        let name_raw = r.take(4)?;
        let end = name_raw.iter().position(|&b| b == 0).unwrap_or(name_raw.len());
        let name = String::from_utf8_lossy(&name_raw[..end]).into_owned();
        let mut values = [0f32; 7];
        for v in &mut values {
            *v = r.f32()?;
        }
        let tail = r.take(drec - 32)?.to_vec();
        dummies.push(FsmDummy {
            name,
            values,
            tail,
        });
    }

    Ok(FsmModel {
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
pub fn serialize_fsm(m: &FsmModel) -> Result<Vec<u8>, String> {
    if m.reserved256.len() != VERTEX_OFFSET - HEADER_LEN {
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
        let path_bytes = mat.texture_path.as_bytes();
        if path_bytes.contains(&0u8) {
            return Err("纹理路径含非法字符".into());
        }
        mbuf.extend_from_slice(path_bytes);
        mbuf.push(0);
        mbuf.extend_from_slice(&mat.tail);
    }
    let dummy_off = material_off + mbuf.len();

    // 挂点区
    if m.dummies.is_empty() {
        return Err("挂点数为 0（挂点区至少应有数量字段，布局未在样本中验证）".into());
    }
    let mut dbuf = Vec::new();
    push_u32(&mut dbuf, m.dummies.len() as u32);
    for d in &m.dummies {
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

    // 头部
    let mut out = Vec::with_capacity(dummy_off + dbuf.len());
    out.extend_from_slice(b"SMM\0");
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
    debug_assert_eq!(out.len(), VERTEX_OFFSET);
    for v in &m.vertices {
        push_f32(&mut out, *v);
    }
    out.extend_from_slice(&sbuf);
    out.extend_from_slice(&mbuf);
    out.extend_from_slice(&dbuf);
    Ok(out)
}

/// 目录列举用的轻量统计：(顶点数, 三角形数)。
/// 只读头部 + 子网格区（跳过索引数组），不解析顶点与材质。
/// 泛型 Reader：磁盘 File 与 CPK 内存读取器共用。
pub fn light_meta<R: std::io::Read>(r: &mut R) -> Option<(u32, u32)> {
    let mut header = [0u8; 0x2C];
    r.read_exact(&mut header).ok()?;
    if &header[0..4] != b"SMM\0" {
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
    let u16at = |o: usize| u16::from_le_bytes([buf[o - 0x2C], buf[o + 1 - 0x2C]]);
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
    for i in 0..submesh_count {
        if pos + 36 > material_off {
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
    use std::collections::BTreeSet;

    // 项目 res 内的实测样本
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Weapon/Jian");
    const FILE: &str = "J01.FSM";

    fn read_sample() -> Option<Vec<u8>> {
        fs::read(format!("{ROOT}/{FILE}")).ok()
    }

    #[test]
    fn roundtrip_j01() {
        let Some(bytes) = read_sample() else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        let m = parse_fsm(FILE, &bytes).expect("解析 J01.FSM");
        let out = serialize_fsm(&m).unwrap();
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
    fn parse_j01_structure() {
        let Some(bytes) = read_sample() else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        let m = parse_fsm(FILE, &bytes).unwrap();
        assert_eq!(m.version, 1);
        assert_eq!(m.unknown, 1);
        assert_eq!(m.vertex_count, 154);
        assert_eq!(m.reserved256.len(), 256);
        assert!(m.reserved256.iter().all(|&b| b == 0));

        // 头部包围盒 = (min, max)，与网格实测范围一致
        assert!((m.bbox_min[0] - -8.9955).abs() < 1e-3);
        assert!((m.bbox_max[1] - 105.5784).abs() < 1e-3);
        assert!((m.bbox_max[2] - 2.6726).abs() < 1e-3);

        // 首顶点位精确采样（逆向时由 hex 直接读出）
        assert_eq!(m.vertices.len(), 154 * 8);
        assert_eq!(m.vertices[0].to_bits(), 0xb2b5e57d);
        assert_eq!(m.vertices[1].to_bits(), 0x42519440); // y = 52.395
        assert_eq!(m.vertices[2].to_bits(), 0xc0299042); // z = -2.649
        assert_eq!(m.vertices[7].to_bits(), 0x3c3ef540); // uv.v

        assert_eq!(m.submeshes.len(), 1);
        let s = &m.submeshes[0];
        assert_eq!(s.index, 0);
        assert_eq!(s.triangles.len(), 112 * 3);
        // 索引恰好覆盖全部 154 个顶点
        let uni: BTreeSet<u16> = s.triangles.iter().copied().collect();
        assert_eq!(uni.len(), 154);

        assert_eq!(m.materials.len(), 1);
        let mat = &m.materials[0];
        assert_eq!(mat.index, 0);
        assert_eq!(mat.texture_path, r"system\weapon\jian\J01-S01.tga");
        assert_eq!(mat.pre, [0x00, 0x00, 0xff]);
        assert!((mat.spec_power - 12.8).abs() < 0.01);
        assert_eq!(mat.diffuse, [1.0, 1.0, 1.0, 1.0]);

        // 挂点：名序 GE2/GE1/F02/F01，y 分量沿剑身分布
        let names: Vec<&str> = m.dummies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["GE2", "GE1", "F02", "F01"]);
        assert!((m.dummies[0].values[1] - -20.0).abs() < 1e-4); // GE2 柄尾
        assert!((m.dummies[1].values[1] - 150.0).abs() < 1e-4); // GE1 超出剑尖
        assert!((m.dummies[2].values[1] - 11.0586).abs() < 1e-3); // F02 护手
        assert!((m.dummies[3].values[1] - 104.3022).abs() < 1e-3); // F01 近剑尖
        assert_eq!(m.dummies[0].tail, vec![0x00, 0xcc, 0xcc, 0xcc]);
    }

    #[test]
    fn light_meta_matches_full_parse() {
        let meta = light_meta_from_file(Path::new(&format!("{ROOT}/{FILE}")));
        let Some(meta) = meta else {
            eprintln!("跳过: 测试文件不存在");
            return;
        };
        assert_eq!(meta, (154, 112));
    }
}
