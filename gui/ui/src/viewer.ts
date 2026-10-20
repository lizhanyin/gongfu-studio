import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { FskinModel, FskelMeta, Influence } from "./types";

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
  mat: THREE.MeshStandardMaterial;
  model: FskinModel;
  /** 跨缝拆分产生的副本顶点，编辑顶点时需同步 */
  clones: CloneVert[];
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

/** 取权重最高的至多 4 个影响并归一化；超出骨架范围的骨骼剔除 */
function topInfluences(infs: Influence[], boneCount: number) {
  const valid = infs.filter((i) => i.boneIndex < boneCount);
  const src = valid.length ? valid : [{ boneIndex: 0, weight: 1 }];
  const sorted = [...src].sort((a, b) => b.weight - a.weight).slice(0, 4);
  const sum = sorted.reduce((s, i) => s + Math.max(i.weight, 0), 0) || 1;
  const idx = sorted.map((i) => i.boneIndex);
  const w = sorted.map((i) => Math.max(i.weight, 0) / sum);
  while (idx.length < 4) {
    idx.push(0);
    w.push(0);
  }
  return { idx, w };
}

export class Viewer {
  private scene = new THREE.Scene();
  private camera: THREE.PerspectiveCamera;
  private renderer: THREE.WebGLRenderer;
  private controls: OrbitControls;
  private parts = new Map<string, PartObj>();
  private container: HTMLElement;
  private mode: RenderMode = "shaded";
  private raf = 0;
  private ro: ResizeObserver;
  private texCache = new Map<string, THREE.DataTexture>();
  private onPick: (path: string) => void = () => {};

  // ---- FSKEL 骨架 ----
  private skelBones: THREE.Bone[] = [];
  private skelRoot: THREE.Bone | null = null;
  private skelHelper: THREE.SkeletonHelper | null = null;
  private jointMesh: THREE.InstancedMesh | null = null;
  /** 全部帧的扁平姿态：帧 × 骨 × 7 */
  private fskelPoses: Float32Array | null = null;
  private fskelBoneCount = 0;
  /** 部件蒙皮绑定的骨架（动作模式下网格跟随骨骼变形） */
  private skelSkeleton: THREE.Skeleton | null = null;
  private qa = new THREE.Quaternion();
  private qb = new THREE.Quaternion();

  constructor(container: HTMLElement) {
    this.container = container;
    this.renderer = new THREE.WebGLRenderer({ antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio);
    container.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(50, 1, 0.5, 5000);
    this.camera.position.set(185, 200, 340);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.target.set(0, 85, 0);

    this.scene.background = new THREE.Color(0x000000);
    const hemi = new THREE.HemisphereLight(0xdde6ff, 0x30281e, 1.15);
    this.scene.add(hemi);
    const dir = new THREE.DirectionalLight(0xffffff, 1.6);
    dir.position.set(150, 250, 180);
    this.scene.add(dir);
    const dir2 = new THREE.DirectionalLight(0x8899ff, 0.5);
    dir2.position.set(-160, 120, -140);
    this.scene.add(dir2);

    const grid = new THREE.GridHelper(600, 60, 0x2a3550, 0x1a2033);
    this.scene.add(grid);
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
      this.renderer.render(this.scene, this.camera);
    };
    loop();
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
  }

