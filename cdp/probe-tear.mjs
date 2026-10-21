// 撕裂探针：node probe-tear.mjs [frame]
// 混合边（裙链顶点 ↔ 非链顶点，如裙身↔腰带交界）的拉伸量：
// |当前边长| - |纯LBS边长|。非裙顶点恒 LBS，裙顶点被推出+松弛后位移，
// 位移在边方向的投影即撕缝宽度。定位"裙腰交界水平裂缝"类问题。
import WebSocket from "ws";
import { request } from "node:http";

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;
const frame = process.argv[2] ? Number(process.argv[2]) : null;

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
const page = targets.find((t) => t.type === "page" && /tauri\.localhost|localhost:1420/.test(t.url));
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

const expr = `(() => {
  const v = window.__viewer;
  ${frame !== null ? `v.applyFskelFrame(${frame}, ${frame}, 0);` : ""}
  const n = v.fskelBoneCount;
  const lbs = (p, i) => {  // 裙顶点纯 LBS 重算（与 skinPart 同源矩阵）
    let x = 0, y = 0, z = 0;
    for (const inf of p.inflOf[i]) {
      if (inf.boneIndex >= n) continue;
      const M = (v.anchoredMats.size && v.anchoredMats.has(inf.boneIndex)
        ? v.anchoredMats.get(inf.boneIndex) : v.worldMats[inf.boneIndex]).elements;
      const lp = inf.boneLocalPos;
      x += inf.weight * (M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
      y += inf.weight * (M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
      z += inf.weight * (M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
    }
    return [x, y, z];
  };
  const out = [];
  for (const p of v.parts.values()) {
    const chain = new Uint8Array(p.inflOf.length);
    for (let i = 0; i < p.inflOf.length; i++)
      for (const inf of p.inflOf[i])
        if (inf.boneIndex < n && v.isBoneHelperName(inf.boneIndex)) { chain[i] = 1; break; }
    const pa = p.geom.getAttribute("position").array;
    const idxAttr = p.geom.getIndex();
    const idx = idxAttr ? idxAttr.array : null;
    const triCount = idx ? Math.floor(idx.length / 3) : Math.floor(p.inflOf.length / 3);
    const seen = new Set();
    for (let t = 0; t < triCount; t++) {
      const a = idx ? idx[t*3] : t*3, b = idx ? idx[t*3+1] : t*3+1, c = idx ? idx[t*3+2] : t*3+2;
      const vs = [a, b, c];
      const anyChain = vs.some(i => chain[i]), anyNon = vs.some(i => !chain[i]);
      if (!anyChain || !anyNon) continue;  // 只看混合三角形
      for (const s of vs) for (const o of vs) {
        if (s === o || !chain[s] || chain[o]) continue;
        const key = s + ":" + o;
        if (seen.has(key)) continue;
        seen.add(key);
        const cur = Math.hypot(pa[s*3]-pa[o*3], pa[s*3+1]-pa[o*3+1], pa[s*3+2]-pa[o*3+2]);
        const L = lbs(p, s);
        const base = Math.hypot(L[0]-pa[o*3], L[1]-pa[o*3+1], L[2]-pa[o*3+2]);
        out.push({ s, o, cur: +cur.toFixed(3), base: +base.toFixed(3), stretch: +(cur-base).toFixed(3),
          sy: +pa[s*3+1].toFixed(1), bones: [...new Set(p.inflOf[s].map(x => v.skelBones[x.boneIndex]?.name))].join("+") });
      }
    }
  }
  out.sort((a, b) => b.stretch - a.stretch);
  return JSON.stringify({ frame: v.diag.i0, act: v.actName, total: out.length,
    top: out.slice(0, 25), over05: out.filter(e => e.stretch > 0.5).length });
})()`;

ws.on("open", async () => {
  await send("Runtime.enable");
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  const val = r.result?.result?.value;
  if (!val) { console.error("evaluate failed:", JSON.stringify(r.result).slice(0, 300)); process.exit(1); }
  const j = JSON.parse(val);
  console.log(`动作=${j.act} 帧=${j.frame} 混合边=${j.total} 拉伸>0.5=${j.over05}`);
  console.log("裙顶点  非裙点  当前  LBS基准  拉伸  y    裙端链骨");
  for (const e of j.top) {
    console.log(`${String(e.s).padEnd(6)} ${String(e.o).padEnd(6)} ${String(e.cur).padEnd(6)} ${String(e.base).padEnd(7)} ${String(e.stretch).padEnd(5)} ${String(e.sy).padEnd(5)} ${e.bones}${e.stretch > 0.5 ? " ◀撕" : ""}`);
  }
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 60000);
