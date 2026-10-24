// 从文件读 JS 表达式经 CDP 求值：node evalfile.mjs <expr-file>
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
const expr = readFileSync(process.argv[2], "utf8").trim();
console.log(execFileSync("node", ["drive.mjs", expr], { encoding: "utf8" }));
