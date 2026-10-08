<script setup lang="ts">
/**
 * MD3 旋钮。
 *
 * 视觉完全走 Material 3 的语言：**扁平 + 粗圆角**，没有渐变、没有高光、没有拟物刻度盘。
 * 一圈粗轨道（surface 色）＋ 一段粗主色圆弧 ＋ 一条圆头指示条；聚焦时是一圈圆环
 * （不是浏览器给 SVG 画的方框 outline）。
 *
 * 交互按插件惯例：
 * - 上下拖动改值（Shift 精细，拖到两端吸附）；
 * - 双击回默认值；滚轮也能调；
 * - 键盘 ↑↓←→ / Home / End（暴露成 `role="slider"`）。
 *
 * 拖动中持续发 `update:modelValue`（面板要跟手），松手才发 `commit`（写回工程）。
 */
import { computed, ref } from "vue";

const props = withDefaults(
  defineProps<{
    modelValue: number;
    min: number;
    max: number;
    /** 拖动灵敏度：走完全程需要的像素。 */
    sensitivity?: number;
    /** 双击复位用的默认值。 */
    default?: number;
    label: string;
    /** 旋钮下方显示的当前值（已格式化）。 */
    display?: string;
    size?: number;
    disabled?: boolean;
  }>(),
  { sensitivity: 150, size: 54 },
);

const emit = defineEmits<{
  (e: "update:modelValue", value: number): void;
  (e: "commit", value: number): void;
}>();

const dragging = ref(false);
const focused = ref(false);

/** 弧的起止角（0° 在正上方，逆时针为负）。 */
const START = -135;
const SWEEP = 270;

const ratio = computed(() => {
  const span = props.max - props.min;
  if (span <= 0) return 0;
  return Math.min(1, Math.max(0, (props.modelValue - props.min) / span));
});

const angle = computed(() => START + ratio.value * SWEEP);

/** 几何：以 (c, c) 为圆心、半径 r 画弧。 */
const c = computed(() => props.size / 2);
const stroke = computed(() => Math.max(4, props.size * 0.1));
const r = computed(() => props.size / 2 - stroke.value / 2 - 1);

function point(deg: number, radius: number): [number, number] {
  const rad = ((deg - 90) * Math.PI) / 180;
  return [c.value + radius * Math.cos(rad), c.value + radius * Math.sin(rad)];
}

function arc(fromDeg: number, toDeg: number): string {
  const [x1, y1] = point(fromDeg, r.value);
  const [x2, y2] = point(toDeg, r.value);
  const large = Math.abs(toDeg - fromDeg) > 180 ? 1 : 0;
  return `M ${x1} ${y1} A ${r.value} ${r.value} 0 ${large} 1 ${x2} ${y2}`;
}

const track = computed(() => arc(START, START + SWEEP));
const fill = computed(() => (ratio.value <= 0.002 ? "" : arc(START, angle.value)));

/** 指示条：圆心附近到轨道内侧的一条圆头短线。 */
const indicator = computed(() => {
  const [x1, y1] = point(angle.value, r.value * 0.2);
  const [x2, y2] = point(angle.value, r.value * 0.72);
  return { x1, y1, x2, y2 };
});

function clamp(value: number): number {
  return Math.min(props.max, Math.max(props.min, value));
}

function set(value: number, commit = false): void {
  const next = clamp(value);
  if (next !== props.modelValue) emit("update:modelValue", next);
  if (commit) emit("commit", next);
}

let lastY = 0;
let moved = false;

function onPointerDown(e: PointerEvent): void {
  if (props.disabled) return;
  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  dragging.value = true;
  moved = false;
  lastY = e.clientY;
}

function onPointerMove(e: PointerEvent): void {
  if (!dragging.value) return;
  const dy = lastY - e.clientY;
  if (dy === 0) return;
  lastY = e.clientY;
  moved = true;
  const span = props.max - props.min;
  const fast = e.shiftKey ? 0.2 : 1;
  const next = props.modelValue + (dy / props.sensitivity) * span * fast;
  // 拖到两端时吸附，免得停在 0.9999。
  const snapped =
    Math.abs(next - props.min) < span * 0.01
      ? props.min
      : Math.abs(next - props.max) < span * 0.01
        ? props.max
        : next;
  set(snapped);
}

function onPointerUp(e: PointerEvent): void {
  if (!dragging.value) return;
  (e.currentTarget as HTMLElement).releasePointerCapture?.(e.pointerId);
  dragging.value = false;
  if (moved) emit("commit", props.modelValue);
}

function onDoubleClick(): void {
  if (props.disabled || props.default === undefined) return;
  set(props.default, true);
}

function onWheel(e: WheelEvent): void {
  if (props.disabled) return;
  e.preventDefault();
  const span = props.max - props.min;
  set(props.modelValue - Math.sign(e.deltaY) * span * (e.shiftKey ? 0.005 : 0.03), true);
}

function onKey(e: KeyboardEvent): void {
  if (props.disabled) return;
  const span = props.max - props.min;
  const step = span * (e.shiftKey ? 0.001 : 0.02);
  if (e.key === "ArrowUp" || e.key === "ArrowRight") set(props.modelValue + step, true);
  else if (e.key === "ArrowDown" || e.key === "ArrowLeft") set(props.modelValue - step, true);
  else if (e.key === "Home") set(props.min, true);
  else if (e.key === "End") set(props.max, true);
  else return;
  e.preventDefault();
}
</script>

<template>
  <div class="knob" :class="{ disabled, dragging }">
    <svg
      :width="size"
      :height="size"
      role="slider"
      tabindex="0"
      :aria-label="label"
      :aria-valuemin="min"
      :aria-valuemax="max"
      :aria-valuenow="modelValue"
      :aria-valuetext="display"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @dblclick="onDoubleClick"
      @wheel="onWheel"
      @keydown="onKey"
      @focus="focused = true"
      @blur="focused = false"
      @mousedown.prevent
    >
      <!-- 聚焦环：自己画的，别让浏览器给 SVG 套一个方框 -->
      <circle
        v-if="focused"
        class="ring"
        :cx="c"
        :cy="c"
        :r="size / 2 - 1"
        :stroke-width="2"
      />
      <!-- 底盘 -->
      <circle class="face" :cx="c" :cy="c" :r="r - stroke * 0.75" />
      <path class="track" :d="track" :stroke-width="stroke" />
      <path v-if="fill" class="fill" :d="fill" :stroke-width="stroke" />
      <line
        class="indicator"
        :x1="indicator.x1"
        :y1="indicator.y1"
        :x2="indicator.x2"
        :y2="indicator.y2"
        :stroke-width="stroke * 0.5"
      />
    </svg>
    <span class="label">{{ label }}</span>
    <span v-if="display" class="value">{{ display }}</span>
  </div>
</template>

<style scoped>
.knob {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  min-width: 56px;
  user-select: none;
}
.knob.disabled {
  opacity: 0.45;
}
svg {
  display: block;
  cursor: ns-resize;
  touch-action: none;
  outline: none;
}
.knob.dragging svg {
  cursor: grabbing;
}
.ring {
  fill: none;
  stroke: rgb(var(--md-primary) / 0.6);
}
.face {
  fill: rgb(var(--md-surface-container-highest));
}
.track {
  fill: none;
  stroke: rgb(var(--md-outline-variant));
  stroke-linecap: round;
}
.fill {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-linecap: round;
}
.indicator {
  stroke: rgb(var(--md-on-surface));
  stroke-linecap: round;
}
.label {
  font-size: 10px;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.value {
  font-size: 9.5px;
  font-variant-numeric: tabular-nums;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
</style>
