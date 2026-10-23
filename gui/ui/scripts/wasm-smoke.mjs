/**
 * fmath wasm 产物冒烟测试（Node 直连，不经 Vite/Tauri）：
 *   node scripts/wasm-smoke.mjs
 * 校验 wasm-bindgen 胶水层（切片转换/视图/返回值）与核心数值：
 * 蒙皮 LBS、绑定拟合、姿态世界矩阵（含镜像）、裙摆去穿透、LBS 残差。
 * 前置：npm run wasm（先构建 src/wasm 产物）。
 */
import { readFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const { initSync, Gfm } = await import(pathToFileURL(join(here, "../src/wasm/fmath.js")).href);
initSync(readFileSync(join(here, "../src/wasm/fmath_bg.wasm")));

let failed = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`ok   ${name}`);
  } else {
    failed++;
    console.error(`FAIL ${name} ${detail}`);
  }
}
const near = (a, b, eps = 1e-4) => Math.abs(a - b) < eps;

// ---- 1. 蒙皮 LBS：骨 0 平移 (10,0,0)，权重 1，boneLocalPos=(1,2,3)/(4,5,6) ----
{
  const g = new Gfm();
  const id = g.register_part(
    new Float32Array([0, 0, 0, 0, 0, 0]), // src_pos（authored，未直接用）
    new Float32Array([0, 1, 0, 0, 1, 0]),
    new Uint32Array([0, 1]),
    2,
    new Uint32Array([0, 1, 2]),
    new Uint32Array([0, 0]),
    new Float32Array([1, 1]),
    new Float32Array([1, 2, 3, 4, 5, 6])
  );
  const world = new Float64Array(16);
  world[0] = 1; world[5] = 1; world[10] = 1; world[12] = 10; world[15] = 1;
  g.set_world_mats(world, 1);
  g.skin_part(id);
  const pos = g.pos_view(id);
  check("skin lbs translate", near(pos[0], 11) && near(pos[1], 2) && near(pos[3], 14) && near(pos[5], 6), JSON.stringify(Array.from(pos)));
  // 法线静态（src_nrm 回写）
  const nrm = g.nrm_view(id);
  check("skin normal static", nrm[0] === 0 && nrm[1] === 1);
  const v0 = g.views_version();
  const id2 = g.register_part(
    new Float32Array(3), new Float32Array(3), new Uint32Array([0]), 1,
    new Uint32Array([0, 1]), new Uint32Array([0]), new Float32Array([1]), new Float32Array(3)
  );
  check("views_version bumps on register", g.views_version() !== v0);
  g.unregister_part(id2);
  // 注销后旧视图可能已分离（内存增长）——重取视图仍可用
  check("pos_view after unregister", g.pos_view(id) !== undefined);
}

// ---- 2. 绑定拟合：旋转 90° 绕 z + 平移 (5,-2,8) ----
{
  const g = new Gfm();
  const local = [1, 0, 0, 0, 1, 0, 0, 0, 1, 2, -3, 4];
  // world = R·local + t，R = 绕 z 90°（x→y）
  const rot = (p) => [-p[1], p[0], p[2]];
  const worldPts = [];
  for (let i = 0; i < 4; i++) {
    const p = [local[i * 3], local[i * 3 + 1], local[i * 3 + 2]];
    const r = rot(p);
    worldPts.push(r[0] + 5, r[1] - 2, r[2] + 8);
  }
  const m = g.fit_bind_matrix(new Float32Array(local), new Float32Array(worldPts));
  check("fit rigid rotation", near(m[0], 0, 1e-3) && near(m[1], 1, 1e-3) && near(m[4], -1, 1e-3), Array.from(m).map((v) => v.toFixed(3)).join(","));
  check("fit rigid translation", near(m[12], 5, 1e-3) && near(m[13], -2, 1e-3) && near(m[14], 8, 1e-3));
}

// ---- 3. 姿态世界矩阵：2 骨链 + 镜像传播 ----
{
  const g = new Gfm();
  const poses = new Float32Array([
    0, 0, 0, 0, 0, 0, 1, // 骨 0 单位
    10, 0, 0, 0, 0, 0, 1, // 骨 1 父=0 平移 (10,0,0)
  ]);
  g.set_poses(poses, 2, new Int32Array([-1, 0]));
  let w = g.worlds_from_poses(0);
  check("worlds chain", near(w[16 + 12], 10) && near(w[16], 1), `t=${w[28]}`);
  g.set_mirror(new Uint32Array([0]));
  w = g.worlds_from_poses(0);
  check("worlds mirror propagates", near(w[16 + 12], -10) && near(w[16], -1), `t=${w[28]}`);
}

// ---- 4. LBS 残差：identity 姿态 + 一致的 authored → ≈0 ----
{
  const g = new Gfm();
  g.set_poses(new Float32Array([0, 0, 0, 0, 0, 0, 1]), 1, new Int32Array([-1]));
  g.register_part(
    new Float32Array([5, 6, 7]), // authored = boneLocalPos（identity 下一致）
    new Float32Array(3),
    new Uint32Array([0]),
    1,
    new Uint32Array([0, 1]),
    new Uint32Array([0]),
    new Float32Array([1]),
    new Float32Array([5, 6, 7])
  );
  const r = g.lbs_residual(0, 0, 0, new Uint32Array([0]));
  check("lbs residual identity ~0", r < 1e-4, `r=${r}`);
}

// ---- 5. 裙摆去穿透：胶囊沿 +X，腿网格面过顶点（dm=0）→ 推出 0.15 ----
{
  const g = new Gfm();
  const tri = [5, 0, 0.1, 2, -3, 0.1, 8, -3, 0.1, 5, 3, 0.1];
  const id = g.register_part(
    new Float32Array(tri),
    new Float32Array(12),
    new Uint32Array([0, 1, 2, 3]),
    4,
    new Uint32Array([0, 1, 2, 3, 4]),
    new Uint32Array([0, 0, 0, 0]),
    new Float32Array([1, 1, 1, 1]),
    new Float32Array(12)
  );
  const world = new Float64Array(16);
  world[0] = 1; world[5] = 1; world[10] = 1; world[15] = 1;
  g.set_world_mats(world, 1);
  g.setup_skirt(
    new Uint32Array([id, 0]), // 裙顶点 = (part 0, vert 0)
    new Uint32Array(0), // 无裙三角形
    new Uint32Array([id, 1, 2, 3]), // 腿网格三角形
    new Uint32Array(0),
    new Uint32Array([0, 0]), // 无邻接
    new Uint32Array(0),
    new Float32Array([0, 10]) // 胶囊：骨 0，段长 10
  );
  const touched = g.skirt_depen(NaN);
  const z = g.pos_view(id)[2];
  check("depen push to mesh margin", near(z, 0.25, 1e-4), `z=${z}`);
  check("depen touched part", Array.from(touched).includes(id), JSON.stringify(Array.from(touched)));
}

console.log(failed ? `\n${failed} FAILED` : "\nall smoke checks passed");
process.exit(failed ? 1 : 0);
