import { ref } from "vue";
import { api } from "../api";
import { loadTexData, texCanvasUrl, TEX_KINDS } from "../tex";
import type { TexData } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/**
 * 检查器贴图面板（FAK / FSM / FSCN 模式共用）：各材质当前绑定贴图、可选贴图
 * 清单（下拉换图）、预览缩略图。换图走热替换回调（viewer.setFakTexture /
 * setFsmTexture），不落盘——CPK 内模型本就不可写，目录内模型保存仍走原导出。
 */
export function useTexPanelState(
  texCache: Map<string, TexData | null>,
  applyTex: (matIdx: number, file: string, tex: TexData) => void
) {
  /** 各材质当前绑定的贴图文件（路径优先解析结果；null=未找到） */
  const matFiles = ref<(string | null)[]>([]);
  /** 模型同目录可替换贴图（换图下拉选项；含当前绑定即使其不在同目录） */
  const dirTexFiles = ref<{ name: string; path: string }[]>([]);
  const texUrlCache = new Map<string, string>();

  /** 贴图预览缩略图 dataURL（解码经 texCache 复用，URL 结果缓存） */
  async function texPreviewUrl(file: string | null): Promise<string | null> {
    if (!file) return null;
    const hit = texUrlCache.get(file);
    if (hit) return hit;
    let d = texCache.get(file) ?? null;
    if (!d) {
      try {
        d = await loadTexData(file);
        texCache.set(file, d);
      } catch {
        return null;
      }
    }
    const url = texCanvasUrl(d);
    texUrlCache.set(file, url);
    return url;
  }

  /** 模型打开后调用：刷新同目录贴图清单，并把跨目录解析到的当前绑定并入选项 */
  async function loadDirOptions(modelPath: string) {
    const dir = modelPath.replace(/[\\/][^\\/]*$/, "");
    let entries: { kind: string; name: string; path: string }[] = [];
    try {
      entries = await api.listResDir(dir);
    } catch {
      entries = [];
    }
    let opts = entries
      .filter((e) => TEX_KINDS.test(e.kind))
      .map((e) => ({ name: e.name, path: e.path }))
      .sort((a, b) => a.name.localeCompare(b.name));
    // 模型同目录无贴图时（FSCN 建筑网格贴图在 building\texture 等引用目录），
    // 用首个已解析绑定所在目录补全换图选项
    if (!opts.length) {
      const first = matFiles.value.find((f): f is string => !!f);
      if (first) {
        try {
          const tdir = first.replace(/[\\/][^\\/]*$/, "");
          const tentries = await api.listResDir(tdir);
          opts = tentries
            .filter((e) => TEX_KINDS.test(e.kind))
            .map((e) => ({ name: e.name, path: e.path }))
            .sort((a, b) => a.name.localeCompare(b.name));
        } catch {
          // 引用目录列举失败：保持空清单，仅显示当前绑定
        }
      }
    }
    for (const f of matFiles.value) {
      if (f && !opts.some((o) => o.path === f)) {
        opts.unshift({ name: shortName(f), path: f });
      }
    }
    dirTexFiles.value = opts;
  }

  /** 换图：解码新贴图 → 热替换到引用该材质的全部子网格 → 更新绑定状态 */
  async function setTexture(matIdx: number, file: string) {
    try {
      const tex = await loadTexData(file);
      texCache.set(file, tex);
      applyTex(matIdx, file, tex);
      matFiles.value = matFiles.value.map((f, i) => (i === matIdx ? file : f));
    } catch (e) {
      message.error(`纹理解码失败 ${shortName(file)}: ${e}`);
    }
  }

  return { matFiles, dirTexFiles, texPreviewUrl, loadDirOptions, setTexture };
}

export type TexPanelState = ReturnType<typeof useTexPanelState>;
