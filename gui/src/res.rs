//! 资源目录列举（前端资源树懒加载 + 资源网格）。
//!
//! 经由 crate::vfs 统一寻址：既支持磁盘目录，也支持 `cpk:` 前缀的
//! CPK 包内路径（如 `cpk:D:\game\system.cpk\AvatarGirl`）。
//!
//! "整体模型目录"判定：仅限 AvatarBoy / AvatarGirl / Monster 下的目录（整体
//! 预览目前只支持这些），且直接子项含 .fskin / .fak / .fsm，或 act/ 子目录含
//! .FSKEL（如 system/AvatarGirl、system/Monster/G080）。其余目录（如
//! system/Weapon/Jian 含 .fsm）不开放整体预览，后续支持的目录类型再加。

use serde::Serialize;
use std::io::Read;

use crate::{fak, fsm, fskin, fskel, vfs};

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ResEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// 小写扩展名（fskin/fskel/dds/...），目录为 "dir"
    pub kind: String,
    pub byte_len: u64,
    /// 目录是否为"整体模型"（仅 AvatarBoy/AvatarGirl/Monster 范围内可整体预览）
    pub is_model: bool,
    pub vertex_count: Option<u32>,
    pub triangle_count: Option<u32>,
    pub frame_count: Option<u32>,
    pub bone_count: Option<u32>,
}

fn has_ext(name: &str, ext: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(|e| e.eq_ignore_ascii_case(ext))
        .unwrap_or(false)
}

fn dir_contains(dir: &str, ext: &str) -> bool {
    vfs::list_dir(dir)
        .map(|es| es.iter().any(|e| !e.is_dir && has_ext(&e.name, ext)))
        .unwrap_or(false)
}

/// 整体预览支持的根目录名（路径任一层级命中即视为在范围内，大小写不敏感）
const MODEL_PREVIEW_ROOTS: [&str; 3] = ["avatarboy", "avatargirl", "monster"];

/// 整体预览范围：目前仅 AvatarBoy / AvatarGirl / Monster 下的目录支持；
/// 其余目录（如 Weapon）即使含模型文件也不开放，后续有支持的类型再扩充
fn in_model_scope(dir: &str) -> bool {
    dir.split(['\\', '/'])
        .any(|seg| MODEL_PREVIEW_ROOTS.iter().any(|r| seg.eq_ignore_ascii_case(r)))
}

fn is_model_dir(dir: &str) -> bool {
    if !in_model_scope(dir) {
        return false;
    }
    if dir_contains(dir, "fskin") || dir_contains(dir, "fak") || dir_contains(dir, "fsm") {
        return true;
    }
    dir_contains(&vfs::join_vpath(dir, "act"), "fskel")
}

/// 轻量统计：按扩展名选择解析器（失败仅缺省 meta）
fn light_stats(e: &vfs::VfsEntry, kind: &str) -> (Option<u32>, Option<u32>, Option<u32>, Option<u32>) {
    match kind {
        // 三角形数需要完整解析；失败仅缺省 meta
        "fskin" => match vfs::read_file(&e.path).ok().and_then(|b| fskin::parse_fskin(&e.path, &b).ok()) {
            Some(m) => (
                Some(m.vertices.len() as u32),
                Some(m.triangle_count() as u32),
                None,
                None,
            ),
            None => (None, None, None, None),
        },
        // 帧数/骨骼数在文件头（0x114 骨骼数），只读头部避免整读大文件
        "fskel" => {
            let mut buf = vec![0u8; 0x118];
            let meta = vfs::open_reader(&e.path)
                .and_then(|mut r| r.read_exact(&mut buf).map_err(|e| e.to_string()))
                .ok()
                .and_then(|_| fskel::light_meta(&buf));
            match meta {
                Some((frames, bones)) => (None, None, Some(frames), Some(bones)),
                None => (None, None, None, None),
            }
        }
        // 轻量统计只走头部 + 子网格区，不解析逐帧顶点数据
        "fak" => match vfs::open_reader(&e.path).ok().and_then(|mut r| fak::light_meta(&mut r)) {
            Some((vc, tc, fc)) => (Some(vc), Some(tc), Some(fc), None),
            None => (None, None, None, None),
        },
        // 轻量统计只走头部 + 子网格区，不解析顶点与材质
        "fsm" => match vfs::open_reader(&e.path).ok().and_then(|mut r| fsm::light_meta(&mut r)) {
            Some((vc, tc)) => (Some(vc), Some(tc), None, None),
            None => (None, None, None, None),
        },
        _ => (None, None, None, None),
    }
}

