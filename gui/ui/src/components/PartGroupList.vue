<template>
  <div class="section-title">部件组（同几何 = 换装变体）</div>
  <div v-if="!skin.modelSet" class="kv" style="padding: 8px">尚未扫描目录</div>
  <div
    v-for="g in skin.modelSet?.groups ?? []"
    :key="g.signature"
    class="part-card"
    :class="{ selected: skin.selectedPart === skin.groupSel.get(g.signature) }"
    @click="skin.selectPart(skin.groupSel.get(g.signature) ?? null)"
  >
    <div class="row">
      <n-switch
        size="small"
        :value="skin.groupVis.get(g.signature) ?? false"
        @update:value="(v: boolean) => skin.toggleGroup(g, v)"
      />
      <span class="name" :title="g.label">{{ g.label }}</span>
      <span v-if="skin.isDirty(skin.groupSel.get(g.signature))" class="dirty-dot">●</span>
      <n-select
        v-if="g.parts.length > 1"
        size="tiny"
        style="width: 140px"
        :value="skin.groupSel.get(g.signature)"
        :options="g.parts.map((p) => ({ label: p.fileName, value: p.path }))"
        @update:value="(v: string) => skin.changeVariant(g, v)"
      />
    </div>
    <div class="meta" v-if="skin.partInfoOf(g)">
      {{ skin.partInfoOf(g)!.vertexCount }} 顶点 · {{ skin.partInfoOf(g)!.triangleCount }} 三角形 ·
      骨骼 {{ bonesText(skin.partInfoOf(g)!) }}
      <br />
      纹理: {{ texShort(skin.partInfoOf(g)!) }}
    </div>
  </div>
</template>

<script setup lang="ts">
import type { SkinStore } from "../composables/useSkinMode";
import type { PartInfo } from "../types";
import { shortName } from "../utils";

defineProps<{ skin: SkinStore }>();

function bonesText(p: PartInfo) {
  if (!p.bones.length) return "-";
  const hex = p.bones.map((b) => "0x" + b.toString(16));
  return hex.length > 4 ? `${hex[0]}..${hex[hex.length - 1]} (${hex.length})` : hex.join(",");
}

function texShort(p: PartInfo) {
  return p.textureFile ? shortName(p.textureFile) : "(未找到)";
}
</script>
