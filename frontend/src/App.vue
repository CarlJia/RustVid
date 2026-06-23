<template>
  <div class="shell">
    <header>
      <h1>
        <router-link to="/">RustVid 视频处理后台</router-link>
      </h1>
      <p class="lead">
        上传视频,选择用途预设,导出 MP4 或 HLS/m3u8 文件包。
      </p>
    </header>
    <main>
      <router-view />
    </main>
  </div>
</template>

<script setup lang="ts">
// 应用根组件
</script>
