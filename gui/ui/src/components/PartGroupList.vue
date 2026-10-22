<template>
  <div class="section-title">部件组（同几何 = 换装变体）</div>
  <div v-if="skin.modelSet && skin.outfitStyles.length" class="outfit-row">
    <span class="outfit-label">整套换装</span>
    <n-button
      size="tiny"
      :type="skin.outfitCurrent === 0 ? 'primary' : 'default'"
      @click="skin.selectOutfit(null)"
    >
      默认
    </n-button>
    <n-button
      v-for="n in skin.outfitStyles"
      :key="n"
      size="tiny"
      :type="skin.outfitCurrent === n ? 'primary' : 'default'"
      @click="skin.selectOutfit(n)"
    >
      款式 {{ n }}
    </n-button>
  </div>
  <div v-if="!skin.modelSet" class="kv" style="padding: 8px">尚未扫描目录</div>
  <template v-for="row in rows" :key="row.key">
    <!-- 替换槽（头发等）：跨几何组合并为一张卡，选项互斥 -->
    <div
      v-if="row.slot"
      class="part-card"
      :class="{ selected: skin.selectedPart === skin.slotCurrent(row.slot), replaced: !!replacedBy(row.slot) }"
      @click="skin.selectPart(skin.slotCurrent(row.slot))"
    >
      <div class="row">
        <n-switch
          size="small"
          :value="skin.slotVisible(row.slot)"
          @update:value="(v: boolean) => skin.toggleSlot(row.slot, v)"
        />
        <span class="name">{{ slotLabel(row.slot) }}</span>
        <span v-if="replacedBy(row.slot)" class="replaced-note">由{{ replacedBy(row.slot) }}替换</span>
        <span v-else-if="skin.isDirty(skin.slotCurrent(row.slot))" class="dirty-dot">●</span>
        <n-select
          size="tiny"
          style="width: 140px"
          :value="skin.slotCurrent(row.slot) ?? undefined"
          :options="slotSelectOptions(row.slot)"
          @update:value="(v: string) => skin.selectSlotVariant(row.slot, v)"
        />
      </div>
      <div class="meta" v-if="slotInfo(row.slot)">
        {{ slotInfo(row.slot)!.vertexCount }} 顶点 · {{ slotInfo(row.slot)!.triangleCount }} 三角形 ·
        骨骼 {{ bonesText(slotInfo(row.slot)!) }}
        <br />
        纹理: {{ texShort(slotInfo(row.slot)!) }}
      </div>
    </div>

    <!-- 独立部件组（原有行为） -->
    <div
      v-else
      class="part-card"
      :class="{
        selected: skin.selectedPart === skin.groupSel.get(row.group!.signature),
      }"
      @click="skin.selectPart(skin.groupSel.get(row.group!.signature) ?? null)"
    >
      <div class="row">
        <n-switch
          size="small"
          :value="skin.groupVis.get(row.group!.signature) ?? false"
          @update:value="(v: boolean) => skin.toggleGroup(row.group!, v)"
        />
        <span class="name" :title="row.group!.label">{{ groupLabel(row.group!) }}</span>
        <span v-if="skin.isDirty(skin.groupSel.get(row.group!.signature))" class="dirty-dot">●</span>
        <n-select
          v-if="row.group!.parts.length > 1"
          size="tiny"
          style="width: 140px"
          :value="skin.groupSel.get(row.group!.signature)"
          :options="row.group!.parts.map((p) => ({ label: p.fileName, value: p.path }))"
          @update:value="(v: string) => skin.changeVariant(row.group!, v)"
        />
      </div>
      <div class="meta" v-if="skin.partInfoOf(row.group!)">
        {{ skin.partInfoOf(row.group!)!.vertexCount }} 顶点 · {{ skin.partInfoOf(row.group!)!.triangleCount }} 三角形 ·
        骨骼 {{ bonesText(skin.partInfoOf(row.group!)!) }}
        <br />
        纹理: {{ texShort(skin.partInfoOf(row.group!)!) }}
      </div>
    </div>
  </template>
