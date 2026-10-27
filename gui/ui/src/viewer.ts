import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { FakModel, FsmModel, FskinModel, FskelMeta, Influence, SfxCyl, SfxFile, SfxKeyState, SfxPath, Vertex } from "./types";
import { gfm, gfmEnabled } from "./gfm";

export interface TexData {
  width: number;
  height: number;
  rgba: Uint8Array; // 自顶向下 RGBA8
}

export type RenderMode = "shaded" | "bones" | "uv";

interface CloneVert {
  /** 原顶点索引 */
  src: number;
  /** 跨缝调整的整数偏移（±1，普通副本为 0） */
  du: number;
  dv: number;
}

interface PartObj {
  mesh: THREE.Mesh;
  geom: THREE.BufferGeometry;
  /** 每子网格一个材质（几何按 groups 划分；单网格部件长度为 1） */
  mats: THREE.MeshStandardMaterial[];
  model: FskinModel;
  /** 跨缝拆分产生的副本顶点，编辑顶点时需同步 */
  clones: CloneVert[];
  /** 饰品透明叠层（共享几何，TFaceDetail 贴花） */
  overlayMesh: THREE.Mesh | null;
  overlayMat: THREE.MeshStandardMaterial | null;
  /** 几何顶点（含拆缝副本）的影响表，副本沿用源顶点 */
  inflOf: Influence[][];
  /** 各几何顶点的源顶点法线（扁平 xyz，编辑同步用；蒙皮法线恒静态，见 skinPart） */
  srcNrm: Float32Array;
  /** 几何当前是否处于蒙皮态（false = authored 绑定姿势直出） */
  skinned: boolean;
  /** fmath wasm 部件槽位（-1 = TS 回退模式未注册） */
  gfmId: number;
  /** 合成件（长发 FSM 代理，见 addBoneAttached）：boneLocalPos 按
   *  当前骨架绑定姿态生成、数据自洽，链诊断/锚定闸门跳过（纯 LBS 即正确，
   *  再锚定会叠加骨盆载体双重运动） */
  syntheticHair?: boolean;
  /** 合成刚性挂接件（帽子 FSM 代理）：单骨 w=1 烘焙 mount，纯 LBS 刚体
   *  随骨；与 syntheticHair 同样跳过链/裙/障碍池，但不参与长发体表去穿透 */
  syntheticMount?: boolean;
}

function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// 注：本游戏 UV 的 v=0 对应纹理图像顶部（图像坐标，与 OBJ 的 v=0 在底相反）。
// DataTexture 数据第 0 行即 v=0 采样行，直接用自顶向下的 RGBA，切勿做行翻转。

export function texDataFromDto(dto: {
  width: number;
  height: number;
  rgbaBase64: string;
}): TexData {
  return {
    width: dto.width,
    height: dto.height,
    rgba: b64ToBytes(dto.rgbaBase64),
  };
}

function boneColor(bone: number): THREE.Color {
  const hue = ((bone * 0.6180339887) % 1 + 1) % 1;
  return new THREE.Color().setHSL(hue, 0.75, 0.55);
}

/** 诊断日志环形缓冲：window.__diagLog（CDP 无 console 历史 API，历史取回靠它）。
 * 模块加载即建空数组，CDP 侧随时可查（空 = 尚无日志）。 */
const DIAG_MAX = 500;
interface DiagEntry {
  t: number;
  level: "log" | "warn";
  msg: string;
}
const diagBuf = ((window as unknown as { __diagLog?: DiagEntry[] }).__diagLog =
  [] as DiagEntry[]);
function pushDiagLog(level: "log" | "warn", msg: string) {
  diagBuf.push({ t: Date.now(), level, msg });
  if (diagBuf.length > DIAG_MAX) diagBuf.splice(0, diagBuf.length - DIAG_MAX);
}

/** 3×3 行列式（行主序） */
function mat3Det(m: number[]): number {
  return (
    m[0] * (m[4] * m[8] - m[5] * m[7]) -
    m[1] * (m[3] * m[8] - m[5] * m[6]) +
    m[2] * (m[3] * m[7] - m[4] * m[6])
  );
}

function mat3Inv(m: number[]): number[] | null {
  const det = mat3Det(m);
  if (Math.abs(det) < 1e-12) return null;
  const d = 1 / det;
  return [
    (m[4] * m[8] - m[5] * m[7]) * d,
    (m[2] * m[7] - m[1] * m[8]) * d,
    (m[1] * m[5] - m[2] * m[4]) * d,
    (m[5] * m[6] - m[3] * m[8]) * d,
    (m[0] * m[8] - m[2] * m[6]) * d,
    (m[2] * m[3] - m[0] * m[5]) * d,
    (m[3] * m[7] - m[4] * m[6]) * d,
    (m[1] * m[6] - m[0] * m[7]) * d,
    (m[0] * m[4] - m[1] * m[3]) * d,
  ];
}

/** 极分解 Newton 迭代 M ← (M + M⁻ᵀ)/2，收敛到 H 的最近正交旋转因子；退化返回 null */
/**
 * 初始视角与模型朝向：绑定姿势人物面朝 -Z，为让初始渲染即正面朝向相机（+Z 侧），
 * 模型整体绕 Y 旋转 MODEL_YAW（纯渲染侧：仅网格/可视化挂 modelRoot，骨骼层级留在 scene，
 * worldMats/蒙皮/去穿透/CDP 探针仍为数据空间坐标）。相机方位保持 +Z 侧不动，画面轴向与坐标轴习惯一致。
 */
const INIT_VIEW_DIR = new THREE.Vector3(0.55, 0.35, 1).normalize();
const MODEL_YAW = Math.PI;

function polarRotation(h: number[]): number[] | null {
  let m = h.slice();
  for (let it = 0; it < 32; it++) {
    const mi = mat3Inv(m);
    if (!mi) return null;
    const next = new Array<number>(9);
    let delta = 0;
    for (let r = 0; r < 3; r++) {
      for (let c = 0; c < 3; c++) {
        const v = (m[r * 3 + c] + mi[c * 3 + r]) / 2;
        next[r * 3 + c] = v;
        delta = Math.max(delta, Math.abs(v - m[r * 3 + c]));
      }
    }
    m = next;
    if (delta < 1e-10) break;
  }
  return mat3Det(m) > 0.5 ? m : null; // 未收敛或落入反射
}

/** 挂接特效实例（viewer 侧渲染对象；播放状态在 useEffectMode composable） */
interface FxItemObj {
  id: number;
  group: THREE.Group;
  /** 带贴图节点各一块公告板（板位 = 节点偏移 + 路径积分位置）；
   *  on = 节点显隐（≈引擎 FindEffect 粒度） */
  boards: {
    node: number;
    sprite: THREE.Sprite;
    mat: THREE.SpriteMaterial;
    frames: (THREE.Texture | null)[];
    cur: THREE.Texture | null;
    on: boolean;
    /** 节点 prop+112 偏移（引擎公告板位 = 偏移+路径位置，组矩阵负责挂接系） */
    basePos: THREE.Vector3;
    /** 引用的路径名（"" = 定点；须在 motions 中存在） */
    pathName: string;
  }[];
  /** 文件路径名 → 路径对象 + 模拟器（同名共享一份，引擎 SGEffectNode 同口径） */
  motions: Map<string, { p: SfxPath; sim: MotionSim }>;
  mode: "bone" | "world";
  boneIdx: number;
  boneName: string;
  worldPos: [number, number, number];
  offset: [number, number, number];
  eulerRad: [number, number, number];
  scale: number;
  visible: boolean;
}

/** EffectMotion 路径模拟态：引擎语义 = 状态按时长推进（循环回绕）、
 *  状态间线性插值速度、位置逐帧积分（EffectMotion::Update） */
interface MotionSim {
  /** 特效龄（含 delay 之前的等待） */
  t: number;
  /** 当前状态序与态内时间 */
  idx: number;
  elapsed: number;
  pos: THREE.Vector3;
  /** 非循环且末态播完（引擎保持末速度持续漂，5s 寿命兜底；预览口径停住） */
  done: boolean;
}

function makeMotionSim(): MotionSim {
  return { t: 0, idx: 0, elapsed: 0, pos: new THREE.Vector3(), done: false };
}

/** 推进一个路径：dt 秒（引擎 MainUpdate 每帧调 Update；这里等步长积分） */
function tickMotionSim(p: SfxPath, s: MotionSim, dt: number) {
  if (s.done || dt <= 0) return;
  s.t += dt;
  if (s.t < p.delay) return; // 引擎：特效龄超 delay 才开始积分
  const n = p.states.length;
  if (!n) {
    s.done = true;
    return;
  }
  s.elapsed += dt;
  let guard = 0;
  while (s.elapsed > p.states[s.idx][0] && guard++ < 10000) {
    s.elapsed -= p.states[s.idx][0];
    if (s.idx + 1 < n) s.idx++;
    else if (p.looped) s.idx = 0;
    else {
      s.done = true; // 预览停住口径（见 MotionSim.done 注释）
      return;
    }
  }
  const cur = p.states[s.idx];
  const next = s.idx + 1 < n ? p.states[s.idx + 1] : p.looped ? p.states[0] : null;
  const f = Math.min(s.elapsed / Math.max(cur[0], 1e-6), 1);
  let vx = cur[1], vy = cur[2], vz = cur[3];
  if (next) {
    vx += (next[1] - vx) * f;
    vy += (next[2] - vy) * f;
    vz += (next[3] - vz) * f;
  }
  s.pos.x += vx * dt;
  s.pos.y += vy * dt;
  s.pos.z += vz * dt;
}

/** 文件路径表 → 模拟器表（空状态路径不进表 = 引用节点按定点处理） */
function makeMotionMap(paths: SfxPath[]): Map<string, { p: SfxPath; sim: MotionSim }> {
  const m = new Map<string, { p: SfxPath; sim: MotionSim }>();
  for (const p of paths) if (p.name && p.states.length) m.set(p.name, { p, sim: makeMotionSim() });
  return m;
}

/** 节点引用的有效路径名（"none"/空/未定义路径 → ""） */
function effPathName(
  pathName: string,
  motions: Map<string, { p: SfxPath; sim: MotionSim }>
): string {
  return pathName && pathName !== "none" && motions.has(pathName) ? pathName : "";
}

export class Viewer {
  private scene = new THREE.Scene();
  /** 模型根组（绕 Y 旋转 MODEL_YAW）：网格/骨架可视化挂此处；骨骼层级与蒙皮数学留在 scene（数据空间） */
  private modelRoot = new THREE.Group();
  /** 场景网格地板（视图菜单可隐藏） */
  private grid = new THREE.GridHelper(600, 60, 0x2a3550, 0x1a2033);
  private camera: THREE.PerspectiveCamera;
  private renderer: THREE.WebGLRenderer;
  private controls: OrbitControls;
  private parts = new Map<string, PartObj>();
  private container: HTMLElement;
  private mode: RenderMode = "shaded";
  private raf = 0;
  /** 渲染冻结开关（holdRender）：true 时渲染循环跳过 draw */
  private renderHold = false;
  private ro: ResizeObserver;
  private texCache = new Map<string, THREE.DataTexture>();
  private onPick: (path: string) => void = () => {};

  // ---- fmath wasm 内核（模型计算迁入 Rust；gfmEnabled=false 走 TS 内置路径）----
  /** 上次重绑的 wasm 视图版本（register/unregister 自增后需全量重取视图） */
  private gfmViewsVer = -1;
  /** 世界矩阵平表上传暂存（n×16，f64） */
  private gfmWorldFlat: Float64Array | null = null;
  private _skirtRecord = false;

  // ---- FSKEL 骨架 ----
  private skelBones: THREE.Bone[] = [];
  private skelRoot: THREE.Bone | null = null;
  private skelHelper: THREE.SkeletonHelper | null = null;
  /** 自建可拾取骨线条（每段=父→骨，segment i 对应 boneSegOf[i]）：
   *  点击显示骨名+整段变色（SkeletonHelper 无法按段取色/拾取） */
  private boneLines: THREE.LineSegments | null = null;
  private boneSegOf: number[] = [];
  /** 骨线条基色 / 选中高亮色 */
  private static readonly BONE_BASE = new THREE.Color(0x5fd0a8);
  private static readonly BONE_HI = new THREE.Color(0xff4d4d);
  /** 选中的骨骼索引（-1 = 无）；点击骨线条/关节/检查器列表联动 */
  selectedBone = -1;
  /** 骨骼选中回调（useActMode 注册，驱动检查器列表联动） */
  onBonePick: (bone: number) => void = () => {};
  private boneLabelEl: HTMLDivElement | null = null;
  private jointMesh: THREE.InstancedMesh | null = null;
  /** 骨架可视化（线条 + 关节小球）显隐，跨动作加载保持（默认关） */
  private skeletonVisible = false;
  /** 全部帧的扁平姿态：帧 × 骨 × 7 */
  private fskelPoses: Float32Array | null = null;
  private fskelBoneCount = 0;
  private qa = new THREE.Quaternion();
  private qb = new THREE.Quaternion();
  private tmpVec = new THREE.Vector3();
  /** 当前帧各骨世界矩阵（蒙皮数据源，applyFskelFrame 时刷新） */
  private worldMats: THREE.Matrix4[] = [];
  /** 帧 0（绑定姿态）各骨世界矩阵快照：骨骼挂接件的静态基准。挂接若取
   *  点击瞬间的姿态，头部俯仰经 Yawπ 共轭会留残差——不同时刻戴的帽贴合
   *  度有细微差别；固定绑定姿态后渲染结果与挂接时刻无关（等价引擎逐帧
   *  重算 boneW(t)·offset），日志数值也可复现 */
  private bindWorldMats: THREE.Matrix4[] = [];
  /** 各骨共识绑定拟合（boneLocalPos→pos，跨部件多数分簇）。authored pos
   *  与多数部件摆放不一致的部件（Girl_hair05×0x6）静态模式下按此渲染
   *  绑定姿态，与头部对齐；一致部件保持 authored 原样。 */
  private bindMats: THREE.Matrix4[] = [];

  // ---- 裙摆布料骨（Bone* 链）----
  /** 骨架父索引（worldsFromPoses 组合世界矩阵用） */
  private skelParents: number[] = [];
  /** 镜像骨：FSKEL 帧记录尾部 1B 标志=1 的骨（引擎骨记录 +52，含 Bone*
   *  链根、手指/脚趾 Nub、tag_LWeapon 等挂点骨）。数据驱动（mirrorFlagsFirst，
   *  fskel.rs 直接解析）；无标志数据时 detectMirrorBones 回退残差推导 */
  private mirrorBones = new Set<number>();
  /** 每骨镜像标志（首帧，来自 FskelMeta.mirrorFirst）；null = 数据不可用 */
  private mirrorFlagsFirst: number[] | null = null;
  /** 帧间镜像标志翻转的骨（挂点类）：骨索引 → 逐帧标志 */
  private mirrorUnstable = new Map<number, number[]>();
  private mirrorScale = new THREE.Vector3(-1, -1, -1);
  /** 布料骨：Bone* 布料链成员（见 detectCloth） */
  private clothBones = new Set<number>();
  /** 每帧布料合成矩阵（弹簧模拟驱动，updateClothMats 刷新） */
  private clothMats = new Map<number, THREE.Matrix4>();
  /** 布料链载体骨索引（链根的公共父骨，裙摆 = 骨盆），无布料骨时 -1 */
  private clothCarrier = -1;

  // ---- 静态错位链锚定（B 类修复：qm 系裙摆飞出）----
  /** 判定为静态错位链的链根（diagnoseAction 时由 diagnoseClothChains 填写） */
  private anchoredRoots = new Set<number>();
  /** 锚定链数据：载体骨 + 载体绑定逆 + 各成员绑定拟合 */
  private anchoredChains = new Map<
    number,
    { carrier: number; tpInv: THREE.Matrix4; bindOf: Map<number, THREE.Matrix4> }
  >();
  /** 锚定骨 → 合成矩阵 A = 载体携带 × 绑定拟合（updateAnchoredMats 每帧刷新，
   * skinPart 用 boneLocalPos 路径加权，混合顶点同样正确） */
  private anchoredMats = new Map<number, THREE.Matrix4>();
  /** 布料弹簧模拟开关（默认关）。F3D.dll 反汇编确证引擎无运行时布料：
   * 游戏对 Bone* 链直接回放 FSKEL 烘焙动画（InvalidateRenderData 帧插值
   * +父级合成，无物理步；骨骼 API 仅 GetBoneSpace 等只读）。默认走与
   * 游戏一致的精确 LBS（链骨矩阵直接来自 FSKEL 姿态，条带随舞蹈等动作
   * 里的烘焙链动画摆动）；开启后 detectCloth/updateClothMats 的次级运动
   * 模拟接管链骨渲染（"比游戏更好"的增强，走路时裙摆滞后摆动）。
   * 无论开关，applySkirtDePen 的去穿透 pass 恒开兜底裙片-腿穿模。
   */
  private clothSimEnabled = false;
  /** 载体数据绑定拟合的逆（骨盆绑定系 ↔ 世界） */
  private clothTpInv: THREE.Matrix4 | null = null;
  /** 每根布料骨的次级运动模拟状态 */
  private clothSims: {
    bone: number;
    /** 锚点/弹簧目标（骨盆绑定系）：顶点簇上端质心 / 全簇质心 */
    anchor: THREE.Vector3;
    target: THREE.Vector3;
    /** 模拟点（世界坐标，NaN = 待重置） */
    pos: THREE.Vector3;
    prev: THREE.Vector3;
  }[] = [];
  /** 上次 applyFskelFrame 的绝对帧（i0+alpha），检测 seek */
  private clothLastAbs = NaN;

  // ---- 裙摆防穿模（恒开，与布料模拟开关无关）----
  /** 裙片几何顶点（受 Bone* 链影响的，含接缝克隆）：part + inflOf 下标 */
  private skirtVerts: { p: PartObj; i: number }[] = [];
  /** 裙片三角形（任一顶点受链影响）：面片跨穿（裙×腿三角形相交）推出用 */
  private skirtTris: { p: PartObj; a: number; b: number; c: number }[] = [];
  /** 腿网格三角形（无链影响的 Thigh/Calf/Foot 权重三角形，含靴）：
   *  裙三角形-腿三角形相交测试的靶集——真实网格判定，无胶囊半径两难 */
  private skirtLegTris: { p: PartObj; a: number; b: number; c: number }[] = [];
  /** 躯干三角形（既无链也无腿影响，含腰带/上衣/骨盆）：动态接触跟随
   *  （防贴合开缝）的目标池——腿三角形不参与，推离腿正是去穿透目的 */
  private skirtBodyTris: { p: PartObj; a: number; b: number; c: number }[] = [];
  /** 数据采集开关（cdp/collect-frames.mjs）：开启时 applySkirtDePenVerts
   *  把每次推出记入 skirtPushLog（帧开头清空；关闭时零开销）。
   *  phase 1=顶点胶囊 / 2=跨穿 tri-tri / 3=复扫 */
  get skirtRecord(): boolean {
    return this._skirtRecord;
  }
  set skirtRecord(on: boolean) {
    this._skirtRecord = on;
    if (gfmEnabled) gfm!.set_skirt_record(on);
  }
  skirtPushLog: { v: number; phase: number; k: number; cap?: string }[] = [];
  /** 推出量时间平滑（"细微形变"修复）：无状态推出在相交起止边界
   *  0↔2.6~3.4 瞬跳（collect-frames 帧间抖动实测）。每顶点存上帧位移
   *  向量做入快出慢 lerp（布料式）；seek/部件变更重置；平滑后顶点若
   *  仍在胶囊内立即回退满推出（顶点级正确性不妥协，跨穿残余仅 1 帧
   *  且 ≤25%） */
  private skirtPrevDisp = new Float64Array(0);
  private skirtOrigSnap = new Float64Array(0);
  private skirtPrevAbs = NaN;
  /** 位移场空间松弛（尖角修复）：skirtVerts 邻接表（三角形边 + 拆缝克隆对
   *  互为邻居，防接缝撕开）与双缓冲。阶段二按命中三角形整面外推，未命中
   *  面的邻居零位移，同条布带位移差 2.5+ → 布面翻折尖角（run9 v29/30
   *  外缘常驻尖角、475-477 v36 反向尖谷实测） */
  private skirtAdj: number[][] = [];
  private skirtRelaxA = new Float64Array(0);
  private skirtRelaxB = new Float64Array(0);
  /** 平滑系数：入快（2 帧到 ~94%）、出慢（布料回弹感） */
  private static readonly SKIRT_SMOOTH_UP = 0.75;
  private static readonly SKIRT_SMOOTH_DOWN = 0.3;
  /** 位移场空间松弛系数（2 轮邻域均值；仿真把最大邻域差分 2.36→0.52） */
  private static readonly SKIRT_RELAX_R = 0.5;
  /** 动态接触跟随：裙顶边推离腿时把位移传给贴合躯干面的比例（余下
   *  0.25 为缝上限 ~0.3，布料厚度内不可见） */
  private static readonly SKIRT_FOLLOW = 0.75;
  /** 接触跟随判据：LBS 基准下距躯干三角形近于此值视为贴合（非焊接） */
  private static readonly SKIRT_WELD_D = 0.6;
  /** 接触跟随判据：位移使距离增大超过此值才算被拉开 */
  private static readonly SKIRT_WELD_SEP = 0.25;
  /** 腿部碰撞胶囊：骨索引 + 绑定段长（骨原点→子骨方向，蒙皮随动） */
  private legCaps: { bone: number; len: number }[] = [];
  /** 去穿透目标间隙（胶囊半径之上的余量） */
  private static readonly SKIRT_MARGIN = 0.25;
  /** 腿网格口径收口：胶囊粗筛命中后再验腿网格最近距离，距网格面
   *  小于它才真推。真实腿面比 0.18 系数胶囊瘦 ~1.6，纯胶囊口径会把
   *  贴合腿面的裙顶边（裙腰贴腿根 0.5~0.6，设计如此）推离腿面，
   *  沿贴合线开缝（wlk4 105-112 裙腰横缝根因；与跨穿推出的胶囊
   *  误伤同源，见阶段二注释） */
  private static readonly SKIRT_MESH_MARGIN = 0.15;
  /** 单顶点单帧最大推出量（防动作数据极端穿透撕裂网格） */
  private static readonly SKIRT_MAX_PUSH = 6;
  /** 近静止链的锚定门槛：|纯LBS−锚定摆放| 帧平均失配超过它才视为链数据
   *  错位（qm 系实测 60~123）；正常动作的链静态姿态相对绑定摆放失配
   *  最大 ~15（std8 后中裙片），纯 LBS 不穿腿——游戏即按纯 LBS 渲染 */
  private static readonly ANCHOR_MISMATCH_MIN = 25;
  /** 长发合成件与身体网格的最小间隙（体表外推目标距离） */
  private static readonly HAIR_BODY_MARGIN = 1.5;
  private tmpScale = new THREE.Vector3(1, 1, 1);
  // 模拟参数：弹簧 K（s⁻²）、速度阻尼（上扬加重/下落轻放，借鉴 AniCloth_SW
  // 各向异性空气阻尼：y↑3.2 vs y↓0.4）、重力（u/s²，模型身高 ~160）
  private static readonly CLOTH_K = 260;
  private static readonly CLOTH_DAMP = 0.9;
  private static readonly CLOTH_DAMP_UP = 0.78;
  private static readonly CLOTH_DAMP_DOWN = 0.92;
  private static readonly CLOTH_G = 300;
  /** 腿部胶囊半径系数（×段长）：实测大腿网格面 ≈0.145×段长，取 0.18 含余量 */
  private static readonly CLOTH_LEG_R = 0.18;
  /** 帽子挂接偏移（骨局部系；经 fix 节点双层 Yaw 抵消后所见方向：
   *  X=沿骨轴上下、Y=左右、Z=前后(−Z 为脸侧)）。仅对**无 tag_Hat 挂点骨的
   *  骨架**的回落挂法生效（挂普通 head 骨）：头部装备模型自带头发
   *  （替换原发型，时装同理），锚点对齐头发区域：FM01 发丝与基础发上下缘
   *  贴齐实测 lift≈7（hat_probe），花环帽实测需 10；前后原点对位差≈3；
   *  左右 0.5 微调。由面板偏移实测值 (3,0.5,1) 折算固化 */
  private static readonly HAT_OFFSET = new THREE.Vector3(10, -1.5, -3);
  /** tag 挂点直挂的锚点补偿（fix/面板帧）：tag_Hat 骨原点相对游戏实际渲染
   *  挂点的残差，由用户面板实测折算固化。面板偏移在此基础上微调（+） */
  private static readonly TAG_MOUNT_OFFSET = new THREE.Vector3(-1.2, 0.1, 5.6);
  // 每帧复用的临时对象
  private cCarry = new THREE.Matrix4();
  private cAnchor = new THREE.Vector3();
  private cTarget = new THREE.Vector3();
  private cVel = new THREE.Vector3();
  private cDir1 = new THREE.Vector3();
  private cT1 = new THREE.Matrix4();
  /** 链锚定的携带矩阵临时量（updateAnchoredMats 用） */
  private cAnchorM = new THREE.Matrix4();

  // ---- FAK 顶点动画 ----
  private fakModel: FakModel | null = null;
  private fakMeshes: THREE.Mesh[] = [];
  private fakGeoms: THREE.BufferGeometry[] = [];
  private fakMats: THREE.MeshStandardMaterial[] = [];
  /** fakMats 同位：各子网格引用的材质序号（多节可共用同一材质） */
  private fakMatIdx: number[] = [];
  /** 所有子网格共享的顶点属性（位置/法线逐帧更新，UV 恒定） */
  private fakPos: THREE.BufferAttribute | null = null;
  private fakNrm: THREE.BufferAttribute | null = null;

  // ---- FSM 静态武器网格 ----
  private fsmModel: FsmModel | null = null;
  private fsmMeshes: THREE.Mesh[] = [];
  private fsmGeoms: THREE.BufferGeometry[] = [];
  private fsmMats: THREE.MeshStandardMaterial[] = [];
  /** fsmMats 同位：各子网格引用的材质序号 */
  private fsmMatIdx: number[] = [];
  /** 挂点标记（小球），mesh/材质与 fsmMeshes 分开管理 */
  private fsmDummyGroup: THREE.Group | null = null;
  private fsmDummyGeoms: THREE.SphereGeometry[] = [];
  private fsmDummyMats: THREE.Material[] = [];

  // ---- SFX 特效播放器（按节点 prop 偏移真实摆放 + 路径动画）----
  private sfxGroup: THREE.Group | null = null;
  /** 各贴图节点一个展示体：Quad/未知 = 相机朝向 Sprite，Cross = 交叉面片组
   * （ground 追加水平片），Cylinder = 程序管网格（引擎 sub_10087E60 /
   * sub_100898F0 同构）；node = 节点序 */
  private sfxBoards: {
    node: number;
    obj: THREE.Object3D;
    mat: THREE.SpriteMaterial | THREE.MeshBasicMaterial;
    /** 解析缓存贴图与其展示克隆（含 U×V 分帧 UV 变换）；null = 未解析到 */
    baseTex: THREE.Texture | null;
    tex: THREE.Texture | null;
    /** U×V 帧网格（[1,1] = 整图单帧）与引擎翻页参数 */
    grid: [number, number];
    fps: number;
    hold: boolean;
    /** 节点显隐（检查器逐节点开关） */
    on: boolean;
    /** 节点偏移（板位 = basePos + 路径积分位置） */
    basePos: THREE.Vector3;
    /** 引用的路径名（"" = 定点） */
    pathName: string;
    /** 标称半高（标签摆放） */
    h: number;
    /** Quad/Cross 状态关键帧（插值动画；空 = 静态）与动画游标 */
    states: SfxKeyState[];
    lastT: number;
    rotAcc: number;
    /** 构建期基准尺寸（插值尺寸的回退/缩放基准） */
    baseScale: [number, number];
    label: THREE.Sprite;
    labelTex: THREE.CanvasTexture;
  }[] = [];
  /** 路径名 → 路径对象+模拟器（同名共享；setSfx 重建） */
  private sfxMotions = makeMotionMap([]);

  // ---- 挂接特效（角色叠加播放：挂骨跟随 / 世界放置；与上方 SFX 预览模式互不影响）----
  /** 每实例一个 Group 挂 modelRoot（matrixAutoUpdate=false，矩阵由 updateFxMatrices 重算） */
  private fxItems = new Map<number, FxItemObj>();
  private fxTmpA = new THREE.Matrix4();
  private fxTmpB = new THREE.Matrix4();
  private fxTmpQ = new THREE.Quaternion();
  private fxTmpE = new THREE.Euler();
  private fxTmpV = new THREE.Vector3();
  private fxTmpS = new THREE.Vector3();
  /** 挂接特效公告板标称高度（世界单位；角色身高 ~170，武器光效观感标定） */
  private static readonly FX_BOARD_H = 48;

  // ---- 诊断日志（穿模/显示异常排查，所有日志开头统一为 "动作名: "）----
  /** 当前动作名（诊断日志前缀），useActMode 在加载/切换动作时设置 */
  private actName: string | null = null;
  /** 上次镜像检测结果签名（集合变化才重打明细；addPart/removePart 会反复检测） */
  private lastMirrorSig = "";
  /** 播放期累计诊断，applyFskelFrame 按 30 个新帧窗口经 diagFlush 输出 */
  private diag = {
    /** 上次计入的帧号（scrub 反复应用同帧不重复计数） */
    i0: -1,
    frames: 0,
    /** 裙摆去穿透：窗口内最大推出需求量与所在腿骨 */
    maxNeed: 0,
    maxBone: -1,
    /** 推出量被 SKIRT_MAX_PUSH 钳制的顶点次（余量部分仍穿腿） */
    clamp: 0,
    /** 布料模拟（仅 clothSimEnabled）事件计数 */
    clothSeek: 0,
    clothVelClamp: 0,
    clothSafeClamp: 0,
    clothLagClamp: 0,
  };

  constructor(container: HTMLElement) {
    this.container = container;
    // CDP 诊断后门：node drive.mjs 可直接查询内部状态（如去穿透残余量）
    (window as unknown as { __viewer?: Viewer }).__viewer = this;
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    container.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(50, 1, 0.5, 5000);
    // 初始相机与 frameBox 同方位（+Z 侧、略俯视），加载模型前后视角一致
    this.camera.position.set(0, 85, 0).addScaledVector(INIT_VIEW_DIR, 380);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.target.set(0, 85, 0);

    this.scene.background = new THREE.Color(0x000000);
    this.modelRoot.rotation.y = MODEL_YAW;
    this.scene.add(this.modelRoot);
    const hemi = new THREE.HemisphereLight(0xdde6ff, 0x30281e, 1.15);
    this.scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 1.6);
    dir.position.set(150, 250, 180);
    this.scene.add(dir);
    const dir2 = new THREE.DirectionalLight(0x8899ff, 0.5);
    dir2.position.set(-160, 120, -140);
    this.scene.add(dir2);

    this.scene.add(this.grid);
    const axis = new THREE.AxesHelper(60);
    axis.position.y = 0.2;
    this.scene.add(axis);

    this.ro = new ResizeObserver(() => this.resize());
    this.ro.observe(container);
    this.resize();

    this.renderer.domElement.addEventListener("pointerdown", (e) => {
      const sx = e.clientX,
        sy = e.clientY;
      const up = (e2: PointerEvent) => {
        window.removeEventListener("pointerup", up);
        if (Math.abs(e2.clientX - sx) + Math.abs(e2.clientY - sy) < 5) {
          this.pick(e2.clientX, e2.clientY);
        }
      };
      window.addEventListener("pointerup", up);
    });

