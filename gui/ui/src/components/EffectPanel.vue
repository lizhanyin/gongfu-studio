<template>
  <div class="fx-panel">
    <!-- 添加：全量下拉（按 Effect 首级目录分组 = 游戏侧用途语义） -->
    <div class="fx-add-row">
      <n-select
        size="tiny"
        filterable
        clearable
        v-model:value="pick"
        :options="catalogOptions"
        :loading="fx.catalogLoading"
        :placeholder="fx.catalog.length ? '选择特效…' : '当前根无 system\\Effect'"
      />
      <n-button size="tiny" type="primary" :disabled="!pick" @click="add">
        挂上
      </n-button>
    </div>
    <div v-if="siblingHint" class="fx-hint">{{ siblingHint }}</div>

    <!-- NPC 原版配置一键还原（System\NPC\*_a.csv 的 eft1/eft2 挂骨对） -->
    <div class="fx-add-row" v-if="fx.npcCsvList.length">
      <n-select
        size="tiny"
        filterable
        clearable
        v-model:value="npcPick"
        :options="npcOptions"
        placeholder="NPC 原版配置还原…"
      />
      <n-button size="tiny" :disabled="!npcPick" @click="importNpc">还原</n-button>
    </div>

    <div v-if="!fx.count" class="fx-empty">
      选择 .sfx 挂到当前角色（挂骨跟随 / 世界放置，可叠挂多个）
    </div>

    <!-- 叠挂实例列表：每类控件独立成行，数字框弹性分配（默认 320px 检查器宽不挤） -->
    <div class="fx-item" v-for="inst in list" :key="inst.id">
      <div class="fx-head">
        <n-switch size="small" :value="inst.visible" @update:value="(v: boolean) => fx.setTransform(inst.id, { visible: v })" />
        <span class="fx-name" :title="`${inst.path}（${nodeSummary(inst)}）`">{{ shortName(inst.path) }}</span>
        <span class="fx-meta">{{ inst.nodes.length }}节点</span>
        <n-button size="tiny" quaternary :title="inst.playing ? '暂停' : '播放'" @click="fx.togglePlay(inst.id)">
          {{ inst.playing ? "⏸" : "▶" }}
        </n-button>
        <n-button size="tiny" quaternary title="重播" @click="fx.replayInst(inst.id)">↻</n-button>
        <n-button size="tiny" quaternary title="移除" @click="fx.remove(inst.id)">✕</n-button>
      </div>

      <!-- 播放进度（引擎 5000ms 生命周期；循环 = 进度回绕） -->
      <div class="fx-row">
        <div class="fx-bar"><div class="fx-bar-fill" :style="{ width: barPct(inst) + '%' }"></div></div>
        <span class="fx-time">{{ inst.elapsed.toFixed(1) }}/{{ inst.lifetime.toFixed(0) }}s</span>
        <span class="ctl-lbl">速度</span>
        <n-input-number
          size="tiny"
          class="fx-num-s"
          :value="inst.speed"
          :min="0.1"
          :max="8"
          :step="0.5"
          :show-button="false"
          @update:value="(v: number | null) => (inst.speed = v ?? 1)"
        />
      </div>

      <!-- 开关行：循环 / 叠加 / 缩放 -->
      <div class="fx-row">
        <span class="ctl-lbl">循环</span>
        <n-switch size="small" :value="inst.loop" @update:value="(v: boolean) => (inst.loop = v)" />
        <span class="ctl-lbl">叠加</span>
        <n-switch size="small" :value="inst.additive" @update:value="(v: boolean) => fx.setTransform(inst.id, { additive: v })" />
        <span class="ctl-lbl">缩放</span>
        <n-input-number
          size="tiny"
          class="fx-num-s"
          :value="inst.scale"
          :min="0.05"
          :max="20"
          :step="0.25"
          :show-button="false"
          @update:value="(nv: number | null) => fx.setTransform(inst.id, { scale: nv ?? 1 })"
        />
      </div>

      <!-- 挂点：挂骨（tag 组在前）/ 世界放置 -->
      <div class="fx-row">
        <n-radio-group
          size="tiny"
          :value="inst.mode"
          @update:value="(m: string) => fx.setTransform(inst.id, { mode: m as 'bone' | 'world', bone: m === 'bone' ? inst.bone || defaultBone : '' })"
        >
          <n-radio-button value="bone">挂骨</n-radio-button>
          <n-radio-button value="world">世界</n-radio-button>
        </n-radio-group>
        <n-select
          v-if="inst.mode === 'bone'"
          size="tiny"
          filterable
          class="fx-bone"
          :value="inst.bone"
          :options="boneOptions"
          placeholder="选择挂骨…"
          @update:value="(b: string) => fx.setTransform(inst.id, { bone: b })"
        />
        <div v-else class="fx-head-spacer"></div>
        <n-button
          v-if="inst.mode === 'world'"
          size="tiny"
          title="取 .sfx 根矩阵平移（编辑器摆放位）"
          @click="takeRootTrans(inst)"
        >
          取根矩阵
        </n-button>
      </div>

      <!-- 世界坐标（世界模式；X/Y/Z 弹性三等分） -->
      <div class="fx-row" v-if="inst.mode === 'world'">
        <template v-for="(v, i) in inst.worldPos" :key="'w' + i">
          <span class="ctl-lbl">{{ "XYZ"[i] }}</span>
          <n-input-number
            size="tiny"
            class="fx-num"
            :value="v"
            :step="10"
            :show-button="false"
            @update:value="(nv: number | null) => setWorldPos(inst, i, nv)"
          />
        </template>
      </div>

      <!-- 偏移 / 旋转（骨局部系；写记忆），各占一行三等分 -->
      <div class="fx-row">
        <span class="ctl-lbl">偏移</span>
        <n-input-number
          v-for="(v, i) in inst.offset"
          :key="'o' + i"
          size="tiny"
          class="fx-num"
          :value="v"
          :step="1"
          :show-button="false"
          @update:value="(nv: number | null) => setVec(inst, 'offset', i, nv)"
        />
      </div>
      <div class="fx-row">
        <span class="ctl-lbl">旋转</span>
        <n-input-number
          v-for="(v, i) in inst.euler"
          :key="'e' + i"
          size="tiny"
          class="fx-num"
          :value="v"
          :step="15"
          :show-button="false"
          @update:value="(nv: number | null) => setVec(inst, 'euler', i, nv)"
        />
      </div>

      <!-- 节点静音（≈引擎 FindEffect 粒度；音效/网格节点本就不渲染） -->
      <div class="fx-nodes" v-if="inst.nodes.length">
        <button
          v-for="(n, i) in inst.nodes"
          :key="i"
          class="fx-chip"
          :class="{ off: inst.nodeMute[i], nor: !n.hasTex }"
          :title="`${n.kind}${n.name ? ' · ' + n.name : ''}${n.hasSnd ? '（音效：检查器可试听）' : ''}${n.hasMesh ? '（网格节点暂不渲染）' : ''}`"
          @click="fx.toggleNode(inst.id, i)"
        >
          #{{ i }} {{ kindLabel(n.kind) }}{{ n.name ? `·${n.name}` : "" }}{{ !n.hasTex ? "·无渲染" : "" }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { EffectStore, FxAttach } from "../composables/useEffectMode";
