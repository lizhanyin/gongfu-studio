//! 装扮目录扫描探针：cargo run --example dress_scan -- <角色资源根>
//! 列各槽位物品与贴图解析结果（验证路径优先解析）。
fn main() {
    let root = std::env::args().nth(1).expect("root");
    let cat = gongfu_studio_lib::dress_scan_for_test(&root).expect("scan");
    for s in &cat.slots {
        println!("slot {} kind={} items={}", s.key, s.kind, s.items.len());
        for it in s.items.iter().take(6) {
            println!("  {} v{} t{} tex={:?}", it.name, it.vertex_count, it.triangle_count, it.texture_files);
        }
    }
}
