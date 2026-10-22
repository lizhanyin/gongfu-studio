import type { TexData } from "./viewer";

// 从 viewer.ts 剥出的纹理解码 helper：App 与 3 个模式 composable 需要运行时引用它。
// 若留在 viewer.ts，这些引用会把 viewer.ts 的 HMR 边界顶到 App.vue（整窗口重挂），
// 破坏 components/ViewerHost.vue 的“改 viewer.ts 只重建视口”。
// viewer.ts 里的同名函数与 b64ToBytes 已无引用，可删。
export function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function texDataFromDto(dto: {
  width: number;
  height: number;
  rgbaBase64: string;
}): TexData {
  return {
    width: dto.width,
    height: dto.height,
    rgba: b64ToBytes(dto.rgbaBase64),
  };
}

/** 浏览器可原生解码的图片（png/jpg/bmp/gif/webp）：data URL + 解码取宽高 */
export function imageUrlFromB64(
  b64: string,
  ext: string,
): Promise<{ url: string; width: number; height: number }> {
  const url = `data:${ext === "jpg" ? "image/jpeg" : `image/${ext}`};base64,${b64}`;
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () =>
      resolve({ url, width: img.naturalWidth, height: img.naturalHeight });
    img.onerror = () => reject(new Error(`webview 无法解码 .${ext} 图片`));
    img.src = url;
  });
}

/**
 * TGA 解码（Rust 侧只解游戏 DDS，TGA 在前端解）：
 * 真彩 24/32bit、灰度 8bit，未压缩与 RLE 均支持；
 * 原点在左下（descriptor 无 0x20 位）时翻转为自顶向下。色表 / 16bit 不支持。
 */
export function decodeTga(bytes: Uint8Array): TexData {
  if (bytes.length < 18) throw new Error("TGA 头不完整");
  const cmapType = bytes[1];
  const type = bytes[2];
  const w = bytes[12] | (bytes[13] << 8);
  const h = bytes[14] | (bytes[15] << 8);
  const depth = bytes[16];
  if (w <= 0 || h <= 0) throw new Error(`TGA 尺寸无效 ${w}×${h}`);
  if (cmapType !== 0) throw new Error("不支持色表 TGA");
  const rle = type === 10 || type === 11;
  if (type !== 2 && type !== 3 && type !== 10 && type !== 11) {
    throw new Error(`不支持的 TGA 类型 ${type}`);
  }
  const bpp = depth >> 3;
  if (bpp !== 1 && bpp !== 3 && bpp !== 4) {
    throw new Error(`不支持的 TGA 位深 ${depth}`);
  }
  let p = 18 + bytes[0]; // 跳过 image id
  const n = w * h;
  const px = new Uint8Array(n * 4);
  const pxAt = (o: number): number[] => {
    if (o + bpp > bytes.length) throw new Error("TGA 数据不完整");
    if (bpp === 1) return [bytes[o], bytes[o], bytes[o], 255];
    // TGA 真彩字节序为 BGR(A)
    return [bytes[o + 2], bytes[o + 1], bytes[o], bpp === 4 ? bytes[o + 3] : 255];
  };
  let i = 0;
  const put = (v: number[]) => {
    px[i++] = v[0];
    px[i++] = v[1];
    px[i++] = v[2];
    px[i++] = v[3];
  };
  if (!rle) {
    for (let k = 0; k < n; k++) put(pxAt(p + k * bpp));
  } else {
    for (let k = 0; k < n; ) {
      if (p >= bytes.length) throw new Error("TGA 数据不完整");
      const hdr = bytes[p++];
      const cnt = (hdr & 0x7f) + 1;
      if (hdr & 0x80) {
        const v = pxAt(p); // RLE 包：cnt 个相同像素
        p += bpp;
        for (let c = 0; c < cnt; c++) put(v);
      } else {
        for (let c = 0; c < cnt; c++) {
          put(pxAt(p)); // raw 包：cnt 个字面像素
          p += bpp;
        }
      }
      k += cnt;
    }
  }
  if (!(bytes[17] & 0x20)) {
    // 原点左下 → 垂直翻转到自顶向下
    const row = w * 4;
    const tmp = new Uint8Array(row);
    for (let y = 0; y < h >> 1; y++) {
      const a = y * row;
      const b = (h - 1 - y) * row;
      tmp.set(px.subarray(a, a + row));
      px.copyWithin(a, b, b + row);
      px.set(tmp, b);
    }
  }
  return { width: w, height: h, rgba: px };
}
