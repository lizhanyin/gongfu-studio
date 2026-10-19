<template>
  <n-config-provider :theme="darkTheme" :locale="zhCN" :theme-overrides="themeOverrides">
    <div class="app">
      <header class="app-header">
        <span class="title">Gongfu Studio</span>
        <n-button-group size="small">
          <n-button :type="topMode === 'skin' ? 'primary' : 'default'" @click="switchMode('skin')">皮肤</n-button>
          <n-button :type="topMode === 'act' ? 'primary' : 'default'" @click="switchMode('act')">动作</n-button>
        </n-button-group>
        <template v-if="topMode === 'skin'">
          <n-input v-model:value="rootDir" size="small" style="width: 300px" placeholder="模型目录 (AvatarGirl)" @keyup.enter="scan" />
          <n-button size="small" @click="pickDir">浏览</n-button>
          <n-button size="small" type="primary" :loading="scanning" @click="scan">扫描</n-button>
          <n-divider vertical />
          <n-button-group size="small">
            <n-button :type="renderMode === 'shaded' ? 'primary' : 'default'" @click="renderMode = 'shaded'">着色</n-button>
            <n-button :type="renderMode === 'bones' ? 'primary' : 'default'" @click="renderMode = 'bones'">骨骼</n-button>
            <n-button :type="renderMode === 'uv' ? 'primary' : 'default'" @click="renderMode = 'uv'">UV</n-button>
          </n-button-group>
          <n-checkbox size="small" v-model:checked="wireframe">线框</n-checkbox>
          <n-button size="small" @click="viewer?.frameAll()">定位</n-button>
          <div style="flex: 1"></div>
          <n-button size="small" @click="exportObj" :disabled="!visiblePaths.length">导出 OBJ</n-button>
          <n-button size="small" type="warning" @click="saveSelected" :disabled="!selectedModel">保存当前部件</n-button>
        </template>
        <template v-else>
          <n-button size="small" type="primary" :loading="actScanning" @click="scanAct">扫描动作目录</n-button>
          <n-button size="small" @click="pickActFile">打开动作文件</n-button>
          <n-checkbox size="small" v-model:checked="showSkinOverlay" @update:checked="viewer?.setPartsVisible($event)">叠加皮肤网格</n-checkbox>
          <n-button size="small" @click="viewer?.frameSkeleton()">定位</n-button>
          <div style="flex: 1"></div>
          <n-button size="small" @click="exportBvh(false)" :disabled="!actMeta">导出 BVH (全部帧)</n-button>
          <n-button size="small" @click="exportBvh(true)" :disabled="!actMeta">导出 BVH (当前动作)</n-button>
          <n-button size="small" type="primary" @click="exportGlb" :disabled="!actMeta">导出 GLB</n-button>
        </template>
      </header>

      <div class="app-main">
        <aside class="left-panel">
          <!-- 皮肤模式：部件组 -->
          <template v-if="topMode === 'skin'">
            <div class="section-title">部件组（同几何 = 换装变体）</div>
            <div v-if="!modelSet" class="kv" style="padding: 8px">尚未扫描目录</div>
            <div
              v-for="g in modelSet?.groups ?? []"
              :key="g.signature"
              class="part-card"
              :class="{ selected: selectedPart === groupSel.get(g.signature) }"
              @click="selectPart(groupSel.get(g.signature) ?? null)"
            >
              <div class="row">
                <n-switch
                  size="small"
                  :value="groupVis.get(g.signature) ?? false"
                  @update:value="(v: boolean) => toggleGroup(g, v)"
                />
                <span class="name" :title="g.label">{{ g.label }}</span>
                <span v-if="isDirty(groupSel.get(g.signature))" class="dirty-dot">●</span>
                <n-select
                  v-if="g.parts.length > 1"
                  size="tiny"
                  style="width: 140px"
                  :value="groupSel.get(g.signature)"
                  :options="g.parts.map((p) => ({ label: p.fileName, value: p.path }))"
                  @update:value="(v: string) => changeVariant(g, v)"
                />
              </div>
              <div class="meta" v-if="partInfoOf(g)">
                {{ partInfoOf(g)!.vertexCount }} 顶点 · {{ partInfoOf(g)!.triangleCount }} 三角形 ·
                骨骼 {{ bonesText(partInfoOf(g)!) }}
                <br />
                纹理: {{ texShort(partInfoOf(g)!) }}
              </div>
            </div>
          </template>

          <!-- 动作模式：文件 + 动作列表 -->
          <template v-else>
            <div class="section-title">动作文件（{{ actFiles.length }}）</div>
            <div v-if="!actFiles.length && !actScanning" class="kv" style="padding: 8px">
              点击「扫描动作目录」读取 {{ rootDir }}\\act 下的 .FSKEL，或「打开动作文件」选择单个文件。
            </div>
            <div
              v-for="f in actFiles"
              :key="f.path"
              class="part-card"
              :class="{ selected: actMeta?.path === f.path }"
              @click="openActFile(f.path)"
            >
              <div class="row">
                <span class="name">{{ f.fileName }}</span>
                <span class="meta-inline">{{ f.frameCount }} 帧</span>
              </div>
              <div class="meta">{{ f.boneCount }} 骨骼 · {{ (f.byteLen / 1024).toFixed(0) }} KB</div>
            </div>

            <template v-if="actMeta">
              <div class="section-title" style="margin-top: 10px">动作分段（{{ actMeta.actions.length }}）</div>
              <div
                v-for="(a, i) in actMeta.actions"
                :key="i"
                class="part-card"
                :class="{ selected: currentAction === i }"
                @click="selectAction(i)"
              >
                <div class="row">
                  <span class="name">{{ a.name }}</span>
                  <span class="meta-inline">{{ a.frameCount }} 帧 · {{ (a.frameCount / 30).toFixed(2) }}s</span>
                </div>
              </div>
            </template>
          </template>
        </aside>

        <div class="viewport" ref="viewportRef">
          <div class="viewport-overlay">
            <div>部件: {{ visibleCount }} / 组: {{ modelSet?.groups.length ?? 0 }}</div>
            <div>顶点: {{ statVerts }} · 三角形: {{ statTris }}</div>
          </div>
          <div class="viewport-hint">左键旋转 · 右键平移 · 滚轮缩放 · 点击网格选中部件</div>

          <!-- 动作播放条 -->
          <div v-if="topMode === 'act' && actMeta" class="play-bar">
            <n-button size="small" circle @click="playing = !playing">{{ playing ? "⏸" : "▶" }}</n-button>
            <n-slider
              style="flex: 1; min-width: 120px"
              :min="0"
              :max="Math.max(actionFrames - 1, 1)"
              :step="0.01"
              :value="playTime"
              :format-tooltip="(v: number) => `${Math.floor(v)} 帧 · ${(v / 30).toFixed(2)}s`"
              @update:value="scrub"
            />
            <span class="play-info">
              {{ Math.floor(playTime) }}/{{ actionFrames - 1 }} 帧 ·
              {{ (playTime / 30).toFixed(2) }}s
            </span>
            <n-select
              size="tiny"
              style="width: 74px"
              :value="speed"
              :options="[0.25, 0.5, 1, 2].map((s) => ({ label: s + 'x', value: s }))"
              @update:value="(v: number) => (speed = v)"
            />
          </div>
        </div>

        <aside class="right-panel">
          <!-- 动作模式：文件信息 -->
          <div v-if="topMode === 'act'" class="kv" style="padding: 12px">
            <template v-if="actMeta">
              <b>{{ actMeta.fileName }}</b><br />
              版本 0x{{ actMeta.version.toString(16) }} · 校验 0x{{ actMeta.checksum.toString(16) }}<br />
              骨骼 {{ actMeta.boneCount }} · 帧 {{ actMeta.frameCount }} ·
              {{ (1 / actMeta.frameDt).toFixed(0) }} FPS<br />
              动作 {{ actMeta.actions.length }} 个 ·
              总时长 {{ (actMeta.frameCount / 30).toFixed(2) }}s<br />
              <span v-if="actLoading">帧数据加载中…</span>
              <n-divider style="margin: 8px 0" />
              导出说明：BVH 导入 Blender 得到骨架 + 动作；GLB 面向
              three.js / 引擎（含全部动作分段）。
            </template>
            <template v-else>
              动作模式：预览 .FSKEL 骨架动画，支持播放 / 变速 / 逐帧拖动。<br /><br />
              左侧选择动作文件后自动播放。
            </template>
          </div>

          <div v-if="topMode === 'skin' && !selectedModel" class="kv" style="padding: 12px">
            点击左侧部件组或视口中的网格以编辑。<br /><br />
            支持编辑: 材质颜色 / 高光 / 纹理路径 / 顶点坐标与 UV / 包围盒重算。<br />
            保存采用字节级往返验证过的序列化器。
          </div>
          <template v-if="topMode === 'skin' && selectedModel">
            <div class="kv" style="margin-bottom: 6px">
              <b>{{ shortName(selectedModel.path) }}</b>
              <span v-if="isDirty(selectedModel.path)" class="dirty-dot"> ● 已修改</span>
            </div>
            <n-tabs type="line" size="small" default-value="material">
              <n-tab-pane name="material" tab="材质">
                <div class="form-row">
                  <label>漫反射</label>
                  <n-color-picker
                    style="flex: 1"
                    :modes="['rgb']"
                    :show-alpha="true"
                    :value="rgbaStr(selectedModel.material.diffuse)"
                    @update:value="(v: string) => setColor(selectedModel!.material.diffuse, v)"
                  />
                </div>
                <div class="form-row">
                  <label>环境光</label>
                  <n-color-picker
                    style="flex: 1"
                    :modes="['rgb']"
                    :show-alpha="true"
                    :value="rgbaStr(selectedModel.material.ambient)"
                    @update:value="(v: string) => setColor(selectedModel!.material.ambient, v)"
                  />
                </div>
                <div class="form-row">
                  <label>自发光</label>
                  <n-color-picker
                    style="flex: 1"
                    :modes="['rgb']"
                    :show-alpha="true"
                    :value="rgbaStr(selectedModel.material.emissive)"
                    @update:value="(v: string) => setColor(selectedModel!.material.emissive, v)"
                  />
                </div>
                <div class="form-row">
                  <label>高光色</label>
                  <n-color-picker
                    style="flex: 1"
                    :modes="['rgb']"
                    :show-alpha="true"
                    :value="rgbaStr(selectedModel.material.specular)"
                    @update:value="(v: string) => setColor(selectedModel!.material.specular, v)"
                  />
                </div>
                <div class="form-row">
                  <label>高光强度</label>
                  <n-input-number
                    size="small"
                    style="flex: 1"
                    :value="selectedModel.material.specPower"
                    @update:value="(v: number | null) => { selectedModel!.material.specPower = v ?? 0; afterEdit(); }"
                  />
                </div>
                <div class="form-row">
                  <label>纹理路径</label>
                  <n-input
                    size="small"
                    style="flex: 1"
                    :value="selectedModel.material.texturePath"
                    @update:value="(v: string) => { selectedModel!.material.texturePath = v; }"
                  />
                </div>
                <n-button size="small" @click="applyTexture" style="margin-bottom: 10px">
                  应用纹理（按文件名匹配目录内贴图）
                </n-button>
                <div v-if="texPreviewUrl" style="margin-top: 6px">
                  <img class="tex-preview" :src="texPreviewUrl" style="width: 100%" />
                  <div class="kv" style="margin-top: 4px">
                    {{ texInfoText }}
                  </div>
                </div>
              </n-tab-pane>

              <n-tab-pane name="vertices" tab="顶点">
                <n-data-table
                  size="small"
                  :columns="vertexColumns"
                  :data="selectedModel.vertices"
                  :pagination="{ pageSize: 25 }"
                  :max-height="520"
                  virtual-scroll
                />
              </n-tab-pane>

              <n-tab-pane name="file" tab="文件">
                <div class="kv">
                  <b>路径:</b> {{ selectedModel.path }}<br />
                  <b>版本:</b> 0x{{ selectedModel.version.toString(16) }}<br />
                  <b>顶点:</b> {{ selectedModel.vertices.length }} ·
                  <b>三角形:</b> {{ selectedModel.triangles.length / 3 }}<br />
                  <b>包围盒 max:</b> {{ fmt3(selectedModel.bboxMax) }}<br />
                  <b>包围盒 min:</b> {{ fmt3(selectedModel.bboxMin) }}<br />
                  <b>哈希1:</b> 0x{{ (selectedModel.indexMeta.hash1 >>> 0).toString(16) }} ·
                  <b>哈希2:</b> 0x{{ (selectedModel.indexMeta.hash2 >>> 0).toString(16) }}
                </div>
                <n-space style="margin-top: 10px">
                  <n-button size="small" @click="recomputeBbox">按顶点重算包围盒</n-button>
                  <n-button size="small" type="warning" @click="saveSelected">保存</n-button>
                  <n-button size="small" @click="saveAs">另存为</n-button>
                </n-space>
              </n-tab-pane>
            </n-tabs>
          </template>
        </aside>
      </div>

      <footer class="app-footer">
        <span>{{ modelSet ? `目录: ${modelSet.rootDir}` : "" }}</span>
        <span>可见部件: {{ visiblePaths.map(shortName).join(" + ") || "无" }}</span>
      </footer>
    </div>
  </n-config-provider>
