//! 裙摆去穿透（viewer.ts applySkirtDePenVerts 六阶段管线，恒开，纯位置修正）：
//! ①顶点逐胶囊依次推出 ②面片跨穿 tri-tri 整面外推 ②.5 位移场空间松弛
//! ③复扫顶点 ④推出量时间平滑（入快出慢 + 安全网） ④.5 贴腿顶点位移衰减
//! ⑤动态接触跟随（位移传给躯干三角形）。
//! 几何工具 triTri/ptTriDist/ptTriBary/inTri 为 Ericson/Möller–Trumbore 式标量实现，
//! 与 TS 版逐分支对照（含早退与共面跳过）。

use crate::skin::PartSkin;
use std::collections::HashMap;

const SKIRT_MARGIN: f64 = 0.25;
const SKIRT_MESH_MARGIN: f64 = 0.15;
const SKIRT_MAX_PUSH: f64 = 6.0;
const SKIRT_SMOOTH_UP: f64 = 0.75;
const SKIRT_SMOOTH_DOWN: f64 = 0.3;
const SKIRT_RELAX_R: f64 = 0.5;
const SKIRT_FOLLOW: f64 = 0.75;
const SKIRT_WELD_D: f64 = 0.6;
const SKIRT_WELD_SEP: f64 = 0.25;
const CLOTH_LEG_R: f64 = 0.18;

pub struct TriRef {
    pub part: usize,
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

#[derive(Clone)]
pub struct LegCapDef {
    pub bone: u32,
    pub len: f32,
}

pub struct SkirtRig {
    /// 裙片顶点（part 槽位, 几何顶点下标）
    pub verts_part: Vec<usize>,
    pub verts_idx: Vec<u32>,
    /// (part, vert) → 裙顶点下标（边/三角引用非裙端点时跳过推送）
    pub sv_index: HashMap<(usize, u32), u32>,
    pub tris: Vec<TriRef>,
    pub leg_tris: Vec<TriRef>,
    pub body_tris: Vec<TriRef>,
    /// 邻接表 CSR
    pub adj_off: Vec<u32>,
    pub adj: Vec<u32>,
    pub caps: Vec<LegCapDef>,
    // ---- 持久状态（setup 时随点集重建）----
    pub prev_disp: Vec<f64>,
    pub orig_snap: Vec<f64>,
    pub prev_abs: f64,
    pub relax_a: Vec<f64>,
    pub relax_b: Vec<f64>,
}

impl SkirtRig {
    pub fn empty() -> Self {
        SkirtRig {
            verts_part: vec![],
            verts_idx: vec![],
            sv_index: HashMap::new(),
            tris: vec![],
            leg_tris: vec![],
            body_tris: vec![],
            adj_off: vec![],
            adj: vec![],
            caps: vec![],
            prev_disp: vec![],
            orig_snap: vec![],
            prev_abs: f64::NAN,
            relax_a: vec![],
            relax_b: vec![],
        }
    }
}

#[derive(Default, Clone)]
pub struct DepenDiag {
    pub max_need: f64,
    pub max_bone: i32,
    pub clamp: u32,
}

pub struct PushRec {
    pub v: u32,
    pub phase: u8,
    pub k: f64,
    /// 腿骨索引（phase 2 无）
    pub bone: i32,
}

struct LiveCap {
    ax: f64,
    ay: f64,
    az: f64,
    bx: f64,
    by: f64,
    bz: f64,
    r: f64,
    bone: u32,
}

fn pos_of<'a>(parts: &'a [Option<PartSkin>], slot: usize) -> &'a [f32] {
    &parts[slot].as_ref().expect("裙 rig 引用了已注销部件").pos
}

fn pos_mut<'a>(parts: &'a mut [Option<PartSkin>], slot: usize) -> &'a mut Vec<f32> {
    &mut parts[slot].as_mut().expect("裙 rig 引用了已注销部件").pos
}

