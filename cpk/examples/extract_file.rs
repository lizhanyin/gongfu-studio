//! 提取 CPK 内指定后缀路径包含关键字的文件到目标目录
use cpk::package::Cpk;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut cpk = Cpk::open(&args[1]).expect("open");
    let key = args[2].to_lowercase();
    let out_dir = &args[3];
    std::fs::create_dir_all(out_dir).unwrap();
    for e in cpk.entries().into_iter().filter(|e| !e.is_dir && e.full_path.to_lowercase().contains(&key)) {
        let name = e.full_path.rsplit(['\\', '/']).next().unwrap_or("f").to_string();
        let bytes = cpk.read_file(&e).expect("read");
        let n = bytes.len();
        let dest = std::path::Path::new(out_dir).join(&name);
        std::fs::write(dest, &bytes).unwrap();
        println!("{} {}", name, n);
    }
}