</template>

<script setup lang="ts">
import { h, onMounted, onUnmounted, reactive, ref, computed, watch } from "vue";
import {
  darkTheme,
  zhCN,
  createDiscreteApi,
  NInputNumber,
  type GlobalThemeOverrides,
} from "naive-ui";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api } from "./api";
import type { FskinModel, FskelBrief, FskelMeta, PartGroup, PartInfo } from "./types";
import { Viewer, texDataFromDto, type TexData } from "./viewer";

const { message } = createDiscreteApi(["message"], {
  configProviderProps: { theme: darkTheme },
});

const themeOverrides: GlobalThemeOverrides = {
  common: { bodyColor: "#101014", cardColor: "#181820" },
};

const rootDir = ref("G:\\pojie\\ai\\AvatarGirl");
const modelSet = ref<Awaited<ReturnType<typeof api.scanModelSet>> | null>(null);
const scanning = ref(false);
const renderMode = ref<"shaded" | "bones" | "uv">("shaded");
const wireframe = ref(false);
const selectedPart = ref<string | null>(null);

const loaded = reactive(new Map<string, FskinModel>());
const texCache = reactive(new Map<string, TexData>());
const dirty = reactive(new Set<string>());
const groupSel = reactive(new Map<string, string>()); // signature -> path
const groupVis = reactive(new Map<string, boolean>()); // signature -> bool

