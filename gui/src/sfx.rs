//! .sfx 特效脚本解析与序列化
//!
//! 布局（2026-10 对 system.cpk 全量 786 个 .sfx 逆向归纳，扫描节点数与
//! 头部计数 100% 对齐；参数区常见 0xCD 未初始化填充，系 MSVC 调试版
//! 导出的原貌，原样保留）:
//!   0x00  f32×16 根变换矩阵（行主序，末行为平移，[3][3] 恒 1.0；
//!         Scn 场景特效为世界坐标摆放位，Combat 特效多为近单位阵）
//!   0x40  u32 节点数 N
//!   0x44  节点记录 ×N 顺序排列:
//!     [u32 类型名长(含NUL)][类型名 NUL][u32 节点号][节点名 32B 缓冲
//!      GBK NUL 填充][类型参数变长区…]
//!   11 种节点类型: EffectQuad(公告板) / EffectParticleEmitter(粒子) /
//!   EffectCross(交叉面) / EffectCylinder(柱面) / EffectBeam(光束) /
//!   EffectGrid(面片) / EffectMesh(FAK 网格) / EffectSphere(球面) /
//!   EffectMsg(数值飘字) / EffectExplosive / EffectSound(音效)
//!   尾部路径数组（SGEffectNode::Deserialize 第二个对象数组，引擎侧
//!   AddEffectPath 挂接）: [u32 数量 M][M × 路径记录]，路径记录同为
//!   [u32 类型名长][类型名 NUL][路径数据]，实测 786 文件恒为 EffectMotion
//!   （速度积分路径，名字如 lujing/beamlujing，供节点 prop+80 挂接路径名引用）。
//!
//! 类型参数区逐类型变长（F3D.dll 各类 Deserialize 实测；[节点号][32B 名字]
//! 实为 prop 定长块的头部字段）——EffectQuad prop 184B、EffectCross 208B、
//! EffectMotion 124B，后接 [u32 状态数][状态 ×72B/124B/60B] 与 340B
//! EffectBase 公共尾块（贴图路径在 base+12）。prop 头部两性字段已逆向：
//! Quad/Cross 的 +112 = 位置偏移 vec3（公告板世界位 = 偏移+路径积分位置），
//! +80 = 32B 挂接路径名（none/空 = 定点）。完整状态字段图未逆向。
//! 节点边界按「下一个 [u32 长度][已知类型名]」定位并以三条不变量校验：
//! 节点数 == N、类型名在已知集合内、末节点收于尾部路径数组/文件尾（false
//! positive 会使链错位、末尾校验失败而报错，不会静默错切）。参数字节
//! 原样保留，serialize 字节级还原（见 roundtrip 测试）；贴图
//! `system\effect\tex\…\*.tga`、网格 `system\…\*.fak` 与音效
//! `sound\…\*.wav` 引用另按结构化字段提取供查看器使用。

use serde::{Deserialize, Serialize};

/// 已知节点类型（大小写按文件原样匹配；新类型出现时解析会报错并提示）
pub const KNOWN_KINDS: [&str; 11] = [
    "EffectQuad",
    "EffectParticleEmitter",
    "EffectCross",
    "EffectCylinder",
    "EffectBeam",
    "EffectGrid",
    "EffectMesh",
    "EffectSphere",
    "EffectMsg",
    "EffectExplosive",
    "EffectSound",
];

