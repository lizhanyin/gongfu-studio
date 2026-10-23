import { computed, reactive, ref, watch, type ShallowRef } from "vue";
import { api } from "../api";
import type {
  CharacterCatalog,
  EquipItem,
  FskinModel,
  FsmModel,
} from "../types";
import { texDataFromDto, b64ToBytes, decodeTga } from "../tex";
import type { TexData, Viewer } from "../viewer";
import type { SkinStore } from "./useSkinMode";
import type { ActStore } from "./useActMode";
import { message } from "../message";
import { shortName } from "../utils";

/** 装备槽 → 穿上后隐藏的基础部件槽（scan_model_set 的槽名）。
 *  hat 隐藏 hair：头部装备模型自带头发（替换原发型，游戏实测） */
const SLOT_HIDE_BASE: Record<string, string[]> = {
  hat: ["hair"],
  armonr: ["upper"],
  lower: ["lower"],
  glove: [],
  boot: ["boot"],
};

/** 时装（全身含头，模型同样自带头发 → 发型一并替换，游戏实测）隐藏的基础部件槽 */
const FASHION_HIDE_BASE = ["hair", "upper", "lower", "boot"];

/** 与时装互斥渲染的身体装备槽（帽子可与时装共存） */
const FASHION_MUTEX_SLOTS = ["armonr", "lower", "glove", "boot"];

/** 基础模型目录的全部替换槽（皮肤模式的槽名） */
const BASE_SLOTS = ["hair", "upper", "lower", "boot"];

function stemOf(file: string): string {
  return (file.split(/[\\/]/).pop() ?? "").replace(/\.[^.]*$/, "").toLowerCase();
}

/**
 * 角色装扮模式：组合 skin（基础模型 + 目录内变体）与 act（骨架动作），
 * 在此之上管理装备槽（Hat/Armonr/Lower/Glove/Boot）与时装（shizhuang）。
 * 装备/时装贴图先查角色全根索引（跨目录解析脸型等），回落基础目录索引。
 */
