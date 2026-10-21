// 髋/腿根穿模探针：node probe-hipclip.mjs [frame | f0-f1]
// 无参=当前帧；数字=定格该帧；区间=逐帧扫描（每帧最小网格距离+最差顶点）。
// 裙顶点 vs girl_Lower 身体网格三角形（腿/骨盆分区）的最近距离，与腿胶囊
// 间隙对照 —— 定位"胶囊外但网格内"的覆盖缺口。
// 三角形分类：三顶点影响均无 Bone* 链 → 身体三角；按 Thigh/Calf/Foot vs
// Pelvis 分区（任一顶点命中即归该区）。
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
ws.on("open", async () => {
  await send("Runtime.enable");
  if (frameArg !== null) {
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
    const vcls = new Array(p.inflOf.length);
    for (let i = 0; i < p.inflOf.length; i++) {
      let c = "other";
      for (const inf of p.inflOf[i]) {
        const t = clsOf.get(inf.boneIndex) ?? "other";
        if (t === "chain") { c = "chain"; break; }
        if (t === "leg") c = c === "pelvis" ? "mixed" : "leg";
        else if (t === "pelvis") c = c === "leg" ? "mixed" : "pelvis";
      }
      vcls[i] = c;
    }
    const tris = [];
    for (let t = 0; t < triCount; t++) {
      const a = idx ? idx[t*3] : t*3, b = idx ? idx[t*3+1] : t*3+1, c = idx ? idx[t*3+2] : t*3+2;
      if (vcls[a] === "chain" || vcls[b] === "chain" || vcls[c] === "chain") continue;
      const mk = (vcls[a]==="leg"||vcls[b]==="leg"||vcls[c]==="leg") ? "leg"
        : (vcls[a]==="pelvis"||vcls[b]==="pelvis"||vcls[c]==="pelvis") ? "pelvis" : null;
      if (mk) tris.push({ a3: a*3, b3: b*3, c3: c*3, kind: mk });
    }
    const d2pt = (px,py,pz, ax,ay,az, bx,by,bz, cx,cy,cz) => {
      const abx=bx-ax,aby=by-ay,abz=bz-az, acx=cx-ax,acy=cy-ay,acz=cz-az;
      const apx=px-ax,apy=py-ay,apz=pz-az;
      const d1=abx*apx+aby*apy+abz*apz, d2=acx*apx+acy*apy+acz*apz;
      if (d1<=0 && d2<=0) return apx*apx+apy*apy+apz*apz;
      const bpx=px-bx,bpy=py-by,bpz=pz-bz;
      const d3=abx*bpx+aby*bpy+abz*bpz, d4=acx*bpx+acy*bpy+acz*bpz;
      if (d3>=0 && d4<=d3) return bpx*bpx+bpy*bpy+bpz*bpz;
      const vc=d1*d4-d3*d2;
      if (vc<=0 && d1>=0 && d3<=0) { const t=d1/(d1-d3); const qx=ax+abx*t-px,qy=ay+aby*t-py,qz=az+abz*t-pz; return qx*qx+qy*qy+qz*qz; }
      const cpx=px-cx,cpy=py-cy,cpz=pz-cz;
      const d5=abx*cpx+aby*cpy+abz*cpz, d6=acx*cpx+acy*cpy+acz*cpz;
      if (d6>=0 && d5<=d6) return cpx*cpx+cpy*cpy+cpz*cpz;
      const vb=d5*d2-d1*d6;
      if (vb<=0 && d2>=0 && d6<=0) { const t=d2/(d2-d6); const qx=ax+acx*t-px,qy=ay+acy*t-py,qz=az+acz*t-pz; return qx*qx+qy*qy+qz*qz; }
      const va=d3*d6-d5*d4;
      if (va<=0 && d4-d3>=0 && d5-d6>=0) { const t=(d4-d3)/((d4-d3)+(d5-d6)); const qx=bx+(cx-bx)*t-px,qy=by+(cy-by)*t-py,qz=bz+(cz-bz)*t-pz; return qx*qx+qy*qy+qz*qz; }
      const den=va+vb+vc; const t0=vb/den, t1=vc/den;
      const qx=ax+abx*t0+acx*t1-px, qy=ay+aby*t0+acy*t1-py, qz=az+abz*t0+acz*t1-pz;
      return qx*qx+qy*qy+qz*qz;
    };
    const caps = v.legCaps.map(c => {
      const e = v.worldMats[c.bone].elements;
      return { ax: e[12], ay: e[13], az: e[14], bx: e[12]+e[0]*c.len, by: e[13]+e[1]*c.len, bz: e[14]+e[2]*c.len, r: c.len*0.18 };
    });
    const capClear = (x, y, z) => {
      let best = 1e9;
      for (const c of caps) {
        const abx=c.bx-c.ax, aby=c.by-c.ay, abz=c.bz-c.az;
        const ab2=abx*abx+aby*aby+abz*abz;
        let t = ab2>1e-9 ? ((x-c.ax)*abx+(y-c.ay)*aby+(z-c.az)*abz)/ab2 : 0;
        t = Math.min(Math.max(t,0),1);
        const d = Math.hypot(x-(c.ax+abx*t), y-(c.ay+aby*t), z-(c.az+abz*t));
        best = Math.min(best, d - c.r);
      }
      return best;
    };
    const chainOf = new Map();
    const roots = [];
    for (let b = 0; b < n; b++) if (v.isBoneHelperName(b)) { const par = v.skelParents[b]; if (par < 0 || !v.isBoneHelperName(par)) roots.push(b); }
    for (const r of roots) for (const mm of v.boneChainMembers(r)) chainOf.set(mm, v.skelBones[r].name);

    const analyze = () => {
      let minLeg = 1e9, minPel = 1e9;
      const worst = [];
      for (const sv of v.skirtVerts) {
        const o = sv.i * 3;
        const x = pos[o], y = pos[o+1], z = pos[o+2];
        let mLeg = 1e9, mPel = 1e9;
        for (const t of tris) {
          const d = Math.sqrt(d2pt(x,y,z, pos[t.a3],pos[t.a3+1],pos[t.a3+2], pos[t.b3],pos[t.b3+1],pos[t.b3+2], pos[t.c3],pos[t.c3+1],pos[t.c3+2]));
          if (t.kind === "leg") { if (d < mLeg) mLeg = d; } else { if (d < mPel) mPel = d; }
        }
        minLeg = Math.min(minLeg, mLeg); minPel = Math.min(minPel, mPel);
        let chain = "-";
        for (const inf of sv.p.inflOf[sv.i]) { const c = chainOf.get(inf.boneIndex); if (c) { chain = c; break; } }
        worst.push({ chain, x: +x.toFixed(1), y: +y.toFixed(1), z: +z.toFixed(1), leg: +mLeg.toFixed(2), pel: +mPel.toFixed(2), cap: +capClear(x,y,z).toFixed(2) });
      }
      worst.sort((a, b) => Math.min(a.leg, a.pel) - Math.min(b.leg, b.pel));
      return { minLeg: +minLeg.toFixed(2), minPel: +minPel.toFixed(2), worst: worst.slice(0, 6) };
    };

    if (${scan ? `true` : `false`}) {
      const out = [];
      for (let f = ${scan ? scan[0] : 0}; f <= ${scan ? scan[1] : 0}; f++) {
        v.applyFskelFrame(f, f, 0);
        const a = analyze();
        out.push({ f, minLeg: a.minLeg, minPel: a.minPel, worst: a.worst.slice(0, 3) });
      }
      return JSON.stringify({ frame: v.diag.i0, scan: out });
    }
    return JSON.stringify(Object.assign({ frame: v.diag.i0, bodyTris: tris.length, legTris: tris.filter(t => t.kind === "leg").length }, analyze()));
  })()`;
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  const val = r.result?.result?.value;
  try { console.log(JSON.stringify(JSON.parse(val), null, 1)); }
  catch { console.log(val ?? JSON.stringify(r.result).slice(0, 800)); }
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 60000);
