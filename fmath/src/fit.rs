//! 点对 (骨骼局部, 世界) 的刚体最小二乘拟合（viewer.ts fitBindMatrix）：
//! H = Σ(b-cb)(a-ca)ᵀ 极分解求 R（n≥3），n=2 方向对齐，t = cb - R·ca。
//! 输入为扁平 xyz 对，输出列主序 4×4（对应 THREE.Matrix4.elements）。

use crate::mat::{polar_rotation, quat_from_unit_vectors, quat_to_mat4, Mat16, IDENTITY};

pub fn fit_bind_matrix(local: &[f32], world: &[f32]) -> Mat16 {
    let n = local.len() / 3;
    debug_assert_eq!(world.len() / 3, n);
    if n == 0 {
        return IDENTITY;
    }
    let l = |i: usize| [local[i * 3] as f64, local[i * 3 + 1] as f64, local[i * 3 + 2] as f64];
    let w = |i: usize| [world[i * 3] as f64, world[i * 3 + 1] as f64, world[i * 3 + 2] as f64];
    let mut ca = [0.0f64; 3];
    let mut cb = [0.0f64; 3];
    for i in 0..n {
        let a = l(i);
        let b = w(i);
        for k in 0..3 {
            ca[k] += a[k] / n as f64;
            cb[k] += b[k] / n as f64;
        }
    }
    // R 行主序
    let r_row: [f64; 9] = if n >= 3 {
        let mut h = [0.0f64; 9];
        for i in 0..n {
            let a = l(i);
            let b = w(i);
            for row in 0..3 {
                for col in 0..3 {
                    h[row * 3 + col] += (b[row] - cb[row]) * (a[col] - ca[col]);
                }
            }
        }
        polar_rotation(&h).unwrap_or([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]) // 退化：仅平移
    } else if n == 2 {
        // 两点：方向对齐（绕轴扭转未定，好过没有）
        let norm = |v: [f64; 3]| {
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            let s = if len == 0.0 { 1.0 } else { 1.0 / len };
            [v[0] * s, v[1] * s, v[2] * s]
        };
        let da = norm([l(1)[0] - l(0)[0], l(1)[1] - l(0)[1], l(1)[2] - l(0)[2]]);
        let db = norm([w(1)[0] - w(0)[0], w(1)[1] - w(0)[1], w(1)[2] - w(0)[2]]);
        let e = quat_to_mat4(quat_from_unit_vectors(da, db));
        // 列主序 → 行主序
        [
            e[0], e[4], e[8], e[1], e[5], e[9], e[2], e[6], e[10],
        ]
    } else {
        [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
    };
    let mut out = [0.0f64; 16];
    for col in 0..3 {
        for row in 0..3 {
            out[col * 4 + row] = r_row[row * 3 + col];
        }
    }
    for r in 0..3 {
        out[12 + r] = cb[r]
            - (r_row[r * 3] * ca[0] + r_row[r * 3 + 1] * ca[1] + r_row[r * 3 + 2] * ca[2]);
    }
    out[15] = 1.0;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_identity() {
        assert_eq!(fit_bind_matrix(&[], &[]), IDENTITY);
    }

    #[test]
    fn recovers_rigid_transform() {
        // 旋转 90° 绕 z + 平移 (5,-2,8)：世界点 = R·局部 + t
        let local: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, -3.0, 4.0];
        let rot = |p: [f64; 3]| [-p[1], p[0], p[2]];
        let world: Vec<f32> = local
            .chunks(3)
            .flat_map(|c| {
                let p = [c[0] as f64, c[1] as f64, c[2] as f64];
                let r = rot(p);
                [r[0] + 5.0, r[1] - 2.0, r[2] + 8.0]
            })
            .map(|v| v as f32)
            .collect();
        let m = fit_bind_matrix(&local, &world);
        let expect: Mat16 = [
            0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, -2.0, 8.0, 1.0,
        ];
        for k in 0..16 {
            assert!((m[k] - expect[k]).abs() < 1e-4, "m={m:?}");
        }
    }

    #[test]
    fn two_points_align_direction() {
        // 两点局部沿 x，世界沿 y：拟合应把 x 轴映到 y 轴
        let local: Vec<f32> = vec![0.0, 0.0, 0.0, 4.0, 0.0, 0.0];
        let world: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 5.0, 1.0];
        let m = fit_bind_matrix(&local, &world);
        // M·(4,0,0) 应接近 (1,5,1)
        let px = m[0] * 4.0 + m[12];
        let py = m[1] * 4.0 + m[13];
        let pz = m[2] * 4.0 + m[14];
        assert!((px - 1.0).abs() < 1e-5 && (py - 5.0).abs() < 1e-5 && (pz - 1.0).abs() < 1e-5);
    }
}
