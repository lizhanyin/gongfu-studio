//! 列 MHair 与 Hat 目录
use cpk::package::Cpk;
fn main() {
    let path = std::env::args().nth(1).expect("cpk");
    let cpk = Cpk::open(&path).expect("open");
    let mut es: Vec<_> = cpk
        .entries()
        .into_iter()
        .filter(|e| {
            !e.is_dir && {
                let p = e.full_path.to_lowercase();
                p.contains(r"\mhair\") || p.contains(r"\hat\")
            }
        })
        .collect();
    es.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    for e in &es {
        println!("{}  {}B", e.full_path, e.origin_size);
    }
    println!("共 {}", es.len());
}
