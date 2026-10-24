//! 临时探针：列 system.cpk 时装目录中款式号 38 的文件
use cpk::package::Cpk;

fn main() {
    let path = std::env::args().nth(1).expect("用法: ls_sz38 <cpk>");
    let cpk = Cpk::open(&path).expect("打开");
    let mut es: Vec<_> = cpk
        .entries()
        .into_iter()
        .filter(|e| {
            !e.is_dir
                && {
                    let p = e.full_path.to_lowercase();
                    (p.contains(r"\shizhuang\") || p.contains(r"\transformfashion\"))
                        && p.contains("38")
                }
        })
        .collect();
    es.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    for e in &es {
        println!("{}  {}B", e.full_path, e.origin_size);
    }
    println!("共 {} 个文件", es.len());
}
