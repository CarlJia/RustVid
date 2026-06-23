<template>
  <div class="card">
    <div class="card-header">
      <h2>历史任务</h2>
      <button
        v-if="failedCount > 0"
        @click="onPurgeFailed"
        :disabled="purging"
        class="secondary danger-link"
        :title="`清理 ${failedCount} 个失败任务及其残留文件`"
      >
        {{ purging ? "清理中…" : `一键清理 ${failedCount} 个失败任务` }}
      </button>
    </div>

    <p v-if="jobs.length === 0" class="message">暂无任务,上传一个视频试试。</p>
    <table v-else>
      <thead>
        <tr>
          <th>任务</th>
          <th>源文件</th>
          <th>预设</th>
          <th>输出</th>
          <th>状态</th>
          <th>创建时间</th>
          <th>操作</th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="job in jobs" :key="job.id">
          <td>
            <code>{{ job.id.slice(0, 8) }}</code>
          </td>
          <td class="filename-cell" :title="job.upload_filename ?? '—'">
            {{ job.upload_filename ?? "—" }}
          </td>
          <td>
            <div class="preset-cell">
              <strong>{{ job.preset_label }}</strong>
              <span class="muted">{{ job.preset_summary }}</span>
            </div>
          </td>
          <td>
            <div class="target-cell">
              <strong>{{ job.target_label }}</strong>
              <span class="muted">{{ job.target }}</span>
            </div>
          </td>
          <td>
            <StatusBadge :status="job.status" />
          </td>
          <td class="time-cell">{{ formatTime(job.created_at) }}</td>
          <td>
            <router-link :to="{ name: 'job', params: { id: job.id } }"
              >查看</router-link
            >
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from "vue";
import StatusBadge from "./StatusBadge.vue";
import { ask as askDialog, message as messageDialog } from "@tauri-apps/plugin-dialog";
import { deleteFailedJobs, type JobInfo } from "../api";

const props = defineProps<{ jobs: JobInfo[] }>();
const emit = defineEmits<{ purged: [] }>();

const purging = ref(false);

const failedCount = computed(
  () => props.jobs.filter((j) => j.status === "failed").length,
);

async function onPurgeFailed() {
  if (failedCount.value === 0) return;
  // Tauri 2 默认禁用 window.confirm,改用原生 dialog 的 ask()
  const confirmed = await askDialog(
    `确认清理 ${failedCount.value} 个失败任务?\n会同时删除它们的源文件、转码产物和数据库记录,操作不可撤销。`,
    {
      title: "清理失败任务",
      kind: "warning",
      okLabel: "清理",
      cancelLabel: "取消",
    },
  );
  if (!confirmed) return;
  purging.value = true;
  try {
    const n = await deleteFailedJobs();
    emit("purged");
    await messageDialog(`已清理 ${n} 个失败任务`, {
      title: "清理完成",
      kind: "info",
    });
  } catch (e) {
    await messageDialog(`清理失败:${e}`, {
      title: "清理失败",
      kind: "error",
    });
  } finally {
    purging.value = false;
  }
}

// SQLite CURRENT_TIMESTAMP 是 UTC(无时区后缀),前端按 UTC 解读再转本地展示
function formatTime(iso: string): string {
  if (!iso) return "—";
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const d = new Date(normalized);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}
</script>

<style scoped>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
  gap: 1rem;
  flex-wrap: wrap;
}
.card-header h2 {
  margin: 0;
  font-size: 1.1rem;
}
.danger-link {
  background: transparent;
  color: var(--danger);
  border: 1px solid var(--danger);
  padding: 0.375rem 0.75rem;
  font-size: 0.8125rem;
  font-weight: 500;
}
.danger-link:hover:not(:disabled) {
  background: var(--danger);
  color: white;
  opacity: 1;
}
.preset-cell,
.target-cell {
  display: flex;
  flex-direction: column;
  gap: 0.125rem;
}
.preset-cell .muted,
.target-cell .muted {
  color: var(--muted);
  font-size: 0.75rem;
}
.time-cell {
  color: var(--muted);
  font-variant-numeric: tabular-nums;
  font-size: 0.8125rem;
  white-space: nowrap;
}
.filename-cell {
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 0.8125rem;
  color: var(--fg);
}
</style>