/// 列举虚拟目录直接子项：目录在前、文件在后，各自按名称不区分大小写排序。
/// fskin / fskel / fak / fsm 附带轻量统计（解析失败仅缺省 meta）。
pub fn list_dir(dir: &str) -> Result<Vec<ResEntry>, String> {
    let entries = vfs::list_dir(dir)?;
    let mut dirs: Vec<ResEntry> = Vec::new();
    let mut files: Vec<ResEntry> = Vec::new();
    for e in entries {
        if e.is_dir {
            let is_model = is_model_dir(&e.path);
            dirs.push(ResEntry {
                name: e.name,
                path: e.path,
                is_dir: true,
                kind: "dir".into(),
                byte_len: 0,
                is_model,
                vertex_count: None,
                triangle_count: None,
                frame_count: None,
                bone_count: None,
            });
        } else {
            let kind = e
                .name
                .rsplit('.')
                .next()
                .map(|x| x.to_lowercase())
                .unwrap_or_default();
            let (vc, tc, fc, bc) = light_stats(&e, &kind);
            files.push(ResEntry {
                name: e.name,
                path: e.path,
                is_dir: false,
                kind,
                byte_len: e.byte_len,
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

    /// CPK 模式：打包样例目录后列举，meta 统计与整体模型判定应与磁盘一致
    #[test]
    fn list_cpk_meta_matches_disk() {
        let cpk_path = crate::vfs::build_system_cpk("res");
        let root = format!("cpk:{}", cpk_path.display());
        let j = crate::vfs::join_vpath;

        let sys = list_dir(&root).unwrap();
        let girl = sys.iter().find(|e| e.name == "AvatarGirl").unwrap();
        assert!(girl.is_dir && girl.is_model, "AvatarGirl 应为整体模型目录");
        assert!(
            sys.iter().any(|e| e.name == "Monster" && !e.is_model),
            "Monster 普通目录"
        );

        // fskin meta
        let girl_dir = list_dir(&j(&root, "AvatarGirl")).unwrap();
        let head = girl_dir
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("girl_head.fskin"))
            .unwrap();
        assert!(head.vertex_count.unwrap_or(0) > 0);
        // fak meta
        let g080 = list_dir(&j(&root, "Monster\\G080")).unwrap();
        let fak = g080.iter().find(|e| e.kind == "fak").unwrap();
        assert_eq!(fak.vertex_count, Some(821));
        assert_eq!(fak.frame_count, Some(133));
        // fsm meta
        let jian = list_dir(&j(&root, "Weapon\\Jian")).unwrap();
        let fsm = jian.iter().find(|e| e.kind == "fsm").unwrap();
        assert_eq!(fsm.vertex_count, Some(154));
        assert_eq!(fsm.triangle_count, Some(112));
        // fskel meta
        let act = list_dir(&j(&root, "AvatarGirl\\act")).unwrap();
        assert!(
            act.iter()
                .any(|e| e.kind == "fskel" && e.frame_count.unwrap_or(0) > 0)
        );
        let _ = std::fs::remove_dir_all(cpk_path.parent().unwrap());
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

    #[test]
    fn list_jian_has_fsm_meta() {
        // Jian 含 .fsm 但不在 AvatarBoy/AvatarGirl/Monster 下，不标为"整体模型"
        let weapon = list_dir(&format!("{RES}/system/Weapon")).unwrap();
        let dir = weapon.iter().find(|e| e.name == "Jian").unwrap();
        assert!(dir.is_dir && !dir.is_model);
        let entries = list_dir(&format!("{RES}/system/Weapon/Jian")).unwrap();
        let fsm = entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("J01.FSM"))
            .unwrap();
        assert_eq!(fsm.kind, "fsm");
        assert_eq!(fsm.vertex_count, Some(154));
        assert_eq!(fsm.triangle_count, Some(112));
    }
}
