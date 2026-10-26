<template>
  <n-menu
    class="menubar"
    mode="horizontal"
    :value="null"
    :options="options"
    @update:value="onCommand"
  />
</template>

<script setup lang="ts">
import { computed, h } from "vue";
import type { MenuOption } from "naive-ui";
import type { SkinStore } from "../composables/useSkinMode";
import type { ActStore } from "../composables/useActMode";
import type { ResourceTreeStore } from "../composables/useResourceTree";
import type { DressStore } from "../composables/useDressMode";

/**
 * 下拉条目左侧的勾选槽：所有条目（含普通命令项）都占同等宽度的图标位，
 * ✓ 仅在勾选态显示，各条目文字左缘对齐。
 */
const tick = (on: boolean) => () => h("span", { class: "menu-tick" }, on ? "✓" : "");

const props = defineProps<{
  skin: SkinStore;
  act: ActStore;
  res: ResourceTreeStore;
  dress?: DressStore;
}>();
const emit = defineEmits<{
  (e: "dress"): void;
  (e: "export-full-glb"): void;
}>();

/** 按作用分组的命令菜单：文件 / 视图（皮肤渲染）/ 动作 / 导出 */
const options = computed<MenuOption[]>(() => {
  const skinOff = !props.skin.hasParts;
  const actOff = !props.act.actMeta;
  return [
    {
      label: "文件",
      key: "file",
      children: [
        { label: "打开资源目录…", key: "open-root", icon: tick(false) },
        { label: "打开 CPK 资源包…", key: "open-cpk", icon: tick(false) },
        { type: "divider", key: "file-d0" },
        { label: "角色装扮…", key: "dress", icon: tick(false) },
        { type: "divider", key: "file-d1" },
        {
          label: props.res.treeLoading ? "刷新中…" : "刷新",
          key: "refresh",
          disabled: props.res.treeLoading,
          icon: tick(false),
        },
        { type: "divider", key: "file-d2" },
        {
          label: "保存当前部件",
          key: "save-part",
          disabled: !props.skin.selectedModel,
          icon: tick(false),
        },
      ],
    },
    {
      label: "视图",
      key: "view",
      children: [
        { label: "着色", key: "rm-shaded", disabled: skinOff, icon: tick(props.skin.renderMode === "shaded") },
        { label: "骨骼", key: "rm-bones", disabled: skinOff, icon: tick(props.skin.renderMode === "bones") },
        { label: "UV", key: "rm-uv", disabled: skinOff, icon: tick(props.skin.renderMode === "uv") },
        { type: "divider", key: "view-d1" },
        { label: "线框", key: "wireframe", disabled: skinOff, icon: tick(props.skin.wireframe) },
        { label: "选中高亮", key: "pick-highlight", disabled: skinOff, icon: tick(props.skin.pickHighlight) },
        { label: "定位模型", key: "frame-all", disabled: skinOff, icon: tick(false) },
      ],
    },
    {
      label: "动作",
      key: "act",
      children: [
        { label: "叠加皮肤网格", key: "act-overlay", disabled: actOff, icon: tick(props.act.showSkinOverlay) },
        { label: "显示骨架", key: "act-skeleton", disabled: actOff, icon: tick(props.act.showSkeleton) },
        { label: "定位骨架", key: "frame-skel", disabled: actOff, icon: tick(false) },
        { type: "divider", key: "act-d1" },
        { label: "关闭动作", key: "close-act", disabled: actOff, icon: tick(false) },
      ],
    },
    {
      label: "导出",
      key: "export",
      children: [
        {
          label: "导出整体 GLB（装配模型）…",
          key: "exp-full-glb",
          disabled: !props.skin.visiblePaths.length && !props.dress?.catalog,
          icon: tick(false),
        },
        {
          label: "导出 OBJ…",
          key: "exp-obj",
          disabled: !props.skin.visiblePaths.length,
          icon: tick(false),
        },
        { type: "divider", key: "export-d1" },
        { label: "导出 BVH（全部帧）…", key: "exp-bvh-all", disabled: actOff, icon: tick(false) },
        { label: "导出 BVH（当前动作）…", key: "exp-bvh-cur", disabled: actOff, icon: tick(false) },
        { label: "导出 GLB…", key: "exp-glb", disabled: actOff, icon: tick(false) },
      ],
    },
  ];
});

function onCommand(key: string | number) {
  switch (key) {
    case "open-root":
      void props.res.pickRoot();
      break;
    case "open-cpk":
      void props.res.pickCpk();
      break;
    case "dress":
      emit("dress");
      break;
    case "refresh":
      void props.res.refreshRoot();
      break;
    case "save-part":
      props.skin.saveSelected();
      break;
    case "rm-shaded":
      props.skin.renderMode = "shaded";
      break;
    case "rm-bones":
      props.skin.renderMode = "bones";
      break;
    case "rm-uv":
      props.skin.renderMode = "uv";
      break;
    case "wireframe":
      props.skin.wireframe = !props.skin.wireframe;
      break;
    case "pick-highlight":
      props.skin.pickHighlight = !props.skin.pickHighlight;
      break;
    case "frame-all":
      props.skin.frameAll();
      break;
    case "act-overlay":
      props.act.setSkinOverlay(!props.act.showSkinOverlay);
      break;
    case "act-skeleton":
      props.act.setSkeletonVisible(!props.act.showSkeleton);
      break;
    case "frame-skel":
      props.act.frameSkeleton();
      break;
    case "close-act":
      props.act.closeAct();
      break;
    case "exp-full-glb":
      emit("export-full-glb");
      break;
    case "exp-obj":
      void props.skin.exportObj();
      break;
    case "exp-bvh-all":
      void props.act.exportBvh(false);
      break;
    case "exp-bvh-cur":
      void props.act.exportBvh(true);
      break;
    case "exp-glb":
      void props.act.exportGlb();
      break;
  }
}
</script>