const viewportRef = ref<HTMLElement | null>(null);
let viewer: Viewer | null = null;

// ---------- FSKEL 动作模式 ----------
const topMode = ref<"skin" | "act">("skin");
const actFiles = ref<FskelBrief[]>([]);
const actScanning = ref(false);
const actMeta = ref<FskelMeta | null>(null);
const actPoses = ref<Float32Array | null>(null);
const actLoading = ref(false);
const currentAction = ref(0);
const playing = ref(false);
const playTime = ref(0);
const speed = ref(1);
const showSkinOverlay = ref(true);
let playRaf = 0;
let lastTs = 0;

const currentActionInfo = computed(() => actMeta.value?.actions[currentAction.value] ?? null);
/** 当前动作的帧区间 [from, to) */
const actionRange = computed<[number, number]>(() => {
  const a = currentActionInfo.value;
  if (!a) return [0, actMeta.value?.frameCount ?? 0];
  return [a.firstFrame, a.firstFrame + a.frameCount];
});
const actionFrames = computed(() => actionRange.value[1] - actionRange.value[0]);

function switchMode(m: "skin" | "act") {
  if (topMode.value === m) return;
  topMode.value = m;
  if (m === "act") {
    viewer?.setPartsVisible(showSkinOverlay.value);
    if (!actFiles.value.length) scanAct();
  } else {
    playing.value = false;
    viewer?.clearFskel();
    viewer?.setPartsVisible(true);
  }
}

