import { reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { FsmModel } from "../types";
import { resolveModelTextures } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";
import { useTexPanelState } from "./useTexPanel";

/**
 * FSM 静态武器网格：加载 + 材质贴图解析（路径优先 + 同目录 stem 回落，
 * 与 FAK/装扮模式同口径）。与皮肤/骨架/FAK 模式互斥（App 层负责切换时清理）。
 */
export function useFsmMode(viewer: ShallowRef<Viewer | null>) {
  const fsmModel = ref<FsmModel | null>(null);
  const fsmLoading = ref(false);
  const texCache = reactive(new Map<string, TexData | null>());

  /** 检查器贴图面板：绑定清单 / 同目录可选 / 预览 / 热替换（FAK/FSM 共用） */
  const texPanel = useTexPanelState(texCache, (mi, f, t) =>
    viewer.value?.setFsmTexture(mi, f, t)
  );

  function resolveMaterials(model: FsmModel) {
    return resolveModelTextures(
      model.path,
      model.materials.map((m) => m.texturePath),
      texCache,
      (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
    );
  }

  async function openFsmFile(path: string) {
    fsmLoading.value = true;
    try {
      const model = await api.loadFsm(path);
      fsmModel.value = model;
      const mats = await resolveMaterials(model);
      texPanel.matFiles.value = mats.map((s) => s.file);
      texPanel.loadDirOptions(model.path);
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
    texPanel.matFiles.value = [];
    texPanel.dirTexFiles.value = [];
    viewer.value?.clearFsm();
  }

  return reactive({
    fsmModel,
    fsmLoading,
    openFsmFile,
    closeFsm,
    texPanel,
  });
}

export type FsmStore = ReturnType<typeof useFsmMode>;
