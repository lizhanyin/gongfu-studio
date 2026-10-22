import { computed, reactive, ref, watch, type ShallowRef } from "vue";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import type { FskinModel, ModelSet, PartGroup, PartInfo, TexSlot, TexVariant } from "../types";
import { trianglesOf } from "../types";
import { texDataFromDto, b64ToBytes, decodeTga } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/** 衣服槽（YiFu 换装目录）：整套切换时联动 */
const CLOTHING_SLOTS = ["upper", "lower", "boot"];

/** 部件款式号：根目录成员 = 0（默认），girl_Upper_2 → 2 */
function styleNum(p: PartInfo): number {
  if (p.inRoot) return 0;
  return Number((p.fileName.match(/_(\d+)\.fskin$/i) ?? [])[1] ?? 0);
}

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
  /** 选中部件时是否压暗其余部件（默认关） */
  const pickHighlight = ref(false);
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
      // DDS 走后端解码；TGA 前端解码（与装扮模式同一套）；失败不缓存，便于修复后重试
      let d: TexData | null = null;
      if (/\.dds$/i.test(file)) {
        d = texDataFromDto(await api.loadTexture(file));
      } else if (/\.tga$/i.test(file)) {
        d = decodeTga(b64ToBytes(await api.readFileB64(file)));
      } else {
        message.warning(`暂不支持解码该格式贴图: ${shortName(file)}`);
        return null;
      }
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
    const file = resolveTexture(m.materials[0]?.texturePath ?? "");
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
    const file = resolveTexture(m.materials[0]?.texturePath ?? "");
    const d = file ? texCache.get(file) : null;
    return d ? `${d.width}×${d.height} · ${shortName(file!)}` : file ? `${shortName(file)} (未加载)` : "目录中未找到匹配贴图";
  });

  /** 按文件名匹配目录内贴图，应用到部件的全部材质槽（护甲类多子网格各有贴图）；
   *  可反复调用，每次按当前纹理路径重新解析 */
  async function applyTexture() {
    const m = selectedModel.value;
    const v = viewer.value;
    if (!m || !v) return;
    afterEdit();
    let applied = 0;
    for (let i = 0; i < m.materials.length; i++) {
      const md = m.materials[i];
      const file = md.texturePath ? resolveTexture(md.texturePath) : null;
      const d = file ? await ensureTexture(file) : null;
      if (d && file) {
        v.refreshTexture(m.path, d, file, i);
        applied++;
      } else {
        v.clearTexture(m.path, i);
      }
    }
    v.applyMaterial(m.path);
    if (!applied) message.error("目录中未找到匹配贴图");
  }

  // ---------- 纹理切换槽（脸部纹理 / 饰品）与部件配色 ----------

  /** 每个纹理槽当前选中的贴图路径（overlay 槽 null = 关闭） */
  const texSlotSel = reactive(new Map<string, string | null>());

  /** 磁盘贴图路径 → 游戏风格材质路径（保存后与原始数据同构） */
  function gameTexPath(file: string): string {
    const root = rootDir.value.replace(/[\\/]+$/, "");
    const rel = file.toLowerCase().startsWith(root.toLowerCase())
      ? file.slice(root.length).replace(/^[\\/]+/, "")
      : file;
    const base = root.split(/[\\/]+/).filter(Boolean).slice(-2).join("\\");
    return `${base}\\${rel.replace(/\.dds$/i, ".tga")}`;
  }

  /** 应用纹理槽选项：替换类换材质贴图，叠层类挂透明饰品网格 */
  async function applyTexVariant(slot: TexSlot, path: string | null) {
    const v = viewer.value;
    if (!v) return;
    texSlotSel.set(slot.key, path);
    if (!path) {
      if (slot.overlay) v.setPartOverlay(slot.part, null, null);
      return;
    }
    const tex = await ensureTexture(path);
    if (!tex) return;
    if (slot.overlay) {
      v.setPartOverlay(slot.part, tex, path);
      return;
    }
    v.refreshTexture(slot.part, tex, path);
    const m = loaded.get(slot.part);
    const md = m?.materials[0];
    if (m && md) {
      md.texturePath = gameTexPath(path);
      afterEdit();
      v.applyMaterial(slot.part);
    }
  }

  /** 选中部件的配色族（如 ff01_00/01/02 同前缀贴图互为配色） */
  const partTexVariants = computed<TexVariant[]>(() => {
    const m = selectedModel.value;
    const set = modelSet.value;
    if (!m || !set) return [];
    const file = resolveTexture(m.materials[0]?.texturePath ?? "");
    if (!file) return [];
    const stem = (file.split(/[\\/]/).pop() ?? "").replace(/\.[^.]*$/, "").toLowerCase();
    const i = stem.lastIndexOf("_");
    if (i <= 0) return [];
    const fam = stem.slice(0, i);
    return set.textures
      .filter((t) => t.stem.startsWith(fam + "_"))
      .map((t) => ({ label: t.stem.slice(fam.length + 1), path: t.path }))
      .sort((a, b) => a.label.localeCompare(b.label));
  });

  /** 当前部件贴图在配色族中的后缀（ff01_02 → "02"），无族返回 null */
  const partTexCurrent = computed<string | null>(() => {
    const m = selectedModel.value;
    if (!m || !partTexVariants.value.length) return null;
    const file = resolveTexture(m.materials[0]?.texturePath ?? "");
    if (!file) return null;
    const stem = (file.split(/[\\/]/).pop() ?? "").replace(/\.[^.]*$/, "").toLowerCase();
    const i = stem.lastIndexOf("_");
    return i > 0 ? stem.slice(i + 1) : null;
  });

  /** 应用部件配色（发型颜色等）：替换主材质贴图 */
  async function applyPartTexture(path: string) {
    const m = selectedModel.value;
    const v = viewer.value;
    if (!m || !v) return;
    const tex = await ensureTexture(path);
    if (!tex) return;
    v.refreshTexture(m.path, tex, path);
    const md = m.materials[0];
    if (md) md.texturePath = gameTexPath(path);
    afterEdit();
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
    // 每材质槽各自解析贴图（护甲类多子网格各有贴图；特效网格无材质）
    const texes: (TexData | null)[] = [];
    const files: (string | null)[] = [];
    for (const md of m.materials) {
      const file = md.texturePath ? resolveTexture(md.texturePath) : null;
      files.push(file);
      texes.push(file ? await ensureTexture(file) : null);
    }
    v.addPart(path, m, texes, files);
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

  // ---------- 替换槽（头发等跨几何组互斥） ----------

  /** 槽的全部可选部件：跨组收集，同几何+同贴图去重（根目录成员优先） */
  function slotOptions(slot: string): PartInfo[] {
    const set = modelSet.value;
    if (!set) return [];
    const seen = new Map<string, PartInfo>();
    for (const g of set.groups.filter((g) => g.slot === slot)) {
      for (const p of [...g.parts].sort((a, b) => Number(b.inRoot) - Number(a.inRoot))) {
        const key = `${g.signature}|${p.textureFile ?? p.texturePath}`;
        if (!seen.has(key)) seen.set(key, p);
      }
    }
    return [...seen.values()];
  }

  /** 槽当前显示的部件路径（无可见组时取第一组的当前选择） */
  function slotCurrent(slot: string): string | null {
    const set = modelSet.value;
    if (!set) return null;
    const groups = set.groups.filter((g) => g.slot === slot);
    for (const g of groups) {
      if (groupVis.get(g.signature)) return groupSel.get(g.signature) ?? null;
    }
    return groups.length ? (groupSel.get(groups[0].signature) ?? null) : null;
  }

  const slotVisible = (slot: string) => {
    const set = modelSet.value;
    if (!set) return false;
    return set.groups.some((g) => g.slot === slot && groupVis.get(g.signature));
  };

  /** 选择槽选项：目标组显示该部件，同槽其余组隐藏（互斥替换） */
  async function selectSlotVariant(slot: string, path: string) {
    const set = modelSet.value;
    if (!set) return;
    const target = set.groups
      .filter((g) => g.slot === slot)
      .find((g) => g.parts.some((p) => p.path === path));
    if (!target) return;
    for (const g of set.groups.filter((g) => g.slot === slot)) {
      if (g.signature === target.signature) continue;
      if (groupVis.get(g.signature)) {
        const cur = groupSel.get(g.signature);
        if (cur) viewer.value?.removePart(cur);
        groupVis.set(g.signature, false);
      }
    }
    const old = groupSel.get(target.signature);
    if (old && old !== path) viewer.value?.removePart(old);
    groupSel.set(target.signature, path);
    groupVis.set(target.signature, true);
    await addToViewer(path);
    if (selectedPart.value && selectedPart.value === old) selectPart(path);
  }

  /** 槽整体开关 */
  async function toggleSlot(slot: string, on: boolean) {
    const set = modelSet.value;
    if (!set) return;
    const groups = set.groups.filter((g) => g.slot === slot);
    if (on) {
      const cur = slotCurrent(slot);
      if (cur) await selectSlotVariant(slot, cur);
    } else {
      for (const g of groups) {
        if (groupVis.get(g.signature)) {
          const cur = groupSel.get(g.signature);
          if (cur) viewer.value?.removePart(cur);
          groupVis.set(g.signature, false);
        }
      }
    }
  }

  // ---------- 整套换装（上衣/下装/鞋子同号款式一键切换） ----------

  /** 三槽共有的款式号（可整套切换） */
  const outfitStyles = computed<number[]>(() => {
    const common = new Set(
      CLOTHING_SLOTS.flatMap((s) => slotOptions(s)).map(styleNum).filter((n) => n > 0)
    );
    for (const s of CLOTHING_SLOTS) {
      const nums = new Set(slotOptions(s).map(styleNum));
      for (const n of [...common]) if (!nums.has(n)) common.delete(n);
    }
    return [...common].sort((a, b) => a - b);
  });

  /** 当前整套款式号：0 = 全默认，n = 三槽同款，-1 = 混合搭配 */
  const outfitCurrent = computed<number>(() => {
    const nums = CLOTHING_SLOTS.map((s) => {
      const cur = slotCurrent(s);
      const p = cur ? slotOptions(s).find((o) => o.path === cur) : null;
      return p ? styleNum(p) : -1;
    });
    return nums.every((n) => n >= 0 && n === nums[0]) ? nums[0] : -1;
  });

  /** 整套切换：null = 全部恢复默认，n = 各槽选款式 n */
  async function selectOutfit(n: number | null) {
    for (const s of CLOTHING_SLOTS) {
      const target = slotOptions(s).find((p) => styleNum(p) === (n ?? 0));
      if (target) await selectSlotVariant(s, target.path);
    }
  }

  function selectPart(path: string | null) {
    // 再次点击已选中的部件 → 取消选中，整个模型恢复完整高亮
    const next = path !== null && selectedPart.value === path ? null : path;
    selectedPart.value = next;
    viewer.value?.highlight(pickHighlight.value ? next : null);
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
  /** 扫描目录并装配部件；onlyPath 给定时只装配该文件所属组（单文件预览） */
  async function scan(dir?: string, onlyPath?: string) {
    scanning.value = true;
    try {
      if (dir) rootDir.value = dir;
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
      // 纹理槽回默认：替换类选首项（原始），叠层类关闭
      texSlotSel.clear();
      for (const s of set.texSlots) texSlotSel.set(s.key, s.overlay ? null : (s.variants[0]?.path ?? null));

      for (const g of set.groups) {
        const def = g.parts.find((p) => p.inRoot) ?? g.parts[0];
        groupSel.set(g.signature, def.path);
        if (onlyPath) {
          // 只让目标文件所属组可见，其余组保留默认选中便于后续在变体面板打开
          groupVis.set(g.signature, g.parts.some((p) => p.path === onlyPath));
          if (g.parts.some((p) => p.path === onlyPath)) groupSel.set(g.signature, onlyPath);
        } else {
          groupVis.set(g.signature, !!g.parts.find((p) => p.inRoot));
        }
      }
      // 依次加载可见部件（根目录 5 件 = 完整角色）
      for (const g of set.groups) {
        if (groupVis.get(g.signature)) await addToViewer(groupSel.get(g.signature)!);
      }
      viewer.value?.frameAll();
      if (onlyPath) {
        selectedPart.value = onlyPath;
        viewer.value?.highlight(pickHighlight.value ? onlyPath : null);
        message.success(`已加载部件: ${shortName(onlyPath)}`);
      } else {
        message.success(
          `扫描完成: ${set.groups.length} 组 · 已装配 ${set.groups.filter((g) => groupVis.get(g.signature)).length} 个部件`
        );
      }
    } catch (e) {
      message.error(`扫描失败: ${e}`);
    } finally {
      scanning.value = false;
    }
  }

  /** 卸下全部部件并清空模型集（切换到 FAK 等互斥模式时调用） */
  function clearScene() {
    for (const p of [...loaded.keys()]) viewer.value?.removePart(p);
    loaded.clear();
    dirty.clear();
    groupSel.clear();
    groupVis.clear();
    selectedPart.value = null;
    modelSet.value = null;
    viewer.value?.highlight(null);
  }

  /** 只卸下场景中的全部部件（保留模型集与面板状态）：
   *  换骨架等部件必须与骨架严格配套的时序用，之后 scan 重新装配 */
  function unloadParts() {
    for (const p of [...loaded.keys()]) viewer.value?.removePart(p);
    loaded.clear();
    selectedPart.value = null;
    viewer.value?.highlight(null);
  }

  /** 单文件预览：扫描父目录（贴图/变体信息照常可用）但只装配该部件 */
  async function soloPreview(path: string) {
    const dir = path.replace(/[\\/][^\\/]*$/, "");
    await scan(dir, path);
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
  /** 是否有皮肤部件可供操作（情境化显示皮肤工具栏/检查器） */
  const hasParts = computed(() => loaded.size > 0);
  const statVerts = computed(() => {
    let n = 0;
    for (const p of visiblePaths.value) n += loaded.get(p)?.vertices.length ?? 0;
    return n;
  });
  const statTris = computed(() => {
    let n = 0;
    for (const p of visiblePaths.value) {
      const m = loaded.get(p);
      if (m) n += trianglesOf(m).length / 3;
    }
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
  // 开关切换时立即应用到当前选中状态
  watch(pickHighlight, (v) => viewer.value?.highlight(v ? selectedPart.value : null));

  return reactive({
    rootDir,
    modelSet,
    scanning,
    renderMode,
    wireframe,
    pickHighlight,
    selectedPart,
    loaded,
    texCache,
    dirty,
    groupSel,
    groupVis,
    selectedModel,
    visiblePaths,
    visibleCount,
    hasParts,
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
    slotOptions,
    slotCurrent,
    slotVisible,
    selectSlotVariant,
    toggleSlot,
    outfitStyles,
    outfitCurrent,
    selectOutfit,
    selectPart,
    addToViewer,
    soloPreview,
    texSlotSel,
    applyTexVariant,
    partTexVariants,
    partTexCurrent,
    applyPartTexture,
    setColor,
    touchVertex,
    scan,
    clearScene,
    unloadParts,
    pickDir,
    saveSelected,
    saveAs,
    recomputeBbox,
    exportObj,
    frameAll,
  });
}

export type SkinStore = ReturnType<typeof useSkinMode>;
