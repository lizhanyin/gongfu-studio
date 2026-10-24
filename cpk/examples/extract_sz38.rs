//! 提取指定 CPK 的 SZ_38.fskin 并打印 sha256 前 16 hex
use cpk::package::Cpk;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut cpk = Cpk::open(&args[1]).expect("open");
    let e = cpk.entries().into_iter()
        .find(|e| e.full_path.to_lowercase().ends_with(r"\shizhuang\sz_38.fskin"))
        .expect("not found");
    let bytes = cpk.read_file(&e).expect("read");
    let n = bytes.len();
    std::fs::write(&args[2], &bytes).expect("write");
    println!("{} -> {} ({}B)", e.full_path, args[2], n);
}
