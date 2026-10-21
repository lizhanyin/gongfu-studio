/**
 * 用真实 three.js 蒙皮管线验证 viewer 的"共识 + 离群覆盖"绑定：
 * 数据来自 Rust dump_hair05_json（head/hair05 顶点 + sta.FSKEL 首帧姿态）。
 * 对比 SkinnedMesh.applyBoneTransform 结果与游戏逐顶点 LBS（Σ w·M·local）。
 * 用法：node scripts/verify-hair05.mjs <hair05_check.json 路径>
 */
import * as THREE from "three";
import { readFileSync } from "node:fs";

const dumpPath = process.argv[2] ?? "../../target/hair05_check.json";
const dumpUrl = /^[a-zA-Z]:[\\/]/.test(dumpPath)
  ? new URL("file:///" + dumpPath.replace(/\\/g, "/"))
  : new URL(dumpPath, import.meta.url);
const dump = JSON.parse(readFileSync(dumpUrl, "utf8"));

// ---- 以下四个函数与 viewer.ts 保持逐行一致 ----
function mat3Det(m) {
  return (
    m[0] * (m[4] * m[8] - m[5] * m[7]) -
    m[1] * (m[3] * m[8] - m[5] * m[6]) +
    m[2] * (m[3] * m[7] - m[4] * m[6])
  );
}
function mat3Inv(m) {
  const det = mat3Det(m);
  if (Math.abs(det) < 1e-12) return null;
  const d = 1 / det;
  return [
    (m[4] * m[8] - m[5] * m[7]) * d,
    (m[2] * m[7] - m[1] * m[8]) * d,
    (m[1] * m[5] - m[2] * m[4]) * d,
    (m[5] * m[6] - m[3] * m[8]) * d,
    (m[0] * m[8] - m[2] * m[6]) * d,
    (m[2] * m[3] - m[0] * m[5]) * d,
    (m[3] * m[7] - m[4] * m[6]) * d,
    (m[1] * m[6] - m[0] * m[7]) * d,
    (m[0] * m[4] - m[1] * m[3]) * d,
  ];
}
function polarRotation(h) {
  let m = h.slice();
  for (let it = 0; it < 32; it++) {
    const mi = mat3Inv(m);
    if (!mi) return null;
    const next = new Array(9);
    let delta = 0;
    for (let r = 0; r < 3; r++) {
      for (let c = 0; c < 3; c++) {
        const v = (m[r * 3 + c] + mi[c * 3 + r]) / 2;
        next[r * 3 + c] = v;
        delta = Math.max(delta, Math.abs(v - m[r * 3 + c]));
      }
    }
    m = next;
    if (delta < 1e-10) break;
  }
  return mat3Det(m) > 0.5 ? m : null;
}
function fitBindMatrix(pairs) {
  const n = pairs.length;
  if (n === 0) return new THREE.Matrix4();
  const ca = [0, 0, 0];
  const cb = [0, 0, 0];
  for (const [a, b] of pairs) {
    for (let k = 0; k < 3; k++) {
      ca[k] += a[k] / n;
      cb[k] += b[k] / n;
    }
  }
  let R = null;
  if (n >= 3) {
    const h = new Array(9).fill(0);
    for (const [a, b] of pairs) {
      for (let r = 0; r < 3; r++) {
        for (let c = 0; c < 3; c++) h[r * 3 + c] += (b[r] - cb[r]) * (a[c] - ca[c]);
      }
    }
    R = polarRotation(h);
  } else if (n === 2) {
    const da = new THREE.Vector3(
      pairs[1][0][0] - pairs[0][0][0],
      pairs[1][0][1] - pairs[0][0][1],
      pairs[1][0][2] - pairs[0][0][2]
    ).normalize();
    const db = new THREE.Vector3(
      pairs[1][1][0] - pairs[0][1][0],
      pairs[1][1][1] - pairs[0][1][1],
      pairs[1][1][2] - pairs[0][1][2]
    ).normalize();
    const q = new THREE.Quaternion().setFromUnitVectors(da, db);
    const e = new THREE.Matrix4().makeRotationFromQuaternion(q).elements;
    R = [e[0], e[4], e[8], e[1], e[5], e[9], e[2], e[6], e[10]];
  }
  if (!R) R = [1, 0, 0, 0, 1, 0, 0, 0, 1];
  const t = [0, 1, 2].map(
    (r) => cb[r] - (R[r * 3] * ca[0] + R[r * 3 + 1] * ca[1] + R[r * 3 + 2] * ca[2])
  );
  return new THREE.Matrix4().set(
    R[0], R[1], R[2], t[0],
    R[3], R[4], R[5], t[1],
    R[6], R[7], R[8], t[2],
    0, 0, 0, 1
  );
}
function bindTransformDiff(a, b) {
  let dr = 0;
  for (let i = 0; i < 16; i++) {
    dr = Math.max(dr, Math.abs(a.elements[i] - b.elements[i]));
  }
  const ta = [a.elements[12], a.elements[13], a.elements[14]];
  const tb = [b.elements[12], b.elements[13], b.elements[14]];
  const dt = Math.hypot(ta[0] - tb[0], ta[1] - tb[1], ta[2] - tb[2]);
  return Math.max(dr, dt);
}

