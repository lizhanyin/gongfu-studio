import type { TexData } from "./viewer";

// 从 viewer.ts 剥出的纹理解码 helper：App 与 3 个模式 composable 需要运行时引用它。
// 若留在 viewer.ts，这些引用会把 viewer.ts 的 HMR 边界顶到 App.vue（整窗口重挂），
// 破坏 components/ViewerHost.vue 的“改 viewer.ts 只重建视口”。
// viewer.ts 里的同名函数与 b64ToBytes 已无引用，可删。
function b64ToBytes(b64: string): Uint8Array {
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
