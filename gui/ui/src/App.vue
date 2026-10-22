<template>
  <n-config-provider :theme="darkTheme" :locale="zhCN" :theme-overrides="themeOverrides">
    <div class="app">
      <header class="app-header">
        <span class="title">Gongfu Studio</span>
        <MenuBar :skin="skin" :act="act" :res="res" @dress="openDressMode()" />
        <div class="header-spacer"></div>
        <n-input
          v-model:value="res.rootDir"
          size="small"
          class="root-input"
          placeholder="资源根目录 (res / cpk:…)"
          title="回车切换；或用 文件 菜单打开"
          @keyup.enter="res.changeRoot(res.rootDir)"
        />
      </header>

      <div class="app-main">
        <!-- 左：资源树（res 为根，全高） -->
        <aside class="tree-panel">
          <ResourceTree :tree="res" @preview-model="previewModelDir" @dress="openDressMode" />
        </aside>

        <div class="right-col">
          <!-- 右上：预览/操作 -->
          <div class="preview-row">
            <div class="viewport" ref="viewportRef">
              <div class="viewport-overlay">
                <template v-if="fak.fakModel">
                  <div>FAK 顶点动画 · {{ fak.currentActionInfo?.name ?? "" }}</div>
                  <div>顶点: {{ fak.fakModel.vertexCount }} · 当前帧 {{ Math.floor(fak.absFrame) }}/{{ fak.fakModel.frames.length }}</div>
                </template>
                <template v-else-if="fsm.fsmModel">
                  <div>FSM 静态网格 · {{ shortName(fsm.fsmModel.path) }}</div>
                  <div>
                    顶点: {{ fsm.fsmModel.vertexCount }} · 三角形:
                    {{ fsm.fsmModel.submeshes.reduce((s, x) => s + x.triangles.length / 3, 0) }}
                  </div>
                </template>
                <template v-else>
                  <div v-if="act.actMeta">
                    动作 {{ act.currentActionInfo?.name ?? "" }} · 当前帧
                    {{ Math.floor(act.absFrame) }}/{{ act.totalFrames - 1 }}
                  </div>
                  <div>部件: {{ skin.visibleCount }} / 组: {{ skin.modelSet?.groups.length ?? 0 }}</div>
                  <div>顶点: {{ skin.statVerts }} · 三角形: {{ skin.statTris }}</div>
                </template>
              </div>
              <div class="viewport-hint">左键旋转 · 右键平移 · 滚轮缩放 · 点击网格选中部件</div>

              <PlayBar v-if="act.actMeta" :player="act" />
              <PlayBar v-if="fak.fakModel" :player="fak" />

              <!-- 贴图 2D 预览浮层 -->
              <div v-if="texPreview" class="tex2d">
                <div class="tex2d-bar">
                  <span>{{ texPreview.name }}</span>
                  <n-button size="tiny" @click="texPreview = null">关闭</n-button>
                </div>
                <img class="tex2d-img" :src="texPreview.url" />
              </div>
            </div>

            <!-- 无渲染宿主：viewer.ts 的 HMR 边界，改 viewer.ts 只重建视口 -->
            <ViewerHost :target="viewportRef" @ready="onViewerReady" />

            <!-- 检查器（可折叠）：独立组件，改 InspectorPanel.vue 只重载检查器子树 -->
            <InspectorPanel :skin="skin" :act="act" :fak="fak" :fsm="fsm" :dress="dress" />
          </div>

          <!-- 右下：当前目录资源网格 -->
          <div class="grid-panel">
            <ResourceGrid
              :tree="res"
              @preview="previewEntry"
              @open-dir="res.selectDir"
              @preview-model="previewModelDir"
              @overlay-add="overlayAdd"
              @dress="openDressMode"
            />
          </div>
        </div>
      </div>

      <footer class="app-footer">
        <span v-if="dress.catalog">装扮: {{ dress.catalog.root }}</span>
        <span v-if="skin.modelSet">模型: {{ skin.modelSet.rootDir }}</span>
        <span v-if="act.actMeta">动作: {{ act.actMeta.fileName }}</span>
        <span v-if="fak.fakModel">FAK: {{ shortName(fak.fakModel.path) }}</span>
        <span v-if="fsm.fsmModel">FSM: {{ shortName(fsm.fsmModel.path) }}</span>
        <span>可见部件: {{ skin.visiblePaths.map(shortName).join(" + ") || "无" }}</span>
      </footer>
    </div>
  </n-config-provider>
