<script setup lang="ts">
/**
 * 控制器泳道：经典 DAW 钢琴卷帘下方那一条「直角条」。
 *
 * 每个音符对应一根矩形条：
 * - **横位置与宽度**与上方卷帘里的音符**一一对应**（同一起点、同一时长、同一缩放、
 *   同一绝对时间轴——所以片段有偏移时也照样对齐）；
 * - **高度**表示当前所选属性。
 *
 * 目前只做两个**所有音源都通用**的属性（不同插件将来会带来各自的属性）：
 * - **力度**（演奏强度）：手弹得多重。内置合成器把它折算成振幅；
 * - **音量**（单音电平）：纯增益微调，不改变音色，默认 127（不衰减）。
 *
 * 交互：上下拖动条子改值（多选整组、松手才提交一次）；空白处左键框选；
 * 右键点击删除、右键拖动框选批量删除。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { capturePointer } from "@/utils/pointer";
import { useI18n } from "@/i18n";
import { useTimelineStore } from "@/stores/timeline";
import type { Note } from "@/api/types";

/** 泳道可显示的属性。 */
export type ControllerProperty = "volume" | "velocity";

interface Spec {
  /** i18n key（渲染时翻译）。 */
  labelKey: string;
  hintKey: string;
  min: number;
  max: number;
  step: number;
  digits: number;
  /** 取值。 */
  get: (n: Note) => number;
  /** 生成改写补丁。 */
  patch: (v: number) => Partial<Note>;
}

const MAX_MIDI = 127;

const SPECS: Record<ControllerProperty, Spec> = {
  volume: {
    labelKey: "lane.volume",
    hintKey: "lane.volumeHint",
    min: 0,
    max: MAX_MIDI,
    step: 1,
    digits: 0,
    get: (n) => n.volume,
    patch: (v) => ({ volume: Math.round(v) }),
  },
  velocity: {
    labelKey: "lane.velocity",
    hintKey: "lane.velocityHint",
    min: 0,
    max: MAX_MIDI,
    step: 1,
    digits: 0,
    get: (n) => n.velocity,
    patch: (v) => ({ velocity: Math.round(v) }),
  },
};

/** 分段按钮的展示顺序（按需求原文：音量在前）。 */
const OPTIONS: ControllerProperty[] = ["volume", "velocity"];

const props = defineProps<{
  notes: Note[];
  selectedIndices: number[];
  property: ControllerProperty;
  /** 片段在时间轴上的起点（拍）：音符坐标是相对片段的，这里换算成绝对时间。 */
  offsetBeats: number;
  /** 泳道内容高度（条形的最大高度）。 */
  contentHeight: number;
}>();

const emit = defineEmits<{
  (e: "update:property", value: ControllerProperty): void;
  (e: "select", indices: number[]): void;
  (e: "apply", patch: Partial<Note>): void;
  (e: "delete", indices: number[]): void;
}>();

const timeline = useTimelineStore();
const i18n = useI18n();
const spec = computed(() => SPECS[props.property]);

/** 拖动中的值（预览用）；松手才提交。 */
const dragValue = ref<number | null>(null);
const mode = ref<"value" | "marquee" | "marquee-delete" | null>(null);
const marquee = reactive({ x1: 0, y1: 0, x2: 0, y2: 0 });

const bodyEl = ref<HTMLElement | null>(null);
const canvasEl = ref<HTMLElement | null>(null);

/** 画布：宽度与时间轴一致，并用 transform 跟随横向滚动（与上方卷帘逐像素同步）。 */
const canvasStyle = computed(() => ({
  width: `${timeline.contentWidth}px`,
  transform: `translateX(${-timeline.scrollLeft}px)`,
  ...timeline.gridStyle,
}));

const marqueeStyle = computed(() => ({
  left: `${Math.min(marquee.x1, marquee.x2)}px`,
  top: `${Math.min(marquee.y1, marquee.y2)}px`,
  width: `${Math.abs(marquee.x2 - marquee.x1)}px`,
  height: `${Math.abs(marquee.y2 - marquee.y1)}px`,
}));

/** 某个数值对应的条形高度（px）。 */
function heightFor(value: number): number {
  const { min, max } = spec.value;
  const ratio = (value - min) / (max - min || 1);
  return Math.max(2, Math.min(1, Math.max(0, ratio)) * props.contentHeight);
}

/** 渲染用数值：拖动中的已选音符显示预览值。 */
function valueOf(index: number, note: Note): number {
  if (dragValue.value !== null && props.selectedIndices.includes(index)) return dragValue.value;
  return spec.value.get(note);
}

