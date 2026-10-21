//! 虚拟资源层：磁盘目录与 CPK 资源包统一寻址。
//!
//! CPK 寻址格式 `cpk:<包文件路径>[\包内路径]`，如
//! `cpk:D:\game\system.cpk\AvatarGirl\act\bck1.FSKEL`。
//! 解析：剥离 `cpk:` 后沿路径祖先链找最长存在且扩展名为 .cpk 的磁盘文件，
//! 其后的分量即包内路径（'\' 分隔、大小写不敏感匹配，与游戏 CRC 查找一致）。
//! 所有读取接口先判 CPK 寻址，否则回落文件系统——上层命令无需感知两种模式。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub const CPK_PREFIX: &str = "cpk:";

/// 列举 / 遍历得到的子项。
#[derive(Clone, Debug)]
pub struct VfsEntry {
    pub name: String,
    /// 完整虚拟路径：目录模式 = 磁盘路径；CPK 模式 = `cpk:...\system.cpk\包内路径`
    pub path: String,
    pub is_dir: bool,
    /// 文件的字节长度（CPK 为解压后大小），目录为 0
    pub byte_len: u64,
}

pub fn is_cpk_path(v: &str) -> bool {
    v.starts_with(CPK_PREFIX)
}

/// 拆出 (CPK 包文件磁盘路径, 包内路径)。非 CPK 寻址返回 None；
/// 包内路径统一 '\' 分隔、无首尾分隔符，包根为空串。
pub fn split_cpk(vpath: &str) -> Option<(PathBuf, String)> {
    let rest = vpath.strip_prefix(CPK_PREFIX)?;
    // 沿祖先链从完整路径开始缩短，第一个"存在的 .cpk 文件"即最长匹配
    for anc in Path::new(rest).ancestors() {
        let is_cpk_file = anc.is_file()
            && anc
                .extension()
                .map(|e| e.to_string_lossy().eq_ignore_ascii_case("cpk"))
                .unwrap_or(false);
        if is_cpk_file {
            let n = anc.as_os_str().len();
            let inner = rest[n..].trim_matches(['\\', '/']).replace('/', "\\");
            return Some((anc.to_path_buf(), inner));
        }
    }
    // 未命中存在的包文件：仍按格式给出包路径让上层报"打开失败"，而不是当成磁盘目录
    let mut best: Option<PathBuf> = None;
    for anc in Path::new(rest).ancestors() {
        if anc
            .extension()
            .map(|e| e.to_string_lossy().eq_ignore_ascii_case("cpk"))
            .unwrap_or(false)
        {
            best = Some(anc.to_path_buf());
            break;
        }
    }
    let cpk = best?;
    let n = cpk.as_os_str().len();
    Some((cpk, rest[n..].trim_matches(['\\', '/']).replace('/', "\\")))
}

/// 拼接虚拟子路径（root 去尾分隔符后接 '\'）
pub fn join_vpath(root: &str, name: &str) -> String {
    format!("{}\\{}", root.trim_end_matches(['\\', '/']), name)
}

/// 打开的包：句柄 + 只读浏览索引。大包（如 system.cpk）两万余条目，
/// 若每次列举/读取都重新物化全表（沿父链拼全路径 + GBK 解码）代价极高，
/// 故打开时一次性构建目录索引并常驻（GUI 只读包，索引无失效问题）。
struct PackHandle {
    pack: Mutex<cpk::Cpk>,
    /// 全表条目快照（表序），下方索引均存此处下标
    entries: Vec<cpk::Entry>,
    /// 小写全路径 → 条目下标（表序首个；文件与目录条目均可定位）
    by_lower: HashMap<String, usize>,
    /// 小写目录路径（包根为空串）→ 直接子项条目下标（表序）。
    /// 目录条目可能缺失，深层文件路径同样能推导出各级目录。
    children: HashMap<String, Vec<usize>>,
}

fn cpk_cache() -> &'static Mutex<HashMap<PathBuf, Arc<PackHandle>>> {
    static CACHE: LazyLock<Mutex<HashMap<PathBuf, Arc<PackHandle>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    &CACHE
}

