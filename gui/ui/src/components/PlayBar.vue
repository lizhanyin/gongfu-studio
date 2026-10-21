<template>
  <div class="play-bar">
    <n-button size="small" circle @click="player.playing = !player.playing">{{ player.playing ? "⏸" : "▶" }}</n-button>
    <n-slider
      style="flex: 1; min-width: 120px"
      :min="0"
      :max="Math.max(player.actionFrames - 1, 1)"
      :step="0.01"
      :value="player.playTime"
      :format-tooltip="(v: number) => `${Math.floor(v)} 帧 · ${(v / player.fps).toFixed(2)}s`"
      @update:value="player.scrub"
    />
    <span class="play-info">
      {{ Math.floor(player.playTime) }}/{{ player.actionFrames - 1 }} 帧{{ absText }} ·
      {{ (player.playTime / player.fps).toFixed(2) }}s
    </span>
    <n-select
      size="tiny"
      style="width: 74px"
      :value="player.speed"
      :options="[0.25, 0.5, 1, 2].map((s) => ({ label: s + 'x', value: s }))"
      @update:value="(v: number) => (player.speed = v)"
    />
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";

/** 通用播放条：FSKEL 动作与 FAK 顶点动画共用（store 均满足此结构） */
export interface PlayerLike {
  playing: boolean;
  /** 当前动作内的浮点帧号 */
  playTime: number;
  /** 当前动作总帧数 */
  actionFrames: number;
  /** 当前帧在文件内的全局帧号（动作起始帧 + playTime），可选 */
  absFrame?: number;
  /** 文件总帧数（与 absFrame 配套显示），可选 */
  totalFrames?: number;
  /** 显示用帧率（FSKEL=30，FAK 为各帧时长倒数均值） */
  fps: number;
  speed: number;
  scrub(v: number): void;
}

const props = defineProps<{ player: PlayerLike }>();

/** 全局帧号后缀；单动作即全文件（全局=动作内）时不重复显示 */
const absText = computed(() => {
  const p = props.player;
  const abs = p.absFrame;
  const total = p.totalFrames;
  if (typeof abs !== "number" || !total) return "";
  if (total === p.actionFrames && Math.floor(abs) === Math.floor(p.playTime)) return "";
  return ` · 全局 ${Math.floor(abs)}/${total - 1}`;
});
</script>
