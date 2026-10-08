<script setup lang="ts">
/** 可拖拽的面板分隔线。 */
const props = defineProps<{ side: "left" | "right" }>();
const emit = defineEmits<{ (e: "resize", delta: number): void }>();

let startX = 0;

function onDown(e: PointerEvent): void {
  startX = e.clientX;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
}

function onMove(e: PointerEvent): void {
  if (!(e.target as HTMLElement).hasPointerCapture(e.pointerId)) return;
  const delta = e.clientX - startX;
  if (delta === 0) return;
  startX = e.clientX;
  emit("resize", props.side === "left" ? delta : -delta);
}
</script>

<template>
  <div class="panel-resizer" @pointerdown="onDown" @pointermove="onMove" />
</template>

<style scoped>
.panel-resizer {
  width: 3px;
  cursor: col-resize;
  background-color: rgb(var(--md-outline-variant) / 0.35);
  transition: background-color 0.15s ease;
  flex: 0 0 auto;
}
.panel-resizer:hover {
  background-color: rgb(var(--md-primary) / 0.7);
}
</style>
