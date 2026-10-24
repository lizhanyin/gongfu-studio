//! FSM 材质贴图路径探针：cargo run --example fsm_mat -- <fsm文件...>
fn main() {
    for p in std::env::args().skip(1) {
        match gongfu_studio_lib::fsm_materials_for_test(&p) {
            Ok(mats) => {
                println!("== {}", p.rsplit(['\\', '/']).next().unwrap_or(&p));
                if mats.is_empty() {
                    println!("  (无材质)");
                }
                for (i, (idx, path)) in mats.iter().enumerate() {
                    println!("  mat[{i}] index={idx} path={path:?}");
                }
            }
            Err(e) => eprintln!("{p}: {e}"),
        }
    }
}
