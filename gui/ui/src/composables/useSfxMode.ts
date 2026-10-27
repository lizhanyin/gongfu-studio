import { computed, onMounted, onUnmounted, reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { SfxFile } from "../types";
import { resolveModelTextures, texCanvasUrl } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/**
 * SFX 特效模式：3D 公告板播放器（公告板按节点 prop+112 偏移真实摆放，
 * 挂路径名的节点做 EffectMotion 速度积分动画，见 viewer.setSfx）+
 * 检查器节点树与资源引用。贴图引用
 * （system\effect\tex\…\*.tga）与 FAK/FSM/FSCN 同「路径优先」口径解析：
 * 按文件路径祖先（CPK 根）拼接命中，未命中换 .dds 重试。
 * 支持改写节点贴图引用路径（raw 内原槽位就地覆写，长度只减不增，
 * 序列化仍字节对齐）并保存（CPK 内文件不可写）。
 */
export function useSfxMode(viewer: ShallowRef<Viewer | null>) {
  const sfxModel = ref<SfxFile | null>(null);
  const sfxLoading = ref(false);
  const texCache = reactive(new Map<string, TexData | null>());
  /** 有未保存的路径改写 */
  const dirty = ref(false);
  /** 翻页帧率（文件未记录，默认 12，可调） */
  const fps = ref(12);
  const playing = ref(false);
  /** 播放头（浮点帧号，0..actionFrames-1） */
  const playTime = ref(0);
  const speed = ref(1);
  /** 节点显隐表（与 nodes 对齐，检查器逐节点开关） */
  const nodeVisible = ref<boolean[]>([]);

  /** 贴图引用 → 解析结果（file 为命中的实际路径；url 为缩略图 dataURL） */
  const texSlots = reactive(new Map<string, { file: string | null; url: string | null }>());

  /** 全部节点贴图引用去重保序（每节点至多 1 条，集中查看用） */
  const uniqueTextures = computed(() => {
    const m = sfxModel.value;
    if (!m) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const n of m.nodes)
      for (const t of n.textures)
        if (!seen.has(t)) {
          seen.add(t);
          out.push(t);
        }
    return out;
  });

  /** 播放时长（帧）= 最长节点贴图序列；单帧文件静态展示 */
  const actionFrames = computed(() => {
    const m = sfxModel.value;
    if (!m) return 1;
    return Math.max(1, ...m.nodes.map((n) => n.textures.length));
  });

  function fillTexSlots(refs: string[], slots: { tex: TexData | null; file: string | null }[]) {
    texSlots.clear();
    refs.forEach((r, i) => {
      if (texSlots.has(r)) return; // 重复引用只记一次（同槽结果一致）
      texSlots.set(r, {
        file: slots[i]?.file ?? null,
        url: slots[i]?.tex ? texCanvasUrl(slots[i].tex!) : null,
      });
    });
  }

  /** 全量解析所有节点贴图引用（保持各节点槽位顺序，供翻页与缩略图共用） */
  async function resolveNodeTextures(model: SfxFile) {
    const refs = model.nodes.flatMap((n) => n.textures);
    const slots = await resolveModelTextures(
      model.path,
      refs,
      texCache,
      (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
    );
    fillTexSlots(refs, slots);
    let o = 0;
    return model.nodes.map((n) => {
      const s = slots.slice(o, o + n.textures.length);
      o += n.textures.length;
      return s;
    });
  }

  async function openSfxFile(path: string) {
    sfxLoading.value = true;
    playing.value = false;
    try {
      const model = await api.loadSfx(path);
      sfxModel.value = model;
      dirty.value = false;
      playTime.value = 0;
      nodeVisible.value = model.nodes.map(() => true);
      const nodeSlots = await resolveNodeTextures(model);
      viewer.value?.setSfx(model, nodeSlots);
      viewer.value?.frameAll();
      playing.value = true;
    } catch (e) {
      message.error(`SFX 加载失败: ${e}`);
    } finally {
      sfxLoading.value = false;
    }
  }

  function closeSfx() {
    playing.value = false;
    sfxModel.value = null;
    texSlots.clear();
    viewer.value?.clearSfx();
  }

  function scrub(v: number) {
    playTime.value = v;
    applyPlayTime();
  }

  function applyPlayTime() {
    const total = actionFrames.value;
    const t = Math.min(Math.max(playTime.value, 0), Math.max(total - 1, 0));
    viewer.value?.applySfxFrame(Math.floor(t));
  }

  let playRaf = 0;
  let lastTs = 0;

  function playTick(ts: number) {
    playRaf = requestAnimationFrame(playTick);
    const dt = Math.min((ts - lastTs) / 1000, 0.1);
    lastTs = ts;
    if (!playing.value || !sfxModel.value) return;
    // 路径动画（EffectMotion 速度积分）与贴图翻页同拍推进
    viewer.value?.tickSfxMotion(dt * speed.value);
    const total = actionFrames.value;
    playTime.value += dt * fps.value * speed.value;
    if (playTime.value >= total) playTime.value = 0;
    applyPlayTime();
  }

  onMounted(() => {
    lastTs = performance.now();
    playRaf = requestAnimationFrame(playTick);
  });
  onUnmounted(() => cancelAnimationFrame(playRaf));

  /** 叠加混合开关（发光贴图默认叠加） */
  function setBlending(additive: boolean) {
    viewer.value?.setSfxBlending(additive);
  }

  /** 节点显隐（公告板+标签一并隐藏） */
  function toggleNodeVisible(i: number) {
    if (i >= nodeVisible.value.length) return;
    nodeVisible.value[i] = !nodeVisible.value[i];
    viewer.value?.setSfxNodeVisible(i, nodeVisible.value[i]);
  }

  /**
   * 改写节点贴图引用路径：raw 内定位该引用第 refIdx 次出现的字节串，就地
   * 覆写（新路径 + NUL + 0 填充至原长，总长不变保证节点链/偏移不变；新路径
   * 只能不长于原路径）。随后重解析贴图并热替换到播放器，置脏待保存。
   */
  async function setNodeTexture(nodeIdx: number, refIdx: number, newRef: string) {
    const model = sfxModel.value;
    const node = model?.nodes[nodeIdx];
    if (!model || !node) return;
    const oldRef = node.textures[refIdx];
    if (!oldRef) return;
    const norm = newRef.replace(/[\\/]+/g, "\\").trim();
    if (!/^[\x20-\x7e]{4,}$/.test(norm) || !norm.includes("\\")) {
      message.error("新路径须为含目录的可打印 ASCII 路径（如 system\\effect\\tex\\a\\b.dds）");
      return;
    }
    if (!/^[\x20-\x7e]+$/.test(oldRef)) {
      message.error("原引用含非 ASCII 字符（GBK），暂不支持定位改写");
      return;
    }
    try {
      node.raw = replaceRefBytes(node.raw, oldRef, refIdx, norm);
    } catch (e) {
      message.error(`路径改写失败: ${e}`);
      return;
    }
    node.textures[refIdx] = norm;
    dirty.value = true;
    // 重解析该引用（路径优先，含 .dds 重试）并刷新缩略图/播放器
    const [slot] = await resolveModelTextures(
      model.path,
      [norm],
      texCache,
      (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
    );
    texSlots.set(norm, {
      file: slot.file,
      url: slot.tex ? texCanvasUrl(slot.tex) : null,
    });
    if (slot.tex && slot.file) viewer.value?.setSfxTexture(nodeIdx, refIdx, slot.file, slot.tex);
    if (!slot.tex) message.warning(`新路径未解析到贴图: ${shortName(norm)}`);
  }

  /** raw 内第 occIdx 次出现的 NUL 结尾路径引用就地覆写（长度不变） */
  function replaceRefBytes(raw: number[], oldRef: string, occIdx: number, newRef: string): number[] {
    const enc = (s: string) => Array.from(new TextEncoder().encode(s));
    const oldB = enc(oldRef);
    const newB = enc(newRef);
    if (newB.length > oldB.length) {
      throw new Error(`新路径 ${newB.length}B 超过原槽位 ${oldB.length}B（只支持不长于原路径的改写）`);
    }
    let count = 0;
    for (let i = 0; i + oldB.length < raw.length; i++) {
      // 须为 token 起点（前一字节不可打印），与 sfx.rs extract_refs 的切词口径一致
      const prev = i > 0 ? raw[i - 1] : 0;
      if (prev >= 0x20 && prev < 0x7f) continue;
      let match = true;
      for (let k = 0; k < oldB.length; k++)
        if (raw[i + k] !== oldB[k]) {
          match = false;
          break;
        }
      if (!match || raw[i + oldB.length] !== 0) continue;
      if (count++ !== occIdx) continue;
      const out = raw.slice();
      for (let k = 0; k < oldB.length; k++) out[i + k] = k < newB.length ? newB[k] : 0;
      out[i + newB.length] = 0;
      return out;
    }
    throw new Error("未在节点参数区定位到该引用（字节与引用列表不一致）");
  }

  /** 保存 .sfx（CPK 内不可写，由后端拒绝并报错） */
  async function saveSfx() {
    const model = sfxModel.value;
    if (!model) return;
    try {
      await api.saveSfx(model.path, model);
      dirty.value = false;
      message.success(`已保存 ${shortName(model.path)}`);
    } catch (e) {
      message.error(`保存失败: ${e}`);
    }
  }

  return reactive({
    sfxModel,
    sfxLoading,
    openSfxFile,
    closeSfx,
    texSlots,
    uniqueTextures,
    // 播放器（PlayBar PlayerLike 同构：帧号口径）
    playing,
    playTime,
    actionFrames,
    fps,
    speed,
    scrub,
    setBlending,
    nodeVisible,
    toggleNodeVisible,
    // 路径改写与保存
    dirty,
    setNodeTexture,
    saveSfx,
  });
}

export type SfxStore = ReturnType<typeof useSfxMode>;
