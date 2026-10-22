//! 模型导出：OBJ 合并导出（多个 fskin）与右键单文件导出（模型 + 关联纹理）

use crate::{fak, fsm, fskin, vfs};
use std::fs;
use std::path::Path;

/// 虚拟路径的父目录（去最后一段）
fn parent_of(path: &str) -> Option<&str> {
    path.rsplit_once(['\\', '/']).map(|(d, _)| d)
}

/// 文件名去扩展名
fn stem_of(name: &str) -> &str {
    name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name)
}

/// 导出为一组 obj + mtl，纹理复制到输出目录
pub fn export_obj(paths: &[String], out_dir: &str, base_name: &str) -> Result<String, String> {
    let dir = Path::new(out_dir);
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let obj_path = dir.join(format!("{base_name}.obj"));
    let mtl_path = dir.join(format!("{base_name}.mtl"));

    let mut obj = String::new();
    obj.push_str(&format!("# 由 Gongfu Studio 导出, {} 个部件\n", paths.len()));
    obj.push_str(&format!("mtllib {base_name}.mtl\n"));

    let mut mtl = String::new();
    let mut v_off = 1usize;
    let mut vt_off = 1usize;
    let mut vn_off = 1usize;
    let mut tex_seq = 0usize;

    for p in paths {
        let bytes = vfs::read_file(p).map_err(|e| format!("读取 {p}: {e}"))?;
        let m = fskin::parse_fskin(p, &bytes)?;
        let triangles = m.triangles_merged();
        let name = Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("part");
        let mat_name = format!("mat_{name}");
        obj.push_str(&format!("o {name}\ng {name}\n"));

        // 材质（多子网格取首槽；无材质特效网格用中性色）
        let (diffuse, ambient, specular, spec_power, texture_path) = match m.materials.first() {
            Some(a) => (
                a.diffuse,
                a.ambient,
                a.specular,
                a.spec_power,
                Some(&a.texture_path),
            ),
            None => (
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
                [0.0, 0.0, 0.0, 1.0],
                10.0,
                None,
            ),
        };
        mtl.push_str(&format!(
            "newmtl {mat_name}\nKd {:.4} {:.4} {:.4}\nKa {:.4} {:.4} {:.4}\nKs {:.4} {:.4} {:.4}\nNs {:.1}\n",
            diffuse[0], diffuse[1], diffuse[2],
            ambient[0], ambient[1], ambient[2],
            specular[0], specular[1], specular[2],
            spec_power,
        ));
        // 纹理: 复制到输出目录
        if let Some(src) = texture_path
            .and_then(|t| t.rsplit(['\\', '/']).next().map(|b| b.to_string()))
        {
            let stem = src.rsplit_once('.').map(|(s, _)| s).unwrap_or(&src);
            // 在部件同目录树（模型根目录）中查找同名 dds/tga
            let model_root = parent_of(p).and_then(parent_of);
            if let Some(found) = find_texture(model_root, stem) {
                tex_seq += 1;
                let ext = found
                    .rsplit('.')
                    .next()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "dds".into());
                let tex_name = format!("tex_{tex_seq:02}_{stem}.{ext}");
                let dst = dir.join(&tex_name);
                if let Ok(data) = vfs::read_file(&found) {
                    if fs::write(&dst, &data).is_ok() {
                        mtl.push_str(&format!("map_Kd {tex_name}\n"));
                    }
                }
            }
        }
        mtl.push('\n');
        obj.push_str(&format!("usemtl {mat_name}\n"));

        for v in &m.vertices {
            obj.push_str(&format!(
                "v {:.5} {:.5} {:.5}\n",
                v.pos[0], v.pos[1], v.pos[2]
            ));
        }
        for v in &m.vertices {
            obj.push_str(&format!(
                "vt {:.5} {:.5}\n",
                v.uv[0],
                1.0 - v.uv[1]
            ));
        }
        // 跨缝面：手臂等条带 UV 环绕 u=0/1 接缝（引擎用重复寻址），
        // 为这些面的角点追加 ±1 调整后的 vt，否则插值横穿整张纹理
        let mut extra_vt: Vec<(f32, f32)> = Vec::new();
        let mut face_vt: Vec<Option<[usize; 3]>> = vec![None; triangles.len() / 3];
        for (fi, tri3) in triangles.chunks_exact(3).enumerate() {
            let corners = [tri3[0] as usize, tri3[1] as usize, tri3[2] as usize];
            let us = corners.map(|i| m.vertices[i].uv[0]);
            let vs = corners.map(|i| m.vertices[i].uv[1]);
            let du = us.iter().cloned().fold(f32::MIN, f32::max)
                - us.iter().cloned().fold(f32::MAX, f32::min);
            let dv = vs.iter().cloned().fold(f32::MIN, f32::max)
                - vs.iter().cloned().fold(f32::MAX, f32::min);
            if du <= 0.5 && dv <= 0.5 {
                continue;
            }
            let mut idxs = [0usize; 3];
            for (slot, &vi) in corners.iter().enumerate() {
                let (mut u, mut v) = (m.vertices[vi].uv[0], m.vertices[vi].uv[1]);
                if du > 0.5 {
                    while u - us[0] > 0.5 {
                        u -= 1.0;
                    }
                    while u - us[0] < -0.5 {
                        u += 1.0;
                    }
                }
                if dv > 0.5 {
                    while v - vs[0] > 0.5 {
                        v -= 1.0;
                    }
                    while v - vs[0] < -0.5 {
                        v += 1.0;
                    }
                }
                extra_vt.push((u, 1.0 - v));
                idxs[slot] = m.vertices.len() + extra_vt.len() - 1;
            }
            face_vt[fi] = Some(idxs);
        }
        for (u, v) in &extra_vt {
            obj.push_str(&format!("vt {u:.5} {v:.5}\n"));
        }
        for v in &m.vertices {
            obj.push_str(&format!(
                "vn {:.5} {:.5} {:.5}\n",
                v.normal[0], v.normal[1], v.normal[2]
            ));
        }
        let t = &triangles;
        for (fi, tri3) in t.chunks_exact(3).enumerate() {
            let corners = [tri3[0] as usize, tri3[1] as usize, tri3[2] as usize];
            let vts = face_vt[fi].unwrap_or(corners);
            obj.push_str(&format!(
                "f {}/{}/{} {}/{}/{} {}/{}/{}\n",
                corners[0] + v_off, vts[0] + vt_off, corners[0] + vn_off,
                corners[1] + v_off, vts[1] + vt_off, corners[1] + vn_off,
                corners[2] + v_off, vts[2] + vt_off, corners[2] + vn_off,
            ));
        }
        v_off += m.vertices.len();
        vt_off += m.vertices.len() + extra_vt.len();
        vn_off += m.vertices.len();
    }

    fs::write(&obj_path, obj).map_err(|e| e.to_string())?;
    fs::write(&mtl_path, mtl).map_err(|e| e.to_string())?;
    Ok(obj_path.to_string_lossy().into_owned())
}

