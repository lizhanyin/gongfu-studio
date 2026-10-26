<!-- 检查器（可折叠）：换装变体 / 纹理切换 / 动作 / FAK / FSM / 部件检查。
     从 App.vue 挪出为独立组件：改本文件 = 只重载检查器子树（plugin-vue 自接受），
     视口与资源树不受影响；内部折叠状态在重载时重置。 -->
<template>
  <aside
    class="inspector"
    :class="{ collapsed: inspectorCollapsed, dragging: inspectorDragging }"
    :style="!inspectorCollapsed ? { width: inspectorWidth + 'px' } : undefined"
  >
    <!-- 左缘拖拽调整宽度（折叠时隐藏） -->
    <div
      v-if="!inspectorCollapsed"
      class="drag-handle drag-v left-edge"
      title="拖拽调整宽度"
      @mousedown="startInspectorDrag"
    ></div>
    <div class="inspector-head">
      <span v-if="!inspectorCollapsed">检查器</span>
      <!-- 浏览时装目录时：顶部常驻基础模型（性别）切换，便于对照男女款 -->
      <n-button-group
        v-if="!inspectorCollapsed && fashionBrowsing && dress?.catalog"
        size="tiny"
        class="head-base-switch"
      >
        <n-button
          v-for="b in dress.catalog.bases"
          :key="b.key"
          size="tiny"
          :type="dress.baseKey === b.key ? 'primary' : 'default'"
          :title="`切换基础模型: ${b.label}`"
          @click="dress.switchBase(b.key)"
        >
          {{ b.gender === "f" ? "女体" : "男体" }}
        </n-button>
      </n-button-group>
      <n-button
        size="tiny"
        quaternary
        :title="inspectorCollapsed ? '展开' : '折叠'"
        @click="inspectorCollapsed = !inspectorCollapsed"
      >
        {{ inspectorCollapsed ? "«" : "»" }}
      </n-button>
    </div>
    <div v-show="!inspectorCollapsed" class="inspector-body">
      <n-collapse
        v-model:expanded-names="expandedNames"
        class="inspector-collapse"
        display-directive="show"
      >
        <n-collapse-item
          v-if="skin.modelSet?.groups.length"
          title="换装变体"
          name="variants"
        >
          <DressPanel v-if="dress?.catalog" :dress="dress" :act-dir-skeleton="actSkeletonNames" />
          <PartGroupList :skin="skin" :dress="dress" />
        </n-collapse-item>
        <n-collapse-item
          v-if="skin.modelSet?.texSlots.length || skin.partTexVariants.length"
          title="纹理切换"
          name="tex"
        >
          <TexSlotPanel :skin="skin" />
        </n-collapse-item>
        <n-collapse-item v-if="act.actMeta || skin.hasParts" title="动作" name="act">
          <ActFileInfo :act="act" :act-dir="skin.rootDir + '\\act'" />
        </n-collapse-item>
        <n-collapse-item v-if="act.actMeta" title="骨骼" name="bones">
          <BoneList :act="act" />
        </n-collapse-item>
        <n-collapse-item
          v-if="skin.modelSet || dress?.catalog"
          title="特效挂载"
          name="fx"
        >
          <EffectPanel :fx="fx" :act="act" :res="res" />
        </n-collapse-item>
        <n-collapse-item v-if="fak.fakModel" title="FAK 信息" name="fak">
          <FakFileInfo :fak="fak" />
        </n-collapse-item>
        <n-collapse-item v-if="fsm.fsmModel" title="FSM 信息" name="fsm">
          <FsmFileInfo :fsm="fsm" />
        </n-collapse-item>
        <n-collapse-item v-if="fscn.fscnModel" title="FSCN 信息" name="fscn">
          <FscnFileInfo :fscn="fscn" />
        </n-collapse-item>
        <n-collapse-item v-if="sfx.sfxModel" title="SFX 特效" name="sfx">
          <SfxFileInfo :sfx="sfx" />
        </n-collapse-item>
        <n-collapse-item v-if="skin.selectedPart" title="部件检查" name="part">
          <PartInspector :skin="skin" />
        </n-collapse-item>
      </n-collapse>
      <div
        v-if="!skin.hasParts && !act.actMeta && !fak.fakModel && !fsm.fsmModel && !fscn.fscnModel && !sfx.sfxModel"
        class="inspector-empty"
      >
        从资源网格选择文件预览
      </div>
    </div>
    <!-- 拖拽提示浮标（Teleport 到 body 避免被 overflow 裁剪） -->
    <Teleport to="body">
      <div
        v-if="tip"
        class="drag-tip"
        :style="{ left: tip.x + 14 + 'px', top: tip.y + 14 + 'px' }"
      >
        检查器 {{ inspectorWidth }}px
      </div>
    </Teleport>
  </aside>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { SkinStore } from "../composables/useSkinMode";