const HEADER_LEN: usize = 0x44;
/// 节点号(u32) + 名字 32B 缓冲的最小参数区长度
const NODE_MIN_RAW: usize = 36;
const NAME_BUF: usize = 32;
/// EffectMotion 路径记录的 prop 定长块（[u32 id][32B 名字] 头部 + 类型参数）
const MOTION_PROP: usize = 124;
/// EffectMotion 状态定长 60B（前 28B = 时长+速度+角速度 7×f32，余 32B 未逆向）
const MOTION_STATE: usize = 60;
/// EffectBase 公共尾块长度（786 文件实测；个别节点存档为 344，由边界扫描兼容）
const BASE_BLOB: usize = 340;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfxNode {
    /// 节点类型（EffectQuad 等 11 种）
    pub kind: String,
    /// GBK 解码节点名（"lujing"/"光剑雾_3" 等设计师命名，可为空）；
    /// 由 raw 派生展示用，编辑需改 raw（当前只读）
    pub name: String,
    /// 位置偏移（prop+112 的 D3DXVECTOR3；Quad/Cross 逆向确证，
    /// 其余类型 prop 布局未逆向恒 [0,0,0]）。公告板位 = 偏移 + 路径积分位置
    pub offset: [f32; 3],
    /// 挂接路径名（prop+80；"none"/空 = 定点，否则引用 paths 中同名 EffectMotion）
    pub path_name: String,
    /// 类型名之后的整段原始字节（节点号 + 名字缓冲 + 类型参数），
    /// 序列化原样写回保证字节级还原
    pub raw: Vec<u8>,
    /// 贴图引用（system\effect\tex\…\*.tga，含逐帧动画序列）
    pub textures: Vec<String>,
    /// 网格引用（EffectMesh 节点的 system\…\*.fak）
    pub models: Vec<String>,
    /// 音效引用（EffectSound 节点的 sound\…\*.wav）
    pub sounds: Vec<String>,
}

