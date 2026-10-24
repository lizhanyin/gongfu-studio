//! 套装38 头发链探针：cargo run --example sz38_probe -- <system.cpk> <girl.FSKEL>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: sz38_probe <system.cpk> <girl.FSKEL>");
        std::process::exit(2);
    }
    match gongfu_studio_lib::sz38_probe_for_test(&args[1], &args[2]) {
        Ok(out) => println!("{out}"),
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }
}