/// 打开（并缓存）CPK 包并构建目录索引；表解析与索引构建各只做一次
fn open_cpk(path: &Path) -> Result<Arc<PackHandle>, String> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Some(h) = cpk_cache().lock().unwrap().get(&key) {
        return Ok(h.clone());
    }
    let pack =
        cpk::Cpk::open(path).map_err(|e| format!("打开 CPK 失败 {}: {e}", path.display()))?;
    let entries = pack.entries();
    let mut by_lower = HashMap::with_capacity(entries.len());
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let lp = e.full_path.to_lowercase();
        // 末段为空的损坏条目不进子项表，避免拼出空段路径
        match lp.rfind('\\') {
            Some(p) if p + 1 < lp.len() => {
                children.entry(lp[..p].to_string()).or_default().push(i);
            }
            None if !lp.is_empty() => {
                children.entry(String::new()).or_default().push(i);
            }
            _ => {}
        }
        by_lower.entry(lp).or_insert(i);
    }
    let arc = Arc::new(PackHandle {
        pack: Mutex::new(pack),
        entries,
        by_lower,
        children,
    });
    cpk_cache().lock().unwrap().insert(key, arc.clone());
    Ok(arc)
}

/// 读出包内一个文件条目（大小写不敏感匹配全路径，索引 O(1) 定位）
fn read_cpk_entry(cpk_path: &Path, inner: &str) -> Result<Vec<u8>, String> {
    let h = open_cpk(cpk_path)?;
    let key = inner.to_lowercase();
    let entry = h
        .by_lower
        .get(&key)
        .map(|&i| &h.entries[i])
        .ok_or_else(|| format!("CPK 内文件不存在: {inner}"))?;
    if entry.is_dir {
        return Err(format!("CPK 内文件不存在: {inner}"));
    }
    let bytes = h
        .pack
        .lock()
        .unwrap()
        .read_file(entry)
        .map_err(|e| format!("读取 {inner} 失败: {e}"))?;
    Ok(bytes)
}

/// 读文件（两种模式统一入口）
pub fn read_file(vpath: &str) -> Result<Vec<u8>, String> {
    match split_cpk(vpath) {
        Some((cpk_path, inner)) => read_cpk_entry(&cpk_path, &inner),
        None => fs::read(vpath).map_err(|e| format!("读取失败: {e}")),
    }
}

/// 虚拟路径是否存在（文件或目录）
pub fn exists(vpath: &str) -> bool {
    match split_cpk(vpath) {
        Some((cpk_path, inner)) => {
            let Ok(h) = open_cpk(&cpk_path) else {
                return false;
            };
            if inner.is_empty() {
                return true;
            }
            // 条目自身（文件或目录条目），或作为隐式目录（有子项）均存在
            let key = inner.to_lowercase();
            h.by_lower.contains_key(&key) || h.children.contains_key(&key)
        }
        None => Path::new(vpath).exists(),
    }
}

/// 列举虚拟目录直接子项（顺序不定，由上层排序）
pub fn list_dir(vpath: &str) -> Result<Vec<VfsEntry>, String> {
    let Some((cpk_path, inner)) = split_cpk(vpath) else {
        return list_fs_dir(vpath);
    };
    let h = open_cpk(&cpk_path)?;
    let key = inner.to_lowercase();
    // 直接子项按首段聚合：目录条目与文件路径都能推导出目录（包内目录条目可能缺失）
    let off = if key.is_empty() { 0 } else { key.len() + 1 };
    let mut agg: BTreeMap<String, (bool, u64, String)> = BTreeMap::new();
    if let Some(kids) = h.children.get(&key) {
        for &i in kids {
            let e = &h.entries[i];
            let lp = e.full_path.to_lowercase();
            let rest = lp.get(off..).unwrap_or_default();
            if rest.is_empty() {
                continue;
            }
            let first = rest.split('\\').next().unwrap_or_default().to_string();
            // 显示名保留条目存储的大小写（键与全路径仅 ASCII 大小写差异，长度一致）
            let display = e
                .full_path
                .get(off..)
                .unwrap_or_default()
                .split('\\')
                .next()
                .unwrap_or_default()
                .to_string();
            let slot = agg.entry(first).or_insert((false, 0, display));
            if rest.contains('\\') || e.is_dir {
                slot.0 = true; // 有更深路径或是目录条目 → first 是目录
            } else {
                slot.1 = e.origin_size as u64;
            }
        }
    }
    Ok(agg
        .into_iter()
        .map(|(name, (is_dir, byte_len, display))| VfsEntry {
            name: display,
            path: join_vpath(vpath, &name),
            is_dir,
            byte_len,
        })
        .collect())
}

