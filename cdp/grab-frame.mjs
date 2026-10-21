// 抓取 viewer WebGL canvas 当前帧保存 PNG
import WebSocket from "ws";
import { request } from "node:http";
import { writeFileSync } from "node:fs";

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;
const out = process.argv[2] ?? "../target/std8_frame.png";

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
  const r = await send("Runtime.evaluate", {
    expression: `new Promise(res => requestAnimationFrame(() => res(document.querySelector('canvas').toDataURL('image/png'))))`,
    awaitPromise: true,
    returnByValue: true,
  });
  const url = r.result?.result?.value;
  writeFileSync(out, Buffer.from(url.slice(url.indexOf(",") + 1), "base64"));
  console.log("saved", out);
  ws.close();
  process.exit(0);
});
setTimeout(() => { console.error("timeout"); process.exit(1); }, 15000);