/// 裙摆去穿透主 pass。返回被改动 position 的部件槽位（TS 侧标 needsUpdate）。
#[allow(clippy::too_many_arguments)]
pub fn skirt_depen(
    parts: &mut [Option<PartSkin>],
    rig: &mut SkirtRig,
    world: &[f64],
    diag: &mut DepenDiag,
    record: bool,
    abs: f64,
    push_log: &mut Vec<PushRec>,
) -> Vec<usize> {
    if rig.verts_part.is_empty() || rig.caps.is_empty() || world.is_empty() {
        return vec![];
    }
    if record {
        push_log.clear();
    }
    // 每帧腿胶囊世界坐标
    let mut caps: Vec<LiveCap> = vec![];
    for c in &rig.caps {
        if world.len() < (c.bone as usize + 1) * 16 {
            continue;
        }
        let e = &world[c.bone as usize * 16..c.bone as usize * 16 + 16];
        caps.push(LiveCap {
            ax: e[12],
            ay: e[13],
            az: e[14],
            bx: e[12] + e[0] * c.len as f64,
            by: e[13] + e[1] * c.len as f64,
            bz: e[14] + e[2] * c.len as f64,
            r: c.len as f64 * CLOTH_LEG_R,
            bone: c.bone,
        });
    }
    if caps.is_empty() {
        return vec![];
    }
    let sv_count = rig.verts_part.len();
    let mut touched = vec![false; parts.len()];
    let mut budget = vec![SKIRT_MAX_PUSH; sv_count];
    // 阶段二"全外面片斜切满推"的顶点标记：阶段四点五豁免衰减
    let mut no_decay = vec![0u8; sv_count];
    // 去穿透前快照（阶段四平滑的位移基准 = skinPart 输出位置）
    for s in 0..sv_count {
        let o = rig.verts_idx[s] as usize * 3;
        let pa = pos_of(parts, rig.verts_part[s]);
        rig.orig_snap[s * 3] = pa[o] as f64;
        rig.orig_snap[s * 3 + 1] = pa[o + 1] as f64;
        rig.orig_snap[s * 3 + 2] = pa[o + 2] as f64;
    }

    // 阶段一：顶点推出（最多 3 轮：Thigh/Calf 胶囊在膝盖交叠，单遍依次
    // 推出在重叠区可能不收敛，迭代到无推出为止）
    for s in 0..sv_count {
        let slot = rig.verts_part[s];
        let o = rig.verts_idx[s] as usize * 3;
        let pa0 = pos_of(parts, slot);
        let mut px = pa0[o] as f64;
        let mut py = pa0[o + 1] as f64;
        let mut pz = pa0[o + 2] as f64;
        let o0 = (px, py, pz);
        let mut exhausted = false;
        for _pass in 0..3 {
            if exhausted {
                break;
            }
            let mut pushed = false;
            for c in &caps {
                let (abx, aby, abz) = (c.bx - c.ax, c.by - c.ay, c.bz - c.az);
                let (apx, apy, apz) = (px - c.ax, py - c.ay, pz - c.az);
                let ab2 = abx * abx + aby * aby + abz * abz;
                let mut t = if ab2 > 1e-9 {
                    (apx * abx + apy * aby + apz * abz) / ab2
                } else {
                    0.0
                };
                t = t.clamp(0.0, 1.0);
                let qx = c.ax + abx * t;
                let qy = c.ay + aby * t;
                let qz = c.az + abz * t;
                let vx = px - qx;
                let vy = py - qy;
                let vz = pz - qz;
                let d = (vx * vx + vy * vy + vz * vz).sqrt();
                let mut need = c.r + SKIRT_MARGIN - d;
                if need <= 1e-3 {
                    continue;
                }
                if need > diag.max_need {
                    diag.max_need = need;
                    diag.max_bone = c.bone as i32;
                }
                // 网格口径收口：胶囊命中但贴腿网格（裙腰顶边贴腿根）不推
                need = need.min((SKIRT_MESH_MARGIN - leg_signed_dist(parts, rig, &caps, px, py, pz)).max(0.0));
                if need <= 1e-3 {
                    continue;
                }
                if budget[s] <= 1e-3 {
                    diag.clamp += 1; // 预算耗尽，余量仍穿
                    exhausted = true;
                    break;
                }
                let k = need.min(budget[s]);
                if k < need {
                    diag.clamp += 1;
                }
                budget[s] -= k;
                if record {
                    push_log.push(PushRec { v: s as u32, phase: 1, k, bone: c.bone as i32 });
                }
                if d > 1e-6 {
                    px += (vx / d) * k;
                    py += (vy / d) * k;
                    pz += (vz / d) * k;
                } else {
                    py += k; // 退化：朝上推出
                }
                pushed = true;
            }
            if !pushed {
                break;
            }
        }
        if (px, py, pz) != o0 {
            let pa = pos_mut(parts, slot);
            pa[o] = px as f32;
            pa[o + 1] = py as f32;
            pa[o + 2] = pz as f32;
            touched[slot] = true;
        }
    }

    // 阶段二：面片跨穿推出（2 轮迭代）。判据 = 裙三角形与腿网格三角形的
    // 精确相交；方向 = 沿"违反最多"的胶囊轴径向外推，幅度取该口径加下限。
    for _round in 0..2 {
        let mut any_hit = false;
        let tris = std::mem::take(&mut rig.tris);
        for t in &tris {
            let sa = rig.sv_index.get(&(t.part, t.a)).copied().map(|v| v as i32).unwrap_or(-1);
            let sb = rig.sv_index.get(&(t.part, t.b)).copied().map(|v| v as i32).unwrap_or(-1);
            let sc = rig.sv_index.get(&(t.part, t.c)).copied().map(|v| v as i32).unwrap_or(-1);
            if sa < 0 && sb < 0 && sc < 0 {
                continue;
            }
            let pa = pos_of(parts, t.part);
            // —— 不可变阶段：找首个相交腿三角形 + 计算方向/幅度 ——
            let mut hit: Option<(usize, u32, u32, u32)> = None;
            for lt in &rig.leg_tris {
                let lp = pos_of(parts, lt.part);
                if tri_tri(pa, t.a as usize * 3, t.b as usize * 3, t.c as usize * 3,
                    lp, lt.a as usize * 3, lt.b as usize * 3, lt.c as usize * 3) {
                    hit = Some((lt.part, lt.a, lt.b, lt.c));
                    break;
                }
            }
            let Some((hp, ha, hb, hc)) = hit else { continue };
            any_hit = true;
            // 命中三角形对的几何中心做径向基准
            let pa2 = pos_of(parts, hp);
            let mut mx = 0.0;
            let mut my = 0.0;
            let mut mz = 0.0;
            for v in [t.a, t.b, t.c, ha, hb, hc] {
                let i = v as usize * 3;
                let src = if v == ha || v == hb || v == hc { pa2 } else { pa };
                mx += src[i] as f64;
                my += src[i + 1] as f64;
                mz += src[i + 2] as f64;
            }
            mx /= 6.0;
            my /= 6.0;
            mz /= 6.0;
            let mut need = 0.35f64;
            let mut ddx = 0.0f64;
            let mut ddy = 1.0f64;
            let mut ddz = 0.0f64;
            for c in &caps {
                let (abx, aby, abz) = (c.bx - c.ax, c.by - c.ay, c.bz - c.az);
                let ab2 = abx * abx + aby * aby + abz * abz;
                let mut tp = if ab2 > 1e-9 {
                    ((mx - c.ax) * abx + (my - c.ay) * aby + (mz - c.az) * abz) / ab2
                } else {
                    0.0
                };
                tp = tp.clamp(0.0, 1.0);
                let vx = mx - (c.ax + abx * tp);
                let vy = my - (c.ay + aby * tp);
                let vz = mz - (c.az + abz * tp);
                let d = (vx * vx + vy * vy + vz * vz).sqrt();
                let nd = c.r + SKIRT_MARGIN - d;
                if nd > need {
                    need = nd;
                    if d > 1e-6 {
                        ddx = vx / d;
                        ddy = vy / d;
                        ddz = vz / d;
                    }
                }
            }
            // 幅度收口：三顶点到被剐腿三角形的最近距离 + 0.3 为上限（浅剐浅推）
            {
                let la = [ha as usize * 3, hb as usize * 3, hc as usize * 3];
                let lx = [pa2[la[0]] as f64, pa2[la[1]] as f64, pa2[la[2]] as f64];
                let ly = [pa2[la[0] + 1] as f64, pa2[la[1] + 1] as f64, pa2[la[2] + 1] as f64];
                let lz = [pa2[la[0] + 2] as f64, pa2[la[1] + 2] as f64, pa2[la[2] + 2] as f64];
                let mut dmin = f64::INFINITY;
                for vi in [t.a as usize * 3, t.b as usize * 3, t.c as usize * 3] {
                    dmin = dmin.min(pt_tri_dist(
                        pa[vi] as f64, pa[vi + 1] as f64, pa[vi + 2] as f64,
                        lx[0], ly[0], lz[0], lx[1], ly[1], lz[1], lx[2], ly[2], lz[2],
                    ));
                }
                need = need.min(dmin + 0.3);
                if need < 0.35 {
                    need = 0.35;
                }
            }
            let k = need
                .min(if sa >= 0 { budget[sa as usize] } else { f64::INFINITY })
                .min(if sb >= 0 { budget[sb as usize] } else { f64::INFINITY })
                .min(if sc >= 0 { budget[sc as usize] } else { f64::INFINITY });
            if k > 1e-3 {
                // dm 判据用帧首 LBS 基准位置（orig_snap）
                let dm_of = |s: i32| -> f64 {
                    if s < 0 {
                        return f64::INFINITY;
                    }
                    let s3 = s as usize * 3;
                    leg_signed_dist(
                        parts, rig, &caps,
                        rig.orig_snap[s3], rig.orig_snap[s3 + 1], rig.orig_snap[s3 + 2],
                    )
                };
                let (dma, dmb, dmc) = (dm_of(sa), dm_of(sb), dm_of(sc));
                let w_of = |dm: f64| if dm <= SKIRT_MESH_MARGIN { 1.0 } else { (1.0 - (dm - SKIRT_MESH_MARGIN) / 0.5).max(0.0) };
                // —— 可变阶段：施加推出 ——
                if dma < 0.0 || dmb < 0.0 || dmc < 0.0 {
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sa, k * w_of(dma), ddx, ddy, ddz);
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sb, k * w_of(dmb), ddx, ddy, ddz);
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sc, k * w_of(dmc), ddx, ddy, ddz);
                } else {
                    // 斜切满推也要区分：贴腿面顶点压到 0.3，不贴的满推并标记免衰减
                    let w_push = |dm: f64| if dm >= SKIRT_MESH_MARGIN + 0.5 { 1.0 } else { 0.3 };
                    for (s, dm) in [(sa, dma), (sb, dmb), (sc, dmc)] {
                        if s >= 0 && w_push(dm) >= 1.0 {
                            no_decay[s as usize] = 1;
                        }
                    }
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sa, k * w_push(dma), ddx, ddy, ddz);
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sb, k * w_push(dmb), ddx, ddy, ddz);
                    push_vert(parts, rig, &mut budget, record, push_log, &mut touched, sc, k * w_push(dmc), ddx, ddy, ddz);
                }
            }
        }
        rig.tris = tris;
        if !any_hit {
            break;
        }
    }

    // 阶段二点五：位移场空间松弛（邻域均值 ×2 迭代，把推出量扩散抹平梯度）
    if rig.adj_off.len() == sv_count + 1 && sv_count > 0 {
        for s in 0..sv_count {
            let o = rig.verts_idx[s] as usize * 3;
            let pa = pos_of(parts, rig.verts_part[s]);
            rig.relax_a[s * 3] = pa[o] as f64 - rig.orig_snap[s * 3];
            rig.relax_a[s * 3 + 1] = pa[o + 1] as f64 - rig.orig_snap[s * 3 + 1];
            rig.relax_a[s * 3 + 2] = pa[o + 2] as f64 - rig.orig_snap[s * 3 + 2];
        }
        for _iter in 0..2 {
            for s in 0..sv_count {
                let off = rig.adj_off[s] as usize;
                let end = rig.adj_off[s + 1] as usize;
                let s3 = s * 3;
                if off == end {
                    rig.relax_b[s3..s3 + 3].copy_from_slice(&rig.relax_a[s3..s3 + 3]);
                    continue;
                }
                let n = (end - off) as f64;
                let mut ax = 0.0;
                let mut ay = 0.0;
                let mut az = 0.0;
                for &nb in &rig.adj[off..end] {
                    let nb3 = nb as usize * 3;
                    ax += rig.relax_a[nb3];
                    ay += rig.relax_a[nb3 + 1];
                    az += rig.relax_a[nb3 + 2];
                }
                ax /= n;
                ay /= n;
                az /= n;
                rig.relax_b[s3] = rig.relax_a[s3] + (ax - rig.relax_a[s3]) * SKIRT_RELAX_R;
                rig.relax_b[s3 + 1] = rig.relax_a[s3 + 1] + (ay - rig.relax_a[s3 + 1]) * SKIRT_RELAX_R;
                rig.relax_b[s3 + 2] = rig.relax_a[s3 + 2] + (az - rig.relax_a[s3 + 2]) * SKIRT_RELAX_R;
            }
            rig.relax_a.copy_from_slice(&rig.relax_b);
        }
        for s in 0..sv_count {
            let slot = rig.verts_part[s];
            let o = rig.verts_idx[s] as usize * 3;
            let s3 = s * 3;
            let pa = pos_mut(parts, slot);
            pa[o] = (rig.orig_snap[s3] + rig.relax_a[s3]) as f32;
            pa[o + 1] = (rig.orig_snap[s3 + 1] + rig.relax_a[s3 + 1]) as f32;
            pa[o + 2] = (rig.orig_snap[s3 + 2] + rig.relax_a[s3 + 2]) as f32;
            touched[slot] = true;
        }
    }

    // 阶段三：复扫顶点（跨穿推出可能把端点推进其他胶囊，一轮收敛）
    for s in 0..sv_count {
        let slot = rig.verts_part[s];
        let o = rig.verts_idx[s] as usize * 3;
        let pa0 = pos_of(parts, slot);
        let mut px = pa0[o] as f64;
        let mut py = pa0[o + 1] as f64;
        let mut pz = pa0[o + 2] as f64;
        let o0 = (px, py, pz);
        for c in &caps {
            let (abx, aby, abz) = (c.bx - c.ax, c.by - c.ay, c.bz - c.az);
            let (apx, apy, apz) = (px - c.ax, py - c.ay, pz - c.az);
            let ab2 = abx * abx + aby * aby + abz * abz;
            let mut t = if ab2 > 1e-9 {
                (apx * abx + apy * aby + apz * abz) / ab2
            } else {
                0.0
            };
            t = t.clamp(0.0, 1.0);
            let vx = px - (c.ax + abx * t);
            let vy = py - (c.ay + aby * t);
            let vz = pz - (c.az + abz * t);
            let d = (vx * vx + vy * vy + vz * vz).sqrt();
            let mut need = c.r + SKIRT_MARGIN - d;
            if need <= 1e-3 {
                continue;
            }
            // 网格口径收口（与阶段一同口径）
            need = need.min((SKIRT_MESH_MARGIN - leg_signed_dist(parts, rig, &caps, px, py, pz)).max(0.0));
            if need <= 1e-3 {
                continue;
            }
            if budget[s] <= 1e-3 {
                diag.clamp += 1;
                break;
            }
            let k = need.min(budget[s]);
            if k < need {
                diag.clamp += 1;
            }
            budget[s] -= k;
            if record {
                push_log.push(PushRec { v: s as u32, phase: 3, k, bone: c.bone as i32 });
            }
            if d > 1e-6 {
                px += (vx / d) * k;
                py += (vy / d) * k;
                pz += (vz / d) * k;
            } else {
                py += k;
            }
        }
        if (px, py, pz) != o0 {
            let pa = pos_mut(parts, slot);
            pa[o] = px as f32;
            pa[o + 1] = py as f32;
            pa[o + 2] = pz as f32;
            touched[slot] = true;
        }
    }

    // 阶段四：推出量时间平滑（入快出慢 lerp；seek 直接用当帧原始值；
    // 平滑后仍在胶囊内 → 回退满推出，顶点级正确性不妥协）
    let seek = !abs.is_finite()
        || !rig.prev_abs.is_finite()
        || (abs - rig.prev_abs).abs() > 2.0;
    rig.prev_abs = abs;
    for s in 0..sv_count {
        let slot = rig.verts_part[s];
        let o = rig.verts_idx[s] as usize * 3;
        let s3 = s * 3;
        let (ox, oy, oz) = (rig.orig_snap[s3], rig.orig_snap[s3 + 1], rig.orig_snap[s3 + 2]);
        let pa0 = pos_of(parts, slot);
        let dx = pa0[o] as f64 - ox;
        let dy = pa0[o + 1] as f64 - oy;
        let dz = pa0[o + 2] as f64 - oz;
        let (mut nx, mut ny, mut nz);
        if seek {
            nx = dx;
            ny = dy;
            nz = dz;
        } else {
            let (pdx, pdy, pdz) = (rig.prev_disp[s3], rig.prev_disp[s3 + 1], rig.prev_disp[s3 + 2]);
            let r = if (dx * dx + dy * dy + dz * dz).sqrt()
                >= (pdx * pdx + pdy * pdy + pdz * pdz).sqrt() - 1e-9
            {
                SKIRT_SMOOTH_UP
            } else {
                SKIRT_SMOOTH_DOWN
            };
            nx = pdx + (dx - pdx) * r;
            ny = pdy + (dy - pdy) * r;
            nz = pdz + (dz - pdz) * r;
            // 顶点级安全网：平滑结果仍在胶囊内 → 满推出（网格口径收口）
            let fx = ox + nx;
            let fy = oy + ny;
            let fz = oz + nz;
            let mut need = 0.0f64;
            for c in &caps {
                let (abx, aby, abz) = (c.bx - c.ax, c.by - c.ay, c.bz - c.az);
                let ab2 = abx * abx + aby * aby + abz * abz;
                let mut t = if ab2 > 1e-9 {
                    ((fx - c.ax) * abx + (fy - c.ay) * aby + (fz - c.az) * abz) / ab2
                } else {
                    0.0
                };
                t = t.clamp(0.0, 1.0);
                let d = ((fx - (c.ax + abx * t)).powi(2)
                    + (fy - (c.ay + aby * t)).powi(2)
                    + (fz - (c.az + abz * t)).powi(2))
                .sqrt();
                need = need.max(c.r + SKIRT_MARGIN - d);
            }
            if need > 1e-3 {
                need = need.min(
                    (SKIRT_MESH_MARGIN - leg_signed_dist(parts, rig, &caps, fx, fy, fz)).max(0.0),
                );
            }
            if need > 1e-3 {
                nx = dx;
                ny = dy;
                nz = dz;
            }
        }
        let pa = pos_mut(parts, slot);
        pa[o] = (ox + nx) as f32;
        pa[o + 1] = (oy + ny) as f32;
        pa[o + 2] = (oz + nz) as f32;
        rig.prev_disp[s3] = nx;
        rig.prev_disp[s3 + 1] = ny;
        rig.prev_disp[s3 + 2] = nz;
        touched[slot] = true;
    }

    // 阶段四点五：贴腿外侧裙顶点的位移最终衰减（判据用 LBS 基准位置的 dm）
    for s in 0..sv_count {
        let slot = rig.verts_part[s];
        let o = rig.verts_idx[s] as usize * 3;
        let s3 = s * 3;
        let dm = leg_signed_dist(parts, rig, &caps, rig.orig_snap[s3], rig.orig_snap[s3 + 1], rig.orig_snap[s3 + 2]);
        if dm <= SKIRT_MESH_MARGIN || no_decay[s] != 0 {
            continue;
        }
        let w = ((1.0 - (dm - SKIRT_MESH_MARGIN) / 0.5).max(0.0)).max(0.2);
        {
            let pa = pos_mut(parts, slot);
            pa[o] = (rig.orig_snap[s3] + (pa[o] as f64 - rig.orig_snap[s3]) * w) as f32;
            pa[o + 1] = (rig.orig_snap[s3 + 1] + (pa[o + 1] as f64 - rig.orig_snap[s3 + 1]) * w) as f32;
            pa[o + 2] = (rig.orig_snap[s3 + 2] + (pa[o + 2] as f64 - rig.orig_snap[s3 + 2]) * w) as f32;
        }
        rig.prev_disp[s3] *= w;
        rig.prev_disp[s3 + 1] *= w;
        rig.prev_disp[s3 + 2] *= w;
        touched[slot] = true;
    }

    // 阶段五：动态接触跟随（防贴合开缝）：裙顶边位移传给贴合躯干三角形三顶点
    if !rig.body_tris.is_empty() {
        // 累积缓冲：同一躯干顶点可能收到多个裙点的跟随位移，最后统一施加
        let mut acc: HashMap<(usize, u32), [f64; 3]> = HashMap::new();
        for s in 0..sv_count {
            let s3 = s * 3;
            let (dx0, dy0, dz0) = (rig.prev_disp[s3], rig.prev_disp[s3 + 1], rig.prev_disp[s3 + 2]);
            if (dx0 * dx0 + dy0 * dy0 + dz0 * dz0).sqrt() < 0.3 {
                continue;
            }
            let (ox, oy, oz) = (rig.orig_snap[s3], rig.orig_snap[s3 + 1], rig.orig_snap[s3 + 2]);
            let mut best_d = SKIRT_WELD_D;
            let mut bt: Option<(usize, u32, u32, u32)> = None;
            let mut wa = 0.0f64;
            let mut wb = 0.0f64;
            let mut wc = 0.0f64;
            let mut bary = [0.0f64; 4];
            for t in &rig.body_tris {
                let pa2 = pos_of(parts, t.part);
                let a3 = t.a as usize * 3;
                let b3 = t.b as usize * 3;
                let c3 = t.c as usize * 3;
                // 粗筛：三顶点都远于 4 不可能贴合（三角形外接半径 <3.4）
                let da = (pa2[a3] as f64 - ox).powi(2) + (pa2[a3 + 1] as f64 - oy).powi(2) + (pa2[a3 + 2] as f64 - oz).powi(2);
                let db = (pa2[b3] as f64 - ox).powi(2) + (pa2[b3 + 1] as f64 - oy).powi(2) + (pa2[b3 + 2] as f64 - oz).powi(2);
                let dc = (pa2[c3] as f64 - ox).powi(2) + (pa2[c3 + 1] as f64 - oy).powi(2) + (pa2[c3 + 2] as f64 - oz).powi(2);
                if da > 16.0 && db > 16.0 && dc > 16.0 {
                    continue;
                }
                let d = pt_tri_bary(&mut bary, ox, oy, oz,
                    pa2[a3] as f64, pa2[a3 + 1] as f64, pa2[a3 + 2] as f64,
                    pa2[b3] as f64, pa2[b3 + 1] as f64, pa2[b3 + 2] as f64,
                    pa2[c3] as f64, pa2[c3 + 1] as f64, pa2[c3 + 2] as f64);
                if d < best_d {
                    best_d = d;
                    bt = Some((t.part, t.a, t.b, t.c));
                    wa = bary[1];
                    wb = bary[2];
                    wc = bary[3];
                }
            }
            let Some((bp, ba, bb, bc)) = bt else { continue };
            // 位移后与同一三角形的距离（三角形未被推，位置即 LBS）
            let pa2 = pos_of(parts, bp);
            let (a3, b3, c3) = (ba as usize * 3, bb as usize * 3, bc as usize * 3);
            let d1 = pt_tri_bary(&mut bary, ox + dx0, oy + dy0, oz + dz0,
                pa2[a3] as f64, pa2[a3 + 1] as f64, pa2[a3 + 2] as f64,
                pa2[b3] as f64, pa2[b3 + 1] as f64, pa2[b3 + 2] as f64,
                pa2[c3] as f64, pa2[c3 + 1] as f64, pa2[c3 + 2] as f64);
            if d1 - best_d <= SKIRT_WELD_SEP {
                continue;
            }
            let f = SKIRT_FOLLOW;
            for (vi, w) in [(ba, wa), (bb, wb), (bc, wc)] {
                if w <= 1e-3 {
                    continue;
                }
                let e = acc.entry((bp, vi)).or_insert([0.0; 3]);
                e[0] += dx0 * f * w;
                e[1] += dy0 * f * w;
                e[2] += dz0 * f * w;
            }
        }
        let mut touched_slots: Vec<usize> = acc.keys().map(|(p, _)| *p).collect();
        touched_slots.sort_unstable();
        touched_slots.dedup();
        for ((bp, vi), e) in &acc {
            let pa = pos_mut(parts, *bp);
            let o = *vi as usize * 3;
            pa[o] = (pa[o] as f64 + e[0]) as f32;
            pa[o + 1] = (pa[o + 1] as f64 + e[1]) as f32;
            pa[o + 2] = (pa[o + 2] as f64 + e[2]) as f32;
        }
        for p in touched_slots {
            touched[p] = true;
        }
    }

    touched
        .into_iter()
        .enumerate()
        .filter(|(_, t)| *t)
        .map(|(i, _)| i)
        .collect()
}

