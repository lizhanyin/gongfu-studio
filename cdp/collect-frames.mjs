// 逐帧渲染状态采集：node collect-frames.mjs <f0>-<f1> [--shots] [--out 前缀]
// 前提：viewer 已加载部件与动作（skirtRecord 探针需新构建）。
// 每帧采集：去穿透推出日志（阶段/顶点/幅度/胶囊）、每裙顶点 最终位置 vs 纯LBS
// 位移、帧间抖动（同顶点位移的帧间变化）。--shots 额外逐帧抓 WebGL PNG。
// 输出：target/<前缀>frame-report-<f0>-<f1>.json + 控制台摘要表。
import WebSocket from "ws";
import { request } from "node:http";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// 输出定位到仓库 target/（相对脚本位置，任意 CWD 可运行）
const TARGET = join(dirname(fileURLToPath(import.meta.url)), "..", "target");

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;
const args = process.argv.slice(2);
const m = args.find((a) => /^\d+-\d+$/.test(a));
if (!m) {
  console.error("usage: node collect-frames.mjs <f0>-<f1> [--shots] [--out prefix]");
  process.exit(2);
}
const [f0, f1] = m.split("-").map(Number);
const shots = args.includes("--shots");
const outArg = args.find((a) => a.startsWith("--out="));
const prefix = outArg ? outArg.slice(6) : "";

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
// tauri.localhost=打包构建；localhost:1420=dev 模式（tauri dev / run-dev.bat）
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

// 页面内采集表达式：循环帧 → 记录推出日志 + 每顶点位移（vs 纯LBS重算）
const collectExpr = (a, b) => `(() => {
  const v = window.__viewer;
  const n = v.fskelBoneCount;
  v.skirtRecord = true;
  const out = [];
  for (let f = ${a}; f <= ${b}; f++) {
    v.applyFskelFrame(f, f, 0);
    const pushes = v.skirtPushLog.map(p => [p.v, p.phase, +p.k.toFixed(3), p.cap ?? ""]);
    const disp = [];
    for (const sv of v.skirtVerts) {
      const pa = sv.p.geom.getAttribute("position").array;
      const o = sv.i * 3;
      let x = 0, y = 0, z = 0;
      let ok = true;
      for (const inf of sv.p.inflOf[sv.i]) {
        if (inf.boneIndex >= n) { ok = false; continue; }
        // 锚定/模拟链骨取当前渲染矩阵（与 skinPart 同源）
        const M = (v.anchoredMats.size && v.anchoredMats.has(inf.boneIndex)
          ? v.anchoredMats.get(inf.boneIndex) : v.worldMats[inf.boneIndex]).elements;
        const lp = inf.boneLocalPos;
        x += inf.weight * (M[0]*lp[0]+M[4]*lp[1]+M[8]*lp[2]+M[12]);
        y += inf.weight * (M[1]*lp[0]+M[5]*lp[1]+M[9]*lp[2]+M[13]);
        z += inf.weight * (M[2]*lp[0]+M[6]*lp[1]+M[10]*lp[2]+M[14]);
      }
      disp.push(ok ? [
        +(pa[o]-x).toFixed(3), +(pa[o+1]-y).toFixed(3), +(pa[o+2]-z).toFixed(3)
      ] : [0,0,0]);
    }
    out.push({ f, pushes, disp });
  }
  v.skirtRecord = false;
  return JSON.stringify(out);
})()`;

ws.on("open", async () => {
  await send("Runtime.enable");
  const state = await send("Runtime.evaluate", {
    expression: `JSON.stringify({act: __viewer.actName, verts: __viewer.skirtVerts.length, frames: __viewer.fskelPoses.length/__viewer.fskelBoneCount/7})`,
    returnByValue: true,
  });
  const st = JSON.parse(state.result?.result?.value ?? "{}");
  console.log(`动作=${st.act} 裙顶点=${st.verts} 总帧=${st.frames} 采集 ${f0}..${f1}`);

  const frames = [];
  const CHUNK = 120;
  for (let a = f0; a <= f1; a += CHUNK) {
    const b = Math.min(a + CHUNK - 1, f1);
    const r = await send("Runtime.evaluate", { expression: collectExpr(a, b), returnByValue: true });
    const val = r.result?.result?.value;
    if (!val) { console.error("采集失败:", JSON.stringify(r.result).slice(0, 300)); process.exit(1); }
    frames.push(...JSON.parse(val));
  }

  // 分析：每帧摘要 + 帧间抖动（同顶点位移向量变化）
  const len = (d) => Math.hypot(d[0], d[1], d[2]);
  const report = frames.map((fr, i) => {
    const p1 = fr.pushes.filter((p) => p[1] === 1);
    const p2 = fr.pushes.filter((p) => p[1] === 2);
    const p3 = fr.pushes.filter((p) => p[1] === 3);
    const mags = fr.disp.map(len);
    const maxDisp = Math.max(...mags);
    let maxJit = 0, jitV = -1;
    if (i > 0) {
      const prev = frames[i - 1].disp;
      fr.disp.forEach((d, vi) => {
        const j = Math.hypot(d[0] - prev[vi][0], d[1] - prev[vi][1], d[2] - prev[vi][2]);
        if (j > maxJit) { maxJit = j; jitV = vi; }
      });
    }
    return {
      f: fr.f,
      p1: p1.length, p2: p2.length, p3: p3.length,
      maxK: +Math.max(0, ...fr.pushes.map((p) => p[2])).toFixed(2),
      maxDisp: +maxDisp.toFixed(2),
      maxDispV: mags.indexOf(maxDisp),
      jitter: +maxJit.toFixed(2),
      jitterV: jitV,
    };
  });

  mkdirSync(TARGET, { recursive: true });
  const outPath = join(TARGET, `${prefix}frame-report-${f0}-${f1}.json`);
  writeFileSync(outPath, JSON.stringify({ act: st.act, f0, f1, frames, report }, null, 1));
  console.log(`已写入 ${outPath}\n`);
  console.log("帧   p1  p2  p3  maxK  位移max(v#)  帧间抖动max(v#)");
  for (const r of report) {
    const flag = r.jitter > 1 ? " ◀抖动" : "";
    console.log(
      `${String(r.f).padEnd(4)} ${String(r.p1).padEnd(3)} ${String(r.p2).padEnd(3)} ${String(r.p3).padEnd(3)} ${String(r.maxK).padEnd(5)} ${r.maxDisp.toFixed(2).padStart(5)}(v${r.maxDispV})  ${r.jitter.toFixed(2).padStart(5)}(v${r.jitterV})${flag}`
    );
  }

  if (shots) {
    const shotDir = join(TARGET, "frameshots");
    mkdirSync(shotDir, { recursive: true });
    for (const fr of frames) {
      const r = await send("Runtime.evaluate", {
        expression: `(() => { __viewer.applyFskelFrame(${fr.f}, ${fr.f}, 0); return new Promise(res => requestAnimationFrame(() => res(document.querySelector('canvas').toDataURL('image/png')))); })()`,
        awaitPromise: true,
        returnByValue: true,
      });
      const url = r.result?.result?.value;
      if (url) {
        writeFileSync(join(shotDir, `${prefix}f${fr.f}.png`), Buffer.from(url.slice(url.indexOf(",") + 1), "base64"));
      }
    }
    console.log(`\n截图已写入 ${shotDir} (${frames.length} 帧)`);
  }
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 300000);
