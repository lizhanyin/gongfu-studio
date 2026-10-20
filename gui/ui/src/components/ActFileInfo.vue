<template>
  <div class="kv" style="padding: 12px">
    <template v-if="act.actMeta">
      <b>{{ act.actMeta.fileName }}</b><br />
      版本 0x{{ act.actMeta.version.toString(16) }} · 校验 0x{{ act.actMeta.checksum.toString(16) }}<br />
      骨骼 {{ act.actMeta.boneCount }} · 帧 {{ act.actMeta.frameCount }} ·
      {{ (1 / act.actMeta.frameDt).toFixed(0) }} FPS<br />
      动作 {{ act.actMeta.actions.length }} 个 ·
      总时长 {{ (act.actMeta.frameCount / 30).toFixed(2) }}s<br />
      <span v-if="act.actLoading">帧数据加载中…</span>
      <n-divider style="margin: 8px 0" />
      导出说明：BVH 导入 Blender 得到骨架 + 动作；GLB 面向
      three.js / 引擎（含全部动作分段）。
    </template>
    <template v-else>
      动作模式：预览 .FSKEL 骨架动画，支持播放 / 变速 / 逐帧拖动。<br /><br />
      左侧选择动作文件后自动播放。
    </template>
  </div>
</template>

<script setup lang="ts">
import type { ActStore } from "../composables/useActMode";

defineProps<{ act: ActStore }>();
</script>