// ---- 骨架（等价 viewer.setFskel：世界矩阵 = 父 × T × R）----
const n = dump.bone_count;
const bones = Array.from({ length: n }, () => new THREE.Bone());
let root = null;
dump.parents.forEach((p, i) => {
  if (p >= 0 && p < n) bones[p].add(bones[i]);
  else root = bones[i];
});
for (let b = 0; b < n; b++) {
  const o = b * 7;
  bones[b].position.set(dump.poses[o], dump.poses[o + 1], dump.poses[o + 2]);
  bones[b].quaternion.set(
    dump.poses[o + 3],
    dump.poses[o + 4],
    dump.poses[o + 5],
    dump.poses[o + 6]
  );
}
const scene = new THREE.Scene();
scene.add(root);
root.updateMatrixWorld(true);

// ---- 每部件拟合（与 fillPartInverses 一致：≥3 exact 点，diff>0.1 覆盖）----
function buildSkeleton(verts, consensusFits) {
  const exact = Array.from({ length: n }, () => []);
  for (const v of verts) {
    for (const [bone, w, local] of v.influences) {
      if (v.influences.length === 1 || w >= 0.99) exact[bone].push([local, v.pos]);
    }
  }
  const inverses = Array.from({ length: n }, () => new THREE.Matrix4());
  for (let b = 0; b < n; b++) {
    let t = consensusFits[b];
    if (exact[b].length >= 3) {
      const own = fitBindMatrix(exact[b]);
      if (bindTransformDiff(own, t) > 0.1) t = own;
    }
    inverses[b].copy(t).invert();
  }
  return new THREE.Skeleton(bones, inverses);
}
// 共识：head 的拟合（exact 优先，纯 approx 骨也拟合；脚本仅装 head+hair05，
// 真实 viewer 装全套部件时骨骼 4/5 等还有身体部件的 exact 点）
const headExact = Array.from({ length: n }, () => []);
const headApprox = Array.from({ length: n }, () => []);
for (const v of dump.head) {
  for (const [bone, w, local] of v.influences) {
    if (v.influences.length === 1 || w >= 0.99) headExact[bone].push([local, v.pos]);
    else headApprox[bone].push([local, v.pos]);
  }
}
const consensusFits = Array.from({ length: n }, (_, b) => {
  const pool = headExact[b].length >= 3 ? headExact[b] : headExact[b].concat(headApprox[b]);
  return pool.length >= 2 ? fitBindMatrix(pool) : new THREE.Matrix4();
});

// ---- 用 SkinnedMesh.applyBoneTransform 走真实 CPU 蒙皮 ----
function skinnedPositions(verts) {
  const geom = new THREE.BufferGeometry();
  const total = verts.length;
  const pos = new Float32Array(total * 3);
  const idx = new Uint16Array(total * 4);
  const wgt = new Float32Array(total * 4);
  verts.forEach((v, i) => {
    pos.set(v.pos, i * 3);
    const sorted = [...v.influences].sort((a, b) => b[1] - a[1]).slice(0, 4);
    const sum = sorted.reduce((s, inf) => s + Math.max(inf[1], 0), 0) || 1;
    for (let k = 0; k < 4; k++) {
      idx[i * 4 + k] = sorted[k] ? sorted[k][0] : 0;
      wgt[i * 4 + k] = sorted[k] ? Math.max(sorted[k][1], 0) / sum : 0;
    }
  });
  geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
  geom.setAttribute("skinIndex", new THREE.Uint16BufferAttribute(idx, 4));
  geom.setAttribute("skinWeight", new THREE.Float32BufferAttribute(wgt, 4));
  const mesh = new THREE.SkinnedMesh(geom, new THREE.MeshBasicMaterial());
  mesh.bind(buildSkeleton(verts, consensusFits), new THREE.Matrix4());
  const out = [];
  const tmp = new THREE.Vector3();
  for (let i = 0; i < total; i++) {
    tmp.fromBufferAttribute(geom.getAttribute("position"), i);
    mesh.applyBoneTransform(i, tmp);
    out.push([tmp.x, tmp.y, tmp.z]);
  }
  return out;
}
// 游戏 LBS：Σ w·M_b·local_b
function gamePositions(verts) {
  const m = new THREE.Matrix4();
  return verts.map((v) => {
    const o = new THREE.Vector3();
    for (const [bone, w, local] of v.influences) {
      m.copy(bones[bone].matrixWorld);
      const x = new THREE.Vector3(...local).applyMatrix4(m);
      o.addScaledVector(x, w);
    }
    return [o.x, o.y, o.z];
  });
}

