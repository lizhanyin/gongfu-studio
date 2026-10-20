/**
 * 复刻 viewer.detectCloth/updateClothMats/skinPart 布料分支，
 * 用 girl_Lower + girl.FSKEL 前 90 帧逐帧数值验证：
 *   1) 帧 0 各布料骨簇应贴回 authored（无尖刺、无错位）
 *   2) 逐帧各顶点相对骨盆携带位置的偏移应有界
 *   3) 面板方位不翻转：簇质心相对骨盆的方位角与绑定方位角一致
 * 用法：node scripts/verify-skirt-cloth.mjs [skirt_check.json 路径]
 */
import * as THREE from "three";
import { readFileSync } from "node:fs";

const dumpPath = process.argv[2] ?? "G:/pojie/ai/Gongfu-studio/target/skirt_check.json";
const dump = JSON.parse(readFileSync(dumpPath, "utf8"));
const N = dump.bone_count;
const parents = dump.parents;
const poses = Float32Array.from(dump.poses); // 帧×骨×7
const frames = poses.length / (N * 7);

// ---- 与 viewer 相同的世界矩阵组合 ----
function worldsAt(f) {
  const out = Array.from({ length: N }, () => new THREE.Matrix4());
  const base = Math.min(Math.max(f, 0), frames - 1) * N * 7;
  for (let b = 0; b < N; b++) {
    const o = base + b * 7;
    const q = new THREE.Quaternion(poses[o + 3], poses[o + 4], poses[o + 5], poses[o + 6]).normalize();
    out[b].compose(
      new THREE.Vector3(poses[o], poses[o + 1], poses[o + 2]),
      q,
      new THREE.Vector3(1, 1, 1)
    );
    const p = parents[b];
    if (p >= 0 && p < b) out[b].premultiply(out[p]);
  }
  return out;
}

// ---- Kabsch 拟合（与 viewer.fitBindMatrix 的 ≥3 点路径一致）----
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
    (m[4] * m[8] - m[5] * m[7]) * d, (m[2] * m[7] - m[1] * m[8]) * d, (m[1] * m[5] - m[2] * m[4]) * d,
    (m[5] * m[6] - m[3] * m[8]) * d, (m[0] * m[8] - m[2] * m[6]) * d, (m[2] * m[3] - m[0] * m[5]) * d,
    (m[3] * m[7] - m[4] * m[6]) * d, (m[1] * m[6] - m[0] * m[7]) * d, (m[0] * m[4] - m[1] * m[3]) * d,
  ];
}
function polarRotation(h) {
  let m = h.slice();
  for (let it = 0; it < 32; it++) {
    const mi = mat3Inv(m);
    if (!mi) return null;
    const next = new Array(9);
    let delta = 0;
    for (let r = 0; r < 3; r++)
      for (let c = 0; c < 3; c++) {
        const v = (m[r * 3 + c] + mi[c * 3 + r]) / 2;
        next[r * 3 + c] = v;
        delta = Math.max(delta, Math.abs(v - m[r * 3 + c]));
      }
    m = next;
    if (delta < 1e-10) break;
  }
  return mat3Det(m) > 0.5 ? m : null;
}
function fitBindMatrix(pairs) {
  const n = pairs.length;
  if (n === 0) return new THREE.Matrix4();
  const ca = [0, 0, 0], cb = [0, 0, 0];
  for (const [a, b] of pairs) for (let k = 0; k < 3; k++) { ca[k] += a[k] / n; cb[k] += b[k] / n; }
  let R;
  if (n >= 3) {
    const h = new Array(9).fill(0);
    for (const [a, b] of pairs)
      for (let r = 0; r < 3; r++) for (let c = 0; c < 3; c++) h[r * 3 + c] += (b[r] - cb[r]) * (a[c] - ca[c]);
    R = polarRotation(h);
  }
  if (!R) R = [1, 0, 0, 0, 1, 0, 0, 0, 1];
  const t = [0, 1, 2].map((r) => cb[r] - (R[r * 3] * ca[0] + R[r * 3 + 1] * ca[1] + R[r * 3 + 2] * ca[2]));
  return new THREE.Matrix4().set(R[0], R[1], R[2], t[0], R[3], R[4], R[5], t[1], R[6], R[7], R[8], t[2], 0, 0, 0, 1);
}

