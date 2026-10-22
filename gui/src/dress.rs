//! 角色装扮目录扫描（检查器「角色装扮」面板数据源）。
//!
//! 游戏资源布局：AvatarGirl / AvatarBoy 为男女基础模型目录；system 下
//! Hat / Armonr / Lower / Glove / Boot 为装备模型目录（帽子为 .fsm 静态
//! 网格，其余为 .fskin 蒙皮网格）；shizhuang / TransformFashion 为时装
//! （单文件全身模型，脸型贴图在基础模型目录）。
//!
//! 装备文件名前缀区分性别：两字母前缀首字母 f=女体、m=男体，第二字母为
//! 部件类型 —— Armonr FS/MS、Boot FX/MX、Glove FW/MW、Hat FM/MW、
//! Lower FK/MW（男体除上装 MS、鞋 MX 外统一 MW 系）。其余前缀视为通用；
//! 前缀字母后紧跟的数字为款式号，同号跨槽组成套装。无材质槽的 fskin
//! （FK55 类特效网格）不参与换装，剔除。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::{fsm, fskin, fskel, vfs};

/// 基础模型目录（目录名小写, 标签, 性别）
const BASE_DIRS: [(&str, &str, &str); 2] = [
    ("avatargirl", "女体 AvatarGirl", "f"),
    ("avatarboy", "男性 AvatarBoy", "m"),
];

/// 装备槽目录（目录名小写 = 槽 key, 标签, 文件类型）
const SLOT_DIRS: [(&str, &str, &str); 5] = [
    ("hat", "帽子", "fsm"),
    ("armonr", "护甲", "fskin"),
    ("lower", "下装", "fskin"),
    ("glove", "手套", "fskin"),
    ("boot", "鞋子", "fskin"),
];

