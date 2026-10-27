//! SFX 参数区结构扫描探针：cargo run --example sfx_flags -- <目录或 cpk: 根...>
//! 在未逆向的节点参数区（节点号 u32 + 名字 32B 之后的变长区）里统计：
//!   1. 头部 u32 字段取值直方图（找恒 0/1 的布尔候选 / 0xCCCCCCCC 未初始化）；
//!   2. 值 == 贴图引用数的「计数字段」候选；
//!   3. 相邻贴图引用偏移差（验证 72B 槽步长类定长数组）；
//!   4. 布尔候选字段取值与「该节点是否有贴图」的交叉分布（找「不渲染」
//!      类标志的线索——离线无法定语义，只报相关性）。

use std::collections::BTreeMap;

use gongfu_studio_lib::{sfx_parse_for_test, vfs_walk_for_test};

/// 参数区里 NUL 结尾贴图 token 的偏移（与 sfx.rs extract_refs 同口径）
fn tex_offsets(params: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < params.len() {
        let start = i;
        while i < params.len() && (0x20..0x7f).contains(&params[i]) {
            i += 1;
        }
        let tok = &params[start..i];
        let lower = |p: &[u8]| tok.len() >= p.len() && tok[..p.len()].eq_ignore_ascii_case(p);
        if !tok.is_empty()
            && i < params.len()
            && params[i] == 0
            && tok.len() >= 8
            && tok.iter().any(|&c| c == b'\\')
            && lower(b"system\\")
            && !tok.ends_with(b".fak")
            && !tok.ends_with(b".FAK")
        {
            out.push(start);
        }
        i += 1;
    }
    out
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn main() {
    let mut files = Vec::new();
    for root in std::env::args().skip(1) {
        files.extend(vfs_walk_for_test(&root, &["sfx"]));
    }
    if files.is_empty() {
        println!("未扫描到 .sfx");
        return;
    }
    files.sort();

    // kind → u32 偏移 → 值 → 节点数
    let mut hist: BTreeMap<String, BTreeMap<usize, BTreeMap<u32, usize>>> = BTreeMap::new();
    // kind → u32 偏移 → 命中「值==贴图数」的节点数
    let mut count_hits: BTreeMap<String, BTreeMap<usize, usize>> = BTreeMap::new();
    // kind → 相邻贴图偏移差 → 出现次数
    let mut strides: BTreeMap<String, BTreeMap<usize, usize>> = BTreeMap::new();
    let mut totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut fails = 0usize;
    let mut tex_ref_total = 0usize;
    let mut tex_node_total = 0usize;

    for (path, _name) in &files {
        let m = match sfx_parse_for_test(path) {
            Ok(m) => m,
            Err(_) => {
                fails += 1;
                continue;
            }
        };
        for n in &m.nodes {
            *totals.entry(n.kind.clone()).or_default() += 1;
            let params = &n.raw[36.min(n.raw.len())..];
            let texs = tex_offsets(params);
            tex_ref_total += n.textures.len();
            if !n.textures.is_empty() {
                tex_node_total += 1;
            }
            for w in 1..texs.len() {
                *strides
                    .entry(n.kind.clone())
                    .or_default()
                    .entry(texs[w] - texs[w - 1])
                    .or_default() += 1;
            }
            for off in (0..params.len().saturating_sub(3))
                .step_by(4)
                .take(20)
            {
                let Some(v) = u32_at(params, off) else { continue };
                *hist.entry(n.kind.clone())
                    .or_default()
                    .entry(off)
                    .or_default()
                    .entry(v)
                    .or_default() += 1;
                if v as usize == texs.len() {
                    *count_hits.entry(n.kind.clone()).or_default().entry(off).or_default() += 1;
                }
            }
        }
    }

    println!(
        "== 文件 {} · 解析失败 {} · 总节点 {} · 贴图引用 {} · 有贴图节点 {} ==",
        files.len(),
        fails,
        totals.values().sum::<usize>(),
        tex_ref_total,
        tex_node_total
    );
    // 正常带贴图的类型（EffectMsg/EffectSound 无贴图，计数字段全是 0 值噪音）
    let tex_kinds = [
        "EffectQuad",
        "EffectParticleEmitter",
        "EffectCross",
        "EffectCylinder",
        "EffectBeam",
        "EffectGrid",
        "EffectSphere",
        "EffectMesh",
    ];
    for (kind, total) in &totals {
        println!("\n== {kind}（{total} 节点）==");
        // 贴图槽步长
        if let Some(s) = strides.get(kind) {
            let mut v: Vec<_> = s.iter().collect();
            v.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
            let top: Vec<String> = v
                .iter()
                .take(4)
                .map(|(d, c)| format!("{d}B×{c}"))
                .collect();
            println!("  贴图引用相邻偏移差: {}", top.join("  "));
        }
        // 计数字段候选（全部节点命中才报；贴图数为 0 的节点全是噪音，跳过）
        if let Some(ch) = count_hits.get(kind) {
            for (off, hits) in ch {
                if hits == total && tex_kinds.contains(&kind.as_str()) {
                    println!("  计数字段: u32@+{off} == 贴图数（{hits}/{total} 全命中）");
                }
            }
        }
        // u32 直方图（只列值域小的字段）
        if let Some(hh) = hist.get(kind) {
            for (off, vh) in hh {
                let n_at = vh.values().sum::<usize>();
                if vh.len() > 8 || vh.len() < 2 {
                    continue; // 单值字段列在布尔段；多值字段信息量低
                }
                let items: Vec<String> = vh
                    .iter()
                    .map(|(v, c)| format!("{v:#x}→{c}"))
                    .collect();
                println!("  u32@+{off} (n={n_at}): {}", items.join("  "));
            }
        }
    }

    // ---- 字节级下钻：单字节 0/1 字段（0xCD 填充包裹）与交叉验证 ----
    println!("\n== 字节级布尔字段（参数区 +0..96，值域 ⊆ {{0,1,2,0xCD}} 且含 0 与 1）==");
    // (kind, off) → 值 → 节点数
    let mut bytes: BTreeMap<(String, usize), BTreeMap<u8, usize>> = BTreeMap::new();
    for (path, _name) in &files {
        let Ok(m) = sfx_parse_for_test(path) else { continue };
        for n in &m.nodes {
            let params = &n.raw[36.min(n.raw.len())..];
            for off in 0..params.len().min(96) {
                *bytes
                    .entry((n.kind.clone(), off))
                    .or_default()
                    .entry(params[off])
                    .or_default() += 1;
            }
        }
    }
    for ((kind, off), vh) in &bytes {
        let has01 = vh.contains_key(&0) && vh.contains_key(&1);
        let small = vh.keys().all(|v| matches!(v, 0 | 1 | 2 | 0xCD));
        if has01 && small && vh.len() >= 2 {
            let items: Vec<String> = vh.iter().map(|(v, c)| format!("{v:#x}→{c}")).collect();
            println!("  {kind} byte@+{off}: {}", items.join("  "));
        }
    }

    // ---- +0/+8 两布尔与「有无贴图」交叉（跨类型汇总）----
    println!("\n== byte@+0 × byte@+8 交叉（跨类型）==");
    let mut cross: BTreeMap<(u8, u8, bool), usize> = BTreeMap::new();
    for (path, _name) in &files {
        let Ok(m) = sfx_parse_for_test(path) else { continue };
        for n in &m.nodes {
            let params = &n.raw[36.min(n.raw.len())..];
            let b0 = params.first().copied().unwrap_or(0xFF);
            let b8 = params.get(8).copied().unwrap_or(0xFF);
            *cross.entry((b0, b8, !n.textures.is_empty())).or_default() += 1;
        }
    }
    let mut rows: Vec<_> = cross.iter().collect();
    rows.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
    for ((b0, b8, has_tex), c) in rows.iter().take(12) {
        println!("  +0={b0:#04x} +8={b8:#04x} 有贴图={has_tex}: {c}");
    }

    // ---- byte@+0 异常值节点（前 8 个，看参数头 24B）----
    println!("\n== byte@+0 异常值节点（字段疑似变体/浮点的样本）==");
    let mut shown = 0;
    'outer: for (path, _name) in &files {
        let Ok(m) = sfx_parse_for_test(path) else { continue };
        for n in &m.nodes {
            let params = &n.raw[36.min(n.raw.len())..];
            let Some(&b0) = params.first() else { continue };
            if matches!(b0, 0 | 1 | 0xCD) || params.get(1) != Some(&0xCD) {
                continue;
            }
            println!(
                "  {} #{} {} {:?}: {}",
                path.rsplit(['\\', '/']).next().unwrap_or(path),
                n.raw
                    .get(0..4)
                    .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
                    .unwrap_or(0),
                n.kind,
                n.name,
                params
                    .iter()
                    .take(24)
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            shown += 1;
            if shown >= 8 {
                break 'outer;
            }
        }
    }

    // ---- +0=1 / +8=1 节点样本与文件分布（语义线索）----
    println!("\n== byte@+0=1 节点样本（文件 / 类型 / 名）==");
    let mut files_with_flag: BTreeMap<String, usize> = BTreeMap::new();
    let mut samples = 0;
    let mut all_flagged = 0usize;
    for (path, _name) in &files {
        let Ok(m) = sfx_parse_for_test(path) else { continue };
        let mut in_file = 0usize;
        for n in &m.nodes {
            let params = &n.raw[36.min(n.raw.len())..];
            if params.first() == Some(&1) {
                in_file += 1;
                if samples < 24 {
                    println!(
                        "  {} · {} · {:?}",
                        path.rsplit(['\\', '/']).next().unwrap_or(path),
                        n.kind,
                        n.name
                    );
                    samples += 1;
                }
            }
        }
        if in_file > 0 {
            files_with_flag.insert(
                path.rsplit(['\\', '/']).next().unwrap_or(path).to_string(),
                in_file,
            );
        }
        if in_file > 0 && in_file == m.nodes.len() {
            all_flagged += 1;
            if all_flagged <= 10 {
                println!(
                "  [全节点+0=1] {}（{} 节点）",
                path.rsplit(['\\', '/']).next().unwrap_or(path),
                m.nodes.len()
            );
            }
        }
    }
    println!(
        "  +0=1 涉及文件数: {} / 总 {} · 整文件全节点 +0=1 的文件: {}",
        files_with_flag.len(),
        files.len(),
        all_flagged
    );
}
