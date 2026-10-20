//! .fskin (SKIN v1.0) 解析与序列化
//!
//! 布局:
//!   0x00 "SKIN" | 0x04 版本 | 0x08 顶点数 | 0x0C 顶点偏移(恒0x138)
//!   0x10 1 | 0x14 索引区偏移 | 0x18 1 | 0x1C 材质区偏移
//!   0x20 bbox max/min 各3个f32 | 0x38..0x138 保留256字节
//!   顶点记录(变长): [pos3][normal3][uv2][影响数n][n×(骨骼u32+权重f32+骨骼局部坐标3f32)]
//!   索引区: 小头 + 三角形数 + u16索引×3n + 填充
//!   材质区: f32头 + diffuse/ambient/emissive/specular 各4f32 + 高光强度
//!           + 3字节前缀 + 纹理路径 + 尾部填充

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const VERTEX_OFFSET: usize = 0x138;

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
    pub lead: f32,
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub emissive: [f32; 4],
    pub specular: [f32; 4],
    pub spec_power: f32,
    pub pre: [u8; 3],
    pub texture_path: String,
    pub tail: Vec<u8>,
}

/// 索引区里语义未定的字段，按原样保留保证往返一致
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IndexMeta {
    pub zero1: u32,
    pub zero2: u16,
    pub vertex_count2: u16,
    pub pad4: [u16; 4],
    pub hash1: u32,
    pub hash2: u32,
    pub one1: u32,
    pub one2: u32,
    pub pad_tail: Vec<u8>,
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
    /// 扁平 u16 索引，每 3 个为一个三角形
    pub triangles: Vec<u16>,
    pub index_meta: IndexMeta,
    pub material: Material,
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
    rh.skip(4)?; // 索引区数量 = 1
    let index_off = rh.u32()? as usize;
    rh.skip(4)?; // 材质数量 = 1
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
        if !(1..=8).contains(&n) {
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

    // ---- 索引区 ----
    let zero1 = r.u32()?;
    let zero2 = r.u16()?;
    let vertex_count2 = r.u16()?;
    let mut pad4 = [0u16; 4];
    for v in &mut pad4 {
        *v = r.u16()?;
    }
    let hash1 = r.u32()?;
    let hash2 = r.u32()?;
    let one1 = r.u32()?;
    let one2 = r.u32()?;
    let tri_count = r.u32()? as usize;
    let mut triangles = Vec::with_capacity(tri_count * 3);
    for _ in 0..tri_count * 3 {
        triangles.push(r.u16()?);
    }
    let pad_tail = bytes[r.pos..material_off].to_vec();

    // ---- 材质区 ----
    r.pos = material_off;
    let lead = r.f32()?;
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
    let p_end = bytes[p_start..]
        .iter()
        .position(|&c| c == 0)
        .ok_or("纹理路径未终止")?
        + p_start;
    let texture_path = String::from_utf8_lossy(&bytes[p_start..p_end]).into_owned();
    r.pos = p_end + 1;
    let tail = bytes[r.pos..].to_vec();

    Ok(FskinModel {
        path: path.to_string(),
        version,
        bbox_max,
        bbox_min,
        reserved256,
        vertices,
        triangles,
        index_meta: IndexMeta {
            zero1,
            zero2,
            vertex_count2,
            pad4,
            hash1,
            hash2,
            one1,
            one2,
            pad_tail,
        },
        material: Material {
            lead,
            diffuse,
            ambient,
            emissive,
            specular,
            spec_power,
            pre,
            texture_path,
            tail,
        },
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

    // 索引区
    let im = &m.index_meta;
    if m.triangles.len() % 3 != 0 {
        return Err("三角形索引数不是 3 的倍数".into());
    }
    let mut ibuf = Vec::new();
    push_u32(&mut ibuf, im.zero1);
    push_u16(&mut ibuf, im.zero2);
    push_u16(&mut ibuf, m.vertices.len().min(u16::MAX as usize) as u16);
    for v in im.pad4.iter() {
        push_u16(&mut ibuf, *v);
    }
    push_u32(&mut ibuf, im.hash1);
    push_u32(&mut ibuf, im.hash2);
    push_u32(&mut ibuf, im.one1);
    push_u32(&mut ibuf, im.one2);
    push_u32(&mut ibuf, (m.triangles.len() / 3) as u32);
    for t in &m.triangles {
        push_u16(&mut ibuf, *t);
    }
    ibuf.extend_from_slice(&im.pad_tail);
    let material_off = index_off + ibuf.len();

    // 材质区
    let mat = &m.material;
    let mut mbuf = Vec::new();
    push_f32(&mut mbuf, mat.lead);
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

    // 头部
    let mut out = Vec::with_capacity(material_off + mbuf.len());
    out.extend_from_slice(b"SKIN");
    push_u32(&mut out, m.version);
    push_u32(&mut out, m.vertices.len() as u32);
    push_u32(&mut out, VERTEX_OFFSET as u32);
    push_u32(&mut out, 1);
    push_u32(&mut out, index_off as u32);
    push_u32(&mut out, 1);
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
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if ext.eq_ignore_ascii_case("fskin") {
                out.push(p);
            }
        }
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

/// 扫描目录：按 (顶点数, 索引区偏移) 几何签名把换装变体归为一组；
/// 含根目录文件的组排在前面且默认选中根目录成员。
pub fn scan_model_set(dir: &str) -> Result<ModelSet, String> {
    let root = PathBuf::from(dir);
    if !root.is_dir() {
        return Err(format!("目录不存在: {dir}"));
    }
    // 纹理索引: 小写文件名(不含扩展名) -> 路径
    let mut tex_files = Vec::<PathBuf>::new();
    walk_images(&root, &mut tex_files);
    let mut tex_index: BTreeMap<String, String> = BTreeMap::new();
    for t in &tex_files {
        if let Some(stem) = t.file_stem().and_then(|s| s.to_str()) {
            tex_index
                .entry(stem.to_lowercase())
                .or_insert_with(|| t.to_string_lossy().into_owned());
        }
    }

    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();
    let mut parts = Vec::new();
    let mut sigs = Vec::new();
    for f in files {
        let bytes = fs::read(&f).map_err(|e| format!("读取 {}: {e}", f.display()))?;
        let m = parse_fskin(&f.to_string_lossy(), &bytes)?;
        // 几何签名: 顶点数 + 索引区偏移（变长顶点记录总长），相同即为同一网格的换装变体
        sigs.push(format!("{}_{}", m.vertices.len(), m.triangles.len()));
        let tex_file = m
            .material
            .texture_path
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
        let in_root = f.parent() == Some(&root);
        let rel_dir = f
            .parent()
            .and_then(|p| p.strip_prefix(&root).ok())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        parts.push(PartInfo {
            path: f.to_string_lossy().into_owned(),
            file_name: f
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            rel_dir,
            in_root,
            vertex_count: m.vertices.len(),
            triangle_count: m.triangles.len() / 3,
            bbox_min: m.bbox_min,
            bbox_max: m.bbox_max,
            texture_path: m.material.texture_path.clone(),
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
                    p.parent()
                        .and_then(|d| d.file_name())
                        .map(|n| n.eq_ignore_ascii_case(dir_name))
                        .unwrap_or(false)
                })
                .map(|p| TexVariant {
                    label: p
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    path: p.to_string_lossy().into_owned(),
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

    Ok(ModelSet {
        root_dir: dir.to_string(),
        groups,
        textures: tex_index
            .into_iter()
            .map(|(stem, path)| TexInfo { stem, path })
            .collect(),
        tex_slots,
    })
}

fn walk_images(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_images(&p, out);
        } else if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            if matches!(
                ext.to_ascii_lowercase().as_str(),
                "dds" | "tga" | "png" | "jpg" | "bmp"
            ) {
                out.push(p);
            }
        }
    }
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    // 项目根目录 res/system 为 pack 解压后的实际目录
    const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/AvatarGirl");

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
    /// 回归：蒙皮共识投票按"部件骨骼覆盖数"加权（viewer.ts refreshBindInverses
    /// 的数据面镜像）。Girl_hair05 在 0x6 骨上有 256 个单影响点，按点数投票会
    /// 压过头部（124 点），把共享 0x6 的部件（头部/基础发型）装到 hair05 的
    /// 离群绑定上——两套绑定相差约 5° 旋转 + 4 单位平移，发髻与颅顶脱开数
    /// 单位（用户报告的"5 号头发绑定位置异常"）。按骨骼覆盖数加权后头部
    /// （5 骨）赢得 0x6 共识，hair05 按头部绑定渲染，与头部保持刚性贴合。
    #[test]
    fn hair05_consensus_vote_regression() {
        use crate::fskel;
        let load = |rel: &str| parse_fskin(&format!("{ROOT}/{rel}"), &fs::read(format!("{ROOT}/{rel}")).unwrap()).unwrap();
        let head = load("girl_head.fskin");
        let h1 = load("MHair/Girl_hair01.fskin");
        let h5 = load("MHair/Girl_hair05.fskin");

        // ---- 拟合工具（与 viewer.fitBindMatrix 相同：Kabsch + 极分解） ----
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
        let kabsch = |pairs: &[([f32; 3], [f32; 3])]| -> [[f32; 4]; 4] {
            let n = pairs.len() as f32;
            let mut ca = [0f32; 3]; let mut cb = [0f32; 3];
            for (a, b) in pairs { for k in 0..3 { ca[k] += a[k]/n; cb[k] += b[k]/n; } }
            let mut m = [0f32; 9];
            for (a, b) in pairs { for r in 0..3 { for c in 0..3 { m[r*3+c] += (b[r]-cb[r])*(a[c]-ca[c]); } } }
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
            assert!(det3(&m) > 0.5, "极分解应收敛到纯旋转");
            let t = [cb[0]-(m[0]*ca[0]+m[1]*ca[1]+m[2]*ca[2]),
                     cb[1]-(m[3]*ca[0]+m[4]*ca[1]+m[5]*ca[2]),
                     cb[2]-(m[6]*ca[0]+m[7]*ca[1]+m[8]*ca[2])];
            let mut o = [[0f32; 4]; 4];
            for i in 0..3 { for j in 0..3 { o[i][j] = m[i*3+j]; } o[i][3] = t[i]; }
            o[3][3] = 1.0; o
        };
        fn mm4(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
            let mut w = [[0f32; 4]; 4];
            for i in 0..4 { for j in 0..4 { w[i][j] = (0..4).map(|k| a[i][k]*b[k][j]).sum(); } }
            w
        }
        fn inv_rigid(m: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
            let mut o = [[0f32; 4]; 4];
            for i in 0..3 { for j in 0..3 { o[i][j] = m[j][i]; } }
            for i in 0..3 { o[i][3] = -(0..3).map(|k| o[i][k]*m[k][3]).sum::<f32>(); }
            o[3][3] = 1.0; o
        }
        fn ap4(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
            [m[0][0]*p[0]+m[0][1]*p[1]+m[0][2]*p[2]+m[0][3],
             m[1][0]*p[0]+m[1][1]*p[1]+m[1][2]*p[2]+m[1][3],
             m[2][0]*p[0]+m[2][1]*p[1]+m[2][2]*p[2]+m[2][3]]
        }
        let diff_t = |a: &[[f32;4];4], b: &[[f32;4];4]| -> f32 {
            let mut d = 0f32;
            for i in 0..4 { for j in 0..4 { d = d.max((a[i][j]-b[i][j]).abs()); } }
            d
        };

        let pairs6 = |m: &FskinModel| -> Vec<([f32;3],[f32;3])> {
            m.vertices.iter()
                .filter(|v| v.influences.len()==1 && v.influences[0].bone_index==6)
                .map(|v| (v.influences[0].bone_local_pos, v.pos)).collect()
        };
        let t_head = kabsch(&pairs6(&head));
        let t_h1 = kabsch(&pairs6(&h1));
        let t_h5 = kabsch(&pairs6(&h5));
        assert!(diff_t(&t_head, &t_h5) >= 0.1, "hair05 与头部 0x6 绑定应超过分簇阈值");
        assert!(diff_t(&t_head, &t_h1) < 0.1, "hair01 应与头部 0x6 绑定一致");

        let span_of = |m: &FskinModel| -> usize {
            let mut s: Vec<u32> = m.vertices.iter().flat_map(|v| v.influences.iter().map(|i| i.bone_index)).collect();
            s.sort_unstable(); s.dedup(); s.len()
        };
        let (sp_head, sp_h1, sp_h5) = (span_of(&head), span_of(&h1), span_of(&h5));

        // 加权投票（修复后规则）：簇得分 = Σ骨骼覆盖数（同覆盖比点数）
        // 场景1：动画模式默认装配 + hair05（hair01 已被互斥槽移除）
        let pts_head = pairs6(&head).len();
        let pts_h5 = pairs6(&h5).len();
        assert!(sp_head > sp_h5,
            "头部（{sp_head} 骨）应凭覆盖数赢得 0x6 共识，而非 hair05（{sp_h5} 骨 {pts_h5} 点 > 头部 {pts_head} 点）");
        // 场景2：hair01 与 hair05 同场（分组面板可各自开关，不走互斥槽）
        let pts_h1 = pairs6(&h1).len();
        assert!(sp_head + sp_h1 > sp_h5 || (sp_head + sp_h1 == sp_h5 && pts_head + pts_h1 > pts_h5),
            "hair01 在场时头部簇更不应输给 hair05");

        // ---- 渲染验证：hair05 按头部绑定渲染后与头部保持刚性贴合 ----
        let skel = fskel::get_or_load(&format!("{ROOT}/act/sta.FSKEL")).unwrap();
        let bone_count = skel.parents.len();
        assert!(bone_count > 47, "骨架应覆盖 head 用到的 0x2f 骨");
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
        let mut world = vec![[[0f32; 4]; 4]; bone_count];
        for b in 0..bone_count {
            let o = b * 7;
            let r = qm([flat[o+3], flat[o+4], flat[o+5], flat[o+6]]);
            let mut m = [[0f32; 4]; 4];
            for i in 0..3 { for j in 0..3 { m[i][j] = r[i][j]; } m[i][3] = flat[o+i]; }
            m[3][3] = 1.0;
            let p = skel.parents[b];
            world[b] = if p >= 0 { mm4(&world[p as usize], &m) } else { m };
        }
        let w2 = mm4(&world[6], &inv_rigid(&t_head));
        let w1 = mm4(&world[6], &inv_rigid(&t_h5));
        // 头部 0x6 纯影响顶点：修复前后自身的位移差 = 旧规则的可见错位量
        let mut old_bug_disp = 0f32;
        for v in &head.vertices {
            if v.influences.len() == 1 && v.influences[0].bone_index == 6 {
                let a = ap4(&w2, v.pos);
                let b = ap4(&w1, v.pos);
                old_bug_disp = old_bug_disp.max(((a[0]-b[0]).powi(2)+(a[1]-b[1]).powi(2)+(a[2]-b[2]).powi(2)).sqrt());
            }
        }
        eprintln!("旧规则（hair05 赢得 0x6）下头部顶点错位最大 {old_bug_disp:.2}");
        assert!(old_bug_disp > 2.0, "数据前提：hair05 绑定与头部的差异应造成 >2 单位错位");
        // hair05 与头部 0x6 纯影响顶点同过 w2 → 距离严格保持（刚性贴合）
        let head6: Vec<[f32;3]> = head.vertices.iter()
            .filter(|v| v.influences.len()==1 && v.influences[0].bone_index==6)
            .map(|v| v.pos).collect();
        let mut worst = 0f32;
        let mut pairs_checked = 0usize;
        for v in h5.vertices.iter().step_by(4) {
            let pr = ap4(&w2, v.pos);
            let mut best_a = (f32::MAX, [0f32;3]);
            for hp in head6.iter().step_by(3) {
                let d = ((v.pos[0]-hp[0]).powi(2)+(v.pos[1]-hp[1]).powi(2)+(v.pos[2]-hp[2]).powi(2)).sqrt();
                if d < best_a.0 { best_a = (d, *hp); }
            }
            let hr = ap4(&w2, best_a.1);
            let d_r = ((pr[0]-hr[0]).powi(2)+(pr[1]-hr[1]).powi(2)+(pr[2]-hr[2]).powi(2)).sqrt();
            worst = worst.max((d_r - best_a.0).abs());
            pairs_checked += 1;
        }
        eprintln!("修复后 hair05↔头部 0x6 距离保持最大偏差 {worst:.4}（{pairs_checked} 对）");
        assert!(worst < 0.05, "hair05 应与头部刚性贴合（距离保持）");
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
            for _ in 0..32 {
                let mi = inv3(&m).expect("迭代矩阵可逆");
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
                    break;
                }
            }
            assert!(det3(&m) > 0.5, "极分解应收敛到纯旋转");
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
        let mut files = Vec::new();
        walk(Path::new(ROOT), &mut files);
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
        assert_eq!(m.triangles.len(), 864);
        assert_eq!(m.material.texture_path, r"system\avatargirl\NvZhuJue_Boot.tga");
        for v in &m.vertices {
            let w: f32 = v.influences.iter().map(|i| i.weight).sum();
            assert!((w - 1.0).abs() < 0.01, "权重和异常 {w}");
        }
    }

    /// boneLocalPos 与顶点世界坐标应为同一刚体映射下的像（间距保持不变）。
    /// 这是 LBS 蒙皮（世界矩阵 × boneLocalPos）成立的前提。
    #[test]
    fn bone_local_pos_is_rigid() {
        fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        }
        let mut files = Vec::new();
        walk(Path::new(ROOT), &mut files);
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
