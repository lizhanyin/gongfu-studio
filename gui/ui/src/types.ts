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

export interface TexImageDto {
  width: number;
  height: number;
  rgbaBase64: string;
}

// ---------------- FSKEL 动作 ----------------

export interface FskelBrief {
  path: string;
  fileName: string;
  frameCount: number;
  boneCount: number;
  byteLen: number;
  actionName: string;
}

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
