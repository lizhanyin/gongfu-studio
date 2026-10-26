<template>
  <div class="kv sfx-root" v-if="sfx.sfxModel">
    <div class="sfx-head kv">
      <b>{{ fileName }}</b> · 根平移 {{ transText }}<br />
      节点 {{ sfx.sfxModel.nodes.length }} · 类型 {{ kindCount }} · 贴图
      {{ sfx.uniqueTextures.length }} · 时长 {{ ((sfx.actionFrames - 1) / sfx.fps).toFixed(2) }}s
    </div>
    <div class="sfx-ctl">
      <span class="ctl-lbl">帧率</span>
      <n-select
        size="tiny"
        style="width: 70px"
        :value="sfx.fps"
        :options="fpsOpts"
        @update:value="(v: number) => (sfx.fps = v)"
      />
      <span class="ctl-lbl">叠加</span>
      <n-switch size="small" :value="additive" @update:value="onBlending" />
      <div class="ctl-spacer"></div>
      <n-button
        size="tiny"
        :type="sfx.dirty ? 'primary' : 'default'"
        :disabled="!sfx.dirty || isCpk"
        :title="saveTitle"
        @click="sfx.saveSfx()"
      >
        保存{{ sfx.dirty ? "*" : "" }}
      </n-button>
    </div>
    <template v-if="sfx.sfxLoading">加载中…</template>
    <template v-else>
      <!-- 节点列表：限高滚动，不再把下方预览顶走 -->
      <div class="sfx-nodelist">
        <div class="sfx-node" v-for="(n, i) in sfx.sfxModel.nodes" :key="i">
          <div class="action-row">
            <span class="sfx-kind">#{{ i }} {{ kindLabel(n.kind) }}</span>
            <span class="action-meta">{{ n.name || "-" }}</span>
          </div>
          <div v-if="n.textures.length" class="sfx-refs">
            <img
              v-for="(t, ri) in n.textures"
              :key="ri"
              class="sfx-thumb"
              :class="{ active: isActive(i, ri) }"
              :src="sfx.texSlots.get(t)?.url ?? undefined"
              :title="t"
              @click="togglePreview(i, ri)"
            />
          </div>
          <div v-if="n.models.length" class="sfx-refs">
            <span v-for="m in n.models" :key="m" class="sfx-ref">网格 {{ shortName(m) }}</span>
          </div>
          <div v-if="n.sounds.length" class="sfx-refs">
            <button
              v-for="s in n.sounds"
              :key="s"
              class="sfx-ref sfx-sound"
              :class="{ playing: playingSound === s }"
              :title="s"
              @click="playSound(s)"
            >
              {{ playingSound === s ? "⏹" : "▶" }} {{ shortName(s) }}
            </button>
          </div>
        </div>
      </div>
      <!-- 贴图放大预览 + 路径改写：吸底固定高度，列表滚动时始终可见 -->
      <div class="sfx-preview">
        <template v-if="previewRef">
          <div class="sfx-preview-bar">
            <span class="sfx-preview-name" :title="previewRef">{{ shortName(previewRef) }}</span>
            <span class="sfx-bind" :class="{ miss: !previewSlot?.file }">
              {{
                previewSlot?.file
                  ? `已绑定 ${shortName(previewSlot.file)}`
                  : "未找到贴图（可改 .dds 路径）"
              }}
            </span>
            <n-button size="tiny" @click="preview = null">收起</n-button>
          </div>
          <img class="sfx-preview-img" :src="previewSlot?.url ?? undefined" />
          <div class="sfx-pathrow">
            <n-select
              size="tiny"
              filterable
              :value="null"
              placeholder="同目录贴图…"
              :options="pathOptions"
              :loading="optsLoading"
              style="width: 45%"
              @update:value="applyPath"
            />
            <n-input
              size="tiny"
              v-model:value="customPath"
              placeholder="自定义路径（不长于原路径）"
              @keyup.enter="applyCustom"
            />
            <n-button size="tiny" @click="applyCustom">应用</n-button>
          </div>
        </template>
        <div v-else class="sfx-preview-empty">点击贴图缩略图：放大预览 / 修改路径</div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { api } from "../api";
import { TEX_KINDS } from "../tex";
import type { SfxStore } from "../composables/useSfxMode";
import { shortName } from "../utils";
import { message } from "../message";

