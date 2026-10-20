//! .fskel (SKEL v1.0) 骨架动画解析与导出
//!
//! 逆向结论（2026-10，AvatarGirl/act 全部 294 文件 17209 帧逐帧验证）：
//!
//! ```text
//! 0x00  "SKEL" + u16 版本(0x0100) + u16 校验
//! 0x08  u32 头大小(0x114)   0x0C u32 帧数   0x10 u32 帧区偏移
//! 0x14  [magic 0xB33BBD2E][01] 骨架对象内存映像（含 x86 指针残渣）
//! 0x114 u32 骨骼数；0x118.. 字符串表 [u32 长度(含\0)][骨骼名]×N + 空串结尾
//! 帧区  每帧（= 一个完整姿态，帧名重复动作名）：
//!       [magic][01][f32 帧间隔][u32][NUL 动作名][u32 骨数]
//!       [0][-1][1][1] 根记录头 + 整棵骨骼树（链式序列化）直至下一帧
//! EOF   [magic][01][f32 1/30][u32 0]
//!
//! 节点记录 [u32 id][u32 父id][u32 子数]：
//!   子数=0（叶子）: [T3][Qxyz]              — Qw=+0.0 时引擎省略
//!   子数≥1: [u32 链子id][子数-1×附加子链根][T3][Q4][u8 00 可选]
//! 链间分隔符（语义未定，按字节形态跳过）: [magic][01] / 裸 01 / f32 1.0 / 00
//! ```
//!
//! 坐标系 Y-up，骨骼世界矩阵 = 父 × T × R(Q)，帧率 30 FPS。
//! 动作时长与 ActTime.csv 完全对应（帧数 = round(毫秒×30/1000)）。

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

const MAGIC: [u8; 4] = [0x2e, 0xbd, 0x3b, 0xb3];
/// EOF 尾注里的帧间隔，兜底用
const DEFAULT_DT: f32 = 1.0 / 30.0;

// ---------------------------------------------------------------- 数据结构

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FskelAction {
    pub name: String,
    pub first_frame: u32,
    pub frame_count: u32,
}

/// 目录扫描用的轻量摘要（只读头部 + 首帧动作名，不逐帧解析）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FskelBrief {
    pub path: String,
    pub file_name: String,
    pub frame_count: u32,
    pub bone_count: u32,
    pub byte_len: u64,
    pub action_name: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FskelMeta {
    pub path: String,
    pub file_name: String,
    pub version: u16,
    pub checksum: u16,
    pub frame_count: u32,
    pub frame_dt: f32,
    pub bone_count: u32,
    pub bone_names: Vec<String>,
    /// 每根骨骼的父索引，根为 -1
    pub parents: Vec<i32>,
    /// 动作分段（同名连续帧合并）
    pub actions: Vec<FskelAction>,
}

#[derive(Clone, Copy, Debug)]
struct BonePose {
    t: [f32; 3],
    q: [f32; 4],
}

pub struct FskelFile {
    pub path: String,
    version: u16,
    checksum: u16,
    pub bone_names: Vec<String>,
    /// 每根骨骼的父索引（取首帧记录流），根为 -1
    pub parents: Vec<i32>,
    pub frame_dt: f32,
    /// 动作分段（同名连续帧合并）
    pub actions: Vec<FskelAction>,
    frames: Vec<Vec<BonePose>>,
}

// ---------------------------------------------------------------- 读取工具

fn u32_at(b: &[u8], p: usize) -> Option<u32> {
    if p + 4 > b.len() {
        return None;
    }
    Some(u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]))
}

fn f32_at(b: &[u8], p: usize) -> Option<f32> {
    u32_at(b, p).map(f32::from_bits)
}

// ---------------------------------------------------------------- 解析

fn read_header(b: &[u8]) -> Result<(u16, u16, u32, usize), String> {
    if b.len() < 0x20 || &b[0..4] != b"SKEL" {
        return Err("不是 SKEL 文件".into());
    }
    let version = u16::from_le_bytes([b[4], b[5]]);
    let checksum = u16::from_le_bytes([b[6], b[7]]);
    let header_size = u32_at(b, 8).ok_or("文件太短")? as usize;
    let frame_count = u32_at(b, 0xc).ok_or("文件太短")?;
    Ok((version, checksum, frame_count, header_size))
}

