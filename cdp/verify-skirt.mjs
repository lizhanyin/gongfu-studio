// 裙摆去穿透数值验证：node verify-skirt.mjs — 采样当前帧所有裙顶点 vs 腿胶囊残余穿透
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
    if (!v || !v.legCaps || !v.legCaps.length) return JSON.stringify({err: "viewer not ready"});
    const caps = v.legCaps.map(c => {
      const e = v.worldMats[c.bone].elements;
      return { ax: e[12], ay: e[13], az: e[14], bx: e[12]+e[0]*c.len, by: e[13]+e[1]*c.len, bz: e[14]+e[2]*c.len, r: c.len*0.18, name: v.skelBones[c.bone] ? v.skelBones[c.bone].name : String(c.bone) };
    });
    let worst = -1e9, bad = 0, worstAt = "";
    for (const sv of v.skirtVerts) {
      const pa = sv.p.geom.getAttribute("position").array;
      const o = sv.i * 3;
      for (const c of caps) {
        const abx = c.bx-c.ax, aby = c.by-c.ay, abz = c.bz-c.az;
        const ab2 = abx*abx+aby*aby+abz*abz;
        let t = ab2 > 1e-9 ? ((pa[o]-c.ax)*abx+(pa[o+1]-c.ay)*aby+(pa[o+2]-c.az)*abz)/ab2 : 0;
        t = Math.min(Math.max(t,0),1);
        const d = Math.hypot(pa[o]-(c.ax+abx*t), pa[o+1]-(c.ay+aby*t), pa[o+2]-(c.az+abz*t));
        const need = c.r + 0.25 - d;
        if (need > worst) { worst = need; worstAt = c.name; }
        if (need > 0.02) bad++;
      }
    }
    return JSON.stringify({frame: v.diag.i0, skirtVerts: v.skirtVerts.length, anchored: v.anchoredMats.size, residualPen: +worst.toFixed(3), badOver2cm: bad, at: worstAt});
  })()`;
  const r = await send("Runtime.evaluate", { expression: expr, returnByValue: true });
  console.log(r.result?.result?.value ?? JSON.stringify(r.result).slice(0, 500));
  ws.close();
  process.exit(0);
});
setTimeout(() => {
  console.error("timeout");
  process.exit(1);
}, 15000);
