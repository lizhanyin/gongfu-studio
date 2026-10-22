<!-- 检查器（可折叠）：换装变体 / 纹理切换 / 动作 / FAK / FSM / 部件检查。
     从 App.vue 挪出为独立组件：改本文件 = 只重载检查器子树（plugin-vue 自接受），
     视口与资源树不受影响；内部折叠状态在重载时重置。 -->
<template>
  <aside class="inspector" :class="{ collapsed: inspectorCollapsed }">
    <div class="inspector-head">
      <span v-if="!inspectorCollapsed">检查器</span>
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
        class="inspector-collapse"
        display-directive="show"
        :default-expanded-names="['variants', 'tex', 'act', 'fak', 'fsm', 'part']"
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
        <n-collapse-item v-if="fak.fakModel" title="FAK 信息" name="fak">
          <FakFileInfo :fak="fak" />
        </n-collapse-item>
        <n-collapse-item v-if="fsm.fsmModel" title="FSM 信息" name="fsm">
          <FsmFileInfo :fsm="fsm" />
        </n-collapse-item>
        <n-collapse-item v-if="skin.selectedPart" title="部件检查" name="part">
          <PartInspector :skin="skin" />
        </n-collapse-item>
      </n-collapse>
      <div
        v-if="!skin.hasParts && !act.actMeta && !fak.fakModel && !fsm.fsmModel"
        class="inspector-empty"
      >
        从资源网格选择文件预览
      </div>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import type { SkinStore } from "../composables/useSkinMode";
import type { ActStore } from "../composables/useActMode";
import type { FakStore } from "../composables/useFakMode";
import type { FsmStore } from "../composables/useFsmMode";
import type { DressStore } from "../composables/useDressMode";
import DressPanel from "./DressPanel.vue";
import PartGroupList from "./PartGroupList.vue";
import TexSlotPanel from "./TexSlotPanel.vue";
import ActFileInfo from "./ActFileInfo.vue";
import FakFileInfo from "./FakFileInfo.vue";
import FsmFileInfo from "./FsmFileInfo.vue";
import PartInspector from "./PartInspector.vue";

const props = defineProps<{
  skin: SkinStore;
  act: ActStore;
  fak: FakStore;
  fsm: FsmStore;
  dress?: DressStore;
}>();

/** 帽子挂点骨骼候选：当前骨架的骨名 */
const actSkeletonNames = computed(() => props.act.actMeta?.boneNames ?? []);

const inspectorCollapsed = ref(false);
</script>

