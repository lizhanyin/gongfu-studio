import { computed, onMounted, onUnmounted, reactive, ref, type Ref, type ShallowRef } from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { FskelBrief, FskelMeta } from "../types";
import type { Viewer } from "../viewer";
import { message } from "../message";

/**
 * FSKEL 动作模式：动作文件扫描 / 加载、播放控制（rAF 循环）、BVH / GLB 导出。
 * 返回 reactive store（嵌套 ref 自动解包），供 App 与各子组件直接使用。
 */
export function useActMode(viewer: ShallowRef<Viewer | null>, rootDir: Ref<string>) {
  const actFiles = ref<FskelBrief[]>([]);
  const actScanning = ref(false);
  const actMeta = ref<FskelMeta | null>(null);
  const actPoses = ref<Float32Array | null>(null);
  const actLoading = ref(false);
  const currentAction = ref(0);
  const playing = ref(false);
  const playTime = ref(0);
  const speed = ref(1);
  const showSkinOverlay = ref(true);
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

  async function scanAct() {
    actScanning.value = true;
    try {
      const dir = rootDir.value.replace(/[\\/]+$/, "") + "\\act";
      actFiles.value = await api.scanActDir(dir);
      message.success(`动作目录扫描完成: ${actFiles.value.length} 个文件`);
    } catch (e) {
      message.error(`扫描动作目录失败: ${e}`);
    } finally {
      actScanning.value = false;
    }
  }

  async function pickActFile() {
    const p = await open({
      defaultPath: rootDir.value,
      filters: [{ name: "FSKEL 动作", extensions: ["fskel"] }],
    });
    if (p) {
      if (!actFiles.value.some((f) => f.path === p)) {
        // 单独打开的文件插到列表最前
        actFiles.value.unshift({
          path: p,
          fileName: (p.split(/[\\/]/).pop() ?? p).replace(/\.fskel$/i, ""),
          frameCount: 0,
          boneCount: 0,
          byteLen: 0,
          actionName: "",
        });
      }
      openActFile(p);
    }
  }

  async function openActFile(path: string) {
    actLoading.value = true;
    playing.value = false;
    try {
      const meta = await api.loadFskelMeta(path);
      const arr = await api.loadFskelFrames(path, 0, meta.frameCount);
      actMeta.value = meta;
      actPoses.value = Float32Array.from(arr);
      viewer.value?.setFskel(meta, actPoses.value);
      // 同步左侧列表条目信息
      const brief = actFiles.value.find((f) => f.path === path);
      if (brief && !brief.frameCount) {
        brief.frameCount = meta.frameCount;
        brief.boneCount = meta.boneCount;
      }
      currentAction.value = 0;
      playTime.value = 0;
      playing.value = true;
      viewer.value?.frameSkeleton();
    } catch (e) {
      message.error(`动作加载失败: ${e}`);
    } finally {
      actLoading.value = false;
    }
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
    actFiles,
    actScanning,
    actMeta,
    actPoses,
    actLoading,
    currentAction,
    playing,
    playTime,
    speed,
    showSkinOverlay,
    currentActionInfo,
    actionRange,
    actionFrames,
    scanAct,
    pickActFile,
    openActFile,
    selectAction,
    scrub,
    setSkinOverlay,
    frameSkeleton,
    exportBvh,
    exportGlb,
  });
}

export type ActStore = ReturnType<typeof useActMode>;
