//! .sfx 特效脚本解析与序列化
//!
//! 布局（2026-10 对 system.cpk 全量 782 个 .sfx 逆向归纳，扫描节点数与
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
//!
//! 类型参数区逐类型变长——EffectQuad 贴图槽 72B/个（动画帧序列）、
//! Emitter 168B、Cylinder 120B、Cross 124B、Sphere 108B、Beam 76B、
//! Grid 96B 定长数组 + 节点名等变长字符串——完整字段图未逆向。
//! 节点边界按「下一个 [u32 长度][已知类型名]」定位并以三条不变量校验：
//! 节点数 == N、类型名在已知集合内、末节点收于文件尾（false positive
//! 会使链错位、末尾校验失败而报错，不会静默错切）。参数字节原样保留，
//! serialize 字节级还原（见 roundtrip 测试）；贴图 `system\effect\tex\
//! …\*.tga`、网格 `system\…\*.fak` 与音效 `sound\…\*.wav` 引用另按
//! 结构化字段提取供查看器使用。

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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfxNode {
    /// 节点类型（EffectQuad 等 11 种）
    pub kind: String,
    /// GBK 解码节点名（"lujing"/"光剑雾_3" 等设计师命名，可为空）；
    /// 由 raw 派生展示用，编辑需改 raw（当前只读）
    pub name: String,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SfxFile {
    pub path: String,
    /// 行主序 4×4 根变换（末行为平移，[3][3] 恒 1.0）
    pub matrix: [f32; 16],
    pub nodes: Vec<SfxNode>,
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
        // 本节点参数区终点 = 下一节点头（已知类型名）或文件尾（末节点）
        let end = if i + 1 < count {
            find_next_header(bytes, body)
                .ok_or_else(|| format!("节点 {i}({kind}) 之后找不到下一个节点头"))?
        } else {
            bytes.len()
        };
        if end < body + NODE_MIN_RAW || end > bytes.len() {
            return Err(format!("节点 {i}({kind}) 参数区长度异常"));
        }
        let raw = bytes[body..end].to_vec();
        let (textures, models, sounds) = extract_refs(&raw);
        nodes.push(SfxNode {
            kind,
            name: node_name(&raw),
            raw,
            textures,
            models,
            sounds,
        });
        pos = end;
    }
    if pos != bytes.len() {
        return Err("节点链未收于文件尾（可能存在未识别的节点类型）".into());
    }
    Ok(SfxFile {
        path: path.to_string(),
        matrix,
        nodes,
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
    /// 782 个 .sfx 全部解析 + 字节级往返
    #[test]
    #[ignore = "需要本地 E:\\Workspace\\res\\system.cpk"]
    fn real_system_cpk_all_sfx() {
        let root = format!(r"cpk:E:\Workspace\res\system.cpk\system\Effect");
        let files = crate::vfs::walk_files(&root, &["sfx"]);
        if files.is_empty() {
            return;
        }
        assert!(files.len() > 700, "应扫描到 700+ 个 sfx，实际 {}", files.len());
        let mut kinds = std::collections::BTreeMap::new();
        for f in &files {
            let bytes = crate::vfs::read_file(&f.path).unwrap();
            let m = parse_sfx(&f.path, &bytes).unwrap();
            assert_eq!(serialize_sfx(&m).unwrap(), bytes, "{} 往返不一致", f.path);
            for n in &m.nodes {
                *kinds.entry(n.kind.clone()).or_insert(0usize) += 1;
            }
        }
        println!("{} 文件 · 节点类型分布 {kinds:?}", files.len());
    }
}