const props = defineProps<{ sfx: SfxStore }>();

/** 当前放大的贴图（节点序 + 节点内贴图槽序：同路径可在多槽重复出现） */
const preview = ref<{ nodeIdx: number; refIdx: number } | null>(null);

const fileName = computed(() => props.sfx.sfxModel?.path.split(/[\\/]/).pop() ?? "");
const kindCount = computed(
  () => new Set(props.sfx.sfxModel?.nodes.map((n) => n.kind)).size
);
const transText = computed(() => {
  const m = props.sfx.sfxModel;
  if (!m) return "";
  return [12, 13, 14].map((i) => Math.round(m.matrix[i])).join(", ");
});
const isCpk = computed(() => /^cpk:/i.test(props.sfx.sfxModel?.path ?? ""));
const saveTitle = computed(() =>
  isCpk.value ? "CPK 包内文件不可写（先导出到磁盘再改）" : "保存贴图路径改写"
);

/** 11 种节点类型的中文标注（未知类型原样展示） */
const KIND_LABELS: Record<string, string> = {
  EffectQuad: "公告板",
  EffectParticleEmitter: "粒子",
  EffectCross: "交叉面",
  EffectCylinder: "柱面",
  EffectBeam: "光束",
  EffectGrid: "面片",
  EffectMesh: "网格",
  EffectSphere: "球面",
  EffectMsg: "飘字",
  EffectExplosive: "爆炸",
  EffectSound: "音效",
};
function kindLabel(kind: string): string {
  return `${KIND_LABELS[kind] ?? kind}`;
}

// ---------------- 播放控制 ----------------

const fpsOpts = [6, 10, 12, 15, 24, 30].map((v) => ({ label: `${v}`, value: v }));
/** 叠加混合（发光特效默认开）；换文件时随 viewer 重建回默认 */
const additive = ref(true);
function onBlending(v: boolean) {
  additive.value = v;
  props.sfx.setBlending(v);
}
watch(
  () => props.sfx.sfxModel,
  (m, prev) => {
    if (m?.path !== prev?.path) {
      additive.value = true;
      preview.value = null;
      stopSound();
    }
  }
);

// ---------------- 缩略图放大与路径改写 ----------------

function isActive(nodeIdx: number, refIdx: number) {
  return preview.value?.nodeIdx === nodeIdx && preview.value?.refIdx === refIdx;
}
function togglePreview(nodeIdx: number, refIdx: number) {
  preview.value = isActive(nodeIdx, refIdx) ? null : { nodeIdx, refIdx };
}

const previewRef = computed(() => {
  const p = preview.value;
  if (!p) return null;
  return props.sfx.sfxModel?.nodes[p.nodeIdx]?.textures[p.refIdx] ?? null;
});
const previewSlot = computed(() =>
  previewRef.value ? props.sfx.texSlots.get(previewRef.value) ?? null : null
);

const customPath = ref("");
watch(previewRef, (r) => (customPath.value = r ?? ""));

/** 同目录可选贴图（引用式路径：引用目录 + 文件名，写入 raw 的口径） */
const pathOptions = ref<{ label: string; value: string }[]>([]);
const optsLoading = ref(false);
watch(
  previewRef,
  async (ref) => {
    pathOptions.value = [];
    if (!ref || !props.sfx.sfxModel) return;
    optsLoading.value = true;
    const model = props.sfx.sfxModel;
    const refDir = ref.replace(/[\\/][^\\/]*$/, "");
    const refName = ref.split(/[\\/]/).pop() ?? "";
    // 模型路径祖先逐级拼接引用目录（CPK 根即命中；同磁盘目录亦然）
    const segs = model.path.split(/[\\/]/);
    for (let i = segs.length - 1; i >= 1 && !pathOptions.value.length; i--) {
      const dir = segs.slice(0, i).join("\\") + "\\" + refDir;
      try {
        const entries = await api.listResDir(dir);
        pathOptions.value = entries
          .filter((e) => TEX_KINDS.test(e.kind))
          .map((e) => ({ label: e.name, value: `${refDir}\\${e.name}` }));
      } catch {
        // 该祖先下无此目录，试上一级
      }
    }
    // 当前引用不在目录清单时（如解析命中的 .dds 重试名）补进选项
    if (!pathOptions.value.some((o) => o.value === ref)) {
      pathOptions.value.unshift({ label: refName, value: ref });
    }
    optsLoading.value = false;
  },
  { immediate: true }
);

