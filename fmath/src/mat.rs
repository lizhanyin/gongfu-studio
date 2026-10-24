//! 标量数学内核，与 viewer.ts（three.js f64 语义）数值口径一致。
//! 矩阵为列主序 [f64; 16]（对应 THREE.Matrix4.elements），3×3 为行主序（对应 TS mat3*）。

pub type Mat16 = [f64; 16];

pub const IDENTITY: Mat16 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// 列主序 4×4 乘法 out = a·b（对应 THREE.Matrix4.multiplyMatrices(a, b)）
pub fn mat4_mul(a: &Mat16, b: &Mat16) -> Mat16 {
    let mut out = [0.0; 16];
    for c in 0..4 {
        for r in 0..4 {
            out[c * 4 + r] =
                a[r] * b[c * 4] + a[4 + r] * b[c * 4 + 1] + a[8 + r] * b[c * 4 + 2] + a[12 + r] * b[c * 4 + 3];
        }
    }
    out
}

/// 列主序 4×4 通用逆（余子式法；奇异返回 None）
pub fn mat4_inverse(m: &Mat16) -> Option<Mat16> {
    // 以行主序视角取元素：r 行 c 列 = m[c*4 + r]
    let at = |r: usize, c: usize| m[c * 4 + r];
    let mut inv = [[0.0f64; 4]; 4];
    for r in 0..4 {
        for cc in 0..4 {
            // 3×3 子式（去掉 r 行 cc 列）
            let rows: Vec<usize> = (0..4).filter(|x| *x != r).collect();
            let cols: Vec<usize> = (0..4).filter(|x| *x != cc).collect();
            let sub = |i: usize, j: usize| at(rows[i], cols[j]);
            let det3 = sub(0, 0) * (sub(1, 1) * sub(2, 2) - sub(1, 2) * sub(2, 1))
                - sub(0, 1) * (sub(1, 0) * sub(2, 2) - sub(1, 2) * sub(2, 0))
                + sub(0, 2) * (sub(1, 0) * sub(2, 1) - sub(1, 1) * sub(2, 0));
            inv[r][cc] = det3 * if (r + cc) % 2 == 0 { 1.0 } else { -1.0 };
        }
    }
    // det 沿第 0 行展开 = Σ at(0,j)·C[0][j]（inv[i][j] 存的是余子式 C[i][j]）
    let det = at(0, 0) * inv[0][0] + at(0, 1) * inv[0][1] + at(0, 2) * inv[0][2] + at(0, 3) * inv[0][3];
    if det.abs() < 1e-12 {
        return None;
    }
    let s = 1.0 / det;
    let mut out = [0.0f64; 16];
    for r in 0..4 {
        for cc in 0..4 {
            // inverse 元素 (r,cc) = C[cc][r]/det（伴随矩阵 = 余子式阵转置）
            out[cc * 4 + r] = inv[cc][r] * s;
        }
    }
    Some(out)
}

/// 3×3 行列式（行主序）
fn mat3_det(m: &[f64; 9]) -> f64 {
    m[0] * (m[4] * m[8] - m[5] * m[7])
        - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6])
}

fn mat3_inv(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = mat3_det(m);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ])
}

/// 极分解 Newton 迭代 M ← (M + M⁻ᵀ)/2，收敛到 H 的最近正交旋转因子（行主序）；
/// 退化（不可逆 / 反射）返回 None。对应 TS polarRotation。
pub fn polar_rotation(h: &[f64; 9]) -> Option<[f64; 9]> {
    let mut m = *h;
    for _ in 0..32 {
        let mi = mat3_inv(&m)?;
        let mut next = [0.0; 9];
        let mut delta: f64 = 0.0;
        for r in 0..3 {
            for c in 0..3 {
                let v = (m[r * 3 + c] + mi[c * 3 + r]) / 2.0;
                next[r * 3 + c] = v;
                delta = delta.max((v - m[r * 3 + c]).abs());
            }
        }
        m = next;
        if delta < 1e-10 {
            break;
        }
    }
    if mat3_det(&m) > 0.5 {
        Some(m)
    } else {
        None
    }
}

/// 四元数归一化（THREE 口径：长度 0 时除 1）
fn quat_normalize(q: [f64; 4]) -> [f64; 4] {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let s = if l == 0.0 { 1.0 } else { 1.0 / l };
    [q[0] * s, q[1] * s, q[2] * s, q[3] * s]
}

/// 单位向量对齐四元数（THREE.Quaternion.setFromUnitVectors 口径）
pub fn quat_from_unit_vectors(f: [f64; 3], t: [f64; 3]) -> [f64; 4] {
    const EPSILON: f64 = 2.220446049250313e-16; // Number.EPSILON
    let r = f[0] * t[0] + f[1] * t[1] + f[2] * t[2] + 1.0;
    let axis = if r < EPSILON {
        if f[0].abs() > f[2].abs() {
            [-f[1], f[0], 0.0]
        } else {
            [0.0, -f[2], f[2]]
        }
    } else {
        [
            f[1] * t[2] - f[2] * t[1],
            f[2] * t[0] - f[0] * t[2],
            f[0] * t[1] - f[1] * t[0],
        ]
    };
    quat_normalize([axis[0] * 0.5, axis[1] * 0.5, axis[2] * 0.5, r * 0.5])
}

