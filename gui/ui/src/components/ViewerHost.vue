<!-- 无渲染宿主：全工程唯一运行时 import viewer.ts 的模块（viewer.ts 的 HMR 边界）。
     viewer.ts 变更 → 本组件自接受重载 → watch 重新触发 → 在同一个 DOM 节点上重建
     Viewer 实例并 emit ready；窗口其余部分（面板 / 资源树 / 输入状态）不受影响。
     上层在 ready 里销毁旧实例并重放当前内容（见 App.vue 的 onViewerReady）。 -->
<script setup lang="ts">
import { watch } from "vue";
import { Viewer } from "../viewer";

const props = defineProps<{ target: HTMLElement | null }>();
const emit = defineEmits<{ ready: [viewer: Viewer] }>();

watch(
  () => props.target,
  (el) => {
    if (el) emit("ready", new Viewer(el));
  },
  { immediate: true },
);

// viewer.ts 的依赖级热更新：只跑本回调、不重载组件 —— 把存活实例的原型换到
// 新类上，方法体/模块常量的修改即刻生效，视口、已加载模型、检查器零重建。
// 改构造器/类字段初始化不会作用于已存在实例，需要时按 F5 整页重建。
if (import.meta.hot) {
  import.meta.hot.accept("../viewer", (mod) => {
    const live = (window as unknown as { __viewer?: Viewer }).__viewer;
    const NewViewer = (mod as unknown as typeof import("../viewer") | undefined)?.Viewer;
    if (NewViewer && live) Object.setPrototypeOf(live, NewViewer.prototype);
  });
}
</script>