/// 阶段二的顶点推送（跨穿整面外推；预算扣减 + phase2 记录）
#[allow(clippy::too_many_arguments)]
fn push_vert(
    parts: &mut [Option<PartSkin>],
    rig: &SkirtRig,
    budget: &mut [f64],
    record: bool,
    push_log: &mut Vec<PushRec>,
    touched: &mut [bool],
    s: i32,
    k: f64,
    dx: f64,
    dy: f64,
    dz: f64,
) {
    if s < 0 || k <= 0.0 {
        return;
    }
    let s = s as usize;
    let slot = rig.verts_part[s];
    let o = rig.verts_idx[s] as usize * 3;
    let pa = pos_mut(parts, slot);
    pa[o] += (dx * k) as f32;
    pa[o + 1] += (dy * k) as f32;
    pa[o + 2] += (dz * k) as f32;
    budget[s] -= k;
    touched[slot] = true;
    if record {
        push_log.push(PushRec { v: s as u32, phase: 2, k, bone: -1 });
    }
}

/// 带符号腿网格距离（粗筛半径 6）：phase 1/3/4 安全网的胶囊→网格口径收口。
/// 取最近腿三角形，按其外向法线侧别定号——腿管近似凸，网格内（真穿透）
/// 为负、外（贴合/远离）为正。
fn leg_signed_dist(
    parts: &[Option<PartSkin>],
    rig: &SkirtRig,
    caps: &[LiveCap],
    px: f64,
    py: f64,
    pz: f64,
) -> f64 {
    let mut best = f64::INFINITY;
    let mut hit: Option<(usize, [usize; 3], [f64; 3])> = None; // (part, [a3,b3,c3], [wa,wb,wc])
    let mut bary = [0.0f64; 4];
    for t in &rig.leg_tris {
        let pa = pos_of(parts, t.part);
        let a3 = t.a as usize * 3;
        let b3 = t.b as usize * 3;
        let c3 = t.c as usize * 3;
        let da = (pa[a3] as f64 - px).powi(2) + (pa[a3 + 1] as f64 - py).powi(2) + (pa[a3 + 2] as f64 - pz).powi(2);
        let db = (pa[b3] as f64 - px).powi(2) + (pa[b3 + 1] as f64 - py).powi(2) + (pa[b3 + 2] as f64 - pz).powi(2);
        let dc = (pa[c3] as f64 - px).powi(2) + (pa[c3 + 1] as f64 - py).powi(2) + (pa[c3 + 2] as f64 - pz).powi(2);
        if da > 64.0 && db > 64.0 && dc > 64.0 {
            continue;
        }
        let d = pt_tri_bary(&mut bary, px, py, pz,
            pa[a3] as f64, pa[a3 + 1] as f64, pa[a3 + 2] as f64,
            pa[b3] as f64, pa[b3 + 1] as f64, pa[b3 + 2] as f64,
            pa[c3] as f64, pa[c3 + 1] as f64, pa[c3 + 2] as f64);
        if d < best {
            best = d;
            hit = Some((t.part, [a3, b3, c3], [bary[1], bary[2], bary[3]]));
        }
    }
    let Some((part, offs, w)) = hit else { return best };
    let pa = pos_of(parts, part);
    let cx = pa[offs[0]] as f64 * w[0] + pa[offs[1]] as f64 * w[1] + pa[offs[2]] as f64 * w[2];
    let cy = pa[offs[0] + 1] as f64 * w[0] + pa[offs[1] + 1] as f64 * w[1] + pa[offs[2] + 1] as f64 * w[2];
    let cz = pa[offs[0] + 2] as f64 * w[0] + pa[offs[1] + 2] as f64 * w[1] + pa[offs[2] + 2] as f64 * w[2];
    // 侧别用胶囊径向（腿管≈圆柱，轴=腿骨）：p−closest 与 closest−轴投影
    // 同向=面外正、反向=面内负。不依赖网格绕向——腿三角形绕向不统一
    let mut refx = 0.0f64;
    let mut refy = 0.0f64;
    let mut refz = 0.0f64;
    let mut ref_len = f64::INFINITY;
    for c in caps {
        let (abx, aby, abz) = (c.bx - c.ax, c.by - c.ay, c.bz - c.az);
        let ab2 = abx * abx + aby * aby + abz * abz;
        let mut t = if ab2 > 1e-9 {
            ((cx - c.ax) * abx + (cy - c.ay) * aby + (cz - c.az) * abz) / ab2
        } else {
            0.0
        };
        t = t.clamp(0.0, 1.0);
        let dx = cx - (c.ax + abx * t);
        let dy = cy - (c.ay + aby * t);
        let dz = cz - (c.az + abz * t);
        let l2 = dx * dx + dy * dy + dz * dz;
        if l2 > 1e-9 && l2 < ref_len {
            ref_len = l2;
            refx = dx;
            refy = dy;
            refz = dz;
        }
    }
    if ref_len.is_finite() {
        let rl = ref_len.sqrt();
        let sgn = ((px - cx) * refx + (py - cy) * refy + (pz - cz) * refz) / rl;
        let sign = if sgn == 0.0 { 1.0 } else { sgn.signum() };
        return ((px - cx).powi(2) + (py - cy).powi(2) + (pz - cz).powi(2)).sqrt() * sign;
    }
    best
}

