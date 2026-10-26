<template>
  <div class="kv" style="padding: 12px" v-if="fscn.fscnModel">
    <b>{{ fileName }}</b><br />
    顶点 {{ fscn.fscnModel.vertexCount }} · 三角形 {{ totalTris }} ·
    子网格 {{ fscn.fscnModel.submeshes.length }}<br />
    尺寸 {{ sizeText }}<br />
    <template v-if="fscn.fscnLoading">加载中…</template>
    <template v-else>
      <n-divider style="margin: 8px 0" />
      <MatTexPanel
        :materials="fscn.fscnModel.materials"
        :mat-files="fscn.texPanel.matFiles"
        :dir-tex-files="fscn.texPanel.dirTexFiles"
        :load-preview="fscn.texPanel.texPreviewUrl"
        @set-texture="(mi, f) => fscn.texPanel.setTexture(mi, f)"
      />
      <n-divider style="margin: 8px 0" />
      <div v-if="fscn.treeStats" class="action-row">
        <span>BVH</span>
        <span class="action-meta"
          >{{ fscn.treeStats.nodes }} 节点 + {{ fscn.treeStats.leaves }} 叶子
          {{ leafTris }} 三角</span
        >
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { FscnStore } from "../composables/useFscnMode";
import MatTexPanel from "./MatTexPanel.vue";

const props = defineProps<{ fscn: FscnStore }>();

const fileName = computed(() =>
  props.fscn.fscnModel?.path.split(/[\\/]/).pop() ?? ""
);
const totalTris = computed(
  () =>
    props.fscn.fscnModel?.submeshes.reduce(
      (s, x) => s + x.triangles.length / 3,
      0
    ) ?? 0
);
/** BVH 叶子三角形总数（解析层已校验 == 子网格三角形总数） */
const leafTris = computed(
  () =>
    props.fscn.fscnModel?.tree.reduce(
      (s, r) => s + (r.type === "leaf" ? r.triangles.length / 3 : 0),
      0
    ) ?? 0
);
const sizeText = computed(() => {
  const m = props.fscn.fscnModel;
  if (!m) return "";
  const d = [0, 1, 2].map((i) => Math.round(m.bboxMax[i] - m.bboxMin[i]));
  return `${d[0]} × ${d[1]} × ${d[2]}`;
});
</script>
