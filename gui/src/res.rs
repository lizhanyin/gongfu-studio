//! 资源目录列举（前端资源树懒加载 + 资源网格）。
//!
//! "整体模型目录"判定：直接子项含 .fskin / .fak，或 act/ 子目录含 .FSKEL
//! （如 system/AvatarGirl、system/Monster/G080）。
//! 未来直接加载 pkg/cpk 时保持 list_dir 命令签名不变、替换实现即可。

use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::Path;

use crate::{fak, fskin, fskel};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// 小写扩展名（fskin/fskel/dds/...），目录为 "dir"
    pub kind: String,
    pub byte_len: u64,
    /// 目录是否为"整体模型"
    pub is_model: bool,
    pub vertex_count: Option<u32>,
    pub triangle_count: Option<u32>,
    pub frame_count: Option<u32>,
    pub bone_count: Option<u32>,
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension()
        .map(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
        .unwrap_or(false)
}

fn dir_contains(dir: &Path, ext: &str) -> bool {
    fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| has_ext(&e.path(), ext)))
        .unwrap_or(false)
}

fn is_model_dir(dir: &Path) -> bool {
    if dir_contains(dir, "fskin") || dir_contains(dir, "fak") {
        return true;
    }
    let act = dir.join("act");
    act.is_dir() && dir_contains(&act, "fskel")
}

/// 列举目录直接子项：目录在前、文件在后，各自按名称不区分大小写排序。
/// fskin / fskel 附带轻量统计（解析失败仅缺省 meta）。
pub fn list_dir(dir: &str) -> Result<Vec<ResEntry>, String> {
    let root = Path::new(dir);
    let entries = fs::read_dir(root).map_err(|e| format!("读取目录失败: {e}"))?;
    let mut dirs: Vec<ResEntry> = Vec::new();
    let mut files: Vec<ResEntry> = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = p.to_string_lossy().into_owned();
        if ft.is_dir() {
            dirs.push(ResEntry {
                name,
                path,
                is_dir: true,
                kind: "dir".into(),
                byte_len: 0,
                is_model: is_model_dir(&p),
                vertex_count: None,
                triangle_count: None,
                frame_count: None,
                bone_count: None,
            });
        } else {
            let kind = p
                .extension()
                .map(|x| x.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let byte_len = e.metadata().map(|m| m.len()).unwrap_or(0);
            let (vc, tc, fc, bc) = if kind == "fskin" {
                // 三角形数需要完整解析；失败仅缺省 meta
                match fs::read(&p).ok().and_then(|b| fskin::parse_fskin(&path, &b).ok()) {
                    Some(m) => (
                        Some(m.vertices.len() as u32),
                        Some((m.triangles.len() / 3) as u32),
                        None,
                        None,
                    ),
                    None => (None, None, None, None),
                }
            } else if kind == "fskel" {
                // 帧数/骨骼数在文件头（0x114 骨骼数），只读头部避免整读大文件
                let mut buf = vec![0u8; 0x118];
                let meta = fs::File::open(&p)
                    .and_then(|mut f| f.read_exact(&mut buf))
                    .ok()
                    .and_then(|_| fskel::light_meta(&buf));
                match meta {
                    Some((frames, bones)) => (None, None, Some(frames), Some(bones)),
                    None => (None, None, None, None),
                }
            } else if kind == "fak" {
                // 轻量统计只走头部 + 子网格区，不解析逐帧顶点数据
                match fak::light_meta_from_file(&p) {
                    Some((vc, tc, fc)) => (Some(vc), Some(tc), Some(fc), None),
                    None => (None, None, None, None),
                }
            } else {
                (None, None, None, None)
            };
            files.push(ResEntry {
                name,
                path,
                is_dir: false,
                kind,
                byte_len,
                is_model: false,
                vertex_count: vc,
                triangle_count: tc,
                frame_count: fc,
                bone_count: bc,
            });
        }
    }
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    dirs.extend(files);
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res");

    #[test]
    fn list_system_finds_model_dirs() {
        let entries = list_dir(&format!("{RES}/system")).unwrap();
        let girl = entries.iter().find(|e| e.name == "AvatarGirl").unwrap();
        assert!(girl.is_dir && girl.is_model, "AvatarGirl 应为整体模型目录");
        let monster = entries.iter().find(|e| e.name == "Monster").unwrap();
        assert!(monster.is_dir && !monster.is_model);
    }

    #[test]
    fn list_avatar_girl_has_fskin_meta() {
        let entries = list_dir(&format!("{RES}/system/AvatarGirl")).unwrap();
        let head = entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("girl_head.fskin"))
            .unwrap();
        assert_eq!(head.kind, "fskin");
        assert!(head.vertex_count.unwrap_or(0) > 0);
        assert!(head.triangle_count.unwrap_or(0) > 0);
        // 目录在前、文件在后
        assert!(entries.first().unwrap().is_dir);
    }

    #[test]
    fn list_act_has_fskel_meta() {
        let entries = list_dir(&format!("{RES}/system/AvatarGirl/act")).unwrap();
        let skel = entries.iter().find(|e| e.kind == "fskel").unwrap();
        assert!(skel.frame_count.unwrap_or(0) > 0);
        assert!(skel.bone_count.unwrap_or(0) > 0);
    }

    #[test]
    fn list_g080_has_fak_meta() {
        // 含 .fak 的目录在上级列表中标为"整体模型"
        let monster = list_dir(&format!("{RES}/system/Monster")).unwrap();
        let dir = monster.iter().find(|e| e.name == "G080").unwrap();
        assert!(dir.is_dir && dir.is_model);
        // FAK 文件带轻量统计
        let entries = list_dir(&format!("{RES}/system/Monster/G080")).unwrap();
        let fak = entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("G080.FAK"))
            .unwrap();
        assert_eq!(fak.kind, "fak");
        assert_eq!(fak.vertex_count, Some(821));
        assert_eq!(fak.triangle_count, Some(925));
        assert_eq!(fak.frame_count, Some(133));
    }
}
