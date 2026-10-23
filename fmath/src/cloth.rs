//! 布料次级运动模拟（viewer.ts updateClothMats）：每根布料骨一个 verlet 点，
//! 弹簧-阻尼跟随携带目标（骨盆绑定系 → 当前世界）+ 微重力 + 腿胶囊推出 +
//! 平移跟随合成矩阵 [T(offset)]·P(f)·Tp⁻¹。

use crate::depen::LegCapDef;
use crate::mat::{mat4_mul, Mat16};
use crate::skin::BoneMats;

// 模拟参数（与 TS 常量一致）：弹簧 K（s⁻²）、各向异性阻尼、重力（u/s²，身高 ~160）
const CLOTH_K: f64 = 260.0;
const CLOTH_DAMP: f64 = 0.9;
const CLOTH_DAMP_UP: f64 = 0.78;
const CLOTH_DAMP_DOWN: f64 = 0.92;
const CLOTH_G: f64 = 300.0;
/// 腿部胶囊半径系数（×段长）
pub const CLOTH_LEG_R: f64 = 0.18;

#[derive(Clone, Copy)]
pub struct ClothSim {
    pub bone: u32,
    /// 锚点/弹簧目标（载体绑定系）：顶点簇上端质心 / 全簇质心
    pub anchor: [f64; 3],
    pub target: [f64; 3],
    /// 模拟点（世界坐标，NaN = 待重置）
    pub pos: [f64; 3],
    pub prev: [f64; 3],
}

pub struct ClothState {
    pub active: bool,
    pub carrier: u32,
    pub tp_inv: Mat16,
    pub sims: Vec<ClothSim>,
    /// 每帧布料合成矩阵（按骨索引；仅布料骨有值）
    pub mats: BoneMats,
    pub last_abs: f64,
}

#[derive(Default, Clone)]
pub struct ClothDiag {
    pub seek: u32,
    pub vel_clamp: u32,
    pub safe_clamp: u32,
    pub lag_clamp: u32,
}

impl ClothState {
    pub fn new(bone_count: usize) -> Self {
        ClothState {
            active: false,
            carrier: 0,
            tp_inv: crate::mat::IDENTITY,
            sims: vec![],
            mats: vec![None; bone_count],
            last_abs: f64::NAN,
        }
    }

    /// 每帧步进 + 合成矩阵刷新（对应 updateClothMats；world 为当前帧世界矩阵平表）
    pub fn update(
        &mut self,
        abs: f64,
        world: &[f64],
        caps: &[LegCapDef],
        diag: &mut ClothDiag,
    ) {
        if !self.active || self.sims.is_empty() {
            return;
        }
        let carrier_mat: Mat16 = match world_mat(world, self.carrier as usize) {
            Some(m) => *m,
            None => return,
        };
        // 携带矩阵：骨盆绑定系 → 当前世界
        let carry = mat4_mul(&carrier_mat, &self.tp_inv);

        let mut dt = 0.0f64;
        let mut seek = false;
        if !abs.is_finite() || !self.last_abs.is_finite() {
            seek = true;
        } else {
            let d = abs - self.last_abs;
            if d < -0.01 || d > 3.0 {
                seek = true; // 倒放/大跳 = seek
            } else {
                dt = d.clamp(0.0, 2.0) / 30.0;
            }
        }
        self.last_abs = abs;
        if seek {
            diag.seek += 1;
        }

        // 胶囊世界坐标（一帧内恒定，先算好；退化段跳过）
        let live_caps: Vec<([f64; 3], [f64; 3], f64)> = caps
            .iter()
            .filter_map(|c| {
                let e = world_mat(world, c.bone as usize)?;
                let abx = e[0] * c.len as f64;
                let aby = e[1] * c.len as f64;
                let abz = e[2] * c.len as f64;
                if abx * abx + aby * aby + abz * abz < 1e-9 {
                    return None;
                }
                Some((
                    [e[12], e[13], e[14]],
                    [e[12] + abx, e[13] + aby, e[14] + abz],
                    c.len as f64 * CLOTH_LEG_R,
                ))
            })
            .collect();

        for s in &mut self.sims {
            let apply = |v: [f64; 3], m: &Mat16| {
                [
                    m[0] * v[0] + m[4] * v[1] + m[8] * v[2] + m[12],
                    m[1] * v[0] + m[5] * v[1] + m[9] * v[2] + m[13],
                    m[2] * v[0] + m[6] * v[1] + m[10] * v[2] + m[14],
                ]
            };
            let anchor_w = apply(s.anchor, &carry);
            let target_w = apply(s.target, &carry);
            if seek || !s.pos[0].is_finite() {
                s.pos = target_w;
                s.prev = target_w;
            } else if dt > 0.0 {
                // verlet：速度阻尼（y 上扬加重抑制裙摆翻上腰，下落轻放保垂感）
                // + 微重力 + 弹簧拉向携带目标（速度限幅防炸）
                let mut vel = [s.pos[0] - s.prev[0], s.pos[1] - s.prev[1], s.pos[2] - s.prev[2]];
                vel[0] *= CLOTH_DAMP;
                vel[2] *= CLOTH_DAMP;
                vel[1] *= if vel[1] > 0.0 { CLOTH_DAMP_UP } else { CLOTH_DAMP_DOWN };
                let vmax = 6.0 * dt * 30.0; // ≤6 单位/帧
                let vl = norm3(vel);
                if vl > vmax {
                    let sc = vmax / vl;
                    vel = [vel[0] * sc, vel[1] * sc, vel[2] * sc];
                    diag.vel_clamp += 1;
                }
                s.prev = s.pos;
                s.pos = [
                    s.pos[0] + vel[0],
                    s.pos[1] + vel[1],
                    s.pos[2] + vel[2],
                ];
                s.pos[1] -= CLOTH_G * dt * dt;
                let r = (CLOTH_K * dt * dt).min(1.0);
                s.pos = [
                    s.pos[0] + (target_w[0] - s.pos[0]) * r,
                    s.pos[1] + (target_w[1] - s.pos[1]) * r,
                    s.pos[2] + (target_w[2] - s.pos[2]) * r,
                ];
            }
            // 腿部胶囊推出（两轮；胶囊沿骨段全长，膝/踝关节处无球间空洞）
            for _ in 0..2 {
                for (a, b, r) in &live_caps {
                    let t = closest_on_seg(s.pos, *a, *b);
                    let dx = s.pos[0] - t[0];
                    let dy = s.pos[1] - t[1];
                    let dz = s.pos[2] - t[2];
                    let d2 = dx * dx + dy * dy + dz * dz;
                    if d2 < r * r && d2 > 1e-8 {
                        let d = d2.sqrt();
                        s.pos = [
                            t[0] + (dx / d) * r,
                            t[1] + (dy / d) * r,
                            t[2] + (dz / d) * r,
                        ];
                    }
                }
            }
            // 安全钳位：模拟点离锚点不超过静止臂长的 2 倍（防碰撞/大步长累积飞出）
            let dir1 = [s.pos[0] - anchor_w[0], s.pos[1] - anchor_w[1], s.pos[2] - anchor_w[2]];
            let arm = dist3(s.target, s.anchor);
            let dl = norm3(dir1);
            if arm > 1e-4 && dl > arm * 2.0 {
                let sc = arm * 2.0 / dl;
                s.pos = [
                    anchor_w[0] + dir1[0] * sc,
                    anchor_w[1] + dir1[1] * sc,
                    anchor_w[2] + dir1[2] * sc,
                ];
                diag.safe_clamp += 1;
            }
            // 应用：平移跟随。不用旋转——簇臂长可能只有 1~3 单位，小滞后会被
            // 放大成接近 180° 的翻转（面板甩飞）；有界平移同样有滞后摆动感且绝无翻转
            let mut off = [s.pos[0] - target_w[0], s.pos[1] - target_w[1], s.pos[2] - target_w[2]];
            let ol = norm3(off);
            if ol > 8.0 {
                let sc = 8.0 / ol; // 裙摆滞后上限
                off = [off[0] * sc, off[1] * sc, off[2] * sc];
                diag.lag_clamp += 1;
            }
            let mut t_m = crate::mat::IDENTITY;
            t_m[12] = off[0];
            t_m[13] = off[1];
            t_m[14] = off[2];
            let out = mat4_mul(&t_m, &carry);
            if (s.bone as usize) < self.mats.len() {
                self.mats[s.bone as usize] = Some(out);
            }
        }
    }
}

