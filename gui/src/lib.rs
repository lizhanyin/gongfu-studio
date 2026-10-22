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
