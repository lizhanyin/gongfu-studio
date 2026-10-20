<template>
  <div class="play-bar">
    <n-button size="small" circle @click="act.playing = !act.playing">{{ act.playing ? "⏸" : "▶" }}</n-button>
    <n-slider
      style="flex: 1; min-width: 120px"
      :min="0"
      :max="Math.max(act.actionFrames - 1, 1)"
      :step="0.01"
      :value="act.playTime"
      :format-tooltip="(v: number) => `${Math.floor(v)} 帧 · ${(v / 30).toFixed(2)}s`"
      @update:value="act.scrub"
    />
    <span class="play-info">
      {{ Math.floor(act.playTime) }}/{{ act.actionFrames - 1 }} 帧 ·
      {{ (act.playTime / 30).toFixed(2) }}s
    </span>
    <n-select
      size="tiny"
      style="width: 74px"
      :value="act.speed"
      :options="[0.25, 0.5, 1, 2].map((s) => ({ label: s + 'x', value: s }))"
      @update:value="(v: number) => (act.speed = v)"
    />
  </div>
</template>

<script setup lang="ts">
import type { ActStore } from "../composables/useActMode";

defineProps<{ act: ActStore }>();
</script>
