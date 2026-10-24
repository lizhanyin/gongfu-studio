//! 提取指定 CPK 的 girl.FSKEL 到临时文件
use cpk::package::Cpk;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut cpk = Cpk::open(&args[1]).expect("open");
    let e = cpk.entries().into_iter()
        .find(|e| e.full_path.to_lowercase().ends_with(r"\avatargirl\act\girl.fskel"))
        .expect("not found");
    let bytes = cpk.read_file(&e).expect("read");
    std::fs::write(&args[2], bytes).expect("write");
    println!("{} -> {} ({}B)", e.full_path, args[2], e.origin_size);
}
