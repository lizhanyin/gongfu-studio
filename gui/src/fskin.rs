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
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TexInfo {
    /// 小写 stem，用于与材质路径匹配
    pub stem: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelSet {
    pub root_dir: String,
    pub groups: Vec<PartGroup>,
    pub textures: Vec<TexInfo>,
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

    // 按几何签名分组
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
        groups.push(PartGroup {
            label: if has_root {
                label
            } else {
                format!("{label} (附加)")
            },
            signature: sig,
            parts: group_parts,
        });
    }
    // 根目录组在前，其余按名称
    groups.sort_by_key(|g| (!g.parts.iter().any(|p| p.in_root), g.label.clone()));

    Ok(ModelSet {
        root_dir: dir.to_string(),
        groups,
        textures: tex_index
            .into_iter()
            .map(|(stem, path)| TexInfo { stem, path })
            .collect(),
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
