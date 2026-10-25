//! NPC FAK 材质贴图探针：列出 CPK 内 NPC 模型的材质贴图引用，
//! 并按「FAK 同目录 stem 匹配」（前端 useFakMode 同口径）报告命中/未命中。
//! cargo run --example npc_tex_probe -- <cpk路径>
fn main() {
    let cpk = std::env::args().nth(1).expect("cpk path");
    let base = format!("cpk:{cpk}\\system\\NPC");

    // 完全模拟前端 useFakMode 口径：每个 FAK 仅列举**自身所在目录**（非递归）
    // 的贴图做 stem 匹配；另建全子树索引统计「放宽到子树能救回多少」
    let mut dir_tex: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut sub_dir: std::collections::BTreeMap<String, String> = Default::default();
    for (tpath, tname) in gongfu_studio_lib::vfs_walk_for_test(&base, &["dds", "tga", "bmp"]) {
        let stem = tname
            .rsplit_once('.')
            .map(|(s, _)| s.to_ascii_lowercase())
            .unwrap_or_default();
        let parent = tpath
            .rsplit_once(['\\', '/'])
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        dir_tex.entry(parent).or_default().push(stem.clone());
        sub_dir.entry(stem).or_insert(tpath);
    }
    println!("贴图索引建立完成：{} 个目录 / 全子树 {} stem", dir_tex.len(), sub_dir.len());

    let models = gongfu_studio_lib::npc_fak_materials_for_test(&base).expect("scan");
    let mut miss_same = 0usize;
    let mut miss_all = 0usize;
    let mut tga_ref = 0usize;
    println!("FAK 模型 {} 个（只展开有问题的）", models.len());
    for (path, mats) in &models {
        let own_dir = path
            .rsplit_once(['\\', '/'])
            .map(|(d, _)| d.to_string())
            .unwrap_or_default();
        let own = dir_tex.get(&own_dir).cloned().unwrap_or_default();
        for t in mats {
            if t.is_empty() {
                continue;
            }
            let stem = t
                .rsplit(['\\', '/'])
                .next()
                .and_then(|b| b.rsplit_once('.').map(|(s, _)| s))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let ext = t.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
            if ext == "tga" {
                tga_ref += 1;
            }
            let hit_same = own.iter().any(|s| *s == stem);
            let hit_all = sub_dir.contains_key(&stem);
            if !hit_same {
                miss_same += 1;
                println!(
                    "  [本目录MISS] {} 引用 {:?} (stem {stem}, .{ext}) 子树命中={}",
                    path, t, hit_all
                );
            }
            if !hit_all {
                miss_all += 1;
            }
        }
    }
    println!(
        "汇总: 材质引用含 .tga 共 {tga_ref} 条；本目录未命中 {miss_same} 条；全子树未命中 {miss_all} 条"
    );

    // 前端「祖先拼接」口径验证：小写 npc 引用 + 子目录模型能否命中包内贴图
    let probe = format!("cpk:{cpk}\\system\\npc\\N302_3.tga");
    match gongfu_studio_lib::vfs_read_for_test(&probe) {
        Ok(b) => println!("路径优先验证: 小写 npc 路径读取成功 ({}B)", b.len()),
        Err(e) => println!("路径优先验证: 读取失败 {e}"),
    }
}
