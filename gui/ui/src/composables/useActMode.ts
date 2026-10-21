import { computed, onMounted, onUnmounted, reactive, ref, type ShallowRef } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { FskelMeta } from "../types";
import type { Viewer } from "../viewer";
import { message } from "../message";

/**
 * FSKEL 动作：加载/播放（rAF 循环）、BVH / GLB 导出。
 * 动作文件浏览由资源树/网格承担，这里只管"已加载的动作"。
 * 返回 reactive store（嵌套 ref 自动解包），供 App 与各子组件直接使用。
 */
export function useActMode(viewer: ShallowRef<Viewer | null>) {
  const actMeta = ref<FskelMeta | null>(null);
  const actPoses = ref<Float32Array | null>(null);
  const actLoading = ref(false);
  const currentAction = ref(0);
  const playing = ref(false);
  const playTime = ref(0);
  const speed = ref(1);
  const showSkinOverlay = ref(true);
  const showSkeleton = ref(false);
  let playRaf = 0;
  let lastTs = 0;

  const currentActionInfo = computed(() => actMeta.value?.actions[currentAction.value] ?? null);
  /** 当前动作的帧区间 [from, to) */
  const actionRange = computed<[number, number]>(() => {
    const a = currentActionInfo.value;
    if (!a) return [0, actMeta.value?.frameCount ?? 0];
    return [a.firstFrame, a.firstFrame + a.frameCount];
  });
  const actionFrames = computed(() => actionRange.value[1] - actionRange.value[0]);
  /** FSKEL 恒定 30 FPS（PlayBar 显示用） */
  const fps = computed(() => 30);

  async function openActFile(path: string) {
    actLoading.value = true;
    playing.value = false;
    try {
      const meta = await api.loadFskelMeta(path);
      const arr = await api.loadFskelFrames(path, 0, meta.frameCount);
      actMeta.value = meta;
      actPoses.value = Float32Array.from(arr);
      const first = meta.actions[0];
      // 诊断日志前缀先于 setFskel：加载/镜像检测日志即带动作名
      viewer.value?.setLogAction(first?.name ?? meta.fileName);
      viewer.value?.setFskel(meta, actPoses.value);
      currentAction.value = 0;
      playTime.value = 0;
      playing.value = true;
      viewer.value?.frameSkeleton();
      viewer.value?.diagnoseAction(
        first?.firstFrame ?? 0,
        first ? first.firstFrame + first.frameCount : meta.frameCount
      );
    } catch (e) {
      message.error(`动作加载失败: ${e}`);
    } finally {
      actLoading.value = false;
    }
  }

  /** 关闭当前动作：卸下骨架，皮肤恢复绑定姿势 */
  function closeAct() {
    playing.value = false;
    actMeta.value = null;
    actPoses.value = null;
    viewer.value?.setLogAction(null);
    viewer.value?.clearFskel();
  }

  function selectAction(i: number) {
    currentAction.value = i;
    playTime.value = 0;
    // 切动作即切换诊断日志前缀并跑一次该动作区间的诊断
    const a = actMeta.value?.actions[i];
    if (a && viewer.value) {
      viewer.value.setLogAction(a.name);
      viewer.value.diagnoseAction(a.firstFrame, a.firstFrame + a.frameCount);
    }
  }

  function scrub(v: number) {
    playTime.value = v;
    applyPlayTime();
  }

  function applyPlayTime() {
    if (!actPoses.value || !actMeta.value) return;
    const [from, to] = actionRange.value;
    const total = to - from;
    const t = Math.min(Math.max(playTime.value, 0), Math.max(total - 1, 0));
    const abs = from + t;
    const i0 = Math.floor(abs);
    const i1 = Math.min(i0 + 1, actMeta.value.frameCount - 1);
    viewer.value?.applyFskelFrame(i0, i1, abs - i0);
  }

  function playTick(ts: number) {
    playRaf = requestAnimationFrame(playTick);
    const dt = Math.min((ts - lastTs) / 1000, 0.1);
    lastTs = ts;
    if (!playing.value || !actMeta.value) return;
    const total = actionFrames.value;
    playTime.value += dt * 30 * speed.value;
    if (playTime.value >= total) playTime.value = 0;
    applyPlayTime();
  }

  /** 叠加皮肤网格开关（模式切换时也会调用以同步可见性） */
  function setSkinOverlay(v: boolean) {
    showSkinOverlay.value = v;
    viewer.value?.setPartsVisible(v);
  }

  /** 骨架可视化开关（隐藏骨架线条与关节小球，蒙皮网格不受影响） */
  function setSkeletonVisible(v: boolean) {
    showSkeleton.value = v;
    viewer.value?.setSkeletonVisible(v);
  }

  function frameSkeleton() {
    viewer.value?.frameSkeleton();
  }

  // ---------- 导出 ----------
  async function exportBvh(currentOnly: boolean) {
    const m = actMeta.value;
    if (!m) return;
    const a = currentActionInfo.value;
    const base = currentOnly && a ? `${a.name}` : m.fileName;
    const p = await save({
      defaultPath: `${base}.bvh`,
      filters: [{ name: "BVH 骨架动画", extensions: ["bvh"] }],
    });
    if (!p) return;
    try {
      const out = await api.exportFskelBvh(m.path, currentOnly ? currentAction.value : null, p);
      message.success(out);
    } catch (e) {
      message.error(`导出失败: ${e}`);
    }
  }

  async function exportGlb() {
    const m = actMeta.value;
    if (!m) return;
    const p = await save({
      defaultPath: `${m.fileName}.glb`,
      filters: [{ name: "glTF 二进制", extensions: ["glb"] }],
    });
    if (!p) return;
    try {
      const out = await api.exportFskelGlb(m.path, p);
      message.success(out);
    } catch (e) {
      message.error(`导出失败: ${e}`);
    }
  }

  onMounted(() => {
    lastTs = performance.now();
    playRaf = requestAnimationFrame(playTick);
  });

  onUnmounted(() => cancelAnimationFrame(playRaf));

  return reactive({
    actMeta,
    actPoses,
    actLoading,
    currentAction,
    playing,
    playTime,
    speed,
    showSkinOverlay,
    showSkeleton,
    currentActionInfo,
    actionRange,
    actionFrames,
    fps,
    openActFile,
    closeAct,
    selectAction,
    scrub,
    setSkinOverlay,
    setSkeletonVisible,
    frameSkeleton,
    exportBvh,
    exportGlb,
  });
}

export type ActStore = ReturnType<typeof useActMode>;
