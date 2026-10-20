<template>
  <div class="res-grid">
    <div class="grid-head">
      <n-breadcrumb>
        <n-breadcrumb-item v-for="(c, i) in crumbs" :key="c.path" @click="emit('openDir', c.path)">
          {{ i === crumbs.length - 1 ? c.name : c.name + " /" }}
        </n-breadcrumb-item>
      </n-breadcrumb>
      <n-button size="tiny" :disabled="!canUp" @click="goUp">↑ 上级</n-button>
      <span class="meta-inline">{{ tree.gridEntries.length }} 项</span>
      <n-spin v-if="tree.gridLoading" :size="12" />
    </div>
    <div class="grid-body">
      <div
        v-for="e in tree.gridEntries"
        :key="e.path"
        class="res-card"
        :class="{ dir: e.isDir, model: e.isModel }"
        :title="e.path"
        @click="onClick(e)"
        @contextmenu="onCtx($event, e)"
      >
        <div class="card-top">
          <span class="kind" :class="e.isDir ? (e.isModel ? 'model' : 'dir') : e.kind">
            {{ kindTag(e) }}
          </span>
          <n-button
            v-if="e.isDir && e.isModel"
            size="tiny"
            class="whole-btn"
            @click.stop="emit('previewModel', e.path)"
          >整体预览</n-button>
        </div>
        <div class="card-name">{{ e.name }}</div>
        <div class="card-meta">{{ metaText(e) }}</div>
      </div>
      <div v-if="!tree.gridLoading && !tree.gridEntries.length" class="grid-empty">空目录</div>
    </div>
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
import { computed, nextTick, ref } from "vue";
import type { DropdownOption } from "naive-ui";
import type { ResourceTreeStore } from "../composables/useResourceTree";
import type { ResEntry } from "../types";

const props = defineProps<{ tree: ResourceTreeStore }>();
const emit = defineEmits<{
  (e: "preview", entry: ResEntry): void;
  (e: "openDir", path: string): void;
  (e: "previewModel", path: string): void;
  (e: "overlayAdd", entry: ResEntry): void;
}>();

// ---------- 面包屑 ----------
const crumbs = computed(() => {
  const root = props.tree.rootDir.replace(/[\\/]+$/, "");
  const cur = props.tree.currentDir ?? root;
  const rel = cur.toLowerCase().startsWith(root.toLowerCase())
    ? cur.slice(root.length).replace(/^[\\/]+/, "")
    : "";
  const parts = rel ? rel.split(/[\\/]+/) : [];
  return [
    { name: props.tree.treeData[0]?.label ?? "根", path: root },
    ...parts.map((_, i) => ({
      name: parts[i],
      path: root + "\\" + parts.slice(0, i + 1).join("\\"),
    })),
  ];
});
const canUp = computed(() => crumbs.value.length > 1);
function goUp() {
  const p = crumbs.value[crumbs.value.length - 2];
  if (p) emit("openDir", p.path);
}

// ---------- 卡片 ----------
function kindTag(e: ResEntry): string {
  if (e.isDir) return e.isModel ? "模型" : "目录";
  const map: Record<string, string> = {
    fskin: "皮肤",
    fskel: "动作",
    fak: "模型",
    dds: "贴图",
    tga: "贴图",
    png: "贴图",
    csv: "CSV",
  };
  return map[e.kind] ?? (e.kind ? e.kind.toUpperCase() : "文件");
}

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(2)} MB`;
}

function metaText(e: ResEntry): string {
  if (e.isDir) return "点击进入";
  if (e.kind === "fskin") return `${e.vertexCount ?? "?"} 顶点 · ${e.triangleCount ?? "?"} 三角`;
  if (e.kind === "fskel") return `${e.frameCount ?? "?"} 帧 · ${e.boneCount ?? "?"} 骨`;
  if (e.kind === "fak")
    return `${e.vertexCount ?? "?"} 顶点 · ${e.triangleCount ?? "?"} 三角 · ${e.frameCount ?? "?"} 帧`;
  return fmtSize(e.byteLen);
}

function onClick(e: ResEntry) {
  if (e.isDir) emit("openDir", e.path);
  else emit("preview", e);
}

// ---------- 右键菜单 ----------
const menuShow = ref(false);
const menuX = ref(0);
const menuY = ref(0);
const menuEntry = ref<ResEntry | null>(null);

const menuOptions = computed<DropdownOption[]>(() => {
  const e = menuEntry.value;
  if (!e) return [];
  const opts: DropdownOption[] = [];
  if (e.isDir) {
    if (e.isModel) opts.push({ key: "model", label: "整体预览模型" });
    opts.push({ key: "enter", label: "进入目录" });
  } else {
    opts.push({ key: "preview", label: "预览" });
    if (e.kind === "fskin") opts.push({ key: "add", label: "叠加到当前场景" });
  }
  return opts;
});

function onCtx(ev: MouseEvent, e: ResEntry) {
  ev.preventDefault();
  menuEntry.value = e;
  menuShow.value = false;
  nextTick(() => {
    menuX.value = ev.clientX;
    menuY.value = ev.clientY;
    menuShow.value = true;
  });
}

function onMenuSelect(key: string | number) {
  const e = menuEntry.value;
  menuShow.value = false;
  if (!e) return;
  if (key === "model") emit("previewModel", e.path);
  else if (key === "enter") emit("openDir", e.path);
  else if (key === "preview") emit("preview", e);
  else if (key === "add") emit("overlayAdd", e);
}
</script>
