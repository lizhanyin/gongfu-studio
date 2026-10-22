import { computed, reactive, ref } from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { ResEntry } from "../types";
import { message } from "../message";
import { shortName } from "../utils";

/** n-tree 节点（树只放目录，文件由资源网格展示）；带索引签名以兼容 naive-ui TreeOption */
export interface TreeNode {
  [k: string]: unknown;
  key: string; // 完整路径
  label: string;
  isDir: true;
  isModel: boolean;
  /** 角色资源根（AvatarGirl/装备/时装目录的父目录，可进入角色装扮） */
  isCharacter: boolean;
  isLeaf: false;
  children?: TreeNode[];
}

function toNode(e: ResEntry): TreeNode {
  return {
    key: e.path,
    label: e.name,
    isDir: true,
    isModel: e.isModel,
    isCharacter: !!e.isCharacter,
    isLeaf: false,
  };
}

/**
 * 资源树（以 res 为根，懒加载目录）+ 当前目录的资源网格数据。
 * 树/网格只负责浏览；预览与整体模型装配由 App 层路由到 skin/act store。
 */
export function useResourceTree() {
  const rootDir = ref(".\\res");
  const treeData = ref<TreeNode[]>([]);
  const treeLoading = ref(false);
  /** 网格当前展示的目录（null = 未初始化） */
  const currentDir = ref<string | null>(null);
  const gridEntries = ref<ResEntry[]>([]);
  const gridLoading = ref(false);

  async function init() {
    treeLoading.value = true;
    try {
      const r = rootDir.value;
      const label = r.startsWith("cpk:")
        ? `📦 ${shortName(r) || r.slice(4)}（CPK）`
        : shortName(r) || r;
      treeData.value = [
        {
          key: r,
          label,
          isDir: true,
          isModel: false,
          isCharacter: false,
          isLeaf: false,
        },
      ];
      await selectDir(r);
    } catch (e) {
      message.error(`资源根目录加载失败: ${e}`);
    } finally {
      treeLoading.value = false;
    }
  }

  async function changeRoot(dir: string) {
    rootDir.value = dir;
    currentDir.value = null;
    gridEntries.value = [];
    await init();
  }

  async function pickRoot() {
    const d = await open({ directory: true, defaultPath: rootDir.value });
    if (d) await changeRoot(d);
  }

  /** 选择 system.cpk 等资源包：以 `cpk:<包路径>` 为根浏览包内目录 */
  async function pickCpk() {
    const d = await open({
      multiple: false,
      filters: [{ name: "CPK 资源包", extensions: ["cpk"] }],
      defaultPath: rootDir.value.replace(/^cpk:/, ""),
    });
    if (d) await changeRoot(`cpk:${d}`);
  }

  /** 选中目录：网格切换为该目录内容 */
  async function selectDir(path: string) {
    gridLoading.value = true;
    try {
      gridEntries.value = await api.listResDir(path);
      currentDir.value = path;
    } catch (e) {
      message.error(`目录读取失败: ${e}`);
    } finally {
      gridLoading.value = false;
    }
  }

  /** 树节点懒加载子目录（供 n-tree on-load） */
  async function loadChildren(path: string): Promise<TreeNode[]> {
    const list = await api.listResDir(path);
    return list.filter((e) => e.isDir).map(toNode);
  }

  // ---------- 导出 / 导入（树与网格右键共用） ----------

  /** 是否 CPK 包浏览模式（包内目录只读，不支持导入） */
  const isCpk = computed(() => rootDir.value.startsWith("cpk:"));

  /** 导出单个资源文件（CPK 包内文件同样可导出）；模型文件（fskin/fsm/fak）连同关联纹理一起导出 */
  async function exportFile(path: string, name: string) {
    const dest = await save({ defaultPath: name });
    if (!dest) return;
    try {
      if (/\.(fskin|fsm|fak)$/i.test(path)) {
        message.success(await api.exportModel(path, dest));
      } else {
        await api.exportFile(path, dest);
        message.success(`已导出: ${name}`);
      }
    } catch (e) {
      message.error(`导出失败: ${e}`);
    }
  }

  /** 递归导出目录（磁盘目录 / CPK 包内子树）到选择的位置 */
  async function exportDir(path: string) {
    const dest = await open({ directory: true, title: "选择导出位置" });
    if (!dest) return;
    try {
      const out = await api.exportDir(path, dest);
      message.success(out);
    } catch (e) {
      message.error(`导出失败: ${e}`);
    }
  }

  /** 导入磁盘文件到目录（同名覆盖）；导入的是当前浏览目录时刷新网格 */
  async function importToDir(dir: string) {
    const src = await open({ multiple: false, title: "选择要导入的文件" });
    if (!src) return;
    try {
      const dest = await api.importFile(src, dir);
      message.success(`已导入: ${shortName(dest)}`);
      if (currentDir.value === dir) await selectDir(dir);
    } catch (e) {
      message.error(`导入失败: ${e}`);
    }
  }

  return reactive({
    rootDir,
    treeData,
    treeLoading,
    currentDir,
    gridEntries,
    gridLoading,
    init,
    changeRoot,
    pickRoot,
    pickCpk,
    refreshRoot: init,
    selectDir,
    loadChildren,
    isCpk,
    exportFile,
    exportDir,
    importToDir,
  });
}

export type ResourceTreeStore = ReturnType<typeof useResourceTree>;