function barStyle(index: number, note: Note): Record<string, string> {
  return {
    left: `${(props.offsetBeats + note.start) * timeline.zoom}px`,
    width: `${Math.max(3, note.duration * timeline.zoom)}px`,
    height: `${heightFor(valueOf(index, note))}px`,
  };
}

function isSelected(index: number): boolean {
  return props.selectedIndices.includes(index);
}

/** 指针 y → 数值（顶部最大）。 */
function valueFromY(clientY: number): number {
  const el = bodyEl.value;
  if (!el) return spec.value.min;
  const rect = el.getBoundingClientRect();
  const usable = Math.max(1, rect.height - 6);
  const ratio = Math.min(1, Math.max(0, 1 - (clientY - rect.top - 3) / usable));
  const { min, max, step } = spec.value;
  const raw = min + ratio * (max - min);
  return Math.min(max, Math.max(min, Math.round(raw / step) * step));
}

function intersects(el: Element): boolean {
  const r = el.getBoundingClientRect();
  const left = Math.min(marquee.x1, marquee.x2);
  const right = Math.max(marquee.x1, marquee.x2);
  const top = Math.min(marquee.y1, marquee.y2);
  const bottom = Math.max(marquee.y1, marquee.y2);
  return !(r.right < left || r.left > right || r.bottom < top || r.top > bottom);
}

// ---------------------------------------------------------------------------
// 指针
// ---------------------------------------------------------------------------

function onBarPointerDown(e: PointerEvent, index: number): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  if (e.metaKey || e.ctrlKey) {
    const next = isSelected(index)
      ? props.selectedIndices.filter((i) => i !== index)
      : [...props.selectedIndices, index].sort((a, b) => a - b);
    emit("select", next);
    return;
  }
  if (!isSelected(index)) emit("select", [index]);
  capturePointer(e.currentTarget, e.pointerId);
  dragValue.value = valueFromY(e.clientY);
  mode.value = "value";
}

function onBodyPointerDown(e: PointerEvent): void {
  if (e.button === 2) {
    if ((e.target as HTMLElement).closest(".bar")) return;
    capturePointer(e.currentTarget, e.pointerId);
    mode.value = "marquee-delete";
    marquee.x1 = marquee.x2 = e.clientX;
    marquee.y1 = marquee.y2 = e.clientY;
    return;
  }
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest(".bar")) return;
  capturePointer(e.currentTarget, e.pointerId);
  mode.value = "marquee";
  marquee.x1 = marquee.x2 = e.clientX;
  marquee.y1 = marquee.y2 = e.clientY;
}

function onPointerMove(e: PointerEvent): void {
  if (mode.value === "value") {
    dragValue.value = valueFromY(e.clientY);
    return;
  }
  if (mode.value === "marquee" || mode.value === "marquee-delete") {
    marquee.x2 = e.clientX;
    marquee.y2 = e.clientY;
  }
}

function collectIntersecting(): number[] {
  const out: number[] = [];
  canvasEl.value?.querySelectorAll<HTMLElement>(".bar").forEach((el) => {
    if (!intersects(el)) return;
    const index = Number(el.dataset.index);
    if (Number.isFinite(index)) out.push(index);
  });
  return out;
}

function onPointerUp(): void {
  const current = mode.value;
  mode.value = null;

  if (current === "value") {
    const value = dragValue.value;
    dragValue.value = null;
    if (value === null) return;
    emit("apply", spec.value.patch(value));
    return;
  }
  if (current === "marquee") {
    emit("select", collectIntersecting());
    return;
  }
  if (current === "marquee-delete") {
    const width = Math.abs(marquee.x2 - marquee.x1);
    const height = Math.abs(marquee.y2 - marquee.y1);
    if (width < 4 && height < 4) return; // 原地右键：交给 contextmenu 处理
    emit("delete", collectIntersecting());
  }
}

function onBarContextMenu(e: MouseEvent, index: number): void {
  e.preventDefault();
  e.stopPropagation();
  emit("delete", isSelected(index) ? [...props.selectedIndices] : [index]);
}

function onContextMenu(e: MouseEvent): void {
  e.preventDefault();
}

/** 头部读数：多选时显示主选（末位）的值。 */
const readout = computed(() => {
  const indices = props.selectedIndices;
  if (indices.length === 0) return null;
  const primary = props.notes[indices[indices.length - 1]];
  if (!primary) return null;
  const value = dragValue.value ?? spec.value.get(primary);
  return { count: indices.length, value: value.toFixed(spec.value.digits) };
});

onMounted(() => {
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
});
onUnmounted(() => {
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
});
</script>

