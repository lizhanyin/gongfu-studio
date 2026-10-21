import { invoke } from "@tauri-apps/api/core";
import type {
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
  loadFskin: (path: string) => invoke<FskinModel>("load_fskin", { path }),
  saveFskin: (path: string, model: FskinModel) =>
    invoke<void>("save_fskin", { path, model }),
  loadTexture: (path: string) => invoke<TexImageDto>("load_texture", { path }),
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
  // FAK 顶点动画模型（含全部帧顶点数据）
  loadFak: (path: string) => invoke<FakModel>("load_fak", { path }),
  saveFak: (path: string, model: FakModel) =>
    invoke<void>("save_fak", { path, model }),
  // FSM 静态武器网格（含顶点 / 材质 / 挂点）
  loadFsm: (path: string) => invoke<FsmModel>("load_fsm", { path }),
  saveFsm: (path: string, model: FsmModel) =>
    invoke<void>("save_fsm", { path, model }),
};
