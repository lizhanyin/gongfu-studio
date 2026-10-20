<template>
  <div class="act-panel">
    <div class="form-row" title="动作目录下的 .FSKEL 文件">
      <label>动作文件</label>
      <n-select
        size="small"
        style="flex: 1"
        filterable
        :value="act.actMeta?.path"
        placeholder="选择动作文件"
        :options="fileOptions"
        :loading="loading"
        @update:value="(p: string) => act.openActFile(p)"
      />
    </div>

    <template v-if="act.actMeta">
      <div class="section-title">动作分段（{{ act.actMeta.actions.length }}）</div>
      <div class="action-list">
        <div
          v-for="(a, i) in act.actMeta.actions"
          :key="i"
          class="action-row"
          :class="{ active: i === act.currentAction }"
          @click="act.selectAction(i)"
        >
          <span class="name" :title="a.name">{{ a.name || `(分段 ${i + 1})` }}</span>
          <span class="action-meta">
            {{ a.firstFrame }}-{{ a.firstFrame + a.frameCount - 1 }} ·
            {{ (a.frameCount * act.actMeta.frameDt).toFixed(2) }}s
          </span>
        </div>
      </div>

      <n-divider style="margin: 8px 0" />
      <div class="kv">
        <b>{{ act.actMeta.fileName }}</b><br />
        版本 0x{{ act.actMeta.version.toString(16) }} · 校验 0x{{ act.actMeta.checksum.toString(16) }}<br />
        骨骼 {{ act.actMeta.boneCount }} · 帧 {{ act.actMeta.frameCount }} ·
        {{ (1 / act.actMeta.frameDt).toFixed(0) }} FPS<br />
        总时长 {{ (act.actMeta.frameCount * act.actMeta.frameDt).toFixed(2) }}s
        <span v-if="act.actLoading"> · 帧数据加载中…</span>
      </div>
    </template>
    <div v-else class="kv" style="padding: 4px 0">选择动作文件后开始播放。</div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { ActStore } from "../composables/useActMode";
import { api } from "../api";

const props = defineProps<{ act: ActStore; actDir: string }>();

const files = ref<{ label: string; value: string }[]>([]);
const loading = ref(false);

async function loadFiles() {
  loading.value = true;
  try {
    const list = await api.listResDir(props.actDir);
    files.value = list
      .filter((e) => e.kind === "fskel")
      .map((e) => ({
        label: `${e.name.replace(/\.fskel$/i, "")} (${e.frameCount ?? "?"} 帧)`,
        value: e.path,
      }));
  } catch {
    files.value = [];
  } finally {
    loading.value = false;
  }
}

watch(() => props.actDir, loadFiles, { immediate: true });

const fileOptions = computed(() => files.value);
</script>
