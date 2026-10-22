<template>
  <div v-if="!model" class="kv" style="padding: 12px">
    点击左侧部件组或视口中的网格以编辑。<br /><br />
    支持编辑: 材质颜色 / 高光 / 纹理路径 / 顶点坐标与 UV / 包围盒重算。<br />
    保存采用字节级往返验证过的序列化器。
  </div>
  <template v-else>
    <div class="kv" style="margin-bottom: 6px">
      <b>{{ shortName(model.path) }}</b>
      <span v-if="skin.isDirty(model.path)" class="dirty-dot"> ● 已修改</span>
    </div>
    <n-tabs type="line" size="small" default-value="material">
      <n-tab-pane name="material" tab="材质">
        <template v-if="!model.materials.length">
          <div class="kv">本网格无材质槽（特效网格）。</div>
        </template>
        <div v-for="(mat, mi) in model.materials" :key="mi" class="mat-block">
          <div v-if="model.materials.length > 1" class="kv" style="margin-bottom: 4px">
            <b>子网格 {{ mi }}</b> · {{ mat.texturePath }}
          </div>
          <div class="form-row">
            <label>漫反射</label>
            <n-color-picker
              style="flex: 1"
              :modes="['rgb']"
              :show-alpha="true"
              :value="rgbaStr(mat.diffuse)"
              @update:value="(v: string) => skin.setColor(mat.diffuse, v)"
            />
          </div>
          <div class="form-row">
            <label>环境光</label>
            <n-color-picker
              style="flex: 1"
              :modes="['rgb']"
              :show-alpha="true"
              :value="rgbaStr(mat.ambient)"
              @update:value="(v: string) => skin.setColor(mat.ambient, v)"
            />
          </div>
          <div class="form-row">
            <label>自发光</label>
            <n-color-picker
              style="flex: 1"
              :modes="['rgb']"
              :show-alpha="true"
              :value="rgbaStr(mat.emissive)"
              @update:value="(v: string) => skin.setColor(mat.emissive, v)"
            />
          </div>
          <div class="form-row">
            <label>高光色</label>
            <n-color-picker
              style="flex: 1"
              :modes="['rgb']"
              :show-alpha="true"
              :value="rgbaStr(mat.specular)"
              @update:value="(v: string) => skin.setColor(mat.specular, v)"
            />
          </div>
          <div class="form-row">
            <label>高光强度</label>
            <n-input-number
              size="small"
              style="flex: 1"
              :value="mat.specPower"
              @update:value="(v: number | null) => { mat.specPower = v ?? 0; skin.afterEdit(); }"
            />
          </div>
          <div class="form-row">
            <label>纹理路径</label>
            <n-input
              size="small"
              style="flex: 1"
              :value="mat.texturePath"
              @update:value="(v: string) => { mat.texturePath = v; }"
            />
          </div>
        </div>
        <div class="apply-row">
          <n-button size="small" @click="skin.applyTexture">应用纹理</n-button>
          <span class="apply-hint">按文件名匹配目录内贴图</span>
        </div>
        <div v-if="skin.texPreviewUrl" style="margin-top: 6px">
          <img class="tex-preview" :src="skin.texPreviewUrl" style="width: 100%" />
          <div class="kv" style="margin-top: 4px">
            {{ skin.texInfoText }}
          </div>
        </div>
      </n-tab-pane>

      <n-tab-pane name="vertices" tab="顶点">
        <n-data-table
          size="small"
          :columns="vertexColumns"
          :data="model.vertices"
          :pagination="{ pageSize: 25 }"
          :max-height="520"
          virtual-scroll
        />
      </n-tab-pane>

      <n-tab-pane name="file" tab="文件">
        <div class="kv">
          <b>路径:</b> {{ model.path }}<br />
          <b>版本:</b> 0x{{ model.version.toString(16) }}<br />
          <b>顶点:</b> {{ model.vertices.length }} ·
          <b>三角形:</b> {{ triCount }}（{{ model.submeshes.length }} 子网格）<br />
          <b>包围盒 max:</b> {{ fmt3(model.bboxMax) }}<br />
          <b>包围盒 min:</b> {{ fmt3(model.bboxMin) }}<br />
          <template v-if="model.submeshes[0]">
            <b>节0附加:</b> {{ model.submeshes[0].misc.map((v: number) => (v >>> 0).toString(16)).join(" ") }}
          </template>
        </div>
        <n-space style="margin-top: 10px">
          <n-button size="small" @click="skin.recomputeBbox">按顶点重算包围盒</n-button>
          <n-button size="small" type="warning" @click="skin.saveSelected">保存</n-button>
          <n-button size="small" @click="skin.saveAs">另存为</n-button>
        </n-space>
      </n-tab-pane>
    </n-tabs>
  </template>
</template>

<script setup lang="ts">
import { computed, h } from "vue";
import { NInputNumber } from "naive-ui";
import type { SkinStore } from "../composables/useSkinMode";
import { shortName } from "../utils";

const props = defineProps<{ skin: SkinStore }>();

const model = computed(() => props.skin.selectedModel);

const triCount = computed(() =>
  model.value ? model.value.submeshes.reduce((s, x) => s + x.triangles.length / 3, 0) : 0
);

const fmt3 = (a: number[]) => a.map((v) => v.toFixed(1)).join(", ");

function rgbaStr(a: number[]) {
  const c = (v: number) => Math.round(Math.max(0, Math.min(1, v)) * 255);
  return `rgba(${c(a[0])}, ${c(a[1])}, ${c(a[2])}, ${+(a[3] ?? 1).toFixed(2)})`;
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
const touch = (row: any) => props.skin.touchVertex(row);
const vertexColumns = [
  { title: "#", key: "idx", width: 60, render: (r: any) => model.value!.vertices.indexOf(r) },
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
</script>

<style scoped>
.mat-block + .mat-block {
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px dashed rgba(128, 128, 128, 0.35);
}
</style>
