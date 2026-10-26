//! SFX 扫描探针：cargo run --example sfx_scan -- <目录或文件...>
//! 递归解析目录下全部 .sfx 并打印概览；末行汇总成功/失败计数。
//! 逆向时对 system.cpk 全量 782 个样本扫描，0 失败、往返全一致。
fn main() {
    let mut ok = 0usize;
    let mut fail = 0usize;
    for root in std::env::args().skip(1) {
        let p = std::path::Path::new(&root);
        let mut files = Vec::new();
        if p.is_dir() {
            collect(p, &mut files);
        } else {
            files.push(p.to_path_buf());
        }
        files.sort();
        for f in files {
            match gongfu_studio_lib::sfx_probe_for_test(&f.to_string_lossy()) {
                Ok(out) => {
                    ok += 1;
                    println!("{out}");
                }
                Err(e) => {
                    fail += 1;
                    println!("FAIL {}: {e}", f.display());
                }
            }
        }
    }
    println!("-- ok {ok}, fail {fail} --");
    if fail > 0 {
        std::process::exit(1);
    }
}

fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(es) = std::fs::read_dir(dir) else { return };
    for e in es.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("sfx"))
        {
            out.push(p);
        }
    }
}
