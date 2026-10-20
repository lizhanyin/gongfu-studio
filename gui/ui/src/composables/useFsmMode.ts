import { reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { FsmModel, ResEntry } from "../types";
import { texDataFromDto, type TexData, type Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/** 同目录内可作贴图的扩展名 */
const TEX_KINDS = /^(dds|tga|png|jpg|bmp)$/;

/**
 * FSM 静态武器网格：加载 + 按材质路径 stem 在同目录解析贴图
 * （J01-S01.tga → j01-s01.dds）。与皮肤/骨架/FAK 模式互斥（App 层负责切换时清理）。
 */
export function useFsmMode(viewer: ShallowRef<Viewer | null>) {
  const fsmModel = ref<FsmModel | null>(null);
  const fsmLoading = ref(false);
  const texCache = reactive(new Map<string, TexData | null>());

  async function resolveMaterials(
    model: FsmModel
  ): Promise<{ tex: TexData | null; file: string | null }[]> {
    const dir = model.path.replace(/[\\/][^\\/]*$/, "");
    let entries: ResEntry[] = [];
    try {
      entries = await api.listResDir(dir);
    } catch {
      // 目录列举失败仅意味着无贴图
    }
    const out: { tex: TexData | null; file: string | null }[] = [];
    for (const mat of model.materials) {
      const stem = (mat.texturePath.split(/[\\/]/).pop() ?? "")
        .replace(/\.[^.]*$/, "")
        .toLowerCase();
      const entry = entries.find(
        (e) =>
          TEX_KINDS.test(e.kind) &&
          e.name.replace(/\.[^.]*$/, "").toLowerCase() === stem
      );
      let tex: TexData | null = null;
      const file = entry?.path ?? null;
      if (file && !texCache.has(file)) {
        try {
          texCache.set(file, texDataFromDto(await api.loadTexture(file)));
        } catch (e) {
          message.error(`纹理解码失败 ${shortName(file)}: ${e}`);
          texCache.set(file, null);
        }
      }
      if (file) tex = texCache.get(file) ?? null;
      out.push({ tex, file });
    }
    return out;
  }

  async function openFsmFile(path: string) {
    fsmLoading.value = true;
    try {
      const model = await api.loadFsm(path);
      fsmModel.value = model;
      const mats = await resolveMaterials(model);
      viewer.value?.setFsm(model, mats);
      viewer.value?.frameAll();
    } catch (e) {
      message.error(`FSM 加载失败: ${e}`);
    } finally {
      fsmLoading.value = false;
    }
  }

  function closeFsm() {
    fsmModel.value = null;
    viewer.value?.clearFsm();
  }

  return reactive({
    fsmModel,
    fsmLoading,
    openFsmFile,
    closeFsm,
  });
}

export type FsmStore = ReturnType<typeof useFsmMode>;
