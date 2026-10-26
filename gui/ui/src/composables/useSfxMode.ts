import { computed, reactive, ref } from "vue";
import { api } from "../api";
import type { SfxFile } from "../types";
import { resolveModelTextures, texCanvasUrl } from "../tex";
import type { TexData } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/**
 * SFX 特效脚本：无 3D 渲染（特效为粒子/公告板运行时行为，离线不可复现），
 * 检查器展示节点树与资源引用。贴图引用（system\effect\tex\…\*.tga）与
 * FAK/FSM/FSCN 同「路径优先」口径：按文件路径祖先（CPK 根）拼接命中，
 * 未命中换 .dds 重试。
 */
export function useSfxMode() {
  const sfxModel = ref<SfxFile | null>(null);
  const sfxLoading = ref(false);
  const texCache = reactive(new Map<string, TexData | null>());

  /** 贴图引用 → 解析结果（file 为命中的实际路径；url 为缩略图 dataURL） */
  const texSlots = reactive(new Map<string, { file: string | null; url: string | null }>());

  /** 全部节点贴图引用去重保序（动画帧序列集中查看） */
  const uniqueTextures = computed(() => {
    const m = sfxModel.value;
    if (!m) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const n of m.nodes)
      for (const t of n.textures)
        if (!seen.has(t)) {
          seen.add(t);
          out.push(t);
        }
    return out;
  });

  async function openSfxFile(path: string) {
    sfxLoading.value = true;
    try {
      const model = await api.loadSfx(path);
      sfxModel.value = model;
      const refs: string[] = [];
      const seen = new Set<string>();
      for (const n of model.nodes)
        for (const t of n.textures)
          if (!seen.has(t)) {
            seen.add(t);
            refs.push(t);
          }
      const slots = await resolveModelTextures(
        model.path,
        refs,
        texCache,
        (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
      );
      texSlots.clear();
      refs.forEach((ref, i) => {
        const s = slots[i];
        texSlots.set(ref, {
          file: s.file,
          url: s.tex ? texCanvasUrl(s.tex) : null,
        });
      });
    } catch (e) {
      message.error(`SFX 加载失败: ${e}`);
    } finally {
      sfxLoading.value = false;
    }
  }

  function closeSfx() {
    sfxModel.value = null;
    texSlots.clear();
  }

  return reactive({
    sfxModel,
    sfxLoading,
    openSfxFile,
    closeSfx,
    texSlots,
    uniqueTextures,
  });
}

export type SfxStore = ReturnType<typeof useSfxMode>;
