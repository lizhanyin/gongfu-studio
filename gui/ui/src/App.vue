<template>
  <n-config-provider :theme="darkTheme" :locale="zhCN" :theme-overrides="themeOverrides">
    <div class="app">
      <header class="app-header">
        <span class="title">Gongfu Studio</span>
        <n-input
          v-model:value="res.rootDir"
          size="small"
          class="root-input"
          placeholder="资源根目录 (res)"
          @keyup.enter="res.changeRoot(res.rootDir)"
        />
        <n-button size="small" @click="res.pickRoot">浏览</n-button>
        <n-button size="small" :loading="res.treeLoading" @click="res.refreshRoot">刷新</n-button>
        <n-divider vertical />
        <SkinToolbar v-if="skin.hasParts" :skin="skin" />
        <ActToolbar v-if="act.actMeta" :act="act" />
      </header>

      <div class="app-main">
        <!-- 左：资源树（res 为根，全高） -->
        <aside class="tree-panel">
          <ResourceTree :tree="res" @preview-model="previewModelDir" />
        </aside>

        <div class="right-col">
          <!-- 右上：预览/操作 -->
          <div class="preview-row">
            <div class="viewport" ref="viewportRef">
              <div class="viewport-overlay">
                <template v-if="fak.fakModel">
                  <div>FAK 顶点动画 · {{ fak.currentActionInfo?.name ?? "" }}</div>
                  <div>顶点: {{ fak.fakModel.vertexCount }} · 帧: {{ fak.fakModel.frames.length }}</div>
                </template>
                <template v-else>
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

            <!-- 检查器（可折叠）：变体 / 动作信息 / 部件检查 -->
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
                  :default-expanded-names="['variants', 'tex', 'act', 'fak', 'part']"
                >
                  <n-collapse-item
                    v-if="skin.modelSet?.groups.length"
                    title="换装变体"
                    name="variants"
                  >
                    <PartGroupList :skin="skin" />
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
                  <n-collapse-item v-if="skin.selectedPart" title="部件检查" name="part">
                    <PartInspector :skin="skin" />
                  </n-collapse-item>
                </n-collapse>
                <div
                  v-if="!skin.hasParts && !act.actMeta && !fak.fakModel"
                  class="inspector-empty"
                >
                  从资源网格选择文件预览
                </div>
              </div>
            </aside>
          </div>

          <!-- 右下：当前目录资源网格 -->
          <div class="grid-panel">
            <ResourceGrid
              :tree="res"
              @preview="previewEntry"
              @open-dir="res.selectDir"
              @preview-model="previewModelDir"
              @overlay-add="overlayAdd"
            />
          </div>
        </div>
      </div>

      <footer class="app-footer">
        <span v-if="skin.modelSet">模型: {{ skin.modelSet.rootDir }}</span>
        <span v-if="act.actMeta">动作: {{ act.actMeta.fileName }}</span>
        <span v-if="fak.fakModel">FAK: {{ shortName(fak.fakModel.path) }}</span>
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
import { useResourceTree } from "./composables/useResourceTree";
import { texDataFromDto } from "./viewer";
import { api } from "./api";
import { message } from "./message";
import { shortName } from "./utils";
import type { ResEntry } from "./types";
import SkinToolbar from "./components/SkinToolbar.vue";
import ActToolbar from "./components/ActToolbar.vue";
import ResourceTree from "./components/ResourceTree.vue";
import ResourceGrid from "./components/ResourceGrid.vue";
import PartGroupList from "./components/PartGroupList.vue";
import TexSlotPanel from "./components/TexSlotPanel.vue";
import PlayBar from "./components/PlayBar.vue";
import ActFileInfo from "./components/ActFileInfo.vue";
import FakFileInfo from "./components/FakFileInfo.vue";
import PartInspector from "./components/PartInspector.vue";

const themeOverrides: GlobalThemeOverrides = {
  common: { bodyColor: "#101014", cardColor: "#181820" },
};

const viewportRef = ref<HTMLElement | null>(null);
const inspectorCollapsed = ref(false);
/** 视口内的 2D 贴图预览（选 3D 内容时关闭） */
const texPreview = ref<{ url: string; name: string } | null>(null);

const viewer = useViewer(viewportRef);
const skin = useSkinMode(viewer);
const act = useActMode(viewer);
const fak = useFakMode(viewer);
const res = useResourceTree();

// ---------------- 预览路由 ----------------

async function previewTexture(e: ResEntry) {
  try {
    const dto = await api.loadTexture(e.path);
    const d = texDataFromDto(dto);
    const cv = document.createElement("canvas");
    cv.width = d.width;
    cv.height = d.height;
    const ctx = cv.getContext("2d")!;
    const img = ctx.createImageData(d.width, d.height);
    img.data.set(d.rgba);
    ctx.putImageData(img, 0, 0);
    texPreview.value = { url: cv.toDataURL(), name: `${e.name} (${d.width}×${d.height})` };
  } catch (err) {
    message.error(`贴图读取失败 ${e.name}: ${err}`);
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
      fak.closeFak();
      await skin.soloPreview(e.path);
      break;
    case "fskel":
      texPreview.value = null;
      fak.closeFak();
      act.openActFile(e.path);
      break;
    case "fak":
      texPreview.value = null;
      await openFakPreview(e.path);
      break;
    case "dds":
    case "tga":
    case "png":
    case "jpg":
    case "bmp":
      await previewTexture(e);
      break;
    default:
      message.info(`暂不支持预览 ${e.name} (.${e.kind})`);
  }
}

/** FAK 与皮肤/骨架模式互斥 */
async function openFakPreview(path: string) {
  skin.clearScene();
  act.closeAct();
  await fak.openFakFile(path);
}

/** 整体模型预览：含 .fak 的目录（怪物）走顶点动画，其余走 fskin 装配 */
async function previewModelDir(dir: string) {
  texPreview.value = null;
  try {
    const entries = await api.listResDir(dir);
    const fakFile = entries.find((e) => e.kind === "fak");
    if (fakFile) {
      await openFakPreview(fakFile.path);
      return;
    }
  } catch {
    // 目录列举失败时退回皮肤扫描（会自行报错）
  }
  fak.closeFak();
  skin.scan(dir);
}

/** fskin 叠加到当前场景并选中（不重建模型集） */
async function overlayAdd(e: ResEntry) {
  texPreview.value = null;
  fak.closeFak();
  await skin.addToViewer(e.path);
  if (skin.selectedPart !== e.path) skin.selectPart(e.path);
}

onMounted(() => {
  viewer.value?.setPickHandler((p) => skin.selectPart(p));
  res.init();
  skin.scan(); // 开箱默认整体预览 rootDir（res/system/AvatarGirl）
});
</script>