/// 资源列举用的轻量头部信息：(帧数, 骨骼数)，非 SKEL 文件返回 None
pub(crate) fn light_meta(b: &[u8]) -> Option<(u32, u32)> {
    let (_v, _c, frame_count, header_size) = read_header(b).ok()?;
    let bone_count = u32_at(b, header_size)?;
    Some((frame_count, bone_count))
}

/// 字符串表：[u32 长度(含 NUL)][字节] 连续存放，返回 (名字列表, 结束偏移)
fn read_names(b: &[u8], mut pos: usize, limit: usize) -> (Vec<String>, usize) {
    let mut names = Vec::new();
    while pos + 4 <= limit {
        let ln = u32_at(b, pos).unwrap_or(0) as usize;
        if ln == 0 || ln > 256 || pos + 4 + ln > limit {
            break;
        }
        let s = &b[pos + 4..pos + 4 + ln - 1];
        // 条目末字节应为 NUL，内容为可打印 ASCII
        if b.get(pos + 4 + ln - 1) != Some(&0) || s.iter().any(|&c| !(0x20..0x7f).contains(&c)) {
            break;
        }
        names.push(String::from_utf8_lossy(s).into_owned());
        pos += 4 + ln;
    }
    (names, pos)
}

/// 帧签名 [骨数][0][-1][1][1] 定位所有帧记录，返回 (偏移, 动作名)
fn find_frames(b: &[u8], bone_count: u32) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if b.len() < 24 {
        return out;
    }
    let pat = bone_count.to_le_bytes();
    let n = b.len() - 20;
    for i in 0..=n {
        if b[i..i + 4] != pat {
            continue;
        }
        if u32_at(b, i + 4) == Some(0)
            && u32_at(b, i + 8) == Some(0xffff_ffff)
            && u32_at(b, i + 12) == Some(1)
            && u32_at(b, i + 16) == Some(1)
        {
            // 名字回扫：跳过一个 NUL 后取可打印段
            let mut s = i;
            if s > 0 && b[s - 1] == 0 {
                s -= 1;
            }
            while s > 0 && (32..127).contains(&b[s - 1]) {
                s -= 1;
            }
            let name = String::from_utf8_lossy(&b[s..i])
                .trim_end_matches('\0')
                .to_string();
            out.push((i, name));
        }
    }
    out
}

/// 帧内节点记录流的遍历器
struct Walker<'a> {
    b: &'a [u8],
    bone_count: u32,
}

impl<'a> Walker<'a> {
    fn looks_like_node(&self, p: usize) -> bool {
        match (u32_at(self.b, p), u32_at(self.b, p + 4), u32_at(self.b, p + 8)) {
            (Some(i), Some(par), Some(fl)) => {
                i < self.bone_count && par < self.bone_count && fl <= 32
            }
            _ => false,
        }
    }

    /// T3 + Q(x,y,z[,w])；引擎在 Qw=+0.0 时省略 w（magic 字节不可能成为合法 w）
    fn read_tq(&self, p: &mut usize) -> Result<BonePose, String> {
        let mut t = [0f32; 3];
        for v in t.iter_mut() {
            *v = f32_at(self.b, *p).ok_or("读取 T 越界")?;
            *p += 4;
        }
        let mut q = [0f32; 4];
        for v in q.iter_mut().take(3) {
            *v = f32_at(self.b, *p).ok_or("读取 Q 越界")?;
            *p += 4;
        }
        if self.b.len() >= *p + 4
            && self.b[*p..*p + 4] != MAGIC
            && u32_at(self.b, *p) != Some(0)
        {
            q[3] = f32_at(self.b, *p).unwrap_or(0.0);
            *p += 4;
        }
        if self.b.get(*p) == Some(&0) {
            *p += 1; // 可选 00 终止符
        }
        Ok(BonePose { t, q })
    }

