//! 功夫世界 CPK 资源包命令行工具（打包/解包/校验）。
//!
//! 格式与用法见工程 README.md；库实现见 cpk crate。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use cpk::{AddOpts, Cpk, Entry};
use glob::Pattern;

#[derive(Parser)]
#[command(
    name = "cpktool",
    about = "功夫世界 CPK 资源包命令行工具（打包/解包/校验）",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 显示包头部与统计信息
    Info {
        /// CPK 文件路径
        cpk: PathBuf,
    },
    /// 列出包内容（默认树形，--flat 平铺，支持通配符）
    List {
        cpk: PathBuf,
        /// 通配符模式（如 *.tga、ui\\*），省略则全部
        patterns: Vec<String>,
        /// 平铺输出完整路径（默认树形）
        #[arg(short, long)]
        flat: bool,
        /// 长格式：含大小/压缩率/过滤器
        #[arg(short, long)]
        long: bool,
    },
    /// 解包 CPK 到目录（可按通配符筛选）
    Unpack {
        cpk: PathBuf,
        /// 输出目录，默认为当前目录
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 通配符模式，省略则全部解包
        patterns: Vec<String>,
    },
    /// 打包：CPK 不存在则新建，存在则追加/覆盖同名条目
    Pack {
        cpk: PathBuf,
        /// 要加入的文件或目录（目录递归）
        paths: Vec<PathBuf>,
        /// 强制不压缩
        #[arg(long)]
        no_compress: bool,
    },
    /// 向已有 CPK 添加文件或目录（同 pack，但要求包已存在）
    Add {
        cpk: PathBuf,
        paths: Vec<PathBuf>,
        #[arg(long)]
        no_compress: bool,
    },
    /// 从 CPK 中删除匹配条目（目录递归删除）
    Remove {
        cpk: PathBuf,
        patterns: Vec<String>,
    },
    /// 校验 CPK 内容完整性（解压比对长度/MD5，全名 CRC 自检）
    Verify {
        cpk: PathBuf,
        /// 只输出统计，不列明细
        #[arg(short, long)]
        quiet: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("错误: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: Command) -> Result<(), cpk::Error> {
    match cmd {
        Command::Info { cpk } => cmd_info(&cpk),
        Command::List {
            cpk,
            patterns,
            flat,
            long,
        } => cmd_list(&cpk, &patterns, flat, long),
        Command::Unpack {
            cpk,
            output,
            patterns,
        } => cmd_unpack(&cpk, output.as_deref(), &patterns),
        Command::Pack {
            cpk,
            paths,
            no_compress,
        } => cmd_pack(&cpk, &paths, no_compress),
        Command::Add {
            cpk,
            paths,
            no_compress,
        } => {
            if !cpk.exists() {
                return Err(cpk::Error::NotFound(cpk.display().to_string()));
            }
            cmd_pack(&cpk, &paths, no_compress)
        }
        Command::Remove { cpk, patterns } => cmd_remove(&cpk, &patterns),
        Command::Verify { cpk, quiet } => cmd_verify(&cpk, quiet),
    }
}

// ---------------------------------------------------------------------
// 子命令实现
// ---------------------------------------------------------------------

fn cmd_info(path: &Path) -> Result<(), cpk::Error> {
    let cpk = Cpk::open(path)?;
    let i = cpk.info();
    let h = &i.header;
    println!("文件        {}", i.path.display());
    println!("磁盘大小    {} 字节 ({:.2} MB)", i.file_len, i.file_len as f64 / 1048576.0);
    println!("包内统计    文件 {} 个，目录 {} 个", i.file_count, i.dir_count);
    println!("头部        版本 {}，头 {} 字节，表区 @ {}（{} 项 × 28B）",
        h.version, h.size_of_header, h.table_start, h.max_table_num);
    println!("表项        有效 {} / 容量 {}，碎片 {} 个（可复用 {} 字节）",
        h.valid_table_num, h.max_table_num, i.fragment_count, i.fragment_space);
    println!("数据        压缩后 {} 字节，原始 {} 字节（压缩率 {:.1}%）",
        i.packed_bytes,
        i.origin_bytes,
        if i.origin_bytes > 0 {
            100.0 * i.packed_bytes as f64 / i.origin_bytes as f64
        } else {
            100.0
        },
    );
    println!("package_size 字段 {}，data_start {}", h.package_size, h.data_start);
    Ok(())
}

fn cmd_list(path: &Path, patterns: &[String], flat: bool, long: bool) -> Result<(), cpk::Error> {
    let cpk = Cpk::open(path)?;
    let entries = cpk.entries();
    let matcher = Matcher::new(patterns);
    let selected: Vec<&Entry> = entries.iter().filter(|e| matcher.matches(e)).collect();

    if flat || long {
        for e in &selected {
            if long {
                let ratio = if e.origin_size > 0 {
                    100.0 * e.packed_size as f64 / e.origin_size as f64
                } else {
                    100.0
                };
                println!(
                    "{:<10} {:>10} {:>10} {:>6.1}%  {}",
                    if e.is_dir { "<DIR>" } else { filter_label(e.filter) },
                    e.packed_size,
                    e.origin_size,
                    ratio,
                    e.full_path
                );
            } else {
                println!("{}", e.full_path);
            }
        }
    } else {
        // 树形：按父目录组织，目录在前、同组按名字排序
        use std::collections::HashMap;
        let mut children: HashMap<u32, Vec<&Entry>> = HashMap::new();
        for e in &selected {
            children.entry(e.father_crc).or_default().push(e);
        }
        for list in children.values_mut() {
            // 目录在前，同组按名字排序
            list.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
        }
        fn walk(
            father: u32,
            children: &HashMap<u32, Vec<&Entry>>,
            depth: usize,
        ) {
            if let Some(kids) = children.get(&father) {
                for (k, e) in kids.iter().enumerate() {
                    let last = k + 1 == kids.len();
                    let branch = if depth == 0 {
                        String::new()
                    } else if last {
                        "└─ ".to_string()
                    } else {
                        "├─ ".to_string()
                    };
                    let pad = "   ".repeat(depth.saturating_sub(1));
                    println!(
                        "{pad}{branch}{}{}",
                        e.name,
                        if e.is_dir { "\\" } else { "" }
                    );
                    if e.is_dir {
                        walk(e.crc, children, depth + 1);
                    }
                }
            }
        }
        walk(0, &children, 0);
    }
    eprintln!(
        "共 {} 个条目（{} 文件 / {} 目录）",
        selected.len(),
        selected.iter().filter(|e| !e.is_dir).count(),
        selected.iter().filter(|e| e.is_dir).count()
    );
    Ok(())
}

fn cmd_unpack(
    path: &Path,
    output: Option<&Path>,
    patterns: &[String],
) -> Result<(), cpk::Error> {
    let dest = output.unwrap_or(Path::new(".")).to_path_buf();
    let mut cpk = Cpk::open(path)?;
    let matcher = Matcher::new(patterns);
    let mut prog = make_progress("解包");
    let report = cpk.extract(&dest, &mut |e| matcher.matches(e), &mut prog)?;
    println!(
        "解包完成：{} 个文件、{} 个目录，共 {} 字节 → {}",
        report.files,
        report.dirs,
        report.bytes,
        dest.display()
    );
    Ok(())
}

fn cmd_pack(path: &Path, paths: &[PathBuf], no_compress: bool) -> Result<(), cpk::Error> {
    if paths.is_empty() {
        return Err(cpk::Error::NotFound("未指定要打包的文件或目录".into()));
    }
    let mut cpk = if path.exists() {
        Cpk::open(path)?
    } else {
        Cpk::create(path, cpk::format::DEFAULT_TABLE_NUM)?
    };
    let opts = AddOpts { no_compress };
    let mut total = 0u64;
    for p in paths {
        let mut prog = make_progress("打包");
        total += cpk.add_path(p, &opts, &mut prog)?;
    }
    println!("打包完成：共加入 {} 个条目 → {}", total, path.display());
    Ok(())
}

fn cmd_remove(path: &Path, patterns: &[String]) -> Result<(), cpk::Error> {
    if patterns.is_empty() {
        return Err(cpk::Error::NotFound("未指定删除模式".into()));
    }
    let mut cpk = Cpk::open(path)?;
    let matcher = Matcher::new(patterns);
    let mut prog = make_progress("删除");
    let n = cpk.remove(&mut |e| matcher.matches(e), &mut prog)?;
    println!("已删除 {} 个条目（数据保留为碎片，可被后续打包复用）", n);
    Ok(())
}

fn cmd_verify(path: &Path, quiet: bool) -> Result<(), cpk::Error> {
    let mut cpk = Cpk::open(path)?;
    let mut prog = make_progress("校验");
    let r = cpk.verify(&mut prog)?;
    println!(
        "校验完成：文件 {} 个（MD5 一致 {}，跳过 {}），路径 CRC 一致 {}/{}",
        r.checked, r.md5_ok, r.md5_skipped, r.crc_checked, r.crc_checked + r.crc_bad.len() as u64
    );
    let ok = r.md5_bad.is_empty() && r.read_bad.is_empty() && r.crc_bad.is_empty();
    if !quiet {
        for p in r.read_bad.iter().chain(r.md5_bad.iter()).take(50) {
            println!("  ! {p}");
        }
        for p in r.crc_bad.iter().take(50) {
            println!("  ! CRC 不一致: {p}");
        }
    }
    if !ok {
        eprintln!("存在校验失败项");
        std::process::exit(2);
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------

fn filter_label(filter: u32) -> &'static str {
    match filter {
        1 => "STORE",
        2 => "LZO",
        4 => "LZSS",
        _ => "?",
    }
}

/// 通配符匹配器：大小写不敏感；模式不含 `\\` 时也匹配简单名。
struct Matcher {
    pats: Vec<(Pattern, bool)>, // (pattern, 是否含路径分隔符)
    opts: glob::MatchOptions,
    all: bool,
}

impl Matcher {
    fn new(patterns: &[String]) -> Matcher {
        let pats = patterns
            .iter()
            .map(|p| {
                let pat = p.replace('/', "\\");
                let full_only = pat.contains('\\');
                let pat = Pattern::new(&pat).unwrap_or_else(|_| Pattern::new("*").unwrap());
                (pat, full_only)
            })
            .collect();
        Matcher {
            pats,
            opts: glob::MatchOptions {
                case_sensitive: false,
                require_literal_separator: false,
                require_literal_leading_dot: false,
            },
            all: patterns.is_empty(),
        }
    }

    fn matches(&self, e: &Entry) -> bool {
        if self.all {
            return true;
        }
        self.pats.iter().any(|(pat, full_only)| {
            pat.matches_with(&e.full_path, self.opts)
                || (!*full_only && pat.matches_with(&e.name, self.opts))
        })
    }
}

/// 进度闭包：每 500 项向 stderr 打一行，避免淹没输出。
fn make_progress(label: &str) -> impl FnMut(&str, u64, u64) {
    let label = label.to_string();
    let mut last_tick = std::time::Instant::now();
    move |name: &str, done: u64, total: u64| {
        if total > 0 && (done % 500 == 0 || done == total) {
            let now = std::time::Instant::now();
            if now.duration_since(last_tick).as_millis() > 100 || done == total {
                eprintln!("  [{label}] {done}/{total}  {name}");
                last_tick = now;
            }
        }
    }
}
