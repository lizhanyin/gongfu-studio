export interface Influence {
  boneIndex: number;
  weight: number;
  boneLocalPos: number[];
}

export interface Vertex {
  pos: number[];
  normal: number[];
  uv: number[];
  influences: Influence[];
}

export interface Material {
  /** 材质槽序号（多材质 = 槽下标） */
  lead: number;
  diffuse: number[];
  ambient: number[];
  emissive: number[];
  specular: number[];
  specPower: number;
  pre: number[];
  texturePath: string;
  tail: number[];
}

/** 索引节 = 子网格，与 materials 槽一一对应 */
export interface FskinSubmesh {
  /** 节号<<16（零材质特效网格为 0xffff0000） */
  zero1: number;
  zero2: number;
  vertexCount2: number;
  /** 24B 附加数据（语义未定，原样保留） */
  misc: number[];
  /** 扁平索引，每 3 个一个三角形 */
  triangles: number[];
}

export interface FskinModel {
  path: string;
  version: number;
  bboxMax: number[];
  bboxMin: number[];
  reserved256: number[];
  vertices: Vertex[];
  submeshes: FskinSubmesh[];
  /** 材质槽（可为空，如特效网格） */
  materials: Material[];
}

/** 合并视图：全部子网格三角形拼接 */
export function trianglesOf(m: FskinModel): number[] {
  const out: number[] = [];
  for (const s of m.submeshes) out.push(...s.triangles);
  return out;
}

/** 主材质：多材质取首槽，无材质返回 null */
export function primaryMaterial(m: FskinModel): Material | null {
  return m.materials[0] ?? null;
}

export interface PartInfo {
  path: string;
  fileName: string;
  relDir: string;
  inRoot: boolean;
  vertexCount: number;
  triangleCount: number;
  bboxMin: number[];
  bboxMax: number[];
  texturePath: string;
  textureFile: string | null;
  bones: number[];
}

export interface PartGroup {
  label: string;
  signature: string;
  parts: PartInfo[];
  /** 替换槽（"hair"/"upper"/"lower"/"boot"）：同槽各组互斥显示；空串 = 独立部件组 */
  slot: string;
}

export interface TexInfo {
  stem: string;
  path: string;
}

export interface TexVariant {
  label: string;
  path: string;
}

/** 纹理切换槽：把一组贴图应用到指定部件（替换材质贴图或透明叠层） */
export interface TexSlot {
  /** "face" | "face_detail" */
  key: string;
  label: string;
  /** 应用到的部件 fskin 路径 */
  part: string;
  /** true = 透明叠层（饰品），false = 替换材质贴图（脸部纹理） */
  overlay: boolean;
  variants: TexVariant[];
}

export interface ModelSet {
  rootDir: string;
  groups: PartGroup[];
  textures: TexInfo[];
  /** 纹理切换槽（脸部纹理 / 饰品等） */
  texSlots: TexSlot[];
  /** 目录配套的默认骨架（act\ 下按目录名关键词选，如 AvatarBoy →
   *  act\boy.FSKEL）；整体预览据此同步骨架（部件 boneIndex 按各自骨架
   *  编号，配错骨架会蒙皮错位），无 act 目录为 null */
  defaultFskel: string | null;
}

// ---------------- 角色装扮 ----------------

/** 基础模型（AvatarGirl / AvatarBoy） */
export interface CharacterBase {
  /** 目录名小写（avatargirl / avatarboy） */
  key: string;
  label: string;
  /** "f" | "m" */
  gender: string;
  dir: string;
  actDir: string;
  /** 默认骨架动作文件（girl.FSKEL 等），无则 null */
  defaultFskel: string | null;
}

/** 装备/时装条目 */
export interface EquipItem {
  name: string;
  path: string;
  /** "f" | "m" | "u"（文件名前缀 fs / ms / 其他） */
  gender: string;
  /** 款式号（前缀字母后的数字），无则 null */
  number: number | null;
  vertexCount: number;
  triangleCount: number;
  /** 每材质槽贴图解析结果（全根索引匹配） */
  textureFiles: (string | null)[];
}

/** 装备槽（hat / armonr / lower / glove / boot） */
export interface CharacterSlot {
  key: string;
  label: string;
  /** "fsm" | "fskin" */
  kind: string;
  items: EquipItem[];
}

export interface CharacterCatalog {
  root: string;
  bases: CharacterBase[];
  slots: CharacterSlot[];
  fashions: EquipItem[];
  /** 全根贴图索引（小写 stem → 路径），跨目录解析装备/时装贴图 */
  textures: TexInfo[];
}

// ---------------- 资源浏览 ----------------