fn list_fs_dir(dir: &str) -> Result<Vec<VfsEntry>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    let mut out = Vec::new();
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
        let byte_len = if ft.is_dir() {
            0
        } else {
            e.metadata().map(|m| m.len()).unwrap_or(0)
        };
        out.push(VfsEntry {
            name,
            path: p.to_string_lossy().into_owned(),
            is_dir: ft.is_dir(),
            byte_len,
        });
    }
    Ok(out)
}

/// 递归收集扩展名匹配的文件（大小写不敏感；CPK 模式直接全表过滤）
pub fn walk_files(vroot: &str, exts: &[&str]) -> Vec<VfsEntry> {
    let ext_match = |p: &str| -> bool {
        p.rsplit('.')
            .next()
            .map(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
            .unwrap_or(false)
    };
    match split_cpk(vroot) {
        Some((cpk_path, inner)) => {
            let Ok(h) = open_cpk(&cpk_path) else {
                return Vec::new();
            };
            let key = inner.to_lowercase();
            // 沿子项索引深度优先收集子树文件下标（只碰子树，不扫全表）
            fn walk_dir(
                h: &PackHandle,
                key: &str,
                hits: &mut Vec<usize>,
                ext_match: &dyn Fn(&str) -> bool,
            ) {
                let Some(kids) = h.children.get(key) else {
                    return;
                };
                let off = if key.is_empty() { 0 } else { key.len() + 1 };
                for &i in kids {
                    let e = &h.entries[i];
                    let lp = e.full_path.to_lowercase();
                    let first = lp
                        .get(off..)
                        .unwrap_or_default()
                        .split('\\')
                        .next()
                        .unwrap_or_default();
                    if first.is_empty() {
                        continue;
                    }
                    let child = if key.is_empty() {
                        first.to_string()
                    } else {
                        format!("{key}\\{first}")
                    };
                    if e.is_dir || h.children.contains_key(&child) {
                        walk_dir(h, &child, hits, ext_match);
                    } else if ext_match(&e.full_path) {
                        hits.push(i);
                    }
                }
            }
            let mut hits = Vec::new();
            walk_dir(&h, &key, &mut hits, &ext_match);
            hits.sort_unstable(); // 表序输出，与全表过滤的历史顺序一致
            let off = if inner.is_empty() { 0 } else { inner.len() + 1 };
            hits.into_iter()
                .map(|i| {
                    let e = &h.entries[i];
                    VfsEntry {
                        // 子路径 = full_path 去掉根前缀（仅 ASCII 大小写差异，长度相同）
                        path: join_vpath(vroot, e.full_path.get(off..).unwrap_or(&e.full_path)),
                        name: e.name.clone(),
                        is_dir: false,
                        byte_len: e.origin_size as u64,
                    }
                })
                .collect()
        }
        None => {
            let mut out = Vec::new();
            let mut stack = vec![vroot.trim_end_matches(['\\', '/']).to_string()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let p = e.path();
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if name.is_empty() {
                        continue;
                    }
                    if p.is_dir() {
                        stack.push(p.to_string_lossy().into_owned());
                    } else if ext_match(&name) {
                        out.push(VfsEntry {
                            name,
                            path: p.to_string_lossy().into_owned(),
                            is_dir: false,
                            byte_len: e.metadata().map(|m| m.len()).unwrap_or(0),
                        });
                    }
                }
            }
            out
        }
    }
}

// ---------------- 顺序读取器 ----------------

enum ReaderInner {
    File(fs::File),
    Mem(Cursor<Vec<u8>>),
}

/// 两种模式统一的顺序读取器：轻量解析只读头部、避免整读大文件；
/// CPK 条目解压后整段驻留内存（资源文件普遍在百 KB 量级）。
pub struct VfsReader(ReaderInner);

