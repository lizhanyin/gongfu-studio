// 逐像素差分：mip 版相对 no-mip 版「变暗」的像素 = mip 渗色直接度量
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";

function decodePng(buf) {
  let pos = 8;
  let w = 0, h = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") { w = data.readUInt32BE(0); h = data.readUInt32BE(4); }
    else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    pos += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = w * 4;
  const out = new Uint8Array(w * h * 4);
  const paeth = (a, b, c) => {
    const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
    return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
  };
  let rp = 0;
  for (let y = 0; y < h; y++) {
    const f = raw[rp++];
    const row = raw.subarray(rp, rp + stride);
    rp += stride;
    const prev = y > 0 ? out.subarray((y - 1) * stride, y * stride) : null;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= 4 ? cur[x - 4] : 0;
      const b = prev ? prev[x] : 0;
      const c = x >= 4 && prev ? prev[x - 4] : 0;
      let val = row[x];
      if (f === 1) val += a;
      else if (f === 2) val += b;
      else if (f === 3) val += (a + b) >> 1;
      else if (f === 4) val += paeth(a, b, c);
      cur[x] = val & 0xff;
    }
  }
  return { w, h, rgba: out };
}

const a = decodePng(readFileSync(process.argv[2])); // no-mip
const b = decodePng(readFileSync(process.argv[3])); // mip
const { w, h } = a;
let darker = 0, lighter = 0;
const dmask = [];
for (let i = 0; i < w * h; i++) {
  const o = i * 4;
  const la = 0.3 * a.rgba[o] + 0.59 * a.rgba[o + 1] + 0.11 * a.rgba[o + 2];
  const lb = 0.3 * b.rgba[o] + 0.59 * b.rgba[o + 1] + 0.11 * b.rgba[o + 2];
  const d = lb - la;
  if (d < -12) { darker++; dmask.push(i); }
  else if (d > 12) lighter++;
}
let minX = 1e9, maxX = -1, minY = 1e9, maxY = -1;
for (const i of dmask) {
  const x = i % w, y = (i / w) | 0;
  if (x < minX) minX = x; if (x > maxX) maxX = x;
  if (y < minY) minY = y; if (y > maxY) maxY = y;
}
console.log(JSON.stringify({ size: [w, h], darker, lighter, darkerBBox: dmask.length ? [minX, minY, maxX, maxY] : null }));