// ---- detectCloth 复刻 ----
const isBoneHelper = (b) => /^Bone\d+$/.test(dump.bone_names[b] ?? "");
const pairsOf = Array.from({ length: N }, () => []);
const vertsOf = Array.from({ length: N }, () => []);
for (const v of dump.lower) {
  for (const inf of v.influences) {
    const b = inf[0];
    if (b >= N) continue;
    if (v.influences.length === 1 || inf[1] >= 0.99) pairsOf[b].push([inf[2], v.pos]);
    vertsOf[b].push(v.pos);
  }
}
const clothBones = new Set();
for (let b = 0; b < N; b++) if (isBoneHelper(b) && vertsOf[b].length > 0) clothBones.add(b);
console.log("布料骨:", [...clothBones].map((b) => `0x${b.toString(16)}(${dump.bone_names[b]})`).join(" "));

// 载体 = 布料骨父的众数
const votes = new Map();
for (const b of clothBones) {
  const p = parents[b];
  if (p >= 0 && p < N && !clothBones.has(p)) votes.set(p, (votes.get(p) ?? 0) + 1);
}
const carrier = [...votes.entries()].sort((a, b) => b[1] - a[1])[0][0];
console.log("载体骨:", `0x${carrier.toString(16)}(${dump.bone_names[carrier]})`, "点数", pairsOf[carrier].length);
const tpInv = fitBindMatrix(pairsOf[carrier]).invert();

// 模拟状态
const sims = [];
for (const b of clothBones) {
  const sorted = [...vertsOf[b]].sort((a, c) => c[1] - a[1]);
  const topN = Math.max(1, Math.ceil(sorted.length * 0.4));
  const mean = (list) => list.reduce((acc, p) => [acc[0] + p[0] / list.length, acc[1] + p[1] / list.length, acc[2] + p[2] / list.length], [0, 0, 0]);
  sims.push({
    bone: b,
    anchor: new THREE.Vector3(...mean(sorted.slice(0, topN))).applyMatrix4(tpInv),
    target: new THREE.Vector3(...mean(sorted)).applyMatrix4(tpInv),
    pos: new THREE.Vector3(NaN, NaN, NaN),
    prev: new THREE.Vector3(NaN, NaN, NaN),
  });
}
// 腿部碰撞
const colliders = [];
for (let b = 0; b < N; b++) {
  if (!/Thigh|Calf|Foot/.test(dump.bone_names[b])) continue;
  let len = 12;
  for (let c = 0; c < N; c++) {
    if (parents[c] === b) {
      const o = c * 7;
      len = Math.hypot(poses[o], poses[o + 1], poses[o + 2]);
      break;
    }
  }
  colliders.push({ bone: b, len });
}
console.log("碰撞骨:", colliders.map((c) => `0x${c.bone.toString(16)}(${dump.bone_names[c.bone]})len=${c.len.toFixed(1)}`).join(" "));

const K = 260, DAMP = 0.9, G = 300;
const clothMats = new Map();
for (const b of clothBones) clothMats.set(b, new THREE.Matrix4());

function updateClothMats(abs, world) {
  const carry = new THREE.Matrix4().multiplyMatrices(world[carrier], tpInv);
  let dt = 0, seek = false;
  let lastAbs = updateClothMats.lastAbs;
  if (!Number.isFinite(abs) || !Number.isFinite(lastAbs)) seek = true;
  else {
    const d = abs - lastAbs;
    if (d < -0.01 || d > 3) seek = true;
    else dt = Math.min(Math.max(d, 0), 2) / 30;
  }
  updateClothMats.lastAbs = abs;
  for (const s of sims) {
    const anchorW = s.anchor.clone().applyMatrix4(carry);
    const targetW = s.target.clone().applyMatrix4(carry);
    if (seek || !Number.isFinite(s.pos.x)) {
      s.pos.copy(targetW);
      s.prev.copy(targetW);
    } else if (dt > 0) {
      const vel = s.pos.clone().sub(s.prev).multiplyScalar(DAMP);
      const vmax = 6 * dt * 30;
      if (vel.length() > vmax) vel.setLength(vmax);
      s.prev.copy(s.pos);
      s.pos.add(vel);
      s.pos.y -= G * dt * dt;
      s.pos.lerp(targetW, Math.min(1, K * dt * dt));
    }
    for (let it = 0; it < 2; it++) {
      for (const c of colliders) {
        const center = new THREE.Vector3(c.len * 0.45, 0, 0).applyMatrix4(world[c.bone]);
        const d = s.pos.clone().sub(center);
        const r = c.len * 0.18;
        if (d.lengthSq() < r * r && d.lengthSq() > 1e-8) s.pos.copy(center).addScaledVector(d.normalize(), r);
      }
    }
    // 安全钳位：模拟点离锚点不超过静止臂长的 2 倍
    {
      const d = s.pos.clone().sub(anchorW);
      const arm = targetW.distanceTo(anchorW);
      if (arm > 1e-4 && d.length() > arm * 2) s.pos.copy(anchorW).addScaledVector(d.normalize(), arm * 2);
    }
    const restDir = targetW.clone().sub(anchorW);
    const curDir = s.pos.clone().sub(anchorW);
    const out = clothMats.get(s.bone);
    // 平移跟随（与 viewer 一致）：offset = pos − target，上限 8
    const off = s.pos.clone().sub(targetW);
    if (off.length() > 8) off.multiplyScalar(8 / off.length());
    out.multiplyMatrices(new THREE.Matrix4().makeTranslation(off.x, off.y, off.z), carry);
    restDir.length(); curDir.length();
  }
}
updateClothMats.lastAbs = NaN;