    /// 节点记录 → (id, 父id, 姿态)。p 停在记录末尾。
    fn parse_record(&self, p: &mut usize) -> Result<(u32, i32, BonePose), String> {
        let id = u32_at(self.b, *p).ok_or("读取 id 越界")?;
        let par = u32_at(self.b, *p + 4).ok_or("读取 parent 越界")?;
        let flag = u32_at(self.b, *p + 8).ok_or("读取 flag 越界")?;
        *p += 12;
        if id >= self.bone_count || (par != 0xffff_ffff && par >= self.bone_count) || flag > 32 {
            return Err(format!("非法节点记录 id={id} par={par} flag={flag} @0x{:x}", *p - 12));
        }
        if flag > 0 {
            *p += 4; // 链子 id
            *p += 4 * (flag as usize - 1); // 附加子链根
        }
        let pose = self.read_tq(p)?;
        Ok((id, par as i32, pose))
    }

    /// 跳过链间分隔符：[magic][01] / 裸 01 / f32 1.0 / 00
    fn skip_seps(&self, p: &mut usize, end: usize) {
        while *p < end {
            if self.b[*p..].starts_with(&MAGIC) && self.b.get(*p + 4) == Some(&1) {
                *p += 5;
            } else if self.b[*p] == 1 && self.looks_like_node(*p + 1) {
                *p += 1;
            } else if self.b[*p..].starts_with(&[0, 0, 0x80, 0x3f]) {
                *p += 4;
            } else if self.b[*p] == 0 && !self.looks_like_node(*p) {
                *p += 1;
            } else {
                return;
            }
        }
    }

    /// 走完整帧。start = 帧签名后 20 字节（根记录的 T3Q4），end = 下一帧偏移。
    /// 返回 (每骨姿态, 每骨父id)。全部骨骼收齐后允许在尾部余渣处停止。
    fn walk_frame(&self, start: usize, end: usize) -> Result<(Vec<BonePose>, Vec<i32>), String> {
        let bc = self.bone_count as usize;
        let mut poses: Vec<Option<BonePose>> = vec![None; bc];
        let mut parents = vec![0i32; bc];
        let mut filled = 1usize;
        let mut p = start;
        poses[0] = Some(self.read_tq(&mut p)?);
        parents[0] = -1;
        let mut guard = 0usize;
        while p < end {
            guard += 1;
            if guard > bc * 4 + 8 {
                return Err(format!("帧解析陷入循环 @0x{p:x}"));
            }
            self.skip_seps(&mut p, end);
            if p >= end {
                break;
            }
            if !self.looks_like_node(p) {
                if filled == bc {
                    break; // 尾部余渣（下一帧前缀），骨骼已收齐
                }
                return Err(format!("帧数据无法识别 @0x{p:x}（已解析 {filled}/{bc} 骨骼）"));
            }
            let (id, par, pose) = self.parse_record(&mut p)?;
            if poses[id as usize].is_some() {
                if filled == bc {
                    break;
                }
                return Err(format!("骨骼 {id} 在同一帧重复出现"));
            }
            poses[id as usize] = Some(pose);
            parents[id as usize] = par;
            filled += 1;
        }
        let mut out = Vec::with_capacity(bc);
        for (i, po) in poses.into_iter().enumerate() {
            out.push(po.ok_or(format!("骨骼 {i} 缺少姿态数据"))?);
        }
        Ok((out, parents))
    }
}

/// 完整解析一个 .fskel
pub fn parse_fskel(path: &str, b: &[u8]) -> Result<FskelFile, String> {
    let (version, checksum, frame_count_hdr, hs) = read_header(b)?;
    let bone_count = u32_at(b, hs).ok_or("缺少骨骼数字段")?;
    let (mut names, names_end) = read_names(b, hs + 4, b.len());
    if names.last().is_some_and(|s| s.is_empty()) {
        names.pop(); // 末尾空串条目不计入
    }
    if names.len() as u32 != bone_count {
        return Err(format!(
            "骨骼名数量 {} 与骨骼数 {bone_count} 不符（字符串表结束于 0x{names_end:x}）",
            names.len()
        ));
    }

    let frames = find_frames(b, bone_count);
    if frames.len() as u32 != frame_count_hdr {
        return Err(format!(
            "帧定位 {} 与头部帧数 {frame_count_hdr} 不符",
            frames.len()
        ));
    }

    // EOF 尾注 [magic][01][f32 dt][u32 0]
    let frame_dt = if b.len() >= 10 && b[b.len() - 10..b.len() - 6] == MAGIC && b[b.len() - 6] == 1
    {
        f32_at(b, b.len() - 5).unwrap_or(DEFAULT_DT)
    } else {
        DEFAULT_DT
    };

    let walker = Walker { b, bone_count };
    let mut all_frames = Vec::with_capacity(frames.len());
    let mut parents = Vec::new();
    for (i, (off, _name)) in frames.iter().enumerate() {
        let end = frames
            .get(i + 1)
            .map(|(o, _)| *o)
            .unwrap_or_else(|| b.len().saturating_sub(20));
        let (poses, par) = walker.walk_frame(off + 20, end)?;
        if i == 0 {
            parents = par;
        }
        all_frames.push(poses);
    }

    // 同名连续帧合并为动作
    let mut actions: Vec<FskelAction> = Vec::new();
    for (i, (_off, name)) in frames.iter().enumerate() {
        match actions.last_mut() {
            Some(a) if a.name == *name => a.frame_count += 1,
            _ => actions.push(FskelAction {
                name: name.clone(),
                first_frame: i as u32,
                frame_count: 1,
            }),
        }
    }

    Ok(FskelFile {
        path: path.to_string(),
        version,
        checksum,
        bone_names: names,
        parents,
        frame_dt,
        actions,
        frames: all_frames,
    })
}