/** 资源目录的一个子项（树节点 / 网格卡片共用） */
export interface ResEntry {
  name: string;
  path: string;
  isDir: boolean;
  /** 小写扩展名，目录为 "dir" */
  kind: string;
  byteLen: number;
  /** 目录是否为"整体模型"（根级含 .fskin 或 act/ 内含 .FSKEL） */
  isModel: boolean;
  /** 目录是否为角色资源根（可进入角色装扮模式） */
  isCharacter?: boolean;
  vertexCount?: number;
  triangleCount?: number;
  frameCount?: number;
  boneCount?: number;
  /** sfx 特效节点数 */
  nodeCount?: number;
}

export interface TexImageDto {
  width: number;
  height: number;
  rgbaBase64: string;
}

// ---------------- FSKEL 动作 ----------------

export interface FskelAction {
  name: string;
  firstFrame: number;
  frameCount: number;
}

export interface FskelMeta {
  path: string;
  fileName: string;
  version: number;
  checksum: number;
  frameCount: number;
  frameDt: number;
  boneCount: number;
  boneNames: string[];
  parents: number[];
  actions: FskelAction[];
  /** 每骨镜像标志（首帧）：1 = 引擎对该骨世界矩阵追加 Scale(-1,-1,-1)
   *  （FSKEL 帧记录尾部 1 字节，IDA sub_1004C290 确证） */
  mirrorFirst?: number[];
  /** 帧间标志翻转的骨（挂点类，如 tag_LWeapon 持械/空手），含逐帧值 */
  mirrorUnstable?: FskelMirrorUnstable[];
}

/** 帧间镜像标志变化的骨 */
export interface FskelMirrorUnstable {
  bone: number;
  name: string;
  /** 逐帧镜像标志（与 frameCount 等长） */
  flags: number[];
}

// ---------------- FAK 顶点动画 ----------------

/** FAK 子网格：头字段为 [基地址][序号][?][顶点数]；triangles 已是共享池全局索引 */
export interface FakSubmesh {
  index: number;
  /** 该节顶点段在共享池中的起始下标（文件内索引的基地址） */
  baseVertex: number;
  /** 语义未定的 u16 0 */
  pad: number;
  /** 该节顶点数（共享池子集） */
  vertexCount: number;
  bboxMax: number[];
  bboxMin: number[];
  /** 扁平 u16 索引，每 3 个一个三角形 */
  triangles: number[];
  tail: number[];
}

/** 逐帧挂点（顶点动画无骨架，挂点逐帧烘焙；与 FSM 挂点同构） */
export interface FakDummy {
  name: string;
  /** 前 3 个位置，后 4 个朝向四元数 */
  values: number[];
  tail: number[];
}

export interface FakFrame {
  name: string;
  /** 该帧持续时长（秒） */
  frameDt: number;
  /** 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2] */
  data: number[];
  /** 该帧之后的挂点块（可为空） */
  dummies: FakDummy[];
}

export interface FakMaterial {
  index: number;
  diffuse: number[];
  ambient: number[];
  emissive: number[];
  specular: number[];
  specPower: number;
  pre: number[];
  texturePath: string;
  tail: number[];
}

export interface FakAction {
  name: string;
  firstFrame: number;
  frameCount: number;
}

export interface FakModel {
  path: string;
  version: number;
  vertexCount: number;
  bboxMax: number[];
  bboxMin: number[];
  reserved256: number[];
  submeshes: FakSubmesh[];
  frames: FakFrame[];
  materials: FakMaterial[];
  actions: FakAction[];
  /** 各区之后的残留字节（G036 类损坏样本；正常文件为空） */
  tail: number[];
}

/** FSM 子网格/材质记录与 FAK 逐字节同构（见 gui/src/fsm.rs） */
export type FsmMaterial = FakMaterial;

/** FSM 子网格：头字段为 [基地址][序号][?][顶点数]；triangles 已是共享池全局索引 */
export interface FsmSubmesh {
  index: number;
  /** 该节顶点段在共享池中的起始下标（文件内索引的基地址） */
  baseVertex: number;
  /** 语义未定的 u16 0 */
  pad: number;
  /** 该节顶点数（共享池子集） */
  vertexCount: number;
  bboxMax: number[];
  bboxMin: number[];
  /** 扁平 u16 索引，每 3 个一个三角形 */
  triangles: number[];
  tail: number[];
}

export interface FsmDummy {
  /** 定长 4 字节名（GE2/GE1/F02/F01） */
  name: string;
  /** 前 3 个为位置，后 4 个疑似四元数 */
  values: number[];
  tail: number[];
}

export interface FsmModel {
  path: string;
  version: number;
  unknown: number;
  vertexCount: number;
  /** 头部包围盒：min 在前 max 在后（子网格记录相反） */
  bboxMin: number[];
  bboxMax: number[];
  reserved256: number[];
  /** 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2] */
  vertices: number[];
  submeshes: FsmSubmesh[];
  materials: FsmMaterial[];
  dummies: FsmDummy[];
}