  private resize() {
    const w = this.container.clientWidth || 1;
    const h = this.container.clientHeight || 1;
    this.renderer.setSize(w, h);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  getTexture(file: string, tex: TexData): THREE.DataTexture {
    let t = this.texCache.get(file);
    if (!t) {
      t = new THREE.DataTexture(
        tex.rgba,
        tex.width,
        tex.height,
        THREE.RGBAFormat
      );
      t.flipY = false;
      // 游戏引擎用重复寻址，手臂等条带 UV 环绕 u=0/1 接缝，必须 Repeat
      t.wrapS = THREE.RepeatWrapping;
      t.wrapT = THREE.RepeatWrapping;
      t.magFilter = THREE.LinearFilter;
      t.minFilter = THREE.LinearMipmapLinearFilter;
      t.generateMipmaps = true;
      t.needsUpdate = true;
      this.texCache.set(file, t);
    }
    return t;
  }

  addPart(path: string, model: FskinModel, tex: TexData | null, texFile: string | null) {
    if (this.parts.has(path)) this.removePart(path);

    const n = model.vertices.length;
    // 用可增长数组：跨缝三角形需要拆分出副本顶点
    const pos = new Array<number>(n * 3);
    const nrm = new Array<number>(n * 3);
    const uv = new Array<number>(n * 2);
    const boneCol = new Array<number>(n * 3);
    const uvCol = new Array<number>(n * 3);
    const clones: CloneVert[] = []; // 跨缝拆分产生的副本顶点
    const pushVert = (i: number, u: number, v: number) => {
      const vp = model.vertices[i];
      const c = boneColor(vp.influences[0]?.boneIndex ?? 0);
      pos.push(vp.pos[0], vp.pos[1], vp.pos[2]);
      nrm.push(vp.normal[0], vp.normal[1], vp.normal[2]);
      uv.push(u, v);
      boneCol.push(c.r, c.g, c.b);
      uvCol.push(
        Math.max(0, Math.min(1, vp.uv[0])),
        Math.max(0, Math.min(1, vp.uv[1])),
        0.35
      );
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
    });

    // 跨缝拆分：手臂等条带 UV 环绕 u=0/1 接缝（引擎用重复寻址）。
    // 三角形两端的顶点分处缝两侧时插值会横穿整张纹理，需复制顶点把 UV 归一到同侧。
    const tri = model.triangles.slice();
    for (let k = 0; k < tri.length; k += 3) {
      const ia = tri[k], ib = tri[k + 1], ic = tri[k + 2];
      const us = [uv[ia * 2], uv[ib * 2], uv[ic * 2]];
      const vs = [uv[ia * 2 + 1], uv[ib * 2 + 1], uv[ic * 2 + 1]];
      const du = Math.max(...us) - Math.min(...us);
      const dv = Math.max(...vs) - Math.min(...vs);
      if (du <= 0.5 && dv <= 0.5) continue;
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

    const geom = new THREE.BufferGeometry();
    geom.setAttribute("position", new THREE.BufferAttribute(Float32Array.from(pos), 3));
    geom.setAttribute("normal", new THREE.BufferAttribute(Float32Array.from(nrm), 3));
    geom.setAttribute("uv", new THREE.BufferAttribute(Float32Array.from(uv), 2));
    geom.setAttribute("color", new THREE.BufferAttribute(Float32Array.from(boneCol), 3));
    geom.setAttribute("color2", new THREE.BufferAttribute(Float32Array.from(uvCol), 3));
    geom.setIndex(tri);
    // 蒙皮属性（未绑骨架时挂上亦无副作用）；跨缝克隆顶点沿用源顶点权重
    const total = model.vertices.length + clones.length;
    const sIdx = new Uint16Array(total * 4);
    const sW = new Float32Array(total * 4);
    const boneCap = this.fskelBoneCount > 0 ? this.fskelBoneCount : Infinity;
    const fillSkin = (ai: number, infs: Influence[]) => {
      const { idx, w } = topInfluences(infs, boneCap);
      for (let k = 0; k < 4; k++) {
        sIdx[ai * 4 + k] = idx[k];
        sW[ai * 4 + k] = w[k];
      }
    };
    model.vertices.forEach((v, i) => fillSkin(i, v.influences));
    clones.forEach((c, k) => fillSkin(model.vertices.length + k, model.vertices[c.src].influences));
    geom.setAttribute("skinIndex", new THREE.Uint16BufferAttribute(sIdx, 4));
    geom.setAttribute("skinWeight", new THREE.Float32BufferAttribute(sW, 4));

    const mat = new THREE.MeshStandardMaterial({
      side: THREE.DoubleSide,
      roughness: 0.75,
      metalness: 0.0,
      vertexColors: false,
    });
    if (tex && texFile) {
      mat.map = this.getTexture(texFile, tex);
      const anyMat = mat as any;
      anyMat.customColorAttribute = "color";
    }
    const m = model.material;
    mat.color.setRGB(m.diffuse[0], m.diffuse[1], m.diffuse[2]);
    mat.emissive.setRGB(m.emissive[0], m.emissive[1], m.emissive[2]);

    const mesh = new THREE.Mesh(geom, mat);
    mesh.userData.path = path;
    this.scene.add(mesh);
    const part: PartObj = { mesh, geom, mat, model, clones };
    this.parts.set(path, part);
    // 动作模式已绑骨架时直接以蒙皮网格入场景
    if (this.skelSkeleton) this.bindPart(part);
    this.applyMode(this.mode);
  }

  removePart(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    this.scene.remove(p.mesh);
    p.geom.dispose();
    p.mat.dispose();
    this.parts.delete(path);
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
      if (mode === "shaded") {
        p.mat.vertexColors = false;
        const m = p.model.material;
        p.mat.color.setRGB(m.diffuse[0], m.diffuse[1], m.diffuse[2]);
        p.mat.emissive.setRGB(m.emissive[0], m.emissive[1], m.emissive[2]);
      } else {
        p.mat.vertexColors = true;
        p.geom.setAttribute(
          "color",
          p.geom.getAttribute(mode === "bones" ? "color" : "color2")
        );
        p.mat.color.setRGB(1, 1, 1);
        p.mat.emissive.setRGB(0, 0, 0);
      }
      p.mat.needsUpdate = true;
    }
  }