impl FskelFile {
    pub fn frame_count(&self) -> u32 {
        self.frames.len() as u32
    }

    pub fn bone_count(&self) -> u32 {
        self.bone_names.len() as u32
    }

    pub fn meta(&self) -> FskelMeta {
        FskelMeta {
            file_name: Path::new(&self.path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: self.path.clone(),
            version: self.version,
            checksum: self.checksum,
            frame_count: self.frame_count(),
            frame_dt: self.frame_dt,
            bone_count: self.bone_count(),
            bone_names: self.bone_names.clone(),
            parents: self.parents.clone(),
            actions: self.actions.clone(),
        }
    }

    /// [from, to) 帧的扁平姿态数组：帧 × 骨 × (tx,ty,tz,qx,qy,qz,qw)
    pub fn frames_flat(&self, from: u32, to: u32) -> Vec<f32> {
        let n = self.frames.len();
        let from = (from as usize).min(n);
        let to = (to as usize).min(n).max(from);
        let mut out = Vec::with_capacity((to - from) * self.bone_names.len() * 7);
        for f in &self.frames[from..to] {
            for p in f {
                out.extend_from_slice(&p.t);
                out.extend_from_slice(&p.q);
            }
        }
        out
    }

    /// 动作选择 → (起始帧, 结束帧, 动作名)；None = 全部
    pub fn action_range(&self, action: Option<usize>) -> (usize, usize, String) {
        match action.and_then(|i| self.actions.get(i)) {
            Some(a) => (
                a.first_frame as usize,
                (a.first_frame + a.frame_count) as usize,
                a.name.clone(),
            ),
            None => (0, self.frames.len(), String::new()),
        }
    }
}

// ---------------------------------------------------------------- 文件缓存

fn cache() -> &'static Mutex<HashMap<String, Arc<FskelFile>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<FskelFile>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析并缓存（最多保留 4 个文件），避免按路径取帧时重复整解析
pub fn get_or_load(path: &str) -> Result<Arc<FskelFile>, String> {
    if let Some(f) = cache().lock().unwrap().get(path) {
        return Ok(f.clone());
    }
    let bytes = fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    let file = Arc::new(parse_fskel(path, &bytes)?);
    let mut c = cache().lock().unwrap();
    if c.len() >= 4 {
        if let Some(k) = c.keys().next().cloned() {
            c.remove(&k);
        }
    }
    c.insert(path.to_string(), file.clone());
    Ok(file)
}

/// 目录内全部 .fskel 的轻量扫描
pub fn scan_dir(dir: &str) -> Result<Vec<FskelBrief>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))?;
    let mut out = Vec::new();
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        let is_fskel = p
            .extension()
            .map(|x| x.to_string_lossy().eq_ignore_ascii_case("fskel"))
            .unwrap_or(false);
        if !is_fskel {
            continue;
        }
        let Ok(bytes) = fs::read(&p) else { continue };
        let Ok((_v, _c, frame_count, hs)) = read_header(&bytes) else {
            continue;
        };
        let bone_count = u32_at(&bytes, hs).unwrap_or(0);
        let action_name = find_frames(&bytes, bone_count)
            .first()
            .map(|(_, n)| n.clone())
            .unwrap_or_default();
        out.push(FskelBrief {
            path: p.to_string_lossy().into_owned(),
            file_name: p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            frame_count,
            bone_count,
            byte_len: p.metadata().map(|m| m.len()).unwrap_or(0),
            action_name,
        });
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(out)
}

