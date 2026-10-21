/**
 * 复刻 viewer.computeBindMats / partBindInconsistent（部件数投票分簇），
 * 检查全套装配里哪些部件会被静态模式判为"离群"而重摆 —— 应只有 hair05。
 * 用法：node scripts/verify-static-flags.mjs <hair05_check.json 路径>
 */
import * as THREE from "three";
import { readFileSync } from "node:fs";

const dumpPath = process.argv[2] ?? "G:/pojie/ai/Gongfu-studio/target/hair05_check.json";
const dumpUrl = /^[a-zA-Z]:[\\/]/.test(dumpPath)
  ? new URL("file:///" + dumpPath.replace(/\\/g, "/"))
  : new URL(dumpPath, import.meta.url);
const dump = JSON.parse(readFileSync(dumpUrl, "utf8"));

// 与 viewer.ts 一致的拟合与差异度量
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
  for (let i = 0; i < 16; i++) dr = Math.max(dr, Math.abs(a.elements[i] - b.elements[i]));
  const dt = Math.hypot(
    a.elements[12] - b.elements[12],
    a.elements[13] - b.elements[13],
    a.elements[14] - b.elements[14]
  );
  return Math.max(dr, dt);
}

// 部件顶点格式：influences = [boneIndex, weight, localPos][]
const parts = dump.all.map(([name, verts]) => ({ name, verts }));
const boneCount = 1 + Math.max(
  0,
  ...parts.flatMap((p) => p.verts.flatMap((v) => v.influences.map(([b]) => b)))
);
const perBone = Array.from({ length: boneCount }, () => ({ exact: [], approx: [] }));
const exactOf = parts.map((p) => {
  const exact = Array.from({ length: boneCount }, () => []);
  const approx = Array.from({ length: boneCount }, () => []);
  for (const v of p.verts) {
    for (const [b, w, local] of v.influences) {
      if (b >= boneCount) continue;
      if (v.influences.length === 1 || w >= 0.99) exact[b].push([local, v.pos]);
      else approx[b].push([local, v.pos]);
    }
  }
  for (let b = 0; b < boneCount; b++) {
    if (exact[b].length) perBone[b].exact.push(exact[b]);
    if (approx[b].length) perBone[b].approx.push(approx[b]);
  }
  return exact;
});

// 共识（部件数投票分簇，阈值 0.1）
const bindMats = perBone.map(({ exact, approx }) => {
  const fits = exact.map((pts) => fitBindMatrix(pts));
  let pool = [];
  if (fits.length >= 2) {
    const ok = (i, j) => bindTransformDiff(fits[i], fits[j]) < 0.1;
    const comps = [];
    const seen = new Set();
    for (let i = 0; i < fits.length; i++) {
      if (seen.has(i)) continue;
      const comp = [i];
      seen.add(i);
      let changed = true;
      while (changed) {
        changed = false;
        for (let j = 0; j < fits.length; j++) {
          if (!seen.has(j) && comp.some((c) => ok(c, j))) {
            comp.push(j);
            seen.add(j);
            changed = true;
          }
        }
      }
      comps.push(comp);
    }
    const best = comps.reduce((a, c) => (c.length > a.length ? c : a));
    pool = best.flatMap((i) => exact[i]);
  } else if (exact.length === 1) {
    pool = [...exact[0]];
  }
  if (!pool.length) return new THREE.Matrix4();
  return fitBindMatrix(pool.length >= 3 ? pool : pool.concat(...approx));
});

let flagged = [];
parts.forEach((p, pi) => {
  for (let b = 0; b < boneCount; b++) {
    if (exactOf[pi][b].length < 3) continue;
    const d = bindTransformDiff(fitBindMatrix(exactOf[pi][b]), bindMats[b]);
    if (d > 0.1) {
      flagged.push(`${p.name} 骨 0x${b.toString(16)} diff=${d.toFixed(3)}`);
      break;
    }
  }
});
console.log("静态模式判为离群(将被重摆)的部件:");
console.log(flagged.length ? flagged.join("\n") : "（无）");
const expect = flagged.every((f) => f.startsWith("MHair/Girl_hair05.fskin"));
if (!expect) {
  console.error("FAIL: 出现预期外的离群部件");
  process.exit(1);
}
console.log(flagged.length === 1 ? "PASS" : "WARN: hair05 未被标记?");