/// 时装目录名（小写；shizhuang = 常规时装，transformfashion = 变身时装）
const FASHION_DIRS: [&str; 2] = ["shizhuang", "transformfashion"];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CharacterBase {
    /// 目录名（avatargirl / avatarboy），小写
    pub key: String,
    pub label: String,
    /// "f" | "m"
    pub gender: String,
    pub dir: String,
    /// act 动作目录路径（可能不存在）
    pub act_dir: String,
    /// 默认骨架动作文件（名称不含数字者，如 girl.FSKEL），无则 None
    pub default_fskel: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EquipItem {
    pub name: String,
    pub path: String,
    /// "f" | "m" | "u"（文件名前缀 fs / ms / 其他）
    pub gender: String,
    /// 款式号（前缀字母后的数字），无则 None
    pub number: Option<u32>,
    pub vertex_count: u32,
    pub triangle_count: u32,
    /// 每材质槽贴图的解析结果（全根贴图索引匹配），无材质槽为空
    pub texture_files: Vec<Option<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSlot {
    /// "hat" | "armonr" | "lower" | "glove" | "boot"
    pub key: String,
    pub label: String,
    /// "fsm" | "fskin"
    pub kind: String,
    /// 目录缺失或无可用物品时为空
    pub items: Vec<EquipItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CharacterCatalog {
    /// 角色资源根（AvatarGirl 等目录的父目录）
    pub root: String,
    pub bases: Vec<CharacterBase>,
    pub slots: Vec<CharacterSlot>,
    pub fashions: Vec<EquipItem>,
    /// 全角色根贴图索引（小写 stem → 路径），供跨目录解析装备/时装贴图
    pub textures: Vec<fskin::TexInfo>,
}

/// 文件名 → (性别, 款式号)：两字母字母前缀首字母定性别 f→女体、m→男体
/// （FS/MS=护甲、FX/MX=鞋、FW/MW=手套、FM/MW=帽、FK/MW=下装），其余前缀
/// 视为通用；紧随前缀的数字为款式号（FS01_Upper → (f, 1)，FM01 → (f, 1)）。
pub fn parse_gender_number(name: &str) -> (String, Option<u32>) {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let lower = stem.to_lowercase();
    let letters: String = lower.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rest = &lower[letters.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let gender = match letters.as_str() {
        l if l.len() == 2 && l.starts_with('f') => "f",
        l if l.len() == 2 && l.starts_with('m') => "m",
        _ => "u",
    }
    .to_string();
    let number = if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    };
    (gender, number)
}

/// 目录直接子项的目录名集合（小写）
fn child_dir_set(dir: &str) -> BTreeSet<String> {
    vfs::list_dir(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.is_dir)
        .map(|e| e.name.to_lowercase())
        .collect()
}

/// 已知角色子目录名（小写）：基础模型 / 时装 / 装备
const KNOWN_CHILD_DIRS: [&str; 9] = [
    "avatargirl",
    "avatarboy",
    "shizhuang",
    "transformfashion",
    "hat",
    "armonr",
    "lower",
    "glove",
    "boot",
];

/// 目录是否为角色资源根（直接子目录含任一已知角色目录）
pub fn is_character_root(dir: &str) -> bool {
    let kids = child_dir_set(dir);
    KNOWN_CHILD_DIRS.iter().any(|k| kids.contains(*k))
}

/// 从输入目录定位角色资源根：本级是 → 本级；否则尝试 `<input>\system`。
/// 磁盘与 `cpk:` 包内路径同样适用。
pub fn find_character_root(input: &str) -> Result<String, String> {
    if is_character_root(input) {
        return Ok(input.trim_end_matches(['\\', '/']).to_string());
    }
    let system = vfs::join_vpath(input, "system");
    if is_character_root(&system) {
        return Ok(system);
    }
    let shown = if input.trim().is_empty() { "(空路径)" } else { input };
    Err(format!(
        "未找到角色资源目录（{shown} 或其 system 子目录下无 AvatarGirl/AvatarBoy/装备/时装目录）"
    ))
}

/// 材质纹理路径（游戏风格，如 system\avatargirl\wf01.dds）→ 小写 stem
fn tex_stem_of(texture_path: &str) -> Option<String> {
    texture_path
        .rsplit(['\\', '/'])
        .next()
        .and_then(|base| base.rsplit_once('.'))
        .map(|(stem, _)| stem.to_lowercase())
        .filter(|s| !s.is_empty())
}

/// act 目录的默认骨架选取见 fskel::default_fskel_of（与皮肤模式共用）

/// 扫描装备/时装目录：逐文件解析统计与材质贴图。
/// 解析失败的文件跳过（目录列举照常）；fskin 无材质槽 = 特效网格，剔除。
fn scan_equip_dir(
    dir: &str,
    kind: &str,
    tex_index: &BTreeMap<String, String>,
) -> Vec<EquipItem> {
    let files = vfs::walk_files(dir, &[kind]);
    let mut items = Vec::new();
    for f in files {
        let Ok(bytes) = vfs::read_file(&f.path) else { continue };
        let (vc, tc, tex_paths): (u32, u32, Vec<String>) = if kind == "fskin" {
            let Ok(m) = fskin::parse_fskin(&f.path, &bytes) else { continue };
            if m.materials.is_empty() {
                continue; // 特效网格（FK55 类）：零材质，不参与换装
            }
            (
                m.vertices.len() as u32,
                m.triangle_count() as u32,
                m.materials.iter().map(|md| md.texture_path.clone()).collect(),
            )
        } else {
            let Ok(m) = fsm::parse_fsm(&f.path, &bytes) else { continue };
            (
                m.vertex_count,
                m.submeshes.iter().map(|s| s.triangles.len() as u32 / 3).sum(),
                m.materials.iter().map(|md| md.texture_path.clone()).collect(),
            )
        };
        let (gender, number) = parse_gender_number(&f.name);
        let texture_files = tex_paths
            .iter()
            .map(|p| tex_stem_of(p).and_then(|stem| tex_index.get(&stem).cloned()))
            .collect();
        items.push(EquipItem {
            name: f.name.clone(),
            path: f.path,
            gender,
            number,
            vertex_count: vc,
            triangle_count: tc,
            texture_files,
        });
    }
    // 女 → 男 → 通用，各按款式号再按名称
    items.sort_by(|a, b| {
        let rank = |g: &str| if g == "f" { 0 } else if g == "m" { 1 } else { 2 };
        (rank(&a.gender), a.number.unwrap_or(u32::MAX), a.name.clone())
            .cmp(&(rank(&b.gender), b.number.unwrap_or(u32::MAX), b.name.clone()))
    });
    items
}

/// 扫描角色资源：基础模型 + 装备槽 + 时装 + 全根贴图索引
pub fn scan_character(input: &str) -> Result<CharacterCatalog, String> {
    let root = find_character_root(input)?;

    // 全根贴图索引：小写 stem → 路径（同 stem 取路径序首个）
    let mut tex_index: BTreeMap<String, String> = BTreeMap::new();
    for t in vfs::walk_files(&root, &["dds", "tga", "png", "jpg", "bmp"]) {
        if let Some(stem) = tex_stem_of(&t.name) {
            tex_index.entry(stem).or_insert_with(|| t.path.clone());
        }
    }

    let mut bases = Vec::new();
    for (dir_name, label, gender) in BASE_DIRS {
        let dir = vfs::join_vpath(&root, dir_name);
        if !vfs::exists(&dir) {
            continue;
        }
        let act_dir = vfs::join_vpath(&dir, "act");
        // 默认骨架优先匹配基础模型关键词（avatargirl → girl）
        let kw = if dir_name.contains("girl") {
            "girl"
        } else if dir_name.contains("boy") {
            "boy"
        } else {
            dir_name
        };
        bases.push(CharacterBase {
            key: dir_name.to_string(),
            label: label.to_string(),
            gender: gender.to_string(),
            default_fskel: fskel::default_fskel_of(&act_dir, kw),
            dir,
            act_dir,
        });
    }

    let mut slots = Vec::new();
    for (dir_name, label, kind) in SLOT_DIRS {
        let dir = vfs::join_vpath(&root, dir_name);
        let items = if vfs::exists(&dir) {
            scan_equip_dir(&dir, kind, &tex_index)
        } else {
            Vec::new()
        };
        slots.push(CharacterSlot {
            key: dir_name.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            items,
        });
    }

    let mut fashions = Vec::new();
    for dir_name in FASHION_DIRS {
        let dir = vfs::join_vpath(&root, dir_name);
        if vfs::exists(&dir) {
            fashions.extend(scan_equip_dir(&dir, "fskin", &tex_index));
        }
    }

    if bases.is_empty()
        && slots.iter().all(|s| s.items.is_empty())
        && fashions.is_empty()
    {
        return Err(format!("{root} 下未找到可用角色资源（基础模型 / 装备 / 时装）"));
    }

    Ok(CharacterCatalog {
        root,
        bases,
        slots,
        fashions,
        textures: tex_index
            .into_iter()
            .map(|(stem, path)| fskin::TexInfo { stem, path })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res_dir() -> String {
        format!("{}\\..\\res", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn gender_number() {
        assert_eq!(
            parse_gender_number("FS01_Upper.fskin"),
            ("f".into(), Some(1))
        );
        assert_eq!(
            parse_gender_number("MS36_Upper.fskin"),
            ("m".into(), Some(36))
        );
        // 各槽两字母前缀：首字母即性别（Armonr FS/MS、Boot FX/MX、Glove FW/MW、
        // Hat FM/MW、Lower FK/MW）
        assert_eq!(parse_gender_number("FX12_Boot.fskin"), ("f".into(), Some(12)));
        assert_eq!(parse_gender_number("MX12_Boot.fskin"), ("m".into(), Some(12)));
        assert_eq!(parse_gender_number("FW03.fskin"), ("f".into(), Some(3)));
        assert_eq!(parse_gender_number("MW03.fskin"), ("m".into(), Some(3)));
        assert_eq!(parse_gender_number("FM01.FSM"), ("f".into(), Some(1)));
        assert_eq!(parse_gender_number("MW01.FSM"), ("m".into(), Some(1)));
        assert_eq!(parse_gender_number("FK07_Lower.fskin"), ("f".into(), Some(7)));
        assert_eq!(parse_gender_number("MW07_Lower.fskin"), ("m".into(), Some(7)));
        // 特效网格 FK55 也是 f 前缀（无材质槽会被 scan_equip_dir 剔除，无影响）
        assert_eq!(
            parse_gender_number("FK55_Bip01 Footsteps01.fskin"),
            ("f".into(), Some(55))
        );
        assert_eq!(parse_gender_number("girl_Upper.fskin"), ("u".into(), None));
        assert_eq!(parse_gender_number("G080.fak"), ("u".into(), Some(80)));
    }

    #[test]
    fn discover_root() {
        // res 本级不是角色根，但其 system 子目录是（含 AvatarGirl）
        let root = find_character_root(&res_dir()).unwrap();
        assert!(root.to_lowercase().ends_with("system"), "root = {root}");
        // 本级即角色根
        assert_eq!(
            find_character_root(&root).unwrap().to_lowercase(),
            root.to_lowercase()
        );
        assert!(find_character_root(&format!("{}\\no_such", res_dir())).is_err());
    }

    #[test]
    fn scan_res_system() {
        let c = scan_character(&res_dir()).unwrap();
        // 基础模型：AvatarGirl（仓库样本必有）
        assert!(
            c.bases.iter().any(|b| b.key == "avatargirl"),
            "bases = {:?}",
            c.bases.iter().map(|b| b.key.clone()).collect::<Vec<_>>()
        );
        let girl = c.bases.iter().find(|b| b.key == "avatargirl").unwrap();
        assert!(girl.default_fskel.is_some(), "girl.FSKEL 应为默认骨架");
        assert!(girl.default_fskel.as_deref().unwrap().to_lowercase().contains("girl"));
        // 装备槽结构完整（目录缺失的槽 items 为空）
        assert_eq!(c.slots.len(), 5);
        let keys: Vec<&str> = c.slots.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["hat", "armonr", "lower", "glove", "boot"]);
        // 贴图索引覆盖 AvatarGirl 全目录（wf01 脸贴图等）
        assert!(c.textures.iter().any(|t| t.stem == "wf01"), "wf01 应在索引中");
    }

    /// 造样本后的装备扫描（res/system/Armonr 等，样本缺失时跳过）
    #[test]
    fn scan_equip_samples() {
        let c = scan_character(&res_dir()).unwrap();
        let armonr = c.slots.iter().find(|s| s.key == "armonr").unwrap();
        if armonr.items.is_empty() {
            eprintln!("跳过: 无 Armonr 装备样本");
            return;
        }
        let fs10 = armonr.items.iter().find(|i| i.name == "FS10_Upper.fskin");
        if let Some(item) = fs10 {
            assert_eq!(item.gender, "f");
            assert_eq!(item.number, Some(10));
            assert!(item.vertex_count > 0);
            assert!(item.triangle_count > 0);
            // 装备贴图应在全根索引中解析到
            assert!(
                item.texture_files.iter().all(|t| t.is_some()),
                "FS10 贴图未解析: {:?}",
                item.texture_files
            );
        }
        // 无材质特效网格不应出现在槽里
        assert!(armonr.items.iter().all(|i| !i.texture_files.is_empty()));
        // 排序：女在前
        let genders: Vec<&str> = armonr.items.iter().map(|i| i.gender.as_str()).collect();
        let first_m = genders.iter().position(|g| *g == "m");
        let last_f = genders.iter().rposition(|g| *g == "f");
        if let (Some(fm), Some(lf)) = (first_m, last_f) {
            assert!(lf < fm, "女体款应排在男性款前: {genders:?}");
        }
    }
}
