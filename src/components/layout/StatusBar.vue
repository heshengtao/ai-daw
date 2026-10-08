<script setup lang="ts">
/** 底部状态栏：引擎状态 / 采样率 / 缓冲。 */
</script>

<template>
  <footer class="status-bar">
    <span class="dot live" />
    <span>引擎就绪</span>
    <span class="sep">·</span>
    <span>44.1 kHz</span>
    <span class="sep">·</span>
    <span>块 256 帧</span>
    <div class="flex-1" />
    <span class="hint">试听为离线渲染预览 · 实时传输将在下一版接入</span>
  </footer>
</template>

<style scoped>
.status-bar {
  height: 26px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 12px;
  font-size: 11px;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-variant));
  background-color: rgb(var(--md-surface-container-low));
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
}
.sep {
  opacity: 0.5;
}
.hint {
  opacity: 0.7;
}
.dot {
  width: 7px;
  height: 7px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface-muted));
}
.dot.live {
  background-color: rgb(var(--md-primary));
}
</style>
