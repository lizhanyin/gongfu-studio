//! fmath：gongfu-studio 前端模型计算内核（viewer.ts 的 wasm 移植）。
//!
//! 数据流约定：
//! - 部件经 `register_part` 注册一次（SoA 影响表 + authored 数据），JS 用
//!   `pos_view/nrm_view` 拿 wasm 内存视图直连 three.js BufferAttribute（零拷贝）；
//! - 每帧只上传一次世界矩阵平表（`set_world_mats`），蒙皮/布料/去穿透结果
//!   直接写 wasm 内部缓冲，JS 侧仅标 needsUpdate；
//! - wasm 内存增长会使旧视图分离：凡 register/unregister 后 `views_version`
//!   自增，JS 需重取全部视图重绑 attribute。
//! - 矩阵列主序（对应 THREE.Matrix4.elements），敏感中间量 f64 与 TS 对齐。

use cloth::{ClothDiag, ClothSim, ClothState};
use depen::{DepenDiag, LegCapDef, PushRec, SkirtRig, TriRef};
use fak::FakState;
use mat::{local_from_pose, mat4_inverse, mat4_mul, Mat16, IDENTITY};
use skin::{lbs_residual, skin_part, BoneMats, PartSkin};
use wasm_bindgen::prelude::*;

mod cloth;
mod depen;
mod fak;
mod fit;
mod mat;
mod skin;

/// 锚定链：A_b = 载体当前世界 × Tp⁻¹ × M_bind_b（updateAnchoredMats）
struct AnchoredChain {
    carrier: u32,
    tp_inv: Mat16,
    binds: Vec<(u32, Mat16)>,
}

struct AnchoredState {
    chains: Vec<AnchoredChain>,
    mats: BoneMats,
    /// 增量锚定：帧 0 世界矩阵的逆（按骨索引；空 = 退回纯锚定）。
    /// A_b(t) = 载体(t)·Tp⁻¹·M_bind(b)·(W₀(b)⁻¹·W_t(b))——基准贴 authored
    /// 摆放、链自身的 FSKEL 动画以帧 0 骨姿态为原点叠加。配套链上
    /// M_bind ≈ W₀，公式退化为纯 LBS；脱节链（SZ_38 披发/ qmw 错位裙）
    /// 基准被拉回绑定摆放。
    w0_inv: Vec<Mat16>,
}

impl AnchoredState {
    fn new() -> Self {
        AnchoredState { chains: vec![], mats: vec![], w0_inv: vec![] }
    }

    fn update(&mut self, world: &[f64]) {
        if self.chains.is_empty() || world.is_empty() {
            return;
        }
        let incremental = !self.w0_inv.is_empty();
        for ch in &self.chains {
            let c = ch.carrier as usize;
            if world.len() < (c + 1) * 16 {
                continue;
            }
            let carrier: &[f64; 16] = world[c * 16..c * 16 + 16].try_into().unwrap();
            let carry = mat4_mul(carrier, &ch.tp_inv);
            for (b, bind) in &ch.binds {
                let bu = *b as usize;
                let base = mat4_mul(&carry, bind);
                let m = if incremental
                    && bu < self.w0_inv.len()
                    && world.len() >= (bu + 1) * 16
                {
                    let wt: &[f64; 16] = world[bu * 16..bu * 16 + 16].try_into().unwrap();
                    let delta = mat4_mul(&self.w0_inv[bu], wt);
                    mat4_mul(&base, &delta)
                } else {
                    base
                };
                if bu < self.mats.len() {
                    self.mats[bu] = Some(m);
                }
            }
        }
    }
}

/// 姿态数据（setFskel 时上传一次）
struct PoseData {
    poses: Vec<f32>,
    bone_count: usize,
    parents: Vec<i32>,
    /// 镜像骨集合（已排序，供二分）
    mirror: Vec<u32>,
    /// worlds_from_poses / lbs_residual 的输出暂存
    worlds_scratch: Vec<f64>,
}

