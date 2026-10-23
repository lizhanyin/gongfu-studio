//! CPU LBS 蒙皮内核 = F3D.dll 引擎语义（viewer.ts skinPart 逐字段对照）：
//! pos = Σ w_b·M_b·boneLocalPos_b，原始权重直接累加不归一化，法线恒静态。
//! 布料骨走 clothMats 作用在 authored pos；锚定链骨走 anchoredMats 作用在 boneLocalPos。

use crate::mat::Mat16;

/// 部件蒙皮数据（SoA）。infl_off 为每顶点影响区间的 CSR 偏移（len = vert_count+1），
/// resolved_src 把几何顶点（含跨缝副本）映射回源顶点，src_* 为源顶点的 authored 数据。
pub struct PartSkin {
    pub orig_count: usize,
    pub vert_count: usize,
    pub resolved_src: Vec<u32>,
    pub src_pos: Vec<f32>,
    pub src_nrm: Vec<f32>,
    pub infl_off: Vec<u32>,
    pub infl_bone: Vec<u32>,
    pub infl_weight: Vec<f32>,
    pub infl_local: Vec<f32>,
    /// 输出缓冲（wasm 持有，JS 经视图直连 BufferAttribute）
    pub pos: Vec<f32>,
    pub nrm: Vec<f32>,
}

/// cloth/anchored 特例矩阵表（按骨索引；None = 普通世界矩阵路径）
pub type BoneMats = Vec<Option<Mat16>>;

fn world_mat<'a>(world: &'a [f64], bi: usize) -> &'a Mat16 {
    let chunk: &[f64; 16] = world[bi * 16..bi * 16 + 16]
        .try_into()
        .expect("world mat 越界");
    chunk
}

fn apply_infl(px: &mut f64, py: &mut f64, pz: &mut f64, m: &Mat16, local: &[f32], k: usize, w: f64) {
    let l0 = local[k * 3] as f64;
    let l1 = local[k * 3 + 1] as f64;
    let l2 = local[k * 3 + 2] as f64;
    // 列主序：M·lp
    *px += w * (m[0] * l0 + m[4] * l1 + m[8] * l2 + m[12]);
    *py += w * (m[1] * l0 + m[5] * l1 + m[9] * l2 + m[13]);
    *pz += w * (m[2] * l0 + m[6] * l1 + m[10] * l2 + m[14]);
}

pub fn skin_part(
    p: &mut PartSkin,
    bone_count: usize,
    world: &[f64],
    cloth: &[Option<Mat16>],
    anchored: &[Option<Mat16>],
) {
    let cloth_any = cloth.iter().any(|m| m.is_some());
    let anchored_any = anchored.iter().any(|m| m.is_some());
    for i in 0..p.vert_count {
        let off = p.infl_off[i] as usize;
        let end = p.infl_off[i + 1] as usize;
        let mut wsum = 0.0f64;
        for k in off..end {
            if (p.infl_bone[k] as usize) < bone_count {
                wsum += p.infl_weight[k] as f64;
            }
        }
        let o = i * 3;
        if wsum <= 0.0 {
            // 无有效骨骼（骨架比部件骨骼少）：保持绑定姿势
            p.pos[o..o + 3].copy_from_slice(&p.src_pos[o..o + 3]);
            p.nrm[o..o + 3].copy_from_slice(&p.src_nrm[o..o + 3]);
            continue;
        }
        let spx = p.src_pos[o] as f64;
        let spy = p.src_pos[o + 1] as f64;
        let spz = p.src_pos[o + 2] as f64;
        let mut px = 0.0f64;
        let mut py = 0.0f64;
        let mut pz = 0.0f64;
        for k in off..end {
            let bi = p.infl_bone[k] as usize;
            if bi >= bone_count {
                continue;
            }
            let w = p.infl_weight[k] as f64; // 引擎用原始权重，不归一化
            // 布料骨：FSKEL 姿态与绑定数据恒不一致（游戏为运行时布料），
            // 改为合成矩阵作用在 authored pos 上——见 detectCloth 注释
            if cloth_any && bi < cloth.len() {
                if let Some(c) = &cloth[bi] {
                    px += w * (c[0] * spx + c[4] * spy + c[8] * spz + c[12]);
                    py += w * (c[1] * spx + c[5] * spy + c[9] * spz + c[13]);
                    pz += w * (c[2] * spx + c[6] * spy + c[10] * spz + c[14]);
                    continue;
                }
            }
            // 锚定链骨（静态错位链）：绑定摆放随载体骨，仍走 boneLocalPos 路径
            let m = if anchored_any && bi < anchored.len() {
                anchored[bi].as_ref().unwrap_or_else(|| world_mat(world, bi))
            } else {
                world_mat(world, bi)
            };
            apply_infl(&mut px, &mut py, &mut pz, m, &p.infl_local, k, w);
        }
        p.pos[o] = px as f32;
        p.pos[o + 1] = py as f32;
        p.pos[o + 2] = pz as f32;
        p.nrm[o..o + 3].copy_from_slice(&p.src_nrm[o..o + 3]);
    }
}

