/**
 * fmath wasm 内核装载（模型计算已迁入 Rust：仓库根 fmath crate）。
 * 模块加载即 await init()——依赖本模块的代码（viewer.ts）在 wasm 就绪后才执行。
 *
 * 回退开关（便于 A/B 数值对比与应急）：URL 加 ?gfm=ts，或 localStorage
 * 置 gfm_off=1 后刷新 → gfmEnabled=false，viewer 走内置 TS 实现路径。
 */
import init, { Gfm } from "./wasm/fmath.js";

await init();

export const gfmEnabled = !(
  new URLSearchParams(location.search).get("gfm") === "ts" ||
  localStorage.getItem("gfm_off") === "1"
);
export const gfm: Gfm | null = gfmEnabled ? new Gfm() : null;
