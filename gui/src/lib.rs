mod dds;
mod dress;
mod fak;
mod fskel;
mod fskin;
mod fsm;
mod obj;
mod res;
mod vfs;

use base64::Engine;
use serde::Serialize;

/// CPK 包内路径暂不支持写回（读浏览/预览已支持）
fn reject_cpk_write(path: &str) -> Result<(), String> {
    if vfs::is_cpk_path(path) {
        Err("暂不支持保存到 CPK 包内，请解包为目录后编辑".into())
    } else {
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TexImageDto {
    width: u32,
    height: u32,
    rgba_base64: String,
}

// 重 IO / 解析命令统一 async + spawn_blocking：非 async 命令在主线程执行，
// 大包扫描/解析会冻结窗口（资源树卡顿的另一半原因）。

#[tauri::command]
async fn scan_model_set(dir: String) -> Result<fskin::ModelSet, String> {
    run_blocking(move || fskin::scan_model_set(&dir)).await
}

/// 角色装扮目录扫描：基础模型 / 装备槽 / 时装 + 全根贴图索引
#[tauri::command]
async fn scan_character(root: String) -> Result<dress::CharacterCatalog, String> {
    run_blocking(move || dress::scan_character(&root)).await
}

#[tauri::command]
async fn load_fskin(path: String) -> Result<fskin::FskinModel, String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&path)?;
        fskin::parse_fskin(&path, &bytes)
    })
    .await
}

#[tauri::command]
async fn save_fskin(path: String, model: fskin::FskinModel) -> Result<(), String> {
    run_blocking(move || {
        reject_cpk_write(&path)?;
        let bytes = fskin::serialize_fskin(&model)?;
        std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
    })
    .await
}

#[tauri::command]
async fn load_texture(path: String) -> Result<TexImageDto, String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&path)?;
        let t = dds::decode_dds(&bytes)?;
        Ok(TexImageDto {
            width: t.width,
            height: t.height,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode(t.rgba),
        })
    })
    .await
}

#[tauri::command]
async fn export_obj(
    paths: Vec<String>,
    out_dir: String,
    base_name: String,
) -> Result<String, String> {
    run_blocking(move || obj::export_obj(&paths, &out_dir, &base_name)).await
}

/// 读取任意资源文件原始字节（base64）：通用图片（png/jpg/bmp 等）由前端 webview 解码
#[tauri::command]
async fn read_file_b64(path: String) -> Result<String, String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&path)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
    .await
}

/// 根据当前顶点重算包围盒，返回新值（前端确认后写回）
#[tauri::command]
async fn compute_bbox(model: fskin::FskinModel) -> Result<([f32; 3], [f32; 3]), String> {
    run_blocking(move || Ok(fskin::compute_bbox(&model))).await
}

// ---------------- FSKEL 动作 ----------------

#[tauri::command]
async fn scan_act_dir(dir: String) -> Result<Vec<fskel::FskelBrief>, String> {
    run_blocking(move || fskel::scan_dir(&dir)).await
}

#[tauri::command]
async fn load_fskel_meta(path: String) -> Result<fskel::FskelMeta, String> {
    run_blocking(move || Ok(fskel::get_or_load(&path)?.meta())).await
}

/// [from, to) 帧的扁平姿态：帧 × 骨 × (tx,ty,tz,qx,qy,qz,qw)
#[tauri::command]
async fn load_fskel_frames(path: String, from: u32, to: u32) -> Result<Vec<f32>, String> {
    run_blocking(move || Ok(fskel::get_or_load(&path)?.frames_flat(from, to))).await
}

#[tauri::command]
async fn export_fskel_bvh(
    path: String,
    action_index: Option<usize>,
    out_path: String,
) -> Result<String, String> {
    run_blocking(move || {
        let f = fskel::get_or_load(&path)?;
        fskel::export_bvh(&f, action_index, &out_path)
    })
    .await
}

#[tauri::command]
async fn export_fskel_glb(path: String, out_path: String) -> Result<String, String> {
    run_blocking(move || {
        let f = fskel::get_or_load(&path)?;
        fskel::export_glb(&f, &out_path)
    })
    .await
}

// ---------------- 资源浏览 ----------------

/// .fak 顶点动画模型（含全部帧顶点数据）
#[tauri::command]
async fn load_fak(path: String) -> Result<fak::FakModel, String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&path)?;
        fak::parse_fak(&path, &bytes)
    })
    .await
}

/// 保存 .fak（未修改的模型字节级还原，见 fak roundtrip 测试）
#[tauri::command]
async fn save_fak(path: String, model: fak::FakModel) -> Result<(), String> {
    run_blocking(move || {
        reject_cpk_write(&path)?;
        let bytes = fak::serialize_fak(&model)?;
        std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
    })
    .await
}

/// .fsm 静态武器网格（含顶点 / 材质 / 挂点）
#[tauri::command]
async fn load_fsm(path: String) -> Result<fsm::FsmModel, String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&path)?;
        fsm::parse_fsm(&path, &bytes)
    })
    .await
}

/// 保存 .fsm（未修改的模型字节级还原，见 fsm roundtrip 测试）
#[tauri::command]
async fn save_fsm(path: String, model: fsm::FsmModel) -> Result<(), String> {
    run_blocking(move || {
        reject_cpk_write(&path)?;
        let bytes = fsm::serialize_fsm(&model)?;
        std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
    })
    .await
}

/// 列举资源目录直接子项（供资源树懒加载 / 资源网格）
#[tauri::command]
async fn list_res_dir(dir: String) -> Result<Vec<res::ResEntry>, String> {
    run_blocking(move || res::list_dir(&dir)).await
}

/// 目录配套的默认骨架（轻量：只列 act 目录，不做全量模型扫描）。
/// 供前端整体预览在装配部件前先换骨架——部件装配与骨架必须配套，
/// 且须先卸旧部件再换（检测状态随部件拟合，顺序见装扮模式 loadBase）
#[tauri::command]
async fn dir_default_fskel(dir: String) -> Result<Option<String>, String> {
    run_blocking(move || Ok(fskin::default_fskel_for_dir(&dir))).await
}

