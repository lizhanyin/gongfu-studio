// 裙摆防穿模探针：cargo run --example skirt_probe -- <fskel> <fskin...>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: skirt_probe <fskel> <fskin...>");
        std::process::exit(2);
    }
    match gongfu_studio_lib::skirt_probe_for_test(&args[1], &args[2..]) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }
}
