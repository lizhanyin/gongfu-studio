import { computed, reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { FscnModel } from "../types";
import { resolveModelTextures } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";
import { useTexPanelState } from "./useTexPanel";

/**
 * FSCN 场景静态网格：几何/材质与 FSM 字段级同构，渲染直接复用
 * viewer.setFsm 路径（FscnModel 结构兼容 FsmModel，挂点恒空）。
 * 贴图与 FAK/FSM 模式同「路径优先」口径：材质引用自带
 * system\building\texture\... 目录，按模型路径祖先（CPK 根）拼接命中，
 * 与模型同目录回落（stem 匹配）在 Building 子目录下通常无命中。
 */
export function useFscnMode(viewer: ShallowRef<Viewer | null>) {
  const fscnModel = ref<FscnModel | null>(null);
  const fscnLoading = ref(false);
  const texCache = reactive(new Map<string, TexData | null>());

  /** 检查器贴图面板：绑定清单 / 同目录可选 / 预览 / 热替换（FAK/FSM/FSCN 共用） */
  const texPanel = useTexPanelState(texCache, (mi, f, t) =>
    viewer.value?.setFsmTexture(mi, f, t)
  );

  /** BVH 统计（检查器）：内部节点 / 叶子数 */
  const treeStats = computed(() => {
    const m = fscnModel.value;
    if (!m) return null;
    let nodes = 0;
    let leaves = 0;
    for (const r of m.tree) r.type === "node" ? nodes++ : leaves++;
    return { nodes, leaves };
  });

  async function openFscnFile(path: string) {
    fscnLoading.value = true;
    try {
      const model = await api.loadFscn(path);
      fscnModel.value = model;
      const mats = await resolveModelTextures(
        model.path,
        model.materials.map((m) => m.texturePath),
        texCache,
        (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
      );
      texPanel.matFiles.value = mats.map((s) => s.file);
      texPanel.loadDirOptions(model.path);
      viewer.value?.setFsm(model, mats);
      viewer.value?.frameAll();
    } catch (e) {
      message.error(`FSCN 加载失败: ${e}`);
    } finally {
      fscnLoading.value = false;
    }
  }

  function closeFscn() {
    fscnModel.value = null;
    texPanel.matFiles.value = [];
    texPanel.dirTexFiles.value = [];
    viewer.value?.clearFsm();
  }

  return reactive({
    fscnModel,
    fscnLoading,
    openFscnFile,
    closeFscn,
    texPanel,
    treeStats,
  });
}

export type FscnStore = ReturnType<typeof useFscnMode>;