<template>
  <div class="lane-block">
    <!-- 头部：属性选择 + 读数 -->
    <div class="lane-head">
      <div class="seg" role="group" :aria-label="i18n.t('lane.velocity')">
        <button
          v-for="opt in OPTIONS"
          :key="opt"
          class="seg-btn"
          :class="{ on: property === opt }"
          :aria-pressed="property === opt"
          :title="i18n.t(SPECS[opt].hintKey)"
          @click="emit('update:property', opt)"
        >
          {{ i18n.t(SPECS[opt].labelKey) }}
        </button>
      </div>

      <span v-if="readout" class="readout">
        {{ i18n.t("lane.selected", { count: readout.count }) }} · {{ i18n.t(spec.labelKey) }}
        <strong class="tabular">{{ readout.value }}</strong>
      </span>
      <span v-else class="readout muted">{{ i18n.t(spec.hintKey) }}</span>
    </div>

    <!-- 泳道本体：左侧 60px 与卷帘的琴键列对齐 -->
    <div class="lane-row">
      <div class="lane-gutter">
        <span class="max">{{ spec.max }}</span>
        <span class="min">{{ spec.min }}</span>
      </div>

      <div class="lane-clip">
        <div
          ref="bodyEl"
          class="lane-body"
          :style="{ height: `${contentHeight + 6}px` }"
          @pointerdown="onBodyPointerDown"
          @pointermove="onPointerMove"
          @contextmenu="onContextMenu"
        >
          <div ref="canvasEl" class="canvas" :style="canvasStyle">
            <div
              v-for="(n, i) in notes"
              :key="i"
              class="bar"
              :class="{ selected: isSelected(i), dragging: mode === 'value' && isSelected(i) }"
              :style="barStyle(i, n)"
              :data-index="i"
              :title="i18n.t(spec.labelKey)"
              @pointerdown="onBarPointerDown($event, i)"
              @contextmenu="onBarContextMenu($event, i)"
            />
          </div>

          <div
            v-if="mode === 'marquee' || mode === 'marquee-delete'"
            class="marquee"
            :class="{ danger: mode === 'marquee-delete' }"
            :style="marqueeStyle"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lane-block {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background-color: var(--app-card);
  user-select: none;
  -webkit-user-select: none;
}
.lane-head {
  height: 28px;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0 12px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
/* 属性选择：自绘分段按钮 */
.seg {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  flex: 0 0 auto;
}
.seg-btn {
  height: 20px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.seg-btn:hover:not(.on) {
  color: rgb(var(--md-on-surface));
}
/* 选中的那一项要**一眼看得出来**：高对比实心底 + 加粗（原来用容器色，和轨道底色太接近）。 */
.seg-btn.on {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-weight: 600;
}
.seg-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.readout {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.readout.muted {
  color: rgb(var(--md-on-surface-muted));
}
.readout strong {
  color: rgb(var(--md-on-surface));
  margin-left: 4px;
}
.tabular {
  font-variant-numeric: tabular-nums;
}
.lane-row {
  display: flex;
  min-width: 0;
}
/* 左侧刻度列：与卷帘的琴键列同宽，保证条形与上方音符逐像素对齐 */
.lane-gutter {
  flex: 0 0 60px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: flex-end;
  padding: 3px 6px 3px 0;
  font-size: 8.5px;
  color: rgb(var(--md-on-surface-muted));
  background-color: var(--app-inset);
  border-right: 1px solid var(--grid-line-row);
  font-variant-numeric: tabular-nums;
}
.lane-clip {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
}
.lane-body {
  position: relative;
  overflow: hidden;
  background-color: var(--app-card);
  cursor: crosshair;
}
/* 网格：与卷帘同一套细分 / 拍 / 小节线 */
.canvas {
  position: absolute;
  inset: 0;
}
/* 直角条：底部对齐的实心矩形 */
.bar {
  position: absolute;
  bottom: 3px;
  border-radius: 1px;
  background-color: rgb(var(--md-primary) / 0.85);
  cursor: ns-resize;
  touch-action: none;
}
.bar.selected {
  background-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 1px rgb(var(--md-on-surface) / 0.55);
}
.bar.dragging {
  opacity: 0.85;
}
.marquee {
  position: fixed;
  z-index: 50;
  border: 1px solid rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.16);
  pointer-events: none;
}
.marquee.danger {
  border-color: rgb(var(--md-error));
  background-color: rgb(var(--md-error) / 0.16);
}
@media (prefers-reduced-motion: reduce) {
  .seg-btn {
    transition: none;
  }
}
</style>