async function scanAct() {
  actScanning.value = true;
  try {
    const dir = rootDir.value.replace(/[\\/]+$/, "") + "\\act";
    actFiles.value = await api.scanActDir(dir);
    message.success(`动作目录扫描完成: ${actFiles.value.length} 个文件`);
  } catch (e) {
    message.error(`扫描动作目录失败: ${e}`);
  } finally {
    actScanning.value = false;
  }
}

async function pickActFile() {
  const p = await open({
    defaultPath: rootDir.value,
    filters: [{ name: "FSKEL 动作", extensions: ["fskel"] }],
  });
  if (p) {
    if (!actFiles.value.some((f) => f.path === p)) {
      // 单独打开的文件插到列表最前
      actFiles.value.unshift({
        path: p,
        fileName: (p.split(/[\\/]/).pop() ?? p).replace(/\.fskel$/i, ""),
        frameCount: 0,
        boneCount: 0,
        byteLen: 0,
        actionName: "",
      });
    }
    openActFile(p);
  }
}

async function openActFile(path: string) {
  actLoading.value = true;
  playing.value = false;
  try {
    const meta = await api.loadFskelMeta(path);
    const arr = await api.loadFskelFrames(path, 0, meta.frameCount);
    actMeta.value = meta;
    actPoses.value = Float32Array.from(arr);
    viewer?.setFskel(meta, actPoses.value);
    // 同步左侧列表条目信息
    const brief = actFiles.value.find((f) => f.path === path);
    if (brief && !brief.frameCount) {
      brief.frameCount = meta.frameCount;
      brief.boneCount = meta.boneCount;
    }
    currentAction.value = 0;
    playTime.value = 0;
    playing.value = true;
    viewer?.frameSkeleton();
  } catch (e) {
    message.error(`动作加载失败: ${e}`);
  } finally {
    actLoading.value = false;
  }
}

