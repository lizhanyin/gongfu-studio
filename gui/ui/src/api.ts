import { invoke } from "@tauri-apps/api/core";
import type {
  CharacterCatalog,
  FakModel,
  FsmModel,
  FskinModel,
  FskelMeta,
  ModelSet,
  ResEntry,
  TexImageDto,
} from "./types";

export const api = {
  scanModelSet: (dir: string) => invoke<ModelSet>("scan_model_set", { dir }),
  /** 角色装扮目录扫描：基础模型 / 装备槽 / 时装 + 全根贴图索引 */
  scanCharacter: (root: string) => invoke<CharacterCatalog>("scan_character", { root }),
  loadFskin: (path: string) => invoke<FskinModel>("load_fskin", { path }),
  saveFskin: (path: string, model: FskinModel) =>
    invoke<void>("save_fskin", { path, model }),
  loadTexture: (path: string) => invoke<TexImageDto>("load_texture", { path }),
  /** 原始文件字节（base64，支持 cpk: 包内路径） */
  readFileB64: (path: string) => invoke<string>("read_file_b64", { path }),
  exportObj: (paths: string[], outDir: string, baseName: string) =>
    invoke<string>("export_obj", { paths, outDir, baseName }),
  computeBbox: (model: FskinModel) =>
    invoke<[number[], number[]]>("compute_bbox", { model }),
  toggleDevtools: () => invoke<void>("toggle_devtools"),
  // FSKEL 动作
  loadFskelMeta: (path: string) => invoke<FskelMeta>("load_fskel_meta", { path }),
  /** [from, to) 帧的扁平姿态：帧 × 骨 × (tx,ty,tz,qx,qy,qz,qw) */
  loadFskelFrames: (path: string, from: number, to: number) =>
    invoke<number[]>("load_fskel_frames", { path, from, to }),
  exportFskelBvh: (path: string, actionIndex: number | null, outPath: string) =>
    invoke<string>("export_fskel_bvh", { path, actionIndex, outPath }),
  exportFskelGlb: (path: string, outPath: string) =>
    invoke<string>("export_fskel_glb", { path, outPath }),
  // 资源浏览
  listResDir: (dir: string) => invoke<ResEntry[]>("list_res_dir", { dir }),
  /** 目录配套的默认骨架（轻量，只列 act 目录）：整体预览装配前先换骨架用 */
  dirDefaultFskel: (dir: string) => invoke<string | null>("dir_default_fskel", { dir }),
  // 资源导出 / 导入（src 支持磁盘与 cpk: 包内路径；导入目标仅磁盘目录）
  exportFile: (src: string, dest: string) => invoke<void>("export_file", { src, dest }),
  /** 模型导出（fskin/fsm/fak）：原样复制模型 + 材质关联纹理一并导出到同目录 */
  exportModel: (src: string, dest: string) => invoke<string>("export_model", { src, dest }),
  exportDir: (src: string, destDir: string) => invoke<string>("export_dir", { src, destDir }),
  importFile: (src: string, destDir: string) => invoke<string>("import_file", { src, destDir }),
  // FAK 顶点动画模型（含全部帧顶点数据）
  loadFak: (path: string) => invoke<FakModel>("load_fak", { path }),
  saveFak: (path: string, model: FakModel) =>
    invoke<void>("save_fak", { path, model }),
  // FSM 静态武器网格（含顶点 / 材质 / 挂点）
  loadFsm: (path: string) => invoke<FsmModel>("load_fsm", { path }),
  saveFsm: (path: string, model: FsmModel) =>
    invoke<void>("save_fsm", { path, model }),
};
