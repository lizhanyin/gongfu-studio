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
