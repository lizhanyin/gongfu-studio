//! 临时探针：读 system.cpk 全部 Hat/*.FSM，dump 挂点数、头部 bbox、
//! 顶点区实测 min/max，判断帽子网格原点是否统一（挂接锚一致性）。

use cpk::package::Cpk;

fn f32at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn main() {
    let path = std::env::args().nth(1).expect("用法: hat_probe <system.cpk>");
    let mut cpk = Cpk::open(&path).expect("打开 cpk");
    let mut hats: Vec<_> = cpk
        .entries()
        .into_iter()
        .filter(|e| !e.is_dir && e.full_path.to_lowercase().ends_with(".fsm"))
        .collect();
    // 只要 Hat 目录（武器留作对照组，可放开）
    hats.retain(|e| e.full_path.to_lowercase().contains("\\hat\\"));
    hats.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    println!("共 {} 个帽子 FSM", hats.len());
    println!(
        "{:<14} {:>5} {:>5} | 头部bbox min..max (Y)      | 顶点实测 min..max (Y)      | dummies",
        "file", "verts", "subs"
    );
    for e in &hats {
        let bytes = cpk.read_file(e).expect("读取 fsm");
        let vc = u32at(&bytes, 0x0C) as usize;
        let sc = u32at(&bytes, 0x14) as usize;
        let dc = u32at(&bytes, 0x24) as usize;
        let hmin_y = f32at(&bytes, 0x2C + 4);
        let hmax_y = f32at(&bytes, 0x2C + 16);
        // 顶点区 0x144 起，每顶点 32B，[pos3][nrm3][uv2]
        let (mut vmin, mut vmax) = ([f32::MAX; 3], [f32::MIN; 3]);
        for i in 0..vc {
            for d in 0..3 {
                let v = f32at(&bytes, 0x144 + i * 32 + d * 4);
                vmin[d] = vmin[d].min(v);
                vmax[d] = vmax[d].max(v);
            }
        }
        let mut names = String::new();
        if dc > 0 {
            let doff = u32at(&bytes, 0x28) as usize;
            for i in 0..dc {
                let p = doff + 4 + i * 36;
                let end = bytes[p..p + 4].iter().position(|&b| b == 0).unwrap_or(4);
                names.push_str(&String::from_utf8_lossy(&bytes[p..p + end]));
                names.push(' ');
            }
        }
        println!(
            "{:<14} {:>5} {:>5} | {:>8.2}..{:<8.2} | {:>8.2}..{:<8.2} | {} {}",
            e.name,
            vc,
            sc,
            hmin_y,
            hmax_y,
            vmin[1],
            vmax[1],
            dc,
            names
        );
        // 顶点 X/Z 范围（前后/左右对称性也一起看）
        println!(
            "{:>16}X {:>8.2}..{:<8.2}  Z {:>8.2}..{:<8.2}",
            "", vmin[0], vmax[0], vmin[2], vmax[2]
        );
    }
}