import type { ActStore } from "../composables/useActMode";
import type { ResourceTreeStore } from "../composables/useResourceTree";
import { shortName } from "../utils";

const props = defineProps<{ fx: EffectStore; act: ActStore; res: ResourceTreeStore }>();

const pick = ref<string | null>(null);
const npcPick = ref<string | null>(null);

/** 资源根（与 App 的装扮重扫同源口径）；变化即重扫特效清单与 NPC 配置表 */
const root = computed(() => (props.res.treeData[0]?.key as string | undefined) ?? "");
watch(root, (r) => props.fx.ensureCatalog(r), { immediate: true });

/** 全量下拉：按 Effect 首级目录分组（Combat/Scn/role/Quest = 游戏用途语义） */
const catalogOptions = computed(() => {
  const groups = new Map<string, { label: string; value: string }[]>();
  for (const e of props.fx.catalog) {
    if (!groups.has(e.group)) groups.set(e.group, []);
    groups.get(e.group)!.push({ label: e.name, value: e.path });
  }
  return [...groups.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([g, children]) => ({ type: "group" as const, label: g, key: g, children }));
});

const npcOptions = computed(() =>
  props.fx.npcCsvList.map((c) => ({ label: c.name, value: c.path }))
);

/** _a/_b 变体配对提示（游戏里同一属性名的攻击两段） */
const siblingHint = computed(() => {
  const p = pick.value;
  if (!p || !/_a\.sfx$/i.test(p)) return null;
  const other = p.replace(/_a\.sfx$/i, "_b.sfx");
  return props.fx.catalog.some((e) => e.path.toLowerCase() === other.toLowerCase())
    ? `存在变体 ${shortName(other)}（_a/_b = 游戏内攻击两段特效）`
    : null;
});

