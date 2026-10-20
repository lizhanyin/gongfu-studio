<template>
  <div class="kv" style="padding: 12px" v-if="fak.fakModel">
    <b>{{ fileName }}</b><br />
    顶点 {{ fak.fakModel.vertexCount }} · 三角形 {{ totalTris }} ·
    子网格 {{ fak.fakModel.submeshes.length }}<br />
    帧 {{ fak.fakModel.frames.length }} · 动作 {{ fak.fakModel.actions.length }} ·
    材质 {{ fak.fakModel.materials.length }}<br />
    <span v-if="fak.fakLoading">帧数据加载中…</span>
    <template v-else>
      <n-divider style="margin: 8px 0" />
      <div
        v-for="(a, i) in fak.fakModel.actions"
        :key="a.name + i"
        class="action-row"
        :class="{ active: i === fak.currentAction }"
        @click="fak.selectAction(i)"
      >
        <span>{{ a.name }}</span>
        <span class="action-meta">{{ a.frameCount }} 帧 · {{ durationOf(i) }}</span>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { FakStore } from "../composables/useFakMode";

const props = defineProps<{ fak: FakStore }>();

const fileName = computed(() =>
  props.fak.fakModel?.path.split(/[\\/]/).pop() ?? ""
);
const totalTris = computed(
  () =>
    props.fak.fakModel?.submeshes.reduce((s, x) => s + x.triangles.length / 3, 0) ??
    0
);
/** 动作总时长（各帧 dt 求和） */
function durationOf(i: number) {
  const m = props.fak.fakModel;
  if (!m) return "";
  const a = m.actions[i];
  let sum = 0;
  for (let k = a.firstFrame; k < a.firstFrame + a.frameCount; k++) {
    sum += m.frames[k]?.frameDt ?? 0;
  }
  return `${sum.toFixed(2)}s`;
}
</script>
