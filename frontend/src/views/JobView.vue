<template>
  <div>
    <p>
      <router-link to="/">← 返回任务列表</router-link>
    </p>

    <div v-if="error" class="error">{{ error }}</div>

    <div v-if="job" class="card">
      <h2>任务详情</h2>
      <table>
        <tbody>
          <tr>
            <th>任务 ID</th>
            <td><code>{{ job.id }}</code></td>
          </tr>
          <tr v-if="job.upload_filename">
            <th>源文件</th>
            <td><code>{{ job.upload_filename }}</code></td>
          </tr>
          <tr>
            <th>预设</th>
            <td>
              <strong>{{ job.preset_label }}</strong>
              <div class="muted">{{ job.preset_summary }}</div>
            </td>
          </tr>
          <tr>
            <th>输出目标</th>
            <td>
              <strong>{{ job.target_label }}</strong>
              <span class="muted">({{ job.target }})</span>
            </td>
          </tr>
          <tr>
            <th>状态</th>
            <td><StatusBadge :status="job.status" /></td>
          </tr>
          <tr v-if="job.error_summary">
            <th>错误</th>
            <td class="error" style="margin: 0">{{ job.error_summary }}</td>
          </tr>
          <tr v-if="job.artifact_id">
            <th>产物 ID</th>
            <td><code>{{ job.artifact_id }}</code></td>
          </tr>
        </tbody>
      </table>

      <!-- 实时转码进度(替代轮询 status) -->
      <div
        v-if="job.status === 'processing' || job.status === 'queued'"
        class="progress-block"
      >
        <div v-if="lastProgress" class="progress-info">
          <div class="progress-stat">
            <span class="label">已编码</span>
            <span class="value">
              {{ formatTime(lastProgress.encoded_secs) }}
              <span
                v-if="job.source_duration_secs"
                class="muted"
              >/ {{ formatTime(job.source_duration_secs) }}</span>
            </span>
          </div>
          <div class="progress-stat">
            <span class="label">速度</span>
            <span class="value">{{ lastProgress.speed.toFixed(2) }}x</span>
          </div>
          <div class="progress-stat">
            <span class="label">剩余</span>
            <span class="value">{{ etaDisplay }}</span>
          </div>
        </div>

        <!-- 进度条:有 percent 时显示,否则降级为 indeterminate 动画 -->
        <div
          v-if="lastProgress && lastProgress.percent !== null"
          class="progress-bar"
          role="progressbar"
          :aria-valuenow="Math.round(lastProgress.percent!)"
          aria-valuemin="0"
          aria-valuemax="100"
        >
          <div
            class="progress-bar-fill"
            :style="{ width: lastProgress.percent + '%' }"
          />
          <span class="progress-bar-label">
            {{ lastProgress.percent.toFixed(1) }}%
          </span>
        </div>
        <div v-else-if="lastProgress" class="progress-bar indeterminate">
          <div class="progress-bar-fill" />
        </div>

        <p v-if="!lastProgress" class="message">
          {{ job.status === "queued" ? "等待 worker 拉取…" : "等待 ffmpeg 启动…" }}
        </p>
      </div>

      <div v-if="job.artifact_id" style="margin-top: 1rem; display: flex; gap: 0.5rem; flex-wrap: wrap">
        <button
          v-if="job.status === 'completed' && downloadPath"
          @click="onDownload"
          :disabled="downloading"
          class="primary"
        >
          {{ downloading ? "保存中…" : "下载到本地" }}
        </button>
        <button v-if="job.status === 'completed'" @click="onReveal" class="secondary">
          在 Finder 中显示
        </button>
        <button v-if="job.status === 'failed'" @click="onRetry">重试</button>
        <button @click="onDelete" class="danger">删除</button>
      </div>

      <div
        v-if="job.status === 'completed' && job.artifact_id && job.target === 'mp4'"
        style="margin-top: 1rem"
      >
        <video
          v-if="previewUrl"
          :src="previewUrl"
          controls
          style="width: 100%; max-height: 480px"
        />
        <p v-else class="message">预览加载中…</p>
      </div>

      <div
        v-else-if="job.status === 'completed' && job.artifact_id && job.target === 'hls'"
        style="margin-top: 1rem"
      >
        <p class="message">
          HLS 输出已打包为 zip,点击"在 Finder 中显示"查看。
        </p>
      </div>
    </div>

    <p v-else-if="!error" class="message">加载中…</p>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import StatusBadge from "../components/StatusBadge.vue";