</template>

<script setup lang="ts">
import { computed } from "vue";
import type { SkinStore } from "../composables/useSkinMode";
import type { DressStore } from "../composables/useDressMode";
import type { PartGroup, PartInfo } from "../types";
import { shortName } from "../utils";

const props = defineProps<{ skin: SkinStore; dress?: DressStore }>();

/** 基础槽被装备/时装替换的来源标签（角色装扮模式） */
const replacedBy = (slot: string) => props.dress?.replacedBaseSlots[slot] ?? null;

/** 独立组显示名：girl_head / boy_head 显示为「头」（保留" (附加)"等后缀） */
const HEAD_RE = /(?:girl|boy)_head/i;
const groupLabel = (g: PartGroup) => g.label.replace(HEAD_RE, "头");

/** 槽卡片固定身体顺序：头组置顶，其后 头发→上衣→下装→鞋子，其余独立组殿后 */
const SLOT_ORDER = ["hair", "upper", "lower", "boot"];
const slotRank = (s: string) => {
  const i = SLOT_ORDER.indexOf(s);
  return i === -1 ? SLOT_ORDER.length : i;
};
const rows = computed(() => {
  const groups = props.skin.modelSet?.groups ?? [];
  const slots = [...new Set(groups.map((g) => g.slot).filter((s): s is string => !!s))].sort(
    (a, b) => slotRank(a) - slotRank(b),
  );
  const independent = groups.filter((g) => !g.slot);
  const head = independent.filter((g) => HEAD_RE.test(g.label));
  const rest = independent.filter((g) => !HEAD_RE.test(g.label));
  return [
    ...head.map((g) => ({ key: g.signature, slot: null as string | null, group: g as PartGroup | null })),
    ...slots.map((s) => ({ key: `slot:${s}`, slot: s, group: null as PartGroup | null })),
    ...rest.map((g) => ({ key: g.signature, slot: null as string | null, group: g as PartGroup | null })),
  ];
});

const slotLabels: Record<string, string> = {
  hair: "头发",
  upper: "上衣",
  lower: "下装",
  boot: "鞋子",
};
const slotLabel = (s: string) => slotLabels[s] ?? s;

/** 槽当前部件的描述信息 */
function slotInfo(slot: string): PartInfo | null {
  const cur = props.skin.slotCurrent(slot);
  if (!cur) return null;
  return props.skin.slotOptions(slot).find((p) => p.path === cur) ?? null;
}

/** 槽选项显示名：根目录成员唯一时 = 默认（基础模型目录语义）；
 *  多个根目录成员（装备目录单模型预览，FS01_Upper / MS01_Upper）用文件名区分，
 *  YiFu 换装取尾部编号（girl_Upper_1 → 款式 1） */
function slotSelectOptions(slot: string): { label: string; value: string }[] {
  const parts = props.skin.slotOptions(slot);
  const rootCount = parts.filter((p) => p.inRoot).length;
  return parts.map((p) => ({
    label: variantLabel(p, rootCount),
    value: p.path,
  }));
}

function variantLabel(p: PartInfo, rootCount: number): string {
  if (p.inRoot) {
    return rootCount > 1 ? p.fileName.replace(/\.fskin$/i, "") : "默认";
  }
  const m = p.fileName.match(/_(\d+)\.fskin$/i);
  if (m) return `款式 ${Number(m[1])}`;
  return p.fileName.replace(/\.fskin$/i, "").replace(/^girl_/i, "");
}

function bonesText(p: PartInfo) {
  if (!p.bones.length) return "-";
  const hex = p.bones.map((b) => "0x" + b.toString(16));
  return hex.length > 4 ? `${hex[0]}..${hex[hex.length - 1]} (${hex.length})` : hex.join(",");
}

function texShort(p: PartInfo) {
  return p.textureFile ? shortName(p.textureFile) : "(未找到)";
}
</script>
