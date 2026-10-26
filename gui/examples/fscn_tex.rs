//! FSCN 贴图解析链路探针：cargo run --example fscn_tex -- <cpk路径> [包内模型路径]
//! 验证：包内模型可解析 → 材质引用(.tga) 按包根祖先拼接，原名未命中时
//! 换 .dds 后应命中可读（building\texture 全量为 .dds，与前端
//! resolveModelTextures 同口径）。
use gongfu_studio_lib::{fscn_materials_for_test, vfs_read_for_test};

fn main() {
    let cpk = std::env::args().nth(1).expect("cpk path");
    let inner = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "system\\Building\\g01\\G01DW01.FSCN".into());
    let vpath = format!("cpk:{cpk}\\{inner}");
    let bytes = vfs_read_for_test(&vpath).expect("读模型");
    // 解析走磁盘路径：落到临时文件
    let tmp = std::env::temp_dir().join("fscn_tex_probe.fscn");
    std::fs::write(&tmp, &bytes).expect("写临时文件");
    let tmp = tmp.to_string_lossy().into_owned();
    let mats = fscn_materials_for_test(&tmp).expect("解析");
    println!("{}: {} 条材质", inner, mats.len());
    let root = format!("cpk:{cpk}");
    // 模型所在目录（同目录 stem 回落用，前端第 2 步）
    let model_dir = match inner.rfind(['\\', '/']) {
        Some(i) => format!("cpk:{cpk}\\{}", &inner[..i]),
        None => format!("cpk:{cpk}"),
    };
    let mut hit = 0;
    for (idx, r) in &mats {
        if r.is_empty() {
            println!("  mat{idx}: <无贴图>");
            continue;
        }
        let tga = format!("{root}\\{}", r.replace('/', "\\"));
        let dds = format!("{}\\{}.dds", root, r.trim_end_matches(".tga").replace('/', "\\"));
        match vfs_read_for_test(&tga) {
            Ok(b) => {
                hit += 1;
                println!("  命中(tga) {} → {} 字节", r, b.len());
            }
            Err(_) => match vfs_read_for_test(&dds) {
                Ok(b) => {
                    hit += 1;
                    println!("  命中(dds) {} → {} 字节", dds, b.len());
                }
                Err(e) => {
                    // 第 2 步：同目录 stem 回落（WK11 引用 system\itemmdl\ 不存在，
                    // 但 wk11.dds 与模型同在 ItemSys\ItemMdl\）
                    let stem = r.rsplit(['\\', '/']).next().unwrap_or(r);
                    let stem = stem.trim_end_matches(".tga").to_lowercase();
                    let same = format!("{}\\{}.dds", model_dir, stem);
                    match vfs_read_for_test(&same) {
                        Ok(b) => {
                            hit += 1;
                            println!("  命中(同目录stem) {} → {} 字节", same, b.len());
                        }
                        Err(_) => println!("  未命中 {} ({e})", r),
                    }
                }
            },
        }
    }
    println!("-- 命中 {}/{} --", hit, mats.len());
}
