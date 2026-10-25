<!-- 材质贴图面板（FAK / FSM 检查器共用）：每材质显示引用路径、当前绑定文件、
     预览缩略图，并提供同目录贴图下拉热替换（不改文件，仅视口内换图）。 -->
<template>
  <div class="mat-tex">
    <div v-for="(mat, mi) in materials" :key="mi" class="mat-block">
      <div class="kv mat-head">
        <b>材质 {{ mi }}</b>
        <span class="tex-ref" :title="mat.texturePath">
          引用 {{ textureName(mat.texturePath) || "—" }}
        </span>
      </div>
      <div class="kv" :class="{ warn: !matFiles[mi] }">
        <template v-if="matFiles[mi]">已绑定 {{ shortName(matFiles[mi]!) }}</template>
        <template v-else>未找到贴图，可从下方手动指定</template>
      </div>
      <n-select
        size="small"
        filterable
        :value="selectedValue(mi)"
        :options="options"
        placeholder="选择贴图替换"
        @update:value="(v: string) => emit('setTexture', mi, v)"
      />
      <img v-if="previews[mi]" class="tex-preview" :src="previews[mi]" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import { shortName } from "../utils";

const props = defineProps<{
  /** 模型材质列表（只取 texturePath 展示） */
  materials: { texturePath: string }[];
  /** 各材质当前绑定的贴图文件（null=未找到） */
  matFiles: (string | null)[];
  /** 同目录可选贴图（下拉选项） */
  dirTexFiles: { name: string; path: string }[];
  /** 预览缩略图加载器（dataURL） */
  loadPreview: (file: string) => Promise<string | null>;
}>();

const emit = defineEmits<{ setTexture: [matIdx: number, file: string] }>();

const options = computed(() =>
  props.dirTexFiles.map((t) => ({ label: t.name, value: t.path }))
);

/** 绑定文件 → 下拉选中值：按归一化路径匹配选项（绑定可能来自路径优先解析，
 *  分隔符与目录列举不一致），匹配不到则原样显示 */
function selectedValue(mi: number): string {
  const f = props.matFiles[mi];
  if (!f) return "";
  const norm = (p: string) => p.replace(/[\\/]+/g, "/").toLowerCase();
  return props.dirTexFiles.find((t) => norm(t.path) === norm(f))?.path ?? f;
}

/** 每材质预览缩略图（换绑后自动刷新；异步竞态按材质序防串台） */
const previews = reactive<Record<number, string | null>>({});
watch(
  () => props.matFiles.map((f) => f).join("\n"),
  () => {
    props.matFiles.forEach((f, mi) => {
      if (!f) {
        previews[mi] = null;
        return;
      }
      props.loadPreview(f).then((url) => {
        if (props.matFiles[mi] === f) previews[mi] = url;
      });
    });
  },
  { immediate: true }
);

function textureName(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}
</script>

<style scoped>
.mat-block + .mat-block {
  margin-top: 10px;
  padding-top: 8px;
  border-top: 1px dashed rgba(128, 128, 128, 0.35);
}
.mat-head {
  display: flex;
  justify-content: space-between;
  gap: 8px;
}
.tex-ref {
  color: #889;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tex-preview {
  margin-top: 6px;
}
.kv.warn {
  color: #d9925f;
}
</style>