fn world_mat<'a>(world: &'a [f64], bi: usize) -> Option<&'a Mat16> {
    if world.len() < (bi + 1) * 16 {
        return None;
    }
    world[bi * 16..bi * 16 + 16].try_into().ok()
}

fn norm3(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    norm3([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn closest_on_seg(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let ab2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let mut t = if ab2 > 1e-9 {
        (ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / ab2
    } else {
        0.0
    };
    t = t.clamp(0.0, 1.0);
    [a[0] + ab[0] * t, a[1] + ab[1] * t, a[2] + ab[2] * t]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_world(n: usize) -> Vec<f64> {
        let mut v = vec![0.0; n * 16];
        for b in 0..n {
            v[b * 16] = 1.0;
            v[b * 16 + 5] = 1.0;
            v[b * 16 + 10] = 1.0;
            v[b * 16 + 15] = 1.0;
        }
        v
    }

    #[test]
    fn seek_resets_to_target() {
        let mut st = ClothState::new(4);
        st.active = true;
        st.carrier = 1;
        st.sims = vec![ClothSim {
            bone: 2,
            anchor: [0.0, 10.0, 0.0],
            target: [0.0, 5.0, 0.0],
            pos: [f64::NAN; 3],
            prev: [f64::NAN; 3],
        }];
        let world = identity_world(4);
        let mut diag = ClothDiag::default();
        st.update(f64::NAN, &world, &[], &mut diag);
        assert!(st.sims[0].pos == [0.0, 5.0, 0.0]);
        assert_eq!(diag.seek, 1);
        // 合成矩阵 = carry（无偏移）
        let m = st.mats[2].unwrap();
        assert!((m[13] - 0.0).abs() < 1e-12);
    }

    #[test]
    fn capsule_push_out() {
        // 模拟点在腿胶囊内：胶囊沿骨骼 +X（a=(0,0,0) b=(len,0,0)），
        // 点在轴旁 0.5 → 推出到胶囊面（半径 = len*0.18）
        let len = 100.0f32;
        let mut st = ClothState::new(4);
        st.active = true;
        st.carrier = 1;
        st.sims = vec![ClothSim {
            bone: 2,
            anchor: [1.0, 0.0, 0.0],
            target: [1.0, 0.0, 0.0],
            pos: [1.0, 0.0, 0.5],
            prev: [1.0, 0.0, 0.5],
        }];
        st.last_abs = 1.0; // dt = 0：跳过 verlet，只测胶囊推出
        let world = identity_world(4);
        let caps = [LegCapDef { bone: 0, len }];
        let mut diag = ClothDiag::default();
        st.update(1.0, &world, &caps, &mut diag);
        let d = st.sims[0].pos[2].abs();
        assert!((d - len as f64 * CLOTH_LEG_R).abs() < 1e-9, "d={d}");
        assert_eq!(diag.lag_clamp, 1); // 偏移 18 > 8 触发滞后钳位
    }
}