function selectAction(i: number) {
  currentAction.value = i;
  playTime.value = 0;
}

function scrub(v: number) {
  playTime.value = v;
  applyPlayTime();
}

function applyPlayTime() {
  if (!actPoses.value || !actMeta.value) return;
  const [from, to] = actionRange.value;
  const total = to - from;
  const t = Math.min(Math.max(playTime.value, 0), Math.max(total - 1, 0));
  const abs = from + t;
  const i0 = Math.floor(abs);
  const i1 = Math.min(i0 + 1, actMeta.value.frameCount - 1);
  viewer?.applyFskelFrame(i0, i1, abs - i0);
}

function playTick(ts: number) {
  playRaf = requestAnimationFrame(playTick);
  const dt = Math.min((ts - lastTs) / 1000, 0.1);
  lastTs = ts;
  if (!playing.value || !actMeta.value) return;
  const total = actionFrames.value;
  playTime.value += dt * 30 * speed.value;
  if (playTime.value >= total) playTime.value = 0;
  applyPlayTime();
}

async function exportBvh(currentOnly: boolean) {
  const m = actMeta.value;
  if (!m) return;
  const a = currentActionInfo.value;
  const base = currentOnly && a ? `${a.name}` : m.fileName;
  const p = await save({
    defaultPath: `${base}.bvh`,
    filters: [{ name: "BVH 骨架动画", extensions: ["bvh"] }],
  });
  if (!p) return;
  try {
    const out = await api.exportFskelBvh(m.path, currentOnly ? currentAction.value : null, p);
    message.success(out);
  } catch (e) {
    message.error(`导出失败: ${e}`);
  }
}

