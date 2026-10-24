//! FSM bbox 探针：cargo run --example fsm_bbox -- <fsm文件...>
fn main() {
    for p in std::env::args().skip(1) {
        match gongfu_studio_lib::fsm_probe_for_test(&p) {
            Ok(out) => println!("{out}"),
            Err(e) => eprintln!("{p}: {e}"),
        }
    }
}
