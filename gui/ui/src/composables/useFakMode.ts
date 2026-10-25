import { computed, onMounted, onUnmounted, reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { FakModel } from "../types";
import { resolveModelTextures } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";
import { useTexPanelState } from "./useTexPanel";

/**
 * FAK 顶点动画：加载/播放（rAF 按每帧自身时长推进）、动作分段切换、贴图解析。
 * 与皮肤/骨架模式互斥（App 层负责切换时清理）。
 */
export function useFakMode(viewer: ShallowRef<Viewer | null>) {
  const fakModel = ref<FakModel | null>(null);
  const fakLoading = ref(false);
  const currentAction = ref(0);
  const playing = ref(false);
  /** 当前动作内的浮点帧号（0..frames-1） */
  const playTime = ref(0);
  const speed = ref(1);
  const texCache = reactive(new Map<string, TexData | null>());
  let playRaf = 0;
  let lastTs = 0;

  /** 检查器贴图面板：绑定清单 / 同目录可选 / 预览 / 热替换（FAK/FSM 共用） */
  const texPanel = useTexPanelState(texCache, (mi, f, t) =>
    viewer.value?.setFakTexture(mi, f, t)
  );

  const currentActionInfo = computed(
    () => fakModel.value?.actions[currentAction.value] ?? null
  );
  const actionRange = computed<[number, number]>(() => {
    const a = currentActionInfo.value;
    if (!a) return [0, fakModel.value?.frames.length ?? 0];
    return [a.firstFrame, a.firstFrame + a.frameCount];
  });
  const actionFrames = computed(() => actionRange.value[1] - actionRange.value[0]);
  /** 当前全局帧号 = 动作起始帧 + 动作内帧号 */
  const absFrame = computed(() => actionRange.value[0] + playTime.value);
  /** 整个文件的总帧数 */
  const totalFrames = computed(() => fakModel.value?.frames.length ?? 0);
  /** 显示用平均帧率（各帧时长不一） */
  const fps = computed(() => {
    const m = fakModel.value;
    if (!m) return 30;
    const [f, t] = actionRange.value;
    let sum = 0;
    for (let i = f; i < t; i++) sum += m.frames[i]?.frameDt ?? 1 / 30;
    return sum > 0 ? (t - f) / sum : 30;
  });

  /** 材质贴图解析共用：路径优先（引用自带 system\npc\… 目录）+ 同目录 stem 回落 */
  function resolveMaterials(model: FakModel) {
    return resolveModelTextures(
      model.path,
      model.materials.map((m) => m.texturePath),
      texCache,
      (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
    );
  }

  async function openFakFile(path: string) {
    fakLoading.value = true;
    playing.value = false;
    try {
      const model = await api.loadFak(path);
      fakModel.value = model;
      currentAction.value = 0;
      playTime.value = 0;
      const mats = await resolveMaterials(model);
      texPanel.matFiles.value = mats.map((s) => s.file);
      texPanel.loadDirOptions(model.path);
      viewer.value?.setFak(model, mats);
      playing.value = true;
      viewer.value?.frameAll();
    } catch (e) {
      message.error(`FAK 加载失败: ${e}`);
    } finally {
      fakLoading.value = false;
    }
  }

  function closeFak() {
    playing.value = false;
    fakModel.value = null;
    texPanel.matFiles.value = [];
    texPanel.dirTexFiles.value = [];
    viewer.value?.clearFak();
  }

  function selectAction(i: number) {
    currentAction.value = i;
    playTime.value = 0;
  }

  function scrub(v: number) {
    playTime.value = v;
    applyPlayTime();
  }

  function applyPlayTime() {
    const m = fakModel.value;
    if (!m) return;
    const [from, to] = actionRange.value;
    const total = to - from;
    const t = Math.min(Math.max(playTime.value, 0), Math.max(total - 1, 0));
    const i0 = from + Math.floor(t);
    const i1 = Math.min(i0 + 1, m.frames.length - 1);
    viewer.value?.applyFakFrame(i0, i1, t - Math.floor(t));
  }

  function playTick(ts: number) {
    playRaf = requestAnimationFrame(playTick);
    const dt = Math.min((ts - lastTs) / 1000, 0.1);
    lastTs = ts;
    const m = fakModel.value;
    if (!playing.value || !m) return;
    const [from, to] = actionRange.value;
    const total = to - from;
    // 按当前整数帧自身的时长推进浮点帧号（各帧 dt 不一）
    const cur = Math.min(Math.floor(playTime.value), total - 1);
    const frameDt = m.frames[from + cur]?.frameDt || 1 / 30;
    playTime.value += (dt * speed.value) / frameDt;
    if (playTime.value >= total) playTime.value = 0;
    applyPlayTime();
  }

  onMounted(() => {
    lastTs = performance.now();
    playRaf = requestAnimationFrame(playTick);
  });
  onUnmounted(() => cancelAnimationFrame(playRaf));

  return reactive({
    fakModel,
    fakLoading,
    currentAction,
    playing,
    playTime,
    speed,
    currentActionInfo,
    actionRange,
    actionFrames,
    absFrame,
    totalFrames,
    fps,
    openFakFile,
    closeFak,
    selectAction,
    scrub,
    texPanel,
  });
}

export type FakStore = ReturnType<typeof useFakMode>;
