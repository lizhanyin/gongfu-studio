//! FAK 顶点动画逐帧插值（viewer.ts applyFakFrame）：
//! 帧数据布局每顶点 8×f32 = pos3 + nrm3 + uv2，位置/法线 lerp，法线写入时归一化。

pub struct FakState {
    /// 全部帧拼接（frame_count × vc × 8）
    pub frames: Vec<f32>,
    pub frame_count: usize,
    pub vc: usize,
    pub pos: Vec<f32>,
    pub nrm: Vec<f32>,
}

impl FakState {
    pub fn new(frames: Vec<f32>, frame_count: usize, vc: usize) -> Self {
        FakState {
            frames,
            frame_count,
            vc,
            pos: vec![0.0; vc * 3],
            nrm: vec![0.0; vc * 3],
        }
    }

    /// 应用第 i0 帧；alpha>0 且 i1 有效时对 i1 插值（对应 TS 的 d0/d1 判空逻辑）
    pub fn frame(&mut self, i0: i32, i1: i32, alpha: f64) {
        if i0 < 0 || (i0 as usize) >= self.frame_count {
            return;
        }
        let i0 = i0 as usize;
        let stride = self.vc * 8;
        let d0 = i0 * stride;
        let d1 = if alpha > 0.0 && i1 != i0 as i32 && i1 >= 0 && (i1 as usize) < self.frame_count {
            Some(i1 as usize * stride)
        } else {
            None
        };
        let a = alpha;
        for i in 0..self.vc {
            let b = i * 8;
            let o = i * 3;
            let (nx, ny, nz) = match d1 {
                Some(d1) => {
                    self.pos[o] = lerp(self.frames[d0 + b], self.frames[d1 + b], a);
                    self.pos[o + 1] = lerp(self.frames[d0 + b + 1], self.frames[d1 + b + 1], a);
                    self.pos[o + 2] = lerp(self.frames[d0 + b + 2], self.frames[d1 + b + 2], a);
                    (
                        lerp(self.frames[d0 + b + 3], self.frames[d1 + b + 3], a),
                        lerp(self.frames[d0 + b + 4], self.frames[d1 + b + 4], a),
                        lerp(self.frames[d0 + b + 5], self.frames[d1 + b + 5], a),
                    )
                }
                None => {
                    self.pos[o] = self.frames[d0 + b];
                    self.pos[o + 1] = self.frames[d0 + b + 1];
                    self.pos[o + 2] = self.frames[d0 + b + 2];
                    (
                        self.frames[d0 + b + 3],
                        self.frames[d0 + b + 4],
                        self.frames[d0 + b + 5],
                    )
                }
            };
            // 文件内法线未归一化（长度 2.1~3.0），写入时归一化保证光照正确
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if len > 1e-6 {
                self.nrm[o] = (nx / len) as f32;
                self.nrm[o + 1] = (ny / len) as f32;
                self.nrm[o + 2] = (nz / len) as f32;
            } else {
                self.nrm[o] = nx as f32;
                self.nrm[o + 1] = ny as f32;
                self.nrm[o + 2] = nz as f32;
            }
        }
    }
}

fn lerp(a: f32, b: f32, t: f64) -> f32 {
    (a as f64 + (b as f64 - a as f64) * t) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_direct_copy_and_normalize() {
        // 1 帧 1 顶点：pos=(1,2,3) nrm=(0,0,5)（未归一化）
        let frames = vec![1.0, 2.0, 3.0, 0.0, 0.0, 5.0, 0.5, 0.5];
        let mut f = FakState::new(frames, 1, 1);
        f.frame(0, 0, 0.0);
        assert_eq!(f.pos, vec![1.0, 2.0, 3.0]);
        assert_eq!(f.nrm, vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn frame_interpolation() {
        // 2 帧各 1 顶点，alpha=0.5
        let frames = vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, //
            10.0, 20.0, 30.0, 0.0, 0.0, 1.0, 0.0, 0.0,
        ];
        let mut f = FakState::new(frames, 2, 1);
        f.frame(0, 1, 0.5);
        assert!((f.pos[0] - 5.0).abs() < 1e-6);
        assert!((f.pos[1] - 10.0).abs() < 1e-6);
        assert!((f.nrm[0] - 0.70710678).abs() < 1e-6);
    }

    #[test]
    fn frame_oob_noop() {
        let frames = vec![1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut f = FakState::new(frames, 1, 1);
        f.frame(5, 5, 0.0); // 越界：不崩溃不改写
        assert_eq!(f.pos, vec![0.0; 3]);
    }
}