</template>

<script setup lang="ts">
import { onMounted, ref } from "vue";
import { darkTheme, zhCN, type GlobalThemeOverrides } from "naive-ui";
import { useViewer } from "./composables/useViewer";
import { useSkinMode } from "./composables/useSkinMode";
import { useActMode } from "./composables/useActMode";
import { useFakMode } from "./composables/useFakMode";
import { useFsmMode } from "./composables/useFsmMode";
import { useDressMode } from "./composables/useDressMode";
import { useResourceTree } from "./composables/useResourceTree";
import { texDataFromDto, b64ToBytes, decodeTga, imageUrlFromB64 } from "./tex";
import type { TexData, Viewer } from "./viewer";
import { api } from "./api";
import { message } from "./message";
import { shortName } from "./utils";
import type { ResEntry } from "./types";
import MenuBar from "./components/MenuBar.vue";
import ResourceTree from "./components/ResourceTree.vue";
import ResourceGrid from "./components/ResourceGrid.vue";
import PlayBar from "./components/PlayBar.vue";
import ViewerHost from "./components/ViewerHost.vue";
import InspectorPanel from "./components/InspectorPanel.vue";

const themeOverrides: GlobalThemeOverrides = {
  common: { bodyColor: "#101014", cardColor: "#181820" },
};

const viewportRef = ref<HTMLElement | null>(null);
/** 视口内的 2D 贴图预览（选 3D 内容时关闭） */
const texPreview = ref<{ url: string; name: string } | null>(null);

const { viewer, adopt } = useViewer();
const skin = useSkinMode(viewer);
const act = useActMode(viewer);
const fak = useFakMode(viewer);
const fsm = useFsmMode(viewer);
const dress = useDressMode(viewer, skin, act);
const res = useResourceTree();

// ---------------- 视口宿主（viewer.ts 的 HMR 边界）----------------

/** ViewerHost 重建视口（首次挂载 / viewer.ts 热更）后：换实例、重挂拾取、重放当前内容 */
let viewportBooted = false;
function onViewerReady(v: Viewer) {
  const first = !viewportBooted;
  viewportBooted = true;
  adopt(v);
  viewer.value?.setPickHandler((p) => skin.selectPart(p));
  // 首次挂载的内容加载由下方 onMounted 负责，避免双扫描
  if (!first) rerenderViewport();
}

/** 视口重建后重放当前内容（互斥优先级与 previewModelDir 一致：fak > fsm > act > dress > skin 扫描） */
async function rerenderViewport() {
  if (fak.fakModel) {
    const action = fak.currentAction;
    await fak.openFakFile(fak.fakModel.path);
    fak.currentAction = action;
    return;
  }
  if (fsm.fsmModel) {
    await fsm.openFsmFile(fsm.fsmModel.path);
    return;
  }
  if (dress.catalog) {
    await dress.replay();
    return;
  }
  if (act.actMeta) {
    const action = act.currentAction;
    await act.openActFile(act.actMeta.path);
    act.currentAction = action;
    return;
  }
  skin.scan();
}

// ---------------- 预览路由 ----------------

/** RGBA8 数据 → canvas data URL（DDS / TGA 解码结果展示用） */
function texCanvasUrl(d: TexData): string {
  const cv = document.createElement("canvas");
  cv.width = d.width;
  cv.height = d.height;
  const ctx = cv.getContext("2d")!;
  const img = ctx.createImageData(d.width, d.height);
  img.data.set(d.rgba);
  ctx.putImageData(img, 0, 0);
  return cv.toDataURL();
}

