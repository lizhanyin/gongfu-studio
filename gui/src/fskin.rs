//! .fskin (SKIN v1.0) 解析与序列化
//!
//! 布局已与 F3D.dll 引擎解析代码逐字段对照确认（IDA，2026-10）：
//!   ResSkin::ParseSkinFile @0x1004DED0(.​\ResSys\ResSkin.cpp) 读 312B 头
//!   → 顶点循环 12B pos / 12B normal / 8B uv(0x1004DFD3) / 影响表；
//!   F3D_ParseVertexInfluences @0x1004D3F0：4B 影响数(允许 0) +
//!   n × 20B {骨骼u32, 权重f32, 骨骼局部坐标3f32}。
//! 蒙皮消费端 SkinStream_SkinVerts_LBS @0x10050C70：
//!   pos = Σ w·M_boneSpace·boneLocalPos（原始权重累加，无归一化），
//!   法线/uv 为加载时静态拷贝（蒙皮循环只写位置三分量）。
//!
//!   0x00 "SKIN" | 0x04 版本 | 0x08 顶点数 | 0x0C 顶点偏移(恒0x138)
//!   0x10 索引节数 | 0x14 索引区偏移 | 0x18 材质数 | 0x1C 材质区偏移
//!   0x20 bbox max/min 各3个f32 | 0x38..0x138 保留256字节
//!   顶点记录(变长): [pos3][normal3][uv2][影响数n][n×(骨骼u32+权重f32+骨骼局部坐标3f32)]
//!
//! 多子网格（2026-10 对 system.cpk 全量 578 个 fskin 实测归纳，护甲类
//! Armonr 98/126 为多节，节数 1..8）：
//!   索引区 = 索引节数 × 节。节 k:
//!     [节号k<<16 u32][0 u16][顶点数 u16][24B 附加数据][三角形数 u32]
//!     + 三角形数×3 × u16 索引 + 固定 20B 零填充
//!   （附加数据 24B 在 girl 系列为 pad/hash 全零，FS01 节0 含 0xa0，
//!    其余节可见浮点/索引状数据，语义未定，按原样保留；FK55 类特效
//!    网格节头为 0xffff0000/-1，同为原样保留）
//!   材质区 = 材质数 × 0x46C(1132)B 固定槽。槽 k:
//!     [槽号k u32][diffuse/ambient/emissive/specular 各4f32][高光强度f32]
//!     [3字节前缀][纹理路径\0][零填充至槽尾]
//!   文件总长 = 材质区偏移 + 材质数 × 0x46C（材质数可为 0，如 FK55
//!   Bip01 Footsteps01 特效网格：9 顶点 0 影响，索引区直接终结于文件尾）。
//!   顶点影响数实测上限 15（解析上限放宽到 32 防损坏数据）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::vfs;

