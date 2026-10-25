// A/B 对比两张 grab-frame 截图：绿色羽翼邻接的暗线像素计数（mip 渗色指标）
// PNG 解码：仅支持 RGBA8888（canvas toDataURL 产物），zlib inflate + 逐行 unfilter
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";

function decodePng(buf) {
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error("not png");
  let pos = 8;
  let w = 0, h = 0, bitDepth = 0, colorType = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString("ascii", pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === "IHDR") {
      w = data.readUInt32BE(0);
      h = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
    } else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    pos += 12 + len;
  }
  if (bitDepth !== 8 || colorType !== 6) throw new Error(`unsupported ${bitDepth}/${colorType}`);
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

function darkNextToGreen(img) {
  const { w, h, rgba } = img;
  const isGreen = (x, y) => {
    const o = (y * w + x) * 4;
    const r = rgba[o], g = rgba[o + 1], b = rgba[o + 2];
    return g > 60 && g > r + 25 && g > b + 25;
  };
  const isDark = (x, y) => {
    const o = (y * w + x) * 4;
    return 0.3 * rgba[o] + 0.59 * rgba[o + 1] + 0.11 * rgba[o + 2] < 45;
  };
  let dark = 0;
  const darkAdj = new Set();
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++)
      if (isGreen(x, y))
        for (let dy = -2; dy <= 2; dy++)
          for (let dx = -2; dx <= 2; dx++) {
            const nx = x + dx, ny = y + dy;
            if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
            if (isDark(nx, ny)) darkAdj.add(ny * w + nx);
          }
  return { greenPx: (img.greenPx = 0, undefined), darkAdjCount: darkAdj.size };
}

const a = decodePng(readFileSync(process.argv[2]));
const b = decodePng(readFileSync(process.argv[3]));
const ra = darkNextToGreen(a);
const rb = darkNextToGreen(b);
console.log(JSON.stringify({
  a: { file: process.argv[2], ...ra },
  b: { file: process.argv[3], ...rb },
}));
