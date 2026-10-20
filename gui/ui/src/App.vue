<template>
  <n-config-provider :theme="darkTheme" :locale="zhCN" :theme-overrides="themeOverrides">
    <div class="app">
      <header class="app-header">
        <span class="title">Gongfu Studio</span>
        <n-button-group size="small">
          <n-button :type="topMode === 'skin' ? 'primary' : 'default'" @click="switchMode('skin')">皮肤</n-button>
          <n-button :type="topMode === 'act' ? 'primary' : 'default'" @click="switchMode('act')">动作</n-button>
        </n-button-group>
        <SkinToolbar v-if="topMode === 'skin'" :skin="skin" />
        <ActToolbar v-else :act="act" />
      </header>

      <div class="app-main">
        <aside class="left-panel">
          <PartGroupList v-if="topMode === 'skin'" :skin="skin" />
          <ActFileList v-else :act="act" :root-dir="skin.rootDir" />
        </aside>

        <div class="viewport" ref="viewportRef">
          <div class="viewport-overlay">
            <div>部件: {{ skin.visibleCount }} / 组: {{ skin.modelSet?.groups.length ?? 0 }}</div>
            <div>顶点: {{ skin.statVerts }} · 三角形: {{ skin.statTris }}</div>
          </div>
          <div class="viewport-hint">左键旋转 · 右键平移 · 滚轮缩放 · 点击网格选中部件</div>

          <PlayBar v-if="topMode === 'act' && act.actMeta" :act="act" />
        </div>

        <aside class="right-panel">
          <ActFileInfo v-if="topMode === 'act'" :act="act" />
          <PartInspector v-else :skin="skin" />
        </aside>
      </div>

      <footer class="app-footer">
        <span>{{ skin.modelSet ? `目录: ${skin.modelSet.rootDir}` : "" }}</span>
        <span>可见部件: {{ skin.visiblePaths.map(shortName).join(" + ") || "无" }}</span>
      </footer>
    </div>
  </n-config-provider>
</template>

<script setup lang="ts">
import { onMounted, ref, toRef } from "vue";
import { darkTheme, zhCN, type GlobalThemeOverrides } from "naive-ui";
import { useViewer } from "./composables/useViewer";
import { useSkinMode } from "./composables/useSkinMode";
import { useActMode } from "./composables/useActMode";
import { shortName } from "./utils";
import SkinToolbar from "./components/SkinToolbar.vue";
import ActToolbar from "./components/ActToolbar.vue";
import PartGroupList from "./components/PartGroupList.vue";
import ActFileList from "./components/ActFileList.vue";
import PlayBar from "./components/PlayBar.vue";
import ActFileInfo from "./components/ActFileInfo.vue";
import PartInspector from "./components/PartInspector.vue";

const themeOverrides: GlobalThemeOverrides = {
  common: { bodyColor: "#101014", cardColor: "#181820" },
};

const topMode = ref<"skin" | "act">("skin");
const viewportRef = ref<HTMLElement | null>(null);

const viewer = useViewer(viewportRef);
const skin = useSkinMode(viewer);
// 动作目录 = 模型目录 + \act，因此共享皮肤模式的 rootDir
const act = useActMode(viewer, toRef(skin, "rootDir"));

function switchMode(m: "skin" | "act") {
  if (topMode.value === m) return;
  topMode.value = m;
  if (m === "act") {
    act.setSkinOverlay(act.showSkinOverlay);
    if (!act.actFiles.length) act.scanAct();
  } else {
    act.playing = false;
    viewer.value?.clearFskel();
    viewer.value?.setPartsVisible(true);
  }
}

onMounted(() => {
  viewer.value?.setPickHandler((p) => skin.selectPart(p));
  skin.scan();
});
</script>
