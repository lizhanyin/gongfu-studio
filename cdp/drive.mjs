// CDP 驱动器（WebView2 需带 --remote-debugging-port 启动，端口由 CDP_PORT 或默认 9333）：
//   node drive.mjs "<js 表达式>"   一次性求值并打印结果
//   node drive.mjs --diag [N]      取 window.__diagLog 最近 N 条（默认 100，含动作诊断日志历史）
//   node drive.mjs --tail          实时打印页面 console 输出（Runtime.consoleAPICalled）
// 注意 9222 落在本机 Windows 保留端口段 9181-9280 内无法绑定，故用 9333。
import WebSocket from "ws";
import { request } from "node:http";

const CDP_PORT = Number(process.env.CDP_PORT) || 9333;

function getJson(path) {
  return new Promise((resolve, reject) => {
    const req = request(
      { host: "127.0.0.1", port: CDP_PORT, path, method: "GET" },
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

const args = process.argv.slice(2);
const mode = args[0] === "--diag" ? "diag" : args[0] === "--tail" ? "tail" : "eval";
const expr = mode === "eval" ? args[0] : null;
if (mode === "eval" && !expr) {
  console.error('usage: node drive.mjs "<expr>" | --diag [N] | --tail');
  console.error("start app with: WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222");
  process.exit(2);
}

let targets;
try {
  targets = await getJson("/json");
} catch (e) {
  console.error(`cannot reach CDP 127.0.0.1:${CDP_PORT} (${e.message})`);
  console.error("start the app with remote debugging enabled, e.g. Git Bash:");
  console.error(`  WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=${CDP_PORT}" ./target/debug/Gongfu-Studio.exe &`);
  process.exit(1);
}
// 优先 app 页面（tauri.localhost），跳过 DevTools 前端自身的 devtools:// target
const page =
  targets.find((t) => t.type === "page" && /tauri\.localhost/.test(t.url)) ??
  targets.find((t) => t.type === "page" && !t.url.startsWith("devtools://")) ??
  targets.find((t) => t.type === "page");
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
    return;
  }
  // --tail：页面 console 输出（级别已在文本内，如 "动作名: ..."）
  if (mode === "tail" && msg.method === "Runtime.consoleAPICalled") {
    const { type, args } = msg.params;
    const text = args.map((a) => (a.value !== undefined ? String(a.value) : a.description ?? "")).join(" ");
    const tag = type === "warning" ? "WARN" : type === "error" ? "ERR " : "LOG ";
    console.log(`[${tag}] ${text}`);
  }
});
ws.on("error", (e) => {
  console.error("WS ERR", e.message);
  process.exit(1);
});
ws.on("open", async () => {
  await send("Runtime.enable");
  if (mode === "tail") {
    console.error("[tail] connected, streaming console (Ctrl+C to stop)...");
    return; // 长驻
  }
  const expression =
    mode === "diag"
      ? `(window.__diagLog ?? []).slice(-${Number(args[1]) || 100}).map(e => new Date(e.t).toLocaleTimeString() + " " + (e.level === "warn" ? "WARN " : "") + e.msg).join("\\n") || "(no diag entries yet)"`
      : expr;
  const r = await send("Runtime.evaluate", {
    expression,
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
if (mode !== "tail") {
  setTimeout(() => {
    console.error("timeout");
    process.exit(1);
  }, 15000);
}