/// 三角形-三角形相交（标量无分配）：AABB 早退 → 平面侧别早退 → 边-三角形
/// 求交（Möller–Trumbore 式，t∈[0,1]）双向。共面重叠返回 false 跳过。
pub fn tri_tri(
    pa: &[f32], a3: usize, b3: usize, c3: usize,
    lp: &[f32], u3: usize, v3: usize, w3: usize,
) -> bool {
    let (ax, ay, az) = (pa[a3] as f64, pa[a3 + 1] as f64, pa[a3 + 2] as f64);
    let (bx, by, bz) = (pa[b3] as f64, pa[b3 + 1] as f64, pa[b3 + 2] as f64);
    let (cx, cy, cz) = (pa[c3] as f64, pa[c3 + 1] as f64, pa[c3 + 2] as f64);
    let (ux, uy, uz) = (lp[u3] as f64, lp[u3 + 1] as f64, lp[u3 + 2] as f64);
    let (vx, vy, vz) = (lp[v3] as f64, lp[v3 + 1] as f64, lp[v3 + 2] as f64);
    let (wx, wy, wz) = (lp[w3] as f64, lp[w3 + 1] as f64, lp[w3 + 2] as f64);
    // AABB
    if ax.min(bx).min(cx) > ux.max(vx).max(wx) { return false; }
    if ux.min(vx).min(wx) > ax.max(bx).max(cx) { return false; }
    if ay.min(by).min(cy) > uy.max(vy).max(wy) { return false; }
    if uy.min(vy).min(wy) > ay.max(by).max(cy) { return false; }
    if az.min(bz).min(cz) > uz.max(vz).max(wz) { return false; }
    if uz.min(vz).min(wz) > az.max(bz).max(cz) { return false; }
    // 法线（各三角形自身绕向）
    let (abx, aby, abz) = (bx - ax, by - ay, bz - az);
    let (acx, acy, acz) = (cx - ax, cy - ay, cz - az);
    let nvx = aby * acz - abz * acy;
    let nvy = abz * acx - abx * acz;
    let nvz = abx * acy - aby * acx;
    let (uvx, uvy, uvz) = (vx - ux, vy - uy, vz - uz);
    let (uwx, uwy, uwz) = (wx - ux, wy - uy, wz - uz);
    let nux = uvy * uwz - uvz * uwy;
    let nuy = uvz * uwx - uvx * uwz;
    let nuz = uvx * uwy - uvy * uwx;
    // 平面侧别早退
    let d0 = nux * (ax - ux) + nuy * (ay - uy) + nuz * (az - uz);
    let d1 = nux * (bx - ux) + nuy * (by - uy) + nuz * (bz - uz);
    let d2 = nux * (cx - ux) + nuy * (cy - uy) + nuz * (cz - uz);
    if d0 > 1e-9 && d1 > 1e-9 && d2 > 1e-9 { return false; }
    if d0 < -1e-9 && d1 < -1e-9 && d2 < -1e-9 { return false; }
    let e0 = nvx * (ux - ax) + nvy * (uy - ay) + nvz * (uz - az);
    let e1 = nvx * (vx - ax) + nvy * (vy - ay) + nvz * (vz - az);
    let e2 = nvx * (wx - ax) + nvy * (wy - ay) + nvz * (wz - az);
    if e0 > 1e-9 && e1 > 1e-9 && e2 > 1e-9 { return false; }
    if e0 < -1e-9 && e1 < -1e-9 && e2 < -1e-9 { return false; }
    if d0.abs() < 1e-9 && d1.abs() < 1e-9 && d2.abs() < 1e-9 { return false; } // 共面跳过
    // A 的边穿 U 平面且交点在 U 内
    for (px0, py0, pz0, dx, dy, dz) in [
        (ax, ay, az, bx - ax, by - ay, bz - az),
        (bx, by, bz, cx - bx, cy - by, cz - bz),
        (cx, cy, cz, ax - cx, ay - cy, az - cz),
    ] {
        let dn = nux * dx + nuy * dy + nuz * dz;
        if dn.abs() > 1e-12 {
            let t = (nux * (ux - px0) + nuy * (uy - py0) + nuz * (uz - pz0)) / dn;
            if (0.0..=1.0).contains(&t) {
                let (ix, iy, iz) = (px0 + dx * t, py0 + dy * t, pz0 + dz * t);
                if in_tri(ix, iy, iz, ux, uy, uz, vx, vy, vz, wx, wy, wz, nux, nuy, nuz) {
                    return true;
                }
            }
        }
    }
    // U 的边穿 A 平面且交点在 A 内
    for (px0, py0, pz0, dx, dy, dz) in [
        (ux, uy, uz, vx - ux, vy - uy, vz - uz),
        (vx, vy, vz, wx - vx, wy - vy, wz - vz),
        (wx, wy, wz, ux - wx, uy - wy, uz - wz),
    ] {
        let dn = nvx * dx + nvy * dy + nvz * dz;
        if dn.abs() > 1e-12 {
            let t = (nvx * (ax - px0) + nvy * (ay - py0) + nvz * (az - pz0)) / dn;
            if (0.0..=1.0).contains(&t) {
                let (ix, iy, iz) = (px0 + dx * t, py0 + dy * t, pz0 + dz * t);
                if in_tri(ix, iy, iz, ax, ay, az, bx, by, bz, cx, cy, cz, nvx, nvy, nvz) {
                    return true;
                }
            }
        }
    }
    false
}