import {
  save as saveDialog,
  ask as askDialog,
  message as messageDialog,
} from "@tauri-apps/plugin-dialog";
import {
  getJob,
  retryJob,
  deleteJob,
  getArtifactPreviewPath,
  getArtifactDownloadPath,
  downloadArtifact,
  revealInFinder,
  onTranscodeProgress,
  type JobInfo,
  type TranscodeProgressEvent,
} from "../api";

const route = useRoute();
const router = useRouter();
const job = ref<JobInfo | null>(null);
const previewPath = ref<string | null>(null);
const downloadPath = ref<string | null>(null);
const lastProgress = ref<TranscodeProgressEvent | null>(null);
const error = ref<string | null>(null);
let pollTimer: number | null = null;
let unlistenProgress: (() => void) | null = null;

const id = computed(() => String(route.params.id));

const previewUrl = computed(() =>
  previewPath.value ? convertFileSrc(previewPath.value) : null,
);
// 下载文件名:用 job.preset + job.target 拼一个易读的名字
const downloadFilename = computed(() => {
  if (!job.value) return undefined;
  const ext = job.value.target === "mp4" ? "mp4" : "zip";
  return `rustvid-${job.value.preset}-${id.value.slice(0, 8)}.${ext}`;
});

function formatTime(secs: number): string {
  if (!isFinite(secs) || secs < 0) return "--:--";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
}

/// 预估剩余时间(秒);`null` = 无法估算(总时长未知 / 速度未稳定 / 已完成)。
/// 公式:`(source_duration - encoded) / speed`
const etaSeconds = computed<number | null>(() => {
  const p = lastProgress.value;
  const total = job.value?.source_duration_secs;
  if (!p || total == null || total <= 0) return null;
  // ffmpeg 启动瞬时或开 seek 时 speed 会是 0/很小,这时 ETA 抖动极大,等几秒再算
  if (p.speed <= 0.01) return null;
  const remaining = total - p.encoded_secs;
  if (remaining <= 0) return 0; // 已完成或超额(罕见),显示 0 而不是负数
  return remaining / p.speed;
});

const etaDisplay = computed<string>(() => {
  const e = etaSeconds.value;
  if (e == null) return "计算中…";
  if (e === 0) return "即将完成";
  return formatEta(e);
});

function formatEta(secs: number): string {
  // < 60s 显示秒;否则显示 分·秒 或 时·分
  if (secs < 60) return `${Math.round(secs)} 秒`;
  if (secs < 3600) {
    const m = Math.floor(secs / 60);
    const s = Math.round(secs % 60);
    return s === 0 ? `${m} 分` : `${m} 分 ${s} 秒`;
  }
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return m === 0 ? `${h} 时` : `${h} 时 ${m} 分`;
}

async function refresh() {
  try {
    const result = await getJob(id.value);
    if (result == null) {
      error.value = "任务不存在";
      return;
    }
    job.value = result;
    if (result.status === "completed" && result.artifact_id) {
      // 同时拉 preview 和 download path,前端按需用
      if (result.target === "mp4" && !previewPath.value) {
        try {
          previewPath.value = await getArtifactPreviewPath(result.artifact_id);
        } catch (e) {
          error.value = `加载预览路径失败:${e}`;
        }
      }
      if (!downloadPath.value) {
        try {
          downloadPath.value = await getArtifactDownloadPath(result.artifact_id);
        } catch (e) {
          error.value = `加载下载路径失败:${e}`;
        }
      }
    }
    // 任务结束,清掉进度
    if (result.status === "completed" || result.status === "failed") {
      lastProgress.value = null;
    }
  } catch (e) {
    error.value = `加载失败:${e}`;
  }
}

