<template>
  <div class="kv" style="padding: 12px" v-if="sfx.sfxModel">
    <b>{{ fileName }}</b><br />
    节点 {{ sfx.sfxModel.nodes.length }} · 类型 {{ kindCount }} ·
    贴图 {{ totalTex }}<br />
    根平移 {{ transText }}<br />
    <template v-if="sfx.sfxLoading">加载中…</template>
    <template v-else>
      <n-divider style="margin: 8px 0" />
      <div class="sfx-node" v-for="(n, i) in sfx.sfxModel.nodes" :key="i">
        <div class="action-row">
          <span class="sfx-kind">#{{ i }} {{ kindLabel(n.kind) }}</span>
          <span class="action-meta">{{ n.name || "-" }}</span>
        </div>
        <div v-if="n.textures.length" class="sfx-refs">
          <img
            v-for="t in n.textures"
            :key="t"
            class="sfx-thumb"
            :src="sfx.texSlots.get(t)?.url ?? undefined"
            :title="t"
            @click="previewTex = previewTex === t ? null : t"
          />
        </div>
        <div v-if="n.models.length" class="sfx-refs">
          <span v-for="m in n.models" :key="m" class="sfx-ref">网格 {{ shortName(m) }}</span>
        </div>
        <div v-if="n.sounds.length" class="sfx-refs">
          <span v-for="s in n.sounds" :key="s" class="sfx-ref">音效 {{ shortName(s) }}</span>
        </div>
      </div>
      <div v-if="previewTex" class="sfx-preview">
        <div class="sfx-preview-bar">
          <span>{{ shortName(sfx.texSlots.get(previewTex)?.file ?? previewTex) }}</span>
          <n-button size="tiny" @click="previewTex = null">关闭</n-button>
        </div>
        <img class="sfx-preview-img" :src="sfx.texSlots.get(previewTex)?.url ?? undefined" />
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import type { SfxStore } from "../composables/useSfxMode";
import { shortName } from "../utils";

const props = defineProps<{ sfx: SfxStore }>();

/** 点击缩略图放大的引用（再点一次/关闭按钮收起） */
const previewTex = ref<string | null>(null);

const fileName = computed(() =>
  props.sfx.sfxModel?.path.split(/[\\/]/).pop() ?? ""
);
const totalTex = computed(() => props.sfx.uniqueTextures.length);
const kindCount = computed(
  () => new Set(props.sfx.sfxModel?.nodes.map((n) => n.kind)).size
);
const transText = computed(() => {
  const m = props.sfx.sfxModel;
  if (!m) return "";
  return [12, 13, 14]
    .map((i) => Math.round(m.matrix[i]))
    .join(", ");
});

/** 11 种节点类型的中文标注（未知类型原样展示） */
const KIND_LABELS: Record<string, string> = {
  EffectQuad: "公告板",
  EffectParticleEmitter: "粒子",
  EffectCross: "交叉面",
  EffectCylinder: "柱面",
  EffectBeam: "光束",
  EffectGrid: "面片",
  EffectMesh: "网格",
  EffectSphere: "球面",
  EffectMsg: "飘字",
  EffectExplosive: "爆炸",
  EffectSound: "音效",
};
function kindLabel(kind: string): string {
  return `${KIND_LABELS[kind] ?? kind}`;
}
</script>

<style scoped>
.sfx-node {
  padding: 2px 0;
}
.sfx-node .action-row {
  margin-bottom: 2px;
}
.sfx-kind {
  font-weight: 600;
}
.sfx-refs {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding-left: 8px;
}
.sfx-thumb {
  width: 44px;
  height: 44px;
  object-fit: contain;
  image-rendering: pixelated;
  background: #000;
  border: 1px solid #333;
  border-radius: 3px;
  cursor: pointer;
}
.sfx-thumb:hover {
  border-color: #63e2b7;
}
.sfx-ref {
  font-size: 12px;
  color: #aaa;
  background: #22242a;
  border-radius: 3px;
  padding: 1px 6px;
}
.sfx-preview {
  margin-top: 8px;
}
.sfx-preview-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12px;
  color: #ccc;
  margin-bottom: 4px;
  word-break: break-all;
}
.sfx-preview-img {
  width: 100%;
  max-height: 220px;
  object-fit: contain;
  background: #000;
  border: 1px solid #333;
  border-radius: 3px;
}
</style>
