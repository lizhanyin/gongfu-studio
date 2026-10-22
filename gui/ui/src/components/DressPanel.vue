<!-- 角色装扮工作台（检查器「换装变体」面板首屏）：
     基础模型切换 / 整套款式 / 时装 / 装备槽快速浏览。
     下方仍为基础目录部件组（发型/款式变体，PartGroupList）。 -->
<template>
  <div v-if="dress.catalog" class="dress-panel">
    <!-- 基础模型（性别） -->
    <div class="dress-row">
      <span class="dress-label">基础模型</span>
      <n-button-group size="tiny">
        <n-button
          v-for="b in dress.catalog.bases"
          :key="b.key"
          :type="dress.baseKey === b.key ? 'primary' : 'default'"
          @click="dress.switchBase(b.key)"
        >
          {{ b.label }}
        </n-button>
      </n-button-group>
    </div>

    <!-- 整套款式（跨槽同号一键切换 / 翻页浏览） -->
    <div class="dress-row">
      <span class="dress-label">套装</span>
      <n-button
        size="tiny"
        :type="dress.setCurrent === null ? 'primary' : 'default'"
        @click="dress.selectSet(null)"
      >
        默认
      </n-button>
      <n-button size="tiny" quaternary :disabled="!dress.setNumbers.length" @click="dress.stepSet(-1)">
        ‹
      </n-button>
      <span class="dress-set-cur" :title="setCurrentTitle">{{ setCurrentText }}</span>
      <n-button size="tiny" quaternary :disabled="!dress.setNumbers.length" @click="dress.stepSet(1)">
        ›
      </n-button>
    </div>

    <!-- 时装（单文件全身模型，含头） -->
    <div class="part-card dress-slot">
      <div class="row">
        <n-switch
          size="small"
          :value="dress.fashionOn"
          :disabled="!dress.fashionItems.length"
          @update:value="(v: boolean) => dress.toggleFashion(v)"
        />
        <span class="dress-label">时装</span>
        <n-button size="tiny" quaternary :disabled="!dress.fashionItems.length" @click="dress.stepFashion(-1)">
          ‹
        </n-button>
        <n-select
          size="tiny"
          class="dress-select"
          :value="dress.fashionPath ?? undefined"
          :options="fashionOptions"
          :disabled="!dress.fashionItems.length"
          placeholder="无时装资源"
          @update:value="(v: string) => dress.setFashion(v)"
        />
        <n-button size="tiny" quaternary :disabled="!dress.fashionItems.length" @click="dress.stepFashion(1)">
          ›
        </n-button>
      </div>
      <div v-if="dress.currentFashion" class="meta">
        {{ dress.currentFashion.vertexCount }} 顶点 ·
        {{ dress.currentFashion.triangleCount }} 三角形 ·
        纹理: {{ texText(dress.currentFashion) }}
      </div>
    </div>

    <!-- 装备槽 -->
    <div
      v-for="slot in dress.catalog.slots"
      :key="slot.key"
      class="part-card dress-slot"
      :class="{ covered: coveredByFashion(slot.key) }"
    >
      <div class="row">
        <n-switch
          size="small"
          :value="dress.equipOn.get(slot.key) ?? false"
          :disabled="!dress.slotItems(slot.key).length || coveredByFashion(slot.key)"
          @update:value="(v: boolean) => dress.toggleEquip(slot.key, v)"
        />
        <span class="dress-label">{{ slot.label }}</span>
        <n-button
          size="tiny"
          quaternary
          :disabled="!dress.slotItems(slot.key).length || coveredByFashion(slot.key)"
          @click="dress.stepEquip(slot.key, -1)"
        >
          ‹
        </n-button>
        <n-select
          size="tiny"
          class="dress-select"
          :value="dress.equipPath.get(slot.key) ?? undefined"
          :options="slotOptions(slot.key)"
          :disabled="!dress.slotItems(slot.key).length || coveredByFashion(slot.key)"
          :placeholder="slot.items.length ? '选择装备' : '目录缺失'"
          @update:value="(v: string) => dress.setEquip(slot.key, v)"
        />
        <n-button
          size="tiny"
          quaternary
          :disabled="!dress.slotItems(slot.key).length || coveredByFashion(slot.key)"
          @click="dress.stepEquip(slot.key, 1)"
        >
          ›
        </n-button>
      </div>
      <div v-if="dress.currentItem(slot.key)" class="meta">
        {{ dress.currentItem(slot.key)!.vertexCount }} 顶点 ·
        {{ dress.currentItem(slot.key)!.triangleCount }} 三角形 ·
        纹理: {{ texText(dress.currentItem(slot.key)!) }}
      </div>
      <div v-else-if="!slot.items.length" class="meta">(无可用物品)</div>
      <div v-if="coveredByFashion(slot.key)" class="meta covered-note">被时装覆盖</div>

      <!-- 帽子：挂点骨骼与偏移微调 -->
      <div v-if="slot.key === 'hat' && dress.equipOn.get('hat')" class="hat-tuning">
        <div class="row">
          <span class="dress-label">挂骨</span>
          <n-select
            size="tiny"
            class="dress-select"
            :value="dress.hatBone || ''"
            :options="boneOptions"
            @update:value="(v: string) => (dress.hatBone = v)"
          />
        </div>
        <div class="row">
          <span class="dress-label">偏移</span>
          <n-input-number
            v-for="(ax, i) in (['X', 'Y', 'Z'] as const)"
            :key="ax"
            size="tiny"
            :step="1"
            :value="dress.hatOffset[i]"
            @update:value="(v: number | null) => setHatOffset(i, v)"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { DressStore } from "../composables/useDressMode";
import type { EquipItem } from "../types";
import { shortName } from "../utils";

const props = defineProps<{ dress: DressStore; actDirSkeleton: string[] }>();

const itemLabel = (i: EquipItem) =>
  i.number != null ? `#${i.number} ${shortName(i.name)}` : shortName(i.name);

const slotOptions = (slotKey: string) =>
  props.dress.slotItems(slotKey).map((i) => ({ label: itemLabel(i), value: i.path }));

const fashionOptions = computed(() =>
  props.dress.fashionItems.map((i) => ({ label: itemLabel(i), value: i.path }))
);

/** 身体槽被时装覆盖（帽子可与时装共存） */
function coveredByFashion(slotKey: string): boolean {
  return props.dress.fashionOn && slotKey !== "hat";
}

function texText(item: EquipItem): string {
  const files = [...new Set(item.textureFiles.filter((f): f is string => !!f))];
  const missing = item.textureFiles.some((f) => !f);
  const names = files.map(shortName).join(", ");
  return missing ? `${names || "—"} (缺 ${item.textureFiles.length - files.length})` : names || "—";
}

const setCurrentText = computed(() => {
  const n = props.dress.setCurrent;
  return n === null ? "默认" : n === -1 ? "混搭" : `款式 ${n}`;
});
const setCurrentTitle = computed(() => {
  const n = props.dress.setCurrent;
  return n === null
    ? "全部卸下，显示基础模型"
    : n === -1
      ? "各槽款式不一致"
      : `款式 ${n}`;
});

const boneOptions = computed(() => [
  { label: "自动 (含 head)", value: "" },
  ...props.actDirSkeleton.map((n) => ({ label: n, value: n })),
]);

function setHatOffset(i: number, v: number | null) {
  const off = [...props.dress.hatOffset] as [number, number, number];
  off[i] = v ?? 0;
  props.dress.hatOffset = off;
}
</script>