impl PoseData {
    fn compose_worlds(&mut self, frame: i32, mirror_override: MirrorOverride) -> &[f64] {
        let n = self.bone_count;
        if n == 0 {
            return &[];
        }
        let stride = n * 7;
        let last = (self.poses.len() / stride).saturating_sub(1);
        let base = (frame.clamp(0, last as i32) as usize) * stride;
        self.worlds_scratch.clear();
        self.worlds_scratch.resize(n * 16, 0.0);
        for b in 0..n {
            let o = base + b * 7;
            let mir = match mirror_override {
                MirrorOverride::Stored => self.mirror.binary_search(&(b as u32)).is_ok(),
                MirrorOverride::None => false,
                MirrorOverride::Single(root) => root == b as u32,
            };
            let m = local_from_pose(&self.poses[o..o + 7], mir);
            // 父矩阵左乘（TS: out[b].premultiply(out[p])，父序号必小于自身）
            let p = self.parents[b];
            let m = if p >= 0 && (p as usize) < b {
                let pm: &[f64; 16] = self.worlds_scratch[p as usize * 16..p as usize * 16 + 16]
                    .try_into()
                    .unwrap();
                mat4_mul(pm, &m)
            } else {
                m
            };
            self.worlds_scratch[b * 16..b * 16 + 16].copy_from_slice(&m);
        }
        &self.worlds_scratch
    }
}

enum MirrorOverride {
    Stored,
    None,
    Single(u32),
}

/// 诊断计数（wasm 侧累计，TS 的 diagFlush 拉取后 reset）
#[derive(Default, Clone)]
struct Diag {
    depen: DepenDiag,
    cloth: ClothDiag,
}

#[wasm_bindgen]
pub struct Gfm {
    parts: Vec<Option<PartSkin>>,
    views_version: u32,
    world: Vec<f64>,
    bone_count: usize,
    world_set: bool,
    poses: Option<PoseData>,
    anchored: AnchoredState,
    cloth: ClothState,
    /// 腿胶囊定义（setupSkirtRig 上传，cloth/depen 共用）
    caps: Vec<LegCapDef>,
    skirt: SkirtRig,
    skirt_record: bool,
    push_log: Vec<PushRec>,
    diag: Diag,
    fak: Option<FakState>,
}