async function onReveal() {
  if (!job.value?.artifact_id) return;
  try {
    const path = await getArtifactPreviewPath(job.value.artifact_id);
    await revealInFinder(path);
  } catch (e) {
    error.value = `显示文件失败:${e}`;
  }
}

const downloading = ref(false);

async function onDownload() {
  if (!job.value?.artifact_id || !downloadPath.value) return;
  try {
    const ext = downloadFilename.value?.split(".").pop() ?? "bin";
    const dest = await saveDialog({
      defaultPath: downloadFilename.value ?? `rustvid.${ext}`,
      filters: [{ name: "Video", extensions: [ext] }],
    });
    if (!dest) return; // 用户取消
    downloading.value = true;
    const finalPath = await downloadArtifact(job.value.artifact_id, dest);
    await messageDialog(`已保存到:\n${finalPath}`, {
      title: "下载完成",
      kind: "info",
    });
  } catch (e) {
    await messageDialog(`下载失败:${e}`, {
      title: "下载失败",
      kind: "error",
    });
  } finally {
    downloading.value = false;
  }
}

async function onRetry() {
  try {
    job.value = await retryJob(id.value);
    lastProgress.value = null;
  } catch (e) {
    error.value = `重试失败:${e}`;
  }
}

async function onDelete() {
  // Tauri 2 默认禁用 window.confirm(会阻塞主线程),改用原生 dialog 的 ask()
  const confirmed = await askDialog(
    `确认删除这个任务?\n\n会同时删除:\n  • 源上传文件\n  • 转码产物(MP4 / HLS zip)\n  • 数据库记录\n\n操作不可撤销。`,
    {
      title: "删除任务",
      kind: "warning",
      okLabel: "删除",
      cancelLabel: "取消",
    },
  );
  if (!confirmed) return;
  try {
    await deleteJob(id.value);
    router.push({ name: "home" });
  } catch (e) {
    error.value = `删除失败:${e}`;
  }
}

onMounted(async () => {
  await refresh();

  // 订阅进度事件(只处理当前 job)
  unlistenProgress = await onTranscodeProgress((event) => {
    if (event.job_id === id.value) {
      lastProgress.value = event;
    }
  });

  // 轮询作为兜底(防止事件丢失或后端没装 AppHandle)
  pollTimer = window.setInterval(refresh, 3000);
});

onUnmounted(() => {
  if (pollTimer != null) clearInterval(pollTimer);
  if (unlistenProgress) unlistenProgress();
});
</script>

<style scoped>
.btn-download {
  display: inline-block;
  padding: 0.5rem 0.75rem;
  background: var(--accent);
  color: white;
  border: 1px solid var(--accent);
  border-radius: 6px;
  font-weight: 500;
  text-decoration: none;
  cursor: pointer;
}
.btn-download:hover {
  opacity: 0.9;
  text-decoration: none;
}
</style>

<style scoped>
.progress-block {
  margin-top: 1rem;
  padding: 1rem;
  background: var(--bg);
  border-radius: 6px;
  border: 1px solid var(--border);
}
.progress-info {
  display: flex;
  gap: 2rem;
  margin-bottom: 0.75rem;
}
.progress-stat {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.progress-stat .label {
  font-size: 0.75rem;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}
.progress-stat .value {
  font-size: 1.25rem;
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
.progress-stat .value .muted {
  color: var(--muted);
  font-weight: 400;
  font-size: 0.95rem;
  margin-left: 0.25rem;
}

.progress-bar {
  position: relative;
  height: 24px;
  background: var(--card);
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
}
.progress-bar-fill {
  height: 100%;
  background: linear-gradient(
    90deg,
    var(--accent) 0%,
    color-mix(in srgb, var(--accent) 70%, white) 100%
  );
  transition: width 0.4s ease-out;
  border-radius: 12px;
}
.progress-bar-label {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--fg);
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.02em;
}

/* 不知道百分比时的 indeterminate 动画 */
.progress-bar.indeterminate .progress-bar-fill {
  width: 30% !important;
  background: linear-gradient(
    90deg,
    transparent 0%,
    var(--accent) 50%,
    transparent 100%
  );
  animation: indeterminate 1.4s ease-in-out infinite;
}
@keyframes indeterminate {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(400%);
  }
}
</style>