// ---------------- 资源导出 / 导入 ----------------

/// 导出单个资源文件到磁盘目标路径（CPK 包内文件同样可导出）
#[tauri::command]
async fn export_file(src: String, dest: String) -> Result<(), String> {
    run_blocking(move || {
        let bytes = vfs::read_file(&src)?;
        if let Some(p) = std::path::Path::new(&dest).parent() {
            std::fs::create_dir_all(p).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        std::fs::write(&dest, bytes).map_err(|e| format!("写入失败: {e}"))
    })
    .await
}

/// 右键导出模型（fskin / fsm / fak）：原样复制模型 + 材质关联纹理一并导出到同目录
#[tauri::command]
async fn export_model(src: String, dest: String) -> Result<String, String> {
    run_blocking(move || obj::export_model(&src, &dest)).await
}

/// 递归导出目录（磁盘目录或 CPK 包内子树）到目标目录下（以源目录名单独建子目录）
#[tauri::command]
async fn export_dir(src: String, dest_dir: String) -> Result<String, String> {
    run_blocking(move || {
        let files = vfs::collect_tree(&src)?;
        // 目标子目录名 = 源目录最后一段（CPK 包根去掉 .cpk 扩展名）
        let base = src.trim_end_matches(['\\', '/']);
        let mut name = base.rsplit(['\\', '/']).next().unwrap_or("export").to_string();
        if name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".cpk") {
            // 按扩展名字节宽度截断（名称主体可能含非 ASCII，长度不能按字面算）
            name.truncate(name.len() - 4);
        }
        let root = std::path::Path::new(&dest_dir).join(&name);
        std::fs::create_dir_all(&root).map_err(|e| format!("创建目录失败: {e}"))?;
        for (rel, vpath) in &files {
            let out = root.join(rel);
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p).map_err(|e| format!("创建目录失败: {e}"))?;
            }
            let bytes = vfs::read_file(vpath)?;
            std::fs::write(&out, bytes).map_err(|e| format!("写入失败: {e}"))?;
        }
        Ok(format!("已导出 {} 个文件到 {}", files.len(), root.display()))
    })
    .await
}

/// 导入磁盘文件到资源目录（同名覆盖；CPK 包内目录不支持写入）
#[tauri::command]
async fn import_file(src: String, dest_dir: String) -> Result<String, String> {
    run_blocking(move || {
        reject_cpk_write(&dest_dir)?;
        let name = std::path::Path::new(&src)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .ok_or_else(|| "源文件路径无效".to_string())?;
        let dest = std::path::Path::new(&dest_dir).join(&name);
        std::fs::copy(&src, &dest).map_err(|e| format!("复制失败: {e}"))?;
        Ok(dest.to_string_lossy().into_owned())
    })
    .await
}

/// 把阻塞计算移出主线程执行（任务 panic 时转为错误字符串）
async fn run_blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

/// F12 切换开发者工具（release 构建依赖 tauri 的 devtools feature）
#[tauri::command]
fn toggle_devtools(window: tauri::WebviewWindow) {
    if window.is_devtools_open() {
        window.close_devtools();
    } else {
        window.open_devtools();
    }
}

#[cfg_attr(mobile, tauri::mobile_entrypoint)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            scan_model_set,
            scan_character,
            load_fskin,
            save_fskin,
            load_texture,
            export_obj,
            read_file_b64,
            compute_bbox,
            toggle_devtools,
            scan_act_dir,
            load_fskel_meta,
            load_fskel_frames,
            export_fskel_bvh,
            export_fskel_glb,
            load_fak,
            save_fak,
            load_fsm,
            save_fsm,
            list_res_dir,
            dir_default_fskel,
            export_file,
            export_model,
            export_dir,
            import_file
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 失败");
}

/// 供 ddsdump 示例与测试使用
pub fn dds_decode_for_test(bytes: &[u8]) -> Result<Vec<u8>, String> {
    dds::decode_dds(bytes).map(|t| t.rgba)
}

/// 诊断探针（fsm_bbox 示例用）：FSM 顶点数/bbox/挂点概览
pub fn fsm_probe_for_test(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    let m = fsm::parse_fsm(path, &bytes)?;
    let mut mn = [f32::MAX; 3];
    let mut mx = [f32::MIN; 3];
    for i in 0..m.vertex_count as usize {
        for k in 0..3 {
            let v = m.vertices[i * 8 + k];
            mn[k] = mn[k].min(v);
            mx[k] = mx[k].max(v);
        }
    }
    Ok(format!(
        "{}: {}v sub{} bbox y[{:.0},{:.0}] x[{:.0},{:.0}] z[{:.0},{:.0}] dummies[{}]",
        path.rsplit(['\\', '/']).next().unwrap_or(path),
        m.vertex_count,
        m.submeshes.len(),
        mn[1], mx[1], mn[0], mx[0], mn[2], mx[2],
        m.dummies.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(",")
    ))
}

