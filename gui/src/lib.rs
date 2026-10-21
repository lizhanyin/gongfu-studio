mod dds;
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
            load_fskin,
            save_fskin,
            load_texture,
            export_obj,
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
            list_res_dir
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 失败");
}

/// 供 ddsdump 示例与测试使用
pub fn dds_decode_for_test(bytes: &[u8]) -> Result<Vec<u8>, String> {
    dds::decode_dds(bytes).map(|t| t.rgba)
}
