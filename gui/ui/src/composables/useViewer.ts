import { onMounted, onUnmounted, shallowRef, type Ref } from "vue";
import { Viewer } from "../viewer";

/**
 * 管理 three.js Viewer 实例的生命周期：挂载时创建，卸载时销毁。
 * 返回的 shallowRef 供各模式 composable 共享调用。
 */
export function useViewer(viewportRef: Ref<HTMLElement | null>) {
  const viewer = shallowRef<Viewer | null>(null);

  onMounted(() => {
    if (viewportRef.value) viewer.value = new Viewer(viewportRef.value);
  });

  onUnmounted(() => {
    viewer.value?.dispose();
    viewer.value = null;
  });

  return viewer;
}