/// 点到三角形最近距离（Ericson，标量无分配；用于跨穿推出的幅度收口）
pub fn pt_tri_dist(
    px: f64, py: f64, pz: f64,
    ax: f64, ay: f64, az: f64,
    bx: f64, by: f64, bz: f64,
    cx: f64, cy: f64, cz: f64,
) -> f64 {
    let (abx, aby, abz) = (bx - ax, by - ay, bz - az);
    let (acx, acy, acz) = (cx - ax, cy - ay, cz - az);
    let (apx, apy, apz) = (px - ax, py - ay, pz - az);
    let d1 = abx * apx + aby * apy + abz * apz;
    let d2 = acx * apx + acy * apy + acz * apz;
    if d1 <= 0.0 && d2 <= 0.0 {
        return (apx * apx + apy * apy + apz * apz).sqrt();
    }
    let (bpx, bpy, bpz) = (px - bx, py - by, pz - bz);
    let d3 = abx * bpx + aby * bpy + abz * bpz;
    let d4 = acx * bpx + acy * bpy + acz * bpz;
    if d3 >= 0.0 && d4 <= d3 {
        return (bpx * bpx + bpy * bpy + bpz * bpz).sqrt();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let t = d1 / (d1 - d3);
        return ((ax + abx * t - px).powi(2) + (ay + aby * t - py).powi(2) + (az + abz * t - pz).powi(2)).sqrt();
    }
    let (cpx, cpy, cpz) = (px - cx, py - cy, pz - cz);
    let d5 = abx * cpx + aby * cpy + abz * cpz;
    let d6 = acx * cpx + acy * cpy + acz * cpz;
    if d6 >= 0.0 && d5 <= d6 {
        return (cpx * cpx + cpy * cpy + cpz * cpz).sqrt();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let t = d2 / (d2 - d6);
        return ((ax + acx * t - px).powi(2) + (ay + acy * t - py).powi(2) + (az + acz * t - pz).powi(2)).sqrt();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let t = (d4 - d3) / (d4 - d3 + (d5 - d6));
        return ((bx + (cx - bx) * t - px).powi(2) + (by + (cy - by) * t - py).powi(2) + (bz + (cz - bz) * t - pz).powi(2)).sqrt();
    }
    let den = va + vb + vc;
    let t0 = vb / den;
    let t1 = vc / den;
    ((ax + abx * t0 + acx * t1 - px).powi(2)
        + (ay + aby * t0 + acy * t1 - py).powi(2)
        + (az + abz * t0 + acz * t1 - pz).powi(2))
    .sqrt()
}

