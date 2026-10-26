import { api } from "./api";
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
 * 按扩展名加载贴图为 RGBA：DDS 走 Rust 解码，TGA 前端解，其余不支持。
 * 各模式（皮肤/FAK/FSM/装扮）的材质贴图加载统一走此口径——
 * 曾有 FAK/FSM 模式把 .tga 也交给 Rust DDS 解码导致贴图全黑/缺失
 * （system.cpk 的 NPC 模型材质引用 813 条为 .tga）。
 */
export async function loadTexData(file: string): Promise<TexData> {
  if (/\.dds$/i.test(file)) {
    return texDataFromDto(await api.loadTexture(file));
  }
  if (/\.tga$/i.test(file)) {
    return decodeTga(b64ToBytes(await api.readFileB64(file)));
  }
  const name = file.split(/[\\/]/).pop() ?? file;
  throw new Error(`暂不支持解码该格式贴图: ${name}`);
}

/** 同目录内可作贴图的扩展名（目录列举条目按 kind 过滤） */
export const TEX_KINDS = /^(dds|tga|png|jpg|bmp)$/;

/** TexData → canvas dataURL（检查器贴图预览缩略图用） */
export function texCanvasUrl(d: TexData): string {
  const cv = document.createElement("canvas");
  cv.width = d.width;
  cv.height = d.height;
  const ctx = cv.getContext("2d")!;
  const img = ctx.createImageData(d.width, d.height);
  img.data.set(d.rgba);
  ctx.putImageData(img, 0, 0);
  return cv.toDataURL();
}

/** 路径的祖先目录序列（含自身目录）：a\b\c.FAK → [a\b, a] */
function ancestorsOf(path: string): string[] {
  const out: string[] = [];
  let cur = path.replace(/[\\/][^\\/]*$/, "");
  while (cur && !out.includes(cur)) {
    out.push(cur);
    const up = cur.replace(/[\\/][^\\/]*$/, "");
    if (up === cur) break;
    cur = up;
  }
  return out;
}

export type ModelTexSlot = { tex: TexData | null; file: string | null };

/**
 * FAK/FSM/FSCN 单模型材质贴图解析（各模式共用，同「路径优先」口径）：
 * 1. 材质引用自带目录（system\npc\N302_3.tga、FSCN 的
 *    system\building\texture\G01-086-A.tga）时按模型路径祖先逐级拼接试读
 *    （CPK 内即 包根+引用 直达；vfs 查找大小写不敏感）；原名未命中时
 *    换 .dds 扩展名重试——引用按 .tga 发布、包内常为同名 .dds；
 * 2. 回落模型同目录 stem 匹配（引用 J01-S01.tga → 同目录 j01-s01.dds）。
 * 解码失败经 onDecodeError 回调上报；找不到贴图的槽位静默置 null（维持原口径）。
 */
export async function resolveModelTextures(
  modelPath: string,
  refs: string[],
  texCache: Map<string, TexData | null>,
  onDecodeError?: (file: string, e: unknown) => void,
): Promise<ModelTexSlot[]> {
  // 同目录列举一次；祖先路径候选失败才需要它（延迟到首次未命中再取）
  let sameDirEntries: { kind: string; name: string; path: string }[] | null = null;
  const sameDir = async () => {
    if (!sameDirEntries) {
      try {
        sameDirEntries = await api.listResDir(
          modelPath.replace(/[\\/][^\\/]*$/, "")
        );
      } catch {
        sameDirEntries = [];
      }
    }
    return sameDirEntries;
  };

  const memo = new Map<string, ModelTexSlot>();
  const out: ModelTexSlot[] = [];
  for (const ref of refs) {
    const cached = memo.get(ref);
    if (cached) {
      out.push(cached);
      continue;
    }
    let slot: ModelTexSlot = { tex: null, file: null };

    // 1. 路径优先：引用带目录部分才有意义（裸文件名拼祖先 = 同目录全扫，无增益）
    if (ref && /[\\/]/.test(ref)) {
      const norm = ref.replace(/[\\/]+/g, "\\");
      // 引用多按 .tga 发布而包内实为同名 .dds（FSCN 建筑贴图
      // building\texture 全量如此），原名未命中时换 .dds 重试
      const cands = /\.dds$/i.test(norm)
        ? [norm]
        : [norm, norm.replace(/\.[^.]*$/, "") + ".dds"];
      outer: for (const anc of ancestorsOf(modelPath)) {
        for (const c of cands) {
          const candidate = `${anc}\\${c}`;
          if (texCache.has(candidate)) {
            const tex = texCache.get(candidate) ?? null;
            if (tex) {
              slot = { tex, file: candidate };
              break outer;
            }
            continue; // 该候选曾解码失败，试下一个候选/祖先
          }
          try {
            const tex = await loadTexData(candidate);
            texCache.set(candidate, tex);
            slot = { tex, file: candidate };
            break outer;
          } catch {
            // 路径不存在（vfs 快速失败）或解码失败：继续试下一候选/祖先
          }
        }
      }
    }

    // 2. 同目录 stem 匹配
    if (!slot.tex && ref) {
      const stem = (ref.split(/[\\/]/).pop() ?? "")
        .replace(/\.[^.]*$/, "")
        .toLowerCase();
      const entry = (await sameDir()).find(
        (e) =>
          TEX_KINDS.test(e.kind) &&
          e.name.replace(/\.[^.]*$/, "").toLowerCase() === stem
      );
      if (entry) {
        if (!texCache.has(entry.path)) {
          try {
            texCache.set(entry.path, await loadTexData(entry.path));
          } catch (e) {
            onDecodeError?.(entry.path, e);
            texCache.set(entry.path, null);
          }
        }
        slot = { tex: texCache.get(entry.path) ?? null, file: entry.path };
      }
    }

    memo.set(ref, slot);
    out.push(slot);
  }
  return out;
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
