<template>
  <n-button size="small" type="primary" :loading="act.actScanning" @click="act.scanAct">扫描动作目录</n-button>
  <n-button size="small" @click="act.pickActFile">打开动作文件</n-button>
  <n-checkbox size="small" :checked="act.showSkinOverlay" @update:checked="act.setSkinOverlay">叠加皮肤网格</n-checkbox>
  <n-button size="small" @click="act.frameSkeleton()">定位</n-button>
  <div style="flex: 1"></div>
  <n-button size="small" @click="act.exportBvh(false)" :disabled="!act.actMeta">导出 BVH (全部帧)</n-button>
  <n-button size="small" @click="act.exportBvh(true)" :disabled="!act.actMeta">导出 BVH (当前动作)</n-button>
  <n-button size="small" type="primary" @click="act.exportGlb" :disabled="!act.actMeta">导出 GLB</n-button>
</template>

<script setup lang="ts">
import type { ActStore } from "../composables/useActMode";

defineProps<{ act: ActStore }>();
</script>
