import { invoke } from "@tauri-apps/api/core";
import type {
  FskinModel,
  FskelBrief,
  FskelMeta,
  ModelSet,
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
  scanActDir: (dir: string) => invoke<FskelBrief[]>("scan_act_dir", { dir }),
  loadFskelMeta: (path: string) => invoke<FskelMeta>("load_fskel_meta", { path }),
  /** [from, to) 帧的扁平姿态：帧 × 骨 × (tx,ty,tz,qx,qy,qz,qw) */
  loadFskelFrames: (path: string, from: number, to: number) =>
    invoke<number[]>("load_fskel_frames", { path, from, to }),
  exportFskelBvh: (path: string, actionIndex: number | null, outPath: string) =>
    invoke<string>("export_fskel_bvh", { path, actionIndex, outPath }),
  exportFskelGlb: (path: string, outPath: string) =>
    invoke<string>("export_fskel_glb", { path, outPath }),
};