/// 归一化四元数 → 列主序旋转 4×4（对应 Matrix4.makeRotationFromQuaternion）
pub fn quat_to_mat4(q: [f64; 4]) -> Mat16 {
    let [x, y, z, w] = quat_normalize(q);
    let x2 = x + x;
    let y2 = y + y;
    let z2 = z + z;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    [
        1.0 - (yy + zz),
        xy + wz,
        xz - wy,
        0.0,
        xy - wz,
        1.0 - (xx + zz),
        yz + wx,
        0.0,
        xz + wy,
        yz - wx,
        1.0 - (xx + yy),
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

/// 从扁平姿态组合单骨局部矩阵：T·R(Q) 再按镜像骨缩放 Scale(-1,-1,-1)。
/// 对应 worldsFromPoses 内的 compose + scale（scale 乘前三列 = 旋转块取反）。
pub fn local_from_pose(pose: &[f32], mirror: bool) -> Mat16 {
    let mut m = quat_to_mat4([pose[3] as f64, pose[4] as f64, pose[5] as f64, pose[6] as f64]);
    m[12] = pose[0] as f64;
    m[13] = pose[1] as f64;
    m[14] = pose[2] as f64;
    if mirror {
        for e in m.iter_mut().take(11) {
            *e = -*e;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    #[test]
    fn mat4_inverse_roundtrip() {
        // 平移×旋转矩阵的逆：M·M⁻¹ = I
        let m = mat4_mul(
            &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, -3.0, 2.0, 1.0],
            &[0.0, 1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        );
        let mi = mat4_inverse(&m).unwrap();
        let prod = mat4_mul(&m, &mi);
        for k in 0..16 {
            let expect = IDENTITY[k];
            assert!((prod[k] - expect).abs() < 1e-9, "prod[{k}]={}", prod[k]);
        }
        // 一般矩阵：M⁻¹⁻¹ = M
        let g: Mat16 = [1.0, 2.0, 0.5, 0.0, 0.3, 1.0, -0.2, 0.0, 0.0, 0.4, 1.0, 0.0, 3.0, -1.0, 2.0, 1.0];
        let gi = mat4_inverse(&g).unwrap();
        let back = mat4_inverse(&gi).unwrap();
        for k in 0..16 {
            assert!((back[k] - g[k]).abs() < 1e-9);
        }
        // 奇异矩阵
        let sing: Mat16 = [1.0; 16];
        assert!(mat4_inverse(&sing).is_none());
    }

    fn mat4_mul_identity() {
        let m = [
            1.0, 2.0, 3.0, 0.0, 4.0, 5.0, 6.0, 0.0, 7.0, 8.0, 9.0, 0.0, 10.0, 11.0, 12.0, 1.0,
        ];
        let out = mat4_mul(&m, &IDENTITY);
        assert!(out.iter().zip(m.iter()).all(|(a, b)| approx(*a, *b)));
    }

    #[test]
    fn mat4_mul_translation_compose() {
        let t1 = {
            let mut m = IDENTITY;
            m[12] = 5.0;
            m
        };
        let t2 = {
            let mut m = IDENTITY;
            m[13] = -3.0;
            m
        };
        let out = mat4_mul(&t1, &t2);
        assert!(approx(out[12], 5.0) && approx(out[13], -3.0));
    }

    #[test]
    fn polar_recovers_rotation() {
        // H = R·S（已知旋转 90° 绕 z + 对角缩放），极分解应还原 R
        let c = 0.0f64;
        let s = 1.0f64;
        let r = [c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0]; // 行主序
        let scale = [2.0, 0.5, 1.5];
        let mut h = [0.0; 9];
        for i in 0..3 {
            for j in 0..3 {
                h[i * 3 + j] = r[i * 3 + j] * scale[j];
            }
        }
        let rec = polar_rotation(&h).unwrap();
        for k in 0..9 {
            assert!((rec[k] - r[k]).abs() < 1e-9, "rec={rec:?}");
        }
    }

    #[test]
    fn quat_90deg_about_z() {
        let q = quat_from_unit_vectors([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let m = quat_to_mat4(q);
        // x 轴旋转到 y 轴：M·(1,0,0) = 列 0 = (0,1,0)
        assert!(approx(m[0], 0.0) && approx(m[1], 1.0) && approx(m[2], 0.0));
    }

    #[test]
    fn quat_rotation_applies_point() {
        // 绕 z 转 90°：(1,0,0) → (0,1,0)
        let q = [0.0, 0.0, 0.7071067811865476, 0.7071067811865476];
        let m = quat_to_mat4(q);
        let px = m[0] * 1.0 + m[4] * 0.0 + m[8] * 0.0 + m[12];
        let py = m[1] * 1.0 + m[5] * 0.0 + m[9] * 0.0 + m[13];
        assert!(approx(px, 0.0) && approx(py, 1.0));
    }

    #[test]
    fn local_from_pose_mirror_negates_rotation() {
        // 单位旋转 + 镜像 → 旋转块取反、平移保留
        let pose = [1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0];
        let m = local_from_pose(&pose, true);
        assert!(approx(m[0], -1.0) && approx(m[5], -1.0) && approx(m[10], -1.0));
        assert!(approx(m[12], 1.0) && approx(m[13], 2.0) && approx(m[14], 3.0));
    }
}
