//! 临时探针：在 system.cpk 里找 AvatarBoy/AvatarGirl 骨架 FSKEL，
//! 检查骨骼名表里是否有 tag_Hat 等挂点骨骼。

use cpk::package::Cpk;

fn main() {
    let path = std::env::args().nth(1).expect("用法: tag_probe <system.cpk>");
    let mut cpk = Cpk::open(&path).expect("打开 cpk");
    let mut fs = cpk
        .entries()
        .into_iter()
        .filter(|e| !e.is_dir && e.full_path.to_lowercase().ends_with(".fskel"))
        .collect::<Vec<_>>();
    fs.retain(|e| {
        let p = e.full_path.to_lowercase();
        p.contains("avatar") || p.contains("boy") || p.contains("girl")
    });
    fs.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    for e in &fs {
        // 只看每个目录的默认骨架（girl/boy 无数字）
        let stem = e.name.trim_end_matches(".FSKEL").to_lowercase();
        if stem.chars().any(|c| c.is_ascii_digit()) && !stem.starts_with("girl")
            && !stem.starts_with("boy")
        {
            continue;
        }
        let bytes = cpk.read_file(e).expect("读 fskel");
        // 骨骼名表在 0x118 起：[u32 长度][名]×N；扫 ASCII 串找 tag_ 前缀
        let mut tags = vec![];
        let mut i = 0x118;
        while i + 4 <= bytes.len() {
            let len = u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
                as usize;
            if len == 0 || len > 64 || i + 4 + len > bytes.len() {
                break;
            }
            let s = String::from_utf8_lossy(&bytes[i + 4..i + 4 + len - 1]).into_owned();
            if s.starts_with("tag_") || s.to_lowercase().contains("hat") {
                tags.push(s);
            }
            i += 4 + len;
        }
        println!("{}  => {:?}", e.full_path, tags);
    }
}