function applyPath(v: string) {
  const p = preview.value;
  if (p) props.sfx.setNodeTexture(p.nodeIdx, p.refIdx, v);
}
function applyCustom() {
  if (customPath.value.trim()) applyPath(customPath.value.trim());
}

// ---------------- 音效试听（EffectSound 的 sound\…\*.wav） ----------------

const playingSound = ref<string | null>(null);
let audioEl: HTMLAudioElement | null = null;
function stopSound() {
  audioEl?.pause();
  audioEl = null;
  playingSound.value = null;
}
async function playSound(ref: string) {
  if (playingSound.value === ref) {
    stopSound();
    return;
  }
  const model = props.sfx.sfxModel;
  if (!model) return;
  const norm = ref.replace(/[\\/]+/g, "\\");
  const segs = model.path.split(/[\\/]/);
  for (let i = segs.length - 1; i >= 1; i--) {
    try {
      const b64 = await api.readFileB64(segs.slice(0, i).join("\\") + "\\" + norm);
      stopSound();
      audioEl = new Audio(`data:audio/wav;base64,${b64}`);
      playingSound.value = ref;
      audioEl.onended = () => {
        if (playingSound.value === ref) playingSound.value = null;
      };
      void audioEl.play();
      return;
    } catch {
      // 该祖先下无此音效，试上一级
    }
  }
  message.error(`音效未找到: ${shortName(ref)}`);
}
onUnmounted(stopSound);
</script>

<style scoped>
.sfx-root {
  padding: 12px;
}
.sfx-head {
  padding-bottom: 6px;
}
.sfx-ctl {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-bottom: 6px;
}
.ctl-lbl {
  font-size: 12px;
  color: #99a;
}
.ctl-spacer {
  flex: 1;
}
.sfx-node {
  padding: 2px 0;
}
.sfx-node .action-row {
  margin-bottom: 2px;
}
.sfx-kind {
  font-weight: 600;
}
.sfx-refs {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding-left: 8px;
}
.sfx-thumb {
  width: 44px;
  height: 44px;
  object-fit: contain;
  image-rendering: pixelated;
  background: #000;
  border: 1px solid #333;
  border-radius: 3px;
  cursor: pointer;
}
.sfx-thumb:hover {
  border-color: #63e2b7;
}
.sfx-thumb.active {
  border-color: #63e2b7;
  box-shadow: 0 0 0 1px #63e2b7;
}
.sfx-ref {
  font-size: 12px;
  color: #aaa;
  background: #22242a;
  border-radius: 3px;
  padding: 1px 6px;
}
.sfx-sound {
  cursor: pointer;
  border: 0;
}
.sfx-sound:hover {
  color: #dde;
  background: #2c2f38;
}
.sfx-sound.playing {
  color: #63e2b7;
}
/* 节点列表：限高滚动 */
.sfx-nodelist {
  max-height: min(320px, 42vh);
  overflow-y: auto;
  padding-right: 2px;
}
/* 预览：吸底固定高度，随列表滚动保持可见 */
.sfx-preview {
  position: sticky;
  bottom: 0;
  margin-top: 6px;
  padding-top: 6px;
  height: 176px;
  background: rgba(24, 24, 32, 0.96);
  border-top: 1px solid #2a2c36;
  z-index: 1;
}
.sfx-preview-bar {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: #ccc;
  margin-bottom: 4px;
}
.sfx-preview-name {
  flex-shrink: 0;
  max-width: 40%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sfx-bind {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  color: #889;
}
.sfx-bind.miss {
  color: #d9925f;
}
.sfx-preview-img {
  width: 100%;
  height: 104px;
  object-fit: contain;
  background: #000;
  border: 1px solid #333;
  border-radius: 3px;
}
.sfx-pathrow {
  display: flex;
  gap: 4px;
  margin-top: 4px;
}
.sfx-preview-empty {
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #556;
  font-size: 12px;
  border: 1px dashed #333;
  border-radius: 3px;
}
</style>
