import { computed, onMounted, onUnmounted, reactive, ref, type ShallowRef } from "vue";
import { api } from "../api";
import type { SfxFile } from "../types";
import { resolveModelTextures } from "../tex";
import type { TexData, Viewer } from "../viewer";
import { message } from "../message";
import { shortName } from "../utils";

/** 挂接特效实例（UI 态；渲染对象与播放推进由 viewer.applyFxFrame 系列承载） */
export interface FxAttach {
  id: number;
  path: string;
  mode: "bone" | "world";
  bone: string;
  worldPos: [number, number, number];
  offset: [number, number, number];
  euler: [number, number, number];
  scale: number;
  fps: number;
  speed: number;
  loop: boolean;
  playing: boolean;
  /** 已播秒数（loop 时对 lifetime 取模，进度条循环） */
  elapsed: number;
  /** 生命周期（秒）：到点自动停，引擎客户端 0x1388=5000ms 同语义 */
  lifetime: number;
  additive: boolean;
  visible: boolean;
  /** 与 model.nodes 对齐的节点静音表（EffectSound/EffectMesh 恒无渲染） */
  nodeMute: boolean[];
  /** 检查器用的节点摘要（attach 时快照，不随后续编辑变化） */
  nodes: { kind: string; name: string; hasTex: boolean; hasSnd: boolean; hasMesh: boolean }[];
  /** 根矩阵平移（「取根矩阵」世界摆放一键填充） */
  rootTrans: [number, number, number];
}

/** 每特效挂接记忆：path → 挂点与变换。localStorage "fx.attachTuning.v1" 持久化 */
const FX_TUNING_KEY = "fx.attachTuning.v1";
type FxTuning = Pick<
  FxAttach,
  "mode" | "bone" | "worldPos" | "offset" | "euler" | "scale" | "additive"
>;
function loadFxTuning(): Map<string, FxTuning> {
  const m = new Map<string, FxTuning>();
  try {
    const v = JSON.parse(localStorage.getItem(FX_TUNING_KEY) ?? "null");
    if (v && typeof v === "object") {
      for (const [k, t] of Object.entries(v)) {
        const tt = t as FxTuning;
        if (
          (tt.mode === "bone" || tt.mode === "world") &&
          typeof tt.bone === "string" &&
          Array.isArray(tt.offset) &&
          tt.offset.length === 3
        ) {
          m.set(k, {
            mode: tt.mode,
            bone: tt.bone,
            worldPos: [...(tt.worldPos ?? [0, 0, 0])] as [number, number, number],
            offset: [...tt.offset] as [number, number, number],
            euler: [...(tt.euler ?? [0, 0, 0])] as [number, number, number],
            scale: tt.scale ?? 1,
            additive: tt.additive ?? true,
          });
        }
      }
    }
  } catch {
    // 配置损坏从空开始
  }
  return m;
}

/** 特效清单条目（group = Effect 下首级目录名，即游戏侧用途语义） */
export interface FxCatalogEntry {
  path: string;
  group: string;
  name: string;
}

/**
 * 特效挂载模式：把 .sfx 以公告板播放器叠加到当前角色上（挂骨跟随 / 世界
 * 放置），多实例叠挂、节点级静音、生命周期到点自停（5s）——对齐逆向出的
 * 引擎语义（SGAttachableEffectNode::BindToTagNode 挂骨、SetWorldMat 世界
 * 摆放、客户端 5000ms 生命周期、FindEffect 节点粒度）。与 SFX 预览模式
 * （独立检查场景）互不影响；角色离开场景（FAK/FSM 等预览）由 App 调 clear。
 */
