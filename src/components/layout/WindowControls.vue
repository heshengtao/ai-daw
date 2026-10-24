<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'

const isMaximized = ref(false)
const appWindow = getCurrentWindow()

let unlisten: (() => void) | null = null

onMounted(async () => {
  isMaximized.value = await appWindow.isMaximized()
  unlisten = await appWindow.onResized(() => {
    appWindow.isMaximized().then(v => { isMaximized.value = v })
  })
})

onUnmounted(() => {
  unlisten?.()
})
</script>

<template>
  <div class="window-controls" data-tauri-drag-region="false">
    <button class="window-control-btn" @click="appWindow.minimize()" :title="'Minimize'">
      <svg width="16" height="16" viewBox="0 0 11 11">
        <rect x="0.5" y="5" width="10" height="1" fill="currentColor" rx="0.5" />
      </svg>
    </button>
    <button
      class="window-control-btn"
      @click="appWindow.toggleMaximize()"
      :title="isMaximized ? 'Restore' : 'Maximize'"
    >
      <svg v-if="isMaximized" width="16" height="16" viewBox="0 0 11 11">
        <rect x="2.5" y="0.5" width="8" height="8" rx="1" fill="none" stroke="currentColor" stroke-width="0.9" />
        <rect x="0.5" y="2.5" width="8" height="8" rx="1" fill="rgb(var(--md-surface))" stroke="currentColor" stroke-width="0.9" />
      </svg>
      <svg v-else width="16" height="16" viewBox="0 0 11 11">
        <rect x="1" y="1" width="9" height="9" rx="1.5" fill="none" stroke="currentColor" stroke-width="1" />
      </svg>
    </button>
    <button class="window-control-btn" @click="appWindow.close()" :title="'Close'">
      <svg width="16" height="16" viewBox="0 0 11 11">
        <path d="M1 1l9 9M10 1L1 10" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.window-controls {
  display: flex;
  align-items: center;
  gap: 1px;
}
/* 与顶栏 .bar-btn 完全一致：34×34 圆形 MD3 图标按钮 */
.window-control-btn {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.15s ease, color 0.15s ease, transform 0.15s ease;
}
.window-control-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.window-control-btn:active {
  transform: scale(0.94);
}
.window-control-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}

@media (prefers-reduced-motion: reduce) {
  .window-control-btn {
    transition: none;
  }
  .window-control-btn:active {
    transform: none;
  }
}
</style>
