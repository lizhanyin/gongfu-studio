<!-- 检查器「骨骼」折叠项：全部骨骼名称列表（可过滤），点击联动 3D 高亮，
     反向接收 viewer 点击骨线条/关节的选中回调（selectedBone）。 -->
<template>
  <div class="bone-list">
    <n-input
      v-model:value="filter"
      size="tiny"
      placeholder="过滤骨名 / #序号"
      clearable
    />
    <div class="bone-rows">
      <div
        v-for="item in filtered"
        :key="item.idx"
        class="bone-row"
        :class="{ sel: act.selectedBone === item.idx }"
        @click="act.selectBone(item.idx)"
        :title="`#${item.idx} ${item.name}`"
      >
        <span class="bone-idx">#{{ item.idx }}</span>
        <span class="bone-name">{{ item.name }}</span>
      </div>
      <div v-if="!filtered.length" class="bone-empty">无匹配骨骼</div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import type { ActStore } from "../composables/useActMode";

const props = defineProps<{ act: ActStore }>();
const filter = ref("");
const filtered = computed(() => {
  const kw = filter.value.trim().toLowerCase();
  const names = props.act.boneList ?? [];
  const out: { idx: number; name: string }[] = [];
  names.forEach((name, idx) => {
    if (!kw || name.toLowerCase().includes(kw) || String(idx).includes(kw.replace(/^#/, ""))) {
      out.push({ idx, name });
    }
  });
  return out;
});
</script>

<style scoped>
.bone-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.bone-rows {
  max-height: 300px;
  overflow-y: auto;
  border: 1px solid var(--n-border-color, #3a3a3a);
  border-radius: 4px;
}
.bone-row {
  display: flex;
  gap: 8px;
  padding: 2px 8px;
  cursor: pointer;
  font-size: 12px;
  line-height: 18px;
  white-space: nowrap;
}
.bone-row:hover {
  background: rgba(127, 127, 127, 0.18);
}
.bone-row.sel {
  background: rgba(255, 77, 77, 0.22);
  color: #ff7b7b;
}
.bone-idx {
  color: #8f9aa5;
  min-width: 34px;
  text-align: right;
}
.bone-empty {
  padding: 8px;
  color: #8f9aa5;
  font-size: 12px;
  text-align: center;
}
</style>