// ---------------------------------------------------------------- 四元数工具

fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-10 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

fn quat_to_matrix(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [x, y, z, w] = quat_normalize(q);
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

/// BVH 通道序 Zrotation Xrotation Yrotation：R = Rz(z)·Rx(x)·Ry(y)，返回 (z,x,y) 角度
fn quat_to_bvh_euler(q: [f32; 4]) -> (f32, f32, f32) {
    let m = quat_to_matrix(q);
    // Rz·Rx·Ry 各元素：m[2][1]=sin x，m[0][1]=-sin z·cos x，m[1][1]=cos z·cos x，
    //                   m[2][0]=-cos x·sin y，m[2][2]=cos x·cos y
    let sx = m[2][1].clamp(-1.0, 1.0);
    let x = sx.asin();
    let (y, z) = if sx.abs() < 0.999_99 {
        ((-m[2][0]).atan2(m[2][2]), (-m[0][1]).atan2(m[1][1]))
    } else {
        // 万向锁（x=±90°）：m00=cos(z∓y)，m02=±sin(z∓y)，取 y=0
        // x=+90°: z+y=atan2(m02,m00)；x=−90°: y−z=atan2(m02,m00) → z=−atan2
        let half = m[0][2].atan2(m[0][0]);
        if sx > 0.0 {
            (0.0, half)
        } else {
            (0.0, -half)
        }
    };
    (z.to_degrees(), x.to_degrees(), y.to_degrees())
}

// ---------------------------------------------------------------- BVH 导出

/// BVH 名字不允许空白
fn bvh_name(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join("_")
}

fn dist3(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// 导出 BVH（Blender 原生导入为骨架 + 动作）。
/// action = None 导出全部帧；否则导出指定动作段（以该段首帧为静止偏移）。
pub fn export_bvh(f: &FskelFile, action: Option<usize>, out_path: &str) -> Result<String, String> {
    let (from, to, name) = f.action_range(action);
    let bc = f.bone_count() as usize;
    let rest = &f.frames[from];

    let mut children: Vec<Vec<u32>> = vec![Vec::new(); bc];
    let mut roots = Vec::new();
    for (i, &p) in f.parents.iter().enumerate() {
        if p >= 0 {
            children[p as usize].push(i as u32);
        } else {
            roots.push(i as u32);
        }
    }
    if roots.len() != 1 {
        return Err(format!("期望单一根骨骼，实际 {}", roots.len()));
    }
    let root = roots[0];

    // 平移是否逐帧变化（>0.01 判定）：这类关节需要 6 通道（位置+旋转），
    // 其余骨骼 T 恒定，只出旋转通道（偏移静止于首帧）。
    let animated_t: Vec<bool> = (0..bc)
        .map(|b| {
            f.frames[from..to]
                .iter()
                .any(|fr| dist3(fr[b].t, rest[b].t) > 0.01)
        })
        .collect();

    // 先序遍历（与 MOTION 通道顺序一致）
    let mut order: Vec<u32> = Vec::with_capacity(bc);
    fn preorder(children: &Vec<Vec<u32>>, bone: u32, out: &mut Vec<u32>) {
        out.push(bone);
        for &c in &children[bone as usize] {
            preorder(children, c, out);
        }
    }
    preorder(&children, root, &mut order);

    // 层级输出：递归深度优先
    fn emit_tree(
        f: &FskelFile,
        children: &Vec<Vec<u32>>,
        root: u32,
        bone: u32,
        depth: usize,
        rest: &Vec<BonePose>,
        animated_t: &Vec<bool>,
        lines: &mut Vec<String>,
    ) {
        let pose = &rest[bone as usize];
        let ind = "  ".repeat(depth);
        let kind = if bone == root { "ROOT" } else { "JOINT" };
        lines.push(format!("{ind}{kind} {}", bvh_name(&f.bone_names[bone as usize])));
        lines.push(format!("{ind}{{"));
        if bone == root {
            lines.push(format!(
                "{ind}  OFFSET 0 0 0\n{ind}  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation"
            ));
        } else if animated_t[bone as usize] {
            // 平移有动画的关节：位置通道每帧携带绝对局部平移（替换 OFFSET）
            lines.push(format!(
                "{ind}  OFFSET {:.6} {:.6} {:.6}\n{ind}  CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation",
                pose.t[0], pose.t[1], pose.t[2]
            ));
        } else {
            lines.push(format!(
                "{ind}  OFFSET {:.6} {:.6} {:.6}\n{ind}  CHANNELS 3 Zrotation Xrotation Yrotation",
                pose.t[0], pose.t[1], pose.t[2]
            ));
        }
        if children[bone as usize].is_empty() {
            let len = (pose.t[0] * pose.t[0] + pose.t[1] * pose.t[1] + pose.t[2] * pose.t[2]).sqrt();
            let ext = if len > 1e-4 {
                [pose.t[0] / len * 2.0, pose.t[1] / len * 2.0, pose.t[2] / len * 2.0]
            } else {
                [2.0, 0.0, 0.0]
            };
            lines.push(format!("{ind}  End Site"));
            lines.push(format!("{ind}  {{"));
            lines.push(format!("{ind}    OFFSET {:.6} {:.6} {:.6}", ext[0], ext[1], ext[2]));
            lines.push(format!("{ind}  }}"));
        }
        for &c in &children[bone as usize] {
            emit_tree(f, children, root, c, depth + 1, rest, animated_t, lines);
        }
        lines.push(format!("{ind}}}"));
    }
    let mut lines: Vec<String> = Vec::new();
    emit_tree(f, &children, root, root, 0, rest, &animated_t, &mut lines);

    let n = to - from;
    let mut out = format!(
        "HIERARCHY\n{}\nMOTION\nFrames: {}\nFrame Time: {:.8}\n",
        lines.join("\n"),
        n,
        f.frame_dt
    );
    for fi in from..to {
        let frame = &f.frames[fi];
        let mut row: Vec<String> = Vec::with_capacity(bc * 3 + 3);
        for &b in &order {
            let p = &frame[b as usize];
            if b == root || animated_t[b as usize] {
                row.push(format!("{:.4}", p.t[0]));
                row.push(format!("{:.4}", p.t[1]));
                row.push(format!("{:.4}", p.t[2]));
            }
            let (z, x, y) = quat_to_bvh_euler(p.q);
            row.push(format!("{z:.4}"));
            row.push(format!("{x:.4}"));
            row.push(format!("{y:.4}"));
        }
        out.push_str(&row.join(" "));
        out.push('\n');
    }

    fs::write(out_path, out).map_err(|e| format!("写入失败: {e}"))?;
    let label = if name.is_empty() {
        format!("{n} 帧")
    } else {
        format!("{name} {n} 帧")
    };
    Ok(format!("已导出 BVH（{label}）: {out_path}"))
}

// ---------------------------------------------------------------- GLB 导出

/// 导出 glTF 2.0 二进制（.glb）。
/// 骨骼树 → 节点层级（以首帧为 TRS 静止姿势）；每个动作 → 一条命名动画
/// （每根骨骼 ROTATION 四元数 + TRANSLATION 通道，LINEAR 插值）。
/// 不含皮肤/网格 —— 面向 three.js / 引擎管线；Blender 骨架导入请用 BVH。
pub fn export_glb(f: &FskelFile, out_path: &str) -> Result<String, String> {
    let bc = f.bone_count() as usize;

    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<(usize, usize)> = Vec::new();
    let mut accessors: Vec<(usize, usize, &'static str)> = Vec::new(); // (view, count, type)

    fn add_f32s(
        bin: &mut Vec<u8>,
        views: &mut Vec<(usize, usize)>,
        accessors: &mut Vec<(usize, usize, &'static str)>,
        data: &[f32],
        ty: &'static str,
    ) -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let off = bin.len();
        let mut bytes = Vec::with_capacity(data.len() * 4);
        for v in data {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bin.extend_from_slice(&bytes);
        views.push((off, bytes.len()));
        // glTF accessor 的 count 是元素数（SCALAR=1 / VEC3=3 / VEC4=4 个 float）
        let comps = match ty {
            "VEC4" => 4,
            "VEC3" => 3,
            _ => 1,
        };
        accessors.push((views.len() - 1, data.len() / comps, ty));
        accessors.len() - 1
    }

    // 节点：骨骼 i -> glTF node i；TRS 取首帧
    let rest = &f.frames[0];
    let mut node_children: Vec<Vec<usize>> = vec![Vec::new(); bc];
    let mut scene_roots = Vec::new();
    for i in 0..bc {
        if f.parents[i] >= 0 {
            node_children[f.parents[i] as usize].push(i);
        } else {
            scene_roots.push(i);
        }
    }
    let nodes: Vec<String> = (0..bc)
        .map(|i| {
            let p = &rest[i];
            let q = quat_normalize(p.q);
            let children = if node_children[i].is_empty() {
                String::new()
            } else {
                let ids: Vec<String> = node_children[i].iter().map(|x| x.to_string()).collect();
                format!(",\"children\":[{}]", ids.join(","))
            };
            format!(
                "{{\"name\":{},\"translation\":[{:.6},{:.6},{:.6}],\"rotation\":[{:.6},{:.6},{:.6},{:.6}]{}}}",
                serde_json::to_string(&f.bone_names[i]).unwrap(),
                p.t[0], p.t[1], p.t[2],
                q[0], q[1], q[2], q[3], children
            )
        })
        .collect();

    // 每个动作一条动画（时间轴归零）
    let mut animations: Vec<String> = Vec::new();
    for a in &f.actions {
        let from = a.first_frame as usize;
        let to = from + a.frame_count as usize;
        let times: Vec<f32> = (0..to - from).map(|k| k as f32 * f.frame_dt).collect();
        let ti = add_f32s(&mut bin, &mut views, &mut accessors, &times, "SCALAR");
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for b in 0..bc {
            let rot: Vec<f32> = f.frames[from..to]
                .iter()
                .flat_map(|fr| quat_normalize(fr[b].q).into_iter())
                .collect();
            let ri = add_f32s(&mut bin, &mut views, &mut accessors, &rot, "VEC4");
            samplers.push(format!("{{\"input\":{ti},\"output\":{ri},\"interpolation\":\"LINEAR\"}}"));
            channels.push(format!(
                "{{\"sampler\":{},\"target\":{{\"node\":{b},\"path\":\"rotation\"}}}}",
                samplers.len() - 1
            ));
            let tr: Vec<f32> = f.frames[from..to]
                .iter()
                .flat_map(|fr| fr[b].t.into_iter())
                .collect();
            let tri = add_f32s(&mut bin, &mut views, &mut accessors, &tr, "VEC3");
            samplers.push(format!("{{\"input\":{ti},\"output\":{tri},\"interpolation\":\"LINEAR\"}}"));
            channels.push(format!(
                "{{\"sampler\":{},\"target\":{{\"node\":{b},\"path\":\"translation\"}}}}",
                samplers.len() - 1
            ));
        }
        animations.push(format!(
            "{{\"name\":{},\"channels\":[{}],\"samplers\":[{}]}}",
            serde_json::to_string(&a.name).unwrap(),
            channels.join(","),
            samplers.join(",")
        ));
    }

    let accessor_json: Vec<String> = accessors
        .iter()
        .map(|(vi, count, ty)| {
            format!("{{\"bufferView\":{vi},\"componentType\":5126,\"count\":{count},\"type\":\"{ty}\"}}")
        })
        .collect();
    let view_json: Vec<String> = views
        .iter()
        .map(|(off, len)| format!("{{\"buffer\":0,\"byteOffset\":{off},\"byteLength\":{len}}}"))
        .collect();

    let json = format!(
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"Gongfu Studio (Gongfu-Studio)\"}},\
\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\
\"nodes\":[{}],\
\"animations\":[{}],\
\"accessors\":[{}],\"bufferViews\":[{}],\
\"buffers\":[{{\"byteLength\":{}}}]}}",
        scene_roots
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(","),
        nodes.join(","),
        animations.join(","),
        accessor_json.join(","),
        view_json.join(","),
        bin.len()
    );

    // GLB 容器：12B 头 + JSON chunk + BIN chunk（各 4 字节对齐）
    let mut json_chunk = json.into_bytes();
    while json_chunk.len() % 4 != 0 {
        json_chunk.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json_chunk.len() + 8 + bin.len();
    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_chunk);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);

    fs::write(out_path, out).map_err(|e| format!("写入失败: {e}"))?;
    Ok(format!(
        "已导出 GLB（{} 个动作 · {bc} 骨骼 · {} 帧）: {out_path}",
        f.actions.len(),
        f.frame_count()
    ))
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    // 项目根目录 res/system 为 pack 解压后的实际目录
    const ACT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../res/system/AvatarGirl/act");

    #[test]
    fn parse_all_act_files() {
        let Ok(entries) = fs::read_dir(ACT_DIR) else {
            eprintln!("测试目录不存在，跳过: {ACT_DIR}");
            return;
        };
        let mut paths: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .map(|e| e.to_string_lossy().eq_ignore_ascii_case("fskel"))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        assert!(paths.len() > 200, "应至少有 200 个 FSKEL 文件");
        let mut total_frames = 0usize;
        for p in &paths {
            let bytes = fs::read(p).unwrap();
            let f = parse_fskel(&p.to_string_lossy(), &bytes)
                .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert_eq!(f.frames.len() as u32, f.frame_count());
            assert_eq!(f.parents[0], -1, "根骨骼父应为 -1");
            total_frames += f.frames.len();
        }
        eprintln!("共 {} 文件 {total_frames} 帧全部解析通过", paths.len());
    }

    #[test]
    fn euler_roundtrip() {
        // 任意四元数 -> BVH 欧拉 -> Rz·Rx·Ry 矩阵应与原四元数矩阵一致
        let cases: [[f32; 4]; 7] = [
            [0.0, 0.0, 0.0, 1.0],
            [0.1, 0.2, 0.3, 0.9],
            [0.7071, 0.0, 0.0, 0.7071],
            [0.5, 0.5, 0.5, 0.5],
            // 手指 Nub 的 180° 翻转（精确万向锁）与 +90° 情形
            [0.0, -1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.7071, 0.0, 0.0, 0.0],
        ];
        let mm = |a: [[f32; 3]; 3], b: [[f32; 3]; 3]| -> [[f32; 3]; 3] {
            let mut o = [[0f32; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    o[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
                }
            }
            o
        };
        for q in cases {
            let (zd, xd, yd) = quat_to_bvh_euler(q);
            let (z, x, y) = (zd.to_radians(), xd.to_radians(), yd.to_radians());
            let rz = [[z.cos(), -z.sin(), 0.0], [z.sin(), z.cos(), 0.0], [0.0, 0.0, 1.0]];
            let rx = [[1.0, 0.0, 0.0], [0.0, x.cos(), -x.sin()], [0.0, x.sin(), x.cos()]];
            let ry = [[y.cos(), 0.0, y.sin()], [0.0, 1.0, 0.0], [-y.sin(), 0.0, y.cos()]];
            let m = mm(mm(rz, rx), ry);
            let want = quat_to_matrix(q);
            for i in 0..3 {
                for j in 0..3 {
                    assert!(
                        (m[i][j] - want[i][j]).abs() < 1e-4,
                        "欧拉往返失败 {q:?} -> ({zd},{xd},{yd})"
                    );
                }
            }
        }
    }

    #[test]
    fn export_smoke() {
        for name in ["sta", "lrn1", "girl"] {
            let p = format!("{ACT_DIR}/{name}.FSKEL");
            let Ok(bytes) = fs::read(&p) else {
                eprintln!("{name}.FSKEL 不存在，跳过");
                continue;
            };
            let f = parse_fskel(&p, &bytes).unwrap();
            let tmp = std::env::temp_dir();
            let bvh = tmp.join(format!("{name}_test.bvh"));
            export_bvh(&f, None, bvh.to_str().unwrap()).unwrap();
            let content = fs::read_to_string(&bvh).unwrap();
            assert!(content.starts_with("HIERARCHY"));
            assert!(content.contains("MOTION"));
            assert!(content.contains("ROOT Bip0101"));
            assert!(content.contains(&format!("Frames: {}", f.frame_count())));
            let glb = tmp.join(format!("{name}_test.glb"));
            export_glb(&f, glb.to_str().unwrap()).unwrap();
            let b = fs::read(&glb).unwrap();
            assert_eq!(&b[0..4], b"glTF");
            assert!(b.len() > 1000);
        }
    }
}
