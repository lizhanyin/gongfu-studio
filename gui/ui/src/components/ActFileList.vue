<template>
  <div class="section-title">动作文件（{{ act.actFiles.length }}）</div>
  <div v-if="!act.actFiles.length && !act.actScanning" class="kv" style="padding: 8px">
    点击「扫描动作目录」读取 {{ rootDir }}\\act 下的 .FSKEL，或「打开动作文件」选择单个文件。
  </div>
  <div
    v-for="f in act.actFiles"
    :key="f.path"
    class="part-card"
    :class="{ selected: act.actMeta?.path === f.path }"
    @click="act.openActFile(f.path)"
  >
    <div class="row">
      <span class="name">{{ f.fileName }}</span>
      <span class="meta-inline">{{ f.frameCount }} 帧</span>
    </div>
    <div class="meta">{{ f.boneCount }} 骨骼 · {{ (f.byteLen / 1024).toFixed(0) }} KB</div>
  </div>

  <template v-if="act.actMeta">
    <div class="section-title" style="margin-top: 10px">动作分段（{{ act.actMeta.actions.length }}）</div>
    <div
      v-for="(a, i) in act.actMeta.actions"
      :key="i"
      class="part-card"
      :class="{ selected: act.currentAction === i }"
      @click="act.selectAction(i)"
    >
      <div class="row">
        <span class="name">{{ a.name }}</span>
        <span class="meta-inline">{{ a.frameCount }} 帧 · {{ (a.frameCount / 30).toFixed(2) }}s</span>
      </div>
    </div>
  </template>
</template>

<script setup lang="ts">
import type { ActStore } from "../composables/useActMode";

defineProps<{ act: ActStore; rootDir: string }>();
</script>
