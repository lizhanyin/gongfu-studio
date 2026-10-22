/** 取路径最后一段作为显示名 */
export const shortName = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** 写入系统剪贴板；Clipboard API 失败（webview 失焦/权限受限）时降级 execCommand */
export async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // 继续走降级路径
    }
  }
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  const ok = document.execCommand("copy");
  document.body.removeChild(ta);
  return ok;
}
