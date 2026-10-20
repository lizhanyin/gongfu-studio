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
      {{ Math.floor(player.playTime) }}/{{ player.actionFrames - 1 }} 帧 ·
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
/** 通用播放条：FSKEL 动作与 FAK 顶点动画共用（store 均满足此结构） */
export interface PlayerLike {
  playing: boolean;
  /** 当前动作内的浮点帧号 */
  playTime: number;
  /** 当前动作总帧数 */
  actionFrames: number;
  /** 显示用帧率（FSKEL=30，FAK 为各帧时长倒数均值） */
  fps: number;
  speed: number;
  scrub(v: number): void;
}

defineProps<{ player: PlayerLike }>();
</script>