async function exportGlb() {
  const m = actMeta.value;
  if (!m) return;
  const p = await save({
    defaultPath: `${m.fileName}.glb`,
    filters: [{ name: "glTF 二进制", extensions: ["glb"] }],
  });
  if (!p) return;
  try {
    const out = await api.exportFskelGlb(m.path, p);
    message.success(out);
  } catch (e) {
    message.error(`导出失败: ${e}`);
  }
}

const selectedModel = computed(() =>
  selectedPart.value ? loaded.get(selectedPart.value) ?? null : null
);

// ---------- 工具 ----------
const shortName = (p: string) => p.split(/[\\/]/).pop() ?? p;

function partInfoOf(g: PartGroup): PartInfo | null {
  const p = groupSel.get(g.signature);
  return g.parts.find((x) => x.path === p) ?? null;
}

function bonesText(p: PartInfo) {
  if (!p.bones.length) return "-";
  const hex = p.bones.map((b) => "0x" + b.toString(16));
  return hex.length > 4 ? `${hex[0]}..${hex[hex.length - 1]} (${hex.length})` : hex.join(",");
}

function texShort(p: PartInfo) {
  return p.textureFile ? shortName(p.textureFile) : "(未找到)";
}

const fmt3 = (a: number[]) => a.map((v) => v.toFixed(1)).join(", ");

function rgbaStr(a: number[]) {
  const c = (v: number) => Math.round(Math.max(0, Math.min(1, v)) * 255);
  return `rgba(${c(a[0])}, ${c(a[1])}, ${c(a[2])}, ${+(a[3] ?? 1).toFixed(2)})`;
}

function setColor(arr: number[], v: string) {
  const nums = (v.match(/[\d.]+g?/g) ?? []).map(Number);
  if (nums.length >= 3) {
    arr[0] = nums[0] / 255;
    arr[1] = nums[1] / 255;
    arr[2] = nums[2] / 255;
    arr[3] = nums.length >= 4 ? nums[3] : 1;
  }
  afterEdit();
  viewer?.applyMaterial(selectedPart.value!);
}

function afterEdit() {
  if (selectedPart.value) dirty.add(selectedPart.value);
}

const isDirty = (p?: string | null) => !!p && dirty.has(p);

// ---------- 纹理 ----------
function resolveTexture(texturePath: string): string | null {
  if (!modelSet.value) return null;
  const stem = (texturePath.split(/[\\/]/).pop() ?? "")
    .replace(/\.[^.]*$/, "")
    .toLowerCase();
  if (!stem) return null;
  return modelSet.value.textures.find((t) => t.stem === stem)?.path ?? null;
}

async function ensureTexture(file: string): Promise<TexData | null> {
  if (texCache.has(file)) return texCache.get(file)!;
  try {
    const dto = await api.loadTexture(file);
    const d = texDataFromDto(dto);
    texCache.set(file, d);
    return d;
  } catch (e) {
    message.error(`纹理解码失败 ${shortName(file)}: ${e}`);
    return null;
  }
}

const texPreviewUrl = computed(() => {
  const m = selectedModel.value;
  if (!m) return null;
  const file = resolveTexture(m.material.texturePath);
  if (!file) return null;
  const d = texCache.get(file);
  if (!d) return null;
  const cv = document.createElement("canvas");
  cv.width = d.width;
  cv.height = d.height;
  const ctx = cv.getContext("2d")!;
  const img = ctx.createImageData(d.width, d.height);
  img.data.set(d.rgba);
  ctx.putImageData(img, 0, 0);
  return cv.toDataURL();
});

const texInfoText = computed(() => {
  const m = selectedModel.value;
  if (!m) return "";
  const file = resolveTexture(m.material.texturePath);
  const d = file ? texCache.get(file) : null;
  return d ? `${d.width}×${d.height} · ${shortName(file!)}` : file ? `${shortName(file)} (未加载)` : "目录中未找到匹配贴图";
});

