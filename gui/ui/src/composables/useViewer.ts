import { onUnmounted, shallowRef } from "vue";
import type { Viewer } from "../viewer";

/**
 * 持有 three.js Viewer 实例的共享 shallowRef，供各模式 composable 调用。
 * 实例的创建在 ViewerHost.vue（全工程唯一运行时引用 viewer.ts 的模块，
 * 即 viewer.ts 的 HMR 边界）；视口重建（挂载 / viewer.ts 热更）时经
 * adopt() 换新实例并销毁旧的。
 */
export function useViewer() {
  const viewer = shallowRef<Viewer | null>(null);

  function adopt(instance: Viewer | null) {
    if (viewer.value === instance) return;
    const old = viewer.value;
    viewer.value = instance;
    old?.dispose(); // dispose 会移除旧实例自己的 canvas，避免叠加
  }

  onUnmounted(() => adopt(null));

  return { viewer, adopt };
}
