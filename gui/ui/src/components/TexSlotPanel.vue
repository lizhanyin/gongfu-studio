<template>
  <div v-if="skin.modelSet?.texSlots.length || skin.partTexVariants.length" class="tex-slots">
    <div
      v-for="s in skin.modelSet?.texSlots ?? []"
      :key="s.key"
      class="form-row"
      :title="s.overlay ? '透明叠层（贴花）' : '替换部件材质贴图'"
    >
      <label>{{ s.label }}</label>
      <n-select
        size="small"
        style="flex: 1"
        :value="s.overlay ? (skin.texSlotSel.get(s.key) ?? '') : skin.texSlotSel.get(s.key)"
        :placeholder="s.overlay ? '关闭' : '选择贴图'"
        :options="
          (s.overlay ? [{ label: '关闭', value: '' }] : []).concat(
            s.variants.map((v) => ({ label: v.label, value: v.path }))
          )
        "
        @update:value="(v: string) => skin.applyTexVariant(s, v === '' ? null : v)"
      />
    </div>
    <!-- 当前选中部件的配色族（发型颜色 ff01_00/01/02 等） -->
    <div v-if="skin.partTexVariants.length" class="form-row" title="同前缀贴图互为配色">
      <label>配色</label>
      <n-select
        size="small"
        style="flex: 1"
        :value="skin.partTexCurrent"
        :options="skin.partTexVariants.map((v) => ({ label: v.label, value: v.label }))"
        @update:value="(label: string) => applyColor(label)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import type { SkinStore } from "../composables/useSkinMode";

const props = defineProps<{ skin: SkinStore }>();

/** 配色选项按 label（ff01_02 → "02"）映射回贴图路径 */
function applyColor(label: string) {
  const v = props.skin.partTexVariants.find((x) => x.label === label);
  if (v) props.skin.applyPartTexture(v.path);
}
</script>
