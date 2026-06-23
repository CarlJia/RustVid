<template>
  <span :class="`status status-${badgeClass}`">{{ label }}</span>
</template>

<script setup lang="ts">
import { computed } from "vue";

const props = defineProps<{ status: string }>();

// Rust 端 JobStatus::as_str 返回小写("completed"),前端统一小写比较
const normalized = computed(() => props.status.toLowerCase());

const badgeClass = computed(() => {
  switch (normalized.value) {
    case "queued":
      return "queued";
    case "processing":
      return "processing";
    case "completed":
      return "completed";
    case "failed":
      return "failed";
    case "deleted":
      return "deleted";
    default:
      return "queued";
  }
});

const label = computed(() => {
  switch (normalized.value) {
    case "queued":
      return "排队中";
    case "processing":
      return "处理中";
    case "completed":
      return "已完成";
    case "failed":
      return "失败";
    case "deleted":
      return "已删除";
    default:
      return props.status;
  }
});
</script>