/** 挂骨下拉：tag 挂点骨组在前（引擎正式挂点体系），全部骨骼兜底 */
const boneOptions = computed(() => {
  const names = props.act.actMeta?.boneNames ?? [];
  const tag = names.filter((n) => /^tag_/i.test(n));
  const rest = names.filter((n) => !/^tag_/i.test(n));
  return [
    ...(tag.length
      ? [{ type: "group" as const, label: "挂点骨 (tag_)", key: "tag", children: tag.map((n) => ({ label: n, value: n })) }]
      : []),
    { type: "group" as const, label: "全部骨骼", key: "all", children: rest.map((n) => ({ label: n, value: n })) },
  ];
});

/** 无记忆时的默认挂骨：tag 武器挂点（角色特效最常见锚），无则空=原点 */
const defaultBone = computed(() => {
  const names = props.act.actMeta?.boneNames ?? [];
  return names.find((n) => /^tag_rweapon$/i.test(n)) ?? names.find((n) => /^tag_/i.test(n)) ?? "";
});

const list = computed(() => [...props.fx.instances.values()]);

function add() {
  if (pick.value) props.fx.attach(pick.value);
}

async function importNpc() {
  if (!npcPick.value) return;
  await props.fx.importNpcCsv(npcPick.value, root.value);
}

function setVec(inst: FxAttach, key: "offset" | "euler", i: number, v: number | null) {
  const arr = [...inst[key]] as [number, number, number];
  arr[i] = v ?? 0;
  props.fx.setTransform(inst.id, { [key]: arr } as Partial<Parameters<EffectStore["setTransform"]>[1]>);
}

function setWorldPos(inst: FxAttach, i: number, v: number | null) {
  const arr = [...inst.worldPos] as [number, number, number];
  arr[i] = v ?? 0;
  props.fx.setTransform(inst.id, { worldPos: arr });
}

function takeRootTrans(inst: FxAttach) {
  props.fx.setTransform(inst.id, { worldPos: [...inst.rootTrans] as [number, number, number] });
}

function barPct(inst: FxAttach) {
  return Math.min(100, (inst.elapsed / Math.max(inst.lifetime, 0.01)) * 100);
}

function nodeSummary(inst: FxAttach): string {
  const kinds = new Set(inst.nodes.map((n) => n.kind));
  const tex = inst.nodes.filter((n) => n.hasTex).length;
  return `${inst.nodes.length}节点·可渲染${tex}·${kinds.size}类`;
}

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
  return KIND_LABELS[kind] ?? kind;
}
</script>

<style scoped>
.fx-panel {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.fx-add-row {
  display: flex;
  gap: 4px;
}
.fx-add-row .n-select {
  flex: 1;
}
.fx-hint {
  font-size: 11px;
  color: #8fa3c8;
}
.fx-empty {
  font-size: 12px;
  color: #556;
  border: 1px dashed #333;
  border-radius: 3px;
  padding: 8px;
  text-align: center;
}
.fx-item {
  border: 1px solid #2a2c36;
  border-radius: 4px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.fx-head {
  display: flex;
  align-items: center;
  gap: 6px;
}
.fx-name {
  font-size: 12px;
  font-weight: 600;
  max-width: 45%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fx-meta {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: #889;
}
/* 挂点行：世界模式下把「取根矩阵」按钮推到右侧 */
.fx-head-spacer {
  flex: 1;
}
.fx-bar {
  flex: 1;
  height: 6px;
  background: #22242a;
  border-radius: 3px;
  overflow: hidden;
}
.fx-bar-fill {
  height: 100%;
  background: #63e2b7;
}
.fx-time {
  font-size: 11px;
  color: #99a;
  min-width: 52px;
  text-align: right;
}
.ctl-lbl {
  font-size: 11px;
  color: #99a;
  flex-shrink: 0;
}
/* 数字框：三等分弹性（去步进按钮，窄面板不溢出；滚轮/键入仍可调） */
.fx-num {
  flex: 1;
  min-width: 0;
}
.fx-num-s {
  width: 50px;
  flex-shrink: 0;
}
.fx-row {
  display: flex;
  align-items: center;
  gap: 4px;
}
.fx-bone {
  flex: 1;
  min-width: 90px;
}
.fx-nodes {
  display: flex;
  flex-wrap: wrap;
  gap: 3px;
}
.fx-chip {
  font-size: 11px;
  color: #cdd6e4;
  background: #22242a;
  border: 1px solid #333;
  border-radius: 3px;
  padding: 1px 6px;
  cursor: pointer;
}
.fx-chip:hover {
  border-color: #63e2b7;
}
.fx-chip.off {
  color: #667;
  text-decoration: line-through;
}
.fx-chip.nor {
  color: #776;
  cursor: default;
}
</style>