impl Read for VfsReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.0 {
            ReaderInner::File(f) => f.read(buf),
            ReaderInner::Mem(c) => c.read(buf),
        }
    }
}

pub fn open_reader(vpath: &str) -> Result<VfsReader, String> {
    match split_cpk(vpath) {
        Some((cpk_path, inner)) => Ok(VfsReader(ReaderInner::Mem(Cursor::new(read_cpk_entry(
            &cpk_path, &inner,
        )?)))),
        None => {
            let f = fs::File::open(vpath).map_err(|e| format!("打开失败: {e}"))?;
            Ok(VfsReader(ReaderInner::File(f)))
        }
    }
}

// ---------------- 测试辅助 ----------------

/// 测试辅助：把 res/system 的三个样例目录按原结构打包到临时 CPK。
/// add_path 打目录时取内容进包根，故先复制出 staging 结构；tag 区分并行测试。
#[cfg(test)]
pub(crate) fn build_system_cpk(tag: &str) -> PathBuf {
    fn copy_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for e in fs::read_dir(src).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap();
            if p.is_dir() {
                copy_dir(&p, &dst.join(name));
            } else {
                fs::copy(&p, dst.join(name)).unwrap();
            }
        }
    }
    let res = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../res"));
    let dir = std::env::temp_dir().join(format!("gongfu_cpk_{tag}"));
    let _ = fs::remove_dir_all(&dir);
    let staging = dir.join("staging");
    for sub in ["AvatarGirl", "Monster/G080", "Weapon/Jian"] {
        copy_dir(&res.join("system").join(sub), &staging.join(sub));
    }
    let cpk_path = dir.join("test.cpk");
    let mut pack = cpk::Cpk::create(&cpk_path, 64).unwrap();
    pack.add_path(&staging, &cpk::AddOpts::default(), &mut |_, _, _| {})
        .unwrap();
    cpk_path
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res");

    fn vroot(cpk_path: &Path) -> String {
        format!("cpk:{}", cpk_path.display())
    }

    #[test]
    fn split_cpk_addresses() {
        let (pack, inner) =
            split_cpk("cpk:D:\\game\\system.cpk\\AvatarGirl\\act\\bck1.FSKEL").unwrap();
        assert_eq!(pack, PathBuf::from("D:\\game\\system.cpk"));
        assert_eq!(inner, "AvatarGirl\\act\\bck1.FSKEL");
        // 正斜杠混写 + 尾分隔符归一
        let (_, inner) = split_cpk("cpk:D:/game/system.cpk/AvatarGirl/").unwrap();
        assert_eq!(inner, "AvatarGirl");
        // 非法（不存在的包文件按格式仍拆出包路径供上层报错）
        let (pack, _) = split_cpk("cpk:X:\\no\\such.cpk\\a").unwrap();
        assert_eq!(pack, PathBuf::from("X:\\no\\such.cpk"));
        // 目录模式不受影响
        assert!(split_cpk("D:\\game\\res").is_none());
    }

    #[test]
    fn list_cpk_matches_fs() {
        let cpk_path = build_system_cpk("list");
        let root = vroot(&cpk_path);
        // 包根 = 打包的三个顶层目录
        let names: Vec<String> = list_dir(&root)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        for n in ["AvatarGirl", "Monster", "Weapon"] {
            assert!(names.iter().any(|x| x.eq_ignore_ascii_case(n)), "{n} 缺失");
        }
        // 子目录列举与磁盘目录条目一致（名称 + 类型 + 大小）
        let disk = list_fs_dir(&format!("{RES}/system/AvatarGirl")).unwrap();
        let pack = list_dir(&join_vpath(&root, "AvatarGirl")).unwrap();
        for d in &disk {
            let hit = pack
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(&d.name))
                .unwrap_or_else(|| panic!("{} 缺失", d.name));
            assert_eq!(hit.is_dir, d.is_dir, "{} 类型不符", d.name);
            if !d.is_dir {
                assert_eq!(hit.byte_len, d.byte_len, "{} 大小不符", d.name);
            }
        }
        let _ = fs::remove_dir_all(cpk_path.parent().unwrap());
    }

    #[test]
    fn read_cpk_matches_fs() {
        let cpk_path = build_system_cpk("read");
        let root = vroot(&cpk_path);
        let rel = "AvatarGirl\\act\\bck1.FSKEL";
        let from_cpk = read_file(&join_vpath(&root, rel)).unwrap();
        let from_disk = fs::read(format!("{RES}/system/{rel}")).unwrap();
        assert_eq!(from_cpk, from_disk);
        // 前缀读取（reader 走 Cursor）
        let mut r = open_reader(&join_vpath(&root, rel)).unwrap();
        let mut head = vec![0u8; 0x118];
        r.read_exact(&mut head).unwrap();
        assert_eq!(head[..4], from_disk[..4]);
        // 大小写不敏感
        let upper = join_vpath(&root, "AvatarGirl\\ACT\\BCK1.FSKEL");
        assert_eq!(read_file(&upper).unwrap().len(), from_disk.len());
        assert!(exists(&join_vpath(&root, rel)));
        assert!(!exists(&join_vpath(&root, "AvatarGirl\\no_such.bin")));
        let _ = fs::remove_dir_all(cpk_path.parent().unwrap());
    }

    #[test]
    fn walk_files_cpk_recursive() {
        let cpk_path = build_system_cpk("walk");
        let root = vroot(&cpk_path);
        let girl = walk_files(&join_vpath(&root, "AvatarGirl"), &["fskin"]);
        let disk = walk_files(&format!("{RES}/system/AvatarGirl"), &["fskin"]);
        assert_eq!(girl.len(), disk.len());
        for e in &girl {
            if let Err(err) = read_file(&e.path) {
                panic!("读取失败 {}: {err}", e.path);
            }
        }
        let act = walk_files(&join_vpath(&root, "AvatarGirl\\act"), &["fskel"]);
        assert!(act.len() >= 18, "act 下 FSKEL 应有 18+，实际 {}", act.len());
        let _ = fs::remove_dir_all(cpk_path.parent().unwrap());
    }

    /// 本机大包性能冒烟（手动 `--ignored` 运行；包不存在则跳过）
    #[test]
    #[ignore = "需要本地 E:\\Workspace\\res\\system.cpk"]
    fn real_system_cpk_perf() {
        let p = Path::new("E:\\Workspace\\res\\system.cpk");
        if !p.is_file() {
            return;
        }
        let root = format!("cpk:{}", p.display());
        let t = std::time::Instant::now();
        let top = list_dir(&root).unwrap();
        println!("根列举: {:?}（{} 项）", t.elapsed(), top.len());
        let sys = top
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("system"))
            .unwrap();
        let t = std::time::Instant::now();
        let subs = crate::res::list_dir(&sys.path).unwrap();
        println!(
            "system 列举（含逐子目录 is_model）: {:?}（{} 项）",
            t.elapsed(),
            subs.len()
        );
        let mon = subs
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("Monster"))
            .unwrap();
        let t = std::time::Instant::now();
        let ms = crate::res::list_dir(&mon.path).unwrap();
        println!(
            "Monster 列举（含逐子目录 is_model）: {:?}（{} 项）",
            t.elapsed(),
            ms.len()
        );
    }

    /// 高层链路：CPK 寻址下的整体模型扫描与动作扫描（fskin/fskel 均走 vfs）
    #[test]
    fn cpk_model_and_act_scan() {
        let cpk_path = build_system_cpk("scan");
        let root = vroot(&cpk_path);
        let girl = join_vpath(&root, "AvatarGirl");
        let set = crate::fskin::scan_model_set(&girl).unwrap();
        assert!(
            set.groups.iter().any(|g| g
                .parts
                .iter()
                .any(|p| p.file_name.eq_ignore_ascii_case("girl_head.fskin"))),
            "换装分组应含 girl_head"
        );
        assert!(!set.tex_slots.is_empty(), "TFace 纹理槽应被识别");
        let briefs = crate::fskel::scan_dir(&join_vpath(&girl, "act")).unwrap();
        assert!(briefs.iter().all(|b| b.bone_count > 0));
        // 骨架解析（get_or_load 走 vfs 读取）
        let skel = crate::fskel::get_or_load(&briefs[0].path).unwrap();
        assert!(skel.meta().frame_count > 0);
        let _ = fs::remove_dir_all(cpk_path.parent().unwrap());
    }
}