async function applyTexture() {
  const m = selectedModel.value;
  if (!m || !viewer) return;
  afterEdit();
  const file = resolveTexture(m.material.texturePath);
  if (file) {
    const d = await ensureTexture(file);
    if (d) viewer.refreshTexture(m.path, d, file);
  } else {
    viewer.clearTexture(m.path);
  }
  viewer.applyMaterial(m.path);
}

// ---------- 部件加载 ----------
async function ensureLoaded(path: string): Promise<FskinModel | null> {
  if (loaded.has(path)) return loaded.get(path)!;
  try {
    const m = await api.loadFskin(path);
    loaded.set(path, m);
    return m;
  } catch (e) {
    message.error(`加载失败: ${e}`);
    return null;
  }
}

async function addToViewer(path: string) {
  if (!viewer || viewer.has(path)) return;
  const m = await ensureLoaded(path);
  if (!m) return;
  const file = m.material.texturePath ? resolveTexture(m.material.texturePath) : null;
  const tex = file ? await ensureTexture(file) : null;
  viewer.addPart(path, m, tex, file);
}

function toggleGroup(g: PartGroup, on: boolean) {
  groupVis.set(g.signature, on);
  const p = groupSel.get(g.signature);
  if (!p) return;
  if (on) addToViewer(p);
  else viewer?.removePart(p);
}

async function changeVariant(g: PartGroup, newPath: string) {
  const old = groupSel.get(g.signature);
  if (old) viewer?.removePart(old);
  groupSel.set(g.signature, newPath);
  if (groupVis.get(g.signature)) await addToViewer(newPath);
  if (selectedPart.value === old) selectPart(newPath);
}

function selectPart(path: string | null) {
  selectedPart.value = path;
  viewer?.highlight(path);
  if (path && !loaded.has(path)) ensureLoaded(path);
}

// ---------- 扫描 ----------
async function scan() {
  scanning.value = true;
  try {
    const set = await api.scanModelSet(rootDir.value);
    modelSet.value = set;
    // 清空场景
    for (const p of [...loaded.keys()]) viewer?.removePart(p);
    loaded.clear();
    dirty.clear();
    groupSel.clear();
    groupVis.clear();
    selectedPart.value = null;
    viewer?.highlight(null);

    for (const g of set.groups) {
      const def = g.parts.find((p) => p.inRoot) ?? g.parts[0];
      groupSel.set(g.signature, def.path);
      groupVis.set(g.signature, !!g.parts.find((p) => p.inRoot));
    }
    // 依次加载可见部件（根目录 5 件 = 完整角色）
    for (const g of set.groups) {
      if (groupVis.get(g.signature)) await addToViewer(groupSel.get(g.signature)!);
    }
    viewer?.frameAll();
    message.success(
      `扫描完成: ${set.groups.length} 组 · 已装配 ${set.groups.filter((g) => groupVis.get(g.signature)).length} 个部件`
    );
  } catch (e) {
    message.error(`扫描失败: ${e}`);
  } finally {
    scanning.value = false;
  }
}

async function pickDir() {
  const d = await open({ directory: true, defaultPath: rootDir.value });
  if (d) {
    rootDir.value = d;
    await scan();
  }
}

// ---------- 统计 ----------
const visiblePaths = computed(() => {
  const out: string[] = [];
  if (!modelSet.value) return out;
  for (const g of modelSet.value.groups) {
    if (groupVis.get(g.signature)) {
      const p = groupSel.get(g.signature);
      if (p) out.push(p);
    }
  }
  return out;
});
const visibleCount = computed(() => visiblePaths.value.length);
const statVerts = computed(() => {
  let n = 0;
  for (const p of visiblePaths.value) n += loaded.get(p)?.vertices.length ?? 0;
  return n;
});
const statTris = computed(() => {
  let n = 0;
  for (const p of visiblePaths.value) n += (loaded.get(p)?.triangles.length ?? 0) / 3;
  return n;
});

// ---------- 保存 ----------
async function saveModelTo(path: string, m: FskinModel) {
  try {
    await api.saveFskin(path, m);
    dirty.delete(m.path);
    message.success(`已保存: ${shortName(path)}`);
  } catch (e) {
    message.error(`保存失败: ${e}`);
  }
}

