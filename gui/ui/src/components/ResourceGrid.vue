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
    <div
      ref="bodyRef"
      class="grid-body"
      @scroll.passive="onScroll"
    >
      <div v-if="!tree.gridLoading && !tree.gridEntries.length" class="grid-empty">空目录</div>
      <div v-else class="grid-spacer" :style="{ height: totalH + 'px' }">
        <div
          v-for="row in visibleRows"
          :key="row.i"
          class="grid-row"
          :style="{ top: row.i * PITCH + 'px', gridTemplateColumns: `repeat(${cols}, minmax(0, 1fr))` }"
        >
          <div
            v-for="e in row.items"
            :key="e.path"
            class="res-card"
            :class="{ dir: e.isDir, model: e.isModel }"
            :title="e.path"
            @click="onClick(e)"
            @contextmenu="onCtx($event, e)"
          >
            <div class="card-top">
              <span class="kind" :class="e.isDir ? (e.isCharacter ? 'character' : e.isModel ? 'model' : 'dir') : e.kind">
                {{ kindTag(e) }}
              </span>
              <n-button
                v-if="e.isDir && (e.isModel || e.isCharacter)"
                size="tiny"
                class="whole-btn"
                @click.stop="e.isCharacter ? emit('dress', e.path) : emit('previewModel', e.path)"
              >{{ e.isCharacter ? "角色装扮" : "整体预览" }}</n-button>
            </div>
            <div class="card-name">{{ e.name }}</div>
            <div class="card-meta">{{ metaText(e) }}</div>
          </div>
        </div>
      </div>
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
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { DropdownOption } from "naive-ui";
import type { ResourceTreeStore } from "../composables/useResourceTree";
import type { ResEntry } from "../types";
import { copyText } from "../utils";
import { message } from "../message";

const props = defineProps<{ tree: ResourceTreeStore }>();
const emit = defineEmits<{
  (e: "preview", entry: ResEntry): void;
  (e: "openDir", path: string): void;
  (e: "previewModel", path: string): void;
  (e: "overlayAdd", entry: ResEntry): void;
  (e: "dress", path: string): void;
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
  if (e.isDir) return e.isCharacter ? "角色" : e.isModel ? "模型" : "目录";
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

// ---------- 虚拟滚动 ----------
// 尺寸常量需与 style.css 一致：卡片定高 CARD_H、行/列间距 GAP、最小列宽 CARD_MIN_W
const CARD_H = 82;
const GAP = 8;
const CARD_MIN_W = 150;
const PITCH = CARD_H + GAP;
const OVERSCAN = 2;

const bodyRef = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewH = ref(300);
const cols = ref(1);

let ro: ResizeObserver | null = null;
onMounted(() => {
  const el = bodyRef.value;
  if (!el) return;
  ro = new ResizeObserver((entries) => {
    const r = entries[entries.length - 1].contentRect;
    viewH.value = r.height || viewH.value;
    // 与原 CSS repeat(auto-fill, minmax(CARD_MIN_W, 1fr)) 同公式的列数
    cols.value = Math.max(1, Math.floor((r.width + GAP) / (CARD_MIN_W + GAP)));
  });
  ro.observe(el);
});
onBeforeUnmount(() => ro?.disconnect());

/** 按当前列数把条目切成行 */
const rows = computed(() => {
  const list = props.tree.gridEntries;
  const n = cols.value;
  const out: ResEntry[][] = [];
  for (let i = 0; i < list.length; i += n) out.push(list.slice(i, i + n));
  return out;
});
const totalH = computed(() => rows.value.length * PITCH);
const visibleRows = computed(() => {
  const n = rows.value.length;
  const first = Math.max(0, Math.floor(scrollTop.value / PITCH) - OVERSCAN);
  const last = Math.min(n, Math.ceil((scrollTop.value + viewH.value) / PITCH) + OVERSCAN);
  const out: { i: number; items: ResEntry[] }[] = [];
  for (let i = first; i < last; i++) out.push({ i, items: rows.value[i] });
  return out;
});

function onScroll() {
  scrollTop.value = bodyRef.value?.scrollTop ?? 0;
}

// 切换目录 / 刷新后回到顶部
watch(
  () => props.tree.gridEntries,
  () => {
    scrollTop.value = 0;
    if (bodyRef.value) bodyRef.value.scrollTop = 0;
  },
);

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
    if (e.isCharacter) opts.push({ key: "dress", label: "角色装扮" });
    if (e.isModel) opts.push({ key: "model", label: "整体预览模型" });
    opts.push({ key: "enter", label: "进入目录" });
    opts.push({ type: "divider" }, { key: "export", label: "导出文件" });
    if (!props.tree.isCpk) opts.push({ key: "import", label: "导入文件" });
  } else {
    opts.push({ key: "preview", label: "预览" });
    if (e.kind === "fskin") opts.push({ key: "add", label: "叠加到当前场景" });
    opts.push({ type: "divider" }, { key: "export", label: "导出文件" });
  }
  opts.push({ key: "copyname", label: "复制文件名" });
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

async function onMenuSelect(key: string | number) {
  const e = menuEntry.value;
  menuShow.value = false;
  if (!e) return;
  if (key === "model") emit("previewModel", e.path);
  else if (key === "dress") emit("dress", e.path);
  else if (key === "enter") emit("openDir", e.path);
  else if (key === "preview") emit("preview", e);
  else if (key === "add") emit("overlayAdd", e);
  else if (key === "export") {
    if (e.isDir) props.tree.exportDir(e.path);
    else props.tree.exportFile(e.path, e.name);
  } else if (key === "import") props.tree.importToDir(e.path);
  else if (key === "copyname") {
    (await copyText(e.name))
      ? message.success(`已复制: ${e.name}`)
      : message.error("复制失败");
  }
}
</script>
