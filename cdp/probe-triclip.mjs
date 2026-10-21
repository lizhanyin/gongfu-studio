// 三角形跨穿探针：node probe-triclip.mjs [f0-f1 | frame]
// 裙三角形(任一顶点受 Bone* 链影响) vs 身体三角形(腿/骨盆分区)的
// 三角形-三角形相交逐帧统计 —— 顶点级推出防不住的面片跨穿检测。
// 相交判定：平面侧别早退 + 边-三角形求交(Möller–Trumbore 式) + 共面退化按质心包含。
import WebSocket from "ws";
import { request } from "node:http";

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;
const arg = process.argv[2] ?? null;
const m = arg && /^(\d+)-(\d+)$/.exec(arg);
const scan = m ? [+m[1], +m[2]] : null;
const frameArg = !scan && arg !== null ? Number(arg) : null;

function getJson(path) {
  return new Promise((resolve, reject) => {
    const req = request({ host: "127.0.0.1", port: CDP_PORT, path, method: "GET" }, (res) => {
      let data = "";
      res.on("data", (c) => (data += c));
      res.on("end", () => resolve(JSON.parse(data)));
    });
    req.on("error", reject);
    req.end();
  });
}

const targets = await getJson("/json");
const page = targets.find((t) => t.type === "page" && /tauri\.localhost/.test(t.url));
if (!page) { console.error("no page target"); process.exit(1); }
const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false });
let id = 0;
const pending = new Map();
function send(method, params) {
  return new Promise((resolve) => {
    const mid = ++id;
    pending.set(mid, resolve);
    ws.send(JSON.stringify({ id: mid, method, params }));
  });
}
ws.on("message", (raw) => {
  const msg = JSON.parse(raw.toString());
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
});
ws.on("open", async () => {
  await send("Runtime.enable");
  if (frameArg !== null && Number.isFinite(frameArg)) {
    await send("Runtime.evaluate", { expression: `__viewer.applyFskelFrame(${frameArg}, ${frameArg}, 0)` });
    await new Promise((r) => setTimeout(r, 200));
  }
  const expr = `(() => {
    const v = window.__viewer;
    const n = v.fskelBoneCount;
    const clsOf = new Map();
    for (let b = 0; b < n; b++) {
      const nm = v.skelBones[b]?.name ?? "";
      clsOf.set(b, /Thigh|Calf|Foot/.test(nm) ? "leg" : /Pelvis/.test(nm) ? "pelvis" : v.isBoneHelperName(b) ? "chain" : "other");
    }
    const lower = [...v.parts.entries()].find(([k]) => k.includes("Lower"));
    if (!lower) return JSON.stringify({ err: "no girl_Lower" });
    const [, p] = lower;
    const pos = p.geom.getAttribute("position").array;
    const idxAttr = p.geom.getIndex();
    const idx = idxAttr ? idxAttr.array : null;
    const triCount = idx ? idx.length / 3 : p.inflOf.length / 3;
    const isChainVert = new Array(p.inflOf.length).fill(false);
    for (let i = 0; i < p.inflOf.length; i++)
      for (const inf of p.inflOf[i]) if (clsOf.get(inf.boneIndex) === "chain") { isChainVert[i] = true; break; }
    const skirtTris = [], bodyTris = [];
    for (let t = 0; t < triCount; t++) {
      const a = idx ? idx[t*3] : t*3, b = idx ? idx[t*3+1] : t*3+1, c = idx ? idx[t*3+2] : t*3+2;
      if (isChainVert[a] || isChainVert[b] || isChainVert[c]) { skirtTris.push([a, b, c]); continue; }
      let kind = null;
      outer: for (const i2 of [a, b, c]) {
        for (const inf of p.inflOf[i2]) {
          const t2 = clsOf.get(inf.boneIndex);
          if (t2 === "leg") { kind = "leg"; break outer; }
          if (t2 === "pelvis" && !kind) kind = "pelvis";
        }
      }
      if (kind) bodyTris.push({ v: [a, b, c], kind });
    }

    // ---- 干净的三角形-三角形相交 ----
    const sub = (a, b) => [a[0]-b[0], a[1]-b[1], a[2]-b[2]];
    const crs = (a, b) => [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
    const dt = (a, b) => a[0]*b[0]+a[1]*b[1]+a[2]*b[2];
    const planeN = (A, B, C) => crs(sub(B, A), sub(C, A));
    // 点 P 在三角形 ABC(法线 nn)平面投影是否在三角形内
    const inTri = (P, A, B, C, nn) => {
      const pa = sub(P, A), pb = sub(P, B), pc = sub(P, C);
      const ab = sub(B, A), bc = sub(C, B), ca = sub(A, C);
      return dt(nn, crs(ab, pa)) >= 0 && dt(nn, crs(bc, pb)) >= 0 && dt(nn, crs(ca, pc)) >= 0;
    };
    // 线段 PQ 与平面(nn, 过 A)相交参数 t（null=平行）
    const segPlaneT = (P, Q, nn, A) => {
      const d = sub(Q, P);
      const dn = dt(nn, d);
      if (Math.abs(dn) < 1e-12) return null;
      return dt(nn, sub(A, P)) / dn;
    };
    const triTri = (V0, V1, V2, U0, U1, U2) => {
      // AABB 快筛
      for (let k = 0; k < 3; k++) {
        if (Math.min(V0[k],V1[k],V2[k]) > Math.max(U0[k],U1[k],U2[k])) return false;
        if (Math.min(U0[k],U1[k],U2[k]) > Math.max(V0[k],V1[k],V2[k])) return false;
      }
      const nV = planeN(V0, V1, V2), nU = planeN(U0, U1, U2);
      const dv = [dt(nU, sub(V0, U0)), dt(nU, sub(V1, U0)), dt(nU, sub(V2, U0))];
      if (dv[0] > 0 && dv[1] > 0 && dv[2] > 0) return false;
      if (dv[0] < 0 && dv[1] < 0 && dv[2] < 0) return false;
      const du = [dt(nV, sub(U0, V0)), dt(nV, sub(U1, V0)), dt(nV, sub(U2, V0))];
      if (du[0] > 0 && du[1] > 0 && du[2] > 0) return false;
      if (du[0] < 0 && du[1] < 0 && du[2] < 0) return false;
      const coplanar = Math.abs(dv[0]) < 1e-9 && Math.abs(dv[1]) < 1e-9 && Math.abs(dv[2]) < 1e-9;
      if (coplanar) {
        // 退化：质心包含近似（罕见，宽松处理）
        const cV = [(V0[0]+V1[0]+V2[0])/3, (V0[1]+V1[1]+V2[1])/3, (V0[2]+V1[2]+V2[2])/3];
        const cU = [(U0[0]+U1[0]+U2[0])/3, (U0[1]+U1[1]+U2[1])/3, (U0[2]+U1[2]+U2[2])/3];
        return inTri(cV, U0, U1, U2, nU) || inTri(cU, V0, V1, V2, nV);
      }
      // V 的边穿 U 平面且交点在 U 内
      const eV = [[V0, V1], [V1, V2], [V2, V0]];
      for (const [P, Q] of eV) {
        const t = segPlaneT(P, Q, nU, U0);
        if (t !== null && t >= 0 && t <= 1) {
          const ip = [P[0]+(Q[0]-P[0])*t, P[1]+(Q[1]-P[1])*t, P[2]+(Q[2]-P[2])*t];
          if (inTri(ip, U0, U1, U2, nU)) return true;
        }
      }
      // U 的边穿 V 平面且交点在 V 内
      const eU = [[U0, U1], [U1, U2], [U2, U0]];
      for (const [P, Q] of eU) {
        const t = segPlaneT(P, Q, nV, V0);
        if (t !== null && t >= 0 && t <= 1) {
          const ip = [P[0]+(Q[0]-P[0])*t, P[1]+(Q[1]-P[1])*t, P[2]+(Q[2]-P[2])*t];
          if (inTri(ip, V0, V1, V2, nV)) return true;
        }
      }
      return false;
    };
    const P3 = i => [pos[i*3], pos[i*3+1], pos[i*3+2]];
    const analyze = () => {
      let hitLeg = 0, hitPel = 0;
      const samples = [];
      for (const st of skirtTris) {
        const V0 = P3(st[0]), V1 = P3(st[1]), V2 = P3(st[2]);
        for (const bt of bodyTris) {
          const U0 = P3(bt.v[0]), U1 = P3(bt.v[1]), U2 = P3(bt.v[2]);
          if (triTri(V0, V1, V2, U0, U1, U2)) {
            if (bt.kind === "leg") hitLeg++; else hitPel++;
            if (samples.length < 6) samples.push({
              skirtY: +((V0[1]+V1[1]+V2[1])/3).toFixed(1),
              skirtZ: +((V0[2]+V1[2]+V2[2])/3).toFixed(1),
              skirtX: +((V0[0]+V1[0]+V2[0])/3).toFixed(1),
              vs: bt.kind,
              spanY: +(Math.abs(V0[1]-V1[1]) + Math.abs(V1[1]-V2[1])).toFixed(1),
            });
          }
        }
      }
      return { hitLeg, hitPel, samples };
    };
    if (${scan ? "true" : "false"}) {
      const out = [];
      for (let f = ${scan ? scan[0] : 0}; f <= ${scan ? scan[1] : 0}; f++) {
        v.applyFskelFrame(f, f, 0);
        const a = analyze();
        if (a.hitLeg || a.hitPel) out.push({ f, hitLeg: a.hitLeg, hitPel: a.hitPel, samples: a.samples.slice(0, 3) });
      }
      return JSON.stringify({ frame: v.diag.i0, skirtTris: skirtTris.length, bodyTris: bodyTris.length, legTris: bodyTris.filter(t => t.kind === "leg").length, hits: out });
    }
    return JSON.stringify(Object.assign({ frame: v.diag.i0, skirtTris: skirtTris.length, bodyTris: bodyTris.length }, analyze()));
  })()`;
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  const val = r.result?.result?.value;
  try { console.log(JSON.stringify(JSON.parse(val), null, 1)); }
  catch { console.log(val ?? JSON.stringify(r.result).slice(0, 1200)); }
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 120000);
