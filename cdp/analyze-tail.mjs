// 合并分析 tail 日志：按动作分组统计诊断明细与穿模告警
import { readFileSync } from "node:fs";

const file = process.argv[2];
const lines = readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean);

const actions = new Map(); // name -> {logs: [], warns: [] }
for (const line of lines) {
  const m = line.match(/^\[(LOG |WARN|ERR )\] (.+?): (.*)$/);
  if (!m) continue;
  const [, levelRaw, name, msg] = m;
  const level = levelRaw.trim();
  let a = actions.get(name);
  if (!a) actions.set(name, (a = { logs: [], warns: [], skirt: [], cloth: [] }));
  if (msg.startsWith("裙摆防穿模")) {
    a.skirt.push(msg);
  } else if (msg.startsWith("布料模拟")) {
    a.cloth.push(msg);
  } else if (level === "WARN") {
    a.warns.push(msg);
  } else {
    a.logs.push(msg);
  }
}

for (const [name, a] of actions) {
  console.log(`\n===== ${name} =====`);
  for (const l of a.logs) console.log("  " + l.slice(0, 400));
  for (const w of a.warns) console.log("  [WARN] " + w.slice(0, 400));
  // 裙摆告警合并：穿透值范围 + 涉及骨 + 迭代用尽帧数范围 + 钳制次数
  if (a.skirt.length) {
    const deep = a.skirt.map((s) => {
      const pen = Number((s.match(/最大穿透 ([\d.]+)/) ?? [])[1]);
      const bone = (s.match(/\(骨 ([^)]+)\)/) ?? [])[1] ?? "?";
      const iter = Number((s.match(/迭代用尽仍有穿透 (\d+) 帧/) ?? [])[1] ?? 0);
      const clamp = Number((s.match(/推出被钳\(≥6\) (\d+) 骨次/) ?? [])[1] ?? 0);
      return { pen, bone, iter, clamp };
    });
    const pens = deep.map((d) => d.pen);
    const bones = [...new Set(deep.map((d) => d.bone))];
    const boneCount = {};
    for (const d of deep) boneCount[d.bone] = (boneCount[d.bone] ?? 0) + 1;
    const iters = deep.map((d) => d.iter);
    const clamps = deep.reduce((s, d) => s + d.clamp, 0);
    console.log(
      `  [裙摆告警×${a.skirt.length}] 穿透 ${Math.min(...pens).toFixed(2)}~${Math.max(...pens).toFixed(2)} · 骨 ${Object.entries(boneCount).map(([b, c]) => `${b}×${c}`).join(",")} · 迭代用尽 ${Math.min(...iters)}~${Math.max(...iters)} 帧/窗口 · 钳制骨次合计 ${clamps}`
    );
  }
  if (a.cloth.length) {
    const seeks = a.cloth.reduce((s, c) => s + Number((c.match(/seek重置 (\d+)/) ?? [])[1] ?? 0), 0);
    const vels = a.cloth.reduce((s, c) => s + Number((c.match(/速度限幅 (\d+)/) ?? [])[1] ?? 0), 0);
    const safes = a.cloth.reduce((s, c) => s + Number((c.match(/安全钳位 (\d+)/) ?? [])[1] ?? 0), 0);
    const lags = a.cloth.reduce((s, c) => s + Number((c.match(/滞后钳位 (\d+)/) ?? [])[1] ?? 0), 0);
    console.log(
      `  [布料模拟×${a.cloth.length}] seek ${seeks} · 速度限幅 ${vels} · 安全钳位 ${safes} · 滞后钳位 ${lags}`
    );
  }
}