/// 点到三角形最近点 + 重心权（wA+wB+wC=1；out = [d, wA, wB, wC]）
pub fn pt_tri_bary(
    out: &mut [f64; 4],
    px: f64, py: f64, pz: f64,
    ax: f64, ay: f64, az: f64,
    bx: f64, by: f64, bz: f64,
    cx: f64, cy: f64, cz: f64,
) -> f64 {
    let (abx, aby, abz) = (bx - ax, by - ay, bz - az);
    let (acx, acy, acz) = (cx - ax, cy - ay, cz - az);
    let (apx, apy, apz) = (px - ax, py - ay, pz - az);
    let d1 = abx * apx + aby * apy + abz * apz;
    let d2 = acx * apx + acy * apy + acz * apz;
    let set = |out: &mut [f64; 4], d: f64, wa: f64, wb: f64, wc: f64| {
        out[0] = d;
        out[1] = wa;
        out[2] = wb;
        out[3] = wc;
        d
    };
    if d1 <= 0.0 && d2 <= 0.0 {
        return set(out, (apx * apx + apy * apy + apz * apz).sqrt(), 1.0, 0.0, 0.0);
    }
    let (bpx, bpy, bpz) = (px - bx, py - by, pz - bz);
    let d3 = abx * bpx + aby * bpy + abz * bpz;
    let d4 = acx * bpx + acy * bpy + acz * bpz;
    if d3 >= 0.0 && d4 <= d3 {
        return set(out, (bpx * bpx + bpy * bpy + bpz * bpz).sqrt(), 0.0, 1.0, 0.0);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let t = d1 / (d1 - d3);
        let d = ((ax + abx * t - px).powi(2) + (ay + aby * t - py).powi(2) + (az + abz * t - pz).powi(2)).sqrt();
        return set(out, d, 1.0 - t, t, 0.0);
    }
    let (cpx, cpy, cpz) = (px - cx, py - cy, pz - cz);
    let d5 = abx * cpx + aby * cpy + abz * cpz;
    let d6 = acx * cpx + acy * cpy + acz * cpz;
    if d6 >= 0.0 && d5 <= d6 {
        return set(out, (cpx * cpx + cpy * cpy + cpz * cpz).sqrt(), 0.0, 0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let t = d2 / (d2 - d6);
        let d = ((ax + acx * t - px).powi(2) + (ay + acy * t - py).powi(2) + (az + acz * t - pz).powi(2)).sqrt();
        return set(out, d, 1.0 - t, 0.0, t);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let t = (d4 - d3) / (d4 - d3 + (d5 - d6));
        let d = ((bx + (cx - bx) * t - px).powi(2) + (by + (cy - by) * t - py).powi(2) + (bz + (cz - bz) * t - pz).powi(2)).sqrt();
        return set(out, d, 0.0, 1.0 - t, t);
    }
    let den = va + vb + vc;
    let t0 = vb / den;
    let t1 = vc / den;
    let d = ((ax + abx * t0 + acx * t1 - px).powi(2)
        + (ay + aby * t0 + acy * t1 - py).powi(2)
        + (az + abz * t0 + acz * t1 - pz).powi(2))
    .sqrt();
    set(out, d, 1.0 - t0 - t1, t0, t1)
}

/// 点在三角形内（已知三角形法线；绕向：cross(边, 点-边起点)·n ≥ 0 全边成立）
fn in_tri(
    px: f64, py: f64, pz: f64,
    ax: f64, ay: f64, az: f64,
    bx: f64, by: f64, bz: f64,
    cx: f64, cy: f64, cz: f64,
    nx: f64, ny: f64, nz: f64,
) -> bool {
    // AB × AP · n
    let t = nx * ((bx - ax) * (pz - az) - (bz - az) * (py - ay))
        + ny * ((bz - az) * (px - ax) - (bx - ax) * (pz - az))
        + nz * ((bx - ax) * (py - ay) - (by - ay) * (px - ax));
    if t < 0.0 {
        return false;
    }
    // BC × BP · n
    let t = nx * ((cx - bx) * (pz - bz) - (cz - bz) * (py - by))
        + ny * ((cz - bz) * (px - bx) - (cx - bx) * (pz - bz))
        + nz * ((cx - bx) * (py - by) - (cy - by) * (px - bx));
    if t < 0.0 {
        return false;
    }
    // CA × CP · n
    let t = nx * ((ax - cx) * (pz - cz) - (az - cz) * (py - cy))
        + ny * ((az - cz) * (px - cx) - (ax - cx) * (pz - cz))
        + nz * ((ax - cx) * (py - cy) - (ay - cy) * (px - cx));
    t >= 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tri_tris_crossing() {
        // 裙三角 (z=0 平面) 与腿三角 (x=0 平面) 互相穿越
        let skirt: Vec<f32> = vec![-5.0, -1.0, 0.0, 5.0, -1.0, 0.0, 0.0, 1.0, 0.0];
        let leg: Vec<f32> = vec![0.0, -1.0, -5.0, 0.0, -1.0, 5.0, 0.0, 1.0, 0.0];
        assert!(tri_tri(&skirt, 0, 3, 6, &leg, 0, 3, 6));
    }

    #[test]
    fn tri_tris_separate() {
        let a: Vec<f32> = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let b: Vec<f32> = vec![5.0, 5.0, 5.0, 6.0, 5.0, 5.0, 5.0, 6.0, 5.0];
        assert!(!tri_tri(&a, 0, 3, 6, &b, 0, 3, 6));
    }

    #[test]
    fn tri_tris_coplanar_skip() {
        // 共面重叠（服装双层布的设计内重合）返回 false
        let a: Vec<f32> = vec![0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0, 0.0];
        let b: Vec<f32> = vec![0.5, 0.5, 0.0, 2.5, 0.5, 0.0, 0.5, 2.5, 0.0];
        assert!(!tri_tri(&a, 0, 3, 6, &b, 0, 3, 6));
    }

    #[test]
    fn pt_tri_bary_inside() {
        // 直角三角形 (0,0),(1,0),(0,1) 内点 (0.25,0.25)：重心权 (0.5, 0.25, 0.25)
        let mut out = [0.0f64; 4];
        let d = pt_tri_bary(&mut out, 0.25, 0.25, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0);
        assert!(d.abs() < 1e-12);
        assert!((out[1] - 0.5).abs() < 1e-12 && (out[2] - 0.25).abs() < 1e-12 && (out[3] - 0.25).abs() < 1e-12);
    }

    #[test]
    fn pt_tri_dist_vertex_region() {
        // 点在顶点 A 外侧（d1≤0 且 d2≤0）：距离 = |AP|
        let d = pt_tri_dist(-3.0, -4.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0);
        assert!((d - 5.0).abs() < 1e-12);
    }

    #[test]
    fn capsule_vertex_push_out() {
        // 单部件：顶点 0 = 裙顶点 (5,0,0.1)（胶囊沿骨骼 +X：a=(0,0,0) b=(10,0,0)，轴旁 0.1），
        // 顶点 1/2/3 = 腿网格大三角形（z=0.1 平面，覆盖顶点）→ dm=0
        // 阶段一需要量 = min(r+margin-d, margin-dm) = 0.15 → 顶点 z: 0.1 → 0.25
        let tri = vec![5.0, 0.0, 0.1, 2.0, -3.0, 0.1, 8.0, -3.0, 0.1, 5.0, 3.0, 0.1];
        let mut parts = vec![Some(PartSkin {
            orig_count: 4,
            vert_count: 4,
            resolved_src: vec![0, 1, 2, 3],
            src_pos: tri.clone(),
            src_nrm: vec![0.0; 12],
            infl_off: vec![0, 1, 2, 3, 4],
            infl_bone: vec![0; 4],
            infl_weight: vec![1.0; 4],
            infl_local: vec![0.0; 12],
            pos: tri,
            nrm: vec![0.0; 12],
        })];
        let mut rig = SkirtRig::empty();
        rig.verts_part = vec![0];
        rig.verts_idx = vec![0];
        rig.sv_index = vec![((0, 0), 0)].into_iter().collect();
        rig.caps = vec![LegCapDef { bone: 0, len: 10.0 }];
        rig.leg_tris = vec![TriRef { part: 0, a: 1, b: 2, c: 3 }];
        rig.prev_disp = vec![0.0; 3];
        rig.orig_snap = vec![0.0; 3];
        rig.relax_a = vec![0.0; 3];
        rig.relax_b = vec![0.0; 3];
        let mut world = vec![0.0; 16];
        world[0] = 1.0;
        world[5] = 1.0;
        world[10] = 1.0;
        world[15] = 1.0;
        let mut diag = DepenDiag::default();
        let mut log = vec![];
        let touched = skirt_depen(&mut parts, &mut rig, &world, &mut diag, false, f64::NAN, &mut log);
        let z = parts[0].as_ref().unwrap().pos[2];
        assert!((z - 0.25).abs() < 1e-4, "z={z}");
        assert_eq!(touched, vec![0]);
    }
}