// ---- 逐帧蒙皮布料顶点并统计 ----
function skinClothVerts(world) {
  return dump.lower.map((v) => {
    const wsum = v.influences.reduce((s, i) => s + i[1], 0);
    const out = new THREE.Vector3();
    for (const inf of v.influences) {
      const b = inf[0];
      const w = inf[1] / wsum;
      if (clothMats.has(b)) {
        out.addScaledVector(new THREE.Vector3(...v.pos).applyMatrix4(clothMats.get(b)), w);
      } else {
        out.addScaledVector(new THREE.Vector3(...inf[2]).applyMatrix4(world[b]), w);
      }
    }
    return out;
  });
}

// 绑定态簇信息（每骨簇质心 + 相对骨盆的方位角）
const bindClusters = new Map();
for (const s of sims) {
  const carried = s.target.clone().applyMatrix4(new THREE.Matrix4()); // 骨盆绑定系
  bindClusters.set(s.bone, carried.clone());
}

let worstF0 = 0;
for (let f = 0; f < frames; f++) {
  const world = worldsAt(f);
  updateClothMats(f, world);
  const out = skinClothVerts(world);
  // 相对骨盆携带绑定位置的偏差（纯摆动量）
  const carryNow = new THREE.Matrix4().multiplyMatrices(world[carrier], tpInv);
  let maxRel = 0, maxAbs = 0;
  for (let i = 0; i < dump.lower.length; i++) {
    const v = dump.lower[i];
    if (!v.influences.some((inf) => clothMats.has(inf[0]))) continue;
    const base = new THREE.Vector3(...v.pos).applyMatrix4(carryNow);
    maxRel = Math.max(maxRel, out[i].distanceTo(base));
    maxAbs = Math.max(maxAbs, out[i].distanceTo(new THREE.Vector3(...v.pos)));
  }
  if (f === 0) worstF0 = maxAbs;
  // 插桩：最坏模拟点的详情 + 骨盆运动
  if (f % 10 === 0 || f === frames - 1) {
    let worst = null;
    for (const s of sims) {
      const targetW = s.target.clone().applyMatrix4(carryNow);
      const d = s.pos.distanceTo(targetW);
      if (!worst || d > worst.d) worst = { b: s.bone, d, arm: targetW.distanceTo(s.target.clone().applyMatrix4(new THREE.Matrix4().multiplyMatrices(worldsAt(0)[carrier], tpInv)).add(new THREE.Vector3())) };
    }
    const pelvis = world[carrier].elements;
    console.log(`    [dbg] 骨盆 t=(${pelvis[12].toFixed(0)},${pelvis[13].toFixed(0)},${pelvis[14].toFixed(0)}) 最坏骨 0x${worst.b.toString(16)} 滞后=${worst.d.toFixed(1)}`);
  }
  if (f % 10 === 0 || f === frames - 1) {
    // 面板方位检查：每骨簇质心相对绑定簇的偏移
    let maxPanelShift = 0;
    for (const s of sims) {
      const cur = new THREE.Vector3(...vertsOf[s.bone][0]).applyMatrix4(clothMats.get(s.bone));
      const carried = new THREE.Vector3(...vertsOf[s.bone][0]).applyMatrix4(carryNow);
      maxPanelShift = Math.max(maxPanelShift, cur.distanceTo(carried));
    }
    console.log(`帧${String(f).padStart(3)}: 偏离authored max=${maxAbs.toFixed(1)} 相对携带 max=${maxRel.toFixed(1)} 面板相对携带 max=${maxPanelShift.toFixed(1)}`);
  }
}
console.log(`\n帧0 贴合度: ${worstF0.toFixed(2)}（应 <5）`);
