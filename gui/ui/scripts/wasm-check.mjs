/**
 * fmath wasm 工具链自检（run.bat / run-dev.bat / run.sh 的依赖步骤调用）：
 *   npm run wasm-check
 * - rustup 的 wasm32-unknown-unknown 目标缺失时自动补装（幂等，rustup 源可达即可）
 * - wasm-pack 缺失时报错并给出安装指引（cargo install 或 npm 分发二进制）
 */
import { execSync } from "node:child_process";

function run(cmd, stdio = "pipe") {
  return execSync(cmd, { stdio, encoding: "utf8" });
}

let failed = false;

try {
  const ver = run("wasm-pack --version").trim();
  console.log(`[wasm-check] ${ver}`);
} catch (e) {
  console.error(`[wasm-check] wasm-pack 不可用（${e.message.split("\n")[0]}），安装任选其一：`);
  console.error("  cargo install wasm-pack   （Rust 工具链安装，推荐）");
  console.error("  npm install -D wasm-pack  （npm 分发二进制，需能访问 GitHub releases）");
  failed = true;
}

try {
  const installed = run("rustup target list --installed");
  if (!installed.includes("wasm32-unknown-unknown")) {
    console.log("[wasm-check] 缺 wasm32-unknown-unknown 编译目标，自动 rustup target add ...");
    run("rustup target add wasm32-unknown-unknown", "inherit");
  }
} catch {
  console.error("[wasm-check] rustup 不可用（Tauri 开发同样需要），请先安装 Rust 工具链");
  failed = true;
}

process.exit(failed ? 1 : 0);
