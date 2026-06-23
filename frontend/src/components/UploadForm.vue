<template>
  <div class="card">
    <h2>上传新视频</h2>

    <div v-if="error" class="error">{{ error }}</div>
    <p v-if="message" class="message">{{ message }}</p>

    <div class="form-row">
      <label>视频文件</label>
      <div style="display: flex; gap: 0.5rem; align-items: center">
        <button type="button" @click="onPickFile" :disabled="busy">
          {{ pickedPath ? "重新选择" : "选择文件" }}
        </button>
        <span v-if="pickedPath" class="message" style="margin: 0">
          {{ pickedName }} ({{ formatBytes(pickedSize) }})
        </span>
        <span v-else class="message" style="margin: 0">未选择</span>
      </div>
    </div>

    <div class="form-row">
      <label>预设</label>
      <select v-model="presetId" :disabled="busy">
        <option v-for="p in presets" :key="p.id" :value="p.id">
          {{ p.name }} — {{ p.resolution }} ({{ p.bitrate_hint }})
        </option>
      </select>
    </div>

    <div class="form-row">
      <label>输出格式</label>
      <div style="display: flex; gap: 1rem">
        <label style="display: flex; align-items: center; gap: 0.25rem">
          <input type="radio" v-model="target" value="mp4" :disabled="busy" />
          MP4
        </label>
        <label style="display: flex; align-items: center; gap: 0.25rem">
          <input type="radio" v-model="target" value="hls" :disabled="busy" />
          HLS (m3u8)
        </label>
      </div>
    </div>

    <button
      type="button"
      @click="onSubmit"
      :disabled="!pickedPath || busy"
    >
      {{ busy ? "处理中…" : "开始转码" }}
    </button>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useRouter } from "vue-router";
import {
  pickVideoFile,
  getPresets,
  createUpload,
  createJob,
  processNext,
  type Preset,
} from "../api";

const emit = defineEmits<{ uploaded: [] }>();

const router = useRouter();
const presets = ref<Preset[]>([]);
const presetId = ref("blog");
const target = ref<"mp4" | "hls">("mp4");

const pickedPath = ref<string | null>(null);
const pickedName = ref<string | null>(null);
const pickedSize = ref<number>(0);
const busy = ref(false);
const error = ref<string | null>(null);
const message = ref<string | null>(null);

onMounted(async () => {
  try {
    presets.value = await getPresets();
  } catch (e) {
    error.value = `加载预设失败:${e}`;
  }
});

function basename(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i >= 0 ? p.slice(i + 1) : p;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

async function onPickFile() {
  error.value = null;
  try {
    const path = await pickVideoFile();
    if (path) {
      pickedPath.value = path;
      pickedName.value = basename(path);
      // 没法直接 stat,让后端在 create_upload 时拿真实大小
    }
  } catch (e) {
    error.value = `选择文件失败:${e}`;
  }
}

async function onSubmit() {
  if (!pickedPath.value) return;
  busy.value = true;
  error.value = null;
  message.value = "上传中…";
  try {
    const upload = await createUpload(pickedName.value ?? "video", pickedPath.value);
    pickedSize.value = upload.length;
    message.value = "转码任务已提交,等待处理…";
    const job = await createJob(upload.id, presetId.value, target.value);
    // 触发 worker 立即处理
    await processNext().catch(() => {});
    emit("uploaded");
    router.push({ name: "job", params: { id: job.id } });
  } catch (e) {
    error.value = `提交失败:${e}`;
  } finally {
    busy.value = false;
  }
}
</script>
