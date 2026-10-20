mod dds;
mod fak;
mod fskin;
mod fskel;
mod obj;
mod res;

use base64::Engine;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TexImageDto {
    width: u32,
    height: u32,
    rgba_base64: String,
}

#[tauri::command]
fn scan_model_set(dir: String) -> Result<fskin::ModelSet, String> {
    fskin::scan_model_set(&dir)
}

#[tauri::command]
fn load_fskin(path: String) -> Result<fskin::FskinModel, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    fskin::parse_fskin(&path, &bytes)
}

#[tauri::command]
fn save_fskin(path: String, model: fskin::FskinModel) -> Result<(), String> {
    let bytes = fskin::serialize_fskin(&model)?;
    std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
}

#[tauri::command]
fn load_texture(path: String) -> Result<TexImageDto, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    let t = dds::decode_dds(&bytes)?;
    Ok(TexImageDto {
        width: t.width,
        height: t.height,
        rgba_base64: base64::engine::general_purpose::STANDARD.encode(t.rgba),
    })
}

#[tauri::command]
fn export_obj(paths: Vec<String>, out_dir: String, base_name: String) -> Result<String, String> {
    obj::export_obj(&paths, &out_dir, &base_name)
}

/// 根据当前顶点重算包围盒，返回新值（前端确认后写回）
#[tauri::command]
fn compute_bbox(model: fskin::FskinModel) -> Result<([f32; 3], [f32; 3]), String> {
    Ok(fskin::compute_bbox(&model))
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

// ---------------- FSKEL 动作 ----------------

#[tauri::command]
fn scan_act_dir(dir: String) -> Result<Vec<fskel::FskelBrief>, String> {
    fskel::scan_dir(&dir)
}

#[tauri::command]
fn load_fskel_meta(path: String) -> Result<fskel::FskelMeta, String> {
    Ok(fskel::get_or_load(&path)?.meta())
}

/// [from, to) 帧的扁平姿态：帧 × 骨 × (tx,ty,tz,qx,qy,qz,qw)
#[tauri::command]
fn load_fskel_frames(path: String, from: u32, to: u32) -> Result<Vec<f32>, String> {
    Ok(fskel::get_or_load(&path)?.frames_flat(from, to))
}

#[tauri::command]
fn export_fskel_bvh(
    path: String,
    action_index: Option<usize>,
    out_path: String,
) -> Result<String, String> {
    let f = fskel::get_or_load(&path)?;
    fskel::export_bvh(&f, action_index, &out_path)
}

#[tauri::command]
fn export_fskel_glb(path: String, out_path: String) -> Result<String, String> {
    let f = fskel::get_or_load(&path)?;
    fskel::export_glb(&f, &out_path)
}

// ---------------- 资源浏览 ----------------

/// .fak 顶点动画模型（含全部帧顶点数据）
#[tauri::command]
fn load_fak(path: String) -> Result<fak::FakModel, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("读取失败: {e}"))?;
    fak::parse_fak(&path, &bytes)
}

/// 保存 .fak（未修改的模型字节级还原，见 fak roundtrip 测试）
#[tauri::command]
fn save_fak(path: String, model: fak::FakModel) -> Result<(), String> {
    let bytes = fak::serialize_fak(&model)?;
    std::fs::write(&path, bytes).map_err(|e| format!("写入失败: {e}"))
}

/// 列举资源目录直接子项（供资源树懒加载 / 资源网格）
#[tauri::command]
fn list_res_dir(dir: String) -> Result<Vec<res::ResEntry>, String> {
    res::list_dir(&dir)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
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
            list_res_dir
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 失败");
}

/// 供 ddsdump 示例与测试使用
pub fn dds_decode_for_test(bytes: &[u8]) -> Result<Vec<u8>, String> {
    dds::decode_dds(bytes).map(|t| t.rgba)
}