/// 从模型根目录（AvatarGirl 等，含 `cpk:` 寻址）向下查找与纹理 stem 同名的图片文件
fn find_texture(root: Option<&str>, stem: &str) -> Option<String> {
    let root = root?;
    vfs::walk_files(root, &["dds", "tga", "png", "jpg", "bmp"])
        .into_iter()
        .find(|e| stem_of(&e.name).eq_ignore_ascii_case(stem))
        .map(|e| e.path)
}

/// 右键导出模型文件（fskin / fsm / fak）：原样复制模型字节，并把材质引用的
/// 纹理（按 stem 在模型根目录树中定位，与 OBJ 导出同口径）复制到同一目录，
/// 纹理保留原始文件名。未找到的纹理在返回文案中注明，不作为失败。
pub fn export_model(src: &str, dest: &str) -> Result<String, String> {
    let bytes = vfs::read_file(src).map_err(|e| format!("读取 {src}: {e}"))?;
    let name = src.rsplit(['\\', '/']).next().unwrap_or(src);
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let texture_paths: Vec<String> = match ext.as_str() {
        "fskin" => fskin::parse_fskin(src, &bytes)?
            .materials
            .iter()
            .map(|m| m.texture_path.clone())
            .collect(),
        "fsm" => fsm::parse_fsm(src, &bytes)?
            .materials
            .iter()
            .map(|m| m.texture_path.clone())
            .collect(),
        "fak" => fak::parse_fak(src, &bytes)?
            .materials
            .iter()
            .map(|m| m.texture_path.clone())
            .collect(),
        _ => Vec::new(),
    };

    // 材质引用按 stem 去重（不同材质常共享同一张贴图），保持首次出现顺序
    let mut stems: Vec<String> = Vec::new();
    for t in &texture_paths {
        if t.is_empty() {
            continue;
        }
        if let Some(base) = t.rsplit(['\\', '/']).next() {
            let stem = stem_of(base).to_string();
            if !stems.contains(&stem) {
                stems.push(stem);
            }
        }
    }

    let dest_dir = Path::new(dest)
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| "导出路径无效".to_string())?
        .to_path_buf();
    fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    fs::write(dest, &bytes).map_err(|e| format!("写入失败: {e}"))?;

    let model_root = parent_of(src).and_then(parent_of);
    let mut copied = 0usize;
    let mut missing = 0usize;
    for stem in &stems {
        let found = find_texture(model_root, stem).and_then(|p| {
            let data = vfs::read_file(&p).ok()?;
            let base = p.rsplit(['\\', '/']).next()?.to_string();
            fs::write(dest_dir.join(&base), &data).is_ok().then_some(base)
        });
        match found {
            Some(_) => copied += 1,
            None => missing += 1,
        }
    }

    Ok(match (stems.is_empty(), missing) {
        (true, _) => format!("已导出: {name}"),
        (false, 0) => format!("已导出: {name}（含 {copied} 张纹理）"),
        (false, _) => format!("已导出: {name}（纹理 {copied} 张，{missing} 张未找到）"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res");

    /// fskin：字节级原样复制 + 引用纹理（nvzhujue_boot）一并落盘
    #[test]
    fn export_fskin_with_textures() {
        let out = std::env::temp_dir().join("gongfu_export_model/fskin");
        let _ = fs::remove_dir_all(&out);
        let src = format!("{RES}/system/AvatarGirl/girl_Boot.fskin");
        let dest = out.join("girl_Boot.fskin");
        let summary = export_model(&src, dest.to_str().unwrap()).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), fs::read(&src).unwrap(), "模型应原样复制");
        assert_eq!(summary, "已导出: girl_Boot.fskin（含 1 张纹理）");
        assert!(out.join("nvzhujue_boot.dds").is_file(), "关联纹理应导出到同目录");
        let _ = fs::remove_dir_all(&out);
    }

    /// fsm / fak：同样携带材质纹理；fsm 多材质时按 stem 去重后逐张导出
    #[test]
    fn export_fsm_fak_with_textures() {
        let out = std::env::temp_dir().join("gongfu_export_model/mixed");
        let _ = fs::remove_dir_all(&out);

        let src = format!("{RES}/system/Weapon/Jian/J01.FSM");
        let summary = export_model(&src, out.join("J01.FSM").to_str().unwrap()).unwrap();
        assert!(summary.contains("含 1 张纹理"), "{summary}");
        assert!(out.join("j01-s01.dds").is_file());

        let src = format!("{RES}/system/Monster/G080/G080.FAK");
        let summary = export_model(&src, out.join("G080.FAK").to_str().unwrap()).unwrap();
        assert!(summary.contains("含 2 张纹理"), "{summary}");
        assert!(out.join("g080.dds").is_file() && out.join("g080+.dds").is_file());
        let _ = fs::remove_dir_all(&out);
    }
}
