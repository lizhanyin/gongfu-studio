<template>
  <div class="res-tree">
    <div class="section-title">资源目录</div>
    <n-tree
      block-line
      expand-on-click
      selectable
      :data="tree.treeData"
      :selected-keys="selKeys"
      key-field="key"
      label-field="label"
      :on-load="handleLoad"
      :node-props="nodeProps"
      :render-suffix="renderSuffix"
      @update:selected-keys="onSelect"
    />
    <n-dropdown
      placement="bottom-start"
      trigger="manual"
      :x="menuX"
      :y="menuY"
      :options="menuOptions"
      :show="menuShow"
      @select="onMenuSelect"
      @clickoutside="menuShow = false"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, h, nextTick, ref } from "vue";
import type { TreeOption } from "naive-ui";
import type { ResourceTreeStore } from "../composables/useResourceTree";
import { message } from "../message";

const props = defineProps<{ tree: ResourceTreeStore }>();
const emit = defineEmits<{ (e: "previewModel", path: string): void }>();

const selKeys = computed(() => (props.tree.currentDir ? [props.tree.currentDir] : []));

async function handleLoad(node: TreeOption) {
  try {
    node.children = await props.tree.loadChildren(String(node.key));
  } catch (e) {
    message.error(`子目录加载失败: ${e}`);
    node.children = [];
  }
}

function onSelect(keys: string[]) {
  if (keys.length && keys[0] !== props.tree.currentDir) {
    props.tree.selectDir(keys[0]);
  }
}

// ---------- 右键菜单 ----------
const menuShow = ref(false);
const menuX = ref(0);
const menuY = ref(0);
const menuNode = ref<TreeOption | null>(null);

const menuOptions = computed(() => {
  const o = menuNode.value;
  if (!o) return [];
  const opts: { key: string; label: string }[] = [{ key: "show", label: "在下方显示内容" }];
  if (o.isModel) opts.unshift({ key: "model", label: "整体预览模型" });
  return opts;
});

function nodeProps({ option }: { option: TreeOption }) {
  return {
    onContextmenu: (e: MouseEvent) => {
      e.preventDefault();
      menuNode.value = option;
      menuShow.value = false;
      nextTick(() => {
        menuX.value = e.clientX;
        menuY.value = e.clientY;
        menuShow.value = true;
      });
    },
  };
}

function onMenuSelect(key: string | number) {
  const p = String(menuNode.value?.key ?? "");
  menuShow.value = false;
  if (!p) return;
  if (key === "model") emit("previewModel", p);
  else if (key === "show") props.tree.selectDir(p);
}

/** 模型目录徽标 */
function renderSuffix({ option }: { option: TreeOption }) {
  return option.isModel ? h("span", { class: "model-badge" }, "模型") : null;
}
</script>
