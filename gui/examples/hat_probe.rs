// 帽子挂接探针：cargo run --example hat_probe -- <fskel> <fsm> <fskin...>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: hat_probe <fskel> <fsm> <fskin...>");
        std::process::exit(2);
    }
    match gongfu_studio_lib::hat_probe_for_test(&args[1], &args[2], &args[3..]) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }
}
