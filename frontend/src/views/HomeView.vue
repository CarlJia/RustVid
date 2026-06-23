<template>
  <div>
    <UploadForm @uploaded="refresh" />

    <div class="card">
      <h2>存储用量</h2>
      <p v-if="usage" class="usage">
        RustVid 已用 {{ formatBytes(usage.used_bytes) }}
      </p>
      <p v-if="usage && usage.disk_total_bytes > 0" class="usage">
        系统磁盘剩余 {{ formatBytes(usage.disk_free_bytes) }} /
        共 {{ formatBytes(usage.disk_total_bytes) }}
        （{{ diskUsedPercent(usage) }}% 已用）
      </p>
      <p v-else-if="usage" class="usage">
        系统磁盘剩余 {{ formatBytes(usage.disk_free_bytes) }}
      </p>
      <p v-else class="message">容量信息暂不可用</p>
    </div>

    <JobList :jobs="jobs" @purged="refresh" />
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import UploadForm from "../components/UploadForm.vue";
import JobList from "../components/JobList.vue";
import { listJobs, getUsage, type JobInfo, type UsageInfo } from "../api";

const jobs = ref<JobInfo[]>([]);
const usage = ref<UsageInfo | null>(null);
let timer: number | null = null;

async function refresh() {
  try {
    jobs.value = await listJobs();
    usage.value = await getUsage();
  } catch (e) {
    console.error("刷新失败", e);
  }
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function diskUsedPercent(u: UsageInfo): number {
  // total=0 时已经在模板里短路了,这里不会触发
  return Math.round(((u.disk_total_bytes - u.disk_free_bytes) / u.disk_total_bytes) * 100);
}

onMounted(() => {
  refresh();
  // 轮询:每 2 秒刷新任务列表(简单实现,Phase 2 改 Tauri event)
  timer = window.setInterval(refresh, 2000);
});

onUnmounted(() => {
  if (timer != null) clearInterval(timer);
});
</script>
