import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { FakModel, FsmModel, FskinModel, FskelMeta, Influence } from "./types";

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
  /** 各几何顶点的源顶点法线（扁平 xyz，编辑同步用；蒙皮法线恒静态，见 skinPart） */
  srcNrm: Float32Array;
  /** 几何当前是否处于蒙皮态（false = authored 绑定姿势直出） */
  skinned: boolean;
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
  /** 各骨共识绑定拟合（boneLocalPos→pos，跨部件多数分簇）。authored pos
   *  与多数部件摆放不一致的部件（Girl_hair05×0x6）静态模式下按此渲染
   *  绑定姿态，与头部对齐；一致部件保持 authored 原样。 */
  private bindMats: THREE.Matrix4[] = [];

  // ---- 裙摆布料骨（Bone* 链）----
  /** 骨架父索引（worldsFromPoses 组合世界矩阵用） */
  private skelParents: number[] = [];
  /** 布料骨：Bone* 布料链成员（见 detectCloth） */
  private clothBones = new Set<number>();
  /** 每帧布料合成矩阵（弹簧模拟驱动，updateClothMats 刷新） */
  private clothMats = new Map<number, THREE.Matrix4>();
  /** 布料链载体骨索引（链根的公共父骨，裙摆 = 骨盆），无布料骨时 -1 */
  private clothCarrier = -1;
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
  /** 腿部碰撞球（骨盆绑定系半径 + 骨索引，中心取骨骼局部 X 轴中点） */
  private clothColliders: { bone: number; len: number }[] = [];
  /** 上次 applyFskelFrame 的绝对帧（i0+alpha），检测 seek */
  private clothLastAbs = NaN;
  private tmpScale = new THREE.Vector3(1, 1, 1);
  // 模拟参数：弹簧 K（s⁻²）、速度阻尼、重力（u/s²，模型身高 ~160）
  private static readonly CLOTH_K = 260;
  private static readonly CLOTH_DAMP = 0.9;
  private static readonly CLOTH_G = 300;
  /** 腿部碰撞球半径系数（×段长）：真实腿半径远小于半段长，肥球会把静止裙摆直接顶飞 */
  private static readonly CLOTH_LEG_R = 0.18;
  // 每帧复用的临时对象
  private cCarry = new THREE.Matrix4();
  private cAnchor = new THREE.Vector3();
  private cTarget = new THREE.Vector3();
  private cVel = new THREE.Vector3();
  private cDir1 = new THREE.Vector3();
  private cCenter = new THREE.Vector3();
  private cT1 = new THREE.Matrix4();

  // ---- FAK 顶点动画 ----
  private fakModel: FakModel | null = null;
  private fakMeshes: THREE.Mesh[] = [];
  private fakGeoms: THREE.BufferGeometry[] = [];
  private fakMats: THREE.MeshStandardMaterial[] = [];
  /** 所有子网格共享的顶点属性（位置/法线逐帧更新，UV 恒定） */
  private fakPos: THREE.BufferAttribute | null = null;
  private fakNrm: THREE.BufferAttribute | null = null;

  // ---- FSM 静态武器网格 ----
  private fsmMeshes: THREE.Mesh[] = [];
  private fsmGeoms: THREE.BufferGeometry[] = [];
  private fsmMats: THREE.MeshStandardMaterial[] = [];
  /** 挂点标记（小球），mesh/材质与 fsmMeshes 分开管理 */
  private fsmDummyGroup: THREE.Group | null = null;
  private fsmDummyGeoms: THREE.SphereGeometry[] = [];
  private fsmDummyMats: THREE.Material[] = [];

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
      inflOf, srcNrm: Float32Array.from(srcNrm),
      skinned: this.skelBones.length > 0,
    };
    this.parts.set(path, part);
    // 动作模式已绑骨架时立即按当前姿态蒙皮；静态模式对齐离群部件
    if (this.skelBones.length) {
      this.detectCloth(); // 布料检测的点集随部件变化
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
    if (this.skelBones.length) {
      this.detectCloth(); // 布料检测的点集随部件变化
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
    for (const m of this.fsmMats) m.wireframe = on;
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
    if (p.skinned) this.skinPart(p);
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

    // CPU 蒙皮缓冲：各骨世界矩阵（法线静态，引擎不做法线蒙皮，无需绑定拟合）
    this.worldMats = Array.from({ length: n }, () => new THREE.Matrix4());
    this.skelParents = meta.parents;
    for (const p of this.parts.values()) p.skinned = true;
    this.detectCloth();

    this.applyFskelFrame(0, 0, 0);
  }

  /**
   * 裙摆布料骨检测与次级运动模拟初始化。
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
  private detectCloth() {
    this.clothBones.clear();
    this.clothMats.clear();
    this.clothSims = [];
    this.clothColliders = [];
    this.clothCarrier = -1;
    this.clothTpInv = null;
    this.clothLastAbs = NaN;
    if (!this.fskelPoses || !this.skelBones.length || !this.skelParents.length) return;
    const n = this.fskelBoneCount;

    // 每骨点数据：单影响点对（载体拟合用）+ 全部影响顶点位置（模拟锚点用）
    const pairsOf: [number[], number[]][][] = Array.from({ length: n }, () => []);
    const vertsOf: number[][][] = Array.from({ length: n }, () => []);
    for (const p of this.parts.values()) {
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
    const isBoneHelper = (b: number) => /^Bone\d+$/.test(this.skelBones[b]?.name ?? "");
    for (let b = 0; b < n; b++) {
      if (isBoneHelper(b) && vertsOf[b].length > 0) this.clothBones.add(b);
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

    // 腿部碰撞球：Thigh/Calf/Foot 骨局部 X 轴（骨骼方向）中点，半径按段长
    for (let b = 0; b < n; b++) {
      const name = this.skelBones[b].name;
      if (!/Thigh|Calf|Foot/.test(name)) continue;
      // 段长 = 子骨局部平移长度（骨骼沿 +X）
      let len = 12;
      for (let c = 0; c < n; c++) {
        if (this.skelParents[c] === b) {
          const o = c * 7;
          len = Math.hypot(this.fskelPoses[o], this.fskelPoses[o + 1], this.fskelPoses[o + 2]);
          break;
        }
      }
      this.clothColliders.push({ bone: b, len });
    }

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
    this.updateClothMats(this.clothLastAbs);
  }

  /** 直接从姿态数组组合某帧各骨世界矩阵（不动当前骨架姿态） */
  private worldsFromPoses(frame: number): THREE.Matrix4[] {
    const n = this.fskelBoneCount;
    const stride = n * 7;
    const poses = this.fskelPoses!;
    const last = Math.max(Math.floor(poses.length / stride) - 1, 0);
    const base = Math.min(Math.max(frame, 0), last) * stride;
    const out = Array.from({ length: n }, () => new THREE.Matrix4());
    for (let b = 0; b < n; b++) {
      const o = base + b * 7;
      this.qa.set(poses[o + 3], poses[o + 4], poses[o + 5], poses[o + 6]).normalize();
      out[b].compose(
        this.tmpVec.set(poses[o], poses[o + 1], poses[o + 2]),
        this.qa,
        this.tmpScale
      );
      const p = this.skelParents[b];
      if (p >= 0 && p < b) out[b].premultiply(out[p]);
    }
    return out;
  }

  /**
   * 布料次级运动步进 + 合成矩阵刷新（applyFskelFrame 每帧调用）。
   * abs = i0+alpha 绝对帧；倒退/跳跃视为 seek，模拟点重置到携带目标。
   */
  private updateClothMats(abs: number) {
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

    for (const s of this.clothSims) {
      this.cAnchor.copy(s.anchor).applyMatrix4(this.cCarry);
      this.cTarget.copy(s.target).applyMatrix4(this.cCarry);
      if (seek || !Number.isFinite(s.pos.x)) {
        s.pos.copy(this.cTarget);
        s.prev.copy(this.cTarget);
      } else if (dt > 0) {
        // verlet：速度阻尼 + 微重力 + 弹簧拉向携带目标（速度限幅防炸）
        this.cVel.subVectors(s.pos, s.prev).multiplyScalar(Viewer.CLOTH_DAMP);
        const vmax = 6 * dt * 30; // ≤6 单位/帧
        if (this.cVel.length() > vmax) this.cVel.setLength(vmax);
        s.prev.copy(s.pos);
        s.pos.add(this.cVel);
        s.pos.y -= Viewer.CLOTH_G * dt * dt;
        s.pos.lerp(this.cTarget, Math.min(1, Viewer.CLOTH_K * dt * dt));
      }
      // 腿部碰撞球推出（两轮；半径=真实腿粗，避免静止裙摆被顶飞）
      for (let iter = 0; iter < 2; iter++) {
        for (const c of this.clothColliders) {
          const m = this.worldMats[c.bone];
          if (!m) continue;
          this.cCenter.set(c.len * 0.45, 0, 0).applyMatrix4(m);
          this.cDir1.subVectors(s.pos, this.cCenter);
          const r = c.len * Viewer.CLOTH_LEG_R;
          const d2 = this.cDir1.lengthSq();
          if (d2 < r * r && d2 > 1e-8) {
            s.pos.copy(this.cCenter).addScaledVector(this.cDir1.normalize(), r);
          }
        }
      }
      // 安全钳位：模拟点离锚点不超过静止臂长的 2 倍（防碰撞/大步长累积飞出）
      this.cDir1.subVectors(s.pos, this.cAnchor);
      const arm = this.cTarget.distanceTo(this.cAnchor);
      if (arm > 1e-4 && this.cDir1.length() > arm * 2) {
        s.pos.copy(this.cAnchor).addScaledVector(this.cDir1.normalize(), arm * 2);
      }
      // 应用：平移跟随。不用旋转——簇臂长可能只有 1~3 单位，小滞后会被
      // 放大成接近 180° 的翻转（面板甩飞）；有界平移同样有滞后摆动感且绝无翻转
      this.cDir1.subVectors(s.pos, this.cTarget);
      const off = this.cDir1.length();
      if (off > 8) this.cDir1.multiplyScalar(8 / off); // 裙摆滞后上限
      const out = this.clothMats.get(s.bone)!;
      out.multiplyMatrices(
        this.cT1.makeTranslation(this.cDir1.x, this.cDir1.y, this.cDir1.z),
        this.cCarry
      );
    }
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
   * 例外：布料骨（detectCloth 标记的裙摆 Bone* 链）改用次级运动模拟的
   * 合成矩阵作用在 authored pos 上（游戏为运行时布料模拟，文件姿态
   * 只是种子，见 detectCloth 注释）。
   * 绑定矩阵近似（共识拟合）只在无骨架的静态模式兜底离群部件；
   * boneLocalPos 是引擎逐顶点写入的精确数据，直接加权即为游戏渲染结果。
   */
  private skinPart(p: PartObj) {
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
    this.updateClothMats(i0 + alpha);
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
    this.skelParents = [];
    this.clothBones.clear();
    this.clothMats.clear();
    this.clothSims = [];
    this.clothColliders = [];
    this.clothCarrier = -1;
    this.clothTpInv = null;
    this.clothLastAbs = NaN;
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

  // ---------------- FSM 静态武器网格 ----------------

  /**
   * 构建静态武器网格：每个子网格一个 Mesh，共享同一份顶点池属性
   * （pos/normal/uv 直接取自文件，法线写入时归一化）。
   * mats[i] 为子网格 i 的贴图；挂点用小球标记（位置取 values 前 3 个）。
   */
  setFsm(model: FsmModel, mats: { tex: TexData | null; file: string | null }[]) {
    this.clearFsm();
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

    model.submeshes.forEach((s, i) => {
      const geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.BufferAttribute(pos, 3));
      geom.setAttribute("normal", new THREE.BufferAttribute(nrm, 3));
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
      this.scene.add(mesh);
      this.fsmMeshes.push(mesh);
      this.fsmGeoms.push(geom);
      this.fsmMats.push(mat);
    });

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
    this.scene.add(group);
    this.fsmDummyGroup = group;
  }

  clearFsm() {
    for (const mesh of this.fsmMeshes) this.scene.remove(mesh);
    for (const g of this.fsmGeoms) g.dispose();
    for (const m of this.fsmMats) m.dispose();
    this.fsmMeshes = [];
    this.fsmGeoms = [];
    this.fsmMats = [];
    if (this.fsmDummyGroup) {
      this.scene.remove(this.fsmDummyGroup);
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
    for (const mesh of this.fsmMeshes) {
      box.expandByObject(mesh);
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