export function useEffectMode(viewer: ShallowRef<Viewer | null>) {
  const instances = reactive(new Map<number, FxAttach>());
  /** 模型与已解析贴图槽（非响应式：体积大，UI 只看 instances 摘要） */
  const store = new Map<number, { model: SfxFile; slots: { file: string | null; tex: TexData | null }[][] }>();
  const texCache = reactive(new Map<string, TexData | null>());
  let nextId = 1;

  // ---------- 每特效挂接记忆 ----------

  const tuning = loadFxTuning();
  let tuningSaveTimer: ReturnType<typeof setTimeout> | undefined;
  function saveTuningOf(inst: FxAttach) {
    tuning.set(inst.path, {
      mode: inst.mode,
      bone: inst.bone,
      worldPos: [...inst.worldPos] as [number, number, number],
      offset: [...inst.offset] as [number, number, number],
      euler: [...inst.euler] as [number, number, number],
      scale: inst.scale,
      additive: inst.additive,
    });
    clearTimeout(tuningSaveTimer);
    tuningSaveTimer = setTimeout(() => {
      try {
        localStorage.setItem(FX_TUNING_KEY, JSON.stringify(Object.fromEntries(tuning)));
      } catch {
        // 存储满等异常忽略（内存态仍生效）
      }
    }, 250);
  }

  // ---------- 特效清单 / NPC 配置（按资源根懒加载） ----------

  const catalog = ref<FxCatalogEntry[]>([]);
  const catalogLoading = ref(false);
  const npcCsvList = ref<{ path: string; name: string }[]>([]);
  let catalogedRoot = "";

  /** 递归列举（跳过 tex 贴图目录与深层，depth≤3 覆盖 Effect\Combat|Scn|role|Quest） */
  async function walkFiles(dir: string, exts: string[], depth = 0): Promise<{ path: string; name: string }[]> {
    if (depth > 3) return [];
    const out: { path: string; name: string }[] = [];
    let entries: Awaited<ReturnType<typeof api.listResDir>>;
    try {
      entries = await api.listResDir(dir);
    } catch {
      return []; // 目录不存在（磁盘根无 system 等）
    }
    for (const e of entries) {
      if (e.isDir) {
        if (/^tex$/i.test(e.name)) continue;
        out.push(...(await walkFiles(e.path, exts, depth + 1)));
      } else if (exts.includes(e.kind)) {
        out.push({ path: e.path, name: e.name });
      }
    }
    return out;
  }

  async function ensureCatalog(root: string) {
    if (!root || root === catalogedRoot) return;
    catalogedRoot = root;
    catalogLoading.value = true;
    try {
      const [files, csvs] = await Promise.all([
        walkFiles(`${root}\\system\\Effect`, ["sfx"]),
        walkFiles(`${root}\\system\\NPC`, ["csv"]),
      ]);
      catalog.value = files.map((f) => {
        const m = f.path.match(/.*?system[\\/]Effect[\\/](.*)/i);
        const rel = m ? m[1] : f.path;
        const group = rel.includes("\\") || rel.includes("/")
          ? rel.split(/[\\/]/)[0]
          : "(根)";
        return { path: f.path, group, name: f.name };
      });
      npcCsvList.value = csvs;
    } finally {
      catalogLoading.value = false;
    }
  }

  // ---------- 实例渲染同步 ----------

  function pushToViewer(inst: FxAttach) {
    const v = viewer.value;
    const s = store.get(inst.id);
    if (!v || !s) return;
    v.addFx(inst.id, s.model, s.slots, {
      mode: inst.mode,
      bone: inst.bone,
      worldPos: inst.worldPos,
      offset: inst.offset,
      euler: inst.euler,
      scale: inst.scale,
      additive: inst.additive,
      visible: inst.visible,
      nodeMute: inst.nodeMute,
    });
  }

  // ---------- 挂载 / 卸载 ----------

  /** 挂上特效（preset 覆盖记忆，NPC csv 还原用）；自动开播 */
  async function attach(path: string, preset?: Partial<FxTuning>) {
    const v = viewer.value;
    if (!v) return;
    try {
      const model = await api.loadSfx(path);
      const refs = model.nodes.flatMap((n) => n.textures);
      const slots = await resolveModelTextures(
        model.path,
        refs,
        texCache,
        (file, e) => message.error(`纹理解码失败 ${shortName(file)}: ${e}`)
      );
      let o = 0;
      const nodeSlots = model.nodes.map((n) => {
        const s = slots.slice(o, o + n.textures.length);
        o += n.textures.length;
        return s;
      });
      const id = nextId++;
      const base: FxTuning =
        tuning.get(path) ?? {
          mode: "bone",
          bone: "",
          worldPos: [0, 0, 0],
          offset: [0, 0, 0],
          euler: [0, 0, 0],
          scale: 1,
          additive: true,
        };
      const t: FxTuning = preset ? { ...base, ...preset } : base;
      const inst: FxAttach = {
        id,
        path,
        mode: t.mode,
        bone: t.bone,
        worldPos: [...t.worldPos] as [number, number, number],
        offset: [...t.offset] as [number, number, number],
        euler: [...t.euler] as [number, number, number],
        scale: t.scale,
        fps: 12,
        speed: 1,
        loop: false,
        playing: true,
        elapsed: 0,
        lifetime: 5,
        additive: t.additive,
        visible: true,
        nodeMute: model.nodes.map((n) => n.kind === "EffectSound"),
        nodes: model.nodes.map((n) => ({
          kind: n.kind,
          name: n.name,
          hasTex: n.textures.length > 0,
          hasSnd: n.sounds.length > 0,
          hasMesh: n.models.length > 0,
        })),
        rootTrans: [model.matrix[12], model.matrix[13], model.matrix[14]],
      };
      store.set(id, { model, slots: nodeSlots });
      instances.set(id, inst);
      pushToViewer(inst);
      // 记忆骨名不在当前骨架（如 NPC 专属 eft1/eft2）→ 提示改选，仍按原点渲染
      if (inst.mode === "bone" && inst.bone) {
        const names = v.boneNames();
        if (!names.some((n) => n.toLowerCase() === inst.bone.toLowerCase())) {
          message.warning(
            `骨 "${inst.bone}" 不在当前骨架（NPC 专属挂骨常见），特效暂置角色原点，可改选 tag 挂骨`
          );
        }
      }
    } catch (e) {
      message.error(`特效加载失败 ${shortName(path)}: ${e}`);
    }
  }

  function remove(id: number) {
    viewer.value?.removeFx(id);
    store.delete(id);
    instances.delete(id);
  }

  function clear() {
    for (const id of [...instances.keys()]) remove(id);
    texCache.clear();
  }

  /** 视口重建（HMR）/ 角色回归后重挂全部实例 */
  function replay() {
    for (const inst of instances.values()) pushToViewer(inst);
  }

  // ---------- 参数更新（写 viewer + 记忆） ----------

  function setTransform(
    id: number,
    t: Partial<{
      mode: "bone" | "world";
      bone: string;
      worldPos: [number, number, number];
      offset: [number, number, number];
      euler: [number, number, number];
      scale: number;
      additive: boolean;
      visible: boolean;
    }>
  ) {
    const inst = instances.get(id);
    if (!inst) return;
    Object.assign(inst, t);
    viewer.value?.setFxTransform(id, t);
    saveTuningOf(inst);
  }

  function toggleNode(id: number, nodeIdx: number) {
    const inst = instances.get(id);
    if (!inst || nodeIdx >= inst.nodeMute.length) return;
    inst.nodeMute[nodeIdx] = !inst.nodeMute[nodeIdx];
    viewer.value?.setFxNodeVisible(id, nodeIdx, !inst.nodeMute[nodeIdx]);
  }

  function togglePlay(id: number) {
    const inst = instances.get(id);
    if (!inst) return;
    if (!inst.playing && !inst.loop && inst.elapsed >= inst.lifetime) {
      replayInst(id); // 播完状态再按播放 = 重播
      return;
    }
    inst.playing = !inst.playing;
  }

  function replayInst(id: number) {
    const inst = instances.get(id);
    if (!inst) return;
    inst.elapsed = 0;
    inst.playing = true;
    viewer.value?.resetFxMotion(id);
    viewer.value?.applyFxFrame(id, 0);
  }

  // ---------- 播放推进（引擎 MainUpdate 语义：elapsed += dt × speed，到寿命自停） ----------

  let raf = 0;
  let lastTs = 0;
  function tick(ts: number) {
    raf = requestAnimationFrame(tick);
    const v = viewer.value;
    if (!v) return;
    const dt = Math.min((ts - lastTs) / 1000, 0.1);
    lastTs = ts;
    if (!instances.size) return;
    // 角色在场而实例不在视口（骨架换载清场）→ 自动重挂，状态不丢
    if (v.skeletonReady()) {
      for (const inst of instances.values()) if (!v.hasFx(inst.id)) pushToViewer(inst);
    }
    for (const inst of instances.values()) {
      if (inst.playing) {
        const step = dt * inst.speed;
        inst.elapsed += step;
        if (!inst.loop && inst.elapsed >= inst.lifetime) {
          inst.elapsed = inst.lifetime;
          inst.playing = false; // 生命周期到点自停（保留末帧，重播按钮恢复）
        } else if (inst.loop) {
          inst.elapsed %= inst.lifetime;
        }
        // 路径动画（EffectMotion 速度积分）与贴图翻页同拍推进
        v.updateFxMotion(inst.id, step);
        v.applyFxFrame(inst.id, Math.floor(inst.elapsed * inst.fps));
      }
    }
    v.updateFxMatrices(); // 暂停实例也要跟随骨骼动画
  }

  // ---------- NPC 原版配置还原 ----------

  /** 解析 System\NPC\*_a.csv：`eft1,system\effect\role\bs013a.sfx,eft2,…` 的
   *  (挂骨名, sfx路径) 成对表（游戏逆向确证的 NPC 特效数据源） */
  function parseNpcCsv(b64: string): { bone: string; path: string }[] {
    const bin = atob(b64);
    const out: { bone: string; path: string }[] = [];
    for (const line of bin.split(/\r?\n/)) {
      const cols = line.split(",");
      for (let i = 0; i + 1 < cols.length; i += 2) {
        const bone = cols[i].trim();
        const path = cols[i + 1].trim();
        if (/^[\x20-\x7e]+$/.test(bone) && /\.sfx$/i.test(path)) {
          out.push({ bone, path });
        }
      }
    }
    return out;
  }

  /** NPC csv → 挂骨名对 sfx 一键还原（路径按当前根拼 system 前缀解析） */
  async function importNpcCsv(csvPath: string, root: string) {
    try {
      const b64 = await api.readFileB64(csvPath);
      const pairs = parseNpcCsv(b64);
      if (!pairs.length) {
        message.warning(`配置里没有特效条目: ${shortName(csvPath)}`);
        return;
      }
      // csv 里的 sfx 路径是 system\effect\role\… 引用式；资源根直拼即 vfs 路径
      const missing: string[] = [];
      for (const p of pairs) {
        const full = /^cpk:/i.test(root) || /[\\/]system$/i.test(root)
          ? `${root}\\${p.path}`
          : `${root}\\${p.path}`;
        // 去重：同一 sfx 只挂一份（eft1/eft2 常指向同一文件）
        if (instances.size && [...instances.values()].some((i) => i.path.toLowerCase() === full.toLowerCase())) {
          continue;
        }
        await attach(full, {
          mode: "bone",
          bone: p.bone,
          worldPos: [0, 0, 0],
          offset: [0, 0, 0],
          euler: [0, 0, 0],
          scale: 1,
          additive: true,
        });
        if (!viewer.value?.boneNames().some((n) => n.toLowerCase() === p.bone.toLowerCase())) {
          missing.push(p.bone);
        }
      }
      message.success(`已按 ${shortName(csvPath)} 挂载 ${pairs.length} 条特效配置`);
      if (missing.length) {
        message.warning(`挂骨 ${[...new Set(missing)].join("/")} 不在角色骨架，已置原点（可改选 tag 挂骨）`);
      }
    } catch (e) {
      message.error(`NPC 配置读取失败: ${e}`);
    }
  }

  onMounted(() => {
    lastTs = performance.now();
    raf = requestAnimationFrame(tick);
  });
  onUnmounted(() => cancelAnimationFrame(raf));

  const count = computed(() => instances.size);

  return reactive({
    instances,
    count,
    catalog,
    catalogLoading,
    npcCsvList,
    ensureCatalog,
    attach,
    remove,
    clear,
    replay,
    setTransform,
    toggleNode,
    togglePlay,
    replayInst,
    importNpcCsv,
  });
}

export type EffectStore = ReturnType<typeof useEffectMode>;