  setWireframe(on: boolean) {
    for (const p of this.parts.values()) p.mat.wireframe = on;
  }

  /** 高亮选中部件：其余半透明 */
  highlight(path: string | null) {
    for (const [k, p] of this.parts) {
      const sel = path === null || k === path;
      p.mat.transparent = !sel;
      p.mat.opacity = sel ? 1.0 : 0.28;
      p.mat.depthWrite = true;
    }
  }

  applyMaterial(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    const m = p.model.material;
    if (this.mode === "shaded") {
      p.mat.color.setRGB(m.diffuse[0], m.diffuse[1], m.diffuse[2]);
      p.mat.emissive.setRGB(m.emissive[0], m.emissive[1], m.emissive[2]);
    }
    p.mat.roughness = Math.max(0.05, Math.min(1, 1 - m.specPower / 60));
  }

  /** 顶点编辑后同步几何（含跨缝拆分的副本顶点） */
  updateVertex(path: string, i: number) {
    const p = this.parts.get(path);
    if (!p) return;
    const v = p.model.vertices[i];
    const apply = (ai: number, du: number, dv: number) => {
      const pos = p.geom.getAttribute("position") as THREE.BufferAttribute;
      pos.setXYZ(ai, v.pos[0], v.pos[1], v.pos[2]);
      const nrm = p.geom.getAttribute("normal") as THREE.BufferAttribute;
      nrm.setXYZ(ai, v.normal[0], v.normal[1], v.normal[2]);
      const uv = p.geom.getAttribute("uv") as THREE.BufferAttribute;
      uv.setXY(ai, v.uv[0] + du, v.uv[1] + dv);
    };
    apply(i, 0, 0);
    p.geom.getAttribute("position").needsUpdate = true;
    p.geom.getAttribute("normal").needsUpdate = true;
    p.geom.getAttribute("uv").needsUpdate = true;
    p.clones.forEach((c, t) => {
      if (c.src === i) apply(p.model.vertices.length + t, c.du, c.dv);
    });
  }

  refreshTexture(path: string, tex: TexData, texFile: string) {
    const p = this.parts.get(path);
    if (!p) return;
    p.mat.map = this.getTexture(texFile, tex);
    p.mat.needsUpdate = true;
  }

