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

export interface IndexMeta {
  zero1: number;
  zero2: number;
  vertexCount2: number;
  pad4: number[];
  hash1: number;
  hash2: number;
  one1: number;
  one2: number;
  padTail: number[];
}

export interface FskinModel {
  path: string;
  version: number;
  bboxMax: number[];
  bboxMin: number[];
  reserved256: number[];
  vertices: Vertex[];
  triangles: number[];
  indexMeta: IndexMeta;
  material: Material;
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
}

export interface TexInfo {
  stem: string;
  path: string;
}

export interface ModelSet {
  rootDir: string;
  groups: PartGroup[];
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
}

// ---------------- FAK 顶点动画 ----------------

export interface FakSubmesh {
  index: number;
  pad: number[];
  bboxMax: number[];
  bboxMin: number[];
  /** 扁平 u16 索引，每 3 个为一个三角形，引用共享顶点池 */
  triangles: number[];
  tail: number[];
}

export interface FakFrame {
  name: string;
  /** 该帧持续时长（秒） */
  frameDt: number;
  /** 扁平顶点数据：每顶点 8 个 f32 = [pos3][normal3][uv2] */
  data: number[];
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
}