import type { ActStore } from "../composables/useActMode";
import type { FakStore } from "../composables/useFakMode";
import type { FsmStore } from "../composables/useFsmMode";
import type { FscnStore } from "../composables/useFscnMode";
import type { SfxStore } from "../composables/useSfxMode";
import type { DressStore } from "../composables/useDressMode";
import type { EffectStore } from "../composables/useEffectMode";
import DressPanel from "./DressPanel.vue";
import PartGroupList from "./PartGroupList.vue";
import TexSlotPanel from "./TexSlotPanel.vue";
import ActFileInfo from "./ActFileInfo.vue";
import FakFileInfo from "./FakFileInfo.vue";
import FsmFileInfo from "./FsmFileInfo.vue";
import FscnFileInfo from "./FscnFileInfo.vue";
import SfxFileInfo from "./SfxFileInfo.vue";
import EffectPanel from "./EffectPanel.vue";
import PartInspector from "./PartInspector.vue";
import BoneList from "./BoneList.vue";
import type { ResourceTreeStore } from "../composables/useResourceTree";

const props = defineProps<{
  skin: SkinStore;
  act: ActStore;
  fak: FakStore;
  fsm: FsmStore;
  fscn: FscnStore;
  sfx: SfxStore;
  dress?: DressStore;
  fx: EffectStore;
  res: ResourceTreeStore;
}>();

/** 帽子挂点骨骼候选：当前骨架的骨名 */
const actSkeletonNames = computed(() => props.act.actMeta?.boneNames ?? []);

/** 正在浏览时装目录（shizhuang / transformfashion，磁盘与 cpk: 路径通用） */
const fashionBrowsing = computed(() => {
  const dir = (props.res.currentDir ?? "").toLowerCase();
  return dir
    .split(/[\\/]+/)
    .some((s) => s === "shizhuang" || s === "transformfashion");
});

const inspectorCollapsed = ref(false);

/** 首个可见分区（各项按文件类型 v-if，加载内容变了「第一项」随之变） */
const firstSection = computed(() => {
  if (props.skin.modelSet?.groups.length) return "variants";
  if (props.skin.modelSet?.texSlots.length || props.skin.partTexVariants.length) return "tex";
  if (props.act.actMeta || props.skin.hasParts) return "act";
  if (props.act.actMeta) return "bones";
  if (props.fak.fakModel) return "fak";
  if (props.fsm.fsmModel) return "fsm";
  if (props.fscn.fscnModel) return "fscn";
  if (props.sfx.sfxModel) return "sfx";
  if (props.skin.selectedPart) return "part";
  return null;
});
/** 默认只展开第一项、其余收起；切换文件类型导致首项变化时重置回来 */
const expandedNames = ref<string[]>([]);
watch(firstSection, (n) => {
  expandedNames.value = n ? [n] : [];
}, { immediate: true });

// ---------------- 宽度拖拽（左缘把手，提示浮标跟随光标，松手持久化） ----------------

const inspectorWidth = ref(
  (() => {
    const n = Number(localStorage.getItem("panel.inspectorW"));
    return Number.isFinite(n) && n > 0 ? Math.min(620, Math.max(220, n)) : 320;
  })()
);
const inspectorDragging = ref(false);
const tip = ref<{ x: number; y: number } | null>(null);

function startInspectorDrag(ev: MouseEvent) {
  ev.preventDefault();
  const x0 = ev.clientX;
  const w0 = inspectorWidth.value;
  inspectorDragging.value = true;
  const onMove = (e: MouseEvent) => {
    inspectorWidth.value = Math.min(620, Math.max(220, w0 - (e.clientX - x0)));
    tip.value = { x: e.clientX, y: e.clientY };
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    inspectorDragging.value = false;
    tip.value = null;
    document.body.classList.remove("panel-dragging");
    localStorage.setItem("panel.inspectorW", String(inspectorWidth.value));
  };
  document.body.classList.add("panel-dragging");
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
}
</script>