  clearTexture(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    p.mat.map = null;
    p.mat.needsUpdate = true;
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
    this.skelHelper = this.skelRoot ? new THREE.SkeletonHelper(this.skelRoot) : null;
    if (this.skelHelper) {
      const mat = this.skelHelper.material as THREE.LineBasicMaterial;
      mat.linewidth = 2;
      this.scene.add(this.skelHelper);
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
    this.scene.add(this.jointMesh);

    // GPU 蒙皮：从部件顶点反推每骨绑定姿势，网格换成跟随骨骼变形的 SkinnedMesh
    this.skelSkeleton = new THREE.Skeleton(this.skelBones, this.computeBoneInverses(n));
    for (const p of this.parts.values()) this.bindPart(p);

    this.applyFskelFrame(0, 0, 0);
  }

  /**
   * 每骨绑定世界矩阵之逆（boneInverses）。数据源是 fskin 顶点影响里的
   * boneLocalPos：单影响顶点满足 M_bind × boneLocalPos = pos（已验证为精确
   * 刚体映射），据此做 Kabsch 拟合；混合权重顶点的 (局部点, pos) 只是近似
   * 对应，仅在单影响点不足 3 个时补充。无任何点对的骨骼对蒙皮无影响。
   */
  private computeBoneInverses(boneCount: number): THREE.Matrix4[] {
    const exact: [number[], number[]][][] = Array.from({ length: boneCount }, () => []);
    const approx: [number[], number[]][][] = Array.from({ length: boneCount }, () => []);
    for (const p of this.parts.values()) {
      for (const v of p.model.vertices) {
        for (const inf of v.influences) {
          if (inf.boneIndex >= boneCount) continue;
          const pair: [number[], number[]] = [inf.boneLocalPos, v.pos];
          if (v.influences.length === 1 || inf.weight >= 0.99) exact[inf.boneIndex].push(pair);
          else approx[inf.boneIndex].push(pair);
        }
      }
    }
    return Array.from({ length: boneCount }, (_, i) =>
      this.fitBindMatrix(exact[i].length >= 3 ? exact[i] : exact[i].concat(approx[i])).invert()
    );
  }

  /** 点对 (骨骼局部, 世界) 的刚体拟合：H = Σ(b-cb)(a-ca)ᵀ 极分解求 R，t = cb - R·ca */
  private fitBindMatrix(pairs: [number[], number[]][]): THREE.Matrix4 {
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

  /** 把部件换成绑定当前骨架的 SkinnedMesh（几何、材质、可见性全沿用） */
  private bindPart(p: PartObj) {
    if (!this.skelSkeleton || p.mesh instanceof THREE.SkinnedMesh) return;
    const s = new THREE.SkinnedMesh(p.geom, p.mat);
    s.userData.path = p.mesh.userData.path;
    s.visible = p.mesh.visible;
    s.frustumCulled = false; // 蒙皮变形后静态包围球不再可靠
    this.scene.remove(p.mesh);
    this.scene.add(s);
    // 顶点与骨骼同处世界空间，bindMatrix 用单位阵
    s.bind(this.skelSkeleton, new THREE.Matrix4());
    p.mesh = s;
  }

  /** 解除蒙皮：换回静态绑定姿势网格 */
  private unbindPart(p: PartObj) {
    if (!(p.mesh instanceof THREE.SkinnedMesh)) return;
    const m = new THREE.Mesh(p.geom, p.mat);
    m.userData.path = p.mesh.userData.path;
    m.visible = p.mesh.visible;
    this.scene.remove(p.mesh);
    this.scene.add(m);
    p.mesh = m;
  }

  /** 应用第 i0 帧姿态；给出 i1/alpha 时做平移 lerp + 旋转 slerp 平滑插值 */
  applyFskelFrame(i0: number, i1: number, alpha: number) {
    if (!this.fskelPoses || !this.skelBones.length) return;
    const poses = this.fskelPoses;
    const n = this.fskelBoneCount;
    const stride = n * 7;
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
    // 关节小球跟随世界坐标
    if (this.skelRoot) {
      this.skelRoot.updateMatrixWorld(true);
      const m = new THREE.Matrix4();
      for (let b = 0; b < n; b++) {
        m.setPosition(this.skelBones[b].getWorldPosition(new THREE.Vector3()));
        this.jointMesh?.setMatrixAt(b, m);
      }
      this.jointMesh!.instanceMatrix.needsUpdate = true;
    }
  }

  clearFskel() {
    for (const p of this.parts.values()) this.unbindPart(p);
    this.skelSkeleton?.dispose();
    this.skelSkeleton = null;
    if (this.skelHelper) {
      this.scene.remove(this.skelHelper);
      this.skelHelper.dispose();
      this.skelHelper = null;
    }
    if (this.jointMesh) {
      this.scene.remove(this.jointMesh);
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
  }

  /** 皮肤网格整体显隐（动作模式下叠加/隐藏绑定姿势网格） */
  setPartsVisible(v: boolean) {
    for (const p of this.parts.values()) p.mesh.visible = v;
  }

  /** 相机对准当前骨架 */
  frameSkeleton() {
    if (!this.skelRoot) return;
    this.skelRoot.updateMatrixWorld(true);
    const box = new THREE.Box3();
    for (const b of this.skelBones) {
      box.expandByPoint(b.getWorldPosition(new THREE.Vector3()));
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
    const dir = new THREE.Vector3(0.55, 0.35, 1).normalize();
    this.controls.target.copy(center);
    this.camera.position.copy(center).addScaledVector(dir, dist);
    this.camera.updateProjectionMatrix();
  }

  dispose() {
    cancelAnimationFrame(this.raf);
    this.ro.disconnect();
    for (const p of this.parts.values()) {
      p.geom.dispose();
      p.mat.dispose();
    }
    for (const t of this.texCache.values()) t.dispose();
    this.controls.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }
}
