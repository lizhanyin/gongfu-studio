import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { FakModel, FskinModel, FskelMeta, Influence } from "./types";

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
  /** 饰品透明叠层（共享几何，TFaceDetail 贴花） */
  overlayMesh: THREE.Mesh | null;
  overlayMat: THREE.MeshStandardMaterial | null;
  /** 几何顶点（含拆缝副本）的影响表，副本沿用源顶点 */
  inflOf: Influence[][];
  /** 各几何顶点的源顶点法线（扁平 xyz，蒙皮与编辑同步用） */
  srcNrm: Float32Array;
  /** 法线增量旋转用的每骨绑定逆矩阵（部件自身拟合，骨架未载时 null） */
  bindInvs: THREE.Matrix4[] | null;
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
  /** 骨架可视化（线条 + 关节小球）显隐，跨动作加载保持 */
  private skeletonVisible = true;
  /** 全部帧的扁平姿态：帧 × 骨 × 7 */
  private fskelPoses: Float32Array | null = null;
  private fskelBoneCount = 0;
  private qa = new THREE.Quaternion();
  private qb = new THREE.Quaternion();
  private tmpVec = new THREE.Vector3();
  /** 当前帧各骨世界矩阵（蒙皮数据源，applyFskelFrame 时刷新） */
  private worldMats: THREE.Matrix4[] = [];
  /** 每骨法线增量 D=W·bindInv 的帧内缓存（dStamp 标记有效性） */
  private dMats: THREE.Matrix4[] = [];
  private dStamp = new Int32Array(0);
  private dStampCounter = 0;
  /** 各骨共识绑定拟合（boneLocalPos→pos，跨部件多数分簇）。authored pos
   *  与多数部件摆放不一致的部件（Girl_hair05×0x6）静态模式下按此渲染
   *  绑定姿态，与头部对齐；一致部件保持 authored 原样。 */
  private bindMats: THREE.Matrix4[] = [];

  // ---- FAK 顶点动画 ----
  private fakModel: FakModel | null = null;
  private fakMeshes: THREE.Mesh[] = [];
  private fakGeoms: THREE.BufferGeometry[] = [];
  private fakMats: THREE.MeshStandardMaterial[] = [];
  /** 所有子网格共享的顶点属性（位置/法线逐帧更新，UV 恒定） */
  private fakPos: THREE.BufferAttribute | null = null;
  private fakNrm: THREE.BufferAttribute | null = null;

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
    mesh.frustumCulled = false; // CPU 蒙皮逐帧变形，静态包围球不可靠
    this.scene.add(mesh);
    const part: PartObj = {
      mesh, geom, mat, model, clones, overlayMesh: null, overlayMat: null,
      inflOf, srcNrm: Float32Array.from(srcNrm), bindInvs: null,
    };
    this.parts.set(path, part);
    // 动作模式已绑骨架时立即按当前姿态蒙皮；静态模式对齐离群部件
    if (this.skelBones.length) {
      this.refreshPartBind(part);
      this.skinPart(part);
    } else {
      this.applyStaticSkin();
    }
    this.applyMode(this.mode);
  }

  removePart(path: string) {
    const p = this.parts.get(path);
    if (!p) return;
    this.removeOverlay(p);
    this.scene.remove(p.mesh);
    p.geom.dispose();
    p.mat.dispose();
    this.parts.delete(path);
    // 静态模式共识可能变化（如锚定部件被移除），重新对齐
    if (!this.skelBones.length) this.applyStaticSkin();
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
    for (const m of this.fakMats) m.wireframe = on;
  }

  /** 高亮选中部件：其余半透明（饰品叠层随之） */
  highlight(path: string | null) {
    for (const [k, p] of this.parts) {
      const sel = path === null || k === path;
      p.mat.transparent = !sel;
      p.mat.opacity = sel ? 1.0 : 0.28;
      p.mat.depthWrite = true;
      if (p.overlayMat) {
        p.overlayMat.opacity = sel ? 1.0 : 0.28;
        p.overlayMat.needsUpdate = true;
      }
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
    if (p.bindInvs) this.skinPart(p);
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
      this.skelHelper.visible = this.skeletonVisible;
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
    this.jointMesh.visible = this.skeletonVisible;
    this.scene.add(this.jointMesh);

    // CPU 蒙皮缓冲：各骨世界矩阵 + 法线增量缓存
    this.worldMats = Array.from({ length: n }, () => new THREE.Matrix4());
    this.dMats = Array.from({ length: n }, () => new THREE.Matrix4());
    this.dStamp = new Int32Array(n);
    for (const p of this.parts.values()) this.refreshPartBind(p);

    this.applyFskelFrame(0, 0, 0);
  }

  /**
   * 部件自身的每骨绑定逆矩阵（仅法线用）。位置蒙皮是逐影响的
   * Σ w·M_b·boneLocalPos（引擎精确数据，无需绑定矩阵）；法线只有
   * 绑定姿势下的混合结果，按 R(M_b·T_b⁻¹) 加权混合——T_b 用本部件
   * 单影响点对刚体拟合（不足 3 点补多影响近似对），旋转误差只影响光照。
   */
  private refreshPartBind(p: PartObj) {
    const bc = this.fskelBoneCount || this.bindMats.length;
    if (!bc) {
      p.bindInvs = null;
      return;
    }
    const exact: [number[], number[]][][] = Array.from({ length: bc }, () => []);
    const approx: [number[], number[]][][] = Array.from({ length: bc }, () => []);
    for (const v of p.model.vertices) {
      for (const inf of v.influences) {
        if (inf.boneIndex >= bc) continue;
        const pair: [number[], number[]] = [inf.boneLocalPos, v.pos];
        if (v.influences.length === 1 || inf.weight >= 0.99) exact[inf.boneIndex].push(pair);
        else approx[inf.boneIndex].push(pair);
      }
    }
    p.bindInvs = Array.from({ length: bc }, (_, b) => {
      const pool = exact[b].length >= 3 ? exact[b] : exact[b].concat(approx[b]);
      return pool.length >= 2
        ? this.fitBindMatrix(pool).invert()
        : new THREE.Matrix4();
    });
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
      else if (p.bindInvs) this.restorePart(p); // 曾按离群蒙皮、现已一致 → 还原
    }
    if (!skinned) return;
    // 与 bindMats 分离的实例：避免动作模式覆写共识拟合
    this.worldMats = this.bindMats.map((m) => m.clone());
    this.dMats = Array.from({ length: this.bindMats.length }, () => new THREE.Matrix4());
    this.dStamp = new Int32Array(this.bindMats.length);
    for (const p of this.parts.values()) {
      if (!this.partBindInconsistent(p)) continue;
      this.refreshPartBind(p);
      this.skinPart(p);
    }
  }

  /**
   * 逐部件 CPU 蒙皮（= 游戏真实 LBS）：
   *   pos = Σ w_b · M_b · boneLocalPos_b   —— 每影响独立精确变换后加权
   *   nrm = normalize(Σ w_b · R(M_b·T_b⁻¹) · n)
   * 绑定矩阵近似（共识/每部件拟合）在裙摆等骨骼点对稀疏、多影响混合的
   * 部件上偏差可达 16+ 单位（与布料运动幅度同量级，动作中穿模的根因）；
   * boneLocalPos 是引擎逐顶点写入的精确数据，直接加权即为游戏渲染结果。
   */
  private skinPart(p: PartObj) {
    const invs = p.bindInvs;
    if (!invs || !this.worldMats.length) return;
    const boneCount = this.fskelBoneCount || this.worldMats.length;
    const vcount = p.model.vertices.length;
    const n = p.inflOf.length;
    const posAttr = p.geom.getAttribute("position") as THREE.BufferAttribute;
    const nrmAttr = p.geom.getAttribute("normal") as THREE.BufferAttribute;
    const pa = posAttr.array as Float32Array;
    const na = nrmAttr.array as Float32Array;
    const stamp = ++this.dStampCounter;
    for (let i = 0; i < n; i++) {
      const infl = p.inflOf[i];
      let wsum = 0;
      for (let k = 0; k < infl.length; k++) {
        if (infl[k].boneIndex < boneCount) wsum += infl[k].weight;
      }
      const o = i * 3;
      const no = (i < vcount ? i : p.clones[i - vcount].src) * 3;
      const s0 = p.srcNrm[no], s1 = p.srcNrm[no + 1], s2 = p.srcNrm[no + 2];
      if (wsum <= 0) {
        // 无有效骨骼（骨架比部件骨骼少）：保持绑定姿势
        const src = i < vcount ? i : p.clones[i - vcount].src;
        const v = p.model.vertices[src];
        pa[o] = v.pos[0]; pa[o + 1] = v.pos[1]; pa[o + 2] = v.pos[2];
        na[o] = s0; na[o + 1] = s1; na[o + 2] = s2;
        continue;
      }
      let px = 0, py = 0, pz = 0, nx = 0, ny = 0, nz = 0;
      for (let k = 0; k < infl.length; k++) {
        const inf = infl[k];
        const bi = inf.boneIndex;
        if (bi >= boneCount) continue;
        const w = inf.weight / wsum;
        const M = this.worldMats[bi].elements;
        const lp = inf.boneLocalPos;
        // 列主序：M·lp
        px += w * (M[0] * lp[0] + M[4] * lp[1] + M[8] * lp[2] + M[12]);
        py += w * (M[1] * lp[0] + M[5] * lp[1] + M[9] * lp[2] + M[13]);
        pz += w * (M[2] * lp[0] + M[6] * lp[1] + M[10] * lp[2] + M[14]);
        if (this.dStamp[bi] !== stamp) {
          this.dStamp[bi] = stamp;
          this.dMats[bi].multiplyMatrices(this.worldMats[bi], invs[bi]);
        }
        const D = this.dMats[bi].elements;
        nx += w * (D[0] * s0 + D[4] * s1 + D[8] * s2);
        ny += w * (D[1] * s0 + D[5] * s1 + D[9] * s2);
        nz += w * (D[2] * s0 + D[6] * s1 + D[10] * s2);
      }
      pa[o] = px; pa[o + 1] = py; pa[o + 2] = pz;
      const nl = Math.hypot(nx, ny, nz);
      if (nl > 1e-6) {
        na[o] = nx / nl; na[o + 1] = ny / nl; na[o + 2] = nz / nl;
      } else {
        na[o] = s0; na[o + 1] = s1; na[o + 2] = s2;
      }
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
    p.bindInvs = null;
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
      this.scene.add(mesh);
      p.overlayMesh = mesh;
    }
  }

  private removeOverlay(p: PartObj) {
    if (p.overlayMesh) {
      this.scene.remove(p.overlayMesh);
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
    // 关节小球跟随世界坐标
    const m = new THREE.Matrix4();
    for (let b = 0; b < n; b++) {
      m.setPosition(this.skelBones[b].getWorldPosition(this.tmpVec));
      this.jointMesh?.setMatrixAt(b, m);
    }
    if (this.jointMesh) this.jointMesh.instanceMatrix.needsUpdate = true;
    // 全部件按当前姿态重新蒙皮
    for (const p of this.parts.values()) this.skinPart(p);
  }

  clearFskel() {
    for (const p of this.parts.values()) this.restorePart(p);
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
    this.worldMats = [];
    this.dMats = [];
    this.dStamp = new Int32Array(0);
    this.applyStaticSkin(); // 默认状态对齐离群部件（hair05）
  }

  // ---------------- FAK 顶点动画 ----------------

  /**
   * 构建顶点动画模型：每个子网格一个 Mesh，几何引用同一份共享顶点池属性
   * （position/normal 逐帧更新，UV 经全帧验证恒定作静态属性）。
   * mats[i] 为子网格 i 的贴图（按材质记录序号一一对应）。
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
    this.fakPos.setUsage(THREE.DynamicDrawUsage);
    this.fakNrm.setUsage(THREE.DynamicDrawUsage);

    model.submeshes.forEach((s, i) => {
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", this.fakPos!);
      geom.setAttribute("normal", this.fakNrm!);
      geom.setAttribute("uv", new THREE.BufferAttribute(uv, 2));
      geom.setIndex(s.triangles);
      const fm = model.materials[Math.min(i, model.materials.length - 1)];
      const mat = new THREE.MeshStandardMaterial({
        side: THREE.DoubleSide,
        roughness: 0.75,
        metalness: 0.0,
      });
      if (fm) {
        mat.color.setRGB(fm.diffuse[0], fm.diffuse[1], fm.diffuse[2]);
        mat.emissive.setRGB(fm.emissive[0], fm.emissive[1], fm.emissive[2]);
        mat.roughness = Math.max(0.05, Math.min(1, 1 - fm.specPower / 60));
      }
      const m = mats[i];
      if (m?.tex && m.file) mat.map = this.getTexture(m.file, m.tex);
      const mesh = new THREE.Mesh(geom, mat);
      mesh.frustumCulled = false; // 顶点动画变形后静态包围球不再可靠
      this.scene.add(mesh);
      this.fakMeshes.push(mesh);
      this.fakGeoms.push(geom);
      this.fakMats.push(mat);
    });
    this.applyFakFrame(0, 0, 0);
  }

  /** 应用第 i0 帧；给出 i1/alpha 时做位置/法线 lerp 平滑插值（法线重归一化） */
  applyFakFrame(i0: number, i1: number, alpha: number) {
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
    for (const mesh of this.fakMeshes) this.scene.remove(mesh);
    for (const g of this.fakGeoms) g.dispose();
    for (const m of this.fakMats) m.dispose();
    this.fakMeshes = [];
    this.fakGeoms = [];
    this.fakMats = [];
    this.fakPos = null;
    this.fakNrm = null;
    this.fakModel = null;
  }

  hasFak() {
    return this.fakMeshes.length > 0;
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
    if (this.jointMesh) this.jointMesh.visible = v;
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
    if (this.fakMeshes.length) {
      // FAK 为逐帧变形网格，用当前顶点位置直接求包围盒
      const pa = this.fakPos?.array as Float32Array | undefined;
      if (pa) {
        for (let i = 0; i < pa.length; i += 3) {
          box.expandByPoint(new THREE.Vector3(pa[i], pa[i + 1], pa[i + 2]));
        }
      }
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