export function useDressMode(
  viewer: ShallowRef<Viewer | null>,
  skin: SkinStore,
  act: ActStore
) {
  const catalog = ref<CharacterCatalog | null>(null);
  const scanning = ref(false);
  const baseKey = ref("");
  /** 槽位开关与当前物品路径（slot key → state） */
  const equipOn = reactive(new Map<string, boolean>());
  const equipPath = reactive(new Map<string, string>());
  const fashionOn = ref(false);
  const fashionPath = ref<string | null>(null);
  /** 帽子挂接骨骼名（空 = 自动选含 head 的骨）与显示偏移 */
  const hatBone = ref("");
  const hatOffset = ref<[number, number, number]>([0, 0, 0]);

  /** 已加载的装备/时装模型（fskin 或 fsm），跨槽位复用 */
  const loaded = reactive(new Map<string, FskinModel | FsmModel>());
  const texCache = reactive(new Map<string, TexData | null>());

  const base = computed(
    () => catalog.value?.bases.find((b) => b.key === baseKey.value) ?? null
  );
  const gender = computed(() => base.value?.gender ?? "f");

  // ---------- 贴图 ----------

  async function ensureTex(file: string): Promise<TexData | null> {
    if (texCache.has(file)) return texCache.get(file) ?? null;
    let d: TexData | null = null;
    try {
      if (/\.dds$/i.test(file)) {
        d = texDataFromDto(await api.loadTexture(file));
      } else if (/\.tga$/i.test(file)) {
        d = decodeTga(b64ToBytes(await api.readFileB64(file)));
      } else {
        message.warning(`暂不支持解码该格式贴图: ${shortName(file)}`);
      }
    } catch (e) {
      message.error(`纹理解码失败 ${shortName(file)}: ${e}`);
    }
    texCache.set(file, d);
    return d;
  }

  /** 材质路径 → 贴图文件：全根索引优先，回落基础目录索引 */
  function resolveTexPath(texturePath: string): string | null {
    const stem = stemOf(texturePath);
    if (!stem) return null;
    return (
      catalog.value?.textures.find((t) => t.stem === stem)?.path ??
      skin.resolveTexture(texturePath)
    );
  }

  /** fskin 每材质槽解析贴图后加入视口（装备/时装共用） */
  async function addFskinPart(path: string) {
    const v = viewer.value;
    if (!v || v.has(path)) return;
    const cached = loaded.get(path);
    let m: FskinModel;
    if (cached && !("dummies" in cached)) {
      m = cached;
    } else {
      m = await api.loadFskin(path);
      loaded.set(path, m);
    }
    const texes: (TexData | null)[] = [];
    const files: (string | null)[] = [];
    for (const md of m.materials) {
      const file = md.texturePath ? resolveTexPath(md.texturePath) : null;
      files.push(file);
      texes.push(file ? await ensureTex(file) : null);
    }
    v.addPart(path, m, texes, files);
  }

  // ---------- 槽位物品（按当前性别过滤） ----------

  function slotItems(slotKey: string): EquipItem[] {
    const slot = catalog.value?.slots.find((s) => s.key === slotKey);
    if (!slot) return [];
    return slot.items.filter((i) => i.gender === "u" || i.gender === gender.value);
  }

  const currentItem = (slotKey: string): EquipItem | null =>
    slotItems(slotKey).find((i) => i.path === equipPath.get(slotKey)) ?? null;

  const fashionItems = computed<EquipItem[]>(() =>
    (catalog.value?.fashions ?? []).filter(
      (i) => i.gender === "u" || i.gender === gender.value
    )
  );
  const currentFashion = computed<EquipItem | null>(
    () => fashionItems.value.find((i) => i.path === fashionPath.value) ?? null
  );

  // ---------- 基础部件显隐联动 ----------

  /** 被装扮隐藏的基础槽（只还原自己隐藏的，不动用户手动关掉的） */
  const hiddenByDress = reactive(new Set<string>());

  /** 被装扮替换的基础槽 → 替换来源标签（"护甲"/"下装"/"时装"…），供部件卡置灰 */
  const replacedBaseSlots = computed<Record<string, string>>(() => {
    const out: Record<string, string> = {};
    if (fashionOn.value) {
      for (const s of FASHION_HIDE_BASE) out[s] = "时装";
    } else {
      for (const s of catalog.value?.slots ?? []) {
        if (!equipOn.get(s.key)) continue;
        for (const b of SLOT_HIDE_BASE[s.key] ?? []) out[b] = s.label;
      }
    }
    return out;
  });

  /** 按当前装扮同步基础部件显隐（隐藏被装备/时装替换的基础件） */
  function syncBaseSlots() {
    const hide = new Set<string>();
    if (fashionOn.value) {
      for (const s of FASHION_HIDE_BASE) hide.add(s);
    } else {
      for (const [slotKey, on] of equipOn) {
        if (!on) continue;
        for (const s of SLOT_HIDE_BASE[slotKey] ?? []) hide.add(s);
      }
    }
    for (const slot of BASE_SLOTS) {
      const wantHide = hide.has(slot);
      const wasHidden = hiddenByDress.has(slot);
      if (wantHide && !wasHidden && skin.slotVisible(slot)) {
        skin.toggleSlot(slot, false);
        hiddenByDress.add(slot);
      } else if (!wantHide && wasHidden) {
        hiddenByDress.delete(slot);
        if (!skin.slotVisible(slot)) skin.toggleSlot(slot, true);
      }
    }
    // 基础头部（girl_head，脸贴图 wf01）不随时装隐藏：时装模型无脸贴图槽，
    // 脸由基础头提供，保持显示与可编辑（换脸型 / 换头型变体）
  }

  // ---------- 时装脸型联动 ----------

  /** 时装头材质槽（贴图 stem 以 wf 开头）下标；无则 -1 */
  function fashionHeadMat(): number {
    const p = fashionPath.value;
    if (!p) return -1;
    const m = loaded.get(p);
    if (!m || !("materials" in m)) return -1;
    return m.materials.findIndex((md) => stemOf(md.texturePath).startsWith("wf"));
  }

  /** 当前脸部纹理选择同步到时装头材质（跨目录解析基础目录的 TFace 贴图） */
  async function applyFaceToFashion() {
    const v = viewer.value;
    const p = fashionPath.value;
    if (!v || !p || !fashionOn.value) return;
    const slot = skin.modelSet?.texSlots.find((s) => s.key === "face");
    if (!slot) return;
    const file = skin.texSlotSel.get("face") ?? slot.variants[0]?.path ?? null;
    const idx = fashionHeadMat();
    if (idx < 0 || !file) return;
    const tex = await ensureTex(file);
    if (tex) v.refreshTexture(p, tex, file, idx);
  }

  // ---------- 装备 / 时装操作 ----------

  function removeDressPart(path: string | null, kind: string) {
    if (!path) return;
    if (kind === "fsm") viewer.value?.removeBoneAttached(path);
    else viewer.value?.removePart(path);
  }

  /** 卸下槽位当前物品（不改变选择路径） */
  function unequipSlot(slotKey: string) {
    const slot = catalog.value?.slots.find((s) => s.key === slotKey);
    const path = equipPath.get(slotKey);
    if (path && slot) removeDressPart(path, slot.kind);
    equipOn.set(slotKey, false);
    syncBaseSlots();
  }

  /** 选择并穿上槽位物品（null = 卸下） */
  async function setEquip(slotKey: string, path: string | null) {
    const slot = catalog.value?.slots.find((s) => s.key === slotKey);
    if (!slot) return;
    if (!path) {
      unequipSlot(slotKey);
      equipPath.delete(slotKey);
      return;
    }
    const old = equipPath.get(slotKey);
    if (old && old !== path) removeDressPart(old, slot.kind);
    equipPath.set(slotKey, path);
    equipOn.set(slotKey, true);
    // 时装覆盖期间身体装备只记录状态不渲染（关时装时恢复）；帽子共存
    const hiddenByFashion =
      fashionOn.value && FASHION_MUTEX_SLOTS.includes(slotKey);
    if (slot.kind === "fsm") await addHat(path);
    else if (!hiddenByFashion) await addFskinPart(path);
    syncBaseSlots();
  }

  function toggleEquip(slotKey: string, on: boolean) {
    if (on) {
      const path =
        equipPath.get(slotKey) ?? slotItems(slotKey)[0]?.path ?? null;
      if (path) void setEquip(slotKey, path);
    } else {
      unequipSlot(slotKey);
    }
  }

  /** 槽内步进浏览（dir = ±1） */
  async function stepEquip(slotKey: string, dir: number) {
    const items = slotItems(slotKey);
    if (!items.length) return;
    const i = items.findIndex((p) => p.path === equipPath.get(slotKey));
    const next =
      i < 0
        ? dir > 0
          ? items[0]
          : items[items.length - 1]
        : items[(i + dir + items.length) % items.length];
    await setEquip(slotKey, next.path);
  }

  // ---------- 帽子（FSM 挂骨骼） ----------

  function autoHatBone(): string {
    if (hatBone.value) return hatBone.value;
    const names = viewer.value?.boneNames() ?? [];
    return (
      names.find((n) => n.toLowerCase().includes("head")) ??
      names.find((n) => n.toLowerCase().includes("tou")) ??
      ""
    );
  }

  async function addHat(path: string) {
    const v = viewer.value;
    if (!v) return;
    const cached = loaded.get(path);
    let m: FsmModel;
    if (cached && "dummies" in cached) {
      m = cached;
    } else {
      m = await api.loadFsm(path);
      loaded.set(path, m);
    }
    const mats: { tex: TexData | null; file: string | null }[] = [];
    for (const md of m.materials) {
      const file = md.texturePath ? resolveTexPath(md.texturePath) : null;
      mats.push({ tex: file ? await ensureTex(file) : null, file });
    }
    v.addBoneAttached(path, m, mats, autoHatBone(), hatOffset.value);
  }

  // ---------- 时装 ----------

  async function setFashion(path: string | null) {
    if (!path) {
      if (fashionPath.value) viewer.value?.removePart(fashionPath.value);
      fashionPath.value = null;
      fashionOn.value = false;
      // 先恢复身体装备渲染，再同步基础槽显隐（装备重新隐藏对应基础件）
      for (const key of FASHION_MUTEX_SLOTS) {
        const s = catalog.value?.slots.find((x) => x.key === key);
        if (!s) continue;
        const p = equipPath.get(s.key);
        if (p && equipOn.get(s.key)) {
          if (s.kind === "fsm") await addHat(p);
          else await addFskinPart(p);
        }
      }
      syncBaseSlots();
      return;
    }
    const old = fashionPath.value;
    if (old && old !== path) viewer.value?.removePart(old);
    fashionPath.value = path;
    fashionOn.value = true;
    // 身体装备与时装互斥显示：卸下渲染（开关状态保留，面板置灰「被时装覆盖」），
    // 帽子与时装共存
    for (const key of FASHION_MUTEX_SLOTS) {
      const s = catalog.value?.slots.find((x) => x.key === key);
      if (!s) continue;
      const p = equipPath.get(s.key);
      if (p && equipOn.get(s.key)) removeDressPart(p, s.kind);
    }
    await addFskinPart(path);
    syncBaseSlots();
    await applyFaceToFashion();
  }

  function toggleFashion(on: boolean) {
    if (on) {
      const p = fashionPath.value ?? fashionItems.value[0]?.path ?? null;
      if (p) void setFashion(p);
    } else if (fashionPath.value) {
      void setFashion(null);
    }
  }

  async function stepFashion(dir: number) {
    const items = fashionItems.value;
    if (!items.length) return;
    const i = items.findIndex((p) => p.path === fashionPath.value);
    const next =
      i < 0
        ? dir > 0
          ? items[0]
          : items[items.length - 1]
        : items[(i + dir + items.length) % items.length];
    await setFashion(next.path);
  }

  // ---------- 整套切换 ----------

  /** 可整套切换的款式号（任一槽存在该号物品即列入） */
  const setNumbers = computed<number[]>(() => {
    if (!catalog.value) return [];
    const nums = new Set<number>();
    for (const s of catalog.value.slots) {
      for (const it of slotItems(s.key)) {
        if (it.number && it.number > 0) nums.add(it.number);
      }
    }
    return [...nums].sort((a, b) => a - b);
  });

  /** 当前套装号：null = 默认（全卸），-1 = 混合搭配 */
  const setCurrent = computed<number | null>(() => {
    let n: number | null = null;
    let any = false;
    for (const s of catalog.value?.slots ?? []) {
      if (!equipOn.get(s.key)) continue;
      const item = currentItem(s.key);
      if (!item) continue;
      any = true;
      const num = item.number ?? 0;
      if (n === null) n = num;
      else if (n !== num) return -1;
    }
    return any ? n : null;
  });

  /** 整套切换：null = 全部卸下恢复基础模型，n = 各槽穿款式 n（无该号的槽卸下） */
  async function selectSet(n: number | null) {
    for (const s of catalog.value?.slots ?? []) {
      const item =
        n === null
          ? null
          : slotItems(s.key).find((i) => i.number === n) ?? null;
      await setEquip(s.key, item?.path ?? null);
    }
  }

  async function stepSet(dir: number) {
    const list: (number | null)[] = [null, ...setNumbers.value];
    const i = list.indexOf(setCurrent.value);
    const next =
      i < 0
        ? dir > 0
          ? list[1] ?? null
          : list[list.length - 1]
        : list[(i + dir + list.length) % list.length];
    await selectSet(next);
  }

  // ---------- 基础模型 ----------

  /** 基础模型目录装配：卸旧部件 → 换骨架 → skin.scan 装配新基础部件。
   *  顺序关键：部件 boneIndex 按各自骨架编号，任何「旧部件×新骨架 /
   *  新部件×旧骨架」中间态都会在蒙皮/链检测中越界（切性别报错根源）。 */
  async function loadBase() {
    const b = base.value;
    if (!b) return;
    // 先卸下全部装扮件与旧基础部件（旧部件×旧骨架，合法）
    removeAllDressParts();
    hiddenByDress.clear();
    skin.unloadParts();
    // 换骨架（此时场景无部件，setFskel 的全量重蒙皮空转，安全）
    if (b.defaultFskel) await act.openActFile(b.defaultFskel);
    else act.closeAct();
    // 装配新基础部件（新部件×新骨架，配套）
    await skin.scan(b.dir);
  }

  function removeAllDressParts() {
    for (const s of catalog.value?.slots ?? []) {
      const path = equipPath.get(s.key);
      if (path) removeDressPart(path, s.kind);
    }
    if (fashionPath.value) removeDressPart(fashionPath.value, "fskin");
  }

  /** 重新穿上全部已开启的装扮（loadBase / 视口重建后调用） */
  async function replayEquips() {
    for (const s of catalog.value?.slots ?? []) {
      // 时装覆盖期间身体装备不渲染（关闭时装时恢复）
      if (fashionOn.value && FASHION_MUTEX_SLOTS.includes(s.key)) continue;
      const path = equipPath.get(s.key);
      if (path && equipOn.get(s.key)) {
        if (s.kind === "fsm") await addHat(path);
        else await addFskinPart(path);
      }
    }
    if (fashionOn.value && fashionPath.value) {
      await addFskinPart(fashionPath.value);
      await applyFaceToFashion();
    }
    syncBaseSlots();
  }

  /** 换基础模型（性别）：装备/时装按 槽位+款式号 迁移，找不到则卸下。
   *  全程冻结渲染，迁移完成后一次性呈现。 */
  async function switchBase(key: string) {
    if (!catalog.value || key === baseKey.value) return;
    const nums = new Map<string, number | null>();
    for (const s of catalog.value.slots) {
      nums.set(
        s.key,
        equipOn.get(s.key) ? currentItem(s.key)?.number ?? null : null
      );
    }
    const fashionNum = fashionOn.value ? currentFashion.value?.number ?? null : null;
    viewer.value?.holdRender(true);
    try {
      baseKey.value = key;
      await loadBase();
      for (const s of catalog.value.slots) {
        const n = nums.get(s.key);
        const item =
          n === null || n === undefined
            ? null
            : slotItems(s.key).find((i) => i.number === n) ?? null;
        if (item) await setEquip(s.key, item.path);
        else if (equipPath.get(s.key)) unequipSlot(s.key);
      }
      if (fashionNum !== null && fashionNum !== undefined) {
        const f =
          fashionItems.value.find((i) => i.number === fashionNum) ?? null;
        if (f) await setFashion(f.path);
        else if (fashionPath.value) await setFashion(null);
      }
      message.info(`已切换基础模型: ${base.value?.label ?? key}`);
    } finally {
      viewer.value?.holdRender(false);
    }
  }

  // ---------- 打开 / 关闭 ----------

  /** 扫描角色资源根并进入装扮模式（重复调用 = 换资源根后重扫，状态重置）。
   *  全程冻结渲染，资源加载完成后一次性呈现，避免逐件装配的中间态。 */
  async function open(root?: string): Promise<boolean> {
    scanning.value = true;
    viewer.value?.holdRender(true);
    try {
      const c = await api.scanCharacter(root ?? "");
      catalog.value = c;
      // 重扫时保留当前基础模型（资源里仍有时），首次默认女体
      baseKey.value =
        c.bases.find((b) => b.key === baseKey.value)?.key ??
        (c.bases.find((b) => b.key === "avatargirl") ?? c.bases[0])?.key ??
        "";
      equipOn.clear();
      equipPath.clear();
      fashionOn.value = false;
      fashionPath.value = null;
      hiddenByDress.clear();
      await loadBase();
      message.success(
        `角色装扮: ${c.bases.length} 基础模型 · ${c.slots
          .map((s) => `${s.label} ${s.items.length}`)
          .join(" / ")} · 时装 ${c.fashions.length}`
      );
      return true;
    } catch (e) {
      // 扫描失败清掉旧目录，避免面板停留在已失效的资源上
      catalog.value = null;
      message.error(`角色装扮扫描失败: ${e}`);
      return false;
    } finally {
      viewer.value?.holdRender(false);
      scanning.value = false;
    }
  }

  /** 视口重建（HMR）后重放：重扫当前基础模型并恢复装扮（冻结渲染一次呈现） */
  async function replay() {
    if (!catalog.value) return;
    viewer.value?.holdRender(true);
    try {
      await loadBase();
      await replayEquips();
    } finally {
      viewer.value?.holdRender(false);
    }
  }

  /** 退出装扮模式（切换到 FAK/FSM/单文件预览时调用） */
  function close() {
    removeAllDressParts();
    loaded.clear();
    texCache.clear();
    catalog.value = null;
    equipOn.clear();
    equipPath.clear();
    fashionOn.value = false;
    fashionPath.value = null;
    hiddenByDress.clear();
  }

  // ---------- 联动监听 ----------

  // 帽子挂点/偏移调整
  watch(hatBone, async () => {
    const path = equipPath.get("hat");
    if (path && equipOn.get("hat")) await addHat(path);
  });
  watch(
    hatOffset,
    () => {
      const path = equipPath.get("hat");
      if (path) viewer.value?.setBoneAttachedOffset(path, hatOffset.value);
    },
    { deep: true }
  );

  // 脸部纹理选择变化 → 同步时装头材质
  watch(
    () => skin.texSlotSel.get("face"),
    () => void applyFaceToFashion()
  );

  return reactive({
    catalog,
    scanning,
    baseKey,
    base,
    gender,
    equipOn,
    equipPath,
    fashionOn,
    fashionPath,
    hatBone,
    hatOffset,
    loaded,
    slotItems,
    currentItem,
    fashionItems,
    currentFashion,
    setNumbers,
    setCurrent,
    replacedBaseSlots,
    setEquip,
    toggleEquip,
    stepEquip,
    setFashion,
    toggleFashion,
    stepFashion,
    selectSet,
    stepSet,
    switchBase,
    open,
    replay,
    close,
    resolveTexPath,
    ensureTex,
  });
}

export type DressStore = ReturnType<typeof useDressMode>;
