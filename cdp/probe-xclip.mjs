// 全部件交叉穿模探针：node probe-xclip.mjs [f0-f1 | frame]
// 所有部件三角形按来源/骨骼分类(strip=Lower布料链, leg=Lower腿, pelvis=Lower骨盆,
// boot=Boot, upper=Upper, hair=Hair, head=head)，统计"软×硬"组合的三角形相交。
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
    // 每部件三角形分类
    const all = []; // {cat, a3, b3, c3, part}
    for (const [path, p] of v.parts) {
      const fname = path.split(/[\\\\/]/).pop();
      const partCat = /Boot/i.test(fname) ? "boot" : /Upper/i.test(fname) ? "upper" : /Hair/i.test(fname) ? "hair" : /head/i.test(fname) ? "head" : "lower";
      const pos = p.geom.getAttribute("position").array;
      const idxAttr = p.geom.getIndex();
      const idx = idxAttr ? idxAttr.array : null;
      const triCount = idx ? idx.length / 3 : p.inflOf.length / 3;
      for (let t = 0; t < triCount; t++) {
        const a = idx ? idx[t*3] : t*3, b = idx ? idx[t*3+1] : t*3+1, c = idx ? idx[t*3+2] : t*3+2;
        let cat = partCat;
        if (partCat === "lower") {
          const cls = [a, b, c].map(i2 => {
            for (const inf of p.inflOf[i2]) {
              const t2 = clsOf.get(inf.boneIndex);
              if (t2 === "chain") return "chain";
              if (t2 === "leg") return "leg";
              if (t2 === "pelvis") return "pelvis";
            }
            return "other";
          });
          cat = cls.includes("chain") ? "strip" : cls.includes("leg") ? "leg" : cls.includes("pelvis") ? "pelvis" : "lowother";
        }
        if (cat === "lowother" || cat === "head") continue;
        all.push({ cat, part: fname, a3: a*3, b3: b*3, c3: c*3 });
      }
    }
    // 软×硬组合
    const soft = new Set(["strip", "upper", "hair"]);
    const hard = new Set(["leg", "boot", "pelvis"]);
    const sub = (a, b, o) => { o[0]=a[0]-b[0]; o[1]=a[1]-b[1]; o[2]=a[2]-b[2]; return o; };
    const crs = (a, b, o) => { o[0]=a[1]*b[2]-a[2]*b[1]; o[1]=a[2]*b[0]-a[0]*b[2]; o[2]=a[0]*b[1]-a[1]*b[0]; return o; };
    const dt3 = (a, b) => a[0]*b[0]+a[1]*b[1]+a[2]*b[2];
    const T1=[0,0,0],T2=[0,0,0],T3=[0,0,0],T4=[0,0,0],T5=[0,0,0],NV=[0,0,0],NU=[0,0,0],TMP=[0,0,0];
    const A0=[0,0,0],A1=[0,0,0],A2=[0,0,0],B0=[0,0,0],B1=[0,0,0],B2=[0,0,0],IP=[0,0,0];
    const inTri = (P, A, B, C, nn) => {
      sub(P,A,T1); sub(B,A,T2); crs(T2,T1,T3); if (dt3(T3,nn) < 0) return false;
      sub(P,B,T1); sub(C,B,T2); crs(T2,T1,T3); if (dt3(T3,nn) < 0) return false;
      sub(P,C,T1); sub(A,C,T2); crs(T2,T1,T3); if (dt3(T3,nn) < 0) return false;
      return true;
    };
    // 每部件位置数组缓存
    const posArr = new Map();
    for (const [path, p] of v.parts) posArr.set(path, p.geom.getAttribute("position").array);
    const partPath = new Map();
    for (const [path, p] of v.parts) partPath.set(p, path);
    // 预取每个三角形的 part path（构造时带）
    for (const t of all) t.pa = posArr.get([...v.parts.entries()].find(([k]) => k.endsWith(t.part))[0]);
    const triTri = (pa, ta, pb, tb) => {
      const V0=[pa[ta.a3],pa[ta.a3+1],pa[ta.a3+2]], V1=[pa[ta.b3],pa[ta.b3+1],pa[ta.b3+2]], V2=[pa[ta.c3],pa[ta.c3+1],pa[ta.c3+2]];
      const U0=[pb[tb.a3],pb[tb.a3+1],pb[tb.a3+2]], U1=[pb[tb.b3],pb[tb.b3+1],pb[tb.b3+2]], U2=[pb[tb.c3],pb[tb.c3+1],pb[tb.c3+2]];
      for (let k = 0; k < 3; k++) {
        if (Math.min(V0[k],V1[k],V2[k]) > Math.max(U0[k],U1[k],U2[k])) return false;
        if (Math.min(U0[k],U1[k],U2[k]) > Math.max(V0[k],V1[k],V2[k])) return false;
      }
      sub(V1,V0,T1); sub(V2,V0,T2); crs(T1,T2,NV);
      sub(U1,U0,T3); sub(U2,U0,T4); crs(T3,T4,NU);
      const d0=dt3(NU,sub(V0,U0,T5)), d1=dt3(NU,sub(V1,U0,T5)), d2=dt3(NU,sub(V2,U0,T5));
      if (d0>1e-9&&d1>1e-9&&d2>1e-9) return false;
      if (d0<-1e-9&&d1<-1e-9&&d2<-1e-9) return false;
      const e0=dt3(NV,sub(U0,V0,T5)), e1=dt3(NV,sub(U1,V0,T5)), e2=dt3(NV,sub(U2,V0,T5));
      if (e0>1e-9&&e1>1e-9&&e2>1e-9) return false;
      if (e0<-1e-9&&e1<-1e-9&&e2<-1e-9) return false;
      if (Math.abs(d0)<1e-9 && Math.abs(d1)<1e-9 && Math.abs(d2)<1e-9) {
        const cV=[(V0[0]+V1[0]+V2[0])/3,(V0[1]+V1[1]+V2[1])/3,(V0[2]+V1[2]+V2[2])/3];
        return inTri(cV,U0,U1,U2,NU);
      }
      const eV=[[V0,V1],[V1,V2],[V2,V0]];
      for (const [P,Q] of eV) {
        sub(Q,P,T1);
        const dn=dt3(NU,T1);
        if (Math.abs(dn)<1e-12) continue;
        sub(U0,P,T2);
        const t=dt3(NU,T2)/dn;
        if (t<0||t>1) continue;
        IP[0]=P[0]+T1[0]*t; IP[1]=P[1]+T1[1]*t; IP[2]=P[2]+T1[2]*t;
        if (inTri(IP,U0,U1,U2,NU)) return true;
      }
      const eU=[[U0,U1],[U1,U2],[U2,U0]];
      for (const [P,Q] of eU) {
        sub(Q,P,T1);
        const dn=dt3(NV,T1);
        if (Math.abs(dn)<1e-12) continue;
        sub(V0,P,T2);
        const t=dt3(NV,T2)/dn;
        if (t<0||t>1) continue;
        IP[0]=P[0]+T1[0]*t; IP[1]=P[1]+T1[1]*t; IP[2]=P[2]+T1[2]*t;
        if (inTri(IP,V0,V1,V2,NV)) return true;
      }
      return false;
    };
    const runFrame = () => {
      const hits = {};
      const samples = [];
      for (let i = 0; i < all.length; i++) {
        const ta = all[i];
        for (let j = i + 1; j < all.length; j++) {
          const tb = all[j];
          // 同部件共享顶点的相邻三角形不算（折痕不是穿模）
          if (ta.part === tb.part) {
            const sa = new Set([ta.a3, ta.b3, ta.c3]);
            if (sa.has(tb.a3) || sa.has(tb.b3) || sa.has(tb.c3)) continue;
          }
          const key = ta.cat + "×" + tb.cat;
          if (key !== "strip×leg") continue;
          if (triTri(ta.pa, ta, tb.pa, tb)) {
            hits[key] = (hits[key] ?? 0) + 1;
            if (samples.length < 10) samples.push({
              pair: key, a: ta.part, b: tb.part,
              x: +ta.pa[ta.a3].toFixed(1), y: +ta.pa[ta.a3+1].toFixed(1), z: +ta.pa[ta.a3+2].toFixed(1),
            });
          }
        }
      }
      return { hits, samples };
    };
    if (${scan ? "true" : "false"}) {
      const out = [];
      for (let f = ${scan ? scan[0] : 0}; f <= ${scan ? scan[1] : 0}; f++) {
        v.applyFskelFrame(f, f, 0);
        const a = runFrame();
        if (Object.keys(a.hits).length) out.push({ f, hits: a.hits, samples: a.samples.slice(0, 4) });
      }
      return JSON.stringify({ frame: v.diag.i0, tris: all.length, byCat: all.reduce((m,t)=>(m[t.cat]=(m[t.cat]??0)+1,m),{}), hits: out });
    }
    return JSON.stringify(Object.assign({ frame: v.diag.i0, tris: all.length, byCat: all.reduce((m,t)=>(m[t.cat]=(m[t.cat]??0)+1,m),{}) }, runFrame()));
  })()`;
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  const val = r.result?.result?.value;
  try { console.log(JSON.stringify(JSON.parse(val), null, 1)); }
  catch { console.log(val ?? JSON.stringify(r.result).slice(0, 1500)); }
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 180000);
