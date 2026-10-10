<script setup lang="ts">
/**
 * 面板之间的可拖拽分隔线：就是一条 1px 细线（命中区向两侧扩 4px，不占布局宽度）。
 *
 * `side` 指的是**分隔线相邻的那一侧**，也就是「谁的尺寸会随拖动变化」：
 * - `left` / `right`：竖直分隔线，按 X 轴发出增量；
 * - `top` / `bottom`：水平分隔线，按 Y 轴发出增量。
 *
 * 发出的增量一律是「**该侧变大**为正」，与鼠标直觉一致：
 * - `left`（面板在分隔线左侧）：往右拖变大；
 * - `right`（面板在右侧）：往左拖变大；
 * - `top`（面板在分隔线上方）：**往下**拖变大；
 * - `bottom`（面板在分隔线下方）：**往上**拖变大。
 *
 * 拖拽状态用显式标志位而不是 `hasPointerCapture`：捕获在部分情形下会失败
 * （元素被移除、合成事件），而那种时候拖拽本身仍然应该继续工作。
 */
import { computed, onMounted, onUnmounted } from "vue";
import { capturePointer, releasePointer } from "@/utils/pointer";

const props = defineProps<{ side: "left" | "right" | "top" | "bottom" }>();
const emit = defineEmits<{ (e: "resize", delta: number): void }>();

const horizontal = computed(() => props.side === "top" || props.side === "bottom");

let dragging = false;
let startX = 0;
let startY = 0;

function onDown(e: PointerEvent): void {
  if (e.button !== 0) return;
  dragging = true;
  startX = e.clientX;
  startY = e.clientY;
  capturePointer(e.target, e.pointerId);
}

function onMove(e: PointerEvent): void {
  if (!dragging) return;
  if (horizontal.value) {
    const delta = e.clientY - startY;
    if (delta === 0) return;
    startY = e.clientY;
    // `top` 的面板在分隔线上方：往下拖（delta>0）变大；`bottom` 反之。
    emit("resize", props.side === "top" ? delta : -delta);
    return;
  }
  const delta = e.clientX - startX;
  if (delta === 0) return;
  startX = e.clientX;
  emit("resize", props.side === "left" ? delta : -delta);
}

/** 指针可能在分隔线之外松开，所以结束事件挂在 window 上。 */
function onUp(e: PointerEvent): void {
  if (!dragging) return;
  dragging = false;
  releasePointer(e.target, e.pointerId);
}

onMounted(() => {
  window.addEventListener("pointerup", onUp);
  window.addEventListener("pointercancel", onUp);
});
onUnmounted(() => {
  window.removeEventListener("pointerup", onUp);
  window.removeEventListener("pointercancel", onUp);
});
</script>

<template>
  <div
    class="panel-resizer"
    :class="{ horizontal, dragging }"
    role="separator"
    :aria-orientation="horizontal ? 'horizontal' : 'vertical'"
    @pointerdown="onDown"
    @pointermove="onMove"
  />
</template>

<style scoped>
.panel-resizer {
  position: relative;
  flex: 0 0 auto;
  width: 1px;
  align-self: stretch;
  background-color: rgb(var(--md-outline-variant) / 0.4);
  cursor: col-resize;
  touch-action: none;
}
.panel-resizer.horizontal {
  width: 100%;
  height: 1px;
  align-self: auto;
  cursor: row-resize;
}
/* 命中区向两侧各扩 4px：不占布局宽度，但更容易抓 */
.panel-resizer::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -4px;
  right: -4px;
}
.panel-resizer.horizontal::before {
  top: -4px;
  bottom: -4px;
  left: 0;
  right: 0;
}
.panel-resizer:hover,
.panel-resizer.dragging {
  background-color: rgb(var(--md-primary) / 0.85);
}
</style>