async function previewTexture(e: ResEntry) {
  try {
    if (e.kind === "dds") {
      const d = texDataFromDto(await api.loadTexture(e.path));
      texPreview.value = { url: texCanvasUrl(d), name: `${e.name} (${d.width}×${d.height})` };
      return;
    }
    if (e.kind === "tga") {
      const d = decodeTga(b64ToBytes(await api.readFileB64(e.path)));
      texPreview.value = { url: texCanvasUrl(d), name: `${e.name} (${d.width}×${d.height})` };
      return;
    }
    // png / jpg / bmp / gif / webp：原始字节交给 webview 原生解码
    const { url, width, height } = await imageUrlFromB64(await api.readFileB64(e.path), e.kind);
    texPreview.value = { url, name: `${e.name} (${width}×${height})` };
  } catch (err) {
    message.error(`图片读取失败 ${e.name}: ${err}`);
  }
}

/** 网格 / 菜单点击的资源 → 按类型路由预览 */
async function previewEntry(e: ResEntry) {
  if (e.isDir) {
    res.selectDir(e.path);
    return;
  }
  switch (e.kind) {
    case "fskin":
      texPreview.value = null;
      dress.close();
      fak.closeFak();
      fsm.closeFsm();
      await skin.soloPreview(e.path);
      break;
    case "fskel":
      texPreview.value = null;
      fak.closeFak();
      fsm.closeFsm();
      act.openActFile(e.path);
      break;
    case "fak":
      texPreview.value = null;
      await openFakPreview(e.path);
      break;
    case "fsm":
      texPreview.value = null;
      await openFsmPreview(e.path);
      break;
    case "dds":
    case "tga":
    case "png":
    case "jpg":
    case "jpeg":
    case "bmp":
    case "gif":
    case "webp":
      await previewTexture(e);
      break;
    default:
      message.info(`暂不支持预览 ${e.name} (.${e.kind})`);
  }
}

/** FAK / FSM 与皮肤/骨架模式互斥 */
async function openFakPreview(path: string) {
  dress.close();
  skin.clearScene();
  act.closeAct();
  fsm.closeFsm();
  await fak.openFakFile(path);
}

async function openFsmPreview(path: string) {
  dress.close();
  skin.clearScene();
  act.closeAct();
  fak.closeFak();
  await fsm.openFsmFile(path);
}

/** 角色装扮模式：扫描资源根（或指定目录）装配基础模型 + 装备/时装 */
async function openDressMode(root?: string) {
  texPreview.value = null;
  fak.closeFak();
  fsm.closeFsm();
  await dress.open(root ?? res.rootDir);
}

/** 整体模型预览：含 .fak（怪物）或 .fsm（武器）的目录走对应网格，其余走 fskin 装配 */
async function previewModelDir(dir: string) {
  texPreview.value = null;
  try {
    const entries = await api.listResDir(dir);
    const fakFile = entries.find((e) => e.kind === "fak");
    if (fakFile) {
      await openFakPreview(fakFile.path);
      return;
    }
    const fsmFile = entries.find((e) => e.kind === "fsm");
    if (fsmFile) {
      await openFsmPreview(fsmFile.path);
      return;
    }
  } catch {
    // 目录列举失败时退回皮肤扫描（会自行报错）
  }
  fak.closeFak();
  fsm.closeFsm();
  dress.close();
  skin.scan(dir);
}

/** fskin 叠加到当前场景并选中（不重建模型集） */
async function overlayAdd(e: ResEntry) {
  texPreview.value = null;
  fak.closeFak();
  fsm.closeFsm();
  await skin.addToViewer(e.path);
  if (skin.selectedPart !== e.path) skin.selectPart(e.path);
}

onMounted(async () => {
  // 视口实例与拾取接线由 ViewerHost 的 ready 回调负责
  res.init();
  // 开箱默认进入角色装扮（资源根含 AvatarGirl/装备目录时），失败回落原整体预览
  const ok = await dress.open(res.rootDir);
  if (!ok) skin.scan();
});
</script>
