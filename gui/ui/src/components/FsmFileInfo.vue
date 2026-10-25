<template>
  <div class="kv" style="padding: 12px" v-if="fsm.fsmModel">
    <b>{{ fileName }}</b><br />
    顶点 {{ fsm.fsmModel.vertexCount }} · 三角形 {{ totalTris }} ·
    子网格 {{ fsm.fsmModel.submeshes.length }}<br />
    尺寸 {{ sizeText }}<br />
    <template v-if="fsm.fsmLoading">加载中…</template>
    <template v-else>
      <n-divider style="margin: 8px 0" />
      <MatTexPanel
        :materials="fsm.fsmModel.materials"
        :mat-files="fsm.texPanel.matFiles"
        :dir-tex-files="fsm.texPanel.dirTexFiles"
        :load-preview="fsm.texPanel.texPreviewUrl"
        @set-texture="(mi, f) => fsm.texPanel.setTexture(mi, f)"
      />
      <n-divider style="margin: 8px 0" />
      <div v-for="d in fsm.fsmModel.dummies" :key="d.name" class="action-row">
        <span>{{ d.name }}</span>
        <span class="action-meta">{{ fmtPos(d.values) }}</span>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { FsmStore } from "../composables/useFsmMode";
import MatTexPanel from "./MatTexPanel.vue";

const props = defineProps<{ fsm: FsmStore }>();

const fileName = computed(() =>
  props.fsm.fsmModel?.path.split(/[\\/]/).pop() ?? ""
);
const totalTris = computed(
  () =>
    props.fsm.fsmModel?.submeshes.reduce((s, x) => s + x.triangles.length / 3, 0) ??
    0
);
const sizeText = computed(() => {
  const m = props.fsm.fsmModel;
  if (!m) return "";
  const d = [0, 1, 2].map((i) => Math.round(m.bboxMax[i] - m.bboxMin[i]));
  return `${d[0]} × ${d[1]} × ${d[2]}`;
});

/** 挂点位置（values 前 3 个 f32） */
function fmtPos(values: number[]) {
  return values
    .slice(0, 3)
    .map((v) => Math.round(v * 100) / 100)
    .join(", ");
}
</script>
