<script setup lang="ts">
/**
 * MD3 滑块：完全自绘（轨道 + 手柄 + 状态层），不使用原生 input[type=range]。
 * 支持指针拖拽与键盘（方向键 / Home / End），带 role="slider" 无障碍语义。
 */
import { computed, ref } from "vue";
import { capturePointer, releasePointer } from "@/utils/pointer";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min?: number;
    max?: number;
    step?: number;
    label?: string;
    disabled?: boolean;
  }>(),
  { min: 0, max: 100, step: 1, disabled: false },
);
const emit = defineEmits<{
  (e: "update:modelValue", v: number): void;
  /** 拖拽/键盘操作结束：调用方据此提交一次正式编辑（而不是每次移动都提交）。 */
  (e: "commit", v: number): void;
}>();

const root = ref<HTMLElement | null>(null);
const dragging = ref(false);

/** 已填充比例（0..1）。 */
const ratio = computed(() => {
  const span = props.max - props.min;
  if (span <= 0) return 0;
  return Math.min(1, Math.max(0, (props.modelValue - props.min) / span));
});

/** 量化到步长并夹在区间内。 */
function quantize(v: number): number {
  const stepped = Math.round((v - props.min) / props.step) * props.step + props.min;
  // 消除浮点误差（例如 0.1 步长时的 0.30000000000000004）。
  const clean = Number(stepped.toFixed(6));
  return Math.min(props.max, Math.max(props.min, clean));
}

/**
 * 最近一次通过 `update:modelValue` 报出去的值。
 *
 * 松手时要提交的是「用户刚选的值」，而 `props.modelValue` 在父组件用草稿模式
 * （只监听 commit、不 v-model）时仍然是旧值——直接用它会提交旧值，表现为「拖了没反应」。
 */
let pending: number | null = null;

function emitValue(v: number): void {
  const next = quantize(v);
  if (next !== props.modelValue) {
    pending = next;
    emit("update:modelValue", next);
  }
}

/** 提交本次交互的最终值（拖拽结束 / 键盘操作）。 */
function commitValue(): void {
  const value = pending ?? props.modelValue;
  pending = null;
  emit("commit", value);
}

function valueFromClientX(clientX: number): void {
  const el = root.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0) return;
  const r = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  emitValue(props.min + r * (props.max - props.min));
}

function onPointerDown(e: PointerEvent): void {
  if (props.disabled) return;
  dragging.value = true;
  capturePointer(e.currentTarget, e.pointerId);
  valueFromClientX(e.clientX);
}

function onPointerMove(e: PointerEvent): void {
  if (!dragging.value) return;
  valueFromClientX(e.clientX);
}

function onPointerUp(e: PointerEvent): void {
  if (!dragging.value) return;
  dragging.value = false;
  releasePointer(e.currentTarget, e.pointerId);
  commitValue();
}

function onKey(e: KeyboardEvent): void {
  if (props.disabled) return;
  const big = props.step * 10;
  let next: number;
  switch (e.key) {
    case "ArrowRight":
    case "ArrowUp":
      next = props.modelValue + props.step;
      break;
    case "ArrowLeft":
    case "ArrowDown":
      next = props.modelValue - props.step;
      break;
    case "PageUp":
      next = props.modelValue + big;
      break;
    case "PageDown":
      next = props.modelValue - big;
      break;
    case "Home":
      next = props.min;
      break;
    case "End":
      next = props.max;
      break;
    default:
      return;
  }
  e.preventDefault();
  emitValue(next);
  // 键盘操作一次到位，直接提交最终值。
  commitValue();
}
</script>

<template>
  <div
    ref="root"
    class="m3-slider"
    :class="{ dragging, disabled }"
    role="slider"
    :tabindex="disabled ? -1 : 0"
    :aria-label="label"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-valuenow="modelValue"
    :aria-disabled="disabled"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @pointerup="onPointerUp"
    @pointercancel="onPointerUp"
    @keydown="onKey"
  >
    <div class="track">
      <div class="track-active" :style="{ width: `${ratio * 100}%` }" />
    </div>
    <div class="thumb" :style="{ left: `${ratio * 100}%` }">
      <span class="thumb-state" />
    </div>
  </div>
</template>

<style scoped>
.m3-slider {
  position: relative;
  width: 100%;
  height: 24px;
  display: flex;
  align-items: center;
  cursor: pointer;
  touch-action: none;
  outline: none;
}
.m3-slider.disabled {
  opacity: 0.38;
  cursor: default;
}
.track {
  position: absolute;
  left: 0;
  right: 0;
  height: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  overflow: hidden;
}
.track-active {
  height: 100%;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  transition: width var(--md-ease-emphasized);
}
.m3-slider.dragging .track-active {
  transition: none;
}
.thumb {
  position: absolute;
  top: 50%;
  width: 4px;
  height: 4px;
  margin-left: -2px;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: left var(--md-ease-emphasized);
}
.m3-slider.dragging .thumb {
  transition: none;
}
/* 手柄本体 */
.thumb::after {
  content: "";
  position: absolute;
  width: 18px;
  height: 18px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  transition:
    width var(--md-ease-emphasized),
    height var(--md-ease-emphasized);
}
.m3-slider:hover .thumb::after,
.m3-slider.dragging .thumb::after {
  width: 20px;
  height: 20px;
}
/* 状态层：悬停 / 拖拽时扩散 */
.thumb-state {
  position: absolute;
  width: 36px;
  height: 36px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  opacity: 0;
  transform: scale(0.6);
  transition:
    opacity var(--md-ease-emphasized),
    transform var(--md-ease-emphasized);
}
.m3-slider:hover .thumb-state {
  opacity: 0.1;
  transform: scale(1);
}
.m3-slider.dragging .thumb-state {
  opacity: 0.16;
  transform: scale(1);
}
.m3-slider:focus-visible .thumb::after {
  box-shadow: 0 0 0 3px rgb(var(--md-primary) / 0.3);
}

@media (prefers-reduced-motion: reduce) {
  .track-active,
  .thumb,
  .thumb::after,
  .thumb-state {
    transition: none;
  }
}
</style>