#[wasm_bindgen]
impl Gfm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Gfm {
        Gfm {
            parts: vec![],
            views_version: 0,
            world: vec![],
            bone_count: 0,
            world_set: false,
            poses: None,
            anchored: AnchoredState::new(),
            cloth: ClothState::new(0),
            caps: vec![],
            skirt: SkirtRig::empty(),
            skirt_record: false,
            push_log: vec![],
            diag: Diag::default(),
            fak: None,
        }
    }

    // ---------------- 部件注册 ----------------

    /// 注册部件（返回槽位号 = 后续 API 的 id）。src_* 含跨缝副本（按几何顶点序，
    /// resolved_src 指回源顶点）；infl_off 为 CSR 偏移。分配可能使旧视图分离，
    /// 调用后按 views_version 重绑。
    pub fn register_part(
        &mut self,
        src_pos: &[f32],
        src_nrm: &[f32],
        resolved_src: &[u32],
        orig_count: u32,
        infl_off: &[u32],
        infl_bone: &[u32],
        infl_weight: &[f32],
        infl_local: &[f32],
    ) -> u32 {
        let vc = infl_off.len().saturating_sub(1);
        let part = PartSkin {
            orig_count: (orig_count as usize).min(vc),
            vert_count: vc,
            resolved_src: resolved_src.to_vec(),
            src_pos: src_pos.to_vec(),
            src_nrm: src_nrm.to_vec(),
            infl_off: infl_off.to_vec(),
            infl_bone: infl_bone.to_vec(),
            infl_weight: infl_weight.to_vec(),
            infl_local: infl_local.to_vec(),
            pos: src_pos.to_vec(),
            nrm: src_nrm.to_vec(),
        };
        self.parts.push(Some(part));
        self.views_version = self.views_version.wrapping_add(1);
        (self.parts.len() - 1) as u32
    }

    pub fn unregister_part(&mut self, id: u32) {
        let i = id as usize;
        if i < self.parts.len() {
            self.parts[i] = None;
        }
        self.views_version = self.views_version.wrapping_add(1);
    }

    pub fn views_version(&self) -> u32 {
        self.views_version
    }

    pub fn pos_view(&self, id: u32) -> Option<js_sys::Float32Array> {
        self.parts.get(id as usize)?.as_ref().map(|p| unsafe {
            // 安全性：视图指向 wasm 内存，仅在 views_version 不变期间有效
            js_sys::Float32Array::view(&p.pos)
        })
    }

    pub fn nrm_view(&self, id: u32) -> Option<js_sys::Float32Array> {
        self.parts.get(id as usize)?.as_ref().map(|p| unsafe {
            js_sys::Float32Array::view(&p.nrm)
        })
    }

    /// 顶点编辑后同步源数据（含引用该源顶点的跨缝副本）
    pub fn set_src_vert(&mut self, id: u32, src_index: u32, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) {
        let Some(part) = self.parts.get_mut(id as usize).and_then(|p| p.as_mut()) else {
            return;
        };
        for j in 0..part.vert_count {
            if part.resolved_src[j] == src_index {
                let o = j * 3;
                part.src_pos[o] = px;
                part.src_pos[o + 1] = py;
                part.src_pos[o + 2] = pz;
                part.src_nrm[o] = nx;
                part.src_nrm[o + 1] = ny;
                part.src_nrm[o + 2] = nz;
                part.pos[o] = px;
                part.pos[o + 1] = py;
                part.pos[o + 2] = pz;
                part.nrm[o] = nx;
                part.nrm[o + 1] = ny;
                part.nrm[o + 2] = nz;
            }
        }
    }

    // ---------------- 每帧路径 ----------------

    /// 上传当前帧世界矩阵平表（列主序 n×16；bone_count 为 TS 的
    /// fskelBoneCount || worldMats.length 口径）
    pub fn set_world_mats(&mut self, mats: &[f64], bone_count: u32) {
        self.world = mats.to_vec();
        self.bone_count = bone_count as usize;
        self.world_set = true;
    }

    /// 刷新锚定合成矩阵（updateAnchoredMats）
    pub fn update_anchored(&mut self) {
        self.anchored.update(&self.world);
    }

    /// 布料模拟步进 + 合成矩阵刷新（updateClothMats）
    pub fn update_cloth(&mut self, abs: f64) {
        let mut diag = std::mem::take(&mut self.diag.cloth);
        let Gfm { cloth, world, caps, .. } = self;
        cloth.update(abs, world, caps, &mut diag);
        self.diag.cloth = diag;
    }

    /// CPU LBS 蒙皮单部件（skinPart）；输出直写 wasm pos/nrm 缓冲
    pub fn skin_part(&mut self, id: u32) {
        if !self.world_set {
            return;
        }
        let Gfm { parts, world, bone_count, cloth, anchored, .. } = self;
        let Some(p) = parts.get_mut(id as usize).and_then(|p| p.as_mut()) else {
            return;
        };
        skin_part(p, *bone_count, world, &cloth.mats, &anchored.mats);
    }

    /// 裙摆去穿透（applySkirtDePenVerts）；返回被改动 position 的部件 id 列表
    pub fn skirt_depen(&mut self, abs: f64) -> Box<[u32]> {
        let mut diag = std::mem::take(&mut self.diag.depen);
        let Gfm { parts, skirt, world, push_log, skirt_record, .. } = self;
        let touched = depen::skirt_depen(
            parts,
            skirt,
            world,
            &mut diag,
            *skirt_record,
            abs,
            push_log,
        );
        self.diag.depen = diag;
        touched.into_iter().map(|i| i as u32).collect()
    }

    // ---------------- 布料/锚定/裙 rig setup ----------------

    /// 布料模拟初始化（detectCloth 的拟合结果上传）：
    /// sims 每项 8×f32 = bone, anchor3, target3
    pub fn setup_cloth(&mut self, carrier: u32, bone_count: u32, tp_inv: &[f64], sims: &[f32]) {
        let sim_count = sims.len() / 8;
        let mut list = Vec::with_capacity(sim_count);
        for i in 0..sim_count {
            let b = sims[i * 8] as u32;
            list.push(ClothSim {
                bone: b,
                anchor: [
                    sims[i * 8 + 1] as f64,
                    sims[i * 8 + 2] as f64,
                    sims[i * 8 + 3] as f64,
                ],
                target: [
                    sims[i * 8 + 4] as f64,
                    sims[i * 8 + 5] as f64,
                    sims[i * 8 + 6] as f64,
                ],
                pos: [f64::NAN; 3],
                prev: [f64::NAN; 3],
            });
        }
        let mut tp = IDENTITY;
        if tp_inv.len() == 16 {
            tp.copy_from_slice(tp_inv);
        }
        let mut st = ClothState::new(bone_count as usize);
        st.active = true;
        st.carrier = carrier;
        st.tp_inv = tp;
        st.sims = list;
        st.last_abs = f64::NAN;
        self.cloth = st;
    }

    pub fn clear_cloth(&mut self) {
        self.cloth = ClothState::new(0);
    }

    /// 锚定链数据上传（fitAnchoredChains 的拟合结果）：
    /// carriers/tp_invs 按链序；binds 为全部链骨绑定拟合串联（bone, 16×mat），
    /// bind_counts[i] 为第 i 条链的骨数
    pub fn setup_anchored(
        &mut self,
        bone_count: u32,
        carriers: &[u32],
        tp_invs: &[f64],
        bind_counts: &[u32],
        bind_bones: &[u32],
        bind_mats: &[f64],
    ) {
        let mut chains = vec![];
        let mut bind_off = 0usize;
        let mut mat_off = 0usize;
        for (ci, carrier) in carriers.iter().enumerate() {
            let mut tp = IDENTITY;
            if tp_invs.len() >= (ci + 1) * 16 {
                tp.copy_from_slice(&tp_invs[ci * 16..ci * 16 + 16]);
            }
            let count = *bind_counts.get(ci).unwrap_or(&0) as usize;
            let mut binds = vec![];
            for _ in 0..count {
                let bone = *bind_bones.get(bind_off).unwrap_or(&0);
                let mut m = IDENTITY;
                if bind_mats.len() >= (mat_off + 1) * 16 {
                    m.copy_from_slice(&bind_mats[mat_off * 16..mat_off * 16 + 16]);
                }
                binds.push((bone, m));
                bind_off += 1;
                mat_off += 1;
            }
            chains.push(AnchoredChain { carrier: *carrier, tp_inv: tp, binds });
        }
        // 增量锚定基準：帧 0 世界矩阵（poses 已上传时计算；否则退回纯锚定）
        let w0_inv = if let Some(p) = self.poses.as_mut() {
            let w0 = p.compose_worlds(0, MirrorOverride::Stored).to_vec();
            (0..bone_count as usize)
                .map(|b| {
                    if w0.len() >= (b + 1) * 16 {
                        let m: &[f64; 16] = w0[b * 16..b * 16 + 16].try_into().unwrap();
                        mat4_inverse(m).unwrap_or(IDENTITY)
                    } else {
                        IDENTITY
                    }
                })
                .collect()
        } else {
            vec![]
        };
        self.anchored = AnchoredState {
            chains,
            mats: vec![None; bone_count as usize],
            w0_inv,
        };
    }

    pub fn clear_anchored(&mut self) {
        self.anchored = AnchoredState::new();
    }

    /// 裙摆防穿模数据上传（setupSkirtRig 的收集结果）。
    /// verts：偶数位 part id、奇数位顶点下标；tris/leg_tris/body_tris 每 4 项
    /// = part, a, b, c；adj 为 CSR；caps 偶数位 bone、奇数位 len。
    #[allow(clippy::too_many_arguments)]
    pub fn setup_skirt(
        &mut self,
        verts: &[u32],
        tris: &[u32],
        leg_tris: &[u32],
        body_tris: &[u32],
        adj_off: &[u32],
        adj: &[u32],
        caps: &[f32],
    ) {
        let mut rig = SkirtRig::empty();
        for v in verts.chunks(2) {
            let part = v[0] as usize;
            let idx = v[1];
            let s = rig.verts_part.len() as u32;
            rig.verts_part.push(part);
            rig.verts_idx.push(idx);
            rig.sv_index.insert((part, idx), s);
        }
        let read_tris = |src: &[u32], out: &mut Vec<TriRef>| {
            for t in src.chunks(4) {
                if t.len() == 4 {
                    out.push(TriRef { part: t[0] as usize, a: t[1], b: t[2], c: t[3] });
                }
            }
        };
        read_tris(tris, &mut rig.tris);
        read_tris(leg_tris, &mut rig.leg_tris);
        read_tris(body_tris, &mut rig.body_tris);
        rig.adj_off = adj_off.to_vec();
        rig.adj = adj.to_vec();
        rig.caps = caps
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| LegCapDef { bone: c[0] as u32, len: c[1] })
            .collect();
        let n3 = rig.verts_part.len() * 3;
        rig.prev_disp = vec![0.0; n3];
        rig.orig_snap = vec![0.0; n3];
        rig.prev_abs = f64::NAN;
        rig.relax_a = vec![0.0; n3];
        rig.relax_b = vec![0.0; n3];
        self.skirt = rig;
        self.caps = self.skirt.caps.clone();
    }

    pub fn clear_skirt(&mut self) {
        self.skirt = SkirtRig::empty();
        self.caps.clear();
    }

    pub fn set_skirt_record(&mut self, on: bool) {
        self.skirt_record = on;
    }

    /// 采集日志（set_skirt_record(true) 时 depen 每帧写入）：
    /// 每记录 4×f32 = v, phase, k, bone（bone -1 = 无）
    pub fn skirt_push_log(&self) -> Box<[f32]> {
        let mut out = Vec::with_capacity(self.push_log.len() * 4);
        for r in &self.push_log {
            out.push(r.v as f32);
            out.push(r.phase as f32);
            out.push(r.k as f32);
            out.push(r.bone as f32);
        }
        out.into()
    }

    // ---------------- 姿态 / 检测 ----------------

    /// 上传全部帧姿态（setFskel）；poses 为帧×骨×7 扁平数组
    pub fn set_poses(&mut self, poses: &[f32], bone_count: u32, parents: &[i32]) {
        self.poses = Some(PoseData {
            poses: poses.to_vec(),
            bone_count: bone_count as usize,
            parents: parents.to_vec(),
            mirror: vec![],
            worlds_scratch: vec![0.0; bone_count as usize * 16],
        });
    }

    pub fn clear_poses(&mut self) {
        self.poses = None;
    }

    /// 镜像骨集合（detectMirrorBones 判定后同步）
    pub fn set_mirror(&mut self, bones: &[u32]) {
        if let Some(p) = self.poses.as_mut() {
            p.mirror = bones.to_vec();
            p.mirror.sort_unstable();
        }
    }

    /// 从姿态组合某帧各骨世界矩阵（worldsFromPoses，用已存镜像集合）。
    /// 返回 wasm 内存视图（下次调用前有效）。
    pub fn worlds_from_poses(&mut self, frame: i32) -> Option<js_sys::Float64Array> {
        let p = self.poses.as_mut()?;
        let s = p.compose_worlds(frame, MirrorOverride::Stored);
        Some(unsafe { js_sys::Float64Array::view(s) })
    }

    /// 镜像骨检测的残差（frame 帧、给定镜像模式）：链上顶点 LBS 残差和。
    /// mirror_mode：0 = 已存镜像集合，1 = 空集（不镜像），2 = 单骨 mirror_root。
    pub fn lbs_residual(
        &mut self,
        frame: i32,
        mirror_mode: u32,
        mirror_root: u32,
        bones: Option<Vec<u32>>,
    ) -> f64 {
        let Some(p) = self.poses.as_mut() else { return 0.0 };
        let world = p.compose_worlds(
            frame,
            match mirror_mode {
                1 => MirrorOverride::None,
                2 => MirrorOverride::Single(mirror_root),
                _ => MirrorOverride::Stored,
            },
        )
        .to_vec();
        let n = p.bone_count;
        let mut sorted = bones.unwrap_or_default();
        sorted.sort_unstable();
        let live: Vec<&PartSkin> = self.parts.iter().flatten().collect();
        lbs_residual(&live, &world, n, &sorted)
    }

    // ---------------- 绑定拟合 ----------------

    /// 点对 (局部, 世界) 刚体拟合（fitBindMatrix），输入扁平 xyz；
    /// 返回列主序 16×f64（TS 侧 Matrix4.fromArray）
    pub fn fit_bind_matrix(&self, local: &[f32], world: &[f32]) -> Box<[f64]> {
        fit::fit_bind_matrix(local, world)
            .into_iter()
            .collect()
    }

    // ---------------- FAK 顶点动画 ----------------

    /// 上传全部帧（frames = 帧拼接，每帧 vc×8）
    pub fn set_fak(&mut self, frames: &[f32], frame_count: u32, vertex_count: u32) {
        self.fak = Some(FakState::new(frames.to_vec(), frame_count as usize, vertex_count as usize));
    }

    pub fn clear_fak(&mut self) {
        self.fak = None;
    }

    pub fn fak_frame(&mut self, i0: i32, i1: i32, alpha: f64) {
        if let Some(f) = self.fak.as_mut() {
            f.frame(i0, i1, alpha);
        }
    }

    pub fn fak_pos_view(&self) -> Option<js_sys::Float32Array> {
        self.fak.as_ref().map(|f| unsafe { js_sys::Float32Array::view(&f.pos) })
    }

    pub fn fak_nrm_view(&self) -> Option<js_sys::Float32Array> {
        self.fak.as_ref().map(|f| unsafe { js_sys::Float32Array::view(&f.nrm) })
    }

    // ---------------- 诊断计数 ----------------

    pub fn diag_max_need(&self) -> f64 {
        self.diag.depen.max_need
    }

    pub fn diag_max_bone(&self) -> i32 {
        self.diag.depen.max_bone
    }

    pub fn diag_clamp(&self) -> u32 {
        self.diag.depen.clamp
    }

    pub fn diag_cloth_seek(&self) -> u32 {
        self.diag.cloth.seek
    }

    pub fn diag_cloth_vel_clamp(&self) -> u32 {
        self.diag.cloth.vel_clamp
    }

    pub fn diag_cloth_safe_clamp(&self) -> u32 {
        self.diag.cloth.safe_clamp
    }

    pub fn diag_cloth_lag_clamp(&self) -> u32 {
        self.diag.cloth.lag_clamp
    }

    /// 诊断窗口输出后清零（diagFlush/diagClear 调用）
    pub fn diag_reset(&mut self) {
        self.diag = Diag::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose_data() -> PoseData {
        // 2 骨：骨 0 单位；骨 1 父 = 0，局部平移 (10,0,0)
        let poses = vec![
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, //
            10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        PoseData {
            poses,
            bone_count: 2,
            parents: vec![-1, 0],
            mirror: vec![],
            worlds_scratch: vec![0.0; 32],
        }
    }

    #[test]
    fn compose_worlds_parent_chain() {
        let mut p = pose_data();
        let w = p.compose_worlds(0, MirrorOverride::Stored).to_vec();
        // 骨 1 世界平移 = (10,0,0)
        assert!((w[16 + 12] - 10.0).abs() < 1e-12);
        assert!((w[16] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn compose_worlds_mirror_propagates() {
        let mut p = pose_data();
        p.mirror = vec![0]; // 骨 0 镜像
        let w = p.compose_worlds(0, MirrorOverride::Stored).to_vec();
        // 镜像骨 0：旋转块取反；子骨 1 继承（x 平移翻转到 -10）
        assert!((w[0] - -1.0).abs() < 1e-12);
        assert!((w[16 + 12] - -10.0).abs() < 1e-12, "w={:?}", &w[16..32]);
    }

    #[test]
    fn compose_worlds_mirror_single_override() {
        let mut p = pose_data();
        let w = p.compose_worlds(0, MirrorOverride::Single(1)).to_vec();
        // 仅骨 1 镜像：自身旋转取反，父单位 → 平移仍是 (10,0,0)？不——
        // 镜像乘在局部 T·R 之后：M = T·R·(-I)，作用到点 = t - p，
        // 父左乘后平移仍 (10,0,0)，旋转块为 -1
        assert!((w[16 + 12] - 10.0).abs() < 1e-12);
        assert!((w[16] - -1.0).abs() < 1e-12);
    }
}