function saveSelected() {
  const m = selectedModel.value;
  if (m) saveModelTo(m.path, m);
}

async function saveAs() {
  const m = selectedModel.value;
  if (!m) return;
  const p = await save({
    defaultPath: shortName(m.path),
    filters: [{ name: "FSKIN", extensions: ["fskin"] }],
  });
  if (p) saveModelTo(p, m);
}

async function recomputeBbox() {
  const m = selectedModel.value;
  if (!m) return;
  try {
    const [mx, mn] = await api.computeBbox(m);
    m.bboxMax = mx;
    m.bboxMin = mn;
    afterEdit();
    message.success("包围盒已按顶点重算");
  } catch (e) {
    message.error(`${e}`);
  }
}

// ---------- 导出 ----------
async function exportObj() {
  if (!visiblePaths.value.length) return;
  const p = await save({
    defaultPath: "avatar_girl.obj",
    filters: [{ name: "OBJ 模型", extensions: ["obj"] }],
  });
  if (!p) return;
  const dir = p.replace(/[\\/][^\\/]*$/, "");
  const base = (p.split(/[\\/]/).pop() ?? "model").replace(/\.obj$/i, "");
  try {
    const out = await api.exportObj(visiblePaths.value, dir, base);
    message.success(`已导出: ${out}（含 mtl 与贴图）`);
  } catch (e) {
    message.error(`导出失败: ${e}`);
  }
}

// ---------- 顶点表 ----------
const numInput = (
  get: (r: any) => number,
  set: (r: any, v: number) => void,
  w = 82
) => ({
  width: w,
  render: (row: any) =>
    h(NInputNumber, {
      size: "tiny",
      showButton: false,
      value: get(row),
      step: 0.05,
      onUpdateValue: (v: number | null) => set(row, v ?? 0),
    }),
});
const touch = (row: any) => {
  const i = selectedModel.value!.vertices.indexOf(row);
  if (i >= 0) viewer?.updateVertex(selectedModel.value!.path, i);
  afterEdit();
};
const vertexColumns = [
  { title: "#", key: "idx", width: 60, render: (r: any) => selectedModel.value!.vertices.indexOf(r) },
  { title: "X", key: "x", ...numInput((r) => r.pos[0], (r, v) => { r.pos[0] = v; touch(r); }) },
  { title: "Y", key: "y", ...numInput((r) => r.pos[1], (r, v) => { r.pos[1] = v; touch(r); }) },
  { title: "Z", key: "z", ...numInput((r) => r.pos[2], (r, v) => { r.pos[2] = v; touch(r); }) },
  { title: "U", key: "u", ...numInput((r) => r.uv[0], (r, v) => { r.uv[0] = v; touch(r); }) },
  { title: "V", key: "v", ...numInput((r) => r.uv[1], (r, v) => { r.uv[1] = v; touch(r); }) },
  {
    title: "法线",
    key: "n",
    width: 130,
    ellipsis: true,
    render: (r: any) => r.normal.map((v: number) => v.toFixed(2)).join(","),
  },
  {
    title: "骨骼",
    key: "b",
    width: 150,
    ellipsis: true,
    render: (r: any) =>
      r.influences.map((i: any) => `0x${i.boneIndex.toString(16)}×${i.weight.toFixed(2)}`).join(" + "),
  },
];

// ---------- 生命周期 ----------
watch(renderMode, (m) => viewer?.setMode(m));
watch(wireframe, (w) => viewer?.setWireframe(w));

onMounted(async () => {
  if (viewportRef.value) {
    viewer = new Viewer(viewportRef.value);
    viewer.setPickHandler((p) => selectPart(p));
  }
  lastTs = performance.now();
  playRaf = requestAnimationFrame(playTick);
  await scan();
});

onUnmounted(() => {
  cancelAnimationFrame(playRaf);
  viewer?.dispose();
  viewer = null;
});
</script>
