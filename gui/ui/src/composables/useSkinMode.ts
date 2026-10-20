import { computed, reactive, ref, watch, type ShallowRef } from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { FskinModel, ModelSet, PartGroup, PartInfo } from "../types";
import { texDataFromDto, type TexData, type Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/**
 * 皮肤模式：目录扫描、部件组装、材质 / 顶点编辑、纹理与保存导出。
 * 返回 reactive store（嵌套 ref 自动解包），供 App 与各子组件直接使用。
 */
export function useSkinMode(viewer: ShallowRef<Viewer | null>) {
  const rootDir = ref(".\\res\\system\\AvatarGirl");
  const modelSet = ref<ModelSet | null>(null);
  const scanning = ref(false);
  const renderMode = ref<"shaded" | "bones" | "uv">("shaded");
  const wireframe = ref(false);
  const selectedPart = ref<string | null>(null);

  const loaded = reactive(new Map<string, FskinModel>());
  const texCache = reactive(new Map<string, TexData>());
  const dirty = reactive(new Set<string>());
  const groupSel = reactive(new Map<string, string>()); // signature -> path
  const groupVis = reactive(new Map<string, boolean>()); // signature -> bool

  const selectedModel = computed(() =>
    selectedPart.value ? loaded.get(selectedPart.value) ?? null : null
  );

  function partInfoOf(g: PartGroup): PartInfo | null {
    const p = groupSel.get(g.signature);
    return g.parts.find((x) => x.path === p) ?? null;
  }

  function afterEdit() {
    if (selectedPart.value) dirty.add(selectedPart.value);
  }

  const isDirty = (p?: string | null) => !!p && dirty.has(p);

  // ---------- 纹理 ----------
  function resolveTexture(texturePath: string): string | null {
    if (!modelSet.value) return null;
    const stem = (texturePath.split(/[\\/]/).pop() ?? "")
      .replace(/\.[^.]*$/, "")
      .toLowerCase();
    if (!stem) return null;
    return modelSet.value.textures.find((t) => t.stem === stem)?.path ?? null;
  }

  async function ensureTexture(file: string): Promise<TexData | null> {
    if (texCache.has(file)) return texCache.get(file)!;
    try {
      const dto = await api.loadTexture(file);
      const d = texDataFromDto(dto);
      texCache.set(file, d);
      return d;
    } catch (e) {
      message.error(`纹理解码失败 ${shortName(file)}: ${e}`);
      return null;
    }
  }

  const texPreviewUrl = computed(() => {
    const m = selectedModel.value;
    if (!m) return null;
    const file = resolveTexture(m.material.texturePath);
    if (!file) return null;
    const d = texCache.get(file);
    if (!d) return null;
    const cv = document.createElement("canvas");
    cv.width = d.width;
    cv.height = d.height;
    const ctx = cv.getContext("2d")!;
    const img = ctx.createImageData(d.width, d.height);
    img.data.set(d.rgba);
    ctx.putImageData(img, 0, 0);
    return cv.toDataURL();
  });

  const texInfoText = computed(() => {
    const m = selectedModel.value;
    if (!m) return "";
    const file = resolveTexture(m.material.texturePath);
    const d = file ? texCache.get(file) : null;
    return d ? `${d.width}×${d.height} · ${shortName(file!)}` : file ? `${shortName(file)} (未加载)` : "目录中未找到匹配贴图";
  });

  async function applyTexture() {
    const m = selectedModel.value;
    const v = viewer.value;
    if (!m || !v) return;
    afterEdit();
    const file = resolveTexture(m.material.texturePath);
    if (file) {
      const d = await ensureTexture(file);
      if (d) v.refreshTexture(m.path, d, file);
    } else {
      v.clearTexture(m.path);
    }
    v.applyMaterial(m.path);
  }

  // ---------- 部件加载 ----------
  async function ensureLoaded(path: string): Promise<FskinModel | null> {
    if (loaded.has(path)) return loaded.get(path)!;
    try {
      const m = await api.loadFskin(path);
      loaded.set(path, m);
      return m;
    } catch (e) {
      message.error(`加载失败: ${e}`);
      return null;
    }
  }

  async function addToViewer(path: string) {
    const v = viewer.value;
    if (!v || v.has(path)) return;
    const m = await ensureLoaded(path);
    if (!m) return;
    const file = m.material.texturePath ? resolveTexture(m.material.texturePath) : null;
    const tex = file ? await ensureTexture(file) : null;
    v.addPart(path, m, tex, file);
  }

  function toggleGroup(g: PartGroup, on: boolean) {
    groupVis.set(g.signature, on);
    const p = groupSel.get(g.signature);
    if (!p) return;
    if (on) addToViewer(p);
    else viewer.value?.removePart(p);
  }

  async function changeVariant(g: PartGroup, newPath: string) {
    const old = groupSel.get(g.signature);
    if (old) viewer.value?.removePart(old);
    groupSel.set(g.signature, newPath);
    if (groupVis.get(g.signature)) await addToViewer(newPath);
    if (selectedPart.value === old) selectPart(newPath);
  }

  function selectPart(path: string | null) {
    // 再次点击已选中的部件 → 取消选中，整个模型恢复完整高亮
    const next = path !== null && selectedPart.value === path ? null : path;
    selectedPart.value = next;
    viewer.value?.highlight(next);
    if (next && !loaded.has(next)) ensureLoaded(next);
  }

  // ---------- 编辑 ----------
  function setColor(arr: number[], v: string) {
    const nums = (v.match(/[\d.]+g?/g) ?? []).map(Number);
    if (nums.length >= 3) {
      arr[0] = nums[0] / 255;
      arr[1] = nums[1] / 255;
      arr[2] = nums[2] / 255;
      arr[3] = nums.length >= 4 ? nums[3] : 1;
    }
    afterEdit();
    viewer.value?.applyMaterial(selectedPart.value!);
  }

  /** 顶点表格内编辑后同步到 viewer */
  function touchVertex(row: unknown) {
    const m = selectedModel.value;
    if (!m) return;
    const i = m.vertices.indexOf(row as never);
    if (i >= 0) viewer.value?.updateVertex(m.path, i);
    afterEdit();
  }

  // ---------- 扫描 ----------
  async function scan() {
    scanning.value = true;
    try {
      const set = await api.scanModelSet(rootDir.value);
      modelSet.value = set;
      // 清空场景
      for (const p of [...loaded.keys()]) viewer.value?.removePart(p);
      loaded.clear();
      dirty.clear();
      groupSel.clear();
      groupVis.clear();
      selectedPart.value = null;
      viewer.value?.highlight(null);

      for (const g of set.groups) {
        const def = g.parts.find((p) => p.inRoot) ?? g.parts[0];
        groupSel.set(g.signature, def.path);
        groupVis.set(g.signature, !!g.parts.find((p) => p.inRoot));
      }
      // 依次加载可见部件（根目录 5 件 = 完整角色）
      for (const g of set.groups) {
        if (groupVis.get(g.signature)) await addToViewer(groupSel.get(g.signature)!);
      }
      viewer.value?.frameAll();
      message.success(
        `扫描完成: ${set.groups.length} 组 · 已装配 ${set.groups.filter((g) => groupVis.get(g.signature)).length} 个部件`
      );
    } catch (e) {
      message.error(`扫描失败: ${e}`);
    } finally {
      scanning.value = false;
    }
  }

  async function pickDir() {
    const d = await open({ directory: true, defaultPath: rootDir.value });
    if (d) {
      rootDir.value = d;
      await scan();
    }
  }

  // ---------- 统计 ----------
  const visiblePaths = computed(() => {
    const out: string[] = [];
    if (!modelSet.value) return out;
    for (const g of modelSet.value.groups) {
      if (groupVis.get(g.signature)) {
        const p = groupSel.get(g.signature);
        if (p) out.push(p);
      }
    }
    return out;
  });
  const visibleCount = computed(() => visiblePaths.value.length);
  const statVerts = computed(() => {
    let n = 0;
    for (const p of visiblePaths.value) n += loaded.get(p)?.vertices.length ?? 0;
    return n;
  });
  const statTris = computed(() => {
    let n = 0;
    for (const p of visiblePaths.value) n += (loaded.get(p)?.triangles.length ?? 0) / 3;
    return n;
  });

  // ---------- 保存 ----------
  async function saveModelTo(path: string, m: FskinModel) {
    try {
      await api.saveFskin(path, m);
      dirty.delete(m.path);
      message.success(`已保存: ${shortName(path)}`);
    } catch (e) {
      message.error(`保存失败: ${e}`);
    }
  }

  function saveSelected() {
    const m = selectedModel.value;
    if (m) saveModelTo(m.path, m);
  }

  async function saveAs() {
    const m = selectedModel.value;
    if (!m) return;
    const p = await save({
      defaultPath: shortName(m.path),
      filters: [{ name: "FSKIN", extensions: ["fskin"] }],
    });
    if (p) saveModelTo(p, m);
  }

  async function recomputeBbox() {
    const m = selectedModel.value;
    if (!m) return;
    try {
      const [mx, mn] = await api.computeBbox(m);
      m.bboxMax = mx;
      m.bboxMin = mn;
      afterEdit();
      message.success("包围盒已按顶点重算");
    } catch (e) {
      message.error(`${e}`);
    }
  }

  // ---------- 导出 ----------
  async function exportObj() {
    if (!visiblePaths.value.length) return;
    const p = await save({
      defaultPath: "avatar_girl.obj",
      filters: [{ name: "OBJ 模型", extensions: ["obj"] }],
    });
    if (!p) return;
    const dir = p.replace(/[\\/][^\\/]*$/, "");
    const base = (p.split(/[\\/]/).pop() ?? "model").replace(/\.obj$/i, "");
    try {
      const out = await api.exportObj(visiblePaths.value, dir, base);
      message.success(`已导出: ${out}（含 mtl 与贴图）`);
    } catch (e) {
      message.error(`导出失败: ${e}`);
    }
  }

  function frameAll() {
    viewer.value?.frameAll();
  }

  watch(renderMode, (m) => viewer.value?.setMode(m));
  watch(wireframe, (w) => viewer.value?.setWireframe(w));

  return reactive({
    rootDir,
    modelSet,
    scanning,
    renderMode,
    wireframe,
    selectedPart,
    loaded,
    texCache,
    dirty,
    groupSel,
    groupVis,
    selectedModel,
    visiblePaths,
    visibleCount,
    statVerts,
    statTris,
    texPreviewUrl,
    texInfoText,
    partInfoOf,
    isDirty,
    afterEdit,
    resolveTexture,
    ensureTexture,
    applyTexture,
    toggleGroup,
    changeVariant,
    selectPart,
    setColor,
    touchVertex,
    scan,
    pickDir,
    saveSelected,
    saveAs,
    recomputeBbox,
    exportObj,
    frameAll,
  });
}

export type SkinStore = ReturnType<typeof useSkinMode>;
