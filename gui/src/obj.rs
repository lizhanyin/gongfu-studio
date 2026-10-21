//! OBJ 导出：多个 fskin 合并为一个完整模型

use crate::fskin::parse_fskin;
use crate::vfs;
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
        let m = parse_fskin(p, &bytes)?;
        let name = Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("part");
        let mat_name = format!("mat_{name}");
        obj.push_str(&format!("o {name}\ng {name}\n"));

        // 材质
        let (r, g, b) = (
            m.material.diffuse[0],
            m.material.diffuse[1],
            m.material.diffuse[2],
        );
        mtl.push_str(&format!(
            "newmtl {mat_name}\nKd {r:.4} {g:.4} {b:.4}\nKa {:.4} {:.4} {:.4}\nKs {:.4} {:.4} {:.4}\nNs {:.1}\n",
            m.material.ambient[0], m.material.ambient[1], m.material.ambient[2],
            m.material.specular[0], m.material.specular[1], m.material.specular[2],
            m.material.spec_power,
        ));
        // 纹理: 复制到输出目录
        if let Some(src) = m
            .material
            .texture_path
            .rsplit(['\\', '/'])
            .next()
            .map(|b| b.to_string())
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
        let mut face_vt: Vec<Option<[usize; 3]>> = vec![None; m.triangles.len() / 3];
        for (fi, tri3) in m.triangles.chunks_exact(3).enumerate() {
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
        let t = &m.triangles;
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
