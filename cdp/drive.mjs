// CDP Runtime.evaluate 驱动器：node drive.mjs "<js 表达式>"
import WebSocket from "ws";
import { request } from "node:http";

function getJson(path) {
  return new Promise((resolve, reject) => {
    const req = request(
      { host: "127.0.0.1", port: 9222, path, method: "GET" },
      (res) => {
        let data = "";
        res.on("data", (c) => (data += c));
        res.on("end", () => resolve(JSON.parse(data)));
      }
    );
    req.on("error", reject);
    req.end();
  });
}

const expr = process.argv[2];
if (!expr) {
  console.error("usage: node drive.mjs <expr>");
  process.exit(2);
}
const targets = await getJson("/json");
const page = targets.find((t) => t.type === "page");
if (!page) {
  console.error("no page target");
  process.exit(1);
}
const ws = new WebSocket(page.webSocketDebuggerUrl, { perMessageDeflate: false });
let id = 0;
const pending = new Map();
function send(method, params) {
  return new Promise((resolve, reject) => {
    const mid = ++id;
    pending.set(mid, { resolve, reject });
    ws.send(JSON.stringify({ id: mid, method, params }));
  });
}
ws.on("message", (raw) => {
  const msg = JSON.parse(raw.toString());
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id).resolve(msg);
    pending.delete(msg.id);
  }
});
ws.on("open", async () => {
  await send("Runtime.enable");
  const r = await send("Runtime.evaluate", {
    expression: expr,
    awaitPromise: true,
    returnByValue: true,
  });
  if (r.result?.exceptionDetails) {
    console.error("EXC", JSON.stringify(r.result.exceptionDetails, null, 1).slice(0, 2000));
  } else {
    const v = r.result?.result?.value;
    console.log(typeof v === "string" ? v : JSON.stringify(v, null, 1));
  }
  ws.close();
  process.exit(0);
});
ws.on("error", (e) => {
  console.error("WS ERR", e.message);
  process.exit(1);
});
setTimeout(() => {
  console.error("timeout");
  process.exit(1);
}, 15000);