/** FSCN 尾部 BVH 记录（判型见 gui/src/fscn.rs：node 第三字段恒 0） */
export type FscnTreeRecord =
  | { type: "node"; left: number; right: number; bboxMax: number[]; bboxMin: number[] }
  | { type: "leaf"; triangles: number[]; bboxMax: number[]; bboxMin: number[] };

/** FSCN 场景静态网格：几何/材质与 FSM 字段级同构（挂点恒空），多一棵尾部 BVH */
export interface FscnModel extends FsmModel {
  tree: FscnTreeRecord[];
}

// ---------------- SFX 特效脚本 ----------------

/** SFX 特效节点（11 种类型：EffectQuad / EffectParticleEmitter 等） */
export interface SfxNode {
  /** 节点类型（EffectQuad 等） */
  kind: string;
  /** GBK 解码节点名（"lujing"/"光传递_3" 等设计师命名，可为空） */
  name: string;
  /** 位置偏移（prop+112；Quad/Cross 逆向确证，其余类型恒 0） */
  offset: number[];
  /** 挂接路径名（prop+80；"none"/空 = 定点，否则引用 paths 中同名 EffectMotion） */
  pathName: string;
  /** 类型名之后的原始参数字节（节点号+名字缓冲+类型参数），保存时原样写回 */
  raw: number[];
  /** 贴图引用（system\effect\tex\…\*.tga；全量实测每节点至多 1 条，帧序列在 tex 目录相邻文件而非文件内列表） */
  textures: string[];
  /** 网格引用（EffectMesh 的 system\…\*.fak） */
  models: string[];
  /** 音效引用（EffectSound 的 sound\…\*.wav） */
  sounds: string[];
  /** 长成后尺寸 [全宽, 全高]（Quad/Cross 状态关键帧半宽×2；0 = 未知回退标称） */
  size: number[];
  /** 长成后颜色 RGBA（0..1，取尺寸最大关键帧） */
  color: number[];
  /** Cross 追加水平第三片（地贴模式，prop+140） */
  ground: boolean;
  /** Cylinder 首状态几何（null = 非 Cylinder） */
  cyl: SfxCyl | null;
  /** 尾块 +260/+264 的 U×V 帧网格（[1,1] = 整图单帧） */
  grid: number[];
  /** 尾块 +268 翻页帧率（0 = 未记录） */
  fps: number;
  /** 尾块 +272 播完停帧 */
  hold: boolean;
  /** 状态关键帧总时长（秒；0 = 未知） */
  dur: number;
  /** Quad/Cross 全部状态关键帧（播放器逐帧插值 RGBA/尺寸并累计转速） */
  states: SfxKeyState[];
}

/** Quad/Cross 状态关键帧（引擎通道 {时长, RGBA, 半宽×2, 半高×2, 转速xyz}） */
export interface SfxKeyState {
  /** 状态时长（秒） */
  dur: number;
  /** RGBA（0..1） */
  rgba: number[];
  /** 全宽/全高 */
  size: number[];
  /** 转速 xyz（rad/s，逐帧累计） */
  rot: number[];
}

/** EffectCylinder 几何参数（引擎 sub_100898F0 通道图） */
export interface SfxCyl {
  /** 环绕圈数（角度总跨度 = wrap×2π） */
  wrap: number;
  /** 底部基准高 */
  baseY: number;
  /** 顶部高（恒值 = 平顶管） */
  topY: number;
  /** 底部半径 */
  rBot: number;
  rTop: number;
  /** 底/顶部花边调制（半径随角度线性叠加） */
  wobBot: number;
  wobTop: number;
  /** 底/顶部锯齿深度（偶数段半径减去） */
  zigBot: number;
  zigTop: number;
  /** 底部螺旋抬升（Y 随角度上升） */
  riseBot: number;
  /** 底/顶部 RGBA */
  rgbaBot: number[];
  rgbaTop: number[];
  /** 环绕分段数 */
  segs: number;
  /** 环绕方向 UV 平铺数 */
  uWrap: number;
}

/** 尾部路径对象（EffectMotion = 速度积分路径；节点按 pathName 引用共享） */
export interface SfxPath {
  kind: string;
  name: string;
  /** 起始延迟秒 */
  delay: number;
  /** 循环（末态回绕首态） */
  looped: boolean;
  /** 关键帧 {时长s, 速度xyz, 角速度xyz}；位置 = 速度逐帧积分 */
  states: number[][];
  /** 记录原始字节（类型名之后），保存时原样写回 */
  raw: number[];
}

export interface SfxFile {
  path: string;
  /** 行主序 4×4 根变换（末行为平移） */
  matrix: number[];
  nodes: SfxNode[];
  /** 尾部路径数组（可为空） */
  paths: SfxPath[];
}