/// 尾部路径对象（EffectPathCtrl 派生，引擎 SGEffectNode 按名挂接；
/// 实测数据恒为 EffectMotion = 速度积分路径）
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfxPath {
    /// 记录类型（"EffectMotion"）
    pub kind: String,
    /// 路径名（"lujing"/"bblujing"/"旋转1" 等，节点 path_name 引用此名）
    pub name: String,
    /// 起始延迟秒（prop+40；特效龄超过它才开始积分）
    pub delay: f32,
    /// 循环播放（prop+44 非 0；末态后回绕首态，否则保持末态速度）
    pub looped: bool,
    /// 关键帧状态 {时长s, 速度xyz, 角速度xyz}（每态 60B 前 28B）；
    /// 位置 = 速度逐帧积分（引擎语义），角速度只作用于路径自身姿态不影响公告板
    pub states: Vec<[f32; 7]>,
    /// 记录原始字节（类型名之后），序列化原样写回
    pub raw: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfxFile {
    pub path: String,
    /// 行主序 4×4 根变换（末行为平移，[3][3] 恒 1.0）
    pub matrix: [f32; 16],
    pub nodes: Vec<SfxNode>,
    /// 尾部路径数组（可为空 = 数量 0 的 4 字节仍在文件中，序列化还原）
    pub paths: Vec<SfxPath>,
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// 在 from 起向前找下一个节点头 [u32 长度][已知类型名\0]；找不到返回 None。
/// 已知类型名匹配是防 false positive 的关键闸门（参数区浮点/路径字节
/// 恰好拼出「长度+完整类型名+NUL」的概率可忽略，且末尾校验兜底）
fn find_next_header(b: &[u8], from: usize) -> Option<usize> {
    for o in from..b.len().saturating_sub(8) {
        let Some(l) = u32_at(b, o) else { continue };
        let l = l as usize;
        // 已知类型名 9~21 字节 + NUL
        if !(10..=22).contains(&l) || o + 4 + l > b.len() {
            continue;
        }
        let s = &b[o + 4..o + 4 + l];
        if s.last() == Some(&0) && KNOWN_KINDS.iter().any(|k| k.as_bytes() == &s[..l - 1]) {
            return Some(o);
        }
    }
    None
}

/// 参数区里的 NUL 结尾可打印路径引用（system\… / sound\… 前缀，全小写）
fn extract_refs(raw: &[u8]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut textures, mut models, mut sounds) = (Vec::new(), Vec::new(), Vec::new());
    let mut i = 0;
    while i < raw.len() {
        let start = i;
        while i < raw.len() && (0x20..0x7f).contains(&raw[i]) {
            i += 1;
        }
        let token = &raw[start..i];
        if !token.is_empty()
            && i < raw.len()
            && raw[i] == 0
            && token.len() >= 8
            && token.iter().any(|&c| c == b'\\')
        {
            let lower_eq = |p: &[u8]| token.len() >= p.len() && token[..p.len()].eq_ignore_ascii_case(p);
            let path = crate::fskin::decode_path(token);
            if lower_eq(b"system\\") {
                if token.ends_with(b".fak") || token.ends_with(b".FAK") {
                    models.push(path);
                } else {
                    textures.push(path);
                }
            } else if lower_eq(b"sound\\") {
                sounds.push(path);
            }
        }
        i += 1;
    }
    (textures, models, sounds)
}

/// raw 参数区头部取节点名：[u32 节点号][32B 名字缓冲]，GBK 解码到 NUL
fn node_name(raw: &[u8]) -> String {
    let buf = &raw[4..4 + NAME_BUF];
    let end = buf.iter().position(|&c| c == 0).unwrap_or(NAME_BUF);
    crate::fskin::decode_path(&buf[..end])
}

/// prop 头部派生字段（Quad/Cross 逆向确证）：+112 偏移 vec3 / +80 路径名。
/// 其余类型 prop 布局未逆向，偏移恒 0、路径名恒空
fn node_place(kind: &str, raw: &[u8]) -> ([f32; 3], String) {
    if kind != "EffectQuad" && kind != "EffectCross" {
        return ([0.0, 0.0, 0.0], String::new());
    }
    let f = |o: usize| f32::from_bits(u32_at(raw, o).unwrap_or(0));
    let offset = [f(112), f(116), f(120)];
    let buf = &raw[80..80 + NAME_BUF];
    let end = buf.iter().position(|&c| c == 0).unwrap_or(NAME_BUF);
    (offset, crate::fskin::decode_path(&buf[..end]))
}

/// 校验并返回尾部路径数组 [u32 M][M×记录] 在 o 处的记录区间表
/// （kind, body..end）。体长按 EffectMotion = prop124 + 4+N×60 + base340
/// 推进；链须恰好收于文件尾，否则该候选作废（786 文件实测全闭合）
fn try_path_array(b: &[u8], o: usize) -> Option<Vec<(&'static str, usize, usize)>> {
    let m = u32_at(b, o)? as usize;
    if m > 99 {
        return None;
    }
    let mut p = o + 4;
    let mut recs = Vec::with_capacity(m);
    for _ in 0..m {
        let tlen = u32_at(b, p)? as usize;
        if !(6..=64).contains(&tlen) || p + 4 + tlen > b.len() {
            return None;
        }
        let name = &b[p + 4..p + 4 + tlen];
        if name.last() != Some(&0) || !name[..tlen - 1].iter().all(|&c| (0x20..0x7f).contains(&c)) {
            return None;
        }
        let kind = std::str::from_utf8(&name[..tlen - 1]).ok()?;
        let body = p + 4 + tlen;
        if kind != "EffectMotion" {
            return None; // 未出现的记录类型（EffectTarget 等），整体回退
        }
        let n = u32_at(b, body + MOTION_PROP)? as usize;
        if n > 99 {
            return None;
        }
        let end = body + MOTION_PROP + 4 + n * MOTION_STATE + BASE_BLOB;
        if end > b.len() {
            return None;
        }
        recs.push(("EffectMotion", body, end));
        p = end;
    }
    if p == b.len() {
        Some(recs)
    } else {
        None
    }
}

/// 末节点参数区终点 = 尾部路径数组起点（最早的有效候选），无则文件尾。
/// 链式校验（数量/类型名/体长恰好收尾）使浮点数据内的 false positive
/// 概率可忽略；无候选时维持旧口径（末节点吞到文件尾、paths 为空）
fn find_tail_start(b: &[u8], body: usize) -> Option<usize> {
    (body + NODE_MIN_RAW..b.len().saturating_sub(3)).find(|&o| try_path_array(b, o).is_some())
}

/// 已定位的路径数组 → 结构化记录（字段区间由 try_path_array 保证合法）
fn parse_path_array(b: &[u8], start: usize) -> Vec<SfxPath> {
    let recs = try_path_array(b, start).unwrap_or_default();
    recs
        .into_iter()
        .map(|(kind, body, end)| {
            let buf = &b[body + 4..body + 4 + NAME_BUF];
            let ne = buf.iter().position(|&c| c == 0).unwrap_or(NAME_BUF);
            let n = u32_at(b, body + MOTION_PROP).unwrap_or(0) as usize;
            let states = (0..n)
                .map(|j| {
                    let s = body + MOTION_PROP + 4 + j * MOTION_STATE;
                    let mut st = [0f32; 7];
                    for (q, v) in st.iter_mut().enumerate() {
                        *v = f32::from_bits(u32_at(b, s + q * 4).unwrap_or(0));
                    }
                    st
                })
                .collect();
            SfxPath {
                kind: kind.to_string(),
                name: crate::fskin::decode_path(&buf[..ne]),
                delay: f32::from_bits(u32_at(b, body + 40).unwrap_or(0)),
                looped: b.get(body + 44).copied().unwrap_or(0) != 0,
                states,
                raw: b[body..end].to_vec(),
            }
        })
        .collect()
}

pub fn parse_sfx(path: &str, bytes: &[u8]) -> Result<SfxFile, String> {
    if bytes.len() < HEADER_LEN {
        return Err("文件太小".into());
    }
    let mut matrix = [0f32; 16];
    for (i, m) in matrix.iter_mut().enumerate() {
        *m = f32::from_bits(u32_at(bytes, i * 4).unwrap());
    }
    if matrix[15] != 1.0 {
        return Err(format!("根矩阵 [3][3] 应为 1.0，实际 {}", matrix[15]));
    }
    let count = u32_at(bytes, 0x40).unwrap() as usize;
    if count == 0 || count > (bytes.len() - HEADER_LEN) / 8 {
        return Err(format!("节点数异常: {count}"));
    }

    let mut pos = HEADER_LEN;
    let mut nodes = Vec::with_capacity(count);
    for i in 0..count {
        if pos + 4 > bytes.len() {
            return Err(format!("节点 {i} 头部越界"));
        }
        let tlen = u32_at(bytes, pos).unwrap() as usize;
        if !(6..=64).contains(&tlen) || pos + 4 + tlen > bytes.len() {
            return Err(format!("节点 {i} 类型名长度异常: {tlen}"));
        }
        let tbytes = &bytes[pos + 4..pos + 4 + tlen];
        if tbytes[tlen - 1] != 0 || !tbytes[..tlen - 1].iter().all(|&c| (0x20..0x7f).contains(&c)) {
            return Err(format!("节点 {i} 类型名非法"));
        }
        let kind = String::from_utf8_lossy(&tbytes[..tlen - 1]).into_owned();
        let body = pos + 4 + tlen;
        // 本节点参数区终点 = 下一节点头（已知类型名）或文件尾（末节点），
        // 末节点实际收于尾部路径数组起点（find_tail_start）
        let end = if i + 1 < count {
            find_next_header(bytes, body)
                .ok_or_else(|| format!("节点 {i}({kind}) 之后找不到下一个节点头"))?
        } else {
            find_tail_start(bytes, body).unwrap_or(bytes.len())
        };
        if end < body + NODE_MIN_RAW || end > bytes.len() {
            return Err(format!("节点 {i}({kind}) 参数区长度异常"));
        }
        let raw = bytes[body..end].to_vec();
        let (textures, models, sounds) = extract_refs(&raw);
        let (offset, path_name) = node_place(&kind, &raw);
        nodes.push(SfxNode {
            kind,
            name: node_name(&raw),
            offset,
            path_name,
            raw,
            textures,
            models,
            sounds,
        });
        pos = end;
    }
    // 尾部路径数组（结构在 find_tail_start 已校验恰好收于文件尾）
    let paths = if pos < bytes.len() {
        parse_path_array(bytes, pos)
    } else {
        Vec::new()
    };
    Ok(SfxFile {
        path: path.to_string(),
        matrix,
        nodes,
        paths,
    })
}

pub fn serialize_sfx(f: &SfxFile) -> Result<Vec<u8>, String> {
    if f.nodes.is_empty() {
        return Err("节点列表为空".into());
    }
    let mut out = Vec::with_capacity(HEADER_LEN + f.nodes.iter().map(|n| n.raw.len() + 32).sum::<usize>());
    for m in f.matrix {
        out.extend_from_slice(&m.to_bits().to_le_bytes());
    }
    out.extend_from_slice(&(f.nodes.len() as u32).to_le_bytes());
    for n in &f.nodes {
        if n.kind.is_empty()
            || n.kind.len() + 1 > 64
            || !n.kind.bytes().all(|c| (0x20..0x7f).contains(&c))
        {
            return Err(format!("节点类型名非法: {}", n.kind));
        }
        out.extend_from_slice(&((n.kind.len() + 1) as u32).to_le_bytes());
        out.extend_from_slice(n.kind.as_bytes());
        out.push(0);
        out.extend_from_slice(&n.raw);
    }
    // 尾部路径数组（[u32 M][M×记录]，记录字节原样写回）
    out.extend_from_slice(&(f.paths.len() as u32).to_le_bytes());
    for p in &f.paths {
        if p.kind.is_empty()
            || p.kind.len() + 1 > 64
            || !p.kind.bytes().all(|c| (0x20..0x7f).contains(&c))
        {
            return Err(format!("路径类型名非法: {}", p.kind));
        }
        out.extend_from_slice(&((p.kind.len() + 1) as u32).to_le_bytes());
        out.extend_from_slice(p.kind.as_bytes());
        out.push(0);
        out.extend_from_slice(&p.raw);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res");

    fn read(rel: &str) -> Vec<u8> {
        std::fs::read(format!("{RES}/system/Effect/{rel}")).unwrap()
    }

    /// 三个样例（Combat 主战特效 / Scn 场景雨 / role 含 EffectMesh）字节级往返
    #[test]
    fn roundtrip_samples() {
        for rel in ["Combat/tx01a_a.sfx", "Scn/tc06.sfx", "role/bs016d.sfx"] {
            let bytes = read(rel);
            let m = parse_sfx(rel, &bytes).unwrap();
            assert_eq!(serialize_sfx(&m).unwrap(), bytes, "{rel} 往返不一致");
        }
    }

    #[test]
    fn parse_tx01a_a() {
        let bytes = read("Combat/tx01a_a.sfx");
        let m = parse_sfx("tx01a_a.sfx", &bytes).unwrap();
        assert_eq!(m.nodes.len(), 9);
        let kinds: Vec<&str> = m.nodes.iter().map(|n| n.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "EffectQuad",
                "EffectCross",
                "EffectCross",
                "EffectQuad",
                "EffectCylinder",
                "EffectQuad",
                "EffectParticleEmitter",
                "EffectParticleEmitter",
                "EffectQuad",
            ]
        );
        // 根矩阵：单位旋转 + 世界平移（[3][3]=1）
        assert_eq!(m.matrix[0], 1.0);
        assert_eq!(m.matrix[5], 1.0);
        assert_eq!(m.matrix[10], 1.0);
        assert!((m.matrix[12] - 3195.73).abs() < 0.1, "{}", m.matrix[12]);
        assert!((m.matrix[13] - -5.17).abs() < 0.1, "{}", m.matrix[13]);
        assert!((m.matrix[14] - 5008.61).abs() < 0.1, "{}", m.matrix[14]);
        // 首节点：GBK 中文名 + system\effect\tex 贴图引用
        assert!(m.nodes[0].name.contains("光传"), "{}", m.nodes[0].name);
        assert!(
            m.nodes[0]
                .textures
                .iter()
                .all(|t| t.starts_with(r"system\effect\tex\")),
            "{:?}",
            m.nodes[0].textures
        );
        assert!(!m.nodes[0].textures.is_empty());
        // 首节点（Quad）逆向字段：位置偏移 (0,100,0)，挂接路径名 none（定点）
        assert_eq!(m.nodes[0].path_name, "none");
        assert!((m.nodes[0].offset[0] - 0.0).abs() < 1e-4);
        assert!((m.nodes[0].offset[1] - 100.0).abs() < 1e-4);
        assert!((m.nodes[0].offset[2] - 0.0).abs() < 1e-4);
        // 本样例无路径对象（尾部仅 4 字节 0 计数，序列化仍须还原）
        assert!(m.paths.is_empty());
    }

    /// 带路径对象的样例（system.cpk 786 文件中 278 个尾部含 EffectMotion）：
    /// 节点链与路径数组切分后各引用同名，roundtrip 依旧字节级
    #[test]
    #[ignore = "需要本地游戏 system.cpk"]
    fn paths_from_game_cpk() {
        let root = r"cpk:E:\Game\Online\新功夫世界\system.cpk\system\Effect\Combat";
        let bytes = match crate::vfs::read_file(&format!(r"{root}\txch01_a.sfx")) {
            Ok(b) => b,
            Err(_) => return, // 包不存在则跳过
        };
        let m = parse_sfx("txch01_a.sfx", &bytes).unwrap();
        let lujing = m.paths.iter().find(|p| p.name == "lujing").expect("lujing 路径");
        assert_eq!(lujing.kind, "EffectMotion");
        assert!(lujing.looped, "lujing 应循环");
        // 两态：3s 上升 17/s + 3s 下降 17/s（浮动路径）
        assert_eq!(lujing.states.len(), 2);
        assert!((lujing.states[0][0] - 3.0).abs() < 1e-3);
        assert!((lujing.states[0][2] - 17.0).abs() < 1e-3);
        assert!((lujing.states[1][2] + 17.0).abs() < 1e-3);
        assert!(
            m.nodes.iter().any(|n| n.path_name == "lujing"),
            "应有节点挂接 lujing 路径"
        );
        assert_eq!(serialize_sfx(&m).unwrap(), bytes, "带路径数组往返不一致");
    }

    #[test]
    fn parse_tc06_sound_node() {
        let bytes = read("Scn/tc06.sfx");
        let m = parse_sfx("tc06.sfx", &bytes).unwrap();
        assert_eq!(m.nodes.len(), 2);
        assert_eq!(m.nodes[0].kind, "EffectParticleEmitter");
        assert_eq!(m.nodes[0].name, "yu");
        let snd = &m.nodes[1];
        assert_eq!(snd.kind, "EffectSound");
        assert_eq!(snd.sounds, vec![r"sound\effect\x176.wav".to_string()]);
        assert!(snd.textures.is_empty());
        // 平移为场景世界坐标（2600+ 量级）
        assert!(m.matrix[12].abs() > 1000.0, "{}", m.matrix[12]);
    }

    #[test]
    fn parse_bs016d_mesh_ref() {
        let bytes = read("role/bs016d.sfx");
        let m = parse_sfx("bs016d.sfx", &bytes).unwrap();
        let mesh = m.nodes.iter().find(|n| n.kind == "EffectMesh").unwrap();
        assert!(
            mesh.models.iter().all(|p| p.ends_with(".fak")),
            "{:?}",
            mesh.models
        );
    }

    #[test]
    fn reject_corrupt() {
        let bytes = read("Combat/tx01a_a.sfx");
        // 头部 / 首节点头截断
        assert!(parse_sfx("x", &bytes[..0x40]).is_err());
        assert!(parse_sfx("x", &bytes[..0x48]).is_err());
        // 节点数改大：链收不到文件尾
        let mut b2 = bytes.clone();
        b2[0x40..0x44].copy_from_slice(&10u32.to_le_bytes());
        assert!(parse_sfx("x", &b2).is_err());
        // 根矩阵 [3][3] 破坏
        let mut b3 = bytes;
        b3[0x3C..0x40].copy_from_slice(&2.0f32.to_bits().to_le_bytes());
        assert!(parse_sfx("x", &b3).is_err());
        // 注：末节点参数区中段截断 1~2 字节格式上不可分辨（参数区变长，
        // 边界按下一节点头定位），属该格式固有特性
    }

    /// 本机大包全量冒烟（手动 `--ignored` 运行；包不存在则跳过）：
    /// 786 个 .sfx 全部解析 + 字节级往返（含尾部 EffectMotion 路径数组切分）
    #[test]
    #[ignore = "需要本地 E:\\Game\\Online\\新功夫世界\\system.cpk"]
    fn real_system_cpk_all_sfx() {
        let root = format!(r"cpk:E:\Game\Online\新功夫世界\system.cpk\system\Effect");
        let files = crate::vfs::walk_files(&root, &["sfx"]);
        if files.is_empty() {
            return;
        }
        assert!(files.len() > 700, "应扫描到 700+ 个 sfx，实际 {}", files.len());
        let mut kinds = std::collections::BTreeMap::new();
        let mut with_paths = 0usize;
        for f in &files {
            let bytes = crate::vfs::read_file(&f.path).unwrap();
            let m = parse_sfx(&f.path, &bytes).unwrap();
            assert_eq!(serialize_sfx(&m).unwrap(), bytes, "{} 往返不一致", f.path);
            for n in &m.nodes {
                *kinds.entry(n.kind.clone()).or_insert(0usize) += 1;
            }
            if !m.paths.is_empty() {
                with_paths += 1;
            }
        }
        println!(
            "{} 文件 · 含路径对象 {} · 节点类型分布 {kinds:?}",
            files.len(),
            with_paths
        );
    }
}