/// 诊断探针（sz38_probe 示例用）：时装部件头发链排查——从 system.cpk 提取
/// shizhuang\SZ_38.fskin，列出 Bone* 链骨使用分布（镜像标志/顶点数/
/// authored Y 范围/帧0 LBS↔authored 残差）与链根顶点簇（识别马尾/披发链）。
pub fn sz38_probe_for_test(cpk_path: &str, fskel_path: &str) -> Result<String, String> {
    use std::collections::BTreeMap;

    let mut cpk = cpk::package::Cpk::open(cpk_path).map_err(|e| format!("打开 cpk 失败: {e:?}"))?;
    let entry = cpk
        .entries()
        .into_iter()
        .find(|e| e.full_path.to_lowercase().ends_with(r"\shizhuang\sz_38.fskin"))
        .ok_or(r"CPK 中找不到 shizhuang\SZ_38.fskin")?;
    let bytes = cpk.read_file(&entry).map_err(|e| format!("读取失败: {e:?}"))?;
    let m = fskin::parse_fskin("SZ_38.fskin", &bytes)?;

    let fb = std::fs::read(fskel_path).map_err(|e| format!("读 FSKEL 失败: {e}"))?;
    let f = fskel::parse_fskel(fskel_path, &fb)?;
    let meta = f.meta();
    let n = meta.bone_count as usize;
    let frame0 = f.frames_flat(0, 1);
    let mir = |b: usize| meta.mirror_first.get(b).copied().unwrap_or(0) != 0;

    // 帧0 世界矩阵：局部 = R(Q)·S(镜像)，世界 = 父世界 × 局部（与引擎合成同构）
    type M4 = [[f32; 4]; 4];
    let mul = |a: &M4, b: &M4| -> M4 {
        let mut o = [[0f32; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j] + a[i][3] * b[3][j];
            }
        }
        o
    };
    let mut wm: Vec<M4> = vec![[[0.0; 4]; 4]; n];
    let mut seen = vec![false; n];
    loop {
        let mut added = false;
        for b in 0..n {
            if seen[b] {
                continue;
            }
            let p = meta.parents[b];
            if p < 0 || seen[p as usize] {
                seen[b] = true;
                let o = b * 7;
                let (tx, ty, tz) = (frame0[o], frame0[o + 1], frame0[o + 2]);
                let (x, y, z, w) = (frame0[o + 3], frame0[o + 4], frame0[o + 5], frame0[o + 6]);
                let nn = (x * x + y * y + z * z + w * w).sqrt().max(1e-10);
                let (x, y, z, w) = (x / nn, y / nn, z / nn, w / nn);
                let s = if mir(b) { -1.0f32 } else { 1.0 };
                let mut l: M4 = [[0.0; 4]; 4];
                l[0][0] = (1.0 - 2.0 * (y * y + z * z)) * s;
                l[0][1] = (2.0 * (x * y + z * w)) * s;
                l[0][2] = (2.0 * (x * z - y * w)) * s;
                l[1][0] = (2.0 * (x * y - z * w)) * s;
                l[1][1] = (1.0 - 2.0 * (x * x + z * z)) * s;
                l[1][2] = (2.0 * (y * z + x * w)) * s;
                l[2][0] = (2.0 * (x * z + y * w)) * s;
                l[2][1] = (2.0 * (y * z - x * w)) * s;
                l[2][2] = (1.0 - 2.0 * (x * x + y * y)) * s;
                l[0][3] = tx;
                l[1][3] = ty;
                l[2][3] = tz;
                l[3][3] = 1.0;
                wm[b] = if (0..n).contains(&(p as usize)) { mul(&wm[p as usize], &l) } else { l };
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    let is_bone_name = |s: &str| s.len() > 4 && s.starts_with("Bone") && s[4..].bytes().all(|c| c.is_ascii_digit());
    let name = |b: usize| meta.bone_names[b].clone();
    let mut out = format!("== SZ_38: {} 顶点 · {} 子网格
", m.vertices.len(), m.submeshes.len());
    for (i, mat) in m.materials.iter().enumerate() {
        out += &format!("  材质#{i}: tex={}
", mat.texture_path);
    }

    let mut solo_of: BTreeMap<usize, usize> = BTreeMap::new();
    for v in &m.vertices {
        for inf in &v.influences {
            let b0 = inf.bone_index as usize;
            if b0 < n && is_bone_name(&name(b0)) && (v.influences.len() == 1 || inf.weight >= 0.99) {
                *solo_of.entry(b0).or_insert(0) += 1;
            }
        }
    }
    out += &format!("  单影响计数: {}
", solo_of.iter().map(|(b, c)| format!("{}#{}:{}", name(*b), b, c)).collect::<Vec<_>>().join(" "));
    let mut stats: BTreeMap<usize, (usize, f32, f32, f64, f32)> = BTreeMap::new();
    for v in &m.vertices {
        for inf in &v.influences {
            let b = inf.bone_index as usize;
            if b >= n || !is_bone_name(&name(b)) {
                continue;
            }
            let mm = &wm[b];
            let px = mm[0][0] * inf.bone_local_pos[0] + mm[0][1] * inf.bone_local_pos[1] + mm[0][2] * inf.bone_local_pos[2] + mm[0][3];
            let py = mm[1][0] * inf.bone_local_pos[0] + mm[1][1] * inf.bone_local_pos[1] + mm[1][2] * inf.bone_local_pos[2] + mm[1][3];
            let pz = mm[2][0] * inf.bone_local_pos[0] + mm[2][1] * inf.bone_local_pos[1] + mm[2][2] * inf.bone_local_pos[2] + mm[2][3];
            let res = ((px - v.pos[0]).powi(2) + (py - v.pos[1]).powi(2) + (pz - v.pos[2]).powi(2)).sqrt();
            let e = stats.entry(b).or_insert((0, f32::MAX, f32::MIN, 0.0, 0.0));
            e.0 += 1;
            e.1 = e.1.min(v.pos[1]);
            e.2 = e.2.max(v.pos[1]);
            e.3 += res as f64;
            e.4 = e.4.max(res);
        }
    }
    out += "== Bone* 链骨使用（· 镜像 · 顶点数 · authoredY · LBS残差 avg/max @帧0）
";
    let mut roots: Vec<usize> = Vec::new();
    {
        let mut root_set: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for (&b, _) in &stats {
            let mut cur = b;
            loop {
                let p = meta.parents[cur];
                if p < 0 || !is_bone_name(&name(p as usize)) {
                    break;
                }
                cur = p as usize;
            }
            root_set.insert(cur);
        }
        roots = root_set.into_iter().collect();
        out += &format!("  所属链根: {}
", roots.iter().map(|&r| format!("{}#{}", name(r), r)).collect::<Vec<_>>().join(" "));
    }
    for (&b, s) in &stats {
        let _ = s;
        let p = meta.parents[b];
        let is_root = p < 0 || !is_bone_name(&name(p as usize));
        if is_root {
            roots.push(b);
        }
        out += &format!(
            "  #{} {} 镜像={} n={} y[{:.0},{:.0}] res {:.1}/{:.1}{}
",
            b, name(b), mir(b), s.0, s.1, s.2, s.3 / s.0 as f64, s.4,
            if is_root { " ←链根" } else { "" }
        );
    }
    out += "== 链根顶点簇
";
    for &r in &roots {
        let mut members = vec![r];
        loop {
            let mut added = false;
            for b in 0..n {
                let p = meta.parents[b];
                if p >= 0 && is_bone_name(&name(b)) && members.contains(&(p as usize)) && !members.contains(&b) {
                    members.push(b);
                    added = true;
                }
            }
            if !added {
                break;
            }
        }
        let (mut ymin, mut ymax, mut cnt) = (f32::MAX, f32::MIN, 0usize);
        for v in &m.vertices {
            for inf in &v.influences {
                if members.contains(&(inf.bone_index as usize)) {
                    ymin = ymin.min(v.pos[1]);
                    ymax = ymax.max(v.pos[1]);
                    cnt += 1;
                }
            }
        }
        let names: Vec<String> = members.iter().map(|&b| format!("{}#{}", name(b), b)).collect();
        out += &format!(
            "  根 {}#{} 镜像={} 成员[{}] 影响{}次 authoredY[{:.0},{:.0}]
",
            name(r), r, mir(r), names.join(","), cnt, ymin, ymax
        );
    }
    // ---- 扩展：基础发型件的链使用 / 镜像变体 / wlk 摆动 ----
    let chains: Vec<usize> = vec![90, 91, 92, 103, 104];
    let base_dir = std::path::Path::new(fskel_path).parent().unwrap().parent().unwrap();
    out += "== 基础发型件的 Bone* 链使用\n";
    let mut hair_files: Vec<std::path::PathBuf> = vec![];
    if let Ok(rd) = std::fs::read_dir(base_dir.join("MHair")) {
        for e2 in rd.flatten() {
            if e2.path().extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case("fskin")).unwrap_or(false) {
                hair_files.push(e2.path());
            }
        }
    }
    hair_files.push(base_dir.join("girl_Hair01.fskin"));
    hair_files.sort();
    for hp in hair_files {
        let Ok(b2) = std::fs::read(&hp) else { continue };
        let Ok(m2) = fskin::parse_fskin(&hp.to_string_lossy(), &b2) else { continue };
        let mut used: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for v in &m2.vertices {
            for inf in &v.influences {
                let bi = inf.bone_index as usize;
                if bi < n && chains.contains(&bi) {
                    used.insert(format!("{}#{}", name(bi), bi));
                }
            }
        }
        if !used.is_empty() {
            out += &format!("  {} 使用 {}\n", hp.file_name().unwrap().to_string_lossy(), used.into_iter().collect::<Vec<_>>().join(","));
        }
    }
    // 镜像变体残差（帧0）：逐链根试镜像
    for &r in &roots {
        let members: Vec<usize> = {
            let mut members = vec![r];
            loop {
                let mut added = false;
                for b in 0..n {
                    let pp = meta.parents[b];
                    if pp >= 0 && is_bone_name(&name(b)) && members.contains(&(pp as usize)) && !members.contains(&b) {
                        members.push(b);
                        added = true;
                    }
                }
                if !added { break; }
            }
            members
        };
        let mut wm2: Vec<M4> = wm.clone();
        {
            let mut seen2 = vec![false; n];
            loop {
                let mut added = false;
                for b in 0..n {
                    if seen2[b] { continue; }
                    let pp = meta.parents[b];
                    if pp < 0 || seen2[pp as usize] {
                        seen2[b] = true;
                        let o = b * 7;
                        let (tx, ty, tz) = (frame0[o], frame0[o + 1], frame0[o + 2]);
                        let (x, y, z, w) = (frame0[o + 3], frame0[o + 4], frame0[o + 5], frame0[o + 6]);
                        let nn = (x * x + y * y + z * z + w * w).sqrt().max(1e-10);
                        let (x, y, z, w) = (x / nn, y / nn, z / nn, w / nn);
                        let sc = if mir(b) || members.contains(&b) { -1.0f32 } else { 1.0 };
                        let mut l: M4 = [[0.0; 4]; 4];
                        l[0][0] = (1.0 - 2.0 * (y * y + z * z)) * sc;
                        l[0][1] = (2.0 * (x * y + z * w)) * sc;
                        l[0][2] = (2.0 * (x * z - y * w)) * sc;
                        l[1][0] = (2.0 * (x * y - z * w)) * sc;
                        l[1][1] = (1.0 - 2.0 * (x * x + z * z)) * sc;
                        l[1][2] = (2.0 * (y * z + x * w)) * sc;
                        l[2][0] = (2.0 * (x * z + y * w)) * sc;
                        l[2][1] = (2.0 * (y * z - x * w)) * sc;
                        l[2][2] = (1.0 - 2.0 * (x * x + y * y)) * sc;
                        l[0][3] = tx;
                        l[1][3] = ty;
                        l[2][3] = tz;
                        l[3][3] = 1.0;
                        wm2[b] = if (0..n).contains(&(pp as usize)) { mul(&wm2[pp as usize], &l) } else { l };
                        added = true;
                    }
                }
                if !added { break; }
            }
        }
        let mut sum = 0.0f64;
        let mut cnt = 0usize;
        for v in &m.vertices {
            for inf in &v.influences {
                if !members.contains(&(inf.bone_index as usize)) { continue; }
                let mm = &wm2[inf.bone_index as usize];
                let px = mm[0][0] * inf.bone_local_pos[0] + mm[0][1] * inf.bone_local_pos[1] + mm[0][2] * inf.bone_local_pos[2] + mm[0][3];
                let py = mm[1][0] * inf.bone_local_pos[0] + mm[1][1] * inf.bone_local_pos[1] + mm[1][2] * inf.bone_local_pos[2] + mm[1][3];
                let pz = mm[2][0] * inf.bone_local_pos[0] + mm[2][1] * inf.bone_local_pos[1] + mm[2][2] * inf.bone_local_pos[2] + mm[2][3];
                sum += ((px - v.pos[0]).powi(2) + (py - v.pos[1]).powi(2) + (pz - v.pos[2]).powi(2)).sqrt() as f64;
                cnt += 1;
            }
        }
        if cnt > 0 {
            out += &format!("  链根 {}#{} 镜像变体残差 avg {:.1}\n", name(r), r, sum / cnt as f64);
        }
    }
    if let Some(a) = meta.actions.iter().find(|a| a.name == "wlk1") {
        let (f0, f1) = ((a.first_frame as usize).max(1), (a.first_frame as usize + a.frame_count as usize).min(f.frame_count() as usize));
        out += &format!("== wlk1 帧[{f0},{f1}) 链动态（局部平移摆动粗测）\n");
        for &r in &roots {
            let mut base: Option<[f64; 3]> = None;
            let mut swing = 0.0f64;
            for fi in (f0..f1).step_by(3) {
                let fr = f.frames_flat(fi as u32, fi as u32 + 1);
                let o = r * 7;
                let p = [fr[o] as f64, fr[o + 1] as f64, fr[o + 2] as f64];
                if let Some(b) = base {
                    let d = ((p[0] - b[0]).powi(2) + (p[1] - b[1]).powi(2) + (p[2] - b[2]).powi(2)).sqrt();
                    swing = swing.max(d);
                } else {
                    base = Some(p);
                }
            }
            out += &format!("  {}#{} 摆动 {:.2} · 帧根世界[{:.0},{:.0},{:.0}]\n", name(r), r, swing, wm[r][0][3], wm[r][1][3], wm[r][2][3]);
        }
    }
    Ok(out)
}

/// 诊断探针（skirt_probe 示例用）：裙摆防穿模/布料链排查——FSKEL 骨名、
/// 腿胶囊长度（与 viewer setupSkirtRig 同口径：len=子骨帧0局部平移长度，
/// r=0.18×len），以及各 FSKIN 部件中 Bone* 链骨的顶点影响分布
/// （权重范围、单影响顶点数、共混骨 top3）。
pub fn skirt_probe_for_test(fskel_path: &str, fskin_paths: &[String]) -> Result<String, String> {
    use std::collections::{BTreeMap, BTreeSet};

    let bytes = std::fs::read(fskel_path).map_err(|e| format!("读 FSKEL 失败: {e}"))?;
    let f = fskel::parse_fskel(fskel_path, &bytes)?;
    let meta = f.meta();
    let n = meta.bone_count as usize;
    let frame0 = f.frames_flat(0, 1);
    let mut out = format!("== FSKEL {}: {} 骨 · {} 帧\n", meta.file_name, n, meta.frame_count);

    let is_bone_name = |s: &str| s.len() > 4 && s.starts_with("Bone") && s[4..].bytes().all(|b| b.is_ascii_digit());

    for b in 0..n {
        let name = &meta.bone_names[b];
        if !(name.contains("Thigh") || name.contains("Calf") || name.contains("Foot")) {
            continue;
        }
        if let Some(c) = (0..n).find(|&c| meta.parents[c] == b as i32) {
            let o = c * 7;
            let len = (frame0[o] * frame0[o] + frame0[o + 1] * frame0[o + 1] + frame0[o + 2] * frame0[o + 2]).sqrt();
            out += &format!("  腿骨 {name}(#{b}): len={len:.2} 胶囊半径={:.2}\n", len * 0.18);
        }
    }

    let bone_ids: BTreeSet<usize> = (0..n).filter(|&b| is_bone_name(&meta.bone_names[b])).collect();
    out += &format!("  Bone* 链骨 {} 根\n", bone_ids.len());

    for path in fskin_paths {
        let bytes = std::fs::read(path).map_err(|e| format!("读 FSKIN 失败: {e}"))?;
        let m = fskin::parse_fskin(path, &bytes)?;
        let fname = path.rsplit(['\\', '/']).next().unwrap_or(path);
        out += &format!("== {fname} ({} 顶点)\n", m.vertices.len());
        struct St {
            cnt: usize,
            solo: usize,
            wmin: f32,
            wmax: f32,
            wsum: f64,
            co: BTreeMap<usize, usize>,
        }
        let mut stats: BTreeMap<usize, St> = BTreeMap::new();
        for v in &m.vertices {
            for inf in &v.influences {
                let bi = inf.bone_index as usize;
                if !bone_ids.contains(&bi) {
                    continue;
                }
                let s = stats.entry(bi).or_insert_with(|| St {
                    cnt: 0,
                    solo: 0,
                    wmin: f32::MAX,
                    wmax: 0.0,
                    wsum: 0.0,
                    co: BTreeMap::new(),
                });
                s.cnt += 1;
                s.wmin = s.wmin.min(inf.weight);
                s.wmax = s.wmax.max(inf.weight);
                s.wsum += inf.weight as f64;
                if v.influences.len() == 1 || inf.weight >= 0.99 {
                    s.solo += 1;
                }
                for other in &v.influences {
                    if (other.bone_index as usize) != bi {
                        *s.co.entry(other.bone_index as usize).or_insert(0) += 1;
                    }
                }
            }
        }
        if stats.is_empty() {
            out += "  (无 Bone* 链影响)\n";
            continue;
        }
        for (bi, s) in &stats {
            let bname = meta.bone_names.get(*bi).map(String::as_str).unwrap_or("?");
            let mut co: Vec<_> = s.co.iter().collect();
            co.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            let co_txt = co
                .iter()
                .take(3)
                .map(|(ob, c)| {
                    let oname = meta
                        .bone_names
                        .get(**ob)
                        .cloned()
                        .unwrap_or_else(|| format!("#{ob}"));
                    format!("{oname}(#{ob})×{c}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            out += &format!(
                "  {bname}(#{bi}): 影响 {} 顶点 · 权重 min/avg/max {:.2}/{:.2}/{:.2} · 单影响 {} · 共混 {}\n",
                s.cnt,
                s.wmin,
                s.wsum / s.cnt as f64,
                s.wmax,
                s.solo,
                if co_txt.is_empty() { "-" } else { &co_txt }
            );
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- 帽子挂接探针

type M3 = [[f32; 3]; 3]; // 行主序，v' = R·v
type V3 = [f32; 3];

fn m3_identity() -> M3 {
    [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ]
}

fn m3_mul(a: &M3, b: &M3) -> M3 {
    let mut r = [[0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            r[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    r
}

fn m3_apply(m: &M3, p: &V3) -> V3 {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
    ]
}

/// 四元数 (x,y,z,w) → 旋转矩阵（three.js 约定，与 viewer applyFskelFrame 一致）
fn quat_to_m3(q: &[f32; 4]) -> M3 {
    let [x, y, z, w] = *q;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        ],
        [
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        ],
        [
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

fn v3_add(a: &V3, b: &V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

struct WorldXf {
    r: M3,
    t: V3,
}

fn bbox_of(out: &mut String, name: &str, pts: &[V3]) -> ([f32; 3], [f32; 3]) {
    let mut mn = [f32::MAX; 3];
    let mut mx = [f32::MIN; 3];
    for p in pts {
        for i in 0..3 {
            mn[i] = mn[i].min(p[i]);
            mx[i] = mx[i].max(p[i]);
        }
    }
    let c = [
        (mn[0] + mx[0]) / 2.0,
        (mn[1] + mx[1]) / 2.0,
        (mn[2] + mx[2]) / 2.0,
    ];
    out.push_str(&format!(
        "{name:26} min[{:7.1},{:7.1},{:7.1}] max[{:7.1},{:7.1},{:7.1}] 中心[{:7.1},{:7.1},{:7.1}]\n",
        mn[0], mn[1], mn[2], mx[0], mx[1], mx[2], c[0], c[1], c[2]
    ));
    (mn, mx)
}

/// 诊断探针（hat_probe 示例用）：帽子挂接排查——FSKEL 帧0 头骨世界矩阵
/// （骨轴 +X 与 +Z 在数据空间的实际朝向）、FM01 帽模局部网格与挂点、
/// girl_head / girl_Hair01 蒙皮后包围盒，并模拟 viewer 当前挂接变换
/// （headWorld · (Rz(-90°)·v + lift·X)），量化前后偏差。
pub fn hat_probe_for_test(fskel_path: &str, fsm_path: &str, fskin_paths: &[String]) -> Result<String, String> {
    let bytes = std::fs::read(fskel_path).map_err(|e| format!("读 FSKEL 失败: {e}"))?;
    let f = fskel::parse_fskel(fskel_path, &bytes)?;
    let meta = f.meta();
    let n = meta.bone_count as usize;
    let pose = f.frames_flat(0, 1); // 帧0: 每骨 (t3, q4)
    let mut out = format!("== FSKEL {}: {n} 骨 · {} 帧\n", meta.file_name, meta.frame_count);

    // 世界矩阵（父链组合，parents 可能乱序，迭代到稳定）
    let mut world: Vec<Option<WorldXf>> = (0..n).map(|_| None).collect();
    let root_idx = (0..n).find(|&i| meta.parents[i] < 0).ok_or("无根骨")?;
    let rq = |i: usize| {
        let o = i * 7;
        (
            [pose[o], pose[o + 1], pose[o + 2]],
            quat_to_m3(&[
                pose[o + 3],
                pose[o + 4],
                pose[o + 5],
                pose[o + 6],
            ]),
        )
    };
    let (t0, r0) = rq(root_idx);
    world[root_idx] = Some(WorldXf { r: r0, t: t0 });
    let mut guard = 0;
    while world.iter().any(|w| w.is_none()) {
        guard += 1;
        if guard > 1000 {
            return Err("骨链解析死循环".into());
        }
        for i in 0..n {
            if world[i].is_none() {
                let p = meta.parents[i] as usize;
                if let Some(pw) = &world[p] {
                    let (t, r) = rq(i);
                    world[i] = Some(WorldXf {
                        r: m3_mul(&pw.r, &r),
                        t: v3_add(&m3_apply(&pw.r, &t), &pw.t),
                    });
                }
            }
        }
    }

    // 头骨：viewer autoHatBone 同款匹配（含 head / tou）
    let head = (0..n)
        .find(|&i| meta.bone_names[i].to_lowercase().contains("head"))
        .or_else(|| (0..n).find(|&i| meta.bone_names[i].to_lowercase().contains("tou")))
        .ok_or("无头骨")?;
    let hw = world[head].as_ref().unwrap();
    out += &format!(
        "== 头骨 {}(#{head}) 帧0: 世界原点 [{:.2},{:.2},{:.2}]\n",
        meta.bone_names[head], hw.t[0], hw.t[1], hw.t[2]
    );
    out += &format!(
        "  local t=[{:.2},{:.2},{:.2}] q=[{:.3},{:.3},{:.3},{:.3}]\n",
        pose[head * 7],
        pose[head * 7 + 1],
        pose[head * 7 + 2],
        pose[head * 7 + 3],
        pose[head * 7 + 4],
        pose[head * 7 + 5],
        pose[head * 7 + 6]
    );
    out += &format!(
        "  骨轴 +X(骨方向)=[{:.3},{:.3},{:.3}]  +Y=[{:.3},{:.3},{:.3}]  +Z=[{:.3},{:.3},{:.3}]\n",
        hw.r[0][0], hw.r[1][0], hw.r[2][0],
        hw.r[0][1], hw.r[1][1], hw.r[2][1],
        hw.r[0][2], hw.r[1][2], hw.r[2][2],
    );
    // 头骨父链（颈部走向）
    out += "  -- 父链(根→头) --\n";
    let mut chain = vec![head];
    while meta.parents[*chain.last().unwrap()] >= 0 {
        chain.push(meta.parents[*chain.last().unwrap()] as usize);
    }
    for &b in chain.iter().rev() {
        let w = world[b as usize].as_ref().unwrap();
        out += &format!(
            "  {:16} 世界t=[{:6.1},{:6.1},{:6.1}] local t=[{:.2},{:.2},{:.2}] q=[{:.3},{:.3},{:.3},{:.3}]\n",
            meta.bone_names[b],
            w.t[0], w.t[1], w.t[2],
            pose[b * 7],
            pose[b * 7 + 1],
            pose[b * 7 + 2],
            pose[b * 7 + 3],
            pose[b * 7 + 4],
            pose[b * 7 + 5],
            pose[b * 7 + 6]
        );
    }

    // ---- 帽模 FSM 局部网格 ----
    let bytes = std::fs::read(fsm_path).map_err(|e| format!("读 FSM 失败: {e}"))?;
    let hat = fsm::parse_fsm(fsm_path, &bytes)?;
    let hat_pts: Vec<V3> = hat
        .vertices
        .chunks(8)
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    out += &format!(
        "\n== {} 帽模（局部坐标, {} 顶点）\n",
        hat.path.rsplit(['\\', '/']).next().unwrap_or(&hat.path),
        hat.vertex_count
    );
    bbox_of(&mut out, "局部 bbox", &hat_pts);
    // 垂发侧判定：按 local Y 分层（下层=垂发/帽檐下发丝，顶层=冠环），
    // 各层的 Z 质心与范围 → 帽模不对称侧在 +Z 还是 −Z
    {
        let mut mn = [f32::MAX; 3];
        let mut mx = [f32::MIN; 3];
        for p in &hat_pts {
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
            }
        }
        let y_mid = (mn[1] + mx[1]) / 2.0;
        let mut layer = |name: &str, pred: &dyn Fn(&V3) -> bool| {
            let sel: Vec<V3> = hat_pts.iter().filter(|p| pred(p)).copied().collect();
            if sel.is_empty() {
                out += &format!("  {name}: 无顶点\n");
                return;
            }
            let n = sel.len();
            let cz = sel.iter().map(|p| p[2]).sum::<f32>() / n as f32;
            let mut zmn = f32::MAX;
            let mut zmx = f32::MIN;
            for p in &sel {
                zmn = zmn.min(p[2]);
                zmx = zmx.max(p[2]);
            }
            out += &format!("  {name}: {n} 点, Z∈[{zmn:.1},{zmx:.1}] 质心Z={cz:.1}\n");
        };
        layer("下层(y<y_mid, 垂发)", &|p: &V3| p[1] < y_mid);
        layer("顶层(y≥y_mid, 冠)", &|p: &V3| p[1] >= y_mid);
    }
    for d in &hat.dummies {
        out += &format!(
            "  dummy {} pos=[{:.2},{:.2},{:.2}] q=[{:.2},{:.2},{:.2},{:.2}]\n",
            d.name,
            d.values[0],
            d.values[1],
            d.values[2],
            d.values[3],
            d.values[4],
            d.values[5],
            d.values[6]
        );
    }

    // ---- 基础头/发 fskin 蒙皮到帧0 ----
    let mut skin_boxes: Vec<(String, [f32; 3], [f32; 3])> = Vec::new();
    for path in fskin_paths {
        let bytes = std::fs::read(path).map_err(|e| format!("读 FSKIN 失败: {e}"))?;
        let m = fskin::parse_fskin(path, &bytes)?;
        let fname = path.rsplit(['\\', '/']).next().unwrap_or(path).to_string();
        let mut pts: Vec<V3> = Vec::with_capacity(m.vertices.len());
        let mut bone_hits = std::collections::BTreeMap::new();
        for v in &m.vertices {
            let mut p = [0f32; 3];
            for inf in &v.influences {
                if let Some(w) = world.get(inf.bone_index as usize).and_then(|w| w.as_ref()) {
                    let lp = m3_apply(&w.r, &inf.bone_local_pos);
                    p = v3_add(
                        &p,
                        &[
                            (lp[0] + w.t[0]) * inf.weight,
                            (lp[1] + w.t[1]) * inf.weight,
                            (lp[2] + w.t[2]) * inf.weight,
                        ],
                    );
                    *bone_hits.entry(inf.bone_index).or_insert(0f32) += inf.weight;
                }
            }
            pts.push(p);
        }
        out += &format!("\n== {fname} 蒙皮@帧0 ({} 顶点)\n", m.vertices.len());
        let (mn, mx) = bbox_of(&mut out, &format!("{fname} bbox"), &pts);
        skin_boxes.push((fname.clone(), mn, mx));
        let mut hits: Vec<_> = bone_hits.into_iter().collect();
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        out += &format!(
            "  主影响骨: {}\n",
            hits.iter()
                .take(4)
                .map(|(b, w)| format!(
                    "{}(#{}, w={:.1})",
                    meta.bone_names.get(*b as usize).map(String::as_str).unwrap_or("?"),
                    b,
                    w
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let y_mid = (mn[1] + mx[1]) / 2.0;
        let upper: Vec<V3> = pts.iter().filter(|p| p[1] > y_mid).copied().collect();
        bbox_of(&mut out, &format!("{fname} 上半"), &upper);
    }

    // ---- 模拟 viewer 当前挂接: data = headWorld · (Rz(-90°)·v + lift·X) ----
    let rz = |v: &V3| -> V3 { [v[1], -v[0], v[2]] };
    for lift in [9.0f32, 0.0] {
        let sim: Vec<V3> = hat_pts
            .iter()
            .map(|v| {
                let b = rz(v);
                let b = [b[0] + lift, b[1], b[2]];
                v3_add(&m3_apply(&hw.r, &b), &hw.t)
            })
            .collect();
        out += &format!("\n== 模拟挂接 lift={lift}: data = headWorld·(Rz(-90°)v+({lift},0,0))\n");
        bbox_of(&mut out, "挂接后 bbox", &sim);
    }

    // ---- 朝向搜索：24 个轴对齐纯旋转 × 平移网格，按对称 chamfer 对齐帽模↔基础发 ----
    // （帽模替换基础发，两者同戴一颗头，最优刚体变换即游戏挂接约定）
    if let Some((hair_name, hmn, _hmx)) = skin_boxes.iter().find(|(n, _, _)| n.contains("Hair")) {
        let bytes = std::fs::read(fskin_paths.iter().find(|p| p.contains("Hair")).unwrap())
            .map_err(|e| format!("读 FSKIN 失败: {e}"))?;
        let hm = fskin::parse_fskin(fskin_paths.iter().find(|p| p.contains("Hair")).unwrap(), &bytes)?;
        let hair_pts: Vec<V3> = {
            let mut pts = Vec::with_capacity(hm.vertices.len());
            for v in &hm.vertices {
                let mut p = [0f32; 3];
                for inf in &v.influences {
                    if let Some(w) = world.get(inf.bone_index as usize).and_then(|w| w.as_ref()) {
                        let lp = m3_apply(&w.r, &inf.bone_local_pos);
                        p = v3_add(
                            &p,
                            &[
                                (lp[0] + w.t[0]) * inf.weight,
                                (lp[1] + w.t[1]) * inf.weight,
                                (lp[2] + w.t[2]) * inf.weight,
                            ],
                        );
                    }
                }
                pts.push(p);
            }
            pts
        };
        let _ = (hair_name, hmn);
        // 均匀下采样
        let step = |v: &[V3], want: usize| -> Vec<V3> {
            let s = (v.len() / want).max(1);
            v.iter().step_by(s).copied().collect()
        };
        let src = step(&hat_pts, 300);
        let dst = step(&hair_pts, 300);
        let chamfer = |a: &[V3], b: &[V3]| -> f32 {
            let mut sum = 0f32;
            for p in a {
                let mut best = f32::MAX;
                for q in b {
                    let d = (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2);
                    best = best.min(d);
                }
                sum += best.sqrt();
            }
            sum / a.len() as f32
        };
        // 24 个纯旋转：列是单位轴的带符号置换
        let axes: [V3; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut results: Vec<(f32, String, M3, V3)> = Vec::new();
        for xi in 0..3 {
            for sx in [1.0f32, -1.0] {
                for yi in 0..3 {
                    if yi == xi {
                        continue;
                    }
                    for sy in [1.0f32, -1.0] {
                        // 第三轴 = 右手系补全
                        let zi = 3 - xi - yi;
                        let sz = if (xi, yi) == (0, 1) || (xi, yi) == (1, 2) || (xi, yi) == (2, 0) {
                            1.0
                        } else {
                            -1.0
                        };
                        let col = |i: usize, s: f32| -> V3 {
                            [axes[i][0] * s, axes[i][1] * s, axes[i][2] * s]
                        };
                        let c1 = col(xi, sx);
                        let c2 = col(yi, sy);
                        let c3 = col(zi, sz);
                        // det 检查（右手系）
                        let det = c1[0] * (c2[1] * c3[2] - c2[2] * c3[1])
                            - c1[1] * (c2[0] * c3[2] - c2[2] * c3[0])
                            + c1[2] * (c2[0] * c3[1] - c2[1] * c3[0]);
                        if det < 0.0 {
                            continue;
                        }
                        let r = [
                            [c1[0], c2[0], c3[0]],
                            [c1[1], c2[1], c3[1]],
                            [c1[2], c2[2], c3[2]],
                        ];
                        let name = format!("meshX→[{:.0},{:.0},{:.0}] Y→[{:.0},{:.0},{:.0}] Z→[{:.0},{:.0},{:.0}]",
                            c1[0], c1[1], c1[2], c2[0], c2[1], c2[2], c3[0], c3[1], c3[2]);
                        // 平移：先质心对齐，再 ±10 步 2.5 小网格精修
                        let src_c = [
                            src.iter().map(|p| p[0]).sum::<f32>() / src.len() as f32,
                            src.iter().map(|p| p[1]).sum::<f32>() / src.len() as f32,
                            src.iter().map(|p| p[2]).sum::<f32>() / src.len() as f32,
                        ];
                        let dst_c = [
                            dst.iter().map(|p| p[0]).sum::<f32>() / dst.len() as f32,
                            dst.iter().map(|p| p[1]).sum::<f32>() / dst.len() as f32,
                            dst.iter().map(|p| p[2]).sum::<f32>() / dst.len() as f32,
                        ];
                        let rot_c = m3_apply(&r, &src_c);
                        let base_t = [
                            dst_c[0] - rot_c[0],
                            dst_c[1] - rot_c[1],
                            dst_c[2] - rot_c[2],
                        ];
                        let mut best = (f32::MAX, [0f32; 3]);
                        for tx in -4..=4i32 {
                            for ty in -4..=4i32 {
                                for tz in -4..=4i32 {
                                    let t = [
                                        base_t[0] + tx as f32 * 2.5,
                                        base_t[1] + ty as f32 * 2.5,
                                        base_t[2] + tz as f32 * 2.5,
                                    ];
                                    let moved: Vec<V3> = src
                                        .iter()
                                        .map(|p| v3_add(&m3_apply(&r, p), &t))
                                        .collect();
                                    let c = chamfer(&moved, &dst);
                                    if c < best.0 {
                                        best = (c, t);
                                    }
                                }
                            }
                        }
                        results.push((best.0, name, r, best.1));
                    }
                }
            }
        }
        results.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        out += "\n== 朝向搜索（帽模→基础发 chamfer，top8；t 为数据空间平移）==\n";
        for (c, name, _, t) in results.iter().take(8) {
            out += &format!("  chamfer={c:6.2}  {name}  t=[{:.1},{:.1},{:.1}]\n", t[0], t[1], t[2]);
        }
        // 当前实现（Option A: Rz(-90°), lift=9）的 chamfer 对照
        let cur: Vec<V3> = hat_pts
            .iter()
            .map(|v| {
                let b = rz(v);
                let b = [b[0] + 9.0, b[1], b[2]];
                v3_add(&m3_apply(&hw.r, &b), &hw.t)
            })
            .collect();
        let cur_sub = step(&cur, 300);
        out += &format!(
            "  当前实现 chamfer={:.2}\n",
            chamfer(&cur_sub, &dst)
        );
        // Option B: Rz(-90°)·Rx(π)（mesh Z→骨−Z）
        let rb = |v: &V3| -> V3 { [v[1], v[0], -v[2]] }; // Rx(π)后Rz(-90°): (x,y,z)->(y,x,-z)
        for lift in [7.0f32, 9.0] {
            let sim: Vec<V3> = hat_pts
                .iter()
                .map(|v| {
                    let b = rb(v);
                    let b = [b[0] + lift, b[1], b[2]];
                    v3_add(&m3_apply(&hw.r, &b), &hw.t)
                })
                .collect();
            let sub = step(&sim, 300);
            out += &format!(
                "  OptionB(meshZ→data+Z) lift={lift} chamfer={:.2}\n",
                chamfer(&sub, &dst)
            );
            bbox_of(&mut out, &format!("OptionB lift={lift} bbox"), &sim);
        }
    }
    Ok(out)
}