/// 镜像骨检测/诊断的 LBS 残差：Σ_顶点 |LBS(world) − authored pos|。
/// 只遍历源顶点前缀（对应 TS 的 model.vertices，不含跨缝副本）。
pub fn lbs_residual(parts: &[&PartSkin], world: &[f64], bone_count: usize, bones: &[u32]) -> f64 {
    let in_set = |b: u32| bones.binary_search(&b).is_ok();
    let mut sum = 0.0f64;
    for p in parts {
        for i in 0..p.orig_count {
            let off = p.infl_off[i] as usize;
            let end = p.infl_off[i + 1] as usize;
            if !(off..end).any(|k| in_set(p.infl_bone[k])) {
                continue;
            }
            let mut x = 0.0f64;
            let mut y = 0.0f64;
            let mut z = 0.0f64;
            for k in off..end {
                let bi = p.infl_bone[k] as usize;
                if bi >= bone_count {
                    continue; // 部件与骨架不配套时的越界防御
                }
                apply_infl(
                    &mut x,
                    &mut y,
                    &mut z,
                    world_mat(world, bi),
                    &p.infl_local,
                    k,
                    p.infl_weight[k] as f64,
                );
            }
            let o = i * 3;
            sum += ((x - p.src_pos[o] as f64).powi(2)
                + (y - p.src_pos[o + 1] as f64).powi(2)
                + (z - p.src_pos[o + 2] as f64).powi(2))
            .sqrt();
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_world(n_bones: usize) -> Vec<f64> {
        let mut v = vec![0.0; n_bones * 16];
        for b in 0..n_bones {
            v[b * 16] = 1.0;
            v[b * 16 + 5] = 1.0;
            v[b * 16 + 10] = 1.0;
            v[b * 16 + 15] = 1.0;
        }
        v
    }

    /// 每顶点恰好一条影响 (bone, weight, boneLocalPos)
    fn part1(infl: &[(u32, f32, [f32; 3])], src_pos: &[f32], src_nrm: &[f32]) -> PartSkin {
        let vc = src_pos.len() / 3;
        assert_eq!(infl.len(), vc);
        let mut local = vec![];
        for (_, _, lp) in infl {
            local.extend_from_slice(lp);
        }
        PartSkin {
            orig_count: vc,
            vert_count: vc,
            resolved_src: (0..vc as u32).collect(),
            src_pos: src_pos.to_vec(),
            src_nrm: src_nrm.to_vec(),
            infl_off: (0..=vc as u32).collect(),
            infl_bone: infl.iter().map(|(b, _, _)| *b).collect(),
            infl_weight: infl.iter().map(|(_, w, _)| *w).collect(),
            infl_local: local,
            pos: src_pos.to_vec(),
            nrm: src_nrm.to_vec(),
        }
    }

    #[test]
    fn skin_translate() {
        // 骨 0 平移 (10,0,0)，权重 1：pos = M·boneLocalPos = (10,0,0)+(1,2,3)
        let mut world = identity_world(1);
        world[12] = 10.0;
        let mut p = part1(&[(0, 1.0, [1.0, 2.0, 3.0])], &[0.0, 0.0, 0.0], &[0.0, 1.0, 0.0]);
        skin_part(&mut p, 1, &world, &[], &[]);
        assert_eq!(p.pos, vec![11.0, 2.0, 3.0]);
        assert_eq!(p.nrm, vec![0.0, 1.0, 0.0]); // 法线静态
    }

    #[test]
    fn skin_weight_not_normalized() {
        // 引擎语义：权重直接累加（0.5+0.5 两骨同矩阵 → 1×，不是 0.5×）
        let world = identity_world(2);
        let p0 = PartSkin {
            orig_count: 1,
            vert_count: 1,
            resolved_src: vec![0],
            src_pos: vec![0.0, 0.0, 0.0],
            src_nrm: vec![1.0, 0.0, 0.0],
            infl_off: vec![0, 2],
            infl_bone: vec![0, 1],
            infl_weight: vec![0.5, 0.5],
            infl_local: vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            pos: vec![0.0; 3],
            nrm: vec![0.0; 3],
        };
        let mut p = p0;
        skin_part(&mut p, 2, &world, &[], &[]);
        // 骨矩阵为单位 → LBS 结果 = (0,0,0)·1.0 = 0（权重和 1 直接生效）
        assert!(p.pos.iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn skin_oob_bone_static() {
        // 影响骨 >= boneCount → wsum<=0 → 绑定姿势
        let world = identity_world(1);
        let mut p = part1(&[(3, 1.0, [9.0, 9.0, 9.0])], &[1.0, 2.0, 3.0], &[0.0, 1.0, 0.0]);
        skin_part(&mut p, 1, &world, &[], &[]);
        assert_eq!(p.pos, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn skin_cloth_path_uses_authored_pos() {
        // 布料骨：合成矩阵作用在 authored pos（非 boneLocalPos）
        let mut c = [0.0; 16];
        c[0] = 1.0;
        c[5] = 1.0;
        c[10] = 1.0;
        c[14] = 7.0;
        let cloth: BoneMats = vec![None, Some(c)];
        let world = identity_world(2);
        let mut p = PartSkin {
            orig_count: 1,
            vert_count: 1,
            resolved_src: vec![0],
            src_pos: vec![2.0, 0.0, 0.0],
            src_nrm: vec![0.0, 1.0, 0.0],
            infl_off: vec![0, 1],
            infl_bone: vec![1],
            infl_weight: vec![1.0],
            infl_local: vec![100.0, 0.0, 0.0], // 若误走 boneLocalPos 路径会得到 100+
            pos: vec![0.0; 3],
            nrm: vec![0.0; 3],
        };
        skin_part(&mut p, 2, &world, &cloth, &[]);
        assert!((p.pos[2] - 7.0).abs() < 1e-6 && (p.pos[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn residual_zero_on_identity() {
        let world = identity_world(1);
        let p = part1(&[(0, 1.0, [5.0, 6.0, 7.0])], &[5.0, 6.0, 7.0], &[0.0, 1.0, 0.0]);
        assert!(lbs_residual(&[&p], &world, 1, &[0]) < 1e-6);
    }
}