for (const [name, verts] of [
  ["girl_head", dump.head],
  ["hair05", dump.hair05],
]) {
  const skinned = skinnedPositions(verts);
  const game = gamePositions(verts);
  let maxErr = 0;
  let at = 0;
  skinned.forEach((p, i) => {
    const d = Math.hypot(p[0] - game[i][0], p[1] - game[i][1], p[2] - game[i][2]);
    if (d > maxErr) {
      maxErr = d;
      at = i;
    }
  });
  console.log(
    `${name.padEnd(10)} three.js 蒙皮 vs 游戏LBS: 最大误差 ${maxErr.toFixed(4)}（顶点 ${at}）`
  );
  if (name === "hair05" && maxErr > 0.01) {
    console.error("FAIL: hair05 未贴合");
    process.exit(1);
  }
}

// ---- 静态默认姿态（未载动作）：离群部件按共识绑定矩阵渲染 ----
// 与 viewer.applyStaticSkin 相同的公式：pos = Σ w·T_consensus,b·local_b
function staticPositions(verts, consFits) {
  return verts.map((v) => {
    const o = new THREE.Vector3();
    for (const [bone, w, local] of v.influences) {
      const x = new THREE.Vector3(...local).applyMatrix4(consFits[bone]);
      o.addScaledVector(x, w);
    }
    return [o.x, o.y, o.z];
  });
}
const centroid = (pts) => {
  const c = [0, 0, 0];
  for (const p of pts) for (let k = 0; k < 3; k++) c[k] += p[k] / pts.length;
  return c;
};
const headAuthored = dump.head.map((v) => v.pos);
const h5Authored = dump.hair05.map((v) => v.pos);
const headStatic = staticPositions(dump.head, consensusFits);
const h5Static = staticPositions(dump.hair05, consensusFits);
const cHeadA = centroid(headAuthored);
const cHeadS = centroid(headStatic);
const before = centroid(h5Authored).map((x, k) => x - cHeadA[k]);
const after = centroid(h5Static).map((x, k) => x - cHeadS[k]);
let headShift = 0;
headStatic.forEach((p, i) => {
  headShift = Math.max(headShift, Math.hypot(p[0] - headAuthored[i][0], p[1] - headAuthored[i][1], p[2] - headAuthored[i][2]));
});
let h5Shift = 0;
h5Static.forEach((p, i) => {
  h5Shift = Math.max(h5Shift, Math.hypot(p[0] - h5Authored[i][0], p[1] - h5Authored[i][1], p[2] - h5Authored[i][2]));
});
console.log(`静态对齐: 头部相对摆放位移 ${headShift.toFixed(3)}（应≈0，不受影响）`);
console.log(`静态对齐: hair05 最大修正量 ${h5Shift.toFixed(3)}（= 共识/authored 刚体差）`);
console.log(`hair05 质心相对头部 修正前 [x${before[0].toFixed(1)} y+${before[1].toFixed(1)} z+${before[2].toFixed(1)}]  修正后 [x${after[0].toFixed(1)} y+${after[1].toFixed(1)} z+${after[2].toFixed(1)}]`);
if (headShift > 1.5) {
  console.error("FAIL: 静态对齐不应移动头部");
  process.exit(1);
}
if (h5Shift < 1.0) {
  console.error("FAIL: hair05 静态修正未生效");
  process.exit(1);
}
console.log("PASS");
