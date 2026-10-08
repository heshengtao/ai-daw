<script setup lang="ts">
/*!
 * Custom Tauri title bar (drag region + window controls).
 * Adapted from ExamBot (https://github.com/heshengtao/exameow), Apache-2.0.
 */
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isLinux, isMacOS, isTauri, isWindows } from "@/utils/platform";
import WindowControls from "./WindowControls.vue";

const isMac = isMacOS();
const showControls = isTauri() && (isWindows() || isLinux());

function onMouseDown(e: MouseEvent): void {
  if (!isTauri()) return;
  if (e.buttons !== 1) return;
  const target = e.target as HTMLElement;
  if (target.closest("button, a, input, select")) return;
  const win = getCurrentWindow();
  if (e.detail === 2) {
    void win.toggleMaximize();
  } else {
    void win.startDragging();
  }
}
</script>

<template>
  <div
    class="sticky top-0 z-40 h-[38px] flex items-center select-none shrink-0"
    :class="isMac ? 'pl-[80px]' : 'pl-3'"
    :style="{ backgroundColor: 'rgb(var(--md-surface))' }"
    @mousedown="onMouseDown"
  >
    <div class="flex items-center gap-2">
      <span class="w-2 h-2 rounded-full" :style="{ backgroundColor: 'rgb(var(--md-primary))' }" />
      <span class="text-xs font-medium" :style="{ color: 'rgb(var(--md-on-surface-variant))' }">daw-party</span>
    </div>
    <div class="flex-1 h-full" />
    <WindowControls v-if="showControls" class="h-full" />
  </div>
</template>