pub const VERTEX_OFFSET: usize = 0x138;
/// 顶点影响数解析上限（全量实测最大 15，防损坏数据走飞）
pub const INFLUENCE_MAX: u32 = 32;
/// 索引节尾部固定零填充
pub const SECTION_PAD: usize = 20;
/// 材质槽固定长度
pub const MATERIAL_SLOT: usize = 0x46C;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Influence {
    pub bone_index: u32,
    pub weight: f32,
    pub bone_local_pos: [f32; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub influences: Vec<Influence>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    /// 材质槽序号（多材质时 = 槽下标；单材质文件为 0）
    pub lead: u32,
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub emissive: [f32; 4],
    pub specular: [f32; 4],
    pub spec_power: f32,
    pub pre: [u8; 3],
    pub texture_path: String,
    /// 槽内路径终止符之后的填充（原样保留保证往返一致）
    pub tail: Vec<u8>,
}

/// 一个索引节 = 一个子网格。节头除三角形数外的字段语义未定，按原样保留。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FskinSubmesh {
    /// 节号<<16（FK55 类无材质特效网格为 0xffff0000）
    pub zero1: u32,
    pub zero2: u16,
    pub vertex_count2: u16,
    /// 24B 附加数据（旧版解析器的 pad4/hash1/hash2/one1/one2 视角）
    pub misc: [u32; 6],
    /// 扁平 u16 索引，每 3 个为一个三角形
    pub triangles: Vec<u16>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FskinModel {
    pub path: String,
    pub version: u32,
    pub bbox_max: [f32; 3],
    pub bbox_min: [f32; 3],
    pub reserved256: Vec<u8>,
    pub vertices: Vec<Vertex>,
    /// 索引节（子网格），与 materials 逐槽对应
    pub submeshes: Vec<FskinSubmesh>,
    /// 材质槽（可为空，如 FK55 特效网格）
    pub materials: Vec<Material>,
}

impl FskinModel {
    /// 全部子网格三角形拼接（渲染 / 导出用的合并视图）
    pub fn triangles_merged(&self) -> Vec<u16> {
        self.submeshes
            .iter()
            .flat_map(|s| s.triangles.iter().copied())
            .collect()
    }

    /// 三角形总数
    pub fn triangle_count(&self) -> usize {
        self.submeshes.iter().map(|s| s.triangles.len() / 3).sum()
    }
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
    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
}

pub fn parse_fskin(path: &str, bytes: &[u8]) -> Result<FskinModel, String> {
    if bytes.len() < VERTEX_OFFSET {
        return Err("文件太小".into());
    }
    if &bytes[0..4] != b"SKIN" {
        return Err("魔数不是 SKIN".into());
    }
    let mut rh = Reader::new(bytes);
    rh.skip(4)?;
    let version = rh.u32()?;
    let vertex_count = rh.u32()? as usize;
    let vertex_off = rh.u32()? as usize;
    let index_count = rh.u32()? as usize;
    let index_off = rh.u32()? as usize;
    let material_count = rh.u32()? as usize;
    let material_off = rh.u32()? as usize;
    let mut bbox_max = [0f32; 3];
    let mut bbox_min = [0f32; 3];
    for v in &mut bbox_max {
        *v = rh.f32()?;
    }
    for v in &mut bbox_min {
        *v = rh.f32()?;
    }
    if vertex_off != VERTEX_OFFSET {
        return Err(format!("异常的顶点偏移 0x{vertex_off:x}"));
    }
    if material_off < index_off || material_off > bytes.len() {
        return Err(format!("异常的材质区偏移 0x{material_off:x}"));
    }
    let reserved256 = bytes[0x38..VERTEX_OFFSET].to_vec();

    // ---- 顶点区 ----
    let mut vertices = Vec::with_capacity(vertex_count);
    let mut r = Reader::new(bytes);
    r.pos = vertex_off;
    while r.pos < index_off {
        let mut pos = [0f32; 3];
        let mut normal = [0f32; 3];
        let mut uv = [0f32; 2];
        for v in pos.iter_mut().chain(normal.iter_mut()).chain(uv.iter_mut()) {
            *v = r.f32()?;
        }
        let n = r.u32()?;
        if n > INFLUENCE_MAX {
            return Err(format!("非法影响骨骼数 {} @0x{:x}", n, r.pos - 4));
        }
        let mut influences = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let bone_index = r.u32()?;
            let weight = r.f32()?;
            let mut bone_local_pos = [0f32; 3];
            for v in bone_local_pos.iter_mut() {
                *v = r.f32()?;
            }
            influences.push(Influence {
                bone_index,
                weight,
                bone_local_pos,
            });
        }
        vertices.push(Vertex {
            pos,
            normal,
            uv,
            influences,
        });
    }
    if r.pos != index_off {
        return Err("顶点区长度与索引区偏移不吻合".into());
    }

    // ---- 索引区：index_count 个节（子网格），每节 36B 头 + 索引 + 20B 零填充 ----
    let mut submeshes = Vec::with_capacity(index_count);
    for k in 0..index_count {
        if r.pos + 36 > material_off {
            return Err(format!("索引节 {k} 头越界 @0x{:x}", r.pos));
        }
        let zero1 = r.u32()?;
        let zero2 = r.u16()?;
        let vertex_count2 = r.u16()?;
        let mut misc = [0u32; 6];
        for v in &mut misc {
            *v = r.u32()?;
        }
        let tri_count = r.u32()? as usize;
        if r.pos + tri_count * 6 + SECTION_PAD > material_off {
            return Err(format!(
                "索引节 {k} 数据越界: {tri_count} 三角形 @0x{:x}",
                r.pos - 4
            ));
        }
        let mut triangles = Vec::with_capacity(tri_count * 3);
        for _ in 0..tri_count * 3 {
            triangles.push(r.u16()?);
        }
        let _pad = r.take(SECTION_PAD)?;
        submeshes.push(FskinSubmesh {
            zero1,
            zero2,
            vertex_count2,
            misc,
            triangles,
        });
    }
    if r.pos != material_off {
        return Err("索引区长度与材质区偏移不吻合".into());
    }

    // ---- 材质区：material_count 个 0x46C 固定槽，文件总长须精确吻合 ----
    let slot_end = material_off.checked_add(material_count * MATERIAL_SLOT).ok_or("材质槽数溢出")?;
    if bytes.len() != slot_end {
        return Err(format!(
            "文件长度 {:#x} 与材质槽不吻合（期望 {slot_end:#x}，材质数 {material_count}）",
            bytes.len()
        ));
    }
    let mut materials = Vec::with_capacity(material_count);
    for _ in 0..material_count {
        let slot_start = r.pos;
        let slot_limit = slot_start + MATERIAL_SLOT;
        let lead = r.u32()?;
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
        let p_start = r.pos;
        let p_end = bytes[p_start..slot_limit]
            .iter()
            .position(|&c| c == 0)
            .ok_or("纹理路径未终止")?
            + p_start;
        let texture_path = decode_path(&bytes[p_start..p_end]);
        r.pos = p_end + 1;
        let tail = bytes[r.pos..slot_limit].to_vec();
        r.pos = slot_limit;
        materials.push(Material {
            lead,
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

    Ok(FskinModel {
        path: path.to_string(),
        version,
        bbox_max,
        bbox_min,
        reserved256,
        vertices,
        submeshes,
        materials,
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

/// 纹理路径按 GBK 解码（游戏资源含中文路径，如 袖256.tga）；
/// ASCII 路径不受影响；GBK 亦无法解码的乱码字节退化为 UTF-8 有损替换。
pub(crate) fn decode_path(bytes: &[u8]) -> String {
    let (s, _, err) = encoding_rs::GBK.decode(bytes);
    if err {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        s.into_owned()
    }
}

/// decode_path 的逆：字符串编码回 GBK 字节；无法映射的字符退化为 UTF-8 字节
pub(crate) fn encode_path(s: &str) -> Vec<u8> {
    let (b, _, err) = encoding_rs::GBK.encode(s);
    if err {
        s.as_bytes().to_vec()
    } else {
        b.into_owned()
    }
}

/// 重新序列化。未修改的模型可字节级还原原文件（见 roundtrip 测试）。
pub fn serialize_fskin(m: &FskinModel) -> Result<Vec<u8>, String> {
    if m.reserved256.len() != VERTEX_OFFSET - 0x38 {
        return Err("保留区长度异常".into());
    }
    // 顶点区
    let mut vbuf = Vec::new();
    for v in &m.vertices {
        for x in v.pos.iter().chain(v.normal.iter()).chain(v.uv.iter()) {
            push_f32(&mut vbuf, *x);
        }
        push_u32(&mut vbuf, v.influences.len() as u32);
        for inf in &v.influences {
            push_u32(&mut vbuf, inf.bone_index);
            push_f32(&mut vbuf, inf.weight);
            for x in inf.bone_local_pos.iter() {
                push_f32(&mut vbuf, *x);
            }
        }
    }
    let index_off = VERTEX_OFFSET + vbuf.len();
    if index_off > u32::MAX as usize {
        return Err("顶点区过大".into());
    }

    // 索引区：每节 36B 头 + 索引 + 20B 零填充
    let mut ibuf = Vec::new();
    for s in &m.submeshes {
        if s.triangles.len() % 3 != 0 {
            return Err("三角形索引数不是 3 的倍数".into());
        }
        push_u32(&mut ibuf, s.zero1);
        push_u16(&mut ibuf, s.zero2);
        push_u16(&mut ibuf, m.vertices.len().min(u16::MAX as usize) as u16);
        for v in s.misc.iter() {
            push_u32(&mut ibuf, *v);
        }
        push_u32(&mut ibuf, (s.triangles.len() / 3) as u32);
        for t in &s.triangles {
            push_u16(&mut ibuf, *t);
        }
        ibuf.extend_from_slice(&[0u8; SECTION_PAD]);
    }
    let material_off = index_off + ibuf.len();

    // 材质区：每材质一个 0x46C 固定槽
    let mut mbuf = Vec::new();
    for mat in &m.materials {
        let slot_start = mbuf.len();
        push_u32(&mut mbuf, mat.lead);
        for arr in [mat.diffuse, mat.ambient, mat.emissive, mat.specular] {
            for x in arr {
                push_f32(&mut mbuf, x);
            }
        }
        push_f32(&mut mbuf, mat.spec_power);
        mbuf.extend_from_slice(&mat.pre);
        let path_bytes = encode_path(&mat.texture_path);
        if path_bytes.contains(&0u8) {
            return Err("纹理路径含非法字符".into());
        }
        mbuf.extend_from_slice(&path_bytes);
        mbuf.push(0);
        mbuf.extend_from_slice(&mat.tail);
        let written = mbuf.len() - slot_start;
        if written != MATERIAL_SLOT {
            return Err(format!(
                "材质槽长度异常: 实际 {written} 字节（应为 {MATERIAL_SLOT}），纹理路径或尾部填充被改动"
            ));
        }
    }

    // 头部
    let mut out = Vec::with_capacity(material_off + mbuf.len());
    out.extend_from_slice(b"SKIN");
    push_u32(&mut out, m.version);
    push_u32(&mut out, m.vertices.len() as u32);
    push_u32(&mut out, VERTEX_OFFSET as u32);
    push_u32(&mut out, m.submeshes.len() as u32);
    push_u32(&mut out, index_off as u32);
    push_u32(&mut out, m.materials.len() as u32);
    push_u32(&mut out, material_off as u32);
    for x in m.bbox_max.iter() {
        push_f32(&mut out, *x);
    }
    for x in m.bbox_min.iter() {
        push_f32(&mut out, *x);
    }
    out.extend_from_slice(&m.reserved256);
    debug_assert_eq!(out.len(), VERTEX_OFFSET);
    out.extend_from_slice(&vbuf);
    out.extend_from_slice(&ibuf);
    out.extend_from_slice(&mbuf);
    Ok(out)
}

pub fn compute_bbox(m: &FskinModel) -> ([f32; 3], [f32; 3]) {
    let mut mx = [f32::MIN; 3];
    let mut mn = [f32::MAX; 3];
    for v in &m.vertices {
        for i in 0..3 {
            mx[i] = mx[i].max(v.pos[i]);
            mn[i] = mn[i].min(v.pos[i]);
        }
    }
    (mx, mn)
}

// ---------------- 模型集扫描 ----------------

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartInfo {
    pub path: String,
    pub file_name: String,
    pub rel_dir: String,
    pub in_root: bool,
    pub vertex_count: usize,
    pub triangle_count: usize,
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub texture_path: String,
    pub texture_file: Option<String>,
    /// 该部件用到的骨骼 ID（去重排序）
    pub bones: Vec<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PartGroup {
    pub label: String,
    pub signature: String,
    pub parts: Vec<PartInfo>,
    /// 替换槽（"hair" / "upper" / "lower" / "boot"）：同槽各组互斥显示，UI 合并为一个可换部件。
    /// 空串 = 独立部件组（现有行为）。
    pub slot: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TexInfo {
    /// 小写 stem，用于与材质路径匹配
    pub stem: String,
    pub path: String,
}

/// 纹理槽的一个可选项
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TexVariant {
    pub label: String,
    pub path: String,
}

/// 纹理切换槽：把一组贴图应用到指定部件（替换材质贴图，或作透明叠层）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TexSlot {
    /// "face" | "face_detail"
    pub key: String,
    pub label: String,
    /// 应用到的部件 fskin 路径
    pub part: String,
    /// true = 透明叠层（TFaceDetail 饰品），false = 替换材质贴图（TFace 脸）
    pub overlay: bool,
    pub variants: Vec<TexVariant>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelSet {
    pub root_dir: String,
    pub groups: Vec<PartGroup>,
    pub textures: Vec<TexInfo>,
    /// 纹理切换槽（脸部纹理 / 饰品等），无对应资源时为空
    pub tex_slots: Vec<TexSlot>,
    /// 目录配套的默认骨架（act\ 下按目录名关键词选，如 AvatarBoy →
    /// act\boy.FSKEL）；前端整体预览据此同步骨架——部件 boneIndex 按
    /// 各自骨架编号，模型目录必须与配套骨架一起加载（AvatarBoy 配
    /// girl.FSKEL 会蒙皮错位渲染成女体）。无 act 目录时为 None
    #[serde(default)]
    pub default_fskel: Option<String>,
}

/// (path 是否为 root 的直接子文件, 相对目录)。path 由 vfs 以 root 为前缀拼出，
/// 字节级前缀必然成立。
fn rel_to_root(root: &str, path: &str) -> (bool, String) {
    let p = path.trim_end_matches(['\\', '/']);
    let Some(rest) = p.strip_prefix(root.trim_end_matches(['\\', '/'])) else {
        return (false, String::new());
    };
    let rel = rest.trim_start_matches(['\\', '/']);
    match rel.rsplit_once(['\\', '/']) {
        Some((dir, _)) => (false, dir.to_string()),
        None => (true, String::new()),
    }
}

/// 按部件文件名关键词判定替换槽：
/// hair* 发型、Upper* 上衣、Lower* 下装、Boot* 鞋子（YiFu/ 换装目录与之对应）。
/// 返回 None = 独立部件组。
fn detect_slot(name_lower: &str) -> Option<&'static str> {
    [("hair", "hair"), ("upper", "upper"), ("lower", "lower"), ("boot", "boot")]
        .into_iter()
        .find(|(kw, _)| name_lower.contains(kw))
        .map(|(_, slot)| slot)
}

/// 目录配套的默认骨架：`act\` 下按目录名关键词（含 girl/boy）选取，
/// 规则见 fskel::default_fskel_of。scan_model_set 与前端整体预览
/// （dir_default_fskel 命令）共用；无 act 目录返回 None
pub fn default_fskel_for_dir(dir: &str) -> Option<String> {
    let lower_dir = dir
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let keyword = if lower_dir.contains("girl") {
        "girl"
    } else if lower_dir.contains("boy") {
        "boy"
    } else {
        lower_dir.as_str()
    };
    crate::fskel::default_fskel_of(&vfs::join_vpath(dir, "act"), keyword)
}

/// 文件名去扩展名
fn stem_of(name: &str) -> String {
    name.rsplit_once('.').map(|(s, _)| s.into()).unwrap_or_else(|| name.into())
}

/// 扫描目录（或 `cpk:` 包内目录）：按 (顶点数, 索引区偏移) 几何签名把换装变体
/// 归为一组；含根目录文件的组排在前面且默认选中根目录成员。
pub fn scan_model_set(dir: &str) -> Result<ModelSet, String> {
    if !vfs::exists(dir) {
        return Err(format!("目录不存在: {dir}"));
    }
    // 纹理索引: 小写文件名(不含扩展名) -> 路径
    let tex_files = vfs::walk_files(dir, &["dds", "tga", "png", "jpg", "bmp"]);
    let mut tex_index: BTreeMap<String, String> = BTreeMap::new();
    for t in &tex_files {
        let stem = stem_of(&t.name).to_lowercase();
        tex_index.entry(stem).or_insert_with(|| t.path.clone());
    }

    let mut files = vfs::walk_files(dir, &["fskin"]);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut parts = Vec::new();
    let mut sigs = Vec::new();
    for f in &files {
        let bytes = vfs::read_file(&f.path).map_err(|e| format!("读取 {}: {e}", f.path))?;
        let m = parse_fskin(&f.path, &bytes).map_err(|e| format!("{}: {e}", f.name))?;
        // 几何签名: 顶点数 + 三角形总数（变长顶点记录总长），相同即为同一网格的换装变体
        sigs.push(format!("{}_{}", m.vertices.len(), m.triangle_count()));
        let texture_path = m
            .materials
            .first()
            .map(|s| s.texture_path.clone())
            .unwrap_or_default();
        let tex_file = texture_path
            .rsplit(['\\', '/'])
            .next()
            .and_then(|base| base.rsplit_once('.').map(|(stem, _)| stem.to_lowercase()))
            .and_then(|stem| tex_index.get(&stem).cloned());
        let mut bones: Vec<u32> = m
            .vertices
            .iter()
            .flat_map(|v| v.influences.iter().map(|i| i.bone_index))
            .collect();
        bones.sort_unstable();
        bones.dedup();
        let (in_root, rel_dir) = rel_to_root(dir, &f.path);
        parts.push(PartInfo {
            path: f.path.clone(),
            file_name: f.name.clone(),
            rel_dir,
            in_root,
            vertex_count: m.vertices.len(),
            triangle_count: m.triangle_count(),
            bbox_min: m.bbox_min,
            bbox_max: m.bbox_max,
            texture_path,
            texture_file: tex_file,
            bones,
        });
    }

    // 按几何签名分组；同槽（如 hair）跨几何组互斥替换
    let mut groups_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, sig) in sigs.into_iter().enumerate() {
        groups_map.entry(sig).or_default().push(i);
    }
    let mut groups: Vec<PartGroup> = Vec::new();
    for (sig, mut idxs) in groups_map {
        idxs.sort_by_key(|&i| (!parts[i].in_root, parts[i].file_name.clone()));
        let label = parts[idxs[0]]
            .file_name
            .trim_end_matches(".fskin")
            .to_string();
        let group_parts: Vec<PartInfo> = idxs.into_iter().map(|i| parts[i].clone()).collect();
        let has_root = group_parts.iter().any(|p| p.in_root);
        // 槽判定：部件名含关键词 → 对应替换槽（同槽各组互斥切换）
        let slot = group_parts
            .iter()
            .find_map(|p| detect_slot(&p.file_name.to_lowercase()))
            .unwrap_or_default()
            .to_string();
        groups.push(PartGroup {
            label: if has_root {
                label
            } else {
                format!("{label} (附加)")
            },
            signature: sig,
            parts: group_parts,
            slot,
        });
    }
    // 根目录组在前，其余按名称
    groups.sort_by_key(|g| (!g.parts.iter().any(|p| p.in_root), g.label.clone()));

    // 纹理切换槽：TFace/ → 脸部纹理替换，TFaceDetail/ → 头部饰品透明叠层。
    // 目标部件 = 材质贴图 stem 以 "wf" 开头的部件（头部）。
    let mut tex_slots: Vec<TexSlot> = Vec::new();
    if let Some(head) = parts.iter().find(|p| {
        p.texture_file
            .as_deref()
            .and_then(|f| f.rsplit(['\\', '/']).next())
            .map(|f| {
                let stem = f
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();
                stem.starts_with("wf")
            })
            .unwrap_or(false)
    }) {
        let head_path = head.path.clone();
        let original = head.texture_file.clone().unwrap_or_default();
        for (dir_name, key, label, overlay) in [
            ("TFace", "face", "脸部纹理", false),
            ("TFaceDetail", "face_detail", "饰品", true),
        ] {
            let mut variants: Vec<TexVariant> = tex_files
                .iter()
                .filter(|p| {
                    // 父目录名 == TFace / TFaceDetail
                    p.path
                        .rsplit(['\\', '/'])
                        .nth(1)
                        .map(|n| n.eq_ignore_ascii_case(dir_name))
                        .unwrap_or(false)
                })
                .map(|p| TexVariant {
                    label: stem_of(&p.name),
                    path: p.path.clone(),
                })
                .collect();
            variants.sort_by(|a, b| a.label.cmp(&b.label));
            if variants.is_empty() {
                continue;
            }
            if !overlay && !original.is_empty() {
                // 替换槽首个选项保持原始贴图
                variants.insert(
                    0,
                    TexVariant {
                        label: "原始".into(),
                        path: original.clone(),
                    },
                );
            }
            tex_slots.push(TexSlot {
                key: key.into(),
                label: label.into(),
                part: head_path.clone(),
                overlay,
                variants,
            });
        }
    }

    // 配套默认骨架：act\ 下按目录名关键词选取（AvatarGirl → girl.FSKEL、
    // AvatarBoy → boy.FSKEL；见 default_fskel_for_dir）
    let default_fskel = default_fskel_for_dir(dir);

    Ok(ModelSet {
        root_dir: dir.to_string(),
        groups,
        textures: tex_index
            .into_iter()
            .map(|(stem, path)| TexInfo { stem, path })
            .collect(),
        tex_slots,
        default_fskel,
    })
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 递归收集 ROOT 下全部 .fskin 磁盘路径（原 fs::walk 的测试替代，走 vfs）
    fn collect_fskins() -> Vec<PathBuf> {
        vfs::walk_files(ROOT, &["fskin"])
            .into_iter()
            .map(|e| PathBuf::from(e.path))
            .collect()
    }

    // 项目根目录 res/system 为 pack 解压后的实际目录
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/AvatarGirl");

    /// 布料骨局部四元数的逐帧变化幅度（girl 采样 + 走路全帧）：
    /// 判定 FSKEL 是否含布料动画；对照身体骨（大腿 0x7e）应有明显摆动
    #[test]
    fn cloth_anim_amount() {
        use crate::fskel;
        for act in ["lwk1.FSKEL", "rwk1.FSKEL", "girl.FSKEL"] {
            let skel = fskel::get_or_load(&format!("{ROOT}/act/{act}")).unwrap();
            let nf = skel.frame_count() as usize;
            let step = if nf > 40 { nf / 40 } else { 1 };
            let frames: Vec<usize> = (0..nf).step_by(step).chain(std::iter::once(nf - 1)).collect();
            let flat: Vec<Vec<f32>> = frames
                .iter()
                .map(|&f| skel.frames_flat(f as u32, f as u32 + 1))
                .collect();
            let vary = |b: usize| -> (f32, f32) {
                let mut mx = 0f32;
                let mut t_sum = 0f32;
                for w in flat.windows(2) {
                    let o0 = b * 7;
                    let a = [w[0][o0 + 3], w[0][o0 + 4], w[0][o0 + 5], w[0][o0 + 6]];
                    let bq = [w[1][o0 + 3], w[1][o0 + 4], w[1][o0 + 5], w[1][o0 + 6]];
                    // 相邻帧角度差（q 与 -q 等价）
                    let dot = (a[0] * bq[0] + a[1] * bq[1] + a[2] * bq[2] + a[3] * bq[3]).abs();
                    let d = 2.0 * dot.min(1.0).acos().to_degrees();
                    mx = mx.max(d);
                    t_sum += d;
                }
                (mx, t_sum / (flat.len() - 1).max(1) as f32)
            };
            let show = |label: &str, bones: &[usize]| {
                let parts: Vec<String> = bones
                    .iter()
                    .map(|&b| {
                        let (mx, avg) = vary(b);
                        format!("0x{b:x}:{avg:5.2}/{mx:6.1}°")
                    })
                    .collect();
                eprintln!("  {label}: {}", parts.join(" "));
            };
            eprintln!("== {act} ({nf}帧) 相邻帧角变化 avg/max：");
            show("布料链根", &[0x59, 0x5f, 0x65, 0x6b, 0x71, 0x77]);
            show("布料链成员", &[0x5a, 0x60, 0x66, 0x6c, 0x72, 0x78]);
            show("身体对照  ", &[0x2, 0x5, 0x7e, 0x82]);
        }
    }
    /// 裙摆布料骨回归：Bone* 布料链的 FSKEL 姿态是布料模拟种子状态，
    /// 与 FSKIN 绑定数据恒不一致（0x6b/0x71 链 girl.FSKEL 全 1633 帧
    /// M·boneLocalPos 残差最小 16.7 不回落；身体骨存在 ≈2.7 的帧）——
    /// 直接精确 LBS 会把裙摆顶点甩出 40+ 单位（穿进躯干的尖刺根因）。
    /// viewer 因此对全部 Bone* 骨（^Bone\d+$，有顶点影响的）改做次级
    /// 运动模拟（弹簧跟随 + 腿碰撞，见 viewer.detectCloth）。本测试固化：
    /// ① Bone* 全员可被名字规则命中；② 纯精确 LBS 在帧 0 的尖刺量级。
    #[test]
    fn skirt_cloth_bones() {
        use crate::fskel;
        let lower = parse_fskin(
            &format!("{ROOT}/girl_Lower.fskin"),
            &fs::read(format!("{ROOT}/girl_Lower.fskin")).unwrap(),
        )
        .unwrap();
        let skel = fskel::get_or_load(&format!("{ROOT}/act/girl.FSKEL")).unwrap();
        let n = skel.bone_names.len();
        let is_bone = |b: usize| skel.bone_names[b].starts_with("Bone");
        // 部件使用的 Bone* 骨全部命中 ^Bone\d+$ 命名规则
        let used: std::collections::BTreeSet<usize> = lower
            .vertices
            .iter()
            .flat_map(|v| v.influences.iter().map(|i| i.bone_index as usize))
            .filter(|&b| is_bone(b))
            .collect();
        assert!(used.len() >= 13, "girl_Lower 应使用 13+ 根 Bone* 骨");
        for &b in &used {
            let name = &skel.bone_names[b];
            assert!(
                name.len() > 4 && name[4..].chars().all(|c| c.is_ascii_digit()),
                r"骨名 {name} 应匹配 ^Bone\d+$"
            );
        }
        eprintln!("girl_Lower 使用 Bone* 骨: {used:?}");

        // 纯精确 LBS 帧 0 的尖刺证据（0x6c/0x6d/0x72/0x73 链甩出 40+）
        let flat = skel.frames_flat(0, 1);
        let qm = |q: [f32; 4]| -> [[f32; 4]; 4] {
            let nz = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            let (x, y, z, w) = (q[0] / nz, q[1] / nz, q[2] / nz, q[3] / nz);
            [
                [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w), 0.0],
                [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w), 0.0],
                [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y), 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ]
        };
        let mut world = vec![[[0f32; 4]; 4]; n];
        for b in 0..n {
            let o = b * 7;
            let mut m = qm([flat[o + 3], flat[o + 4], flat[o + 5], flat[o + 6]]);
            m[0][3] = flat[o];
            m[1][3] = flat[o + 1];
            m[2][3] = flat[o + 2];
            let p = skel.parents[b];
            if p >= 0 && (p as usize) < b {
                let pm = world[p as usize];
                let mut w = [[0f32; 4]; 4];
                for i in 0..4 {
                    for j in 0..4 {
                        w[i][j] = (0..4).map(|k| pm[i][k] * m[k][j]).sum();
                    }
                }
                world[b] = w;
            } else {
                world[b] = m;
            }
        }
        let mut max_dev = 0f32;
        for v in &lower.vertices {
            let wsum: f32 = v.influences.iter().map(|i| i.weight).sum();
            let mut out = [0f32; 3];
            for inf in &v.influences {
                let m = &world[(inf.bone_index as usize).min(n - 1)];
                let lp = inf.bone_local_pos;
                let x = [
                    m[0][0] * lp[0] + m[0][1] * lp[1] + m[0][2] * lp[2] + m[0][3],
                    m[1][0] * lp[0] + m[1][1] * lp[1] + m[1][2] * lp[2] + m[1][3],
                    m[2][0] * lp[0] + m[2][1] * lp[1] + m[2][2] * lp[2] + m[2][3],
                ];
                for k in 0..3 {
                    out[k] += inf.weight / wsum * x[k];
                }
            }
            max_dev = max_dev.max(
                ((out[0] - v.pos[0]).powi(2) + (out[1] - v.pos[1]).powi(2) + (out[2] - v.pos[2]).powi(2)).sqrt(),
            );
        }
        eprintln!("纯精确 LBS 帧0 裙摆最大偏差 {max_dev:.1}");
        assert!(max_dev > 40.0, "纯 LBS 应复现 40+ 尖刺（实测 {max_dev}），若测试资源变化请重估");
    }

    /// 诊断 hair05 残余位移：pos 摆放位置、部件绑定变换 vs 动作参考帧骨骼世界矩阵
    #[test]
    fn hair05_deep_diag() {
        use crate::fskel;
        fn bbox(vs: &[Vertex]) -> ([f32; 3], [f32; 3]) {
            let mut mn = [f32::MAX; 3];
            let mut mx = [f32::MIN; 3];
            for v in vs {
                for k in 0..3 {
                    mn[k] = mn[k].min(v.pos[k]);
                    mx[k] = mx[k].max(v.pos[k]);
                }
            }
            (mn, mx)
        }
        let load = |rel: &str| parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap()).unwrap();
        let head = load("girl_head.fskin");
        let h5 = load("MHair/Girl_hair05.fskin");
        let (hmn, hmx) = bbox(&head.vertices);
        let (mn5, mx5) = bbox(&h5.vertices);
        eprintln!("头部 pos bbox: {:?} .. {:?}", hmn, hmx);
        eprintln!("hair05 pos bbox: {:?} .. {:?}", mn5, mx5);

        // hair05 的 0x6 单影响点对
        let pairs5: Vec<([f32; 3], [f32; 3])> = h5
            .vertices
            .iter()
            .filter(|v| v.influences.len() == 1 && v.influences[0].bone_index == 0x6)
            .map(|v| (v.influences[0].bone_local_pos, v.pos))
            .collect();

        // sta.FSKEL 第 0 帧 0x6 骨世界矩阵（父 × T × R）
        let skel = fskel::get_or_load(&format!("{ROOT}/act/sta.FSKEL")).unwrap();
        let flat = skel.frames_flat(0, 1);
        let qm = |q: [f32; 4]| -> [[f32; 3]; 3] {
            let n = (q[0]*q[0]+q[1]*q[1]+q[2]*q[2]+q[3]*q[3]).sqrt();
            let (x, y, z, w) = (q[0]/n, q[1]/n, q[2]/n, q[3]/n);
            [
                [1.0-2.0*(y*y+z*z), 2.0*(x*y-z*w), 2.0*(x*z+y*w)],
                [2.0*(x*y+z*w), 1.0-2.0*(x*x+z*z), 2.0*(y*z-x*w)],
                [2.0*(x*z-y*w), 2.0*(y*z+x*w), 1.0-2.0*(x*x+y*y)],
            ]
        };
        let pairs_head: Vec<([f32; 3], [f32; 3])> = head
            .vertices
            .iter()
            .filter(|v| v.influences.len() == 1 && v.influences[0].bone_index == 0x6)
            .map(|v| (v.influences[0].bone_local_pos, v.pos))
            .collect();
        // 穷举四元数约定：哪个能让 M6·boneLocalPos ≈ head pos
        let variants: [(&str, Box<dyn Fn(usize) -> [f32; 4]>); 4] = [
            ("xyzw", Box::new(|o: usize| [flat[o+3], flat[o+4], flat[o+5], flat[o+6]])),
            ("wxyz", Box::new(|o: usize| [flat[o+6], flat[o+3], flat[o+4], flat[o+5]])),
            (
                "conj(xyzw)",
                Box::new(|o: usize| [-flat[o+3], -flat[o+4], -flat[o+5], flat[o+6]]),
            ),
            (
                "conj(wxyz)",
                Box::new(|o: usize| [-flat[o+6], -flat[o+3], -flat[o+4], -flat[o+5]]),
            ),
        ];
        for (name, getq) in variants {
            let mut world: Vec<[[f32; 4]; 4]> = vec![[[0.0; 4]; 4]; skel.parents.len()];
            for b in 0..skel.parents.len() {
                let o = b * 7;
                let t = [flat[o], flat[o + 1], flat[o + 2]];
                let r = qm(getq(o));
                let mut m = [[0f32; 4]; 4];
                for i in 0..3 {
                    for j in 0..3 {
                        m[i][j] = r[i][j];
                    }
                    m[i][3] = t[i];
                }
                m[3][3] = 1.0;
                let p = skel.parents[b];
                if p >= 0 {
                    let pm = world[p as usize];
                    let mut w = [[0f32; 4]; 4];
                    for i in 0..4 {
                        for j in 0..4 {
                            w[i][j] = (0..4).map(|k| pm[i][k] * m[k][j]).sum();
                        }
                    }
                    world[b] = w;
                } else {
                    world[b] = m;
                }
            }
            let m6 = &world[0x6];
            let err = pairs_head
                .iter()
                .map(|(a, b)| {
                    let x = [
                        m6[0][0]*a[0] + m6[0][1]*a[1] + m6[0][2]*a[2] + m6[0][3],
                        m6[1][0]*a[0] + m6[1][1]*a[1] + m6[1][2]*a[2] + m6[1][3],
                        m6[2][0]*a[0] + m6[2][1]*a[1] + m6[2][2]*a[2] + m6[2][3],
                    ];
                    ((x[0]-b[0]).powi(2) + (x[1]-b[1]).powi(2) + (x[2]-b[2]).powi(2)).sqrt()
                })
                .fold(0f32, f32::max);
            eprintln!("约定 {name:12} → M6·local vs head pos 最大误差 {err:8.3}");
        }
        let mut world: Vec<[[f32; 4]; 4]> = vec![[[0.0; 4]; 4]; skel.parents.len()];
        for b in 0..skel.parents.len() {
            let o = b * 7;
            let t = [flat[o], flat[o + 1], flat[o + 2]];
            let r = qm([flat[o + 3], flat[o + 4], flat[o + 5], flat[o + 6]]);
            let mut m = [[0f32; 4]; 4];
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] = r[i][j];
                }
                m[i][3] = t[i];
            }
            m[3][3] = 1.0;
            let p = skel.parents[b];
            if p >= 0 {
                let pm = world[p as usize];
                let mut w = [[0f32; 4]; 4];
                for i in 0..4 {
                    for j in 0..4 {
                        w[i][j] = (0..4).map(|k| pm[i][k] * m[k][j]).sum();
                    }
                }
                world[b] = w;
            } else {
                world[b] = m;
            }
        }
        let m6 = world[0x6];
        eprintln!("sta 第0帧 0x6 世界矩阵:");
        for row in m6 {
            eprintln!("  [{:8.3} {:8.3} {:8.3} | {:8.3}]", row[0], row[1], row[2], row[3]);
        }
        // 祖先链逐层检查
        let mut chain = vec![0x6usize];
        while skel.parents[*chain.last().unwrap()] >= 0 {
            chain.push(skel.parents[*chain.last().unwrap()] as usize);
        }
        chain.reverse();
        eprintln!("0x6 祖先链: {:?}", chain);
        for &b in &chain {
            let o = b * 7;
            eprintln!(
                "  骨 0x{:x} 父 {:?} t=({:8.3},{:8.3},{:8.3})",
                b,
                skel.parents[b],
                flat[o],
                flat[o + 1],
                flat[o + 2]
            );
        }
        // M6 · local 与 hair05 authored pos 的偏差 = 当前实现（按部件拟合）在参考帧的显示位置差
        let mut dmax = 0f32;
        let mut dsum = 0f32;
        for (a, b) in &pairs5 {
            let x = [
                m6[0][0]*a[0] + m6[0][1]*a[1] + m6[0][2]*a[2] + m6[0][3],
                m6[1][0]*a[0] + m6[1][1]*a[1] + m6[1][2]*a[2] + m6[1][3],
                m6[2][0]*a[0] + m6[2][1]*a[1] + m6[2][2]*a[2] + m6[2][3],
            ];
            let d = ((x[0]-b[0]).powi(2) + (x[1]-b[1]).powi(2) + (x[2]-b[2]).powi(2)).sqrt();
            dmax = dmax.max(d);
            dsum += d;
        }
        eprintln!(
            "M6·boneLocalPos vs hair05 pos: 平均 {:.3}, 最大 {:.3}（{} 点）",
            dsum / pairs5.len() as f32,
            dmax,
            pairs5.len()
        );
        // 对照：同样量对头部（应接近 0 —— pos 就是参考帧下的摆放）
        let mut hd = 0f32;
        for (a, b) in &pairs_head {
            let x = [
                m6[0][0]*a[0] + m6[0][1]*a[1] + m6[0][2]*a[2] + m6[0][3],
                m6[1][0]*a[0] + m6[1][1]*a[1] + m6[1][2]*a[2] + m6[1][3],
                m6[2][0]*a[0] + m6[2][1]*a[1] + m6[2][2]*a[2] + m6[2][3],
            ];
            hd = hd.max(((x[0]-b[0]).powi(2) + (x[1]-b[1]).powi(2) + (x[2]-b[2]).powi(2)).sqrt());
        }
        eprintln!("M6·boneLocalPos vs head pos: 最大 {:.3}（{} 点）", hd, pairs_head.len());
    }

    /// 诊断：hair05 游戏真实 LBS（M·boneLocalPos）vs viewer 共识渲染的摆放差异
    #[test]
    fn hair05_two_paths() {
        use crate::fskel;
        let load = |rel: &str| parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap()).unwrap();
        let head = load("girl_head.fskin");
        let h5 = load("MHair/Girl_hair05.fskin");
        let h4 = load("MHair/Girl_hair04.fskin");

        // 各部件用到的骨骼
        let bones_of = |m: &FskinModel| {
            let mut b: Vec<u32> = m.vertices.iter().flat_map(|v| v.influences.iter().map(|i| i.bone_index)).collect();
            b.sort_unstable(); b.dedup(); b
        };
        eprintln!("head bones: {:?}", bones_of(&head));
        eprintln!("hair05 bones: {:?}", bones_of(&h5));
        eprintln!("hair04 bones: {:?}", bones_of(&h4));

        // sta.FSKEL 世界矩阵（帧 f）
        let skel = fskel::get_or_load(&format!("{ROOT}/act/sta.FSKEL")).unwrap();
        let qm = |q: [f32; 4]| -> [[f32; 4]; 4] {
            let n = (q[0]*q[0]+q[1]*q[1]+q[2]*q[2]+q[3]*q[3]).sqrt();
            let (x, y, z, w) = (q[0]/n, q[1]/n, q[2]/n, q[3]/n);
            let r = [
                [1.0-2.0*(y*y+z*z), 2.0*(x*y-z*w), 2.0*(x*z+y*w)],
                [2.0*(x*y+z*w), 1.0-2.0*(x*x+z*z), 2.0*(y*z-x*w)],
                [2.0*(x*z-y*w), 2.0*(y*z+x*w), 1.0-2.0*(x*x+y*y)],
            ];
            let mut m = [[0f32; 4]; 4];
            for i in 0..3 { for j in 0..3 { m[i][j] = r[i][j]; } m[i][3] = 0.0; }
            m
        };
        let world_at = |frame: usize| -> Vec<[[f32; 4]; 4]> {
            let flat = skel.frames_flat(frame as u32, frame as u32 + 1);
            let mut world = vec![[[0f32; 4]; 4]; skel.parents.len()];
            for b in 0..skel.parents.len() {
                let o = b * 7;
                let mut m = qm([flat[o+3], flat[o+4], flat[o+5], flat[o+6]]);
                m[0][3] = flat[o]; m[1][3] = flat[o+1]; m[2][3] = flat[o+2]; m[3][3] = 1.0;
                let p = skel.parents[b];
                if p >= 0 {
                    let pm = world[p as usize];
                    let mut w = [[0f32; 4]; 4];
                    for i in 0..4 { for j in 0..4 { w[i][j] = (0..4).map(|k| pm[i][k]*m[k][j]).sum(); } }
                    world[b] = w;
                } else { world[b] = m; }
            }
            world
        };
        let apply = |m: &[[f32; 4]; 4], p: [f32; 3]| -> [f32; 3] {
            [m[0][0]*p[0]+m[0][1]*p[1]+m[0][2]*p[2]+m[0][3],
             m[1][0]*p[0]+m[1][1]*p[1]+m[1][2]*p[2]+m[1][3],
             m[2][0]*p[0]+m[2][1]*p[1]+m[2][2]*p[2]+m[2][3]]
        };
        let centroid = |pts: &[[f32; 3]]| -> [f32; 3] {
            let n = pts.len() as f32;
            let mut c = [0f32; 3];
            for p in pts { for k in 0..3 { c[k] += p[k] / n; } }
            c
        };
        let bbox = |pts: &[[f32; 3]]| -> ([f32; 3], [f32; 3]) {
            let mut mn = [f32::MAX; 3]; let mut mx = [f32::MIN; 3];
            for p in pts { for k in 0..3 { mn[k] = mn[k].min(p[k]); mx[k] = mx[k].max(p[k]); } }
            (mn, mx)
        };

        for frame in [0usize, skel.frame_count() as usize / 2] {
            let world = world_at(frame);
            eprintln!("=== sta 帧 {frame} ===");
            // 游戏真实 LBS：Σ w·M_b·local_b
            let game_pos = |m: &FskinModel| -> Vec<[f32; 3]> {
                m.vertices.iter().map(|v| {
                    let mut o = [0f32; 3];
                    for inf in &v.influences {
                        let x = apply(&world[inf.bone_index as usize], inf.bone_local_pos);
                        for k in 0..3 { o[k] += inf.weight * x[k]; }
                    }
                    o
                }).collect()
            };
            let gh = game_pos(&head); let g5 = game_pos(&h5); let g4 = game_pos(&h4);
            let ch = centroid(&gh); let c5 = centroid(&g5); let c4 = centroid(&g4);
            let (mn5, mx5) = bbox(&g5);
            eprintln!("游戏LBS head 质心 {:?}", ch);
            eprintln!("游戏LBS hair05 质心 {:?} bbox {:?}..{:?}", c5, mn5, mx5);
            eprintln!("游戏LBS hair04 质心 {:?}（对照）", c4);
            eprintln!("游戏LBS 相对摆放 hair05-head {:?}  hair04-head {:?}", [c5[0]-ch[0],c5[1]-ch[1],c5[2]-ch[2]], [c4[0]-ch[0],c4[1]-ch[1],c4[2]-ch[2]]);
            let ah: Vec<[f32;3]> = head.vertices.iter().map(|v| v.pos).collect();
            let a5: Vec<[f32;3]> = h5.vertices.iter().map(|v| v.pos).collect();
            let a4: Vec<[f32;3]> = h4.vertices.iter().map(|v| v.pos).collect();
            let ach = centroid(&ah); let ac5 = centroid(&a5); let ac4 = centroid(&a4);
            eprintln!("authored 相对摆放 hair05-head {:?}  hair04-head {:?}", [ac5[0]-ach[0],ac5[1]-ach[1],ac5[2]-ach[2]], [ac4[0]-ach[0],ac4[1]-ach[1],ac4[2]-ach[2]]);
            eprintln!("游戏LBS 相对 authored 的整体平移: head {:?} hair05 {:?} hair04 {:?}",
                [ch[0]-ach[0],ch[1]-ach[1],ch[2]-ach[2]],
                [c5[0]-ac5[0],c5[1]-ac5[1],c5[2]-ac5[2]],
                [c4[0]-ac4[0],c4[1]-ac4[1],c4[2]-ac4[2]]);
            // 每骨 c_b（head 上的骨骼）：M_b·local − pos 的均值与波动
            let mut by_bone: std::collections::BTreeMap<u32, Vec<[f32;3]>> = Default::default();
            for v in &head.vertices {
                if v.influences.len() == 1 {
                    let inf = &v.influences[0];
                    let x = apply(&world[inf.bone_index as usize], inf.bone_local_pos);
                    by_bone.entry(inf.bone_index).or_default().push([x[0]-v.pos[0], x[1]-v.pos[1], x[2]-v.pos[2]]);
                }
            }
            for (b, cs) in &by_bone {
                let mean = centroid(cs);
                let spread = cs.iter().map(|c| ((c[0]-mean[0]).powi(2)+(c[1]-mean[1]).powi(2)+(c[2]-mean[2]).powi(2)).sqrt()).fold(0f32, f32::max);
                eprintln!("  head 骨 {b:#x}: c̄={:?} 波动 {:.3}", mean, spread);
            }
            let mut by_bone5: std::collections::BTreeMap<u32, Vec<[f32;3]>> = Default::default();
            for v in &h5.vertices {
                if v.influences.len() == 1 {
                    let inf = &v.influences[0];
                    let x = apply(&world[inf.bone_index as usize], inf.bone_local_pos);
                    by_bone5.entry(inf.bone_index).or_default().push([x[0]-v.pos[0], x[1]-v.pos[1], x[2]-v.pos[2]]);
                }
            }
            for (b, cs) in &by_bone5 {
                let mean = centroid(cs);
                let spread = cs.iter().map(|c| ((c[0]-mean[0]).powi(2)+(c[1]-mean[1]).powi(2)+(c[2]-mean[2]).powi(2)).sqrt()).fold(0f32, f32::max);
                eprintln!("  hair05 骨 {b:#x}: c̄={:?} 波动 {:.3} ({}点)", mean, spread, cs.len());
            }
        }
    }

    /// 诊断：ASCII 投影对比三种渲染：authored / 游戏LBS / viewer（共识+离群覆盖）
    #[test]
    fn hair05_ascii_views() {
        use crate::fskel;
        use std::collections::BTreeMap;
        let load = |rel: &str| parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap()).unwrap();
        let head = load("girl_head.fskin");
        let h1 = load("MHair/Girl_hair01.fskin");
        let h5 = load("MHair/Girl_hair05.fskin");

        let skel = fskel::get_or_load(&format!("{ROOT}/act/sta.FSKEL")).unwrap();
        let flat = skel.frames_flat(0, 1);
        let qm = |q: [f32; 4]| -> [[f32; 3]; 3] {
            let n = (q[0]*q[0]+q[1]*q[1]+q[2]*q[2]+q[3]*q[3]).sqrt();
            let (x, y, z, w) = (q[0]/n, q[1]/n, q[2]/n, q[3]/n);
            [
                [1.0-2.0*(y*y+z*z), 2.0*(x*y-z*w), 2.0*(x*z+y*w)],
                [2.0*(x*y+z*w), 1.0-2.0*(x*x+z*z), 2.0*(y*z-x*w)],
                [2.0*(x*z-y*w), 2.0*(y*z+x*w), 1.0-2.0*(x*x+y*y)],
            ]
        };
        let mut world = vec![[[0f32; 4]; 4]; skel.parents.len()];
        for b in 0..skel.parents.len() {
            let o = b * 7;
            let r = qm([flat[o+3], flat[o+4], flat[o+5], flat[o+6]]);
            let mut m = [[0f32; 4]; 4];
            for i in 0..3 { for j in 0..3 { m[i][j] = r[i][j]; } m[i][3] = flat[o+i]; }
            m[3][3] = 1.0;
            let p = skel.parents[b];
            if p >= 0 {
                let pm = world[p as usize];
                let mut w = [[0f32; 4]; 4];
                for i in 0..4 { for j in 0..4 { w[i][j] = (0..4).map(|k| pm[i][k]*m[k][j]).sum(); } }
                world[b] = w;
            } else { world[b] = m; }
        }
        let ap = |m: &[[f32; 4]; 4], p: [f32; 3]| -> [f32; 3] {
            [m[0][0]*p[0]+m[0][1]*p[1]+m[0][2]*p[2]+m[0][3],
             m[1][0]*p[0]+m[1][1]*p[1]+m[1][2]*p[2]+m[1][3],
             m[2][0]*p[0]+m[2][1]*p[1]+m[2][2]*p[2]+m[2][3]]
        };
        // 3x3 工具
        fn det3(m: &[f32; 9]) -> f32 {
            m[0]*(m[4]*m[8]-m[5]*m[7]) - m[1]*(m[3]*m[8]-m[5]*m[6]) + m[2]*(m[3]*m[7]-m[4]*m[6])
        }
        fn inv3(m: &[f32; 9]) -> Option<[f32; 9]> {
            let d = det3(m);
            if d.abs() < 1e-12 { return None; }
            let d = 1.0/d;
            Some([
                (m[4]*m[8]-m[5]*m[7])*d, (m[2]*m[7]-m[1]*m[8])*d, (m[1]*m[5]-m[2]*m[4])*d,
                (m[5]*m[6]-m[3]*m[8])*d, (m[0]*m[8]-m[2]*m[6])*d, (m[2]*m[3]-m[0]*m[5])*d,
                (m[3]*m[7]-m[4]*m[6])*d, (m[1]*m[6]-m[0]*m[7])*d, (m[0]*m[4]-m[1]*m[3])*d,
            ])
        }
        // Kabsch: (local, pos) 点对 → (R, t)，pos ≈ R·local + t
        let kabsch = |pairs: &[([f32; 3], [f32; 3])]| -> ([f32; 9], [f32; 3]) {
            let n = pairs.len() as f32;
            let mut ca = [0f32; 3]; let mut cb = [0f32; 3];
            for (a, b) in pairs { for k in 0..3 { ca[k] += a[k]/n; cb[k] += b[k]/n; } }
            let mut h = [0f32; 9];
            for (a, b) in pairs { for r in 0..3 { for c in 0..3 { h[r*3+c] += (b[r]-cb[r])*(a[c]-ca[c]); } } }
            let mut m = h;
            for _ in 0..32 {
                let mi = inv3(&m).unwrap();
                let mut next = [0f32; 9]; let mut delta = 0f32;
                for r in 0..3 { for c in 0..3 {
                    let v = (m[r*3+c] + mi[c*3+r]) / 2.0;
                    next[r*3+c] = v; delta = delta.max((v - m[r*3+c]).abs());
                } }
                m = next;
                if delta < 1e-10 { break; }
            }
            let t = [cb[0]-(m[0]*ca[0]+m[1]*ca[1]+m[2]*ca[2]),
                     cb[1]-(m[3]*ca[0]+m[4]*ca[1]+m[5]*ca[2]),
                     cb[2]-(m[6]*ca[0]+m[7]*ca[1]+m[8]*ca[2])];
            (m, t)
        };
        // 头部各骨共识拟合（viewer 的 boneInverse 来源；exact 优先，不足补 mixed，
        // 近似真实 viewer 装全部部件时的行为——测试仅装头/发型）
        let mut tc: BTreeMap<u32, ([f32; 9], [f32; 3])> = BTreeMap::new();
        {
            let mut exact: BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> = BTreeMap::new();
            let mut approx: BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> = BTreeMap::new();
            for v in &head.vertices {
                for inf in &v.influences {
                    if v.influences.len() == 1 || inf.weight >= 0.99 {
                        exact.entry(inf.bone_index).or_default().push((inf.bone_local_pos, v.pos));
                    } else {
                        approx.entry(inf.bone_index).or_default().push((inf.bone_local_pos, v.pos));
                    }
                }
            }
            let bones: Vec<u32> = exact.keys().chain(approx.keys()).copied().collect();
            for b in bones {
                let pts = exact.get(&b);
                let pool: Vec<_> = match pts {
                    Some(p) if p.len() >= 3 => p.clone(),
                    _ => pts.map(|p| p.clone()).unwrap_or_default()
                        .into_iter()
                        .chain(approx.get(&b).cloned().unwrap_or_default())
                        .collect(),
                };
                if pool.len() >= 2 { tc.insert(b, kabsch(&pool)); }
            }
        }
        // T⁻¹·p = Rᵀ·(p−t)
        let inv_ap = |rt: &([f32; 9], [f32; 3]), p: [f32; 3]| -> [f32; 3] {
            let (r, t) = rt;
            let d = [p[0]-t[0], p[1]-t[1], p[2]-t[2]];
            [r[0]*d[0]+r[3]*d[1]+r[6]*d[2], r[1]*d[0]+r[4]*d[1]+r[7]*d[2], r[2]*d[0]+r[5]*d[1]+r[8]*d[2]]
        };
        let diff_rt = |a: &([f32; 9], [f32; 3]), b: &([f32; 9], [f32; 3])| -> f32 {
            let dr = a.0.iter().zip(b.0.iter()).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
            let dt = ((a.1[0]-b.1[0]).powi(2) + (a.1[1]-b.1[1]).powi(2) + (a.1[2]-b.1[2]).powi(2)).sqrt();
            dr.max(dt)
        };

        // 三种渲染的顶点位置
        let authored = |m: &FskinModel| -> Vec<[f32; 3]> { m.vertices.iter().map(|v| v.pos).collect() };
        let game = |m: &FskinModel| -> Vec<[f32; 3]> {
            m.vertices.iter().map(|v| {
                let mut o = [0f32; 3];
                for inf in &v.influences {
                    let x = ap(&world[inf.bone_index as usize], inf.bone_local_pos);
                    for k in 0..3 { o[k] += inf.weight * x[k]; }
                }
                o
            }).collect()
        };
        // viewer（修复后）：每部件每骨绑定 = 共识，自身拟合（≥3 单影响点）
        // 与共识差异 >0.1 时覆盖（hair05×0x6 即此情形）
        let viewer = |m: &FskinModel| -> Vec<[f32; 3]> {
            let mut own: BTreeMap<u32, ([f32; 9], [f32; 3])> = BTreeMap::new();
            let mut by: BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> = BTreeMap::new();
            for v in &m.vertices {
                if v.influences.len() == 1 {
                    by.entry(v.influences[0].bone_index).or_default()
                        .push((v.influences[0].bone_local_pos, v.pos));
                }
            }
            for (b, pts) in by { if pts.len() >= 3 { own.insert(b, kabsch(&pts)); } }
            m.vertices.iter().map(|v| {
                let mut o = [0f32; 3];
                for inf in &v.influences {
                    let cons = tc.get(&inf.bone_index);
                    let ov = own.get(&inf.bone_index);
                    let sel = match (cons, ov) {
                        (Some(c), Some(o)) if diff_rt(o, c) > 0.1 => Some(o),
                        (c, _) => c,
                    };
                    let x = match sel {
                        Some(rt) => ap(&world[inf.bone_index as usize], inv_ap(rt, v.pos)),
                        None => v.pos,
                    };
                    for k in 0..3 { o[k] += inf.weight * x[k]; }
                }
                o
            }).collect()
        };

        // 参考中心：authored 用原点；game/consensus 用 head 质心对齐
        let centroid = |pts: &[[f32; 3]]| -> [f32; 3] {
            let n = pts.len() as f32;
            let mut c = [0f32; 3];
            for p in pts { for k in 0..3 { c[k] += p[k]/n; } }
            c
        };

        // ASCII 光栅化（部件叠加：head '.' hair01 ',' hair05 '@'）
        let render = |title: &str, layers: &[(&FskinModel, Vec<[f32; 3]>, char, [f32; 3])]| {
            let w = 100usize;
            // 收集范围
            let (mut mnx, mut mxx, mut mny, mut mxy) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for (_, pts, _, _) in layers {
                for p in pts {
                    mnx = mnx.min(p[0]); mxx = mxx.max(p[0]);
                    mny = mny.min(p[1]); mxy = mxy.max(p[1]);
                }
            }
            let h = ((mxy-mny)/(mxx-mnx) * w as f32 / 2.2).ceil().max(10.0) as usize;
            let mut grid = vec![vec![' '; w]; h];
            let sx = |x: f32| ((x - mnx) / (mxx - mnx) * (w as f32 - 1.0)).round() as usize;
            let sy = |y: f32| ((mxy - y) / (mxy - mny) * (h as f32 - 1.0)).round() as usize;
            for (m, pts, ch, center) in layers {
                let _ = center;
                let idx = m.triangles_merged();
                for t in idx.chunks(3) {
                    let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
                    // 重心采样填充
                    for i in 0..=10usize { for j in 0..=(10-i) {
                        let (u, v) = (i as f32/10.0, j as f32/10.0);
                        let w0 = 1.0-u-v;
                        let x = w0*a[0]+u*b[0]+v*c[0];
                        let y = w0*a[1]+u*b[1]+v*c[1];
                        let (gx, gy) = (sx(x), sy(y));
                        if gx < w && gy < h { grid[gy][gx] = *ch; }
                    } }
                }
            }
            eprintln!("---- {title} (x∈[{mnx:.0},{mxx:.0}] y∈[{mny:.0},{mxy:.0}]) ----");
            for row in &grid { eprintln!("{}", row.iter().collect::<String>()); }
        };

        let parts = [&head, &h1, &h5];
        let ch_of = |i: usize| if i == 0 {'.'} else if i == 1 {','} else {'@'};
        // A: authored
        render("A authored 静态(头'.' hair01',' hair05'@')", &parts.iter().enumerate().map(|(i, m)| {
            (*m, authored(m), ch_of(i), [0.0;3])
        }).collect::<Vec<_>>());
        // B: game-true（对齐 head 质心到 authored head 质心）
        let ah: Vec<[f32;3]> = head.vertices.iter().map(|v| v.pos).collect();
        let ch_auth = centroid(&ah);
        let mut layers_b: Vec<(&FskinModel, Vec<[f32;3]>, char, [f32;3])> = Vec::new();
        let mut head_c = None;
        for (i, m) in parts.iter().enumerate() {
            let g = game(m);
            if i == 0 { head_c = Some(centroid(&g)); }
            layers_b.push((m, g, ch_of(i), [0.0;3]));
        }
        let hc = head_c.unwrap();
        let shift = [ch_auth[0]-hc[0], ch_auth[1]-hc[1], ch_auth[2]-hc[2]];
        for (_, pts, _, _) in &mut layers_b { for p in pts.iter_mut() { for k in 0..3 { p[k] += shift[k]; } } }
        render("B 游戏真实LBS(sta帧0, 头质心对齐)", &layers_b);
        // C: viewer 共识
        let mut layers_c: Vec<(&FskinModel, Vec<[f32;3]>, char, [f32;3])> = Vec::new();
        let mut head_c2 = None;
        for (i, m) in parts.iter().enumerate() {
            let g = viewer(m);
            if i == 0 { head_c2 = Some(centroid(&g)); }
            layers_c.push((m, g, ch_of(i), [0.0;3]));
        }
        let hc2 = head_c2.unwrap();
        let shift2 = [ch_auth[0]-hc2[0], ch_auth[1]-hc2[1], ch_auth[2]-hc2[2]];
        for (_, pts, _, _) in &mut layers_c { for p in pts.iter_mut() { for k in 0..3 { p[k] += shift2[k]; } } }
        render("C viewer蒙皮(共识+离群覆盖,sta帧0, 头质心对齐)", &layers_c);
    }

    /// 回归：同一骨骼上各部件的 (boneLocalPos→pos) 刚体变换一致性。
    /// Girl_hair05 与头部共用 0x6 骨但绑定变换不同（制作时的偏移差异，
    /// 其 authored pos 仍端正贴在头部 —— 见 hair05_deep_diag 的 bbox），
    /// 因此 viewer 的蒙皮绑定矩阵按"共识"拟合：同骨取相互一致的多数部件
    /// 的变换，离群部件（hair05）的差异按刚性附件随骨骼运动，保证贴合；
    /// 本测试用 Kabsch 拟合固化"除 hair05 外与头部一致、hair05 必须不同"
    /// 这一数据事实，为共识分簇（阈值 0.1）提供依据。
    #[test]
    fn hair_bind_offset_consistency() {
        fn det3(m: &[f32; 9]) -> f32 {
            m[0] * (m[4] * m[8] - m[5] * m[7])
                - m[1] * (m[3] * m[8] - m[5] * m[6])
                + m[2] * (m[3] * m[7] - m[4] * m[6])
        }
        fn inv3(m: &[f32; 9]) -> Option<[f32; 9]> {
            let d = det3(m);
            if d.abs() < 1e-12 {
                return None;
            }
            let d = 1.0 / d;
            Some([
                (m[4] * m[8] - m[5] * m[7]) * d,
                (m[2] * m[7] - m[1] * m[8]) * d,
                (m[1] * m[5] - m[2] * m[4]) * d,
                (m[5] * m[6] - m[3] * m[8]) * d,
                (m[0] * m[8] - m[2] * m[6]) * d,
                (m[2] * m[3] - m[0] * m[5]) * d,
                (m[3] * m[7] - m[4] * m[6]) * d,
                (m[1] * m[6] - m[0] * m[7]) * d,
                (m[0] * m[4] - m[1] * m[3]) * d,
            ])
        }
        /// Kabsch 刚体拟合：返回 (R 3x3, t)
        fn kabsch(pairs: &[([f32; 3], [f32; 3])]) -> ([f32; 9], [f32; 3]) {
            let n = pairs.len() as f32;
            let mut ca = [0f32; 3];
            let mut cb = [0f32; 3];
            for (a, b) in pairs {
                for k in 0..3 {
                    ca[k] += a[k] / n;
                    cb[k] += b[k] / n;
                }
            }
            let mut h = [0f32; 9];
            for (a, b) in pairs {
                for r in 0..3 {
                    for c in 0..3 {
                        h[r * 3 + c] += (b[r] - cb[r]) * (a[c] - ca[c]);
                    }
                }
            }
            // 极分解 Newton 迭代
            let mut m = h;
            let mut converged = false;
            for _ in 0..32 {
                let Some(mi) = inv3(&m) else {
                    break;
                };
                let mut next = [0f32; 9];
                let mut delta = 0f32;
                for r in 0..3 {
                    for c in 0..3 {
                        let v = (m[r * 3 + c] + mi[c * 3 + r]) / 2.0;
                        next[r * 3 + c] = v;
                        delta = delta.max((v - m[r * 3 + c]).abs());
                    }
                }
                m = next;
                if delta < 1e-10 {
                    converged = true;
                    break;
                }
            }
            // 与 viewer.ts polarRotation 一致的退化回退：未收敛（秩亏点组会把
            // 迭代吹爆）或落入反射时退化为仅平移
            let ortho_err = |mm: &[f32; 9]| -> f32 {
                let mut e = 0f32;
                for r in 0..3 {
                    for c in 0..3 {
                        let dot: f32 = (0..3).map(|k| mm[r * 3 + k] * mm[c * 3 + k]).sum();
                        e = e.max((dot - if r == c { 1.0 } else { 0.0 }).abs());
                    }
                }
                e
            };
            if !converged || det3(&m) <= 0.5 || ortho_err(&m) > 0.01 {
                return (
                    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]],
                );
            }
            let t = [
                cb[0] - (m[0] * ca[0] + m[1] * ca[1] + m[2] * ca[2]),
                cb[1] - (m[3] * ca[0] + m[4] * ca[1] + m[5] * ca[2]),
                cb[2] - (m[6] * ca[0] + m[7] * ca[1] + m[8] * ca[2]),
            ];
            (m, t)
        }
        /// 变换差异：R 用逐元素差、t 用距离
        fn diff((ra, ta): &([f32; 9], [f32; 3]), (rb, tb): &([f32; 9], [f32; 3])) -> f32 {
            let dr: f32 = ra.iter().zip(rb.iter()).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
            let dt = ((ta[0] - tb[0]).powi(2) + (ta[1] - tb[1]).powi(2) + (ta[2] - tb[2]).powi(2)).sqrt();
            dr.max(dt)
        }

        let files = [
            ("girl_head.fskin", true),
            ("MHair/Girl_hair01.fskin", true),
            ("MHair/Girl_hair02.fskin", true),
            ("MHair/Girl_hair03.fskin", true),
            ("MHair/Girl_hair04.fskin", true),
            ("MHair/Girl_hair05.fskin", false),
            ("MHair/Girl_hair06.fskin", true),
            ("MHair/Girl_hair07.fskin", true),
        ];
        // 头部 0x6 骨的单影响点对 → 基准变换
        let head = parse_fskin(
            &format!("{ROOT}/girl_head.fskin"),
            &fs::read(format!("{ROOT}/girl_head.fskin")).unwrap(),
        )
        .unwrap();
        let head_pairs: Vec<([f32; 3], [f32; 3])> = head
            .vertices
            .iter()
            .filter(|v| v.influences.len() == 1 && v.influences[0].bone_index == 0x6)
            .map(|v| (v.influences[0].bone_local_pos, v.pos))
            .collect();
        assert!(head_pairs.len() > 100, "头部 0x6 单影响点应充足");
        let base = kabsch(&head_pairs);

        for (f, same_as_head) in files {
            let p = format!("{ROOT}/{f}");
            let m = parse_fskin(&p, &fs::read(&p).unwrap()).unwrap();
            let pairs: Vec<([f32; 3], [f32; 3])> = m
                .vertices
                .iter()
                .filter(|v| v.influences.len() == 1 && v.influences[0].bone_index == 0x6)
                .map(|v| (v.influences[0].bone_local_pos, v.pos))
                .collect();
            assert!(pairs.len() > 50, "{f} 在 0x6 骨上应有充足单影响点");
            let t = kabsch(&pairs);
            let d = diff(&base, &t);
            eprintln!("{f:24} 与头部 0x6 绑定变换差异 = {d:.4}");
            if same_as_head {
                assert!(d < 1e-3, "{f} 应与头部共享 0x6 绑定变换（差异 {d}）");
            } else {
                assert!(d > 0.5, "Girl_hair05 的 0x6 绑定变换应与头部不同（差异 {d}）");
            }
        }
    }

    /// 验证"每部件自身绑定矩阵"渲染与游戏真实 LBS（Σ w·M_b·local_b）的吻合度，
    /// 并对照共识绑定的误差。hair05 与头部 0x6 绑定变换不同（见上），共识把
    /// 头部的变换套在 hair05 上会整体扭转部件；自身拟合则对单影响顶点精确
    /// 还原游戏 LBS（M_b·T⁻¹·pos = M_b·local）。
    #[test]
    fn hair05_per_part_bind() {
        use crate::fskel;

        let load = |rel: &str| {
            parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap())
                .unwrap()
        };
        // 全套装配（模拟 viewer 实际场景：默认套装 + 全部发型）
        let files = [
            "girl_head.fskin",
            "girl_Upper.fskin",
            "girl_Lower.fskin",
            "girl_Boot.fskin",
            "girl_Hair01.fskin",
            "MHair/Girl_hair01.fskin",
            "MHair/Girl_hair02.fskin",
            "MHair/Girl_hair03.fskin",
            "MHair/Girl_hair04.fskin",
            "MHair/Girl_hair05.fskin",
            "MHair/Girl_hair06.fskin",
            "MHair/Girl_hair07.fskin",
        ];
        let parts: Vec<FskinModel> = files.iter().map(|f| load(f)).collect();

        let qm = |q: [f32; 4]| -> [[f32; 4]; 4] {
            let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            let (x, y, z, w) = (q[0] / n, q[1] / n, q[2] / n, q[3] / n);
            let r = [
                [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
                [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
                [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
            ];
            let mut m = [[0f32; 4]; 4];
            for i in 0..3 {
                for j in 0..3 {
                    m[i][j] = r[i][j];
                }
            }
            m
        };
        let mul = |a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]| -> [[f32; 4]; 4] {
            let mut w = [[0f32; 4]; 4];
            for i in 0..4 {
                for j in 0..4 {
                    w[i][j] = (0..4).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            w
        };
        let ap = |m: &[[f32; 4]; 4], p: [f32; 3]| -> [f32; 3] {
            [
                m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
                m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
                m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
            ]
        };
        let world_at = |act: &str, frame: usize| -> Vec<[[f32; 4]; 4]> {
            let skel = fskel::get_or_load(&format!("{ROOT}/act/{act}")).unwrap();
            let frame = frame.min(skel.frame_count().saturating_sub(1) as usize);
            let flat = skel.frames_flat(frame as u32, frame as u32 + 1);
            let mut world = vec![[[0f32; 4]; 4]; skel.parents.len()];
            for b in 0..skel.parents.len() {
                let o = b * 7;
                let mut m = qm([flat[o + 3], flat[o + 4], flat[o + 5], flat[o + 6]]);
                m[0][3] = flat[o];
                m[1][3] = flat[o + 1];
                m[2][3] = flat[o + 2];
                m[3][3] = 1.0;
                let p = skel.parents[b];
                world[b] = if p >= 0 { mul(&world[p as usize], &m) } else { m };
            }
            world
        };

        // Kabsch：(local, pos) → (R, t)，pos ≈ R·local + t
        fn det3(m: &[f32; 9]) -> f32 {
            m[0] * (m[4] * m[8] - m[5] * m[7])
                - m[1] * (m[3] * m[8] - m[5] * m[6])
                + m[2] * (m[3] * m[7] - m[4] * m[6])
        }
        fn inv3(m: &[f32; 9]) -> Option<[f32; 9]> {
            let d = det3(m);
            if d.abs() < 1e-12 {
                return None;
            }
            let d = 1.0 / d;
            Some([
                (m[4] * m[8] - m[5] * m[7]) * d,
                (m[2] * m[7] - m[1] * m[8]) * d,
                (m[1] * m[5] - m[2] * m[4]) * d,
                (m[5] * m[6] - m[3] * m[8]) * d,
                (m[0] * m[8] - m[2] * m[6]) * d,
                (m[2] * m[3] - m[0] * m[5]) * d,
                (m[3] * m[7] - m[4] * m[6]) * d,
                (m[1] * m[6] - m[0] * m[7]) * d,
                (m[0] * m[4] - m[1] * m[3]) * d,
            ])
        }
        fn kabsch(pairs: &[([f32; 3], [f32; 3])]) -> ([f32; 9], [f32; 3]) {
            let n = pairs.len() as f32;
            let mut ca = [0f32; 3];
            let mut cb = [0f32; 3];
            for (a, b) in pairs {
                for k in 0..3 {
                    ca[k] += a[k] / n;
                    cb[k] += b[k] / n;
                }
            }
            let mut h = [0f32; 9];
            for (a, b) in pairs {
                for r in 0..3 {
                    for c in 0..3 {
                        h[r * 3 + c] += (b[r] - cb[r]) * (a[c] - ca[c]);
                    }
                }
            }
            let mut m = h;
            let mut converged = false;
            for _ in 0..32 {
                let Some(mi) = inv3(&m) else {
                    break;
                };
                let mut next = [0f32; 9];
                let mut delta = 0f32;
                for r in 0..3 {
                    for c in 0..3 {
                        let v = (m[r * 3 + c] + mi[c * 3 + r]) / 2.0;
                        next[r * 3 + c] = v;
                        delta = delta.max((v - m[r * 3 + c]).abs());
                    }
                }
                m = next;
                if delta < 1e-10 {
                    converged = true;
                    break;
                }
            }
            // 与 viewer.ts polarRotation 一致的退化回退：未收敛（秩亏点组会把
            // 迭代吹爆）或落入反射时退化为仅平移，防止离谱的大数矩阵
            let ortho_err = |mm: &[f32; 9]| -> f32 {
                let mut e = 0f32;
                for r in 0..3 {
                    for c in 0..3 {
                        let dot: f32 = (0..3).map(|k| mm[r * 3 + k] * mm[c * 3 + k]).sum();
                        e = e.max((dot - if r == c { 1.0 } else { 0.0 }).abs());
                    }
                }
                e
            };
            if !converged || det3(&m) <= 0.5 || ortho_err(&m) > 0.01 {
                return (
                    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                    [cb[0] - ca[0], cb[1] - ca[1], cb[2] - ca[2]],
                );
            }
            let t = [
                cb[0] - (m[0] * ca[0] + m[1] * ca[1] + m[2] * ca[2]),
                cb[1] - (m[3] * ca[0] + m[4] * ca[1] + m[5] * ca[2]),
                cb[2] - (m[6] * ca[0] + m[7] * ca[1] + m[8] * ca[2]),
            ];
            (m, t)
        }
        // T⁻¹·p（Rᵀ·(p−t)）
        let inv_ap = |rt: &([f32; 9], [f32; 3]), p: [f32; 3]| -> [f32; 3] {
            let (r, t) = rt;
            let d = [p[0] - t[0], p[1] - t[1], p[2] - t[2]];
            [
                r[0] * d[0] + r[3] * d[1] + r[6] * d[2],
                r[1] * d[0] + r[4] * d[1] + r[7] * d[2],
                r[2] * d[0] + r[5] * d[1] + r[8] * d[2],
            ]
        };

        // 每部件每骨自身拟合（exact 单影响点；<3 个补 mixed 点，同 viewer 逻辑）
        let own_fit = |m: &FskinModel| -> std::collections::BTreeMap<u32, ([f32; 9], [f32; 3])> {
            let mut exact: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                Default::default();
            let mut approx: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                Default::default();
            for v in &m.vertices {
                for inf in &v.influences {
                    let pair = (inf.bone_local_pos, v.pos);
                    if v.influences.len() == 1 || inf.weight >= 0.99 {
                        exact.entry(inf.bone_index).or_default().push(pair);
                    } else {
                        approx.entry(inf.bone_index).or_default().push(pair);
                    }
                }
            }
            let mut out: std::collections::BTreeMap<u32, ([f32; 9], [f32; 3])> = Default::default();
            for (b, pts) in exact {
                let mut pool = pts.clone();
                if pool.len() < 3 {
                    pool.extend(approx.get(&b).map(|v| v.clone()).unwrap_or_default());
                }
                if pool.len() >= 2 {
                    out.insert(b, kabsch(&pool));
                }
            }
            out
        };
        // 共识拟合：忠实模拟 viewer.refreshBindInverses —— 每骨各部件自身拟合
        // 两两分簇（阈值 0.1），取总点数最多的簇合并拟合；无 exact 的部件
        // 不投票；pool 不足 3 点时并入全部 approx 点
        let consensus = |all: &[FskinModel]| -> std::collections::BTreeMap<u32, ([f32; 9], [f32; 3])> {
            let mut bone_set = std::collections::BTreeSet::new();
            let mut exact_of: Vec<std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>>> =
                Vec::new();
            let mut approx_all: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                Default::default();
            for m in all {
                let mut exact: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                    Default::default();
                let mut approx: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                    Default::default();
                for v in &m.vertices {
                    for inf in &v.influences {
                        let pair = (inf.bone_local_pos, v.pos);
                        if v.influences.len() == 1 || inf.weight >= 0.99 {
                            exact.entry(inf.bone_index).or_default().push(pair);
                        } else {
                            approx.entry(inf.bone_index).or_default().push(pair);
                        }
                    }
                }
                for b in exact.keys().chain(approx.keys()) {
                    bone_set.insert(*b);
                }
                for (b, pts) in approx {
                    approx_all.entry(b).or_default().extend(pts);
                }
                exact_of.push(exact);
            }
            let diff_rt = |a: &([f32; 9], [f32; 3]), b: &([f32; 9], [f32; 3])| -> f32 {
                let dr = a.0.iter().zip(b.0.iter()).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
                let dt = ((a.1[0] - b.1[0]).powi(2)
                    + (a.1[1] - b.1[1]).powi(2)
                    + (a.1[2] - b.1[2]).powi(2))
                .sqrt();
                dr.max(dt)
            };
            let mut out: std::collections::BTreeMap<u32, ([f32; 9], [f32; 3])> = Default::default();
            for b in bone_set {
                // 该骨上有 exact 点的部件 → 各自身拟合 → 一致性分簇取最大簇
                let lists: Vec<&Vec<([f32; 3], [f32; 3])>> = exact_of
                    .iter()
                    .filter_map(|e| e.get(&b))
                    .collect();
                let fits: Vec<([f32; 9], [f32; 3])> = lists.iter().map(|p| kabsch(p)).collect();
                let best: Vec<usize> = if fits.len() >= 2 {
                    let ok = |i: usize, j: usize| diff_rt(&fits[i], &fits[j]) < 0.1;
                    let mut comps: Vec<Vec<usize>> = Vec::new();
                    let mut seen = vec![false; fits.len()];
                    for i in 0..fits.len() {
                        if seen[i] {
                            continue;
                        }
                        let mut comp = vec![i];
                        seen[i] = true;
                        let mut changed = true;
                        while changed {
                            changed = false;
                            for j in 0..fits.len() {
                                if !seen[j] && comp.iter().any(|&c| ok(c, j)) {
                                    comp.push(j);
                                    seen[j] = true;
                                    changed = true;
                                }
                            }
                        }
                        comps.push(comp);
                    }
                    comps
                        .into_iter()
                        .max_by_key(|c| c.iter().map(|&i| lists[i].len()).sum::<usize>())
                        .unwrap()
                } else {
                    (0..fits.len()).collect()
                };
                let mut pool: Vec<([f32; 3], [f32; 3])> = Vec::new();
                for &i in &best {
                    pool.extend(lists[i].clone());
                }
                if pool.len() < 3 {
                    pool.extend(approx_all.get(&b).cloned().unwrap_or_default());
                }
                if pool.len() >= 2 {
                    out.insert(b, kabsch(&pool));
                }
            }
            out
        };
        let cons_fit = consensus(&parts);

        // 诊断：各部件每骨 exact/approx 点数与拟合残差
        for (i, m) in parts.iter().enumerate() {
            let mut exact: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                Default::default();
            let mut approx: std::collections::BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> =
                Default::default();
            for v in &m.vertices {
                for inf in &v.influences {
                    let pair = (inf.bone_local_pos, v.pos);
                    if v.influences.len() == 1 || inf.weight >= 0.99 {
                        exact.entry(inf.bone_index).or_default().push(pair);
                    } else {
                        approx.entry(inf.bone_index).or_default().push(pair);
                    }
                }
            }
            let mut bones: Vec<u32> = exact.keys().chain(approx.keys()).copied().collect();
            bones.sort_unstable();
            bones.dedup();
            let line: Vec<String> = bones
                .iter()
                .map(|b| {
                    let e = exact.get(b).map(|v| v.len()).unwrap_or(0);
                    let a = approx.get(b).map(|v| v.len()).unwrap_or(0);
                    let res = if e >= 3 {
                        let (r, tt) = kabsch(&exact[b]);
                        exact[b]
                            .iter()
                            .map(|(l, p)| {
                                let q = [
                                    r[0] * l[0] + r[1] * l[1] + r[2] * l[2] + tt[0],
                                    r[3] * l[0] + r[4] * l[1] + r[5] * l[2] + tt[1],
                                    r[6] * l[0] + r[7] * l[1] + r[8] * l[2] + tt[2],
                                ];
                                ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2))
                                    .sqrt()
                            })
                            .fold(0f32, f32::max)
                    } else {
                        -1.0
                    };
                    format!("0x{b:x}:e{e}/a{a} res{res:.2}")
                })
                .collect();
            eprintln!("{:24} {}", files[i], line.join("  "));
        }

        for (act, frame) in [
            ("sta.FSKEL", 0usize),
            ("sta.FSKEL", 5),
            ("rwk1.FSKEL", 0),
            ("rwk1.FSKEL", 8),
            ("sit1.FSKEL", 0),
            ("lwk1.FSKEL", 0),
            ("lwk1.FSKEL", 7),
        ] {
            let world = world_at(act, frame);
            for (i, m) in parts.iter().enumerate() {
                let fit = own_fit(m);
                let mut own_max = 0f32;
                let mut cons_max = 0f32;
                for v in &m.vertices {
                    let mut game = [0f32; 3];
                    let mut own = [0f32; 3];
                    let mut cons = [0f32; 3];
                    for inf in &v.influences {
                        let mb = &world[inf.bone_index as usize];
                        let g = ap(mb, inf.bone_local_pos);
                        // viewer：共识为底，自身拟合差异 >0.1 时覆盖
                        let cons_rt = cons_fit.get(&inf.bone_index);
                        let own_rt = fit.get(&inf.bone_index);
                        let use_own = match (cons_rt, own_rt) {
                            (Some(c), Some(o)) => {
                                let dr = c.0.iter().zip(o.0.iter()).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
                                let dt = ((c.1[0]-o.1[0]).powi(2) + (c.1[1]-o.1[1]).powi(2) + (c.1[2]-o.1[2]).powi(2)).sqrt();
                                dr.max(dt) > 0.1
                            }
                            (None, Some(_)) => true,
                            _ => false,
                        };
                        let o = match if use_own { own_rt } else { cons_rt } {
                            Some(rt) => ap(mb, inv_ap(rt, v.pos)),
                            None => v.pos,
                        };
                        let c = match cons_fit.get(&inf.bone_index) {
                            Some(rt) => ap(mb, inv_ap(rt, v.pos)),
                            None => v.pos,
                        };
                        for k in 0..3 {
                            game[k] += inf.weight * g[k];
                            own[k] += inf.weight * o[k];
                            cons[k] += inf.weight * c[k];
                        }
                    }
                    let dist = |a: [f32; 3], b: [f32; 3]| {
                        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2))
                            .sqrt()
                    };
                    own_max = own_max.max(dist(game, own));
                    cons_max = cons_max.max(dist(game, cons));
                }
                eprintln!("{act} 帧{frame:2} {:-24} viewer(共识+离群覆盖)误差 {own_max:7.3}  纯共识误差 {cons_max:7.3}", files[i]);
                if files[i].contains("hair05") {
                    assert!(
                        own_max < 0.01,
                        "hair05 离群覆盖后应与游戏 LBS 一致（{act} 帧{frame} 误差 {own_max}）"
                    );
                }
            }
        }
    }

    /// 导出 girl_Lower 顶点 + girl.FSKEL 前 90 帧姿态 + 骨名/父表，
    /// 供 ui 侧 verify-skirt-cloth.mjs 复刻 viewer 布料模拟数值验证。
    #[test]
    fn dump_skirt_json() {
        use crate::fskel;
        #[derive(serde::Serialize)]
        struct Vtx {
            pos: [f32; 3],
            influences: Vec<(u32, f32, [f32; 3])>,
        }
        #[derive(serde::Serialize)]
        struct Dump {
            bone_count: usize,
            bone_names: Vec<String>,
            parents: Vec<i32>,
            /// 连续帧姿态（帧 × 骨 × 7）
            poses: Vec<f32>,
            lower: Vec<Vtx>,
        }
        let lower = parse_fskin(
            &format!("{ROOT}/girl_Lower.fskin"),
            &fs::read(format!("{ROOT}/girl_Lower.fskin")).unwrap(),
        )
        .unwrap();
        let skel = fskel::get_or_load(&format!("{ROOT}/act/girl.FSKEL")).unwrap();
        let frames = skel.frame_count().min(90);
        let dump = Dump {
            bone_count: skel.parents.len(),
            bone_names: skel.bone_names.clone(),
            parents: skel.parents.clone(),
            poses: skel.frames_flat(0, frames),
            lower: lower
                .vertices
                .iter()
                .map(|v| Vtx {
                    pos: v.pos,
                    influences: v
                        .influences
                        .iter()
                        .map(|i| (i.bone_index, i.weight, i.bone_local_pos))
                        .collect(),
                })
                .collect(),
        };
        let out = format!("{}/../target/skirt_check.json", env!("CARGO_MANIFEST_DIR"));
        fs::write(&out, serde_json::to_string(&dump).unwrap()).unwrap();
        eprintln!("dumped {out} ({}帧 {}顶点)", frames, dump.lower.len());
    }

    /// 导出 head/hair05 顶点与 sta.FSKEL 首帧姿态到 JSON，
    /// 供 ui 侧 verify-hair05 脚本用真实 three.js 蒙皮管线验证。
    #[test]
    fn dump_hair05_json() {
        use crate::fskel;
        #[derive(serde::Serialize)]
        struct Vtx {
            pos: [f32; 3],
            influences: Vec<(u32, f32, [f32; 3])>,
        }
        #[derive(serde::Serialize)]
        struct Dump {
            bone_count: usize,
            parents: Vec<i32>,
            poses: Vec<f32>,
            head: Vec<Vtx>,
            hair05: Vec<Vtx>,
            all: Vec<(String, Vec<Vtx>)>,
        }
        let load = |rel: &str| {
            parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap())
                .unwrap()
        };
        let conv = |m: &FskinModel| -> Vec<Vtx> {
            m.vertices
                .iter()
                .map(|v| Vtx {
                    pos: v.pos,
                    influences: v
                        .influences
                        .iter()
                        .map(|i| (i.bone_index, i.weight, i.bone_local_pos))
                        .collect(),
                })
                .collect()
        };
        let skel = fskel::get_or_load(&format!("{ROOT}/act/sta.FSKEL")).unwrap();
        let dump = Dump {
            bone_count: skel.parents.len(),
            parents: skel.parents.clone(),
            poses: skel.frames_flat(0, 1),
            head: conv(&load("girl_head.fskin")),
            hair05: conv(&load("MHair/Girl_hair05.fskin")),
            all: [
                "girl_head.fskin",
                "girl_Upper.fskin",
                "girl_Lower.fskin",
                "girl_Boot.fskin",
                "girl_Hair01.fskin",
                "MHair/Girl_hair01.fskin",
                "MHair/Girl_hair02.fskin",
                "MHair/Girl_hair03.fskin",
                "MHair/Girl_hair04.fskin",
                "MHair/Girl_hair05.fskin",
                "MHair/Girl_hair06.fskin",
                "MHair/Girl_hair07.fskin",
            ]
            .iter()
            .map(|f| (f.to_string(), conv(&load(f))))
            .collect(),
        };
        let out = format!("{}/../target/hair05_check.json", env!("CARGO_MANIFEST_DIR"));
        fs::write(&out, serde_json::to_string(&dump).unwrap()).unwrap();
        eprintln!("dumped {out}");
    }

    /// 头发替换槽与纹理切换槽（TFace/TFaceDetail）
    #[test]
    fn slots_and_tex_slots() {
        let set = scan_model_set(ROOT).unwrap();
        // 头发：girl_Hair01 组 + 6 个附加发型组都标记 hair 槽
        let hair_groups = set
            .groups
            .iter()
            .filter(|g| g.slot == "hair")
            .collect::<Vec<_>>();
        assert_eq!(hair_groups.len(), 7, "应有 7 个头发槽组");
        assert!(set.groups.iter().any(|g| g.slot.is_empty()));

        // 纹理槽：脸部纹理（含"原始"）+ 饰品（叠层）
        let face = set.tex_slots.iter().find(|s| s.key == "face").unwrap();
        assert!(!face.overlay);
        assert_eq!(face.variants.len(), 11, "原始 + wq01..10");
        assert_eq!(face.variants[0].label, "原始");
        assert!(face.part.ends_with("girl_head.fskin"));

        let detail = set
            .tex_slots
            .iter()
            .find(|s| s.key == "face_detail")
            .unwrap();
        assert!(detail.overlay);
        assert_eq!(detail.variants.len(), 11, "xq01..11");
        assert_eq!(detail.part, face.part, "饰品与脸部作用于同一头部部件");
    }

    /// 衣服切换槽：YiFu/ 换装目录的上衣/下装/鞋子与根目录基础件构成互斥槽
    #[test]
    fn clothing_slots() {
        let set = scan_model_set(ROOT).unwrap();
        for (slot, base, prefix) in [
            ("upper", "girl_Upper.fskin", "girl_Upper_"),
            ("lower", "girl_Lower.fskin", "girl_Lower_"),
            ("boot", "girl_Boot.fskin", "girl_boot_"),
        ] {
            let groups: Vec<_> = set.groups.iter().filter(|g| g.slot == slot).collect();
            assert!(!groups.is_empty(), "{slot} 槽缺失");
            let root_parts: Vec<_> = groups
                .iter()
                .flat_map(|g| g.parts.iter().filter(|p| p.in_root))
                .collect();
            assert_eq!(root_parts.len(), 1, "{slot} 槽应有唯一根目录基础件");
            assert!(root_parts[0].file_name.eq_ignore_ascii_case(base));
            for i in 1..=3 {
                let name = format!("{prefix}{i}.fskin");
                let p = groups
                    .iter()
                    .flat_map(|g| g.parts.iter())
                    .find(|p| p.file_name.eq_ignore_ascii_case(&name))
                    .unwrap_or_else(|| panic!("{name} 未入 {slot} 槽"));
                assert!(p.texture_file.is_some(), "{} 贴图未解析", p.file_name);
            }
        }
    }

    /// 开发辅助：打印分组结构（几何签名 vs 纹理）
    #[test]
    fn dump_model_structure() {
        let set = scan_model_set(ROOT).unwrap();
        for g in &set.groups {
            let has_root = g.parts.iter().any(|p| p.in_root);
            eprintln!(
                "== GROUP [{}] sig={} {}",
                g.label,
                g.signature,
                if has_root { "根" } else { "附加" }
            );
            for p in &g.parts {
                let tex_short = p
                    .texture_file
                    .as_deref()
                    .unwrap_or("?")
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or("?")
                    .to_string();
                eprintln!(
                    "   {:22} v={:5} t={:5} rel={:8} tex={:?} -> {}",
                    p.file_name, p.vertex_count, p.triangle_count, p.rel_dir, p.texture_path, tex_short
                );
            }
        }
    }

    #[test]
    fn roundtrip_all_fskins() {
        let files = collect_fskins();
        if files.is_empty() {
            eprintln!("跳过: 测试目录不存在");
            return;
        }
        assert!(files.len() >= 20, "应找到 20+ 个 fskin, 实际 {}", files.len());
        for f in &files {
            let bytes = fs::read(f).unwrap();
            let m = parse_fskin(&f.to_string_lossy(), &bytes)
                .unwrap_or_else(|e| panic!("解析 {} 失败: {e}", f.display()));
            let out = serialize_fskin(&m).unwrap();
            assert_eq!(
                bytes,
                out,
                "往返不一致: {}",
                f.display()
            );
        }
        eprintln!("roundtrip OK on {} files", files.len());
    }

    #[test]
    fn parse_girl_boot() {
        let p = format!(r"{}\girl_Boot.fskin", ROOT);
        let Ok(bytes) = fs::read(&p) else {
            eprintln!("跳过: 文件不存在");
            return;
        };
        let m = parse_fskin(&p, &bytes).unwrap();
        assert_eq!(m.vertices.len(), 198);
        assert_eq!(m.triangles_merged().len(), 864);
        assert_eq!(m.materials[0].texture_path, r"system\avatargirl\NvZhuJue_Boot.tga");
        for v in &m.vertices {
            let w: f32 = v.influences.iter().map(|i| i.weight).sum();
            assert!((w - 1.0).abs() < 0.01, "权重和异常 {w}");
        }
    }

    /// 多子网格护甲（Armonr）：多索引节 + 多材质槽 + 高影响数顶点。
    /// FS36 含 9..13 影响的顶点——旧版 n>8 上限曾把这类文件整个目录扫描卡死
    /// （报错不带文件名，难定位）。FS61 为 6 节 6 材质且含 GBK 中文纹理路径
    /// （袖256.tga，from_utf8_lossy 会破坏往返）。均须字节级往返。
    #[test]
    fn multi_section_roundtrip() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Armonr");
        // (文件, 节数, 材质数, 最大影响数下限)
        let cases = [
            ("FS01_Upper.fskin", 2usize, 2usize, 4usize),
            ("FS36_Upper.fskin", 2, 2, 13),
            ("FS61_Upper.fskin", 6, 6, 1),
        ];
        for (name, secs, mats, infl) in cases {
            let p = format!("{dir}\\{name}");
            let Ok(bytes) = fs::read(&p) else {
                eprintln!("跳过: {p} 不存在");
                return;
            };
            let m = parse_fskin(&p, &bytes).unwrap();
            assert_eq!(m.submeshes.len(), secs, "{name} 节数");
            assert_eq!(m.materials.len(), mats, "{name} 材质数");
            let nmax = m.vertices.iter().map(|v| v.influences.len()).max().unwrap();
            assert!(nmax >= infl, "{name} 最大影响数 {nmax} 应 >= {infl}");
            for (k, s) in m.submeshes.iter().enumerate() {
                assert_eq!(s.zero1 >> 16, k as u32, "{name} 节{k} 节号");
                assert_eq!(s.vertex_count2 as usize, m.vertices.len(), "{name} 节{k} 顶点数");
            }
            // 材质槽序号与纹理路径非空
            for (k, mat) in m.materials.iter().enumerate() {
                assert_eq!(mat.lead, k as u32, "{name} 材质{k} 槽号");
                assert!(!mat.texture_path.is_empty(), "{name} 材质{k} 路径");
            }
            if name.starts_with("FS61") {
                // GBK 中文路径解码后应为 Unicode 文本
                assert!(m.materials[5].texture_path.contains("袖"), "GBK 路径解码异常");
            }
            let out = serialize_fskin(&m).unwrap();
            assert_eq!(bytes, out, "{name} 往返不一致");
        }
    }

    /// 零材质特效网格（FK55 Bip01 Footsteps01）：全部顶点 0 影响、无材质槽，
    /// 索引区直接终结于文件尾，节头为 0xffff0000 标记。
    #[test]
    fn zero_material_mesh() {
        let p = format!(
            "{}\\..\\res\\system\\Lower\\FK55_Bip01 Footsteps01.fskin",
            env!("CARGO_MANIFEST_DIR")
        );
        let Ok(bytes) = fs::read(&p) else {
            eprintln!("跳过: {p} 不存在");
            return;
        };
        let m = parse_fskin(&p, &bytes).unwrap();
        assert_eq!(m.vertices.len(), 9);
        assert!(m.vertices.iter().all(|v| v.influences.is_empty()));
        assert_eq!(m.submeshes.len(), 1);
        assert_eq!(m.submeshes[0].zero1, 0xffff_0000);
        assert_eq!(m.submeshes[0].triangles.len(), 8 * 3);
        assert!(m.materials.is_empty());
        let out = serialize_fskin(&m).unwrap();
        assert_eq!(bytes, out, "往返不一致");
    }

    /// scan_model_set 的配套默认骨架：AvatarGirl（act\girl.FSKEL，无数字且
    /// 含 girl 关键词优先于带数字动作）；无 act 目录时为 None
    #[test]
    fn scan_model_set_default_fskel() {
        let set = scan_model_set(ROOT).unwrap();
        let fskel = set.default_fskel.expect("AvatarGirl 应有默认骨架");
        assert!(
            fskel.to_lowercase().ends_with("avatargirl\\act\\girl.fskel"),
            "实际: {fskel}"
        );
        // 无 act 目录（Weapon/Jian）→ None
        let weapon = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/Weapon/Jian");
        if vfs::exists(weapon) {
            assert!(scan_model_set(weapon).unwrap().default_fskel.is_none());
        }
    }

    /// boneLocalPos 与顶点世界坐标应为同一刚体映射下的像（间距保持不变）。
    /// 这是 LBS 蒙皮（世界矩阵 × boneLocalPos）成立的前提。
    #[test]
    fn bone_local_pos_is_rigid() {
        fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        }
        let files = collect_fskins();
        if files.is_empty() {
            eprintln!("跳过: 测试目录不存在");
            return;
        }
        let mut worst = 0f32; // 相对偏差 |dl-dw|/max(dl,1)
        for f in &files {
            let bytes = fs::read(f).unwrap();
            let m = parse_fskin(&f.to_string_lossy(), &bytes).unwrap();
            // 每根骨骼收集 (局部点, 世界点)。多影响顶点的 pos 是加权混合结果，
            // 与单骨骼不构成刚体对应，只取单影响（权重=1）顶点验证。
            let mut by_bone: BTreeMap<u32, Vec<([f32; 3], [f32; 3])>> = BTreeMap::new();
            for v in &m.vertices {
                if v.influences.len() != 1 {
                    continue;
                }
                let inf = &v.influences[0];
                by_bone
                    .entry(inf.bone_index)
                    .or_default()
                    .push((inf.bone_local_pos, v.pos));
            }
            for (bone, pts) in &by_bone {
                let n = pts.len();
                assert!(n >= 1, "骨骼 {bone} 无点对");
                // 全对采样过大时抽点（步长取整，保证首尾点参与）
                let step = (n / 64).max(1);
                let idx: Vec<usize> = (0..n).step_by(step).chain(std::iter::once(n - 1)).collect();
                for &i in &idx {
                    for &j in &idx {
                        if i >= j {
                            continue;
                        }
                        let dl = dist3(pts[i].0, pts[j].0);
                        let dw = dist3(pts[i].1, pts[j].1);
                        worst = worst.max((dl - dw).abs() / dl.max(1.0));
                    }
                }
            }
        }
        eprintln!("boneLocalPos 刚性最大相对偏差 {worst:.6}");
        assert!(worst < 0.002, "骨骼局部坐标不是刚体映射，最大相对偏差 {worst}");
    }
}
