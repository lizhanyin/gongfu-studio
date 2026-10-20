<template>
  <n-input v-model:value="skin.rootDir" size="small" style="width: 300px" placeholder="模型目录 (AvatarGirl)" @keyup.enter="skin.scan" />
  <n-button size="small" @click="skin.pickDir">浏览</n-button>
  <n-button size="small" type="primary" :loading="skin.scanning" @click="skin.scan">扫描</n-button>
  <n-divider vertical />
  <n-button-group size="small">
    <n-button :type="skin.renderMode === 'shaded' ? 'primary' : 'default'" @click="skin.renderMode = 'shaded'">着色</n-button>
    <n-button :type="skin.renderMode === 'bones' ? 'primary' : 'default'" @click="skin.renderMode = 'bones'">骨骼</n-button>
    <n-button :type="skin.renderMode === 'uv' ? 'primary' : 'default'" @click="skin.renderMode = 'uv'">UV</n-button>
  </n-button-group>
  <n-checkbox size="small" v-model:checked="skin.wireframe">线框</n-checkbox>
  <n-button size="small" @click="skin.frameAll()">定位</n-button>
  <div style="flex: 1"></div>
  <n-button size="small" @click="skin.exportObj" :disabled="!skin.visiblePaths.length">导出 OBJ</n-button>
  <n-button size="small" type="warning" @click="skin.saveSelected" :disabled="!skin.selectedModel">保存当前部件</n-button>
</template>

<script setup lang="ts">
import type { SkinStore } from "../composables/useSkinMode";

defineProps<{ skin: SkinStore }>();
</script>