    const loop = () => {
      this.raf = requestAnimationFrame(loop);
      this.controls.update();
      // 批量装配期间冻结渲染（holdRender），完成后再整体呈现，避免逐件加载的中间态
      if (!this.renderHold) this.renderer.render(this.scene, this.camera);
    };
    loop();
  }

  /** 冻结/恢复渲染循环：换基础模型等批量装配时按序 true→false 配对使用 */
  holdRender(on: boolean) {
    this.renderHold = on;
  }

  setPickHandler(fn: (path: string) => void) {
    this.onPick = fn;
  }

  private pick(cx: number, cy: number) {
    const rect = this.renderer.domElement.getBoundingClientRect();
    const ndc = new THREE.Vector2(
      ((cx - rect.left) / rect.width) * 2 - 1,
      -((cy - rect.top) / rect.height) * 2 + 1
    );
    const ray = new THREE.Raycaster();
    ray.setFromCamera(ndc, this.camera);
    // 骨架可见时优先拾取骨骼（线条 + 关节球）：命中即选中显示骨名并高亮
    if (this.skeletonVisible) {
      ray.params.Line.threshold = 1.2;
      const targets: THREE.Object3D[] = [];
      if (this.boneLines) targets.push(this.boneLines);
      if (this.jointMesh) targets.push(this.jointMesh);
      const bh = ray.intersectObjects(targets, false);
      if (bh.length) {
        const h = bh[0];
        const bone =
          h.object === this.jointMesh
            ? (h as unknown as { instanceId?: number }).instanceId ?? -1
            : this.boneSegOf[Math.floor((h.index ?? 0) / 2)] ?? -1;
        if (bone >= 0) {
          this.selectBone(bone);
          return;
        }
      }
    }
    const meshes = [...this.parts.values()].map((p) => p.mesh);
    const hits = ray.intersectObjects(meshes, false);
    if (hits.length) {
      for (const [path, p] of this.parts) {
        if (p.mesh === hits[0].object) {
          this.onPick(path);
          return;
        }
      }
    }
    this.deselectBone(); // 点空处清除骨骼选中
  }

  private resize() {
    const w = this.container.clientWidth || 1;
    const h = this.container.clientHeight || 1;
    this.renderer.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  /** mask=true 为 mask 类材质（pre[2]≠0xFF，MMAlphaMask 毛发/帽图集）变体：
   *  禁 mipmap——图集不透明暗色 art（发帘/藏青底衬）紧贴装饰岛，mip 混合把
   *  邻岛暗色混进羽翼边缘成黑线（FM38 实测禁 mip 全消；dilateMaskPadding
   *  只治 RGB=0 垫底，治不了暗色 art 渗色）。minFilter 退 Linear 无 mip，
   *  远距轻微闪烁可接受 */
  /** mask 类材质（pre[2]≠0xFF，引擎 MMAlphaMask）的 alpha-test 参照：
   *  引擎 CreateObj 硬编码 ALPHAREF=0xF0(240/255)，随淡入透明度缩放——
   *  高出切把边缘双线性混合的暗色像素全部裁掉（羽翼边黑线根因）。
   *  非 mask 材质维持 0.1（须低于 highlight 压暗 opacity 0.28） */
  private static readonly MASK_ALPHA_REF = 240 / 255;

  private baseAlphaTestOf(mask: boolean): number {
    return mask ? Viewer.MASK_ALPHA_REF : 0.1;
  }

  getTexture(file: string, tex: TexData, flipY = false, mask = false): THREE.DataTexture {
    const key = `${file}|${flipY ? "VF" : "N"}${mask ? "|M" : ""}`;
    let t = this.texCache.get(key);
    if (!t) {
      t = new THREE.DataTexture(
        Viewer.dilateMaskPadding(tex.rgba, tex.width, tex.height),
        tex.width,
        tex.height,
        THREE.RGBAFormat
      );
      t.flipY = flipY;
      // 游戏引擎用重复寻址，手臂等条带 UV 环绕 u=0/1 接缝，必须 Repeat
      t.wrapS = THREE.RepeatWrapping;
      t.wrapT = THREE.RepeatWrapping;
      t.magFilter = THREE.LinearFilter;
      t.minFilter = mask ? THREE.LinearFilter : THREE.LinearMipmapLinearFilter;
      t.generateMipmaps = !mask;
      t.needsUpdate = true;
      this.texCache.set(key, t);
    }
    return t;
  }

  /**
   * 图集垫底色扩张（BFS 最近不透明邻色填充 alpha=0 区的 RGB）：mip 链生成时
   * 透明垫底的 RGB=0 会被箱式滤波混进可见色——RGB 被稀释到≈0 而 alpha 仍
   * 高于 alphaTest 阈值，渲染成纯黑刃片（FM38 装饰黑刃实测根因；mip0 逐
   * 三角形采样 0 黑命中、禁 mip 后黑刃全消）。只改透明区 RGB，alpha 原样
   * 保留，mip0 采样与 alphaTest 行为不变
   */
  private static dilateMaskPadding(
    rgba: Uint8Array,
    w: number,
    h: number
  ): Uint8Array {
    const out = new Uint8Array(rgba);
    const n = w * h;
    const src = new Int32Array(n).fill(-1);
    const q = new Int32Array(n);
    let head = 0;
    let tail = 0;
    for (let i = 0; i < n; i++) {
      if (out[i * 4 + 3] >= 40) {
        src[i] = i;
        q[tail++] = i;
      }
    }
    while (head < tail) {
      const i = q[head++];
      const x = i % w;
      const y = (i / w) | 0;
      if (x > 0 && src[i - 1] < 0) {
        src[i - 1] = src[i];
        out.set(out.subarray(src[i] * 4, src[i] * 4 + 3), (i - 1) * 4);
        q[tail++] = i - 1;
      }
      if (x < w - 1 && src[i + 1] < 0) {
        src[i + 1] = src[i];
        out.set(out.subarray(src[i] * 4, src[i] * 4 + 3), (i + 1) * 4);
        q[tail++] = i + 1;
      }
      if (y > 0 && src[i - w] < 0) {
        src[i - w] = src[i];
        out.set(out.subarray(src[i] * 4, src[i] * 4 + 3), (i - w) * 4);
        q[tail++] = i - w;
      }
      if (y < h - 1 && src[i + w] < 0) {
        src[i + w] = src[i];
        out.set(out.subarray(src[i] * 4, src[i] * 4 + 3), (i + w) * 4);
        q[tail++] = i + w;
      }
    }
    return out;
  }

  /** 材质 pre 三字节判翻转类：pre[2]≠0xFF（引擎挂 MMAlphaMask 的毛发类）→ 需要 UV 岛修复 */
  private static texFlip(pre: number[] | undefined): boolean {
    return !!pre && pre.length > 2 && pre[2] !== 0xff;
  }

  /**
   * UV 岛修复（mask 类材质的图集行序混排，FM38 用户终裁）：只翻"恒等采样
   * 命中 authored 不透明纯黑垫底（alpha≥200 且 RGB≈0，任何 alphaTest 裁不
   * 掉）且翻转后命中减少"的连通簇——整簇 v→1-v。FM38 恰好只有 2×17 花件
   * 簇（恒等落右上紫花+黑垫底=黑块来源，翻转→绿岛绿外瓣紫红花心的圆花）。
   * 其余一切簇（绿羽扇宽簇按 authored 原样、金弧条带对、紫点簇、藏青底衬、
   * 发帘）恒等即正确——花件豁免/绿/金/邻接/条带/总括翻转全部经用户逐轮
   * 实拍否决撤除。
   */
  static repairMaskUvIslands(uv: Float32Array, tris: number[], tex: TexData): number {
    const w = tex.width;
    const h = tex.height;
    const px = tex.rgba;
    const texel = (u: number, v: number) => {
      const x = ((Math.floor(u * w) % w) + w) % w;
      const y = ((Math.floor(v * h) % h) + h) % h;
      return (y * w + x) * 4;
    };
    const nbr = new Map<number, number[]>();
    const push = (a: number, b: number) => {
      const l = nbr.get(a);
      if (l) l.push(b);
      else nbr.set(a, [b]);
    };
    for (let t = 0; t + 2 < tris.length; t += 3) {
      const a = tris[t], b = tris[t + 1], c = tris[t + 2];
      push(a, b); push(a, c); push(b, a); push(b, c); push(c, a); push(c, b);
    }
    const seen = new Set<number>();
    type Cl = { comp: number[]; idOp: number; idPurple: number; idGreen: number; flGreen: number; b0: number; b1: number };
    const cls: Cl[] = [];
    for (const seed of nbr.keys()) {
      if (seen.has(seed)) continue;
      const comp: number[] = [];
      const q = [seed];
      seen.add(seed);
      while (q.length) {
        const x = q.pop()!;
        comp.push(x);
        for (const y of nbr.get(x) ?? []) {
          if (!seen.has(y)) {
            seen.add(y);
            q.push(y);
          }
        }
      }
      const cs = new Set(comp);
      let idOp = 0, idPurple = 0, idGreen = 0, flGreen = 0, b0 = 0, b1 = 0;
      for (let t = 0; t + 2 < tris.length; t += 3) {
        if (!cs.has(tris[t])) continue;
        const a = tris[t], b = tris[t + 1], c = tris[t + 2];
        for (let k = 0; k <= 4; k++) {
          for (let m = 0; m + k <= 4; m++) {
            const q2 = 4 - k - m;
            const uu = (k * uv[a * 2] + m * uv[b * 2] + q2 * uv[c * 2]) / 4;
            const vv = (k * uv[a * 2 + 1] + m * uv[b * 2 + 1] + q2 * uv[c * 2 + 1]) / 4;
            const o = texel(uu, vv);
            if (px[o + 3] >= 150) {
              idOp++;
              const r = px[o], g = px[o + 1], b2 = px[o + 2];
              if (r > 60 && b2 > 50 && g < r * 0.72 && g < b2 * 0.8) idPurple++;
              if (g > r + 15 && g > b2 + 15) idGreen++;
              if (px[o + 3] >= 200 && r + g + b2 < 30) b0++;
            }
            const of2 = texel(uu, 1 - vv);
            if (px[of2 + 3] >= 150) {
              const r = px[of2], g = px[of2 + 1], b2 = px[of2 + 2];
              if (g > r + 15 && g > b2 + 15) flGreen++;
              if (px[of2 + 3] >= 200 && r + g + b2 < 30) b1++;
            }
          }
        }
      }
      cls.push({ comp, idOp, idPurple, idGreen, flGreen, b0, b1 });
    }
    let flippedVerts = 0;
    for (const cl of cls) {
      // 黑占比闸：恒等采样以真实彩色 art 为主的簇不翻——少量 authored 黑
      // 描边（FM38 耳饰盘簇实测 75% 紫盘 + 4% 黑描边）是合法 art，游戏按
      // authored UV 原样渲染 fm38.dds 的紫盘岛（引擎无任何 UV 修复）；
      // 只有恒等落点被黑垫底主导（≥50% 不透明采样为纯黑）才是行序错排
      const wantFlip = cl.b0 > 0 && cl.b1 < cl.b0 && cl.b0 * 2 >= cl.idOp;
      if (wantFlip) {
        for (const i of cl.comp) uv[i * 2 + 1] = 1 - uv[i * 2 + 1];
        flippedVerts += cl.comp.length;
      }
    }
    return flippedVerts;
  }

  /** 按材质分组子网格三角形（材质 index 即下标），供 UV 岛修复逐材质喂图 */
  private static trisByMaterial(model: FsmModel): Map<number, number[]> {
    const byMat = new Map<number, number[]>();
    for (const s of model.submeshes) {
      const mi = Math.min(s.index, model.materials.length - 1);
      const arr = byMat.get(mi) ?? [];
      for (const t of s.triangles) arr.push(t);
      byMat.set(mi, arr);
    }
    return byMat;
  }

  // ---------------- fmath wasm 桥 ----------------

  /**
   * wasm 视图重绑：register/unregister 会令 wasm 线性内存增长，旧 Float32Array
   * 视图随之分离——版本号变化时全量重取视图并换绑 position/normal attribute。
   * 仅在部件增删时发生，帧路径无分配。
   */
  private gfmRebindViews() {
    if (!gfmEnabled) return;
    const ver = gfm!.views_version();
    if (ver === this.gfmViewsVer) return;
    this.gfmViewsVer = ver;
    for (const p of this.parts.values()) {
      if (p.gfmId < 0) continue;
      const pos = new THREE.BufferAttribute(gfm!.pos_view(p.gfmId)!, 3);
      const nrm = new THREE.BufferAttribute(gfm!.nrm_view(p.gfmId)!, 3);
      pos.needsUpdate = true;
      nrm.needsUpdate = true;
      p.geom.setAttribute("position", pos);
      p.geom.setAttribute("normal", nrm);
    }
  }

  /** 当前帧世界矩阵平表上传（列主序 n×16；skin/depen/cloth 消费） */
  private gfmUploadWorldMats() {
    if (!gfmEnabled) return;
    const n = this.worldMats.length;
    if (!n) return;
    if (!this.gfmWorldFlat || this.gfmWorldFlat.length !== n * 16) {
      this.gfmWorldFlat = new Float64Array(n * 16);
    }
    for (let b = 0; b < n; b++) {
      this.gfmWorldFlat.set(this.worldMats[b].elements, b * 16);
    }
    gfm!.set_world_mats(this.gfmWorldFlat, this.fskelBoneCount || n);
  }

  /** 部件蒙皮输出落盘后标记 attribute 上传（skinPart / depen 共用） */
  private markPartAttrs(p: PartObj) {
    (p.geom.getAttribute("position") as THREE.BufferAttribute).needsUpdate = true;
    (p.geom.getAttribute("normal") as THREE.BufferAttribute).needsUpdate = true;
  }

  addPart(
    path: string,
    model: FskinModel,
    texes: (TexData | null)[],
    texFiles: (string | null)[],
    opts?: { uvSeamSplit?: boolean; texMask?: boolean[] }
  ) {
    if (this.parts.has(path)) this.removePart(path);

    const n = model.vertices.length;
    // 用可增长数组：跨缝三角形需要拆分出副本顶点
    const pos = new Array<number>(n * 3);
    const nrm = new Array<number>(n * 3);
    const uv = new Array<number>(n * 2);
    const boneCol = new Array<number>(n * 3);
    const uvCol = new Array<number>(n * 3);
    const clones: CloneVert[] = []; // 跨缝拆分产生的副本顶点
    // 几何顶点（含副本）的源法线与影响表：蒙皮时副本沿用源顶点数据
    const srcNrm = new Array<number>(n * 3);
    const inflOf: Influence[][] = new Array(n);
    const pushVert = (i: number, u: number, v: number) => {
      const vp = model.vertices[i];
      const c = boneColor(vp.influences[0]?.boneIndex ?? 0);
      pos.push(vp.pos[0], vp.pos[1], vp.pos[2]);
      nrm.push(vp.normal[0], vp.normal[1], vp.normal[2]);
      uv.push(u, v);
      boneCol.push(c.r, c.g, c.b);
      // 副本用调整后的 u/v 归一显示（Repeat 下采样值等价）
      uvCol.push(
        Math.max(0, Math.min(1, u)),
        Math.max(0, Math.min(1, v)),
        0.35
      );
      srcNrm.push(vp.normal[0], vp.normal[1], vp.normal[2]);
      inflOf.push(vp.influences);
      clones.push({
        src: i,
        du: Math.round(u - vp.uv[0]),
        dv: Math.round(v - vp.uv[1]),
      });
    };
    model.vertices.forEach((v, i) => {
      pos[i * 3] = v.pos[0]; pos[i * 3 + 1] = v.pos[1]; pos[i * 3 + 2] = v.pos[2];
      nrm[i * 3] = v.normal[0]; nrm[i * 3 + 1] = v.normal[1]; nrm[i * 3 + 2] = v.normal[2];
      uv[i * 2] = v.uv[0]; uv[i * 2 + 1] = v.uv[1];
      const c = boneColor(v.influences[0]?.boneIndex ?? 0);
      boneCol[i * 3] = c.r; boneCol[i * 3 + 1] = c.g; boneCol[i * 3 + 2] = c.b;
      uvCol[i * 3] = Math.max(0, Math.min(1, v.uv[0]));
      uvCol[i * 3 + 1] = Math.max(0, Math.min(1, v.uv[1]));
      uvCol[i * 3 + 2] = 0.35;
      srcNrm[i * 3] = v.normal[0]; srcNrm[i * 3 + 1] = v.normal[1]; srcNrm[i * 3 + 2] = v.normal[2];
      inflOf[i] = v.influences;
    });

    // 跨缝拆分：手臂等条带 UV 环绕 u=0/1 接缝（引擎用重复寻址）。
    // 三角形两端的顶点分处缝两侧时插值会横穿整张纹理，需复制顶点把 UV 归一到同侧。
    // 仅对平铺类纹理成立——图集类（帽子/长发 FSM 图集行序混排）大跨度三角
    // 是 authored 的合法跨度，拆分平移 ±1 会把采样弧改到环绕缝、采到别的
    // 图块（FM38 头右上装饰实测），FSM 合成部件须关（uvSeamSplit:false，
    // 与单独渲染 setFsm 同口径）。
    // 多子网格：各节三角形按序拼接为一个索引流，groups 记录每节的区间给材质分组。
    const splitUv = opts?.uvSeamSplit !== false;
    const tri: number[] = [];
    const groupRanges: Array<[number, number]> = [];
    for (const s of model.submeshes) {
      const start = tri.length;
      for (const t of s.triangles) tri.push(t);
      if (tri.length > start) groupRanges.push([start, tri.length - start]);
    }
    for (let k = 0; k < tri.length; k += 3) {
      const ia = tri[k], ib = tri[k + 1], ic = tri[k + 2];
      const us = [uv[ia * 2], uv[ib * 2], uv[ic * 2]];
      const vs = [uv[ia * 2 + 1], uv[ib * 2 + 1], uv[ic * 2 + 1]];
      const du = Math.max(...us) - Math.min(...us);
      const dv = Math.max(...vs) - Math.min(...vs);
      if (!splitUv || (du <= 0.5 && dv <= 0.5)) continue;
      const news = [ia, ib, ic].map((vi, m) => {
        let u = uv[vi * 2];
        let v = uv[vi * 2 + 1];
        let adj = false;
        if (du > 0.5) {
          while (u - us[0] > 0.5) { u -= 1; adj = true; }
          while (u - us[0] < -0.5) { u += 1; adj = true; }
        }
        if (dv > 0.5) {
          while (v - vs[0] > 0.5) { v -= 1; adj = true; }
          while (v - vs[0] < -0.5) { v += 1; adj = true; }
        }
        if (!adj) return vi;
        pushVert(vi, u, v);
        return uv.length / 2 - 1;
      });
      tri[k] = news[0]; tri[k + 1] = news[1]; tri[k + 2] = news[2];
    }

    // fmath wasm 注册：SoA 影响表 + 含跨缝副本映射的 authored 数据；
    // wasm 视图即几何 position/normal 缓冲（蒙皮/去穿透零拷贝直写）
    const srcNrmF = Float32Array.from(srcNrm);
    let gfmId = -1;
    if (gfmEnabled) {
      const vc = inflOf.length;
      const origN = model.vertices.length;
      const srcPos = new Float32Array(vc * 3);
      const resolved = new Uint32Array(vc);
      const totalInfl = inflOf.reduce((s, a) => s + a.length, 0);
      const off = new Uint32Array(vc + 1);
      const inflBone = new Uint32Array(totalInfl);
      const inflWeight = new Float32Array(totalInfl);
      const inflLocal = new Float32Array(totalInfl * 3);
      let t = 0;
      for (let i = 0; i < vc; i++) {
        off[i] = t;
        const si = i < origN ? i : clones[i - origN].src;
        resolved[i] = si;
        const vp = model.vertices[si];
        srcPos[i * 3] = vp.pos[0];
        srcPos[i * 3 + 1] = vp.pos[1];
        srcPos[i * 3 + 2] = vp.pos[2];
        for (const inf of inflOf[i]) {
          inflBone[t] = inf.boneIndex;
          inflWeight[t] = inf.weight;
          inflLocal[t * 3] = inf.boneLocalPos[0];
          inflLocal[t * 3 + 1] = inf.boneLocalPos[1];
          inflLocal[t * 3 + 2] = inf.boneLocalPos[2];
          t++;
        }
      }
      off[vc] = t;
      gfmId = gfm!.register_part(
        srcPos,
        srcNrmF,
        resolved,
        origN,
        off,
        inflBone,
        inflWeight,
        inflLocal
      );
    }

    const geom = new THREE.BufferGeometry();
    geom.setAttribute(
      "position",
      new THREE.BufferAttribute(gfmId >= 0 ? gfm!.pos_view(gfmId)! : Float32Array.from(pos), 3)
    );
    geom.setAttribute(
      "normal",
      new THREE.BufferAttribute(gfmId >= 0 ? gfm!.nrm_view(gfmId)! : Float32Array.from(nrm), 3)
    );
    geom.setAttribute("uv", new THREE.BufferAttribute(Float32Array.from(uv), 2));
    geom.setAttribute("color", new THREE.BufferAttribute(Float32Array.from(boneCol), 3));
    geom.setAttribute("color2", new THREE.BufferAttribute(Float32Array.from(uvCol), 3));
    geom.setIndex(tri);

    // 每子网格一个材质（护甲类多节网格各有贴图），groups 对应拼接区间。
    // mask 判据与引擎同口径：材质记录 pre[2]≠0xFF ⇒ MMAlphaMask 类按引擎
    // ALPHAREF=240 裁切（合成件路径由调用方显式传入；FSKIN 部件材质自带
    // 同款 pre 位）。基础阈值记入 userData 供 highlight 压暗后还原
    const mats: THREE.MeshStandardMaterial[] = model.submeshes.map((_s, k) => {
      const mask = opts?.texMask?.[k] ?? Viewer.texFlip(model.materials[k]?.pre);
      const base = this.baseAlphaTestOf(mask);
      const mat = new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        roughness: 0.75,
        metalness: 0.0,
        vertexColors: false,
        // 贴图镂空区（发丝/帽檐 alpha=0）按引擎同款 alpha-test 裁掉，
        // 否则 RGB=0 的透明 texel 渲染成黑块。基础阈值须低于 highlight 的
        // 压暗 opacity 0.28，避免未选中件被整件裁没
        alphaTest: base,
      });
      mat.userData.baseAlphaTest = base;
      const tex = texes[k] ?? null;
      const texFile = texFiles[k] ?? null;
      const md = model.materials[k];
      if (tex && texFile) {
        mat.map = this.getTexture(texFile, tex, false, mask);
        (mat as any).customColorAttribute = "color";
      }
      if (md) {
        mat.color.setRGB(md.diffuse[0], md.diffuse[1], md.diffuse[2]);
        mat.emissive.setRGB(md.emissive[0], md.emissive[1], md.emissive[2]);
      }
      return mat;
    });
    if (!mats.length) {
      // 理论上不出现（无节网格）：给个空材质兜底
      mats.push(new THREE.MeshStandardMaterial({ side: THREE.DoubleSide }));
    }
    for (let k = 0; k < groupRanges.length; k++) {
      const [start, count] = groupRanges[k];
      geom.addGroup(start, count, Math.min(k, mats.length - 1));
    }

    const mesh = new THREE.Mesh(geom, mats);
    mesh.userData.path = path;
    mesh.frustumCulled = false; // CPU 蒙皮逐帧变形，静态包围球不可靠
    this.modelRoot.add(mesh);
    const part: PartObj = {
      mesh, geom, mats, model, clones, overlayMesh: null, overlayMat: null,
      inflOf, srcNrm: srcNrmF,
      skinned: this.skelBones.length > 0,
      gfmId,
    };
    this.parts.set(path, part);
    this.gfmRebindViews(); // 注册可能使其他部件的旧视图分离（wasm 内存增长）
    // 动作模式已绑骨架时立即按当前姿态蒙皮；静态模式对齐离群部件
    if (this.skelBones.length) {
      this.detectMirrorBones(); // 镜像判定依赖顶点，部件增删后重跑
      this.setupSkirtRig();
      this.setupHairBodyGrid(); // 裙摆防穿模点集随部件变化
      this.detectCloth(); // 布料检测的点集随部件变化
      this.setupChainAnchors(); // 锚定链拟合随部件顶点变化
      this.diagnoseParts(part);
      this.skinPart(part);
      this.applySkirtDePenVerts();
    } else {
      this.applyStaticSkin();
    }
    this.applyMode(this.mode);
  }

  removePart(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    this.removeOverlay(p);
    this.modelRoot.remove(p.mesh);
    p.geom.dispose();
    for (const m of p.mats) m.dispose();
    this.parts.delete(path);
    if (gfmEnabled && p.gfmId >= 0) {
      gfm!.unregister_part(p.gfmId);
      this.gfmRebindViews();
    }
    if (this.skelBones.length) {
      this.detectMirrorBones(); // 镜像判定依赖顶点，部件增删后重跑
      this.setupSkirtRig();
      this.setupHairBodyGrid(); // 裙摆防穿模点集随部件变化
      this.detectCloth(); // 布料检测的点集随部件变化
      this.setupChainAnchors(); // 锚定链拟合随部件顶点变化
    } else {
      // 静态模式共识可能变化（如锚定部件被移除），重新对齐
      this.applyStaticSkin();
    }
  }

  has(path: string) {
    return this.parts.has(path);
  }

  setMode(mode: RenderMode) {
    this.mode = mode;
    this.applyMode(mode);
  }

  private applyMode(mode: RenderMode) {
    for (const p of this.parts.values()) {
      p.mats.forEach((mat, k) => {
        const md = p.model.materials[k];
        if (mode === "shaded") {
          mat.vertexColors = false;
          if (md) {
            mat.color.setRGB(md.diffuse[0], md.diffuse[1], md.diffuse[2]);
            mat.emissive.setRGB(md.emissive[0], md.emissive[1], md.emissive[2]);
          }
        } else {
          mat.vertexColors = true;
          mat.color.setRGB(1, 1, 1);
          mat.emissive.setRGB(0, 0, 0);
        }
        mat.needsUpdate = true;
      });
      if (mode !== "shaded") {
        p.geom.setAttribute(
          "color",
          p.geom.getAttribute(mode === "bones" ? "color" : "color2")
        );
      }
    }
  }

  setWireframe(on: boolean) {
    for (const p of this.parts.values()) for (const m of p.mats) m.wireframe = on;
    for (const m of this.fakMats) m.wireframe = on;
    for (const m of this.fsmMats) m.wireframe = on;
  }

  setGridVisible(on: boolean) {
    this.grid.visible = on;
  }

  /** 高亮选中部件：其余半透明（饰品叠层随之）。压暗 opacity 0.28 会把
   *  alpha 拉到 mask 类阈值（240/255≈0.94）之下致整件被裁没，压暗期间
   *  同步降到 0.05，恢复时还原各自基础阈值 */
  highlight(path: string | null) {
    for (const [k, p] of this.parts) {
      const sel = path === null || k === path;
      for (const m of p.mats) {
        m.transparent = !sel;
        m.opacity = sel ? 1.0 : 0.28;
        m.alphaTest = sel
          ? (m.userData.baseAlphaTest as number | undefined) ?? 0.1
          : 0.05;
        m.depthWrite = true;
        m.needsUpdate = true;
      }
      if (p.overlayMat) {
        p.overlayMat.opacity = sel ? 1.0 : 0.28;
        p.overlayMat.needsUpdate = true;
      }
    }
  }

  applyMaterial(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    p.mats.forEach((mat, k) => {
      const md = p.model.materials[k];
      if (!md) return;
      if (this.mode === "shaded") {
        mat.color.setRGB(md.diffuse[0], md.diffuse[1], md.diffuse[2]);
        mat.emissive.setRGB(md.emissive[0], md.emissive[1], md.emissive[2]);
      }
      mat.roughness = Math.max(0.05, Math.min(1, 1 - md.specPower / 60));
    });
  }

  /** 顶点编辑后同步几何（含跨缝拆分的副本顶点） */
  updateVertex(path: string, i: number) {
    const p = this.parts.get(path);
    if (!p) return;
    const v = p.model.vertices[i];
    const vcount = p.model.vertices.length;
    const apply = (ai: number, du: number, dv: number) => {
      const pos = p.geom.getAttribute("position") as THREE.BufferAttribute;
      pos.setXYZ(ai, v.pos[0], v.pos[1], v.pos[2]);
      const nrm = p.geom.getAttribute("normal") as THREE.BufferAttribute;
      nrm.setXYZ(ai, v.normal[0], v.normal[1], v.normal[2]);
      const uv = p.geom.getAttribute("uv") as THREE.BufferAttribute;
      uv.setXY(ai, v.uv[0] + du, v.uv[1] + dv);
    };
    apply(i, 0, 0);
    p.srcNrm.set(v.normal, i * 3);
    // wasm 源数据同步（含引用该源顶点的跨缝副本；显示位已由上方视图写入）
    if (gfmEnabled && p.gfmId >= 0) {
      gfm!.set_src_vert(
        p.gfmId, i,
        v.pos[0], v.pos[1], v.pos[2],
        v.normal[0], v.normal[1], v.normal[2]
      );
    }
    p.clones.forEach((c, t) => {
      if (c.src === i) {
        apply(vcount + t, c.du, c.dv);
        p.srcNrm.set(v.normal, (vcount + t) * 3);
      }
    });
    p.geom.getAttribute("position").needsUpdate = true;
    p.geom.getAttribute("normal").needsUpdate = true;
    p.geom.getAttribute("uv").needsUpdate = true;
    // 蒙皮激活时重算该部件，暂停时编辑也能立即生效
    if (p.skinned) this.skinPart(p);
  }

  /** 换主贴图：多子网格时替换首槽（配色族切换语义针对主材质） */
  refreshTexture(path: string, tex: TexData, texFile: string, matIndex = 0) {
    const p = this.parts.get(path);
    const mat = p?.mats[matIndex];
    if (!mat) return;
    // 换贴图保留原材质的 mask 态（禁 mip 变体），FSM 合成件刷新不回退渗色
    const mask = mat.map ? mat.map.generateMipmaps === false : false;
    mat.map = this.getTexture(texFile, tex, false, mask);
    mat.needsUpdate = true;
  }

  /** 清除贴图：matIndex >= 0 只清该材质槽，否则清全部 */
  clearTexture(path: string, matIndex = -1) {
    const p = this.parts.get(path);
    if (!p) return;
    for (const [k, m] of p.mats.entries()) {
      if (matIndex >= 0 && k !== matIndex) continue;
      m.map = null;
      m.needsUpdate = true;
    }
  }

  // ---------------- FSKEL 骨架 ----------------

  /** 构建骨骼树并载入全部帧姿态（世界矩阵 = 父 × T × R(Q)） */
  setFskel(meta: FskelMeta, poses: Float32Array) {
    this.clearFskel();
    this.fskelPoses = poses;
    this.fskelBoneCount = meta.boneCount;

    const n = meta.boneCount;
    this.skelBones = Array.from({ length: n }, () => new THREE.Bone());
    let root: THREE.Bone | null = null;
    meta.boneNames.forEach((name, i) => {
      this.skelBones[i].name = name;
      const p = meta.parents[i];
      if (p >= 0 && p < n) this.skelBones[p].add(this.skelBones[i]);
      else root = this.skelBones[i];
    });
    this.skelRoot = root;
    // 根骨骼可能不是 0 号，统一挂到场景
    if (this.skelRoot) {
      this.skelBones.forEach((b) => {
        if (b !== this.skelRoot && b.parent === null) this.skelRoot!.add(b);
      });
      this.scene.add(this.skelRoot);
    }

    // SkeletonHelper 只接受单一根 Object3D（内部遍历 children），不能传骨骼数组
    // 可拾取骨线条：每段对应一根骨（父→本骨），支持点击变色/显示名。
    // 与关节球共用骨序；root 骨无父不画线。
    {
      const segs: number[] = [];
      this.boneSegOf = [];
      for (let b = 0; b < n; b++) {
        const par = meta.parents[b];
        if (par >= 0 && par < n) {
          this.boneSegOf.push(b);
          segs.push(b, par);
        }
      }
      const g = new THREE.BufferGeometry();
      g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(segs.length * 3), 3));
      const col = new Float32Array(segs.length * 3);
      for (let i = 0; i < segs.length; i++) {
        col[i * 3] = Viewer.BONE_BASE.r;
        col[i * 3 + 1] = Viewer.BONE_BASE.g;
        col[i * 3 + 2] = Viewer.BONE_BASE.b;
      }
      g.setAttribute("color", new THREE.BufferAttribute(col, 3));
      this.boneLines = new THREE.LineSegments(
        g,
        new THREE.LineBasicMaterial({ vertexColors: true })
      );
      this.boneLines.visible = this.skeletonVisible;
      this.modelRoot.add(this.boneLines);
      this.deselectBone();
    }

    // 关节小球（按骨骼索引着色）
    this.jointMesh = new THREE.InstancedMesh(
      new THREE.SphereGeometry(1.4, 10, 8),
      new THREE.MeshStandardMaterial({ roughness: 0.4, metalness: 0.1 }),
      n
    );
    const color = new THREE.Color();
    for (let i = 0; i < n; i++) {
      this.jointMesh.setColorAt(i, color.copy(boneColor(i)));
    }
    this.jointMesh.instanceColor!.needsUpdate = true;
    this.jointMesh.visible = this.skeletonVisible;
    this.modelRoot.add(this.jointMesh);

    // CPU 蒙皮缓冲：各骨世界矩阵（法线静态，引擎不做法线蒙皮，无需绑定拟合）
    this.worldMats = Array.from({ length: n }, () => new THREE.Matrix4());
    this.skelParents = meta.parents;
    // 引擎确定性镜像标志（IDA sub_1004C290：骨记录 +52 随 FSKEL 帧数据携带）
    this.mirrorFlagsFirst =
      meta.mirrorFirst && meta.mirrorFirst.length === n ? meta.mirrorFirst : null;
    this.mirrorUnstable = new Map(
      (meta.mirrorUnstable ?? []).map((u) => [u.bone, u.flags])
    );
    if (gfmEnabled) {
      gfm!.set_poses(poses, meta.boneCount, Int32Array.from(meta.parents));
    }
    for (const p of this.parts.values()) p.skinned = true;
    const frameTotal = Math.max(Math.floor(poses.length / (n * 7)), 0);
    this.dlog(`FSKEL 已加载: ${n} 骨 · ${frameTotal} 帧 · ${meta.actions.length} 个动作 · 部件 ${this.parts.size}`);
    this.detectMirrorBones(true); // 镜像骨检测需部件顶点（无部件时跳过，addPart 会重跑）；骨架重载须全量重判
    this.setupSkirtRig();
      this.setupHairBodyGrid(); // 裙摆防穿模点集（腿胶囊 + 裙片顶点）
    this.detectCloth();
    this.setupChainAnchors(); // 链锚定随骨架重拟合：换骨架后部件仍在场时，
    // addPart 时按旧骨架拟合的锚定必须作废（错锚会拉歪上身/裙片）
    this.diagnoseParts();

    this.applyFskelFrame(0, 0, 0);
    this.bindWorldMats = this.worldMats.map((m) => m.clone()); // 帧 0 快照（挂接静态基准）
    this.refreshBoneAttached(); // 骨架加载/切换后按帧 0 位姿重挂骨骼挂接网格
    this.bindFxToSkeleton(); // 挂接特效按名重解析到新骨架
  }

  // ---- 诊断日志 ----

  /** 设置诊断日志动作名前缀（日志开头统一为 "动作名: "） */
  setLogAction(name: string | null) {
    this.actName = name;
  }

  private dlog(msg: string) {
    const line = `${this.actName ?? "未加载动作"}: ${msg}`;
    console.log(line);
    pushDiagLog("log", line);
  }

  private dwarn(msg: string) {
    const line = `${this.actName ?? "未加载动作"}: ${msg}`;
    console.warn(line);
    pushDiagLog("warn", line);
  }

  /** Bone* 布料辅助骨名判定（链检测/裙摆点集共用） */
  private isBoneHelperName(b: number): boolean {
    return /^Bone\d+$/.test(this.skelBones[b]?.name ?? "");
  }

  /** 链根 root 的 Bone* 链全部成员（含根；父是链成员的 Bone* 骨递归并入） */
  private boneChainMembers(root: number): Set<number> {
    const mem = new Set<number>([root]);
    let changed = true;
    while (changed) {
      changed = false;
      for (let c = 0; c < this.fskelBoneCount; c++) {
        if (!mem.has(c) && this.isBoneHelperName(c) && mem.has(this.skelParents[c])) {
          mem.add(c);
          changed = true;
        }
      }
    }
    return mem;
  }

  /**
   * 动作级诊断（加载/切换动作时调用一次，输出到 console，开头为
   * "动作名: "）：骨骼树完整性（孤儿骨）、帧间姿态突跳（抽搐/闪跳类
   * 显示异常）、布料链姿态与绑定数据一致性（裙摆错位/穿模的主要来源）。
   */
  diagnoseAction(from: number, to: number) {
    this.diagFlush();
    if (!this.fskelPoses || !this.skelBones.length) return;
    const n = this.fskelBoneCount;
    const stride = n * 7;
    const last = Math.max(Math.floor(this.fskelPoses.length / stride) - 1, 0);
    const f0 = Math.min(Math.max(from, 0), last);
    const f1 = Math.min(Math.max(to, f0 + 1), last + 1);
    this.dlog(`动作诊断: 帧 ${f0}..${f1 - 1}（${f1 - f0} 帧） · ${n} 骨 · 部件 ${this.parts.size}`);

    // 骨骼树完整性：父索引越界/自环的骨会被当孤儿挂到根下（位置丢失 → 显示异常）
    const orphans: string[] = [];
    for (let b = 0; b < n; b++) {
      const p = this.skelParents[b];
      if (p >= 0 && (p >= n || p === b)) orphans.push(`${this.skelBones[b].name}(#${b} 父#${p})`);
    }
    if (orphans.length) {
      this.dwarn(`骨骼树: ${orphans.length} 根非法父索引（孤儿挂根）: ${orphans.slice(0, 8).join(", ")}`);
    }

    // 帧间姿态突跳：相邻帧单骨局部平移 >10 单位
    const jumps: string[] = [];
    for (let f = f0 + 1; f < f1 && jumps.length < 6; f++) {
      for (let b = 0; b < n; b++) {
        const o0 = (f - 1) * stride + b * 7;
        const o1 = f * stride + b * 7;
        const d = Math.max(
          Math.abs(this.fskelPoses[o1] - this.fskelPoses[o0]),
          Math.abs(this.fskelPoses[o1 + 1] - this.fskelPoses[o0 + 1]),
          Math.abs(this.fskelPoses[o1 + 2] - this.fskelPoses[o0 + 2])
        );
        if (d > 10) {
          jumps.push(`帧${f - 1}→${f} ${this.skelBones[b].name}(#${b}) 平移跳 ${d.toFixed(1)}`);
          break;
        }
      }
    }
    if (jumps.length) this.dwarn(`帧间突跳(>10/帧, 最多列6): ${jumps.join("; ")}`);

    this.diagnoseClothChains(f0, f1);
  }

  /**
   * 布料链诊断：每条 Bone* 链在帧区间内的摆动幅度 + 精确 LBS 对绑定
   * 位置的偏移 + 近静止链的锚定失配。近乎静止且失配大 = 静态错位链
   * （游戏为运行时布料，viewer 精确 LBS 放错裙片位置 → 锚定绑定摆放
   * 兜底）；近乎静止且失配小 = 链静态姿态即数据意图，纯 LBS（游戏
   * 行为，残余穿透由恒开的去穿透 pass 兜底）；摆动大 = 链带烘焙动画
   * （偏移属动作位移，正常）。LBS↔绑定偏移本身不作判据：动作基准
   * 姿态偏离绑定站立位时，所有链都会"偏移大"。
   */
  private diagnoseClothChains(f0: number, f1: number) {
    const n = this.fskelBoneCount;
    this.anchoredRoots.clear();
    const roots: number[] = [];
    for (let b = 0; b < n; b++) {
      if (!this.isBoneHelperName(b)) continue;
      const p = this.skelParents[b];
      if (p < 0 || !this.isBoneHelperName(p)) roots.push(b);
    }
    if (!roots.length || !this.parts.size) {
      this.setupChainAnchors(); // 清空残留锚定
      return;
    }

    // 骨 → 所属链根；链 → 受链影响顶点（原始顶点，忽略拆缝副本）
    const chainOf = new Map<number, number>();
    for (const r of roots) for (const m of this.boneChainMembers(r)) chainOf.set(m, r);
    const vertsOf = new Map<number, Vertex[]>(roots.map((r) => [r, []]));
    for (const p of this.parts.values()) {
      if (p.syntheticHair || p.syntheticMount) continue;
      for (const v of p.model.vertices) {
        for (const inf of v.influences) {
          const r = chainOf.get(inf.boneIndex);
          if (r !== undefined) {
            vertsOf.get(r)!.push(v);
            break;
          }
        }
      }
    }

    const step = Math.max(1, Math.floor((f1 - f0) / 12));
    const stats = new Map<
      number,
      { verts: Vertex[]; offAvg: number; offMax: number; swingRot: number; swingPos: number }
    >();
    for (const [root, verts] of vertsOf) {
      if (!verts.length) continue;
      let baseDir: THREE.Vector3 | null = null;
      let basePos: THREE.Vector3 | null = null;
      let swingRot = 0; // 度（链根世界朝向相对区间首采样帧）
      let swingPos = 0;
      let offSum = 0;
      let offMax = 0;
      let offCnt = 0;
      for (let f = f0; f < f1; f += step) {
        const W = this.worldsFromPoses(f);
        const e = W[root].elements;
        const dir = new THREE.Vector3(e[0], e[1], e[2]);
        if (dir.lengthSq() < 1e-12) dir.set(1, 0, 0);
        dir.normalize();
        const pos = new THREE.Vector3(e[12], e[13], e[14]);
        if (!baseDir) {
          baseDir = dir.clone();
          basePos = pos.clone();
        } else {
          swingRot = Math.max(
            swingRot,
            (Math.acos(Math.min(1, Math.abs(baseDir.dot(dir)))) * 180) / Math.PI
          );
          swingPos = Math.max(swingPos, pos.distanceTo(basePos));
        }
        // 精确 LBS（引擎语义）结果 vs authored 绑定位置
        for (const v of verts) {
          let x = 0, y = 0, z = 0;
          for (const inf of v.influences) {
            if (inf.boneIndex >= n) continue;
            const M = W[inf.boneIndex].elements;
            const lp = inf.boneLocalPos;
            x += inf.weight * (M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12]);
            y += inf.weight * (M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13]);
            z += inf.weight * (M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]);
          }
          const d = Math.hypot(x - v.pos[0], y - v.pos[1], z - v.pos[2]);
          offSum += d;
          if (d > offMax) offMax = d;
          offCnt++;
        }
      }
      stats.set(root, {
        verts,
        offAvg: offSum / Math.max(offCnt, 1),
        offMax,
        swingRot,
        swingPos,
      });
    }

    // 近静止链（本动作无烘焙链动画）为锚定候选，锚定与否看
    // |纯LBS − 锚定摆放| 失配：两者都随载体（骨盆）运动，差值抵消身体
    // 位移、只隔离链姿态数据与绑定数据的脱节程度。
    const cand = new Set<number>();
    for (const [root, s] of stats) {
      if (s.swingRot < 15 && s.swingPos < 3) cand.add(root);
    }
    // 部件语义判据：时装/发型类部件（非裙槽——路径不含 Lower/Boot）的链
    // 默认 authored 基准。实测帧 0 逐影响失配无法区分：时装裙（FK38_Lower
    // 链89 avg12.1/链95 17.4）与 SZ_38 披发链（12.6）同区间——"对着非绑定
    // 姿态导出"是时装资产常态；裙的偏移在宽松裙容差内无害、头发的偏移在
    // 紧贴身体处穿模，只能按部件语义分流。裙槽部件（Lower/Boot，含基础
    // girl_Lower）保持纯 LBS 现状（std8/run9/qmw 多轮打磨行为不变），
    // 仍可经 mismatch>25 错位门槛锚定。隐藏部件（被时装顶替）不计。
    const skirtSlot = (path: string) => /lower|boot/i.test(path);
    const semanticAnchor = new Map<number, { skirt: boolean; hair: number }>();
    for (const root of vertsOf.keys()) {
      semanticAnchor.set(root, { skirt: false, hair: 0 });
    }
    for (const p of this.parts.values()) {
      if (p.mesh.visible === false || p.syntheticHair || p.syntheticMount) continue;
      const isSkirt = skirtSlot(p.model.path);
      for (const v of p.model.vertices) {
        for (const inf of v.influences) {
          const r = chainOf.get(inf.boneIndex);
          if (r === undefined) break;
          const e = semanticAnchor.get(r);
          if (!e) break;
          if (isSkirt) e.skirt = true;
          else e.hair++;
          break;
        }
      }
    }
    for (const [root, e] of semanticAnchor) {
      if (!e.skirt && e.hair > 0) this.anchoredRoots.add(root);
    }
    const mismOf = new Map<number, number>();
    if (cand.size) {
      const fits = this.fitAnchoredChains(cand);
      const carryM = new THREE.Matrix4();
      const am = new THREE.Matrix4();
      for (const [root, a] of fits) {
        const verts = stats.get(root)!.verts;
        let sum = 0;
        let cnt = 0;
        for (let f = f0; f < f1; f += step) {
          const W = this.worldsFromPoses(f);
          carryM.multiplyMatrices(W[a.carrier], a.tpInv);
          for (const v of verts) {
            let lx = 0, ly = 0, lz = 0, gx = 0, gy = 0, gz = 0;
            for (const inf of v.influences) {
              if (inf.boneIndex >= n) continue;
              const w = inf.weight;
              const lp = inf.boneLocalPos;
              const M = W[inf.boneIndex].elements;
              lx += w * (M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12]);
              ly += w * (M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13]);
              lz += w * (M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]);
              const bind = a.bindOf.get(inf.boneIndex);
              const E = bind ? am.multiplyMatrices(carryM, bind).elements : M;
              gx += w * (E[0] * lp[0] + E[4] * lp[1] + E[8] * lp[2] + E[12]);
              gy += w * (E[1] * lp[0] + E[5] * lp[1] + E[9] * lp[2] + E[13]);
              gz += w * (E[2] * lp[0] + E[6] * lp[1] + E[10] * lp[2] + E[14]);
            }
            sum += Math.hypot(lx - gx, ly - gy, lz - gz);
            cnt++;
          }
        }
        mismOf.set(root, sum / Math.max(cnt, 1));
      }
      for (const [root, m] of mismOf) {
        if (m > Viewer.ANCHOR_MISMATCH_MIN) this.anchoredRoots.add(root);
      }
    }

    for (const [root, s] of stats) {
      const name = this.skelBones[root]?.name ?? `#${root}`;
      const offTxt = `LBS↔绑定偏移 avg ${s.offAvg.toFixed(1)}/max ${s.offMax.toFixed(1)}`;
      const swingTxt = `摆动 ${s.swingRot.toFixed(1)}°/${s.swingPos.toFixed(1)}`;
      if (this.anchoredRoots.has(root)) {
        const sem = semanticAnchor.get(root);
        const semTxt = sem && !sem.skirt && sem.hair > 0 ? ` · 语义锚(${sem.hair}发顶点)` : "";
        const whyTxt = sem && !sem.skirt && sem.hair > 0
          ? "时装/发型部件链(authored 基准)"
          : `锚定失配 ${mismOf.get(root)?.toFixed(1) ?? "?"}(>${Viewer.ANCHOR_MISMATCH_MIN})`;
        this.dwarn(
          `布料链 ${name}(#${root}) 顶点${s.verts.length}: ${offTxt} · ${swingTxt}${semTxt} · ${whyTxt} ⇒ 已增量锚定(绑定摆放随载体+动画增量)`
        );
      } else if (mismOf.has(root)) {
        this.dlog(
          `布料链 ${name}(#${root}) 顶点${s.verts.length}: ${offTxt} · ${swingTxt} 近乎静止 · 锚定失配 ${mismOf.get(root)!.toFixed(1)}(≤${Viewer.ANCHOR_MISMATCH_MIN}) → 链静态姿态即数据意图, 纯LBS(游戏行为), 残穿由去穿透兜底`
        );
      } else if (s.offAvg > 5) {
        this.dlog(
          `布料链 ${name}(#${root}) 顶点${s.verts.length}: ${offTxt} · ${swingTxt} → 链带烘焙动画, 偏移属动作位移`
        );
      } else {
        this.dlog(
          `布料链 ${name}(#${root}) 顶点${s.verts.length}: ${offTxt} · ${swingTxt} → 与绑定一致`
        );
      }
    }
    this.setupChainAnchors();
  }

  /**
   * 静态错位链锚定初始化（B 类修复：qm 系裙摆飞出 60~123 单位）。
   * 每条锚定链记录：载体骨（链根父骨，裙摆=骨盆）的绑定拟合逆 + 各链骨的
   * 绑定拟合（boneLocalPos→authored pos）。渲染时成员骨矩阵换成
   * A_b = 载体当前世界 × Tp⁻¹ × M_bind_b（updateAnchoredMats 每帧刷新）——
   * 载体在绑定位姿时裙片精确贴 authored，载体运动时整体跟随；链自身的
   * FSKEL 姿态（错位的种子）不再参与。布料模拟开启时优先走模拟（clothMats）。
   * 锚定与否由 diagnoseClothChains 按失配门槛决定（ANCHOR_MISMATCH_MIN）。
   */
  private setupChainAnchors() {
    this.anchoredChains.clear();
    this.anchoredMats.clear();
    if (gfmEnabled) gfm!.clear_anchored();
    if (!this.anchoredRoots.size || !this.parts.size) return;
    for (const [root, a] of this.fitAnchoredChains(this.anchoredRoots)) {
      this.anchoredChains.set(root, a);
      for (const b of a.bindOf.keys()) this.anchoredMats.set(b, new THREE.Matrix4());
    }
    if (this.anchoredChains.size) {
      if (gfmEnabled) {
        // 拟合结果上传 wasm（skinPart 消费 wasm 侧 anchoredMats）
        const carriers: number[] = [];
        const tpInvs: number[] = [];
        const counts: number[] = [];
        const bones: number[] = [];
        const mats: number[] = [];
        for (const a of this.anchoredChains.values()) {
          carriers.push(a.carrier);
          tpInvs.push(...a.tpInv.elements);
          counts.push(a.bindOf.size);
          for (const [b, m] of a.bindOf) {
            bones.push(b);
            mats.push(...m.elements);
          }
        }
        gfm!.setup_anchored(
          this.fskelBoneCount || this.worldMats.length,
          Uint32Array.from(carriers),
          Float64Array.from(tpInvs),
          Uint32Array.from(counts),
          Uint32Array.from(bones),
          Float64Array.from(mats)
        );
      }
      this.updateAnchoredMats();
      this.dlog(
        `链锚定生效: ${[...this.anchoredChains.keys()]
          .map((r) => this.skelBones[r]?.name ?? `#${r}`)
          .join(", ")} → 裙片按绑定摆放随 ${this.skelBones[this.anchoredChains.values().next().value!.carrier]?.name ?? "?"} 移动`
      );
    }
  }

  /** 拟合一组链的锚定数据（载体 Tp⁻¹ + 各链骨绑定拟合）。setupChainAnchors
   * 与 diagnoseClothChains 的失配判定共用；无单影响点的中间骨拟合为单位阵
   * （无顶点引用它，矩阵不参与渲染）。 */
  private fitAnchoredChains(
    roots: Set<number>
  ): Map<number, { carrier: number; tpInv: THREE.Matrix4; bindOf: Map<number, THREE.Matrix4> }> {
    const out = new Map<number, { carrier: number; tpInv: THREE.Matrix4; bindOf: Map<number, THREE.Matrix4> }>();
    if (!this.parts.size) return out;
    const n = this.fskelBoneCount;
    const W0 = this.worldFromPosesCached(0);
    // 链级整体刚体修正 C：源 = 帧0 LBS 位置(W₀(b)·lp)，目标 = authored。
    // M_bind(b) = C·W₀(b)；锚定合成 A_b(t) = 载体(t)·W₀(载)⁻¹·C·W_t(b)——
    // 即"纯 LBS 动画结果整体刚体拉回绑定摆放"。逐骨 Kabsch 在发束类资产上
    // 因点近共线退化（SZ_38 披发实测 solo 残差 8~24）；链级 24~32 点沿长
    // 条发束散布良好。配套链 C≈I 自动退化为纯 LBS；极退化（polar 失败）
    // C=I 同样回落纯 LBS，不会更糟。
    for (const root of roots) {
      const carrier = this.skelParents[root];
      if (carrier < 0 || carrier >= n) continue;
      const members = this.boneChainMembers(root);
      const memberSet = new Set(members);
      const src: number[] = [];
      const dst: number[] = [];
      let np = 0;
      for (const p of this.parts.values()) {
        if (p.mesh.visible === false || p.syntheticHair || p.syntheticMount) continue;
        for (const v of p.model.vertices) {
          for (const inf of v.influences) {
            if (inf.boneIndex >= n || !memberSet.has(inf.boneIndex)) continue;
            const M = W0[inf.boneIndex].elements;
            const lp = inf.boneLocalPos;
            src.push(
              M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12],
              M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13],
              M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]
            );
            dst.push(v.pos[0], v.pos[1], v.pos[2]);
            np++;
          }
        }
      }
      if (!np) continue;
      const pairs = Array.from(
        { length: np },
        (_, i) => [[src[i * 3], src[i * 3 + 1], src[i * 3 + 2]], [dst[i * 3], dst[i * 3 + 1], dst[i * 3 + 2]]] as [number[], number[]]
      );
      let C = this.fitBindMatrix(pairs);
      // 拟合质量闸：链内前后发束等非刚体分布会让单一 C 拟合出错误旋转
      // （SZ_38 链101 实测 b104 偏 31.6，比不修还差）。C 自身残差过大时弃用，
      // 回退 C=I = 纯 LBS（链 89 残差 1.5 保留；链 101 残差 ~12 回退）。
      let cres = 0;
      const Ce = C.elements;
      for (let i = 0; i < np; i++) {
        const x = src[i * 3], y = src[i * 3 + 1], z = src[i * 3 + 2];
        cres += Math.hypot(
          Ce[0] * x + Ce[4] * y + Ce[8] * z + Ce[12] - dst[i * 3],
          Ce[1] * x + Ce[5] * y + Ce[9] * z + Ce[13] - dst[i * 3 + 1],
          Ce[2] * x + Ce[6] * y + Ce[10] * z + Ce[14] - dst[i * 3 + 2]
        );
      }
      cres /= np;
      if (cres > 8) C = new THREE.Matrix4();
      const bindOf = new Map<number, THREE.Matrix4>();
      for (const b of members) {
        bindOf.set(b, new THREE.Matrix4().multiplyMatrices(C, W0[b]));
      }
      const w0carrier = this.bindWorldMats[carrier];
      out.set(root, {
        carrier,
        tpInv: w0carrier
          ? w0carrier.clone().invert()
          : this.fitBindMatrix([]).invert(),
        bindOf,
      });
    }
    return out;
  }

  /** worldsFromPoses 帧缓存（fitAnchoredChains 逐链多次调用时复用） */
  private worldsFrameCache = new Map<number, THREE.Matrix4[]>();
  private worldFromPosesCached(frame: number): THREE.Matrix4[] {
    let w = this.worldsFrameCache.get(frame);
    if (!w) {
      w = this.worldsFromPoses(frame);
      this.worldsFrameCache.set(frame, w);
    }
    return w;
  }

  /** 每帧刷新增量锚定合成矩阵（实例复用）：
   *  A_b(t) = 载体(t) × Tp⁻¹ × M_bind_b × (W₀(b)⁻¹ · W_t(b))
   *  基准贴 authored 绑定摆放随载体，链自身 FSKEL 动画以帧 0 骨姿态为
   *  原点叠加（配套链上 M_bind≈W₀ 退化为纯 LBS；脱节链拉回绑定摆放） */
  private updateAnchoredMats() {
    if (gfmEnabled) {
      this.gfmUploadWorldMats();
      gfm!.update_anchored();
      return;
    }
    if (!this.anchoredChains.size || !this.worldMats.length) return;
    const w0 = this.bindWorldMats;
    const delta = new THREE.Matrix4();
    for (const a of this.anchoredChains.values()) {
      const carrier = this.worldMats[a.carrier];
      if (!carrier) continue;
      this.cAnchorM.multiplyMatrices(carrier, a.tpInv);
      for (const [b, bind] of a.bindOf) {
        const out = this.anchoredMats.get(b)!;
        out.multiplyMatrices(this.cAnchorM, bind);
        const base = w0[b];
        const wt = this.worldMats[b];
        if (base && wt) {
          delta.multiplyMatrices(base.clone().invert(), wt);
          out.multiply(delta);
        }
      }
    }
  }

  /**
   * 部件权重诊断（骨架就位后）：引用骨架外骨骼的顶点与权重和≤0 的顶点
   * 在 skinPart 中保持绑定姿势，播放时不随骨骼运动（悬空/滞留类显示
   * 异常）。single 给定时只查该部件（addPart 时），否则全部（setFskel 时）。
   */
  private diagnoseParts(single?: PartObj) {
    const n = this.fskelBoneCount;
    if (!n) return;
    const targets: PartObj[] = single ? [single] : [...this.parts.values()];
    for (const p of targets) {
      let oob = 0;
      let zero = 0;
      const oobBones = new Set<number>();
      for (const v of p.model.vertices) {
        let ws = 0;
        let hasOob = false;
        for (const inf of v.influences) {
          if (inf.boneIndex >= n) {
            hasOob = true;
            oobBones.add(inf.boneIndex);
          } else {
            ws += inf.weight;
          }
        }
        if (hasOob) oob++;
        if (ws <= 0) zero++;
      }
      if (oob || zero) {
        const name = p.model.path.split(/[\\/]/).pop() ?? p.model.path;
        const bits: string[] = [];
        if (oob) bits.push(`${oob} 顶点引用骨架外骨骼 ${[...oobBones].join(",")}`);
        if (zero) bits.push(`${zero} 顶点权重和≤0`);
        this.dwarn(`部件 ${name}: ${bits.join(" · ")} → 保持绑定姿势, 播放时不随骨骼动`);
      }
    }
  }

  /** 输出并重置播放期累计诊断（裙摆去穿透/布料模拟），窗口内无事件则静默 */
  private diagFlush() {
    const d = this.diag;
    if (gfmEnabled) {
      // wasm 侧累计计数并入本窗口（TS 计数为 0，取 max 合并）
      d.maxNeed = Math.max(d.maxNeed, gfm!.diag_max_need());
      d.maxBone = gfm!.diag_max_bone();
      d.clamp = Math.max(d.clamp, gfm!.diag_clamp());
      d.clothSeek = Math.max(d.clothSeek, gfm!.diag_cloth_seek());
      d.clothVelClamp = Math.max(d.clothVelClamp, gfm!.diag_cloth_vel_clamp());
      d.clothSafeClamp = Math.max(d.clothSafeClamp, gfm!.diag_cloth_safe_clamp());
      d.clothLagClamp = Math.max(d.clothLagClamp, gfm!.diag_cloth_lag_clamp());
    }
    if (d.frames > 0 && (d.maxNeed > 0 || d.clamp > 0)) {
      const boneName =
        d.maxBone >= 0 ? this.skelBones[d.maxBone]?.name ?? `#${d.maxBone}` : "?";
      const bits = [`裙摆防穿模 近${d.frames}帧: 最大穿透 ${d.maxNeed.toFixed(2)}(腿 ${boneName})`];
      if (d.clamp) bits.push(`推出被钳(总位移≥${Viewer.SKIRT_MAX_PUSH}) ${d.clamp} 顶点次(余量仍穿腿)`);
      this.dwarn(bits.join(" · "));
    }
    if (
      this.clothSims.length &&
      (d.clothSeek || d.clothVelClamp || d.clothSafeClamp || d.clothLagClamp)
    ) {
      this.dlog(
        `布料模拟 近${d.frames}帧: seek重置 ${d.clothSeek} · 速度限幅 ${d.clothVelClamp} · 安全钳位 ${d.clothSafeClamp} · 滞后钳位 ${d.clothLagClamp}`
      );
    }
    this.diagClear();
  }

  private diagClear() {
    const d = this.diag;
    d.i0 = -1;
    d.frames = 0;
    d.maxNeed = 0;
    d.maxBone = -1;
    d.clamp = 0;
    d.clothSeek = 0;
    d.clothVelClamp = 0;
    d.clothSafeClamp = 0;
    d.clothLagClamp = 0;
    this.lastMirrorSig = "";
    if (gfmEnabled) gfm!.diag_reset();
  }

  /**
   * 裙摆布料骨检测与次级运动模拟初始化（仅 clothSimEnabled=true 时生效，
   * 默认关闭——引擎无运行时布料，默认走游戏忠实的 FSKEL 链动画回放）。
   *
   * Bone* 布料链（裙摆前 2 后 2 + 侧链，链根挂骨盆）的 FSKEL 姿态是布料
   * 模拟的种子状态：与 FSKIN 绑定数据恒不一致（坏链 M·boneLocalPos 残差
   * 26~52 且 girl.FSKEL 全部 1633 帧都不回落），直接精确 LBS 会把裙摆
   * 顶点甩出 50+ 单位（穿进躯干的尖刺）；而走路等动作的 FSKEL 布料姿态
   * 又几乎静止（0~3°/帧），只拿文件数据裙摆不会动——游戏客户端是运行时
   * 驱动布料的。因此 viewer 对全部 Bone* 骨改做自己的次级运动：
   *
   *   contribution = [T(offset)] · P(f) · Tp⁻¹ · v.pos
   *
   * P=载体（骨盆）当前世界矩阵，Tp⁻¹ 把 authored pos 带入骨盆绑定系；
   * 每骨一个 verlet 点做弹簧-阻尼跟随（目标 = 骨盆携带的顶点簇质心）+
   * 微重力 + 腿部球碰撞，offset = 模拟点相对携带目标的滞后（≤8 单位）
   * 以平移施加在整块补丁上——不用旋转（簇臂长仅 1~3 单位，小滞后会被
   * 放大成接近 180° 的翻转把面板甩飞）。静止时裙子精确贴绑定姿态，
   * 运动/急停时裙摆滞后摆动，与游戏行为定性一致。部件增删后需重跑。
   */
  /** 布料弹簧模拟开关：false=游戏忠实（默认，链骨直接回放 FSKEL 动画），见 clothSimEnabled 注释 */
  setClothSim(on: boolean) {
    if (this.clothSimEnabled === on) return;
    this.clothSimEnabled = on;
    if (on) {
      this.setupSkirtRig();
      this.setupHairBodyGrid(); // 腿胶囊等共享数据（addPart/fskel 加载时已建则幂等）
      this.detectCloth();
    } else {
      this.clothBones.clear();
      this.clothMats.clear();
      this.clothSims = [];
      this.clothCarrier = -1;
      this.clothTpInv = null;
      this.clothLastAbs = NaN;
      if (gfmEnabled) gfm!.clear_cloth();
    }
    for (const p of this.parts.values()) this.skinPart(p);
    this.applySkirtDePenVerts(); // 暂停态切换开关也保持去穿透生效
    this.applyHairBodyDepen();
  }

  /**
   * 裙摆防穿模数据（恒开）：裙片顶点清单 + 腿部碰撞胶囊。
   * 与布料模拟开关无关，applyFskelFrame 每帧用一个纯位置的去穿透 pass
   * （applySkirtDePen）保证裙片不进腿。部件增删后重跑。
   */
  private setupSkirtRig() {
    this.skirtVerts = [];
    this.skirtTris = [];
    this.skirtLegTris = [];
    this.skirtBodyTris = [];
    this.legCaps = [];
    const n = this.fskelBoneCount;
    if (!this.fskelPoses || !this.skelParents.length || !n) return;
    const legBones = new Set<number>();
    for (let b = 0; b < n; b++) {
      const name = this.skelBones[b]?.name ?? "";
      if (!/Thigh|Calf|Foot/.test(name)) continue;
      legBones.add(b);
      // 段长 = 子骨局部平移长度（骨骼沿 +X）
      let len = 12;
      for (let c = 0; c < n; c++) {
        if (this.skelParents[c] === b) {
          const o = c * 7;
          len = Math.hypot(this.fskelPoses[o], this.fskelPoses[o + 1], this.fskelPoses[o + 2]);
          break;
        }
      }
      this.legCaps.push({ bone: b, len });
    }
    // 与 skinPart 同源遍历（inflOf 含接缝克隆），收集受 Bone* 链影响的顶点；
    // 同源三角化收集裙片三角形与去重边（跨穿推出用）+ 腿网格三角形。
    // 合成件（长发/刚性帽）不进裙摆池（与其余拟合函数同口径）：其顶点无
    // 链/腿影响，只会以 body 三角形身份进阶段五跟随池——合成发顶点接收裤
    // 侧位移，帽偏移调整会连带被拽动
    for (const p of this.parts.values()) {
      if (p.syntheticHair || p.syntheticMount) continue;
      const chain = new Uint8Array(p.inflOf.length);
      const leg = new Uint8Array(p.inflOf.length);
      for (let i = 0; i < p.inflOf.length; i++) {
        for (const inf of p.inflOf[i]) {
          if (inf.boneIndex >= n) continue;
          if (this.isBoneHelperName(inf.boneIndex)) chain[i] = 1;
          else if (legBones.has(inf.boneIndex)) leg[i] = 1;
        }
        if (chain[i]) this.skirtVerts.push({ p, i });
      }
      const idxAttr = p.geom.getIndex();
      const idx = idxAttr ? (idxAttr.array as ArrayLike<number>) : null;
      const triCount = idx ? Math.floor(idx.length / 3) : Math.floor(p.inflOf.length / 3);
      for (let t = 0; t < triCount; t++) {
        const a = idx ? idx[t * 3] : t * 3;
        const b = idx ? idx[t * 3 + 1] : t * 3 + 1;
        const c = idx ? idx[t * 3 + 2] : t * 3 + 2;
        if (chain[a] || chain[b] || chain[c]) {
          this.skirtTris.push({ p, a, b, c });
        } else if (leg[a] || leg[b] || leg[c]) {
          this.skirtLegTris.push({ p, a, b, c });
        } else {
          this.skirtBodyTris.push({ p, a, b, c });
        }
      }
    }
    // 平滑状态随裙顶点集重建（部件增删会改变下标）
    this.skirtPrevDisp = new Float64Array(this.skirtVerts.length * 3);
    this.skirtOrigSnap = new Float64Array(this.skirtVerts.length * 3);
    this.skirtPrevAbs = NaN;
    // 邻接表：skirtTris 三角形边 + 拆缝克隆对（副本与源顶点互为邻居，
    // 松弛时同步位移防接缝撕开）
    {
      const svIdx = new Map<PartObj, Map<number, number>>();
      this.skirtVerts.forEach((sv, s) => {
        let m = svIdx.get(sv.p);
        if (!m) svIdx.set(sv.p, (m = new Map()));
        m.set(sv.i, s);
      });
      const adj: Set<number>[] = Array.from({ length: this.skirtVerts.length }, () => new Set<number>());
      for (const t of this.skirtTris) {
        const m = svIdx.get(t.p)!;
        const vs = [m.get(t.a), m.get(t.b), m.get(t.c)].filter((x) => x !== undefined) as number[];
        for (let a = 0; a < vs.length; a++)
          for (let b = 0; b < vs.length; b++) if (a !== b) adj[vs[a]].add(vs[b]);
      }
      for (const [p, m] of svIdx) {
        const vcount = p.model.vertices.length;
        for (const [i, s] of m) {
          if (i < vcount) continue;
          const src = m.get(p.clones[i - vcount].src);
          if (src !== undefined) {
            adj[s].add(src);
            adj[src].add(s);
          }
        }
      }
      this.skirtAdj = adj.map((a) => [...a]);
    }
    this.skirtRelaxA = new Float64Array(this.skirtVerts.length * 3);
    this.skirtRelaxB = new Float64Array(this.skirtVerts.length * 3);
    // wasm rig 上传（skirt_depen 消费；点集随部件增删重建，平滑状态一并重置）
    if (gfmEnabled) {
      const verts = new Uint32Array(this.skirtVerts.length * 2);
      this.skirtVerts.forEach((sv, s) => {
        verts[s * 2] = sv.p.gfmId;
        verts[s * 2 + 1] = sv.i;
      });
      const triFlat = (list: { p: PartObj; a: number; b: number; c: number }[]) => {
        const out = new Uint32Array(list.length * 4);
        list.forEach((t, i) => out.set([t.p.gfmId, t.a, t.b, t.c], i * 4));
        return out;
      };
      const adjOff = new Uint32Array(this.skirtAdj.length + 1);
      let adjTotal = 0;
      this.skirtAdj.forEach((nb, i) => {
        adjOff[i] = adjTotal;
        adjTotal += nb.length;
      });
      adjOff[this.skirtAdj.length] = adjTotal;
      const adjFlat = new Uint32Array(adjTotal);
      let ao = 0;
      for (const nb of this.skirtAdj) {
        adjFlat.set(nb, ao);
        ao += nb.length;
      }
      const capsFlat = new Float32Array(this.legCaps.length * 2);
      this.legCaps.forEach((c, i) => capsFlat.set([c.bone, c.len], i * 2));
      gfm!.setup_skirt(
        verts,
        triFlat(this.skirtTris),
        triFlat(this.skirtLegTris),
        triFlat(this.skirtBodyTris),
        adjOff,
        adjFlat,
        capsFlat
      );
    }
  }

  /**
   * 三角形-三角形相交（标量无分配）：AABB 早退 → 平面侧别早退 →
   * 边-三角形求交（Möller–Trumbore 式，t∈[0,1]）双向。共面重叠
   * （服装双层布的设计内重合）返回 false 跳过。绕向符号约定：三角形
   * 内部点满足 cross(边, 点-边起点)·法线 ≥ 0（合成用例自测过，
   * 符号写反会全程假阴性——见 2026-10-21 run9 排查教训）。
   */
  private static triTri(
    pa: Float32Array, a3: number, b3: number, c3: number,
    lp: Float32Array, u3: number, v3: number, w3: number
  ): boolean {
    const ax = pa[a3], ay = pa[a3 + 1], az = pa[a3 + 2];
    const bx = pa[b3], by = pa[b3 + 1], bz = pa[b3 + 2];
    const cx = pa[c3], cy = pa[c3 + 1], cz = pa[c3 + 2];
    const ux = lp[u3], uy = lp[u3 + 1], uz = lp[u3 + 2];
    const vx = lp[v3], vy = lp[v3 + 1], vz = lp[v3 + 2];
    const wx = lp[w3], wy = lp[w3 + 1], wz = lp[w3 + 2];
    // AABB
    if (Math.min(ax, bx, cx) > Math.max(ux, vx, wx)) return false;
    if (Math.min(ux, vx, wx) > Math.max(ax, bx, cx)) return false;
    if (Math.min(ay, by, cy) > Math.max(uy, vy, wy)) return false;
    if (Math.min(uy, vy, wy) > Math.max(ay, by, cy)) return false;
    if (Math.min(az, bz, cz) > Math.max(uz, vz, wz)) return false;
    if (Math.min(uz, vz, wz) > Math.max(az, bz, cz)) return false;
    // 法线（各三角形自身绕向）
    const abx = bx - ax, aby = by - ay, abz = bz - az;
    const acx = cx - ax, acy = cy - ay, acz = cz - az;
    const nvx = aby * acz - abz * acy, nvy = abz * acx - abx * acz, nvz = abx * acy - aby * acx;
    const uvx = vx - ux, uvy = vy - uy, uvz = vz - uz;
    const uwx = wx - ux, uwy = wy - uy, uwz = wz - uz;
    const nux = uvy * uwz - uvz * uwy, nuy = uvz * uwx - uvx * uwz, nuz = uvx * uwy - uvy * uwx;
    // 平面侧别早退
    const d0 = nux * (ax - ux) + nuy * (ay - uy) + nuz * (az - uz);
    const d1 = nux * (bx - ux) + nuy * (by - uy) + nuz * (bz - uz);
    const d2 = nux * (cx - ux) + nuy * (cy - uy) + nuz * (cz - uz);
    if (d0 > 1e-9 && d1 > 1e-9 && d2 > 1e-9) return false;
    if (d0 < -1e-9 && d1 < -1e-9 && d2 < -1e-9) return false;
    const e0 = nvx * (ux - ax) + nvy * (uy - ay) + nvz * (uz - az);
    const e1 = nvx * (vx - ax) + nvy * (vy - ay) + nvz * (vz - az);
    const e2 = nvx * (wx - ax) + nvy * (wy - ay) + nvz * (wz - az);
    if (e0 > 1e-9 && e1 > 1e-9 && e2 > 1e-9) return false;
    if (e0 < -1e-9 && e1 < -1e-9 && e2 < -1e-9) return false;
    if (Math.abs(d0) < 1e-9 && Math.abs(d1) < 1e-9 && Math.abs(d2) < 1e-9) return false; // 共面跳过
    // A 的边穿 U 平面且交点在 U 内
    // 边 (ax,ay,az)→(bx,by,bz)：
    {
      const dx = bx - ax, dy = by - ay, dz = bz - az;
      const dn = nux * dx + nuy * dy + nuz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nux * (ux - ax) + nuy * (uy - ay) + nuz * (uz - az)) / dn;
        if (t >= 0 && t <= 1) {
          const px = ax + dx * t, py = ay + dy * t, pz = az + dz * t;
          if (Viewer.inTri(px, py, pz, ux, uy, uz, vx, vy, vz, wx, wy, wz, nux, nuy, nuz)) return true;
        }
      }
    }
    {
      const dx = cx - bx, dy = cy - by, dz = cz - bz;
      const dn = nux * dx + nuy * dy + nuz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nux * (ux - bx) + nuy * (uy - by) + nuz * (uz - bz)) / dn;
        if (t >= 0 && t <= 1) {
          const px = bx + dx * t, py = by + dy * t, pz = bz + dz * t;
          if (Viewer.inTri(px, py, pz, ux, uy, uz, vx, vy, vz, wx, wy, wz, nux, nuy, nuz)) return true;
        }
      }
    }
    {
      const dx = ax - cx, dy = ay - cy, dz = az - cz;
      const dn = nux * dx + nuy * dy + nuz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nux * (ux - cx) + nuy * (uy - cy) + nuz * (uz - cz)) / dn;
        if (t >= 0 && t <= 1) {
          const px = cx + dx * t, py = cy + dy * t, pz = cz + dz * t;
          if (Viewer.inTri(px, py, pz, ux, uy, uz, vx, vy, vz, wx, wy, wz, nux, nuy, nuz)) return true;
        }
      }
    }
    // U 的边穿 A 平面且交点在 A 内
    {
      const dx = vx - ux, dy = vy - uy, dz = vz - uz;
      const dn = nvx * dx + nvy * dy + nvz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nvx * (ax - ux) + nvy * (ay - uy) + nvz * (az - uz)) / dn;
        if (t >= 0 && t <= 1) {
          const px = ux + dx * t, py = uy + dy * t, pz = uz + dz * t;
          if (Viewer.inTri(px, py, pz, ax, ay, az, bx, by, bz, cx, cy, cz, nvx, nvy, nvz)) return true;
        }
      }
    }
    {
      const dx = wx - vx, dy = wy - vy, dz = wz - vz;
      const dn = nvx * dx + nvy * dy + nvz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nvx * (ax - vx) + nvy * (ay - vy) + nvz * (az - vz)) / dn;
        if (t >= 0 && t <= 1) {
          const px = vx + dx * t, py = vy + dy * t, pz = vz + dz * t;
          if (Viewer.inTri(px, py, pz, ax, ay, az, bx, by, bz, cx, cy, cz, nvx, nvy, nvz)) return true;
        }
      }
    }
    {
      const dx = ux - wx, dy = uy - wy, dz = uz - wz;
      const dn = nvx * dx + nvy * dy + nvz * dz;
      if (Math.abs(dn) > 1e-12) {
        const t = (nvx * (ax - wx) + nvy * (ay - wy) + nvz * (az - wz)) / dn;
        if (t >= 0 && t <= 1) {
          const px = wx + dx * t, py = wy + dy * t, pz = wz + dz * t;
          if (Viewer.inTri(px, py, pz, ax, ay, az, bx, by, bz, cx, cy, cz, nvx, nvy, nvz)) return true;
        }
      }
    }
    return false;
  }

  /** 点到三角形最近距离（Ericson，标量无分配；用于跨穿推出的幅度收口） */
  private static ptTriDist(
    px: number, py: number, pz: number,
    ax: number, ay: number, az: number,
    bx: number, by: number, bz: number,
    cx: number, cy: number, cz: number
  ): number {
    const abx = bx - ax, aby = by - ay, abz = bz - az;
    const acx = cx - ax, acy = cy - ay, acz = cz - az;
    const apx = px - ax, apy = py - ay, apz = pz - az;
    const d1 = abx * apx + aby * apy + abz * apz;
    const d2 = acx * apx + acy * apy + acz * apz;
    if (d1 <= 0 && d2 <= 0) return Math.hypot(apx, apy, apz);
    const bpx = px - bx, bpy = py - by, bpz = pz - bz;
    const d3 = abx * bpx + aby * bpy + abz * bpz;
    const d4 = acx * bpx + acy * bpy + acz * bpz;
    if (d3 >= 0 && d4 <= d3) return Math.hypot(bpx, bpy, bpz);
    const vc = d1 * d4 - d3 * d2;
    if (vc <= 0 && d1 >= 0 && d3 <= 0) {
      const t = d1 / (d1 - d3);
      return Math.hypot(ax + abx * t - px, ay + aby * t - py, az + abz * t - pz);
    }
    const cpx = px - cx, cpy = py - cy, cpz = pz - cz;
    const d5 = abx * cpx + aby * cpy + abz * cpz;
    const d6 = acx * cpx + acy * cpy + acz * cpz;
    if (d6 >= 0 && d5 <= d6) return Math.hypot(cpx, cpy, cpz);
    const vb = d5 * d2 - d1 * d6;
    if (vb <= 0 && d2 >= 0 && d6 <= 0) {
      const t = d2 / (d2 - d6);
      return Math.hypot(ax + acx * t - px, ay + acy * t - py, az + acz * t - pz);
    }
    const va = d3 * d6 - d5 * d4;
    if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
      const t = (d4 - d3) / (d4 - d3 + (d5 - d6));
      return Math.hypot(bx + (cx - bx) * t - px, by + (cy - by) * t - py, bz + (cz - bz) * t - pz);
    }
    const den = va + vb + vc;
    const t0 = vb / den, t1 = vc / den;
    return Math.hypot(ax + abx * t0 + acx * t1 - px, ay + aby * t0 + acy * t1 - py, az + abz * t0 + acz * t1 - pz);
  }

  /** 点到三角形最近点 + 重心权（wA+wB+wC=1，out 复用避免分配；
   *  与 ptTriDist 同分支结构，供接触跟随把位移分配到三顶点） */
  private static ptTriBary(
    out: Float64Array,
    px: number, py: number, pz: number,
    ax: number, ay: number, az: number,
    bx: number, by: number, bz: number,
    cx: number, cy: number, cz: number
  ): number {
    const abx = bx - ax, aby = by - ay, abz = bz - az;
    const acx = cx - ax, acy = cy - ay, acz = cz - az;
    const apx = px - ax, apy = py - ay, apz = pz - az;
    const d1 = abx * apx + aby * apy + abz * apz;
    const d2 = acx * apx + acy * apy + acz * apz;
    const set = (d: number, wa: number, wb: number, wc: number) => {
      out[0] = d; out[1] = wa; out[2] = wb; out[3] = wc;
      return d;
    };
    if (d1 <= 0 && d2 <= 0) return set(Math.hypot(apx, apy, apz), 1, 0, 0);
    const bpx = px - bx, bpy = py - by, bpz = pz - bz;
    const d3 = abx * bpx + aby * bpy + abz * bpz;
    const d4 = acx * bpx + acy * bpy + acz * bpz;
    if (d3 >= 0 && d4 <= d3) return set(Math.hypot(bpx, bpy, bpz), 0, 1, 0);
    const vc = d1 * d4 - d3 * d2;
    if (vc <= 0 && d1 >= 0 && d3 <= 0) {
      const t = d1 / (d1 - d3);
      return set(Math.hypot(ax + abx * t - px, ay + aby * t - py, az + abz * t - pz), 1 - t, t, 0);
    }
    const cpx = px - cx, cpy = py - cy, cpz = pz - cz;
    const d5 = abx * cpx + aby * cpy + abz * cpz;
    const d6 = acx * cpx + acy * cpy + acz * cpz;
    if (d6 >= 0 && d5 <= d6) return set(Math.hypot(cpx, cpy, cpz), 0, 0, 1);
    const vb = d5 * d2 - d1 * d6;
    if (vb <= 0 && d2 >= 0 && d6 <= 0) {
      const t = d2 / (d2 - d6);
      return set(Math.hypot(ax + acx * t - px, ay + acy * t - py, az + acz * t - pz), 1 - t, 0, t);
    }
    const va = d3 * d6 - d5 * d4;
    if (va <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
      const t = (d4 - d3) / (d4 - d3 + (d5 - d6));
      return set(Math.hypot(bx + (cx - bx) * t - px, by + (cy - by) * t - py, bz + (cz - bz) * t - pz), 0, 1 - t, t);
    }
    const den = va + vb + vc;
    const t0 = vb / den, t1 = vc / den;
    return set(
      Math.hypot(ax + abx * t0 + acx * t1 - px, ay + aby * t0 + acy * t1 - py, az + abz * t0 + acz * t1 - pz),
      1 - t0 - t1, t0, t1
    );
  }

  /** 点在三角形内（已知三角形法线；绕向：cross(边, 点-边起点)·n ≥ 0 全边成立） */
  private static inTri(
    px: number, py: number, pz: number,
    ax: number, ay: number, az: number,
    bx: number, by: number, bz: number,
    cx: number, cy: number, cz: number,
    nx: number, ny: number, nz: number
  ): boolean {
    // AB × AP · n
    let t = nx * ((bx - ax) * (pz - az) - (bz - az) * (py - ay))
          + ny * ((bz - az) * (px - ax) - (bx - ax) * (pz - az))
          + nz * ((bx - ax) * (py - ay) - (by - ay) * (px - ax));
    if (t < 0) return false;
    // BC × BP · n
    t = nx * ((cx - bx) * (pz - bz) - (cz - bz) * (py - by))
      + ny * ((cz - bz) * (px - bx) - (cx - bx) * (pz - bz))
      + nz * ((cx - bx) * (py - by) - (cy - by) * (px - bx));
    if (t < 0) return false;
    // CA × CP · n
    t = nx * ((ax - cx) * (pz - cz) - (az - cz) * (py - cy))
      + ny * ((az - cz) * (px - cx) - (ax - cx) * (pz - cz))
      + nz * ((ax - cx) * (py - cy) - (ay - cy) * (px - cx));
    return t >= 0;
  }

  /**
   * 裙摆去穿透（恒开，纯位置修正无状态 → seek/倒放确定性）：
   * skinPart 之后对裙片顶点（受 Bone* 链影响的，含拆缝副本）做腿胶囊推出。
   * **逐顶点、逐胶囊依次推出**——推完一条胶囊立即更新位置再测下一条：
   * 裆部顶点（两腿之间）对左右腿的推出方向相反，必须依次处理才能落到
   * 两腿之外；取"最深穿透"单方向推或整链平移都会把裙拖进裆部/撕裂
   * （qmw1 深蹲骑马实测教训）。混合权重顶点同样一次到位。
   * 单顶点全程总推出预算 ≤ SKIRT_MAX_PUSH（顶点/跨穿两阶段共享）：
   * 两腿间深穿透的逐胶囊推出会来回振荡，预算保证单帧位移有界不撕裂。
   * 顶点推出只保证顶点在胶囊外——大三角形面仍可横穿腿（run9 474-477
   * 后中裙片切大腿后侧实测）。跨穿判据 = 裙三角形×腿网格三角形精确
   * 相交（triTri，与验证探针同口径），命中才沿径向外推三顶点；胶囊
   * 半径/网格内采样等代理判据都有反例（详见阶段二注释）。
   */
  private applySkirtDePenVerts(abs = NaN) {
    // 暂停态的帧外调用（addPart/setClothSim）：其它部件缓冲仍停留在上一轮
    // 去穿透输出（暂停不逐帧重蒙皮），skirtOrigSnap 会把已推位置当基准，
    // 阶段二相交腿网格的 +Y 最小推逐轮棘轮累积——暂停下调帽偏移（长发
    // 重建走 addPart）把裤链顶点逐次推高的根因。先全量重蒙皮恢复"基准=
    // 当前帧 LBS 输出"前置条件，与播放路径 applyFskelFrame 逐帧同口径
    // （同参"重蒙皮+去穿透"循环位和恒定实测）；帧内调用传有限 abs，部件
    // 刚重蒙皮过，不受影响
    if (!Number.isFinite(abs) && this.skelBones.length) {
      for (const p of this.parts.values()) this.skinPart(p);
    }
    if (gfmEnabled) {
      this.gfmUploadWorldMats();
      if (this.skirtRecord) this.skirtPushLog = [];
      const touched = gfm!.skirt_depen(abs);
      for (const p of this.parts.values()) {
        for (let i = 0; i < touched.length; i++) {
          if (touched[i] === p.gfmId) {
            this.markPartAttrs(p);
            break;
          }
        }
      }
      if (this.skirtRecord) {
        // wasm 采集记录（v, phase, k, boneIdx 四元组）转 cdp 采集脚本的原口径
        const log = gfm!.skirt_push_log();
        for (let i = 0; i + 3 < log.length; i += 4) {
          this.skirtPushLog.push({
            v: log[i],
            phase: log[i + 1],
            k: log[i + 2],
            cap: log[i + 3] >= 0 ? this.skelBones[log[i + 3]]?.name : undefined,
          });
        }
      }
      return;
    }
    if (!this.skirtVerts.length || !this.legCaps.length || !this.worldMats.length) return;
    if (this.skirtRecord) this.skirtPushLog = [];
    const caps: {
      ax: number; ay: number; az: number; bx: number; by: number; bz: number;
      r: number; bone: number;
    }[] = [];
    for (const c of this.legCaps) {
      const m = this.worldMats[c.bone];
      if (!m) continue;
      const e = m.elements;
      caps.push({
        ax: e[12], ay: e[13], az: e[14],
        bx: e[12] + e[0] * c.len, by: e[13] + e[1] * c.len, bz: e[14] + e[2] * c.len,
        r: c.len * Viewer.CLOTH_LEG_R,
        bone: c.bone,
      });
    }
    if (!caps.length) return;
    const touched = new Set<THREE.BufferAttribute>();
    const attrOf = (p: PartObj) => p.geom.getAttribute("position") as THREE.BufferAttribute;
    // 带符号腿网格距离（粗筛半径 6）：phase 1/3/4 安全网的胶囊→网格口径
    // 收口。取最近腿三角形，按其外向法线侧别定号——腿管近似凸，网格
    // 内（真穿透）为负、外（贴合/远离）为正。不能用无符号距离：压进
    // 腿管内的顶点到腿面距离恒正，会被误判成"贴合外"放掉穿透
    // （run9 475 深穿 1.7 回归实测）。
    const baryScratch = new Float64Array(4);
    const legSignedDist = (px: number, py: number, pz: number): number => {
      let best = Infinity;
      let bwx = 0, bwy = 0, bwz = 0;
      let ba3 = 0, bb3 = 0, bc3 = 0;
      let bp: PartObj | null = null;
      for (const t of this.skirtLegTris) {
        const pa = attrOf(t.p).array as Float32Array;
        const a3 = t.a * 3, b3 = t.b * 3, c3 = t.c * 3;
        const da = (pa[a3] - px) ** 2 + (pa[a3 + 1] - py) ** 2 + (pa[a3 + 2] - pz) ** 2;
        const db = (pa[b3] - px) ** 2 + (pa[b3 + 1] - py) ** 2 + (pa[b3 + 2] - pz) ** 2;
        const dc = (pa[c3] - px) ** 2 + (pa[c3 + 1] - py) ** 2 + (pa[c3 + 2] - pz) ** 2;
        if (da > 64 && db > 64 && dc > 64) continue;
        const d = Viewer.ptTriBary(baryScratch, px, py, pz,
          pa[a3], pa[a3 + 1], pa[a3 + 2], pa[b3], pa[b3 + 1], pa[b3 + 2], pa[c3], pa[c3 + 1], pa[c3 + 2]);
        if (d < best) {
          best = d;
          bp = t.p; ba3 = a3; bb3 = b3; bc3 = c3;
          bwx = baryScratch[1]; bwy = baryScratch[2]; bwz = baryScratch[3];
        }
      }
      if (!bp) return Infinity;
      const pa = attrOf(bp).array as Float32Array;
      const cx = pa[ba3] * bwx + pa[bb3] * bwy + pa[bc3] * bwz;
      const cy = pa[ba3 + 1] * bwx + pa[bb3 + 1] * bwy + pa[bc3 + 1] * bwz;
      const cz = pa[ba3 + 2] * bwx + pa[bb3 + 2] * bwy + pa[bc3 + 2] * bwz;
      // 侧别用胶囊径向（腿管≈圆柱，轴=腿骨）：p−closest 与
      // closest−轴投影 同向=面外正、反向=面内负。不依赖网格绕向——
      // 腿三角形绕向不统一（实测胯区全反），而"三角形中心−轴"参考在
      // 腿管端帽区退化（方向沿轴，符号随机，run9 475 深穿被判外 2.5）
      let refx = 0, refy = 0, refz = 0, refLen = Infinity;
      for (const c of caps) {
        const abx = c.bx - c.ax, aby = c.by - c.ay, abz = c.bz - c.az;
        const ab2 = abx * abx + aby * aby + abz * abz;
        let t = ab2 > 1e-9 ? ((cx - c.ax) * abx + (cy - c.ay) * aby + (cz - c.az) * abz) / ab2 : 0;
        t = Math.min(Math.max(t, 0), 1);
        const dx = cx - (c.ax + abx * t), dy = cy - (c.ay + aby * t), dz = cz - (c.az + abz * t);
        const l2 = dx * dx + dy * dy + dz * dz;
        if (l2 > 1e-9 && l2 < refLen) { refLen = l2; refx = dx; refy = dy; refz = dz; }
      }
      if (refLen < Infinity) {
        const rl = Math.sqrt(refLen);
        const sgn = ((px - cx) * refx + (py - cy) * refy + (pz - cz) * refz) / rl;
        return Math.hypot(px - cx, py - cy, pz - cz) * Math.sign(sgn || 1);
      }
      return best;
    };
    // (part, inflOf 下标) → skirtVerts 下标：边/三角引用非裙端点时跳过推送
    const svIdx = new Map<PartObj, Map<number, number>>();
    this.skirtVerts.forEach((sv, s) => {
      let m = svIdx.get(sv.p);
      if (!m) svIdx.set(sv.p, (m = new Map()));
      m.set(sv.i, s);
    });
    const budget = new Float64Array(this.skirtVerts.length).fill(Viewer.SKIRT_MAX_PUSH);
    // 阶段二"全外面片斜切满推"的顶点标记：阶段四点五豁免衰减（斜切
    // 修复的推出量不能被贴合衰减砍掉）
    const noDecay = new Uint8Array(this.skirtVerts.length);
    // 去穿透前快照（阶段四平滑的位移基准 = skinPart 输出位置）
    for (let s = 0; s < this.skirtVerts.length; s++) {
      const sv = this.skirtVerts[s];
      const pa = attrOf(sv.p).array as Float32Array;
      const o = sv.i * 3;
      this.skirtOrigSnap[s * 3] = pa[o];
      this.skirtOrigSnap[s * 3 + 1] = pa[o + 1];
      this.skirtOrigSnap[s * 3 + 2] = pa[o + 2];
    }
    const pushVert = (s: number, k: number, dx: number, dy: number, dz: number) => {
      if (s < 0 || k <= 0) return;
      const sv = this.skirtVerts[s];
      const attr = attrOf(sv.p);
      const pa = attr.array as Float32Array;
      const o = sv.i * 3;
      pa[o] += dx * k;
      pa[o + 1] += dy * k;
      pa[o + 2] += dz * k;
      budget[s] -= k;
      touched.add(attr);
      if (this.skirtRecord) this.skirtPushLog.push({ v: s, phase: 2, k });
    };

    // 阶段一：顶点推出（最多 3 轮：Thigh/Calf 胶囊在膝盖交叠，单遍依次
    // 推出在重叠区可能不收敛，迭代到无推出为止）
    for (let s = 0; s < this.skirtVerts.length; s++) {
      const sv = this.skirtVerts[s];
      const attr = attrOf(sv.p);
      const pa = attr.array as Float32Array;
      const o = sv.i * 3;
      let px = pa[o], py = pa[o + 1], pz = pa[o + 2];
      const o0x = px, o0y = py, o0z = pz;
      let exhausted = false;
      for (let pass = 0; pass < 3 && !exhausted; pass++) {
        let pushed = false;
        for (const c of caps) {
          const abx = c.bx - c.ax, aby = c.by - c.ay, abz = c.bz - c.az;
          const apx = px - c.ax, apy = py - c.ay, apz = pz - c.az;
          const ab2 = abx * abx + aby * aby + abz * abz;
          let t = ab2 > 1e-9 ? (apx * abx + apy * aby + apz * abz) / ab2 : 0;
          t = Math.min(Math.max(t, 0), 1);
          const qx = c.ax + abx * t, qy = c.ay + aby * t, qz = c.az + abz * t;
          const vx = px - qx, vy = py - qy, vz = pz - qz;
          const d = Math.hypot(vx, vy, vz);
          let need = c.r + Viewer.SKIRT_MARGIN - d;
          if (need <= 1e-3) continue;
          if (need > this.diag.maxNeed) {
            this.diag.maxNeed = need;
            this.diag.maxBone = c.bone;
          }
          // 网格口径收口：胶囊命中但贴腿网格（裙腰顶边贴腿根）不推
          need = Math.min(need, Math.max(0, Viewer.SKIRT_MESH_MARGIN - legSignedDist(px, py, pz)));
          if (need <= 1e-3) continue;
          if (budget[s] <= 1e-3) {
            this.diag.clamp++; // 预算耗尽，余量仍穿
            exhausted = true;
            break;
          }
          const k = Math.min(need, budget[s]);
          if (k < need) this.diag.clamp++;
          budget[s] -= k;
          if (this.skirtRecord)
            this.skirtPushLog.push({ v: s, phase: 1, k, cap: this.skelBones[c.bone]?.name });
          if (d > 1e-6) {
            px += (vx / d) * k;
            py += (vy / d) * k;
            pz += (vz / d) * k;
          } else {
            py += k; // 退化：朝上推出
          }
          pushed = true;
        }
        if (!pushed) break;
      }
      if (px !== o0x || py !== o0y || pz !== o0z) {
        pa[o] = px;
        pa[o + 1] = py;
        pa[o + 2] = pz;
        touched.add(attr);
      }
    }

    // 阶段二：面片跨穿推出（2 轮迭代）。顶点推出只保证顶点在胶囊外——
    // 大三角形面仍可横穿腿（run9 474-477 后中裙片切大腿后侧实测）。
    // 判据 = 裙三角形与腿网格三角形的精确相交（与验证探针同口径）：
    // 命中才推，零误报——按胶囊/网格内采样判都有反例（胖胶囊把贴腿裙
    // 弦顶出鼓包；射线奇偶把裙片从腿管顶端开口的合法伸入判成穿透，
    // run9 478-482 左腰被误推 3.5 单位）。方向=沿"违反最多"的胶囊轴
    // 径向外推，幅度取该口径加下限；共面重叠（双层布设计）直接跳过
    for (let round = 0; round < 2; round++) {
      let anyHit = false;
      for (const t of this.skirtTris) {
        const sa = svIdx.get(t.p)?.get(t.a) ?? -1;
        const sb = svIdx.get(t.p)?.get(t.b) ?? -1;
        const sc = svIdx.get(t.p)?.get(t.c) ?? -1;
        if (sa < 0 && sb < 0 && sc < 0) continue;
        const pa = attrOf(t.p).array as Float32Array;
        const hitLt = { p: null as PartObj | null, a: 0, b: 0, c: 0 };
        for (const lt of this.skirtLegTris) {
          if (!Viewer.triTri(
            pa, t.a * 3, t.b * 3, t.c * 3,
            attrOf(lt.p).array as Float32Array, lt.a * 3, lt.b * 3, lt.c * 3
          )) continue;
          hitLt.p = lt.p; hitLt.a = lt.a; hitLt.b = lt.b; hitLt.c = lt.c;
          break;
        }
        if (!hitLt.p) continue;
        anyHit = true;
        // 命中三角形对的几何中心做径向基准
        const pa2 = attrOf(hitLt.p).array as Float32Array;
        const mx = (pa[t.a * 3] + pa[t.b * 3] + pa[t.c * 3] + pa2[hitLt.a * 3] + pa2[hitLt.b * 3] + pa2[hitLt.c * 3]) / 6;
        const my = (pa[t.a * 3 + 1] + pa[t.b * 3 + 1] + pa[t.c * 3 + 1] + pa2[hitLt.a * 3 + 1] + pa2[hitLt.b * 3 + 1] + pa2[hitLt.c * 3 + 1]) / 6;
        const mz = (pa[t.a * 3 + 2] + pa[t.b * 3 + 2] + pa[t.c * 3 + 2] + pa2[hitLt.a * 3 + 2] + pa2[hitLt.b * 3 + 2] + pa2[hitLt.c * 3 + 2]) / 6;
        let need = 0.35;
        let ddx = 0, ddy = 1, ddz = 0;
        for (const c of caps) {
          const abx = c.bx - c.ax, aby = c.by - c.ay, abz = c.bz - c.az;
          const ab2 = abx * abx + aby * aby + abz * abz;
          let tp = ab2 > 1e-9
            ? ((mx - c.ax) * abx + (my - c.ay) * aby + (mz - c.az) * abz) / ab2
            : 0;
          tp = Math.min(Math.max(tp, 0), 1);
          const vx = mx - (c.ax + abx * tp), vy = my - (c.ay + aby * tp), vz = mz - (c.az + abz * tp);
          const d = Math.hypot(vx, vy, vz);
          const nd = c.r + Viewer.SKIRT_MARGIN - d;
          if (nd > need) {
            need = nd;
            if (d > 1e-6) {
              ddx = vx / d; ddy = vy / d; ddz = vz / d;
            }
          }
        }
        // 幅度收口：臀根等区域网格戳出胖胶囊，胶囊口径会把仅 1~3 间隙的
        // 浅剐蹭推 3.5+（run9 465-468 左大腿上方持续凸起实测；同一顶点
        // 的多个父三角各自命中还会累计）。取"三顶点到被剐腿三角形的最近
        // 距离 + 0.3"为上限——剐蹭角就在表面附近，该距离≈真实切入深度，
        // 浅剐浅推；且推出后距离实时缩小，多父三角累计自然收敛
        {
          const la = [hitLt.a * 3, hitLt.b * 3, hitLt.c * 3];
          const lx = [pa2[la[0]], pa2[la[1]], pa2[la[2]]];
          const ly = [pa2[la[0] + 1], pa2[la[1] + 1], pa2[la[2] + 1]];
          const lz = [pa2[la[0] + 2], pa2[la[1] + 2], pa2[la[2] + 2]];
          let dmin = Infinity;
          for (const vi of [t.a * 3, t.b * 3, t.c * 3]) {
            dmin = Math.min(
              dmin,
              Viewer.ptTriDist(pa[vi], pa[vi + 1], pa[vi + 2], lx[0], ly[0], lz[0], lx[1], ly[1], lz[1], lx[2], ly[2], lz[2])
            );
          }
          need = Math.min(need, dmin + 0.3);
          if (need < 0.35) need = 0.35;
        }
        const k = Math.min(
          need,
          sa >= 0 ? budget[sa] : Infinity,
          sb >= 0 ? budget[sb] : Infinity,
          sc >= 0 ? budget[sc] : Infinity
        );
        if (k > 1e-3) {
          // 面上存在真插入顶点（dm<0，wlk4 裙腰顶边贴腿根、底边插腿
          // 的斜插面）→ 按各顶点带符号腿网格距离加权：贴面顶边不带走
          // 开缝、插入段满推，三角形弯曲而非平移。面上全在腿外但面片
          // 仍与腿相交（run9 475 动画把裙面斜压切入腿，顶点级口径挡
          // 不住）→ 整面满推并标记免衰减（衰减判据 dm 区分不了贴合法
          // 与斜切）。变形由阶段二点五松弛抹平
          const dmOf = (s: number) => {
            if (s < 0) return Infinity;
            // 用帧首 LBS 基准位置：当帧位置在两轮迭代间漂移，第二轮
            // 会把第一轮刚推出的顶点 dm 推过 0.65 而误判"不贴"满推
            return legSignedDist(
              this.skirtOrigSnap[s * 3], this.skirtOrigSnap[s * 3 + 1], this.skirtOrigSnap[s * 3 + 2]
            );
          };
          const dma = dmOf(sa), dmb = dmOf(sb), dmc = dmOf(sc);
          const wOf = (dm: number) =>
            dm <= Viewer.SKIRT_MESH_MARGIN ? 1
              : Math.max(0, 1 - (dm - Viewer.SKIRT_MESH_MARGIN) / 0.5);
          if (dma < 0 || dmb < 0 || dmc < 0) {
            pushVert(sa, k * wOf(dma), ddx, ddy, ddz);
            pushVert(sb, k * wOf(dmb), ddx, ddy, ddz);
            pushVert(sc, k * wOf(dmc), ddx, ddy, ddz);
          } else {
            // 斜切满推也要区分：贴着腿面的顶点（dm<0.65，wlk4 裙腰顶
            // 边贴腿根）整面推走会与贴合界开缝——按贴合度压到 0.3；
            // 不贴的（run9 大腿后侧 dm≥0.65）满推修复斜切
            const wPush = (dm: number) => (dm >= Viewer.SKIRT_MESH_MARGIN + 0.5 ? 1 : 0.3);
            const mark = (s: number, dm: number) => {
              if (s >= 0 && wPush(dm) >= 1) noDecay[s] = 1;
            };
            mark(sa, dma); mark(sb, dmb); mark(sc, dmc);
            pushVert(sa, k * wPush(dma), ddx, ddy, ddz);
            pushVert(sb, k * wPush(dmb), ddx, ddy, ddz);
            pushVert(sc, k * wPush(dmc), ddx, ddy, ddz);
          }
        }
      }
      if (!anyHit) break;
    }

    // 阶段二点五：位移场空间松弛。阶段一/二都是逐顶点/逐面独立推出，
    // 未命中的邻居顶点零位移，同条布带上位移梯度可达 2.5+ → 布面翻折
    // 尖角（run9 462-486 v29/30 外缘常驻尖角、475-477 v36 反向尖谷
    // collect-frames 邻域差分实测）。对位移场（非位置）做邻域均值松弛，
    // 把推出量扩散给相邻顶点抹平梯度；均值保持不放大位移。被拉回胶囊
    // 内的顶点由阶段三复扫补推到胶囊面——顶点级正确性不妥协，且补推
    // 后恰停在最小必要位置（整面外推的过冲被松弛吸收）。
    if (this.skirtAdj.length === this.skirtVerts.length && this.skirtVerts.length) {
      const n = this.skirtVerts.length;
      const d = this.skirtRelaxA, t = this.skirtRelaxB;
      for (let s = 0; s < n; s++) {
        const sv = this.skirtVerts[s];
        const pa = attrOf(sv.p).array as Float32Array;
        const o = sv.i * 3;
        d[s * 3] = pa[o] - this.skirtOrigSnap[s * 3];
        d[s * 3 + 1] = pa[o + 1] - this.skirtOrigSnap[s * 3 + 1];
        d[s * 3 + 2] = pa[o + 2] - this.skirtOrigSnap[s * 3 + 2];
      }
      for (let iter = 0; iter < 2; iter++) {
        for (let s = 0; s < n; s++) {
          const nbs = this.skirtAdj[s];
          const s3 = s * 3;
          if (!nbs.length) {
            t[s3] = d[s3]; t[s3 + 1] = d[s3 + 1]; t[s3 + 2] = d[s3 + 2];
            continue;
          }
          let ax = 0, ay = 0, az = 0;
          for (const nb of nbs) {
            ax += d[nb * 3]; ay += d[nb * 3 + 1]; az += d[nb * 3 + 2];
          }
          ax /= nbs.length; ay /= nbs.length; az /= nbs.length;
          t[s3] = d[s3] + (ax - d[s3]) * Viewer.SKIRT_RELAX_R;
          t[s3 + 1] = d[s3 + 1] + (ay - d[s3 + 1]) * Viewer.SKIRT_RELAX_R;
          t[s3 + 2] = d[s3 + 2] + (az - d[s3 + 2]) * Viewer.SKIRT_RELAX_R;
        }
        d.set(t);
      }
      for (let s = 0; s < n; s++) {
        const sv = this.skirtVerts[s];
        const attr = attrOf(sv.p);
        const pa = attr.array as Float32Array;
        const o = sv.i * 3;
        pa[o] = this.skirtOrigSnap[s * 3] + d[s * 3];
        pa[o + 1] = this.skirtOrigSnap[s * 3 + 1] + d[s * 3 + 1];
        pa[o + 2] = this.skirtOrigSnap[s * 3 + 2] + d[s * 3 + 2];
        touched.add(attr);
      }
    }

    // 阶段三：复扫顶点（跨穿推出可能把端点推进其他胶囊，一轮收敛）
    for (let s = 0; s < this.skirtVerts.length; s++) {
      const sv = this.skirtVerts[s];
      const attr = attrOf(sv.p);
      const pa = attr.array as Float32Array;
      const o = sv.i * 3;
      let px = pa[o], py = pa[o + 1], pz = pa[o + 2];
      const o0x = px, o0y = py, o0z = pz;
      for (const c of caps) {
        const abx = c.bx - c.ax, aby = c.by - c.ay, abz = c.bz - c.az;
        const apx = px - c.ax, apy = py - c.ay, apz = pz - c.az;
        const ab2 = abx * abx + aby * aby + abz * abz;
        let t = ab2 > 1e-9 ? (apx * abx + apy * aby + apz * abz) / ab2 : 0;
        t = Math.min(Math.max(t, 0), 1);
        const vx = px - (c.ax + abx * t), vy = py - (c.ay + aby * t), vz = pz - (c.az + abz * t);
        const d = Math.hypot(vx, vy, vz);
        let need = c.r + Viewer.SKIRT_MARGIN - d;
        if (need <= 1e-3) continue;
        // 网格口径收口（与阶段一同口径）
        need = Math.min(need, Math.max(0, Viewer.SKIRT_MESH_MARGIN - legSignedDist(px, py, pz)));
        if (need <= 1e-3) continue;
        if (budget[s] <= 1e-3) {
          this.diag.clamp++;
          break;
        }
        const k = Math.min(need, budget[s]);
        if (k < need) this.diag.clamp++;
        budget[s] -= k;
        if (this.skirtRecord)
          this.skirtPushLog.push({ v: s, phase: 3, k, cap: this.skelBones[c.bone]?.name });
        if (d > 1e-6) {
          px += (vx / d) * k;
          py += (vy / d) * k;
          pz += (vz / d) * k;
        } else {
          py += k;
        }
      }
      if (px !== o0x || py !== o0y || pz !== o0z) {
        pa[o] = px;
        pa[o + 1] = py;
        pa[o + 2] = pz;
        touched.add(attr);
      }
    }
    // 阶段四：推出量时间平滑。无状态推出在相交起止帧边界瞬跳（0↔3，
    // collect-frames 帧间抖动实测 2.6~3.4）即"细微形变"主源。对每顶点
    // 总位移做入快出慢 lerp；seek（倒放/大跳/scrub/部件变更后首帧）
    // 直接用当帧原始值。平滑后若顶点仍在任一胶囊内 → 回退满推出，
    // 顶点级正确性不妥协（跨穿残余最多 1 帧且 ≤25% 深度）
    const seek =
      !Number.isFinite(abs) ||
      !Number.isFinite(this.skirtPrevAbs) ||
      Math.abs(abs - this.skirtPrevAbs) > 2;
    this.skirtPrevAbs = abs;
    for (let s = 0; s < this.skirtVerts.length; s++) {
      const sv = this.skirtVerts[s];
      const attr = attrOf(sv.p);
      const pa = attr.array as Float32Array;
      const o = sv.i * 3;
      const ox = this.skirtOrigSnap[s * 3], oy = this.skirtOrigSnap[s * 3 + 1], oz = this.skirtOrigSnap[s * 3 + 2];
      const dx = pa[o] - ox, dy = pa[o + 1] - oy, dz = pa[o + 2] - oz;
      let nx: number, ny: number, nz: number;
      if (seek) {
        nx = dx; ny = dy; nz = dz;
      } else {
        const px = this.skirtPrevDisp[s * 3], py = this.skirtPrevDisp[s * 3 + 1], pz = this.skirtPrevDisp[s * 3 + 2];
        const r =
          Math.hypot(dx, dy, dz) >= Math.hypot(px, py, pz) - 1e-9
            ? Viewer.SKIRT_SMOOTH_UP
            : Viewer.SKIRT_SMOOTH_DOWN;
        nx = px + (dx - px) * r;
        ny = py + (dy - py) * r;
        nz = pz + (dz - pz) * r;
        // 顶点级安全网：平滑结果仍在胶囊内 → 满推出（网格口径收口，
        // 贴腿网格不回退——平滑值即停留在贴合面附近）
        const fx = ox + nx, fy = oy + ny, fz = oz + nz;
        let need = 0;
        for (const c of caps) {
          const abx = c.bx - c.ax, aby = c.by - c.ay, abz = c.bz - c.az;
          const apx = fx - c.ax, apy = fy - c.ay, apz = fz - c.az;
          const ab2 = abx * abx + aby * aby + abz * abz;
          let t = ab2 > 1e-9 ? (apx * abx + apy * aby + apz * abz) / ab2 : 0;
          t = Math.min(Math.max(t, 0), 1);
          const d = Math.hypot(fx - (c.ax + abx * t), fy - (c.ay + aby * t), fz - (c.az + abz * t));
          need = Math.max(need, c.r + Viewer.SKIRT_MARGIN - d);
        }
        if (need > 1e-3) {
          need = Math.min(need, Math.max(0, Viewer.SKIRT_MESH_MARGIN - legSignedDist(fx, fy, fz)));
        }
        if (need > 1e-3) {
          nx = dx; ny = dy; nz = dz;
        }
      }
      pa[o] = ox + nx;
      pa[o + 1] = oy + ny;
      pa[o + 2] = oz + nz;
      this.skirtPrevDisp[s * 3] = nx;
      this.skirtPrevDisp[s * 3 + 1] = ny;
      this.skirtPrevDisp[s * 3 + 2] = nz;
      touched.add(attr);
    }

    // 阶段四点五：贴腿外侧裙顶点的位移最终衰减。这些顶点（裙腰顶边
    // 贴腿根 ~0.5，设计贴合非焊接）不该有位移，但三路来源都会喂它：
    // 跨穿整面外推、松弛扩散（邻居深推）、时间平滑出慢（上帧深推的
    // 残留 lerp）。全部位移确定后按带符号腿网格距离统一收口：
    // dm≤margin 满位移，dm∈[margin, margin+0.5] 线性降到 0.2 保底
    // （留两成跟随防与邻居断裂出尖角）。位置与 skirtPrevDisp 同步缩
    // 放——衰减结果即下一帧平滑基线（wlk4 105-112 裙腰横缝实测）。
    for (let s = 0; s < this.skirtVerts.length; s++) {
      const sv = this.skirtVerts[s];
      const attr = attrOf(sv.p);
      const pa = attr.array as Float32Array;
      const o = sv.i * 3;
      // 判据用 LBS 基准位置的 dm：原本贴合的（外 0.5）衰减；原本穿腿
      // 的（dm<margin）不衰减——用推出后位置会把刚推到位的顶点也砍掉
      // （run9 475 深穿修复被衰减到 20% 实测）
      const dm = legSignedDist(
        this.skirtOrigSnap[s * 3], this.skirtOrigSnap[s * 3 + 1], this.skirtOrigSnap[s * 3 + 2]
      );
      if (dm <= Viewer.SKIRT_MESH_MARGIN || noDecay[s]) continue;
      const w = Math.max(0.2, 1 - (dm - Viewer.SKIRT_MESH_MARGIN) / 0.5);
      const dx0 = this.skirtOrigSnap[s * 3], dy0 = this.skirtOrigSnap[s * 3 + 1], dz0 = this.skirtOrigSnap[s * 3 + 2];
      pa[o] = dx0 + (pa[o] - dx0) * w;
      pa[o + 1] = dy0 + (pa[o + 1] - dy0) * w;
      pa[o + 2] = dz0 + (pa[o + 2] - dz0) * w;
      this.skirtPrevDisp[s * 3] *= w;
      this.skirtPrevDisp[s * 3 + 1] *= w;
      this.skirtPrevDisp[s * 3 + 2] *= w;
      touched.add(attr);
    }

    // 阶段五：动态接触跟随（防贴合开缝）。裙腰顶边与腰带等躯干面是
    // "位置贴合非焊接"（authored 距离 ≥1，动作中可动态贴到 ~0.5）——
    // 去穿透把裙顶边推离腿的同时也推离贴合面，沿贴合线裂开 0.7~1.0
    // （wlk4 105-112 裙腰横缝实测）。对有位移的裙顶点按 LBS 基准位置
    // 找最近躯干三角形（skirtBodyTris：非链且非腿影响，含腰带/上衣），
    // 原本贴合（d0<SKIRT_WELD_D）且被拉开（>SKIRT_WELD_SEP）时，把该
    // 顶点最终位移按最近点重心权传给三角形三顶点（SKIRT_FOLLOW），
    // 贴合面随裙侧移动。腿三角形不在池内——推离腿正是去穿透的目的。
    if (this.skirtBodyTris.length) {
      const bary = new Float64Array(4);
      // 累积缓冲：同一躯干顶点可能收到多个裙点的跟随位移，最后统一施加
      const acc = new Map<THREE.BufferAttribute, Map<number, [number, number, number]>>();
      for (let s = 0; s < this.skirtVerts.length; s++) {
        const Dx = this.skirtPrevDisp[s * 3];
        const Dy = this.skirtPrevDisp[s * 3 + 1];
        const Dz = this.skirtPrevDisp[s * 3 + 2];
        if (Math.hypot(Dx, Dy, Dz) < 0.3) continue;
        const ox = this.skirtOrigSnap[s * 3];
        const oy = this.skirtOrigSnap[s * 3 + 1];
        const oz = this.skirtOrigSnap[s * 3 + 2];
        let bestD = Viewer.SKIRT_WELD_D;
        let bt: { p: PartObj; a: number; b: number; c: number } | null = null;
        let wa = 0, wb = 0, wc = 0;
        for (const t of this.skirtBodyTris) {
          const pa2 = attrOf(t.p).array as Float32Array;
          const a3 = t.a * 3, b3 = t.b * 3, c3 = t.c * 3;
          // 粗筛：三顶点都远于 4 不可能贴合（三角形外接半径 <3.4）
          const da = (pa2[a3] - ox) ** 2 + (pa2[a3 + 1] - oy) ** 2 + (pa2[a3 + 2] - oz) ** 2;
          const db = (pa2[b3] - ox) ** 2 + (pa2[b3 + 1] - oy) ** 2 + (pa2[b3 + 2] - oz) ** 2;
          const dc = (pa2[c3] - ox) ** 2 + (pa2[c3 + 1] - oy) ** 2 + (pa2[c3 + 2] - oz) ** 2;
          if (da > 16 && db > 16 && dc > 16) continue;
          const d = Viewer.ptTriBary(bary, ox, oy, oz,
            pa2[a3], pa2[a3 + 1], pa2[a3 + 2],
            pa2[b3], pa2[b3 + 1], pa2[b3 + 2],
            pa2[c3], pa2[c3 + 1], pa2[c3 + 2]);
          if (d < bestD) {
            bestD = d;
            bt = t;
            wa = bary[1]; wb = bary[2]; wc = bary[3];
          }
        }
        if (!bt) continue;
        // 位移后与同一三角形的距离（三角形未被推，位置即 LBS）
        const pa2 = attrOf(bt.p).array as Float32Array;
        const d1 = Viewer.ptTriBary(bary, ox + Dx, oy + Dy, oz + Dz,
          pa2[bt.a * 3], pa2[bt.a * 3 + 1], pa2[bt.a * 3 + 2],
          pa2[bt.b * 3], pa2[bt.b * 3 + 1], pa2[bt.b * 3 + 2],
          pa2[bt.c * 3], pa2[bt.c * 3 + 1], pa2[bt.c * 3 + 2]);
        if (d1 - bestD <= Viewer.SKIRT_WELD_SEP) continue;
        const f = Viewer.SKIRT_FOLLOW;
        const attr2 = attrOf(bt.p);
        let m = acc.get(attr2);
        if (!m) acc.set(attr2, (m = new Map()));
        const vs: [number, number][] = [[bt.a, wa], [bt.b, wb], [bt.c, wc]];
        for (const [vi, w] of vs) {
          if (w <= 1e-3) continue;
          let e = m.get(vi);
          if (!e) m.set(vi, (e = [0, 0, 0]));
          e[0] += Dx * f * w;
          e[1] += Dy * f * w;
          e[2] += Dz * f * w;
        }
      }
      for (const [attr, m] of acc) {
        const pa = attr.array as Float32Array;
        for (const [vi, e] of m) {
          pa[vi * 3] += e[0];
          pa[vi * 3 + 1] += e[1];
          pa[vi * 3 + 2] += e[2];
        }
        touched.add(attr);
      }
    }

    for (const a of touched) a.needsUpdate = true;
  }

  private detectCloth() {
    if (!this.clothSimEnabled) return; // 游戏忠实模式：链骨走精确 LBS
    this.clothBones.clear();
    this.clothMats.clear();
    this.clothSims = [];
    this.clothCarrier = -1;
    this.clothTpInv = null;
    this.clothLastAbs = NaN;
    if (gfmEnabled) gfm!.clear_cloth(); // 无 poses/无载体等兜底路径也保持 wasm 侧一致
    if (!this.fskelPoses || !this.skelBones.length || !this.skelParents.length) return;
    const n = this.fskelBoneCount;

    // 每骨点数据：单影响点对（载体拟合用）+ 全部影响顶点位置（模拟锚点用）
    const pairsOf: [number[], number[]][][] = Array.from({ length: n }, () => []);
    const vertsOf: number[][][] = Array.from({ length: n }, () => []);
    for (const p of this.parts.values()) {
      // 合成件（长发/NPC 挂饰）不参与布料骨拟合：挂骨下拉可选 Bone* 链骨，
      // 合成顶点会把该链的模拟锚点/目标拉向发簇（与其余拟合函数同口径）
      if (p.syntheticHair || p.syntheticMount) continue;
      for (const v of p.model.vertices) {
        for (const inf of v.influences) {
          if (inf.boneIndex >= n) continue;
          if (v.influences.length === 1 || inf.weight >= 0.99)
            pairsOf[inf.boneIndex].push([inf.boneLocalPos, v.pos]);
          vertsOf[inf.boneIndex].push(v.pos);
        }
      }
    }

    // 布料骨 = Bone* 命名的辅助骨（DCC 布料绑定惯例；裙摆链全员）
    for (let b = 0; b < n; b++) {
      if (this.isBoneHelperName(b) && vertsOf[b].length > 0) this.clothBones.add(b);
    }
    if (!this.clothBones.size) return;

    // 载体骨 = 布料链根的公共父骨（裙摆为骨盆），其数据绑定拟合 Tp
    const carrierVotes = new Map<number, number>();
    for (const b of this.clothBones) {
      const p = this.skelParents[b];
      if (p >= 0 && p < n && !this.clothBones.has(p))
        carrierVotes.set(p, (carrierVotes.get(p) ?? 0) + 1);
    }
    let bestVotes = 0;
    for (const [b, c] of carrierVotes) {
      if (c > bestVotes) {
        bestVotes = c;
        this.clothCarrier = b;
      }
    }
    if (this.clothCarrier < 0 || pairsOf[this.clothCarrier].length < 2) {
      this.clothBones.clear(); // 无载体拟合则退回精确 LBS（不接受半套机制）
      return;
    }
    this.clothTpInv = this.fitBindMatrix(pairsOf[this.clothCarrier]).invert();
    // 腿部碰撞胶囊由 setupSkirtRig 恒开维护（this.legCaps）

    // 每骨模拟状态：锚点 = 顶点簇上端 40% 质心，弹簧目标 = 全簇质心
    // （骨盆绑定系；世界→绑定系用 Tp⁻¹）
    for (const b of this.clothBones) {
      const vs = vertsOf[b];
      const sorted = [...vs].sort((a, c) => c[1] - a[1]); // 按 y 降序
      const topN = Math.max(1, Math.ceil(sorted.length * 0.4));
      const mean = (list: number[][]) =>
        list.reduce(
          (acc, p) => [acc[0] + p[0] / list.length, acc[1] + p[1] / list.length, acc[2] + p[2] / list.length],
          [0, 0, 0]
        );
      const anchor = new THREE.Vector3(...mean(sorted.slice(0, topN))).applyMatrix4(this.clothTpInv);
      const target = new THREE.Vector3(...mean(sorted)).applyMatrix4(this.clothTpInv);
      this.clothSims.push({
        bone: b,
        anchor,
        target,
        pos: new THREE.Vector3(NaN, NaN, NaN),
        prev: new THREE.Vector3(NaN, NaN, NaN),
      });
      this.clothMats.set(b, new THREE.Matrix4());
    }
    // wasm 模拟状态上传（updateClothMats/skinPart 消费 wasm 侧 clothMats）
    if (gfmEnabled) {
      const sims = new Float32Array(this.clothSims.length * 8);
      this.clothSims.forEach((s, i) => {
        sims.set(
          [s.bone, s.anchor.x, s.anchor.y, s.anchor.z, s.target.x, s.target.y, s.target.z],
          i * 8
        );
      });
      gfm!.setup_cloth(
        this.clothCarrier,
        this.fskelBoneCount,
        Float64Array.from(this.clothTpInv!.elements),
        sims
      );
    }
    this.updateClothMats(this.clothLastAbs);
  }

  /** 直接从姿态数组组合某帧各骨世界矩阵（不动当前骨架姿态）。
   *  wasm 路径仅支持无 mirror 覆盖的调用（diagnose 诊断用）；带 mirror 的
   *  调用（镜像检测）走 wasm 的 lbs_residual 一次完成，不会到此分支 */
  private worldsFromPoses(frame: number, mirror?: Set<number>): THREE.Matrix4[] {
    if (gfmEnabled && mirror === undefined) {
      const flat = gfm!.worlds_from_poses(frame);
      const out: THREE.Matrix4[] = [];
      if (flat) {
        for (let b = 0; b < this.fskelBoneCount; b++) {
          out.push(new THREE.Matrix4().fromArray(flat, b * 16));
        }
      }
      return out;
    }
    const n = this.fskelBoneCount;
    const stride = n * 7;
    const poses = this.fskelPoses!;
    const last = Math.max(Math.floor(poses.length / stride) - 1, 0);
    const base = Math.min(Math.max(frame, 0), last) * stride;
    const mir = mirror ?? this.mirrorBones;
    const out = Array.from({ length: n }, () => new THREE.Matrix4());
    for (let b = 0; b < n; b++) {
      const o = base + b * 7;
      this.qa.set(poses[o + 3], poses[o + 4], poses[o + 5], poses[o + 6]).normalize();
      out[b].compose(
        this.tmpVec.set(poses[o], poses[o + 1], poses[o + 2]),
        this.qa,
        this.tmpScale
      );
      // 镜像骨：本地旋转乘 -1（引擎骨记录 +52 标记 → Scale(-1,-1,-1)，
      // T·R·S 作用于点 = t + R·(-p)，与引擎 [S·R|t] 等价）；子骨经
      // 父矩阵自然继承镜像
      if (mir.has(b)) out[b].scale(this.mirrorScale);
      const p = this.skelParents[b];
      if (p >= 0 && p < b) out[b].premultiply(out[p]);
    }
    return out;
  }

  /**
   * 镜像骨判定。
   *
   * 首选数据驱动：FSKEL 帧记录尾部本来就有 1 字节镜像标志（IDA sub_1004C290
   * 逐字段确证 = 引擎骨记录 +52；旧解析把它当"可选终止符/裸 01 分隔符"吞掉）。
   * 标志=1 的骨（Bone* 链根、手指/脚趾 Nub、tag_LWeapon 等）引擎对其世界
   * 矩阵追加 Scale(-1,-1,-1)（sub_10052100 +52），FSKIN 的 boneLocalPos 也
   * 按镜像空间导出。名单跨动作文件稳定；残差检测曾在 Bone37/33/43 上漏检
   * （链静止残差≈0）、Bone01/05/16/66 上两可，标志位一律给出确定答案。
   *
   * 无标志数据（旧后端/异常文件）时回退残差推导：对每条 Bone* 链根试
   * 镜像/不镜像两种合成，取链上顶点 LBS 残差（vs authored）小的一侧——
   * 实测 Bone26/Bone55 镜像后残差 40.6→2.1、38.3→3.2。
   * 两种路径的结果都写入 mirrorBones 并经 syncMirrorVisuals 同步到
   * 可视骨骼 scale 与 wasm 内核（THREE 的 T·R·S 与引擎镜像语义等价）。
   */
  private detectMirrorBones(full = false) {
    this.mirrorBones.clear();
    if (!this.fskelPoses || !this.skelParents.length || !this.skelBones.length) return;
    const n = this.fskelBoneCount;
    // 数据驱动：引擎标志位（骨记录 +52，fskel.rs 解析 FSKEL 帧数据获得）
    if (this.mirrorFlagsFirst && this.mirrorFlagsFirst.length === n) {
      const names: string[] = [];
      for (let b = 0; b < n; b++) {
        if (this.mirrorFlagsFirst[b]) {
          this.mirrorBones.add(b);
          names.push(`${this.skelBones[b].name}(#${b})`);
        }
      }
      const sig = [...this.mirrorBones].sort((a, b) => a - b).join(",");
      if (sig !== this.lastMirrorSig) {
        this.lastMirrorSig = sig;
        const flip = this.mirrorUnstable.size
          ? ` · 帧间翻转: ${[...this.mirrorUnstable.keys()]
              .map((b) => this.skelBones[b]?.name ?? b)
              .join("/")}`
          : "";
        this.dlog(`镜像骨(引擎标志位): ${names.join(" · ")}${flip}`);
      }
      this.syncMirrorVisuals();
      return;
    }
    // 回退：残差推导（无标志数据时）
    // 链根 = Bone* 且父不是 Bone*
    const roots: number[] = [];
    for (let b = 0; b < n; b++) {
      if (!this.isBoneHelperName(b)) continue;
      const p = this.skelParents[b];
      if (p < 0 || !this.isBoneHelperName(p)) roots.push(b);
    }
    if (!roots.length || !this.parts.size) {
      for (let b = 0; b < n; b++) this.skelBones[b].scale.setScalar(1);
      if (gfmEnabled) gfm!.set_mirror(new Uint32Array(0));
      return;
    }
    // 链上顶点的 LBS 残差和（帧 0，vs authored）。wasm 路径：组合 + 残差
    // 一次内核完成（SoA 遍历）；mirror 仅用到 空集/单根 两种模式
    const residual = (mem: Set<number>, mirror: Set<number>): number => {
      if (gfmEnabled) {
        const arr = Uint32Array.from(mem);
        if (mirror.size === 0) return gfm!.lbs_residual(0, 1, 0, arr);
        return gfm!.lbs_residual(0, 2, mirror.values().next().value!, arr);
      }
      const W = this.worldsFromPoses(0, mirror);
      let sum = 0;
      for (const p of this.parts.values()) {
        // 合成件（长发/NPC 挂饰）排除，与其余拟合函数同口径：挂骨下拉可选
        // Bone* 链骨时合成顶点会带生成数据参与链残差投票（镜像侧残差非 0）
        if (p.syntheticHair || p.syntheticMount) continue;
        for (const v of p.model.vertices) {
          if (!v.influences.some((inf) => mem.has(inf.boneIndex))) continue;
          let x = 0, y = 0, z = 0;
          for (const inf of v.influences) {
            if (inf.boneIndex >= n) continue; // 部件与骨架不配套时的越界防御
            const M = W[inf.boneIndex].elements;
            const lp = inf.boneLocalPos;
            x += inf.weight * (M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12]);
            y += inf.weight * (M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13]);
            z += inf.weight * (M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]);
          }
          sum += Math.hypot(x - v.pos[0], y - v.pos[1], z - v.pos[2]);
        }
      }
      return sum;
    };
    // 部件增删的中间窗口（链部件暂缺，全部链上无顶点）：跳过本轮——不清
    // 标志、不打日志，链部件装回后下一轮自然重判。避免换装时标志清了又
    // 建、日志反复打印（套装切换实测）；骨架重载（full）仍全量重判
    const plains = roots.map((r) => residual(this.boneChainMembers(r), new Set()));
    if (!full && plains.every((s) => s === 0)) return;
    const lines: string[] = [];
    roots.forEach((r, i) => {
      const mem = this.boneChainMembers(r);
      const plain = plains[i];
      const mirrored = residual(mem, new Set([r]));
      const isMir = mirrored < plain * 0.5 && mirrored < plain - 5;
      if (isMir) this.mirrorBones.add(r);
      lines.push(
        `${this.skelBones[r].name}(#${r}) 残差 ${plain.toFixed(1)}→${mirrored.toFixed(1)}${isMir ? " ⇒镜像" : ""}`
      );
    });
    const sig = [...this.mirrorBones].sort((a, b) => a - b).join(",");
    if (sig !== this.lastMirrorSig) {
      this.lastMirrorSig = sig;
      this.dlog(`镜像骨检测(链根LBS残差 不镜像→镜像): ${lines.join(" · ")}`);
    }
    this.syncMirrorVisuals();
  }

  // ---- 长发合成件逐帧身体去穿透（applyFskelFrame 末尾调用）----
  /** 身体三角形空间网格（分桶 key = x|y|z 格坐标；构建于 setupHairBodyGrid） */
  private hairBodyCells = new Map<string, { p: PartObj; a: number; b: number; c: number }[]>();
  private hairBodyCellSize = 8;

  /** 重建身体三角形网格桶（部件增删/骨架重载后调用；authored 坐标分桶，
   *  桶 AABB 外扩 6 容纳动画位移 + 间隙余量） */
  private setupHairBodyGrid() {
    this.hairBodyCells.clear();
    const cs = this.hairBodyCellSize;
    const pad = 6;
    for (const p of this.parts.values()) {
      if (p.syntheticHair || p.syntheticMount || p.mesh.visible === false) continue;
      for (const sm of p.model.submeshes) {
        for (let t = 0; t + 2 < sm.triangles.length; t += 3) {
          const a = sm.triangles[t];
          const b = sm.triangles[t + 1];
          const c = sm.triangles[t + 2];
          const va = p.model.vertices[a]?.pos;
          const vb = p.model.vertices[b]?.pos;
          const vc = p.model.vertices[c]?.pos;
          if (!va || !vb || !vc) continue;
          for (let gx = Math.floor((Math.min(va[0], vb[0], vc[0]) - pad) / cs); gx <= Math.floor((Math.max(va[0], vb[0], vc[0]) + pad) / cs); gx++) {
            for (let gy = Math.floor((Math.min(va[1], vb[1], vc[1]) - pad) / cs); gy <= Math.floor((Math.max(va[1], vb[1], vc[1]) + pad) / cs); gy++) {
              for (let gz = Math.floor((Math.min(va[2], vb[2], vc[2]) - pad) / cs); gz <= Math.floor((Math.max(va[2], vb[2], vc[2]) + pad) / cs); gz++) {
                const key = gx + "|" + gy + "|" + gz;
                let arr = this.hairBodyCells.get(key);
                if (!arr) this.hairBodyCells.set(key, (arr = []));
                arr.push({ p, a, b, c });
              }
            }
          }
        }
      }
    }
  }

  /** 逐帧：长发合成件顶点推出身体网格（当前渲染坐标；体内判定用
   *  静止基准点（构建期已外推，必在体外）→ 当前位的穿面检测） */
  private applyHairBodyDepen() {
    if (!this.hairBodyCells.size) return;
    const MARGIN = 1.2;
    const cs = this.hairBodyCellSize;
    for (const part of this.parts.values()) {
      if (!part.syntheticHair) continue;
      const rest = (part.model as FskinModel & { __restPos?: Float64Array }).__restPos;
      const pos = part.mesh.geometry.getAttribute("position");
      if (!pos || !rest) continue;
      const arr = pos.array as Float32Array;
      const gx0 = (x: number) => Math.floor(x / cs);
      for (let i = 0; i < pos.count; i++) {
        const o = i * 3;
        if (o + 2 >= rest.length) continue; // 拆缝副本无静止基准（构建期未外推）
        let px = arr[o];
        let py = arr[o + 1];
        let pz = arr[o + 2];
        const cell = this.hairBodyCells.get(gx0(px) + "|" + gx0(py) + "|" + gx0(pz));
        if (!cell) continue;
        const rx = rest[o];
        const ry = rest[o + 1];
        const rz = rest[o + 2];
        let bestD = Infinity;
        let qx = 0;
        let qy = 0;
        let qz = 0;
        let crossings = 0;
        const dx = rx - px;
        const dy = ry - py;
        const dz = rz - pz;
        const segLen2 = dx * dx + dy * dy + dz * dz;
        for (const tr of cell) {
          const pa = (tr.p.mesh.geometry.getAttribute("position").array as Float32Array);
          const a3 = tr.a * 3;
          const b3 = tr.b * 3;
          const c3 = tr.c * 3;
          const ax = pa[a3], ay = pa[a3 + 1], az = pa[a3 + 2];
          const e1x = pa[b3] - ax, e1y = pa[b3 + 1] - ay, e1z = pa[b3 + 2] - az;
          const e2x = pa[c3] - ax, e2y = pa[c3 + 1] - ay, e2z = pa[c3 + 2] - az;
          // 最近点（Ericson 摘要：三顶点+边，含面内情形——发-体表为浅交，
          // 用三边+顶点最近已够判定距离；面内精确最近对浅穿影响 <0.2）
          const cands: [number, number, number][] = [
            [ax, ay, az],
            [pa[b3], pa[b3 + 1], pa[b3 + 2]],
            [pa[c3], pa[c3 + 1], pa[c3 + 2]],
          ];
          for (let e = 0; e < 3; e++) {
            const s1 = cands[e];
            const s2 = cands[(e + 1) % 3];
            const ex = s2[0] - s1[0], ey = s2[1] - s1[1], ez = s2[2] - s1[2];
            const t = Math.max(0, Math.min(1, ((px - s1[0]) * ex + (py - s1[1]) * ey + (pz - s1[2]) * ez) / (ex * ex + ey * ey + ez * ez || 1)));
            cands.push([s1[0] + ex * t, s1[1] + ey * t, s1[2] + ez * t]);
          }
          for (const q of cands) {
            const dd = (px - q[0]) ** 2 + (py - q[1]) ** 2 + (pz - q[2]) ** 2;
            if (dd < bestD) {
              bestD = dd;
              qx = q[0];
              qy = q[1];
              qz = q[2];
            }
          }
          // rest→cur 穿面计数（Möller；dir = d）
          if (segLen2 > 1e-9) {
            const pxx = dy * e2z - dz * e2y, pxy = dz * e2x - dx * e2z, pxz = dx * e2y - dy * e2x;
            const det = e1x * pxx + e1y * pxy + e1z * pxz;
            if (det > 1e-9 || det < -1e-9) {
              const inv = 1 / det;
              const tx = px - ax, ty = py - ay, tz = pz - az;
              const u = (tx * pxx + ty * pxy + tz * pxz) * inv;
              if (u >= 0 && u <= 1) {
                const qxx = ty * e1z - tz * e1y, qxy = tz * e1x - tx * e1z, qxz = tx * e1y - ty * e1x;
                const vv = (dx * qxx + dy * qxy + dz * qxz) * inv;
                if (vv >= 0 && u + vv <= 1) {
                  const t2 = (e2x * qxx + e2y * qxy + e2z * qxz) * inv;
                  if (t2 > 1e-4 && t2 < 1 - 1e-4) crossings++;
                }
              }
            }
          }
        }
        const d = Math.sqrt(bestD);
        const inside = crossings % 2 === 1;
        if (!inside && d >= MARGIN) continue;
        // 推出方向：体内 → 从最近表面点指向静止基准（必外）；体外近面 → 远离
        let dirx: number, diry: number, dirz: number;
        if (inside) {
          dirx = rx - qx;
          diry = ry - qy;
          dirz = rz - qz;
        } else {
          dirx = px - qx;
          diry = py - qy;
          dirz = pz - qz;
        }
        const dl = Math.hypot(dirx, diry, dirz);
        if (dl < 1e-6) continue;
        arr[o] = qx + (dirx / dl) * MARGIN;
        arr[o + 1] = qy + (diry / dl) * MARGIN;
        arr[o + 2] = qz + (dirz / dl) * MARGIN;
      }
      pos.needsUpdate = true;
    }
  }

  /** 选中骨骼：整段线条变色 + 关节球高亮 + 屏幕浮标显示骨名 */
  selectBone(b: number) {
    if (b === this.selectedBone) {
      if (b >= 0) this.onBonePick(b); // 重复点击仅刷新回调
      return;
    }
    const prev = this.selectedBone;
    this.selectedBone = b;
    const colAttr = this.boneLines?.geometry.getAttribute("color") as
      | THREE.BufferAttribute
      | undefined;
    if (colAttr) {
      const setSeg = (bone: number, c: THREE.Color) => {
        const seg = this.boneSegOf.indexOf(bone);
        if (seg < 0) return;
        for (const v of [seg * 6, seg * 6 + 3]) {
          colAttr.setXYZ(v, c.r, c.g, c.b);
        }
      };
      setSeg(prev, Viewer.BONE_BASE);
      setSeg(b, Viewer.BONE_HI);
      colAttr.needsUpdate = true;
    }
    if (this.jointMesh && prev >= 0) this.jointMesh.setColorAt(prev, boneColor(prev));
    if (this.jointMesh && b >= 0) this.jointMesh.setColorAt(b, Viewer.BONE_HI);
    if (this.jointMesh?.instanceColor) this.jointMesh.instanceColor.needsUpdate = true;
    if (b >= 0) {
      if (!this.boneLabelEl) {
        this.boneLabelEl = document.createElement("div");
        this.boneLabelEl.className = "bone-label";
        this.container.appendChild(this.boneLabelEl);
      }
      this.boneLabelEl.style.display = "block";
    }
    this.onBonePick(b);
  }

  deselectBone() {
    if (this.selectedBone < 0) return;
    this.selectBone(-1);
    if (this.boneLabelEl) this.boneLabelEl.style.display = "none";
  }

  /** 每帧刷新骨线条端点（父/骨当前世界平移） */
  private updateBoneLines() {
    if (!this.boneLines) return;
    const pos = this.boneLines.geometry.getAttribute("position") as THREE.BufferAttribute;
    const arr = pos.array as Float32Array;
    for (let i = 0; i < this.boneSegOf.length; i++) {
      const b = this.boneSegOf[i];
      const par = this.skelParents[b];
      const mb = this.worldMats[b]?.elements;
      const mp = this.worldMats[par]?.elements;
      if (!mb || !mp) continue;
      arr[i * 6] = mp[12];
      arr[i * 6 + 1] = mp[13];
      arr[i * 6 + 2] = mp[14];
      arr[i * 6 + 3] = mb[12];
      arr[i * 6 + 4] = mb[13];
      arr[i * 6 + 5] = mb[14];
    }
    pos.needsUpdate = true;
    this.updateBoneLabel();
  }

  /** 骨名浮标跟随选中骨骼的屏幕投影 */
  private updateBoneLabel() {
    const b = this.selectedBone;
    if (b < 0 || !this.boneLabelEl) return;
    const bone = this.skelBones[b];
    if (!bone) return;
    bone.updateWorldMatrix(true, false);
    const p = bone.getWorldPosition(this.tmpVec).applyMatrix4(this.modelRoot.matrixWorld);
    p.project(this.camera);
    const rect = this.renderer.domElement.getBoundingClientRect();
    const x = ((p.x + 1) / 2) * rect.width;
    const y = ((1 - p.y) / 2) * rect.height;
    this.boneLabelEl.style.left = x + 10 + "px";
    this.boneLabelEl.style.top = y - 14 + "px";
    this.boneLabelEl.textContent = `#${b} ${this.skelBones[b].name}`;
  }

  /** 镜像结果同步：可视骨骼 scale 与 wasm 内核 mirror 集 */
  private syncMirrorVisuals() {
    this.worldsFrameCache.clear(); // 镜像集变化使缓存的世界矩阵失效
    for (let b = 0; b < this.fskelBoneCount; b++) {
      this.skelBones[b].scale.setScalar(this.mirrorBones.has(b) ? -1 : 1);
    }
    if (gfmEnabled) gfm!.set_mirror(Uint32Array.from(this.mirrorBones));
  }

  /**
   * 布料次级运动步进 + 合成矩阵刷新（applyFskelFrame 每帧调用）。
   * abs = i0+alpha 绝对帧；倒退/跳跃视为 seek，模拟点重置到携带目标。
   */
  private updateClothMats(abs: number) {
    if (gfmEnabled) {
      this.gfmUploadWorldMats();
      gfm!.update_cloth(abs);
      return;
    }
    if (!this.clothSims.length || !this.clothTpInv || this.clothCarrier < 0) return;
    const carrier = this.worldMats[this.clothCarrier];
    if (!carrier) return;
    // 携带矩阵：骨盆绑定系 → 当前世界
    this.cCarry.multiplyMatrices(carrier, this.clothTpInv);

    let dt = 0;
    let seek = false;
    if (!Number.isFinite(abs) || !Number.isFinite(this.clothLastAbs)) {
      seek = true;
    } else {
      const d = abs - this.clothLastAbs;
      if (d < -0.01 || d > 3) seek = true; // 倒放/大跳 = seek
      else dt = Math.min(Math.max(d, 0), 2) / 30;
    }
    this.clothLastAbs = abs;
    if (seek) this.diag.clothSeek++;

    for (const s of this.clothSims) {
      this.cAnchor.copy(s.anchor).applyMatrix4(this.cCarry);
      this.cTarget.copy(s.target).applyMatrix4(this.cCarry);
      if (seek || !Number.isFinite(s.pos.x)) {
        s.pos.copy(this.cTarget);
        s.prev.copy(this.cTarget);
      } else if (dt > 0) {
        // verlet：速度阻尼（y 上扬加重抑制裙摆翻上腰，下落轻放保垂感——
        // 借鉴 AniCloth_SW 各向异性空气阻尼 y↑3.2/y↓0.4）+ 微重力 +
        // 弹簧拉向携带目标（速度限幅防炸）
        this.cVel.subVectors(s.pos, s.prev);
        this.cVel.x *= Viewer.CLOTH_DAMP;
        this.cVel.z *= Viewer.CLOTH_DAMP;
        this.cVel.y *= this.cVel.y > 0 ? Viewer.CLOTH_DAMP_UP : Viewer.CLOTH_DAMP_DOWN;
        const vmax = 6 * dt * 30; // ≤6 单位/帧
        if (this.cVel.length() > vmax) {
          this.cVel.setLength(vmax);
          this.diag.clothVelClamp++;
        }
        s.prev.copy(s.pos);
        s.pos.add(this.cVel);
        s.pos.y -= Viewer.CLOTH_G * dt * dt;
        s.pos.lerp(this.cTarget, Math.min(1, Viewer.CLOTH_K * dt * dt));
      }
      // 腿部胶囊推出（两轮；胶囊沿骨段全长，膝/踝关节处无球间空洞）
      for (let iter = 0; iter < 2; iter++) {
        for (const c of this.legCaps) {
          const m = this.worldMats[c.bone];
          if (!m) continue;
          const e = m.elements;
          const abx = e[0] * c.len, aby = e[1] * c.len, abz = e[2] * c.len;
          const ab2 = abx * abx + aby * aby + abz * abz;
          if (ab2 < 1e-9) continue;
          let t =
            ((s.pos.x - e[12]) * abx + (s.pos.y - e[13]) * aby + (s.pos.z - e[14]) * abz) / ab2;
          t = Math.min(Math.max(t, 0), 1);
          const qx = e[12] + abx * t, qy = e[13] + aby * t, qz = e[14] + abz * t;
          const dx = s.pos.x - qx, dy = s.pos.y - qy, dz = s.pos.z - qz;
          const d2 = dx * dx + dy * dy + dz * dz;
          const r = c.len * Viewer.CLOTH_LEG_R;
          if (d2 < r * r && d2 > 1e-8) {
            const d = Math.sqrt(d2);
            s.pos.set(qx + (dx / d) * r, qy + (dy / d) * r, qz + (dz / d) * r);
          }
        }
      }
      // 安全钳位：模拟点离锚点不超过静止臂长的 2 倍（防碰撞/大步长累积飞出）
      this.cDir1.subVectors(s.pos, this.cAnchor);
      const arm = this.cTarget.distanceTo(this.cAnchor);
      if (arm > 1e-4 && this.cDir1.length() > arm * 2) {
        s.pos.copy(this.cAnchor).addScaledVector(this.cDir1.normalize(), arm * 2);
        this.diag.clothSafeClamp++;
      }
      // 应用：平移跟随。不用旋转——簇臂长可能只有 1~3 单位，小滞后会被
      // 放大成接近 180° 的翻转（面板甩飞）；有界平移同样有滞后摆动感且绝无翻转
      this.cDir1.subVectors(s.pos, this.cTarget);
      const off = this.cDir1.length();
      if (off > 8) {
        this.cDir1.multiplyScalar(8 / off); // 裙摆滞后上限
        this.diag.clothLagClamp++;
      }
      const out = this.clothMats.get(s.bone)!;
      out.multiplyMatrices(
        this.cT1.makeTranslation(this.cDir1.x, this.cDir1.y, this.cDir1.z),
        this.cCarry
      );
    }
  }

  /** 点对 (骨骼局部, 世界) 的刚体拟合：H = Σ(b-cb)(a-ca)ᵀ 极分解求 R，t = cb - R·ca。
   *  拟合内核在 wasm（fmath fit_bind_matrix；极分解/两点对齐与 TS 口径一致） */
  private fitBindMatrix(pairs: [number[], number[]][]): THREE.Matrix4 {
    if (gfmEnabled) {
      const local = new Float32Array(pairs.length * 3);
      const world = new Float32Array(pairs.length * 3);
      pairs.forEach(([a, b], i) => {
        local.set(a, i * 3);
        world.set(b, i * 3);
      });
      return new THREE.Matrix4().fromArray(gfm!.fit_bind_matrix(local, world));
    }
    return this.fitBindMatrixLegacy(pairs);
  }

  private fitBindMatrixLegacy(pairs: [number[], number[]][]): THREE.Matrix4 {
    const n = pairs.length;
    if (n === 0) return new THREE.Matrix4();
    const ca = [0, 0, 0];
    const cb = [0, 0, 0];
    for (const [a, b] of pairs) {
      for (let k = 0; k < 3; k++) {
        ca[k] += a[k] / n;
        cb[k] += b[k] / n;
      }
    }
    let R: number[] | null = null;
    if (n >= 3) {
      const h = new Array<number>(9).fill(0);
      for (const [a, b] of pairs) {
        for (let r = 0; r < 3; r++) {
          for (let c = 0; c < 3; c++) h[r * 3 + c] += (b[r] - cb[r]) * (a[c] - ca[c]);
        }
      }
      R = polarRotation(h);
    } else if (n === 2) {
      // 两点：方向对齐（绕轴扭转未定，好过没有）
      const da = new THREE.Vector3(
        pairs[1][0][0] - pairs[0][0][0],
        pairs[1][0][1] - pairs[0][0][1],
        pairs[1][0][2] - pairs[0][0][2]
      ).normalize();
      const db = new THREE.Vector3(
        pairs[1][1][0] - pairs[0][1][0],
        pairs[1][1][1] - pairs[0][1][1],
        pairs[1][1][2] - pairs[0][1][2]
      ).normalize();
      const q = new THREE.Quaternion().setFromUnitVectors(da, db);
      const e = new THREE.Matrix4().makeRotationFromQuaternion(q).elements;
      R = [e[0], e[4], e[8], e[1], e[5], e[9], e[2], e[6], e[10]];
    }
    if (!R) R = [1, 0, 0, 0, 1, 0, 0, 0, 1]; // 退化：仅平移
    const t = [0, 1, 2].map(
      (r) => cb[r] - (R![r * 3] * ca[0] + R![r * 3 + 1] * ca[1] + R![r * 3 + 2] * ca[2])
    );
    return new THREE.Matrix4().set(
      R[0], R[1], R[2], t[0],
      R[3], R[4], R[5], t[1],
      R[6], R[7], R[8], t[2],
      0, 0, 0, 1
    );
  }

  /** 两个绑定刚体变换的差异量（R 逐元素最大差 与 t 距离 取最大） */
  private bindTransformDiff(a: THREE.Matrix4, b: THREE.Matrix4): number {
    let dr = 0;
    for (let i = 0; i < 16; i++) {
      dr = Math.max(dr, Math.abs(a.elements[i] - b.elements[i]));
    }
    const ta = [a.elements[12], a.elements[13], a.elements[14]];
    const tb = [b.elements[12], b.elements[13], b.elements[14]];
    const dt = Math.hypot(ta[0] - tb[0], ta[1] - tb[1], ta[2] - tb[2]);
    return Math.max(dr, dt);
  }

  /** 部件在各骨上的单影响点对（<3 个点的骨骼不出） */
  private exactPairsOf(p: PartObj): [number[], number[]][][] {
    const bc = this.fskelBoneCount || this.bindMats.length;
    const exact: [number[], number[]][][] = Array.from({ length: bc }, () => []);
    for (const v of p.model.vertices) {
      for (const inf of v.influences) {
        if (inf.boneIndex >= bc) continue;
        if (v.influences.length === 1 || inf.weight >= 0.99)
          exact[inf.boneIndex].push([inf.boneLocalPos, v.pos] as [number[], number[]]);
      }
    }
    return exact;
  }

  /**
   * 重算各骨共识绑定拟合（正变换 boneLocalPos→pos）：每骨各部件自身
   * 拟合后两两分簇（阈值 0.1），取总点数最多的簇合并拟合；无 exact 点
   * 的部件不投票，pool 不足 3 点时并入全部 approx 点。
   */
  private computeBindMats() {
    const boneCount = 1 + Math.max(
      0,
      ...[...this.parts.values()].flatMap((p) =>
        p.model.vertices.flatMap((v) => v.influences.map((i) => i.boneIndex))
      )
    );
    const perBone: { exact: [number[], number[]][][]; approx: [number[], number[]][][] }[] =
      Array.from({ length: boneCount }, () => ({ exact: [], approx: [] }));
    for (const p of this.parts.values()) {
      const exact: [number[], number[]][][] = Array.from({ length: boneCount }, () => []);
      const approx: [number[], number[]][][] = Array.from({ length: boneCount }, () => []);
      for (const v of p.model.vertices) {
        for (const inf of v.influences) {
          if (inf.boneIndex >= boneCount) continue;
          const pair = [inf.boneLocalPos, v.pos] as [number[], number[]];
          if (v.influences.length === 1 || inf.weight >= 0.99) exact[inf.boneIndex].push(pair);
          else approx[inf.boneIndex].push(pair);
        }
      }
      for (let b = 0; b < boneCount; b++) {
        if (exact[b].length) perBone[b].exact.push(exact[b]);
        if (approx[b].length) perBone[b].approx.push(approx[b]);
      }
    }
    this.bindMats = Array.from({ length: boneCount }, (_, b) => {
      const { exact, approx } = perBone[b];
      const fits = exact.map((pts) => this.fitBindMatrix(pts));
      const pool: [number[], number[]][] = [];
      if (fits.length >= 2) {
        const ok = (i: number, j: number) => this.bindTransformDiff(fits[i], fits[j]) < 0.1;
        const comps: number[][] = [];
        const seen = new Set<number>();
        for (let i = 0; i < fits.length; i++) {
          if (seen.has(i)) continue;
          const comp = [i];
          seen.add(i);
          let changed = true;
          while (changed) {
            changed = false;
            for (let j = 0; j < fits.length; j++) {
              if (!seen.has(j) && comp.some((c) => ok(c, j))) {
                comp.push(j);
                seen.add(j);
                changed = true;
              }
            }
          }
          comps.push(comp);
        }
        // 按部件数投票（点数会被大部件压倒锚定部件，如 hair05×256 反超头部×124）
        const best = comps.reduce((a, c) => (c.length > a.length ? c : a));
        for (const i of best) pool.push(...exact[i]);
      } else if (exact.length === 1) {
        pool.push(...exact[0]);
      }
      if (!pool.length) return new THREE.Matrix4();
      return this.fitBindMatrix(pool.length >= 3 ? pool : pool.concat(...approx));
    });
  }

  /** 部件的 authored pos 是否与共识摆放不一致（某骨自身拟合与共识差 >0.1） */
  private partBindInconsistent(p: PartObj): boolean {
    if (!this.bindMats.length) return false;
    const exact = this.exactPairsOf(p);
    for (let b = 0; b < this.bindMats.length; b++) {
      if (exact[b].length < 3) continue;
      if (this.bindTransformDiff(this.fitBindMatrix(exact[b]), this.bindMats[b]) > 0.1) return true;
    }
    return false;
  }

  /**
   * 静态（未载动作）默认姿态：一致部件保持 authored 几何；离群部件
   * （Girl_hair05 的 authored pos 与头部/其余发型在 0x6 骨上差一个刚体
   * 变换）按共识绑定矩阵渲染绑定姿态——与动作模式同一套 LBS 公式，
   * M 取共识拟合，发髻端正贴在头上。部件增删后调用。
   */
  private applyStaticSkin() {
    if (this.skelBones.length) return; // 动作模式由 applyFskelFrame 接管
    this.computeBindMats();
    if (!this.bindMats.length) return;
    let skinned = false;
    for (const p of this.parts.values()) {
      if (this.partBindInconsistent(p)) skinned = true;
      else if (p.skinned) this.restorePart(p); // 曾按离群蒙皮、现已一致 → 还原
    }
    if (!skinned) return;
    // 与 bindMats 分离的实例：避免动作模式覆写共识拟合
    this.worldMats = this.bindMats.map((m) => m.clone());
    for (const p of this.parts.values()) {
      if (!this.partBindInconsistent(p)) continue;
      p.skinned = true;
      this.skinPart(p);
    }
  }

  /**
   * 逐部件 CPU 蒙皮 = F3D.dll 引擎语义（SkinStream_SkinVerts_LBS
   * @0x10050C70 与 InitFromResSkin @0x10050BD0 逐字段对照）：
   *   pos = Σ w_b · M_b · boneLocalPos_b —— 每影响自带骨骼局部坐标，
   *        原始权重直接累加（引擎循环无归一化），不需要绑定矩阵；
   *   nrm = 静态 —— 引擎蒙皮循环只写位置三分量，法线是加载时从
   *        顶点记录 +12 拷贝的静态数据，随世界矩阵变换（引擎自身
   *        就不做法线蒙皮，光照用绑定姿势法线）。
   * 例外：布料骨（detectCloth 标记的裙摆 Bone* 链）仅在 clothSimEnabled
   * 开启时改用次级运动模拟的合成矩阵；默认（游戏忠实）链骨与本表第一
   * 行一致——直接用 FSKEL 姿态矩阵，条带跟随动作里烘焙的链动画。
   * 绑定矩阵近似（共识拟合）只在无骨架的静态模式兜底离群部件；
   * boneLocalPos 是引擎逐顶点写入的精确数据，直接加权即为游戏渲染结果。
   */
  private skinPart(p: PartObj) {
    if (gfmEnabled && p.gfmId >= 0) {
      this.gfmUploadWorldMats();
      gfm!.skin_part(p.gfmId);
      this.markPartAttrs(p);
      return;
    }
    if (!this.worldMats.length) return;
    const boneCount = this.fskelBoneCount || this.worldMats.length;
    const vcount = p.model.vertices.length;
    const n = p.inflOf.length;
    const posAttr = p.geom.getAttribute("position") as THREE.BufferAttribute;
    const nrmAttr = p.geom.getAttribute("normal") as THREE.BufferAttribute;
    const pa = posAttr.array as Float32Array;
    const na = nrmAttr.array as Float32Array;
    for (let i = 0; i < n; i++) {
      const infl = p.inflOf[i];
      let wsum = 0;
      for (let k = 0; k < infl.length; k++) {
        if (infl[k].boneIndex < boneCount) wsum += infl[k].weight;
      }
      const o = i * 3;
      const si = i < vcount ? i : p.clones[i - vcount].src;
      const no = si * 3;
      if (wsum <= 0) {
        // 无有效骨骼（骨架比部件骨骼少）：保持绑定姿势
        const v = p.model.vertices[si];
        pa[o] = v.pos[0]; pa[o + 1] = v.pos[1]; pa[o + 2] = v.pos[2];
        na[o] = p.srcNrm[no]; na[o + 1] = p.srcNrm[no + 1]; na[o + 2] = p.srcNrm[no + 2];
        continue;
      }
      let px = 0, py = 0, pz = 0;
      for (let k = 0; k < infl.length; k++) {
        const inf = infl[k];
        const bi = inf.boneIndex;
        if (bi >= boneCount) continue;
        const w = inf.weight; // 引擎用原始权重，不归一化
        // 布料骨：FSKEL 姿态与绑定数据恒不一致（游戏为运行时布料模拟），
        // 改为合成矩阵作用在 authored pos 上——见 detectCloth 注释
        if (this.clothMats.size && this.clothMats.has(bi)) {
          const C = this.clothMats.get(bi)!.elements;
          const sp = p.model.vertices[si].pos;
          px += w * (C[0] * sp[0] + C[4] * sp[1] + C[8] * sp[2] + C[12]);
          py += w * (C[1] * sp[0] + C[5] * sp[1] + C[9] * sp[2] + C[13]);
          pz += w * (C[2] * sp[0] + C[6] * sp[1] + C[10] * sp[2] + C[14]);
          continue;
        }
        // 锚定链骨（静态错位链，见 setupChainAnchors）：绑定摆放随载体骨，
        // 仍走 boneLocalPos 路径 → 多骨混合顶点按权重精确合成
        if (this.anchoredMats.size && this.anchoredMats.has(bi)) {
          const A = this.anchoredMats.get(bi)!.elements;
          const lp = inf.boneLocalPos;
          px += w * (A[0] * lp[0] + A[4] * lp[1] + A[8] * lp[2] + A[12]);
          py += w * (A[1] * lp[0] + A[5] * lp[1] + A[9] * lp[2] + A[13]);
          pz += w * (A[2] * lp[0] + A[6] * lp[1] + A[10] * lp[2] + A[14]);
          continue;
        }
        const M = this.worldMats[bi].elements;
        const lp = inf.boneLocalPos;
        // 列主序：M·lp
        px += w * (M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12]);
        py += w * (M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13]);
        pz += w * (M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]);
      }
      pa[o] = px; pa[o + 1] = py; pa[o + 2] = pz;
      na[o] = p.srcNrm[no]; na[o + 1] = p.srcNrm[no + 1]; na[o + 2] = p.srcNrm[no + 2];
    }
    posAttr.needsUpdate = true;
    nrmAttr.needsUpdate = true;
  }

  /** 卸下骨架时把部件几何还原为绑定姿势（authored 数据） */
  private restorePart(p: PartObj) {
    const vcount = p.model.vertices.length;
    const posAttr = p.geom.getAttribute("position") as THREE.BufferAttribute;
    const nrmAttr = p.geom.getAttribute("normal") as THREE.BufferAttribute;
    const pa = posAttr.array as Float32Array;
    const na = nrmAttr.array as Float32Array;
    for (let i = 0; i < p.inflOf.length; i++) {
      const src = i < vcount ? i : p.clones[i - vcount].src;
      const v = p.model.vertices[src];
      pa[i * 3] = v.pos[0]; pa[i * 3 + 1] = v.pos[1]; pa[i * 3 + 2] = v.pos[2];
      na[i * 3] = v.normal[0]; na[i * 3 + 1] = v.normal[1]; na[i * 3 + 2] = v.normal[2];
    }
    posAttr.needsUpdate = true;
    nrmAttr.needsUpdate = true;
    p.skinned = false;
  }

  /** 饰品透明叠层：tex = null 时移除。共享部件几何，跟随蒙皮/可见性/高亮。 */
  setPartOverlay(path: string, tex: TexData | null, texFile: string | null) {
    const p = this.parts.get(path);
    if (!p) return;
    if (!tex || !texFile) {
      this.removeOverlay(p);
      return;
    }
    if (!p.overlayMat) {
      p.overlayMat = new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        roughness: 0.75,
        metalness: 0,
        transparent: true,
        depthWrite: false,
      });
    }
    p.overlayMat.map = this.getTexture(texFile, tex);
    p.overlayMat.needsUpdate = true;
    if (!p.overlayMesh) {
      const mesh = new THREE.Mesh(p.geom, p.overlayMat);
      mesh.userData.path = path;
      mesh.renderOrder = 1; // 在基础网格之后绘制
      mesh.frustumCulled = false;
      mesh.visible = p.mesh.visible;
      this.modelRoot.add(mesh);
      p.overlayMesh = mesh;
    }
  }

  private removeOverlay(p: PartObj) {
    if (p.overlayMesh) {
      this.modelRoot.remove(p.overlayMesh);
      p.overlayMesh = null;
    }
    if (p.overlayMat) {
      p.overlayMat.dispose();
      p.overlayMat = null;
    }
  }

  /** 应用第 i0 帧姿态；给出 i1/alpha 时做平移 lerp + 旋转 slerp 平滑插值 */
  applyFskelFrame(i0: number, i1: number, alpha: number) {
    if (!this.fskelPoses || !this.skelBones.length) return;
    const poses = this.fskelPoses;
    const n = this.fskelBoneCount;
    const stride = n * 7;
    // 越界钳制：数组外读取会产出 NaN 网格（探针脚本传大帧号实测）
    const last = Math.max(Math.floor(poses.length / stride) - 1, 0);
    const c0 = Math.min(Math.max(Math.round(i0), 0), last);
    const c1 = Math.min(Math.max(Math.round(i1), 0), last);
    i0 = c0;
    i1 = c1;
    const base0 = i0 * stride;
    const base1 = i1 * stride;
    const lerp = alpha > 0 && i1 !== i0;
    for (let b = 0; b < n; b++) {
      const o0 = base0 + b * 7;
      const o1 = base1 + b * 7;
      const bone = this.skelBones[b];
      if (lerp) {
        bone.position.set(
          poses[o0] + (poses[o1] - poses[o0]) * alpha,
          poses[o0 + 1] + (poses[o1 + 1] - poses[o0 + 1]) * alpha,
          poses[o0 + 2] + (poses[o1 + 2] - poses[o0 + 2]) * alpha
        );
        this.qa.set(poses[o0 + 3], poses[o0 + 4], poses[o0 + 5], poses[o0 + 6]);
        this.qb.set(poses[o1 + 3], poses[o1 + 4], poses[o1 + 5], poses[o1 + 6]);
        bone.quaternion.slerpQuaternions(this.qa, this.qb, alpha);
      } else {
        bone.position.set(poses[o0], poses[o0 + 1], poses[o0 + 2]);
        bone.quaternion.set(poses[o0 + 3], poses[o0 + 4], poses[o0 + 5], poses[o0 + 6]);
      }
    }
    if (!this.skelRoot) return;
    this.skelRoot.updateMatrixWorld(true);
    for (let b = 0; b < n; b++) {
      this.worldMats[b].copy(this.skelBones[b].matrixWorld);
    }
    this.updateBoneLines();
    this.updateAnchoredMats();
    // 帽子等挂接件已并入 parts 管线：随下方 skinPart 循环统一蒙皮
    this.updateClothMats(i0 + alpha);
    // 关节小球跟随世界坐标
    const m = new THREE.Matrix4();
    for (let b = 0; b < n; b++) {
      m.setPosition(this.skelBones[b].getWorldPosition(this.tmpVec));
      this.jointMesh?.setMatrixAt(b, m);
    }
    if (this.jointMesh) this.jointMesh.instanceMatrix.needsUpdate = true;
    // 全部件按当前姿态重新蒙皮，随后逐顶点去穿透（改顶点位置，不改骨矩阵）
    for (const p of this.parts.values()) this.skinPart(p);
    this.applySkirtDePenVerts(i0 + alpha);
    this.applyHairBodyDepen(); // 长发合成件 vs 身体网格（静态外推兜不住动作期摆动）
    // 诊断窗口：按新帧号去重（scrub 反复应用同帧不重复计），每 30 帧 flush
    if (i0 !== this.diag.i0) {
      this.diag.i0 = i0;
      if (++this.diag.frames >= 30) this.diagFlush();
    }
  }

  clearFskel() {
    for (const p of this.parts.values()) this.restorePart(p);
    this.bindWorldMats = [];
    if (gfmEnabled) {
      gfm!.clear_poses();
      gfm!.clear_skirt();
      gfm!.clear_anchored();
      gfm!.clear_cloth();
    }
    if (this.skelHelper) {
      this.modelRoot.remove(this.skelHelper);
      this.skelHelper.dispose();
      this.skelHelper = null;
    }
    if (this.boneLines) {
      this.modelRoot.remove(this.boneLines);
      this.boneLines.geometry.dispose();
      (this.boneLines.material as THREE.Material).dispose();
      this.boneLines = null;
    }
    this.boneSegOf = [];
    this.deselectBone();
    if (this.jointMesh) {
      this.modelRoot.remove(this.jointMesh);
      this.jointMesh.geometry.dispose();
      (this.jointMesh.material as THREE.Material).dispose();
      this.jointMesh = null;
    }
    if (this.skelRoot) {
      this.scene.remove(this.skelRoot);
      this.skelRoot = null;
    }
    this.skelBones = [];
    this.fskelPoses = null;
    this.fskelBoneCount = 0;
    this.worldMats = [];
    this.skelParents = [];
    this.diagClear();
    this.mirrorBones.clear();
    this.clothBones.clear();
    this.clothMats.clear();
    this.clothSims = [];
    this.clothCarrier = -1;
    this.clothTpInv = null;
    this.clothLastAbs = NaN;
    this.skirtVerts = [];
    this.legCaps = [];
    this.anchoredRoots.clear();
    this.anchoredChains.clear();
    this.anchoredMats.clear();
    this.refreshBoneAttached(); // 骨架已清空：挂接网格回落为静态摆放
    this.bindFxToSkeleton(); // 挂接特效按名重解析（骨架清空 = 落角色原点）
    this.applyStaticSkin(); // 默认状态对齐离群部件（hair05）
  }

  // ---------------- FAK 顶点动画 ----------------

  /**
   * 构建顶点动画模型：每个子网格一个 Mesh，几何引用同一份共享顶点池属性
   * （position/normal 逐帧更新，UV 经全帧验证恒定作静态属性）。
   * mats[k] 为材质 k 的贴图；子网格按记录自带序号选用材质。
   */
  setFak(model: FakModel, mats: { tex: TexData | null; file: string | null }[]) {
    this.clearFak();
    this.fakModel = model;
    const n = model.vertexCount;
    const frame0 = model.frames[0]?.data ?? [];

    const pos = new Float32Array(n * 3);
    const nrm = new Float32Array(n * 3);
    const uv = new Float32Array(n * 2);
    for (let i = 0; i < n; i++) {
      uv[i * 2] = frame0[i * 8 + 6];
      uv[i * 2 + 1] = frame0[i * 8 + 7];
    }
    this.fakPos = new THREE.BufferAttribute(pos, 3);
    this.fakNrm = new THREE.BufferAttribute(nrm, 3);
    if (gfmEnabled) {
      // 全帧一次性上传二进制（原 number[] JSON 逐帧行访问，序列化/访问双收益）；
      // 顶点池属性换 wasm 视图，fak_frame 直写
      const total = model.frames.reduce((s, f) => s + f.data.length, 0);
      const all = new Float32Array(total);
      let fo = 0;
      for (const f of model.frames) {
        all.set(f.data, fo);
        fo += f.data.length;
      }
      gfm!.set_fak(all, model.frames.length, n);
      this.fakPos = new THREE.BufferAttribute(gfm!.fak_pos_view()!, 3);
      this.fakNrm = new THREE.BufferAttribute(gfm!.fak_nrm_view()!, 3);
    }
    this.fakPos.setUsage(THREE.DynamicDrawUsage);
    this.fakNrm.setUsage(THREE.DynamicDrawUsage);

    model.submeshes.forEach((s) => {
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", this.fakPos!);
      geom.setAttribute("normal", this.fakNrm!);
      geom.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
      geom.setIndex(s.triangles);
      // 材质按记录自带序号（多节可共用同一材质）
      const matIdx = Math.min(s.index, model.materials.length - 1);
      const fm = model.materials[matIdx];
      const mask = Viewer.texFlip(fm?.pre); // 与 addPart 同口径：pre[2]≠0xFF ⇒ 引擎 240 裁切
      const mat = new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        roughness: 0.75,
        metalness: 0.0,
        alphaTest: this.baseAlphaTestOf(mask),
      });
      if (fm) {
        mat.color.setRGB(fm.diffuse[0], fm.diffuse[1], fm.diffuse[2]);
        mat.emissive.setRGB(fm.emissive[0], fm.emissive[1], fm.emissive[2]);
        mat.roughness = Math.max(0.05, Math.min(1, 1 - fm.specPower / 60));
      }
      const m = mats[matIdx];
      if (m?.tex && m.file) mat.map = this.getTexture(m.file, m.tex, false, mask);
      const mesh = new THREE.Mesh(geom, mat);
      mesh.frustumCulled = false; // 顶点动画变形后静态包围球不再可靠
      this.modelRoot.add(mesh);
      this.fakMeshes.push(mesh);
      this.fakGeoms.push(geom);
      this.fakMats.push(mat);
      this.fakMatIdx.push(matIdx);
    });
    this.applyFakFrame(0, 0, 0);
  }

  /** FAK 材质贴图热替换（检查器换图）：按材质序号更新全部引用该材质的子网格 */
  setFakTexture(matIdx: number, file: string, tex: TexData) {
    const fm = this.fakModel?.materials[matIdx];
    const mask = Viewer.texFlip(fm?.pre);
    for (let i = 0; i < this.fakMats.length; i++) {
      if (this.fakMatIdx[i] !== matIdx) continue;
      this.fakMats[i].map = this.getTexture(file, tex, false, mask);
      this.fakMats[i].needsUpdate = true;
    }
  }

  /** 应用第 i0 帧；给出 i1/alpha 时做位置/法线 lerp 平滑插值（法线重归一化） */
  applyFakFrame(i0: number, i1: number, alpha: number) {
    if (gfmEnabled) {
      gfm!.fak_frame(i0, i1, alpha);
      this.fakPos!.needsUpdate = true;
      this.fakNrm!.needsUpdate = true;
      return;
    }
    const m = this.fakModel;
    const pos = this.fakPos;
    const nrm = this.fakNrm;
    if (!m || !pos || !nrm) return;
    const n = m.vertexCount;
    const d0 = m.frames[i0]?.data;
    if (!d0) return;
    const d1 = alpha > 0 && i1 !== i0 ? m.frames[i1]?.data : null;
    const pa = pos.array as Float32Array;
    const na = nrm.array as Float32Array;
    for (let i = 0; i < n; i++) {
      const b = i * 8;
      const o = i * 3;
      if (d1) {
        const a = alpha;
        pa[o] = d0[b] + (d1[b] - d0[b]) * a;
        pa[o + 1] = d0[b + 1] + (d1[b + 1] - d0[b + 1]) * a;
        pa[o + 2] = d0[b + 2] + (d1[b + 2] - d0[b + 2]) * a;
        na[o] = d0[b + 3] + (d1[b + 3] - d0[b + 3]) * a;
        na[o + 1] = d0[b + 4] + (d1[b + 4] - d0[b + 4]) * a;
        na[o + 2] = d0[b + 5] + (d1[b + 5] - d0[b + 5]) * a;
      } else {
        pa[o] = d0[b];
        pa[o + 1] = d0[b + 1];
        pa[o + 2] = d0[b + 2];
        na[o] = d0[b + 3];
        na[o + 1] = d0[b + 4];
        na[o + 2] = d0[b + 5];
      }
      // 文件内法线未归一化（长度 2.1~3.0），写入时归一化保证光照正确
      const len = Math.hypot(na[o], na[o + 1], na[o + 2]);
      if (len > 1e-6) {
        na[o] /= len;
        na[o + 1] /= len;
        na[o + 2] /= len;
      }
    }
    pos.needsUpdate = true;
    nrm.needsUpdate = true;
  }

  clearFak() {
    for (const mesh of this.fakMeshes) this.modelRoot.remove(mesh);
    for (const g of this.fakGeoms) g.dispose();
    for (const m of this.fakMats) m.dispose();
    if (gfmEnabled) gfm!.clear_fak();
    this.fakMeshes = [];
    this.fakGeoms = [];
    this.fakMats = [];
    this.fakMatIdx = [];
    this.fakPos = null;
    this.fakNrm = null;
    this.fakModel = null;
  }

  hasFak() {
    return this.fakMeshes.length > 0;
  }

  // ---------------- FSM 静态武器网格 ----------------

  /**
   * 构建静态网格的 Mesh 列表（每子网格一个，共享同一份顶点池属性，
   * pos/normal/uv 直接取自文件，法线写入时归一化）。setFsm 与骨骼挂接共用。
   */
  private buildFsmMeshes(
    model: FsmModel,
    mats: { tex: TexData | null; file: string | null }[]
  ): {
    meshes: THREE.Mesh[];
    geoms: THREE.BufferGeometry[];
    matsOut: THREE.MeshStandardMaterial[];
    /** matsOut 同位：各子网格引用的材质序号 */
    matIdxOut: number[];
  } {
    const n = model.vertexCount;
    const pos = new Float32Array(n * 3);
    const nrm = new Float32Array(n * 3);
    const uv = new Float32Array(n * 2);
    for (let i = 0; i < n; i++) {
      const b = i * 8;
      const o = i * 3;
      pos[o] = model.vertices[b];
      pos[o + 1] = model.vertices[b + 1];
      pos[o + 2] = model.vertices[b + 2];
      // 文件内法线未归一化（长度 0.86~1.68），写入时归一化保证光照正确
      const len = Math.hypot(
        model.vertices[b + 3],
        model.vertices[b + 4],
        model.vertices[b + 5]
      );
      if (len > 1e-6) {
        nrm[o] = model.vertices[b + 3] / len;
        nrm[o + 1] = model.vertices[b + 4] / len;
        nrm[o + 2] = model.vertices[b + 5] / len;
      }
      uv[i * 2] = model.vertices[b + 6];
      uv[i * 2 + 1] = model.vertices[b + 7];
    }
    // mask 类材质（pre[2]≠0xFF）的 UV 岛修复：图集行序混排（发帘恒等/装饰
    // 翻转），黑垫底簇整簇 v→1-v（FM38 装饰：黑 4→0），干净簇不动
    for (const [mi, tris] of Viewer.trisByMaterial(model)) {
      const m = mats[mi];
      if (!Viewer.texFlip(model.materials[mi]?.pre) || !m?.tex) continue;
      Viewer.repairMaskUvIslands(uv, tris, m.tex);
    }

    const meshes: THREE.Mesh[] = [];
    const geoms: THREE.BufferGeometry[] = [];
    const matsOut: THREE.MeshStandardMaterial[] = [];
    const matIdxOut: number[] = [];
    model.submeshes.forEach((s) => {
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
      geom.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
      geom.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
      geom.setIndex(s.triangles);
      // 材质按记录自带序号（c30 两节共用材质 0；序号即材质下标）
      const matIdx = Math.min(s.index, model.materials.length - 1);
      const fm = model.materials[matIdx];
      const mask = Viewer.texFlip(fm?.pre);
      const base = this.baseAlphaTestOf(mask);
      const mat = new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        roughness: 0.75,
        metalness: 0.0,
        // 帽/长发 FSM 贴图带 alpha 镂空（发帘边缘、帽檐），透明 texel 需
        // 裁掉——mask 类按引擎 MMAlphaMask ALPHAREF=240 高出切（边缘
        // 混合暗像素一并裁掉），普通类 0.1 兜底
        alphaTest: base,
      });
      mat.userData.baseAlphaTest = base;
      if (fm) {
        mat.color.setRGB(fm.diffuse[0], fm.diffuse[1], fm.diffuse[2]);
        mat.emissive.setRGB(fm.emissive[0], fm.emissive[1], fm.emissive[2]);
        mat.roughness = Math.max(0.05, Math.min(1, 1 - fm.specPower / 60));
      }
      const m = mats[matIdx];
      if (m?.tex && m.file)
        mat.map = this.getTexture(m.file, m.tex, false, Viewer.texFlip(fm?.pre));
      meshes.push(new THREE.Mesh(geom, mat));
      geoms.push(geom);
      matsOut.push(mat);
      matIdxOut.push(matIdx);
    });
    return { meshes, geoms, matsOut, matIdxOut };
  }

  /**
   * 构建静态武器网格：每个子网格一个 Mesh，共享同一份顶点池属性
   * （pos/normal/uv 直接取自文件，法线写入时归一化）。
   * mats[i] 为子网格 i 的贴图；挂点用小球标记（位置取 values 前 3 个）。
   */
  setFsm(model: FsmModel, mats: { tex: TexData | null; file: string | null }[]) {
    this.clearFsm();
    this.fsmModel = model;
    const { meshes, geoms, matsOut, matIdxOut } = this.buildFsmMeshes(model, mats);
    for (const mesh of meshes) {
      this.modelRoot.add(mesh);
      this.fsmMeshes.push(mesh);
    }
    this.fsmGeoms = geoms;
    this.fsmMats = matsOut;
    this.fsmMatIdx = matIdxOut;

    // 挂点标记：尺寸按网格包围盒自适应（J01 剑长约 124）
    const diag = Math.hypot(
      model.bboxMax[0] - model.bboxMin[0],
      model.bboxMax[1] - model.bboxMin[1],
      model.bboxMax[2] - model.bboxMin[2]
    );
    const r = Math.max(diag * 0.012, 0.5);
    const group = new THREE.Group();
    for (const d of model.dummies) {
      const geom = new THREE.SphereGeometry(r, 12, 8);
      const mat = new THREE.MeshBasicMaterial({ color: 0xff6a55 });
      const marker = new THREE.Mesh(geom, mat);
      marker.position.set(d.values[0], d.values[1], d.values[2]);
      marker.renderOrder = 2;
      group.add(marker);
      this.fsmDummyGeoms.push(geom);
      this.fsmDummyMats.push(mat);
    }
    this.modelRoot.add(group);
    this.fsmDummyGroup = group;
  }

  clearFsm() {
    for (const mesh of this.fsmMeshes) this.modelRoot.remove(mesh);
    for (const g of this.fsmGeoms) g.dispose();
    for (const m of this.fsmMats) m.dispose();
    this.fsmMeshes = [];
    this.fsmGeoms = [];
    this.fsmMats = [];
    this.fsmMatIdx = [];
    this.fsmModel = null;
    if (this.fsmDummyGroup) {
      this.modelRoot.remove(this.fsmDummyGroup);
      for (const g of this.fsmDummyGeoms) g.dispose();
      for (const m of this.fsmDummyMats) m.dispose();
      this.fsmDummyGroup = null;
    }
    this.fsmDummyGeoms = [];
    this.fsmDummyMats = [];
  }

  hasFsm() {
    return this.fsmMeshes.length > 0;
  }

  /** FSM 材质贴图热替换（检查器换图）：按材质序号更新全部引用该材质的子网格 */
  setFsmTexture(matIdx: number, file: string, tex: TexData) {
    const fm = this.fsmModel?.materials[matIdx];
    const mask = Viewer.texFlip(fm?.pre);
    for (let i = 0; i < this.fsmMats.length; i++) {
      if (this.fsmMatIdx[i] !== matIdx) continue;
      this.fsmMats[i].map = this.getTexture(file, tex, false, mask);
      this.fsmMats[i].needsUpdate = true;
    }
  }

  // ---------------- SFX 特效播放器 ----------------

  /**
   * 构建特效预览场景（2026-10-27 起按引擎真实形态）：Quad/未知类型 = 相机
   * 朝向公告板，尺寸/颜色取状态关键帧「长成后」值（引擎 sub_10095210 通道
   * {RGBA,半宽,半高}）；Cross = 两片垂直交叉面片（ground 追加水平第三片，
   * sub_10087E60）；Cylinder = 程序管网格（21 通道 sub_100898F0：半径/花边/
   * 锯齿/螺旋/上下 RGBA 顶点色）。节点按 prop 偏移摆放，挂路径名的节点位 =
   * 偏移 + EffectMotion 速度积分位置。根矩阵线性部分作用于整组、平移丢弃
   * （Combat 平移为编辑器残留）。U×V 帧网格贴图按引擎参数（fps/停帧）取
   * 子矩形翻页（MMUVPage 同语义）。
   */
  setSfx(model: SfxFile, nodeTex: { file: string | null; tex: TexData | null }[][]) {
    this.clearSfx();
    const group = new THREE.Group();
    this.sfxGroup = group;
    this.sfxMotions = makeMotionMap(model.paths);
    const boards = model.nodes
      .map((n, i) => ({ n, i, slots: nodeTex[i] ?? [] }))
      .filter((b) => b.n.textures.length > 0);
    boards.forEach((b) => {
      const n = b.n;
      const base = new THREE.Vector3(n.offset[0] || 0, n.offset[1] || 0, n.offset[2] || 0);
      const first = b.slots.find((s) => s?.tex);
      const baseTex = first?.tex && first.file ? this.getTexture(first.file, first.tex, true) : null;
      const grid: [number, number] = [
        Math.max(1, Math.round(n.grid?.[0] ?? 1)),
        Math.max(1, Math.round(n.grid?.[1] ?? 1)),
      ];
      const tex = baseTex ? this.sfxBoardTex(baseTex, grid) : null;
      const col = n.color ?? [1, 1, 1, 1];
      const rgba = [col[0] ?? 1, col[1] ?? 1, col[2] ?? 1, col[3] ?? 1];
      let obj: THREE.Object3D;
      let mat: THREE.SpriteMaterial | THREE.MeshBasicMaterial;
      let h: number;
      let bs: [number, number] = [1, 1];
      if (n.kind === "EffectCross" || (n.kind === "EffectCylinder" && n.cyl)) {
        // 世界固定面片/管（MeshBasic 共享材质，双面 + 叠加）
        mat = new THREE.MeshBasicMaterial({
          map: tex ?? undefined,
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
          side: THREE.DoubleSide,
        });
        if (n.kind === "EffectCross") {
          const [w, hh] = this.sfxQuadSize(n, first?.tex ?? null);
          const geo = new THREE.PlaneGeometry(w, hh);
          const g = new THREE.Group();
          g.add(new THREE.Mesh(geo, mat));
          const c = new THREE.Mesh(geo, mat);
          c.rotation.y = Math.PI / 2;
          g.add(c);
          if (n.ground) {
            const d = new THREE.Mesh(geo, mat);
            d.rotation.x = Math.PI / 2;
            g.add(d);
          }
          obj = g;
          h = hh / 2;
          bs = [w, hh];
          this.sfxTint(mat, rgba);
        } else {
          const c = n.cyl!;
          obj = new THREE.Mesh(Viewer.buildCylGeo(c), mat);
          mat.vertexColors = true;
          h = Math.max(40, Math.abs(c.topY - c.baseY) / 2 + Math.abs(c.riseBot));
          bs = [Math.max(c.rBot, c.rTop) * 2, Math.abs(c.topY - c.baseY)];
        }
      } else {
        // 公告板（Quad/粒子等未知几何类型回退）：尺寸按状态关键帧
        const [w, hh] = this.sfxQuadSize(n, first?.tex ?? null);
        mat = new THREE.SpriteMaterial({
          transparent: true,
          depthWrite: false,
          blending: THREE.AdditiveBlending,
          map: tex ?? undefined,
        });
        const sp = new THREE.Sprite(mat);
        sp.scale.set(w, hh, 1);
        obj = sp;
        h = hh / 2;
        bs = [w, hh];
        this.sfxTint(mat, rgba);
      }
      obj.position.copy(base);
      group.add(obj);
      // 名称标签：对应检查器节点序，普通混合不受叠加开关影响
      const { label, tex: labelTex } = this.sfxLabel(
        `#${b.i} ${n.kind}${n.name ? ` · ${n.name}` : ""}`
      );
      label.position.set(base.x, base.y - h - 26, base.z);
      group.add(label);
      this.sfxBoards.push({
        node: b.i,
        obj,
        mat,
        baseTex,
        tex,
        grid,
        fps: n.fps ?? 0,
        hold: !!n.hold,
        on: true,
        basePos: base,
        pathName: effPathName(n.pathName, this.sfxMotions),
        h,
        states: n.states ?? [],
        lastT: 0,
        rotAcc: 0,
        baseScale: bs,
        label,
        labelTex,
      });
    });
    // 根矩阵行主序、末行为平移（D3D 行向量约定）→ 转置进 three 列主序；平移丢弃见上
    const m = model.matrix;
    group.applyMatrix4(
      new THREE.Matrix4().set(
        m[0], m[4], m[8], 0,
        m[1], m[5], m[9], 0,
        m[2], m[6], m[10], 0,
        0, 0, 0, 1
      )
    );
    this.modelRoot.add(group);
    this.applySfxFrame(0);
  }

  /** 节点尺寸 [全宽, 全高]：状态关键帧「长成后」值优先，未知/为 0 回退 150×纵横比 */
  private sfxQuadSize(n: SfxFile["nodes"][number], tex: TexData | null): [number, number] {
    const s = n.size ?? [0, 0];
    const aspect = tex ? tex.width / tex.height : 1;
    const H = 150;
    return [s[0] > 1 ? s[0] : H * Math.min(Math.max(aspect, 0.25), 4), s[1] > 1 ? s[1] : H];
  }

  /** 材质整体着色（RGBA 关键帧；叠加混合下近黑视为无色防吞光） */
  private sfxTint(mat: THREE.SpriteMaterial | THREE.MeshBasicMaterial, rgba: number[]) {
    if (Math.max(rgba[0], rgba[1], rgba[2]) >= 0.02) mat.color.setRGB(rgba[0], rgba[1], rgba[2]);
    mat.opacity = Math.min(Math.max(rgba[3] ?? 1, 0.05), 1);
  }

  /** 板用贴图 = 缓存贴图克隆（独立 UV 变换，勿污染共享缓存）：U×V 分帧取子矩形 */
  private sfxBoardTex(base: THREE.Texture, grid: [number, number]): THREE.Texture {
    if (grid[0] <= 1 && grid[1] <= 1) return base;
    const t = base.clone();
    t.needsUpdate = true;
    t.repeat.set(1 / grid[0], 1 / grid[1]);
    t.offset.set(0, 1 - 1 / grid[1]);
    return t;
  }

  /**
   * EffectCylinder 程序管（引擎 sub_100898F0 同构）：θ=i/segs×wrap×2π，
   * 底/顶半径 = 基准 + θ×花边 − 偶段锯齿，底 Y = baseY+θ×rise，顶 Y = topY，
   * 底/顶部 RGBA 走顶点色（flipY 贴图 V: 底 0 顶 1，U 沿环绕平铺 uWrap 圈）
   */
  private static buildCylGeo(c: SfxCyl): THREE.BufferGeometry {
    const segs = Math.max(3, Math.min(96, Math.round(c.segs) || 20));
    const wrap = c.wrap > 0.01 && c.wrap < 50 ? c.wrap : 1;
    const uW = c.uWrap > 0.01 && c.uWrap < 50 ? c.uWrap : 1;
    const zb = c.rgbaBot ?? [1, 1, 1, 1];
    const zt = c.rgbaTop ?? [1, 1, 1, 1];
    const pos: number[] = [];
    const uv: number[] = [];
    const col: number[] = [];
    const idx: number[] = [];
    for (let i = 0; i <= segs; i++) {
      const th = (i / segs) * wrap * Math.PI * 2;
      const alt = i % 2 === 0 ? 1 : 0;
      const rb = Math.max(0, c.rBot + th * c.wobBot - alt * c.zigBot);
      const rt = Math.max(0, c.rTop + th * c.wobTop - alt * c.zigTop);
      pos.push(rb * Math.sin(th), c.baseY + th * c.riseBot, rb * Math.cos(th));
      pos.push(rt * Math.sin(th), c.topY, rt * Math.cos(th));
      const u = (i / segs) * uW;
      uv.push(u, 0, u, 1);
      col.push(zb[0], zb[1], zb[2], zb[3], zt[0], zt[1], zt[2], zt[3]);
    }
    for (let i = 0; i < segs; i++) {
      const a = i * 2;
      idx.push(a, a + 2, a + 1, a + 1, a + 2, a + 3);
    }
    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
    geo.setAttribute("uv", new THREE.Float32BufferAttribute(uv, 2));
    geo.setAttribute("color", new THREE.Float32BufferAttribute(col, 4));
    geo.setIndex(idx);
    return geo;
  }

  /** 节点标签（canvas 文本 Sprite） */
  private sfxLabel(text: string): { label: THREE.Sprite; tex: THREE.CanvasTexture } {
    const cv = document.createElement("canvas");
    cv.width = 512;
    cv.height = 64;
    const ctx = cv.getContext("2d")!;
    ctx.fillStyle = "rgba(16,16,20,0.72)";
    ctx.fillRect(0, 0, cv.width, cv.height);
    ctx.font = "28px sans-serif";
    ctx.fillStyle = "#9fe8c4";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(text.length > 30 ? text.slice(0, 29) + "…" : text, cv.width / 2, cv.height / 2);
    const tex = new THREE.CanvasTexture(cv);
    const label = new THREE.Sprite(
      new THREE.SpriteMaterial({ map: tex, transparent: true, depthWrite: false })
    );
    label.scale.set(160, 160 * (cv.height / cv.width), 1);
    return { label, tex };
  }

  /** 应用播放时刻（秒）：①Quad/Cross 状态关键帧插值（RGBA/尺寸，向下一
   * 关键帧过渡、末态回绕首态——引擎 Update 同口径）并按转速×dt 累计旋转
   * （公告板走平面内旋转、面片组/管绕世界 Y，与相机基矢的精确组合引擎
   * 侧未逆向完，主导通道口径已贴合光旋类观感）②U×V 帧网格节点按自身
   * 引擎帧率取子矩形翻页（播完停帧=钳末帧否则回绕） */
  applySfxFrame(tSec: number) {
    for (const b of this.sfxBoards) {
      const dt = tSec >= b.lastT && tSec - b.lastT < 1 ? tSec - b.lastT : Math.min(tSec, 0.05);
      b.lastT = tSec;
      if (b.states.length && b.states[0].dur > 0) {
        const total = b.states.reduce((s, k) => s + k.dur, 0);
        let t = ((tSec % total) + total) % total;
        let j = 0;
        while (j < b.states.length - 1 && t >= b.states[j].dur) {
          t -= b.states[j].dur;
          j++;
        }
        const cur = b.states[j];
        const nxt = b.states[j + 1] ?? b.states[0];
        const k = Math.min(Math.max(t / Math.max(cur.dur, 1e-4), 0), 1);
        const mix = (a: number, c: number) => (c - a) * k + a;
        const rgba = [0, 1, 2, 3].map((q) => mix(cur.rgba[q] ?? 1, nxt.rgba[q] ?? 1));
        this.sfxTint(b.mat, rgba);
        const w = mix(cur.size[0] || b.baseScale[0], nxt.size[0] || b.baseScale[0]);
        const hh = mix(cur.size[1] || b.baseScale[1], nxt.size[1] || b.baseScale[1]);
        const rot = [0, 1, 2].map((q) => mix(cur.rot[q] ?? 0, nxt.rot[q] ?? 0));
        const spin = rot[1] !== 0 ? rot[1] : rot[0] || rot[2];
        b.rotAcc += spin * dt;
        const sp = b.obj as THREE.Sprite;
        if (sp.isSprite) {
          sp.scale.set(Math.max(w, 1), Math.max(hh, 1), 1);
          (b.mat as THREE.SpriteMaterial).rotation = b.rotAcc;
        } else {
          const sw = Math.max(w / Math.max(b.baseScale[0], 1), 0.001);
          const sh = Math.max(hh / Math.max(b.baseScale[1], 1), 0.001);
          b.obj.scale.set(sw, sh, sw);
          b.obj.rotation.y = b.rotAcc;
        }
      }
      if (!b.tex || (b.grid[0] <= 1 && b.grid[1] <= 1)) continue;
      const total = b.grid[0] * b.grid[1];
      const fps = b.fps > 0.5 && b.fps < 240 ? b.fps : 12;
      let k2 = Math.floor(Math.max(0, tSec) * fps);
      k2 = b.hold ? Math.min(k2, total - 1) : ((k2 % total) + total) % total;
      const u = b.grid[0];
      b.tex.offset.set((k2 % u) / u, 1 - (Math.floor(k2 / u) + 1) / b.grid[1]);
    }
  }

  /** 路径动画推进（composable 播放循环逐帧调；帧网格翻页由 applySfxFrame 同拍驱动） */
  tickSfxMotion(dt: number) {
    if (!this.sfxGroup) return;
    for (const { p, sim } of this.sfxMotions.values()) tickMotionSim(p, sim, dt);
    for (const b of this.sfxBoards) {
      if (!b.pathName) continue;
      const sim = this.sfxMotions.get(b.pathName)?.sim;
      if (!sim) continue;
      b.obj.position.set(
        b.basePos.x + sim.pos.x,
        b.basePos.y + sim.pos.y,
        b.basePos.z + sim.pos.z
      );
      b.label.position.set(
        b.obj.position.x,
        b.obj.position.y - b.h - 26,
        b.obj.position.z
      );
    }
  }

  /** 路径动画复位（重播；setSfx 重建时随 clearSfx 一并清空） */
  resetSfxMotion() {
    for (const { sim } of this.sfxMotions.values()) {
      sim.t = 0;
      sim.idx = 0;
      sim.elapsed = 0;
      sim.done = false;
      sim.pos.set(0, 0, 0);
    }
    for (const b of this.sfxBoards) {
      b.obj.position.copy(b.basePos);
      b.label.position.set(b.basePos.x, b.basePos.y - b.h - 26, b.basePos.z);
    }
  }

  /** 节点显隐（检查器逐节点开关；展示体与标签一并隐藏） */
  setSfxNodeVisible(nodeIdx: number, on: boolean) {
    const b = this.sfxBoards.find((x) => x.node === nodeIdx);
    if (!b) return;
    b.on = on;
    b.obj.visible = on && !!b.tex;
    b.label.visible = on;
  }

  /** SFX 节点贴图热替换（检查器改路径后刷新展示体；slot 参数保留兼容恒 0） */
  setSfxTexture(nodeIdx: number, _slot: number, file: string, texData: TexData) {
    const b = this.sfxBoards.find((x) => x.node === nodeIdx);
    if (!b) return;
    const base = this.getTexture(file, texData, true);
    if (b.tex && b.tex !== b.baseTex) b.tex.dispose();
    b.baseTex = base;
    b.tex = this.sfxBoardTex(base, b.grid);
    b.mat.map = b.tex;
    b.mat.needsUpdate = true;
    b.obj.visible = b.on && !!b.tex;
  }

  /** 特效混合模式：叠加（发光贴图黑底加色，默认）↔ 普通透明 */
  setSfxBlending(additive: boolean) {
    for (const b of this.sfxBoards) {
      b.mat.blending = additive ? THREE.AdditiveBlending : THREE.NormalBlending;
      b.mat.needsUpdate = true;
    }
  }

  clearSfx() {
    if (this.sfxGroup) {
      this.modelRoot.remove(this.sfxGroup);
      this.sfxGroup = null;
    }
    for (const b of this.sfxBoards) {
      b.mat.dispose();
      if (b.tex && b.tex !== b.baseTex) b.tex.dispose();
      b.obj.traverse((o) => {
        const g = (o as THREE.Mesh).geometry;
        if (g) g.dispose();
      });
      b.labelTex.dispose();
      (b.label.material as THREE.SpriteMaterial).dispose();
    }
    this.sfxBoards = [];
    this.sfxMotions.clear();
  }

  hasSfx() {
    return !!this.sfxGroup;
  }

  // ---------------- 挂接特效播放器（角色叠加） ----------------

  /**
   * 添加挂接特效实例（同 id 先卸再建）。公告板口径：带贴图节点各一块
   * Sprite，按节点 prop+112 偏移落板（Quad/Cross 逆向确证——引擎公告板
   * 位 = 偏移 + EffectMotion 路径积分位置，贴游戏观感），叠加混合；
   * 根矩阵不参与（引擎挂接走 SetWorldMat 整体替换，Combat 根平移为
   * 编辑器残留）。EffectSound 不渲染（音频由检查器试听），EffectMesh
   * 的 .fak 暂不渲染。
   */
  addFx(
    id: number,
    model: SfxFile,
    nodeTex: { file: string | null; tex: TexData | null }[][],
    opts: {
      mode: "bone" | "world";
      bone: string;
      worldPos: [number, number, number];
      offset: [number, number, number];
      euler: [number, number, number];
      scale: number;
      additive: boolean;
      visible: boolean;
      nodeMute?: boolean[];
    }
  ) {
    this.removeFx(id);
    const group = new THREE.Group();
    group.matrixAutoUpdate = false;
    const boards: FxItemObj["boards"] = [];
    const motions = makeMotionMap(model.paths);
    model.nodes.forEach((n, i) => {
      const slots = nodeTex[i] ?? [];
      if (!n.textures.length || !slots.length) return;
      const mat = new THREE.SpriteMaterial({
        transparent: true,
        depthWrite: false,
        blending: opts.additive ? THREE.AdditiveBlending : THREE.NormalBlending,
      });
      const frames = slots.map((s) =>
        s?.tex && s.file ? this.getTexture(s.file, s.tex, true) : null
      );
      const first = slots.find((s) => s?.tex);
      const aspect = first?.tex ? first.tex.width / first.tex.height : 1;
      const H = Viewer.FX_BOARD_H;
      const sprite = new THREE.Sprite(mat);
      sprite.scale.set(H * Math.min(Math.max(aspect, 0.25), 4), H, 1);
      sprite.raycast = () => {}; // 不参与网格拾取（点击选中部件不被特效遮挡）
      group.add(sprite);
      const on = !opts.nodeMute?.[i];
      const basePos = new THREE.Vector3(n.offset[0] || 0, n.offset[1] || 0, n.offset[2] || 0);
      sprite.position.copy(basePos);
      boards.push({ node: i, sprite, mat, frames, cur: null, on, basePos, pathName: effPathName(n.pathName, motions) });
    });
    this.modelRoot.add(group);
    const item = {
      id,
      group,
      boards,
      motions,
      mode: opts.mode,
      boneIdx: -1,
      boneName: opts.mode === "bone" ? opts.bone : "",
      worldPos: [...opts.worldPos] as [number, number, number],
      offset: [...opts.offset] as [number, number, number],
      eulerRad: opts.euler.map((d) => (d * Math.PI) / 180) as [number, number, number],
      scale: opts.scale,
      visible: opts.visible,
    };
    this.fxItems.set(id, item);
    this.resolveFxBone(item);
    group.visible = opts.visible;
    this.updateFxMatrix(item);
    this.applyFxFrame(id, 0);
  }

  /** 路径动画推进（composable 播放循环逐帧调，dt 已乘速度） */
  updateFxMotion(id: number, dt: number) {
    const item = this.fxItems.get(id);
    if (!item) return;
    for (const { p, sim } of item.motions.values()) tickMotionSim(p, sim, dt);
    for (const b of item.boards) {
      if (!b.pathName) continue;
      const sim = item.motions.get(b.pathName)?.sim;
      if (!sim) continue;
      b.sprite.position.set(
        b.basePos.x + sim.pos.x,
        b.basePos.y + sim.pos.y,
        b.basePos.z + sim.pos.z
      );
    }
  }

  /** 路径动画复位（重播：模拟态清零、板回偏移位） */
  resetFxMotion(id: number) {
    const item = this.fxItems.get(id);
    if (!item) return;
    for (const { sim } of item.motions.values()) {
      sim.t = 0;
      sim.idx = 0;
      sim.elapsed = 0;
      sim.done = false;
      sim.pos.set(0, 0, 0);
    }
    for (const b of item.boards) b.sprite.position.copy(b.basePos);
  }

  /** 按名解析骨序（大小写不敏感；未加载/找不到 = -1，特效落角色原点） */
  private resolveFxBone(item: { boneName: string; boneIdx: number }) {
    item.boneIdx = -1;
    if (!item.boneName) return;
    const lower = item.boneName.toLowerCase();
    item.boneIdx = this.skelBones.findIndex((b) => b.name.toLowerCase() === lower);
  }

  /**
   * 重算单个实例的组矩阵。挂骨模式 = modelRoot 世界逆 × 骨世界矩阵（骨骼在
   * scene 数据空间、渲染对象统一挂 modelRoot 带 MODEL_YAW，与蒙皮件同口径
   * 换系）× 骨局部修正 T·R·S；世界模式 = T(worldPos)·R·S 直摆（Combat/Scn
   * 特效的引擎语义即客户端 SetWorldMat 实体位，这里按角色原点相对摆放）。
   */
  private updateFxMatrix(item: FxItemObj) {
    if (item.mode === "bone" && item.boneIdx >= 0 && this.skelBones[item.boneIdx]) {
      const b = this.skelBones[item.boneIdx];
      b.updateWorldMatrix(true, false);
      this.fxTmpA.copy(this.modelRoot.matrixWorld).invert().multiply(b.matrixWorld);
    } else {
      this.fxTmpA.makeTranslation(item.worldPos[0], item.worldPos[1], item.worldPos[2]);
    }
    this.fxTmpE.set(item.eulerRad[0], item.eulerRad[1], item.eulerRad[2]);
    this.fxTmpQ.setFromEuler(this.fxTmpE);
    this.fxTmpB.compose(
      this.fxTmpV.set(item.offset[0], item.offset[1], item.offset[2]),
      this.fxTmpQ,
      this.fxTmpS.setScalar(item.scale)
    );
    item.group.matrix.copy(this.fxTmpA).multiply(this.fxTmpB);
    item.group.matrixWorldNeedsUpdate = true;
  }

  /** 全实例矩阵刷新（composable 每帧调用：暂停时特效仍须跟随骨骼动画） */
  updateFxMatrices() {
    if (!this.fxItems.size) return;
    this.modelRoot.updateWorldMatrix(true, false);
    for (const item of this.fxItems.values()) this.updateFxMatrix(item);
  }

  /** 应用播放帧：各节点按自身帧数取模翻页（短序列提前循环），尊重节点显隐 */
  applyFxFrame(id: number, frame: number) {
    const item = this.fxItems.get(id);
    if (!item) return;
    for (const b of item.boards) {
      if (!b.frames.length) continue;
      const i = ((frame % b.frames.length) + b.frames.length) % b.frames.length;
      const t = b.frames[i] ?? null;
      if (t !== b.cur) {
        b.cur = t;
        b.mat.map = t ?? undefined;
        b.mat.needsUpdate = true;
      }
      b.sprite.visible = item.visible && b.on && !!t;
    }
  }

  /** 更新实例参数（bones 换名重解析；additive/visible 即时生效） */
  setFxTransform(
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
    const item = this.fxItems.get(id);
    if (!item) return;
    if (t.mode !== undefined) item.mode = t.mode;
    if (t.bone !== undefined) item.boneName = t.mode === "world" ? "" : t.bone;
    if (t.mode === "world") item.boneName = "";
    if (t.worldPos) item.worldPos = [...t.worldPos] as [number, number, number];
    if (t.offset) item.offset = [...t.offset] as [number, number, number];
    if (t.euler)
      item.eulerRad = t.euler.map((d) => (d * Math.PI) / 180) as [number, number, number];
    if (t.scale !== undefined) item.scale = t.scale;
    if (t.additive !== undefined)
      for (const b of item.boards) {
        b.mat.blending = t.additive ? THREE.AdditiveBlending : THREE.NormalBlending;
        b.mat.needsUpdate = true;
      }
    if (t.visible !== undefined) item.visible = t.visible;
    this.resolveFxBone(item);
    item.group.visible = item.visible;
    this.updateFxMatrix(item);
  }

  /** 节点显隐（nodeMute 反向口：on=false 即静音该节点） */
  setFxNodeVisible(id: number, nodeIdx: number, on: boolean) {
    const item = this.fxItems.get(id);
    const b = item?.boards.find((x) => x.node === nodeIdx);
    if (!b) return;
    b.on = on;
    b.sprite.visible = item.visible && on && !!b.cur;
  }

  /** 骨架加载/切换/清空后按名重挂全部实例（refreshBoneAttached 同源钩子） */
  private bindFxToSkeleton() {
    for (const item of this.fxItems.values()) {
      this.resolveFxBone(item);
      this.updateFxMatrix(item);
    }
  }

  removeFx(id: number) {
    const item = this.fxItems.get(id);
    if (!item) return;
    this.modelRoot.remove(item.group);
    for (const b of item.boards) b.mat.dispose();
    this.fxItems.delete(id);
  }

  clearFx() {
    for (const id of [...this.fxItems.keys()]) this.removeFx(id);
  }

  hasFx(id: number) {
    return this.fxItems.has(id);
  }

  /** 视口内是否有骨架（composable 据此判断特效可重挂：角色在场） */
  skeletonReady() {
    return this.skelBones.length > 0;
  }

  // ---------------- 骨骼挂接网格（帽子等 FSM 静态件，统一进 parts 管线） ----------------

  /** 挂接件元数据。渲染不再走独立的 Group 层级/逐帧矩阵——全部在构建期把
   *  挂接矩阵（mount = Yaw·boneW0·锚点补偿·姿态旋转+偏移，见 mountOf）烘焙
   *  进顶点，做成合成蒙皮部件经 addPart 进 parts 管线：刚性帽单骨 w=1
   *  （LBS 单骨 = boneW(t)·boneW0⁻¹·pw，与旧 holder 每帧相对运动逐点等价），
   *  长发 FSM 多骨自动绑骨（buildAutoHairPart）。逐帧与身体件同一 skinPart
   *  LBS（含 fmath wasm 内核）+ 显隐/取景/拾取一致；显示系 Yaw 共轭问题
   *  一并消失（顶点在数据空间建模，显示经 modelRoot，与身体件同口径）。 */
  private boneAttached = new Map<
    string,
    {
      model: FsmModel;
      mats: { tex: TexData | null; file: string | null }[];
      boneName: string;
      /** 面板微调偏移（inner 系）：变更经 setBoneAttachedOffset 重建烘焙 */
      offset: [number, number, number];
      /** 解析到的挂接骨实例（骨架重载后重找）；null = 静态回落（零权重
       *  保持绑定姿势 = 旧静态摆放） */
      bone: THREE.Bone | null;
      /** 长发 FSM（多骨自动绑骨）；false = 刚性帽（单骨 w=1） */
      autoHair: boolean;
      /** 网格原点落点（绑定姿态、显示空间，挂接日志口径），构建期更新 */
      origin: THREE.Vector3 | null;
    }
  >();

  /** 挂点骨名解析：精确（忽略大小写）→ 包含匹配；找不到 null */
  private resolveMountBone(boneName: string): THREE.Bone | null {
    const lower = boneName.toLowerCase();
    return (
      this.skelBones.find((b) => b.name.toLowerCase() === lower) ??
      this.skelBones.find((b) => b.name.toLowerCase().includes(lower)) ??
      null
    );
  }

  /** 挂接基准（数据空间）：fix = Yaw·骨绑定姿态·锚点补偿（tag 挂点骨 =
   *  TAG_MOUNT_OFFSET，普通骨回落 = HAT_OFFSET）；mount = fix·inner
   *  （inner = 姿态旋转 + 偏移：tag 转 Ry(π) 前后归位，回落转 Rz(−π/2)）。
   *  origin = fix·offset（网格原点落点，不含 inner 旋转，日志口径） */
  private mountOf(
    bone: THREE.Bone,
    offset: [number, number, number]
  ): { mount: THREE.Matrix4; origin: THREE.Vector3 } {
    const idx = this.skelBones.indexOf(bone);
    const bindW = (idx >= 0 ? this.bindWorldMats[idx] : null) ?? bone.matrixWorld;
    const isTag = bone.name.toLowerCase().startsWith("tag_");
    const anchor = isTag ? Viewer.TAG_MOUNT_OFFSET : Viewer.HAT_OFFSET;
    const fix = new THREE.Matrix4().makeRotationY(MODEL_YAW).multiply(bindW);
    fix.multiply(new THREE.Matrix4().makeTranslation(anchor.x, anchor.y, anchor.z));
    const inner = new THREE.Matrix4()
      .makeRotationY(isTag ? Math.PI : 0)
      .setPosition(offset[0], offset[1], offset[2]);
    if (!isTag) inner.multiply(new THREE.Matrix4().makeRotationZ(-Math.PI / 2));
    return {
      mount: fix.clone().multiply(inner),
      origin: new THREE.Vector3(offset[0], offset[1], offset[2]).applyMatrix4(fix),
    };
  }

  /**
   * FSM 网格挂到骨骼（帽子槽）：经 mount 矩阵烘焙成合成蒙皮部件进 parts
   * 管线（见 boneAttached 注释）——刚性帽单骨 w=1 随骨刚体运动；长发 FSM
   * （FM38 实测 y 跨 87 vs 普通帽 29）自动绑骨成多骨合成件走 LBS（近头随
   * 头、发束随链动画；原版引擎对 FSM 帽是 SGAvatarAttachment 刚挂，游戏里
   * 长发同样刚性穿模，此为 viewer 增强）。骨架未加载或骨名不存在时零权重
   * 保持绑定姿势（= 旧静态摆放）。offset 为 inner 系微调：tag 挂点骨下
   * Y=上下（tag 帧 +Y 朝上），普通 head 骨回落路径 X=沿骨轴上下。
   */
  addBoneAttached(
    path: string,
    model: FsmModel,
    mats: { tex: TexData | null; file: string | null }[],
    boneName: string,
    offset: [number, number, number] = [0, 0, 0]
  ) {
    this.removeBoneAttached(path);
    const autoHair =
      this.isLongHairFsm(model) && this.skelBones.length > 0 && !!this.resolveMountBone(boneName);
    this.boneAttached.set(path, {
      model,
      mats,
      boneName,
      offset,
      bone: null,
      autoHair,
      origin: null,
    });
    if (autoHair) this.buildAutoHairPart(path);
    else this.buildRigidHatPart(path);
    // 挂接监控：晚到装配/骨缺失/多帽叠加（幽灵）都在此暴露，__diagLog 可查。
    // 位置为显示空间、绑定姿态（帧 0）数值
    const e = this.boneAttached.get(path)!;
    const tail = path.replace(/^.*[\\/]/, "");
    if (!e.bone) {
      this.dwarn(`帽子挂接: ${tail} 骨「${boneName}」不存在，回落静态摆放`);
    } else if (!autoHair) {
      // 报绑定姿态（帧 0）数值：与挂接时刻无关、可复现；播放中帽子随头部
      // 运动，实时位置用 boneAttachedInfo() 查
      const idx = this.skelBones.indexOf(e.bone);
      const bindW = (idx >= 0 ? this.bindWorldMats[idx] : null) ?? e.bone.matrixWorld;
      const anchor = new THREE.Vector3().setFromMatrixPosition(bindW);
      const f = (p: THREE.Vector3) => `${p.x.toFixed(1)},${p.y.toFixed(1)},${p.z.toFixed(1)}`;
      this.dlog(
        `帽子挂接: ${tail} → 骨 ${e.bone.name} 锚(${f(anchor)}) · 原点(${e.origin ? f(e.origin) : "-"}) · 偏移 [${offset.join(",")}]`
      );
    }
    if (this.boneAttached.size > 1) {
      this.dwarn(
        `帽子挂接: 同时存在 ${this.boneAttached.size} 顶（${[...this.boneAttached.keys()]
          .map((k) => k.replace(/^.*[\\/]/, ""))
          .join(" + ")}），疑似幽灵叠加`
      );
    }
  }

  /** 挂接状态快照（CDP/诊断用）：`__viewer.boneAttachedInfo()` 直查。
   *  显示空间坐标；origin=网格原点落点（绑定姿态、构建期烘焙口径），
   *  bbox=合成部件当前蒙皮位置的世界包围盒 min/max */
  boneAttachedInfo(): {
    path: string;
    bone: string | null;
    anchor: number[] | null;
    rotY: number;
    offset: number[];
    origin: number[] | null;
    bbox: number[][] | null;
  }[] {
    return [...this.boneAttached.entries()].map(([path, e]) => {
      const p = this.parts.get(path);
      const box = new THREE.Box3();
      let has = false;
      if (p) {
        p.mesh.updateMatrixWorld(true);
        p.mesh.geometry.computeBoundingBox();
        if (p.mesh.geometry.boundingBox) {
          box.copy(p.mesh.geometry.boundingBox).applyMatrix4(p.mesh.matrixWorld);
          has = true;
        }
      }
      const a = e.bone
        ? new THREE.Vector3().setFromMatrixPosition(e.bone.matrixWorld)
        : null;
      const rotY = e.bone?.name.toLowerCase().startsWith("tag_") ? Math.PI : 0;
      return {
        path,
        bone: e.bone?.name ?? null,
        anchor: a ? [+a.x.toFixed(1), +a.y.toFixed(1), +a.z.toFixed(1)] : null,
        rotY: +rotY.toFixed(3),
        offset: e.offset.map((n) => +n.toFixed(2)),
        origin: e.origin
          ? [+e.origin.x.toFixed(1), +e.origin.y.toFixed(1), +e.origin.z.toFixed(1)]
          : null,
        bbox: has
          ? [
              [+box.min.x.toFixed(1), +box.min.y.toFixed(1), +box.min.z.toFixed(1)],
              [+box.max.x.toFixed(1), +box.max.y.toFixed(1), +box.max.z.toFixed(1)],
            ]
          : null,
      };
    });
  }

  /** 帽子网格 vs 身体部件的穿模监控（bbox 级近似：落在某身体部件世界 AABB
   *  内的帽子顶点数与最深下探量；bbox 内 ≠ 真穿面，作观察指标用）。
   *  刚体 FSM 帽（引擎同款挂接）长发的固有穿模靠它量化定位。
   *  帽=挂接合成部件，身体=其余部件（长发合成件保留为障碍，与旧口径同） */
  hatOverlapInfo(): {
    hat: string;
    part: string;
    vertsInBox: number;
    deepestY: number;
  }[] {
    const out: { hat: string; part: string; vertsInBox: number; deepestY: number }[] =
      [];
    const v = new THREE.Vector3();
    for (const hatPath of this.boneAttached.keys()) {
      const hp = this.parts.get(hatPath);
      if (!hp?.mesh?.geometry) continue;
      hp.mesh.updateMatrixWorld(true);
      const hatName = hatPath.replace(/^.*[\\/]/, "");
      for (const p of this.parts.values()) {
        if (p === hp || p.syntheticMount || !p.mesh?.geometry) continue;
        p.mesh.updateMatrixWorld(true);
        p.mesh.geometry.computeBoundingBox();
        const bb = p.mesh.geometry.boundingBox!.clone().applyMatrix4(
          p.mesh.matrixWorld
        );
        bb.expandByScalar(-0.5); // 收缩排除贴面顶点
        let n = 0;
        let deepest = 1e9;
        const pos = hp.mesh.geometry.attributes.position;
        for (let i = 0; i < pos.count; i++) {
          v.fromBufferAttribute(pos, i).applyMatrix4(hp.mesh.matrixWorld);
          if (bb.containsPoint(v)) {
            n++;
            deepest = Math.min(deepest, v.y);
          }
        }
        if (n > 0) out.push({ hat: hatName, part: p.model.path.replace(/^.*[\\/]/, ""), vertsInBox: n, deepestY: +deepest.toFixed(1) });
      }
    }
    if (out.length) {
      this.dwarn(
        `帽子穿模监控: ${out
          .map((o) => `${o.hat}×${o.part} ${o.vertsInBox}顶点(至y${o.deepestY})`)
          .join(" · ")}`
      );
    }
    return out;
  }

  /** 长发 FSM 判定：网格 Y 跨度 > 50（FM38 披发 87 vs 普通帽 29） */
  private isLongHairFsm(m: FsmModel): boolean {
    let mn = Infinity;
    let mx = -Infinity;
    for (let i = 1; i < m.vertices.length; i += 8) {
      const y = m.vertices[i];
      if (y < mn) mn = y;
      if (y > mx) mx = y;
    }
    return mx - mn > 50;
  }

  /** 长发 FSM → 合成蒙皮部件：顶点经挂接矩阵（mountOf，与刚性帽同口径）
   *  变换到数据空间，最近分配到 头/颈骨+tag 挂点骨+Bone* 发链骨（前二骨
   *  距离加权平滑），boneLocalPos = 绑定帧世界矩阵逆 × 世界位置。骨架重载
   *  后由 refreshBoneAttached 重建（lp 相对新骨架重新生成）。 */
  private buildAutoHairPart(path: string) {
    const e = this.boneAttached.get(path);
    if (!e?.autoHair || !this.skelBones.length || !this.bindWorldMats.length) return;
    // 每次构建按名重解析骨实例（骨架重载后旧 Bone 已脱离层级，沿旧实例
    // 会取到陈旧 matrixWorld）
    const bone = this.resolveMountBone(e.boneName);
    if (!bone) return;
    e.bone = bone;
    const { model, mats, offset } = e;
    const idx = this.skelBones.indexOf(bone);
    // 挂接基准 mountOf（含 TAG_MOUNT_OFFSET 标定：那是显示空间的挂点标定，
    // 长发同样适用——实测去掉后 crown 整体偏后~6.5 偏上 0.5 悬空）。长发与
    // 帽的差别不在挂点，而在绑定：长发多骨最近分配，帽单骨 w=1 刚挂
    const { mount, origin } = this.mountOf(bone, offset);
    e.origin = origin; // 诊断口径与刚性帽一致（绑定姿态网格原点落点）

    const n = this.fskelBoneCount;
    // 候选骨：只用头/颈骨 + tag 挂点骨——游戏内 FM38 类长发 FSM 是
    // SGAvatarAttachment 刚挂（引擎无 FSM 蒙皮），发帘随头整体运动、从挂点
    // 垂直下坠（authored 网格自带游戏里的大间隙与全长，参照游戏截图）。
    // Ponytail/Bone* 链都不参与：链弧线会把发帘拉去包背（实测贴背无隙）。
    const cand: number[] = [];
    for (let b = 0; b < n; b++) {
      const nm = this.skelBones[b]?.name ?? "";
      if (/head|neck/i.test(nm) || b === idx) cand.push(b);
    }

    // 身体间隙外推：FM38 类长发按原厂挂接姿态本就贴背/嵌背（刚挂时代游戏
    // 同样穿）。合成时以其余部件的绑定姿态三角形（authored 位置）为障碍，
    // 把发顶点推到体表外 HAIR_BODY_MARGIN——体内点（射线奇偶判定）沿
    // (q−p) 推出、体外近点沿 (p−q) 推开；随后沿三角形邻接做两轮均值松弛
    // 保持发面平整（避免逐点推出起皱）。
    {
      const MARGIN = Viewer.HAIR_BODY_MARGIN;
      const tris: number[] = [];
      for (const p2 of this.parts.values()) {
        // 障碍只取身体件：偏移触发的重建路径里旧发件要到 addPart 才被替换，
        // 若算进障碍会把新发从自身旧壳上再推一层 MARGIN（逐次调整爆跳根因）；
        // 合成挂接帽同样排除（此前非部件、从未进障碍池，口径不变）
        if (p2.mesh.visible === false || p2.syntheticHair || p2.syntheticMount) continue;
        // 障碍三角形用模型 authored 索引（几何索引含拆缝副本，越界 model.vertices）
        for (const sm of p2.model.submeshes) {
          for (let t = 0; t + 2 < sm.triangles.length; t += 3) {
            const va = p2.model.vertices[sm.triangles[t]]?.pos;
            const vb = p2.model.vertices[sm.triangles[t + 1]]?.pos;
            const vc = p2.model.vertices[sm.triangles[t + 2]]?.pos;
            if (!va || !vb || !vc) continue;
            tris.push(va[0], va[1], va[2], vb[0], vb[1], vb[2], vc[0], vc[1], vc[2]);
          }
        }
      }
      if (tris.length) {
        const nv = model.vertexCount;
        const pwArr: THREE.Vector3[] = [];
        for (let i = 0; i < nv; i++) {
          pwArr.push(
            new THREE.Vector3(
              model.vertices[i * 8],
              model.vertices[i * 8 + 1],
              model.vertices[i * 8 + 2]
            ).applyMatrix4(mount)
          );
        }
        // 射线奇偶（自头顶 +Y 向下）体内判定 + 最近三角形点
        const insideOf = (pnt: THREE.Vector3): boolean => {
          let hits = 0;
          const ay = 400;
          for (let i = 0; i < tris.length; i += 9) {
            // AABB 粗筛：三角形在点 xy 附近
            const minX = Math.min(tris[i], tris[i + 3], tris[i + 6]);
            const maxX = Math.max(tris[i], tris[i + 3], tris[i + 6]);
            const minZ = Math.min(tris[i + 2], tris[i + 5], tris[i + 8]);
            const maxZ = Math.max(tris[i + 2], tris[i + 5], tris[i + 8]);
            if (pnt.x < minX - 0.1 || pnt.x > maxX + 0.1 || pnt.z < minZ - 0.1 || pnt.z > maxZ + 0.1) continue;
            // Möller–Trumbore：自 (x,400,z) 向 -Y
            const e1x = tris[i + 3] - tris[i], e1y = tris[i + 4] - tris[i + 1], e1z = tris[i + 5] - tris[i + 2];
            const e2x = tris[i + 6] - tris[i], e2y = tris[i + 7] - tris[i + 1], e2z = tris[i + 8] - tris[i + 2];
            const dx = 0, dy = -1, dz = 0;
            const pxx = dy * e2z - dz * e2y, pxy = dz * e2x - dx * e2z, pxz = dx * e2y - dy * e2x;
            const det = e1x * pxx + e1y * pxy + e1z * pxz;
            if (det > -1e-9 && det < 1e-9) continue;
            const inv = 1 / det;
            const tx = pnt.x - tris[i], ty = ay - tris[i + 1], tz = pnt.z - tris[i + 2];
            const u = (tx * pxx + ty * pxy + tz * pxz) * inv;
            if (u < 0 || u > 1) continue;
            const qxx = ty * e1z - tz * e1y, qxy = tz * e1x - tx * e1z, qxz = tx * e1y - ty * e1x;
            const vv = (dx * qxx + dy * qxy + dz * qxz) * inv;
            if (vv < 0 || u + vv > 1) continue;
            const t = (e2x * qxx + e2y * qxy + e2z * qxz) * inv;
            if (t > 1e-4 && t < 1 - 1e-4) hits++;
          }
          return hits % 2 === 1;
        };
        const nearest = (pnt: THREE.Vector3, out: THREE.Vector3): number => {
          let best = Infinity;
          for (let i = 0; i < tris.length; i += 9) {
            const cx = (tris[i] + tris[i + 3] + tris[i + 6]) / 3;
            const cy = (tris[i + 1] + tris[i + 4] + tris[i + 7]) / 3;
            const cz = (tris[i + 2] + tris[i + 5] + tris[i + 8]) / 3;
            const dd = (pnt.x - cx) ** 2 + (pnt.y - cy) ** 2 + (pnt.z - cz) ** 2;
            if (dd > (best + 4) * (best + 4)) continue;
            // 点到三角形（Ericson）
            const ax = tris[i], ay2 = tris[i + 1], az = tris[i + 2];
            const abx = tris[i + 3] - ax, aby = tris[i + 4] - ay2, abz = tris[i + 5] - az;
            const acx = tris[i + 6] - ax, acy = tris[i + 7] - ay2, acz = tris[i + 8] - az;
            const apx = pnt.x - ax, apy = pnt.y - ay2, apz = pnt.z - az;
            const d1 = apx * abx + apy * aby + apz * abz;
            const d2 = apx * acx + apy * acy + apz * acz;
            let qx: number, qy: number, qz: number;
            if (d1 <= 0 && d2 <= 0) { qx = ax; qy = ay2; qz = az; }
            else {
              const bpx = pnt.x - tris[i + 3], bpy = pnt.y - tris[i + 4], bpz = pnt.z - tris[i + 5];
              const d3 = bpx * abx + bpy * aby + bpz * abz;
              const d4 = bpx * acx + bpy * acy + bpz * acz;
              if (d3 >= 0 && d4 <= d3) { qx = tris[i + 3]; qy = tris[i + 4]; qz = tris[i + 5]; }
              else {
                const vc = d1 * d4 - d3 * d2;
                if (vc <= 0 && d1 >= 0 && d3 <= 0) {
                  const t = d1 / (d1 - d3);
                  qx = ax + abx * t; qy = ay2 + aby * t; qz = az + abz * t;
                } else {
                  const cpx = pnt.x - tris[i + 6], cpy = pnt.y - tris[i + 7], cpz = pnt.z - tris[i + 8];
                  const d5 = cpx * abx + cpy * aby + cpz * abz;
                  const d6 = cpx * acx + cpy * acy + cpz * acz;
                  if (d6 >= 0 && d5 <= d6) { qx = tris[i + 6]; qy = tris[i + 7]; qz = tris[i + 8]; }
                  else {
                    const vb2 = d5 * d2 - d1 * d6;
                    if (vb2 <= 0 && d2 >= 0 && d6 <= 0) {
                      const t = d2 / (d2 - d6);
                      qx = ax + acx * t; qy = ay2 + acy * t; qz = az + acz * t;
                    } else {
                      const va2 = d3 * d6 - d5 * d4;
                      if (va2 <= 0 && d4 - d3 >= 0 && d5 - d6 >= 0) {
                        const t = (d4 - d3) / (d4 - d3 + (d5 - d6));
                        qx = tris[i + 3] + (tris[i + 6] - tris[i + 3]) * t;
                        qy = tris[i + 4] + (tris[i + 7] - tris[i + 4]) * t;
                        qz = tris[i + 5] + (tris[i + 8] - tris[i + 5]) * t;
                      } else {
                        const denom = 1 / (va2 + vb2 + vc);
                        const v = vb2 * denom, w = vc * denom;
                        qx = ax + abx * v + acx * w;
                        qy = ay2 + aby * v + acy * w;
                        qz = az + abz * v + acz * w;
                      }
                    }
                  }
                }
              }
            }
            const dd2 = (pnt.x - qx) ** 2 + (pnt.y - qy) ** 2 + (pnt.z - qz) ** 2;
            if (dd2 < best) { best = dd2; out.set(qx, qy, qz); }
          }
          return Math.sqrt(best);
        };
        const q = new THREE.Vector3();
        const offs: (THREE.Vector3 | null)[] = pwArr.map(() => null);
        let pushed = 0;
        for (let i = 0; i < nv; i++) {
          const pw = pwArr[i];
          const d = nearest(pw, q);
          // 体内检测窗放宽到 12：深嵌体内的发点（时装身体更薄，可距体表
          // 3~6）在 2×MARGIN 窗外曾被当体外远处漏推（套装场景只推 72 个
          // 的根因）
          const ins = d < 12 ? insideOf(pw) : false;
          if (ins || d < MARGIN) {
            const dir = new THREE.Vector3();
            if (ins) dir.subVectors(q, pw); // 体内：从最近表面点指向外
            else dir.subVectors(pw, q); // 体外近面：沿远离方向
            if (dir.lengthSq() < 1e-10) continue;
            dir.normalize();
            const target = new THREE.Vector3().copy(q).addScaledVector(dir, MARGIN);
            offs[i] = target.sub(pw);
            pushed++;
          }
        }
        if (pushed) {
          // 邻接平滑（三角形边）：offset 向邻居均值靠拢，两轮
          const nbr: Set<number>[] = Array.from({ length: nv }, () => new Set<number>());
          for (const sm of model.submeshes) {
            for (let t = 0; t + 2 < sm.triangles.length; t += 3) {
              const a = sm.triangles[t], b2 = sm.triangles[t + 1], c2 = sm.triangles[t + 2];
              nbr[a]?.add(b2); nbr[a]?.add(c2);
              nbr[b2]?.add(a); nbr[b2]?.add(c2);
              nbr[c2]?.add(a); nbr[c2]?.add(b2);
            }
          }
          for (let pass = 0; pass < 2; pass++) {
            const next = offs.map((o, i) => {
              if (!o) return o;
              const avg = o.clone();
              let cnt = 1;
              for (const j of nbr[i]) {
                const oj = offs[j];
                if (oj) { avg.add(oj); cnt++; }
              }
              return avg.divideScalar(cnt).lerp(o, 0.4); // 部分回弹保间隙
            });
            for (let i = 0; i < nv; i++) offs[i] = next[i];
          }
          for (let i = 0; i < nv; i++) {
            const o = offs[i];
            if (o) pwArr[i].add(o);
          }
        }
        // 推开后的位置作为最终绑定位置（挂在 model 上的非类型化字段）
        (model as FsmModel & { __pushedPos?: THREE.Vector3[]; __pushedCount?: number }).__pushedPos = pwArr;
        (model as FsmModel & { __pushedPos?: THREE.Vector3[]; __pushedCount?: number }).__pushedCount = pushed;
        (model as FsmModel & { __restPos?: Float64Array }).__restPos = new Float64Array(
          pwArr.flatMap((w) => [w.x, w.y, w.z])
        );
      }
    }
    // 顶点绑骨：最近两骨距离加权（d2 < d1·1.5 时平滑，否则单骨 w=1）
    const boneCount = new Map<number, number>();
    const nv = model.vertexCount;
    const pushedPos = (model as FsmModel & { __pushedPos?: THREE.Vector3[] }).__pushedPos;
    const synth = this.synthFromFsm(
      path + "#autohair",
      model,
      mats,
      mount,
      (pw, lpOf) => {
        let b1 = cand[0];
        let b2 = cand[1] ?? cand[0];
        let d1 = Infinity;
        let d2 = Infinity;
        for (const b of cand) {
          const bm = this.bindWorldMats[b].elements;
          const d = Math.hypot(pw.x - bm[12], pw.y - bm[13], pw.z - bm[14]);
          if (d < d1) {
            b2 = b1;
            d2 = d1;
            b1 = b;
            d1 = d;
          } else if (d < d2) {
            b2 = b;
            d2 = d;
          }
        }
        const influences: Influence[] = [];
        if (d2 < d1 * 1.5) {
          const w1 = d2 / (d1 + d2);
          influences.push({ boneIndex: b1, weight: w1, boneLocalPos: lpOf(b1, pw) });
          influences.push({ boneIndex: b2, weight: 1 - w1, boneLocalPos: lpOf(b2, pw) });
          boneCount.set(b1, (boneCount.get(b1) ?? 0) + 1);
          boneCount.set(b2, (boneCount.get(b2) ?? 0) + 1);
        } else {
          influences.push({ boneIndex: b1, weight: 1, boneLocalPos: lpOf(b1, pw) });
          boneCount.set(b1, (boneCount.get(b1) ?? 0) + 1);
        }
        return influences;
      },
      pushedPos
    );
    this.addPart(path, synth.model, synth.texes, synth.texFiles, {
      uvSeamSplit: false, // FSM 图集无平铺接缝，拆分会误改 authored 大跨度采样弧
      texMask: synth.texMask, // mask 类图集禁 mip，羽翼边缘暗色渗色
    });
    const part = this.parts.get(path);
    if (part) part.syntheticHair = true;
    const top = [...boneCount.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6);
    const pushedCount = (model as FsmModel & { __pushedCount?: number }).__pushedCount ?? 0;
    this.dlog(
      "长发自动绑骨: " +
        path.replace(/^.*[\/]/, "") +
        " " +
        nv +
        " 顶点(体表外推 " +
        pushedCount +
        ") → " +
        top.map(([b, c]) => (this.skelBones[b]?.name ?? b) + "×" + c).join(" ")
    );
  }

  /** 刚性帽 → 合成蒙皮部件：mount 烘焙进顶点，全部顶点单骨 w=1（挂点骨）。
   *  单骨 LBS = boneW(t)·boneW0⁻¹·pw，与旧 holder 刚挂（boneW(t)·boneW0⁻¹
   *  作用于挂接位顶点）逐点等价，法线静态与引擎蒙皮同口径。骨缺失/无骨架
   *  时零权重保持绑定姿势 = 旧静态摆放（FSM authored 局部坐标直出） */
  private buildRigidHatPart(path: string) {
    const e = this.boneAttached.get(path);
    if (!e || e.autoHair) return;
    const { model, mats, offset, boneName } = e;
    const bone = this.resolveMountBone(boneName);
    e.bone = bone;
    let synth: {
      model: FskinModel;
      texes: (TexData | null)[];
      texFiles: (string | null)[];
      texMask: boolean[];
    };
    if (bone && this.bindWorldMats.length) {
      const idx = this.skelBones.indexOf(bone);
      const { mount, origin } = this.mountOf(bone, offset);
      e.origin = origin;
      synth = this.synthFromFsm(path + "#mount", model, mats, mount, (pw, lpOf) => [
        { boneIndex: idx, weight: 1, boneLocalPos: lpOf(idx, pw) },
      ]);
    } else {
      e.origin = null;
      synth = this.synthFromFsm(path + "#mount", model, mats, new THREE.Matrix4(), (pw) => [
        { boneIndex: 0, weight: 0, boneLocalPos: [pw.x, pw.y, pw.z] },
      ]);
    }
    this.addPart(path, synth.model, synth.texes, synth.texFiles, {
      uvSeamSplit: false, // FSM 图集无平铺接缝，拆分会误改 authored 大跨度采样弧
      texMask: synth.texMask, // mask 类图集禁 mip，羽翼边缘暗色渗色
    });
    const part = this.parts.get(path);
    if (part) part.syntheticMount = true;
  }

  /** FSM → 合成蒙皮 FskinModel：mount 挂接矩阵烘焙进顶点（pos = mount·
   *  authored、normal 按方向量变换即归一化），影响表由 weightFn 给出（长发
   *  最近骨分配 / 刚性帽单骨 w=1 / 静态回落零权重）。UV 岛修复与
   *  buildFsmMeshes 同口径（mask 类图集行序混排按簇色判据分流）。
   *  posOverride 给定时跳过 mount 直接采用（长发体表外推后的位置）。
   *  返回的 texes/texFiles/materials 均按子网格的 s.index 材质引用对位
   *  （setFsm 单独渲染同口径；addPart 按位置序消费，故在此重排） */
  private synthFromFsm(
    synthPath: string,
    model: FsmModel,
    mats: { tex: TexData | null; file: string | null }[],
    mount: THREE.Matrix4,
    weightFn: (
      pw: THREE.Vector3,
      lpOf: (b: number, pw: THREE.Vector3) => number[]
    ) => Influence[],
    posOverride?: THREE.Vector3[]
  ): {
    model: FskinModel;
    texes: (TexData | null)[];
    texFiles: (string | null)[];
    /** 每子网格材质的 mask 类标记（pre[2]≠0xFF）：纹理禁 mip，防暗色邻岛渗色 */
    texMask: boolean[];
  } {
    const inv = new THREE.Matrix4();
    const lpOf = (b: number, pw: THREE.Vector3): number[] => {
      inv.copy(this.bindWorldMats[b]).invert();
      const local = pw.clone().applyMatrix4(inv);
      return [local.x, local.y, local.z];
    };

    const uvFix = new Float32Array(model.vertexCount * 2);
    for (let i = 0; i < model.vertexCount; i++) {
      uvFix[i * 2] = model.vertices[i * 8 + 6];
      uvFix[i * 2 + 1] = model.vertices[i * 8 + 7];
    }
    for (const [mi, tris] of Viewer.trisByMaterial(model)) {
      const mt = mats[mi];
      if (!Viewer.texFlip(model.materials[mi]?.pre) || !mt?.tex) continue;
      Viewer.repairMaskUvIslands(uvFix, tris, mt.tex);
    }

    const verts: Vertex[] = [];
    const v3 = new THREE.Vector3();
    for (let i = 0; i < model.vertexCount; i++) {
      const pw = posOverride?.[i]
        ? posOverride[i].clone()
        : v3
            .set(model.vertices[i * 8], model.vertices[i * 8 + 1], model.vertices[i * 8 + 2])
            .applyMatrix4(mount);
      const nrm = new THREE.Vector3(
        model.vertices[i * 8 + 3],
        model.vertices[i * 8 + 4],
        model.vertices[i * 8 + 5]
      ).transformDirection(mount);
      verts.push({
        pos: [pw.x, pw.y, pw.z],
        normal: [nrm.x, nrm.y, nrm.z],
        uv: [uvFix[i * 2], uvFix[i * 2 + 1]],
        influences: weightFn(pw, lpOf),
      });
    }

    // 材质按子网格的 s.index 引用对位（setFsm 单独渲染同口径；多子网格共用
    // 材质时重复排列，addPart 按位置序消费即可得到正确材质）
    const matIdxOf = (sm: { index: number }) =>
      Math.min(sm.index, model.materials.length - 1);
    return {
      model: {
        path: synthPath,
        version: 1,
        bboxMax: [...model.bboxMax],
        bboxMin: [...model.bboxMin],
        reserved256: [],
        vertices: verts,
        submeshes: model.submeshes.map((sm) => ({
          zero1: (sm.index << 16) >>> 0,
          zero2: 0,
          vertexCount2: sm.vertexCount,
          misc: [0, 0, 0, 0, 0, 0],
          triangles: [...sm.triangles],
        })),
        materials: model.submeshes.map((sm) => {
          const mm = model.materials[matIdxOf(sm)];
          return {
            lead: mm?.index ?? 0,
            diffuse: [...(mm?.diffuse ?? [1, 1, 1])],
            ambient: [...(mm?.ambient ?? [1, 1, 1])],
            emissive: [...(mm?.emissive ?? [0, 0, 0])],
            specular: [...(mm?.specular ?? [0, 0, 0])],
            specPower: mm?.specPower ?? 0,
            pre: [...(mm?.pre ?? [])],
            texturePath: mm?.texturePath ?? "",
            tail: [],
          };
        }),
      },
      texes: model.submeshes.map((sm) => mats[matIdxOf(sm)]?.tex ?? null),
      texFiles: model.submeshes.map((sm) => mats[matIdxOf(sm)]?.file ?? null),
      texMask: model.submeshes.map((sm) =>
        Viewer.texFlip(model.materials[matIdxOf(sm)]?.pre)
      ),
    };
  }

  removeBoneAttached(path: string) {
    const e = this.boneAttached.get(path);
    if (!e) return;
    this.dlog(`帽子卸下: ${path.replace(/^.*[\\/]/, "")} · 剩余 ${this.boneAttached.size - 1}`);
    if (this.parts.has(path)) this.removePart(path); // 合成部件（长发/刚性帽统一）
    this.boneAttached.delete(path);
  }

  hasBoneAttached(path: string) {
    return this.boneAttached.has(path);
  }

  setBoneAttachedOffset(path: string, offset: [number, number, number]) {
    const e = this.boneAttached.get(path);
    if (!e) return;
    // 挂接矩阵烘焙进顶点：改偏移对合成件不产生视觉变化，必须更新元数据并
    // 重建（长发含体表外推+绑骨重跑，刚性帽重烘焙 mount）
    e.offset = [offset[0], offset[1], offset[2]];
    if (e.autoHair) this.buildAutoHairPart(path);
    else this.buildRigidHatPart(path);
  }

  /** 骨名列表（帽槽挂点选择用），按骨架当前骨序 */
  boneNames(): string[] {
    return this.skelBones.map((b) => b.name);
  }

  /**
   * 按当前骨架重建挂接件合成部件（骨架加载/切换/清空后调用）：mount 与
   * boneLocalPos 均按新骨架绑定姿态（帧 0 快照）重新烘焙，渲染结果与重建
   * 时刻无关（等价引擎逐帧重算 boneW(t)·offset）。挂接口径见 mountOf：
   *
   * FSM 网格为**局部坐标**建模（原点 = 挂点锚，帽子/武器均绕原点，实测
   * FM01 bbox ±10~15、J01 ±10~43，不存在世界坐标 authored 的 FSM；帽类
   * FSM 挂点区全为 0，不带任何逐模型修正）。
   *
   * tag_* 挂点骨（引擎约定，AvatarBoy/Girl 全部 FSKEL 均自带 tag_Hat /
   * tag_LWeapon / tag_LWrist / tag_RWeapon / tag_RWrist）：原点直落骨骼原点、
   * 零偏移，姿态 = tag 帧 × Ry(180°)。tag_Hat 的局部 Q(−0.707,−0.707,0,0)
   * 把 tag 帧转成 +Y 朝上，但 tag 的 Z 轴被它翻向 Head 的 −Z——恒等直挂会
   * 前后反面（实测）；Ry(π) 冠方向不变、前后归位，与旧 Head01+Rz(−90°)
   * 手调位的实测姿态逐轴一致（mesh +Y→上、+Z→Head+Z=脑后、+X→Head−Y）。
   * 帽子按 Y-up 围绕原点 authored（FM01 顶点 Y −9.15..15.09 挂
   * tag_Hat(153.8) 后恰覆盖头部 144.4~167.6，78 顶帽子实测同一锚成立，
   * 见 cpk/examples/hat_probe）。
   *
   * 普通骨骼（骨架无 tag 挂点骨时的回落，如挂 Bip01 Head01）：沿用旧手调位
   * ——骨骼原点 + HAT_OFFSET（骨局部系，X=沿骨轴抬升/Y=左右/Z=前后），
   * inner 转 Rz(−90°)（mesh +Y 冠/刃方向对骨轴 +X，前后不翻转——花环帽
   * 游戏内实测）。该挂法只逼近了 tag_Hat 的位置（HAT_OFFSET.x=10≈局部
   * 9.47）未复现其姿态，不对称帽子会有残差，仅兜底用。
   *
   * 运动学：顶点烘焙到数据空间（mount·authored），显示经 modelRoot 的
   * MODEL_YAW(π) 与身体蒙皮件同口径——单骨 LBS = boneW(t)·boneW0⁻¹·pw，
   * 与身体件逐点同向，旧 holder 路径的 Yaw 共轭补偿不再需要。
   *
   * 骨名不存在/骨架未加载时走零权重静态回落（authored 局部坐标直出）。
   */
  private refreshBoneAttached(path?: string) {
    const paths = path ? [path] : [...this.boneAttached.keys()];
    for (const p of paths) {
      const e = this.boneAttached.get(p);
      if (!e) continue;
      if (e.autoHair) {
        // 长发合成件重建（含体表外推重跑）；骨架未就位时 build 前置守卫
        // 直接返回，先卸旧件避免旧骨架数据残留
        if (this.parts.has(p)) this.removePart(p);
        this.buildAutoHairPart(p);
      } else {
        this.buildRigidHatPart(p); // addPart 内部替换旧件
      }
    }
  }

  /** 皮肤网格整体显隐（动作模式下叠加/隐藏绑定姿势网格），饰品叠层跟随 */
  setPartsVisible(v: boolean) {
    for (const p of this.parts.values()) {
      p.mesh.visible = v;
      if (p.overlayMesh) p.overlayMesh.visible = v;
    }
  }

  /** 骨架可视化显隐（SkeletonHelper 线条 + 关节小球），蒙皮网格不受影响 */
  setSkeletonVisible(v: boolean) {
    this.skeletonVisible = v;
    if (this.skelHelper) this.skelHelper.visible = v;
    if (this.boneLines) this.boneLines.visible = v;
    if (this.jointMesh) this.jointMesh.visible = v;
  }

  /** 相机对准当前骨架 */
  frameSkeleton() {
    if (!this.skelRoot) return;
    this.skelRoot.updateMatrixWorld(true);
    const box = new THREE.Box3();
    for (const b of this.skelBones) {
      // 骨骼世界坐标为数据空间，显示时还带 modelRoot 旋转
      box.expandByPoint(
        b.getWorldPosition(new THREE.Vector3()).applyMatrix4(this.modelRoot.matrixWorld)
      );
    }
    this.frameBox(box);
  }

  frameAll() {
    const box = new THREE.Box3();
    let has = false;
    for (const p of this.parts.values()) {
      box.expandByObject(p.mesh);
      has = true;
    }
    if (this.fakMeshes.length) {
      // FAK 为逐帧变形网格，用当前顶点位置直接求包围盒（fakPos 为数据空间，需带上 modelRoot 旋转）
      const pa = this.fakPos?.array as Float32Array | undefined;
      if (pa) {
        for (let i = 0; i < pa.length; i += 3) {
          box.expandByPoint(
            new THREE.Vector3(pa[i], pa[i + 1], pa[i + 2]).applyMatrix4(this.modelRoot.matrixWorld)
          );
        }
      }
      has = true;
    }
    for (const mesh of this.fsmMeshes) {
      box.expandByObject(mesh);
      has = true;
    }
    if (this.sfxGroup) {
      // Sprite 各自带缩放的 matrixWorld，expandByObject 即取公告板实际占位
      box.expandByObject(this.sfxGroup);
      has = true;
    }
    if (!has) return;
    this.frameBox(box);
  }

  /** 相机对准包围盒：按视口宽高比计算恰好容纳模型的距离，再放宽一倍多留边，避免初始过近 */
  private frameBox(box: THREE.Box3) {
    if (box.isEmpty()) return;
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const fovY = (this.camera.fov * Math.PI) / 180;
    const fovX = 2 * Math.atan(Math.tan(fovY / 2) * this.camera.aspect);
    const need = Math.max(
      size.y / 2 / Math.tan(fovY / 2),
      size.x / 2 / Math.tan(fovX / 2)
    );
    const dist = Math.max(need * 2.2, 60);
    const dir = INIT_VIEW_DIR;
    this.controls.target.copy(center);
    this.camera.position.copy(center).addScaledVector(dir, dist);
    this.camera.updateProjectionMatrix();
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.ro.disconnect();
    this.clearSfx();
    this.clearFx();
    for (const p of this.parts.values()) {
      p.geom.dispose();
      for (const m of p.mats) m.dispose();
    }
    for (const path of [...this.boneAttached.keys()]) this.removeBoneAttached(path);
    for (const t of this.texCache.values()) t.dispose();
    this.controls.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}

// vite-watch-bust 2026-10-22
