// 前裙中央垂片诊断：node probe-frontflap.mjs
// 1) 列出各 Bone* 链的绑定 bbox/质心（识别前中垂片）
// 2) std8 全区间逐帧采样：垂片在 纯LBS(游戏行为) vs 锚定摆放 两种模式下
//    对腿胶囊的穿透，并区分链归属
import WebSocket from "ws";
import { request } from "node:http";

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;

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
const page =
  targets.find((t) => t.type === "page" && /tauri\.localhost/.test(t.url)) ??
  targets.find((t) => t.type === "page");
if (!page) {
  console.error("no page target");
  process.exit(1);
}
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
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
});
ws.on("open", async () => {
  await send("Runtime.enable");
  const expr = `(() => {
    const v = window.__viewer;
    if (!v || !v.fskelPoses) return JSON.stringify({err: "viewer not ready / no fskel"});
    const n = v.fskelBoneCount, stride = n * 7;
    const frames = Math.floor(v.fskelPoses.length / stride);
    const out = { act: v.actName, curFrame: v.diag.i0, bones: n, frames,
      anchored: [...v.anchoredRoots].map(b => v.skelBones[b].name + "(#" + b + ")"),
      mirror: [...v.mirrorBones].map(b => v.skelBones[b].name), chains: [] };

    const isBone = b => v.isBoneHelperName(b);
    const roots = [];
    for (let b = 0; b < n; b++) if (isBone(b)) { const p = v.skelParents[b]; if (p < 0 || !isBone(p)) roots.push(b); }
    const chainOf = new Map();
    for (const r of roots) for (const m of v.boneChainMembers(r)) chainOf.set(m, r);

    // 每条链收集原始顶点 {path, p, si}
    const vertsOf = new Map(roots.map(r => [r, []]));
    for (const [path, p] of v.parts) {
      for (let si = 0; si < p.model.vertices.length; si++) {
        for (const inf of p.model.vertices[si].influences) {
          const r = chainOf.get(inf.boneIndex);
          if (r !== undefined) { vertsOf.get(r).push({ path: path.split(/[\\\\/]/).pop(), p, si }); break; }
        }
      }
    }

    // 腿胶囊（随帧 W）
    const legCaps = v.legCaps.map(c => ({ bone: c.bone, len: c.len, name: v.skelBones[c.bone]?.name }));
    const capAt = (W, c) => {
      const e = W[c.bone].elements;
      return { ax: e[12], ay: e[13], az: e[14], bx: e[12]+e[0]*c.len, by: e[13]+e[1]*c.len, bz: e[14]+e[2]*c.len, r: c.len*0.18 };
    };
    const pen = (x, y, z, caps) => {
      let worst = -1e9, at = "";
      for (const c of caps) {
        const abx=c.bx-c.ax, aby=c.by-c.ay, abz=c.bz-c.az;
        const ab2=abx*abx+aby*aby+abz*abz;
        let t = ab2>1e-9 ? ((x-c.ax)*abx+(y-c.ay)*aby+(z-c.az)*abz)/ab2 : 0;
        t = Math.min(Math.max(t,0),1);
        const d = Math.hypot(x-(c.ax+abx*t), y-(c.ay+aby*t), z-(c.az+abz*t));
        const need = c.r + 0.25 - d;
        if (need > worst) { worst = need; at = c.name; }
      }
      return { worst, at };
    };

    // 列主序 4x4 乘法（页面无全局 THREE）
    const mul = (A, B) => {
      const C = new Float64Array(16);
      for (let c = 0; c < 4; c++) for (let r2 = 0; r2 < 4; r2++) {
        let s = 0;
        for (let k = 0; k < 4; k++) s += A[k * 4 + r2] * B[c * 4 + k];
        C[c * 4 + r2] = s;
      }
      return C;
    };
    for (const r of roots) {
      const arr = vertsOf.get(r);
      if (!arr.length) continue;
      let mnx=1e9,mny=1e9,mnz=1e9,mxx=-1e9,mxy=-1e9,mxz=-1e9,cx=0,cy=0,cz=0;
      for (const {p, si} of arr) {
        const s = p.model.vertices[si].pos;
        mnx=Math.min(mnx,s[0]); mny=Math.min(mny,s[1]); mnz=Math.min(mnz,s[2]);
        mxx=Math.max(mxx,s[0]); mxy=Math.max(mxy,s[1]); mxz=Math.max(mxz,s[2]);
        cx+=s[0]; cy+=s[1]; cz+=s[2];
      }
      const info = { name: v.skelBones[r].name, idx: r, verts: arr.length,
        bind: { min:[+mnx.toFixed(1),+mny.toFixed(1),+mnz.toFixed(1)], max:[+mxx.toFixed(1),+mxy.toFixed(1),+mxz.toFixed(1)],
                c:[+(cx/arr.length).toFixed(1),+(cy/arr.length).toFixed(1),+(cz/arr.length).toFixed(1)] },
        parts: [...new Set(arr.map(a=>a.path))],
        anchored: v.anchoredRoots.has(r) };
      // 全区间采样：纯LBS vs 锚定 的穿透 + 链根世界摆动
      const step = Math.max(1, Math.floor(frames / 45));
      let lbsMax=0, lbsAt="", lbsBad=0, ancMax=0, ancBad=0, cnt=0;
      let rootBase=null, swingPos=0;
      const ac = v.anchoredChains.get(r);
      for (let f = 0; f < frames; f += step) {
        const W = v.worldsFromPoses(f);
        const caps = legCaps.map(c => capAt(W, c));
        // 链根摆动（纯LBS世界）
        const e = W[r].elements;
        const rp = [e[12], e[13], e[14]];
        if (!rootBase) rootBase = rp; else swingPos = Math.max(swingPos, Math.hypot(rp[0]-rootBase[0], rp[1]-rootBase[1], rp[2]-rootBase[2]));
        // 锚定矩阵
        let Amats = null;
        if (ac) {
          Amats = new Map();
          const carry = mul(W[ac.carrier].elements, ac.tpInv.elements);
          for (const [b, bind] of ac.bindOf) Amats.set(b, mul(carry, bind.elements));
        }
        for (const {p, si} of arr) {
          const infl = p.inflOf[si];
          let lx=0,ly=0,lz=0, ax2=0,ay2=0,az2=0, okL=true, okA=!!Amats;
          for (const inf of infl) {
            if (inf.boneIndex >= n) { okL = false; okA = false; continue; }
            const w = inf.weight, lp = inf.boneLocalPos;
            const M = W[inf.boneIndex].elements;
            lx += w*(M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
            ly += w*(M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
            lz += w*(M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
            const A = Amats?.get(inf.boneIndex);
            if (A) {
              const E = A;
              ax2 += w*(E[0]*lp[0]+E[4]*lp[1]+E[8]*lp[2]+E[12]);
              ay2 += w*(E[1]*lp[0]+E[5]*lp[1]+E[9]*lp[2]+E[13]);
              az2 += w*(E[2]*lp[0]+E[6]*lp[1]+E[10]*lp[2]+E[14]);
            } else {
              ax2 += w*(M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
              ay2 += w*(M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
              az2 += w*(M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
            }
          }
          cnt++;
          if (okL) { const q = pen(lx,ly,lz,caps); if (q.worst>lbsMax){lbsMax=q.worst;lbsAt=q.at;} if (q.worst>0.02) lbsBad++; }
          if (okA) { const q = pen(ax2,ay2,az2,caps); if (q.worst>ancMax){ancMax=q.worst;} if (q.worst>0.02) ancBad++; }
        }
      }
      info.swingPos = +swingPos.toFixed(1);
      info.pureLBS = { maxPen: +lbsMax.toFixed(2), at: lbsAt, bad: lbsBad, samples: cnt };
      if (ac) info.anchoredMode = { maxPen: +ancMax.toFixed(2), bad: ancBad };
      // |LBS − 锚定摆放| 失配（每 100 帧窗口 avg，取最大窗口）：
      // 两者的链外影响都走 W，差值只隔离链数据错位（身体位移被抵消）
      if (ac) {
        let wm = null;
        const winAvg = [];
        for (let f = 0; f < frames; f += step) {
          if (!wm) wm = { f0: f, sum: 0, cnt: 0 };
          const W = v.worldsFromPoses(f);
          const carry = mul(W[ac.carrier].elements, ac.tpInv.elements);
          const Amats = new Map();
          for (const [b, bind] of ac.bindOf) Amats.set(b, mul(carry, bind.elements));
          for (const {p, si} of arr) {
            const infl = p.inflOf[si];
            let lx=0,ly=0,lz=0, ax2=0,ay2=0,az2=0;
            for (const inf of infl) {
              if (inf.boneIndex >= n) continue;
              const w = inf.weight, lp = inf.boneLocalPos;
              const M = W[inf.boneIndex].elements;
              lx += w*(M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
              ly += w*(M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
              lz += w*(M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
              const A = Amats.get(inf.boneIndex);
              if (A) {
                ax2 += w*(A[0]*lp[0]+A[4]*lp[1]+A[8]*lp[2]+A[12]);
                ay2 += w*(A[1]*lp[0]+A[5]*lp[1]+A[9]*lp[2]+A[13]);
                az2 += w*(A[2]*lp[0]+A[6]*lp[1]+A[10]*lp[2]+A[14]);
              } else {
                ax2 += w*(M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
                ay2 += w*(M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
                az2 += w*(M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
              }
            }
            wm.sum += Math.hypot(lx-ax2, ly-ay2, lz-az2); wm.cnt++;
          }
          if (f - wm.f0 >= 99 || f + step >= frames) {
            if (wm.cnt) winAvg.push({ from: wm.f0, avg: +(wm.sum/wm.cnt).toFixed(1) });
            wm = null;
          }
        }
        winAvg.sort((a,b) => b.avg - a.avg);
        info.mismatchWin = winAvg.slice(0, 4);
      }
      out.chains.push(info);
    }
    return JSON.stringify(out);
  })()`;
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  const val = r.result?.result?.value;
  try {
    const j = JSON.parse(val);
    console.log(JSON.stringify(j, null, 1));
  } catch {
    console.log(r.result?.result?.value ?? JSON.stringify(r.result).slice(0, 800));
  }
  ws.close();
  process.exit(0);
});
setTimeout(() => {
  console.error("timeout");
  process.exit(1);
}, 30000);
