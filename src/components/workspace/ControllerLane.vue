<script setup lang="ts">
/**
 * 参数车道：经典 DAW 钢琴卷帘下方那一条「泳道」，按所选目标切换两种形态。
 *
 * - **力度**（`velocity`，逐音符）：每根直角条的**横位置与宽度**与上方卷帘里的音符
 *   **一一对应**（同一起点、同一时长、同一缩放、同一绝对时间轴——所以片段有偏移时也照样对齐），
 *   高度是力度。上下拖动改值（多选整组、松手才提交一次）、空白处左键框选、右键点击删除、
 *   右键拖动框选批量删除。走 `set_notes`，与旧控制器泳道逐字一致。
 * - **其余目标**（`gain_db` / `pan` / `inst:<id>` / `fx<i>:<id>`）：就是**编排视图自动化车道的
 *   同一份数据**（`Track.automation`），用共享的 `AutomationCurve` 画曲线。左侧 60px 刻度列与
 *   琴键列对齐（所以曲线传 `insetPx = 0`，不画自带刻度列）。
 *
 * 组件本身**不认识工程**：力度改动经 `apply` 交回、曲线经 `applyCurve` 交回，由外壳（`EditView`）
 * 落实提交。可选目标由外壳经 `targets` 传入（本组件只在最前面补上「力度」这一项）。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import AutomationCurve from "@/components/workspace/AutomationCurve.vue";
import { capturePointer } from "@/utils/pointer";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useTimelineStore } from "@/stores/timeline";
import type { AutomationPoint, Note } from "@/api/types";
import type { AutomationTarget } from "@/utils/automation";

/** 逐音符属性的取值域与读写（目前只有力度）。 */
interface Spec {
  min: number;
  max: number;
  step: number;
  digits: number;
  /** 取值。 */
  get: (n: Note) => number;
  /** 生成改写补丁。 */
  patch: (v: number) => Partial<Note>;
}

/**
 * 力度：本组件唯一保留的**逐音符**属性（写在 MIDI Note On 里，所有乐器通用）。
 *
 * 其余目标一律是**轨道参数曲线**（`Track.automation`）——它们不是逐音符的量，由
 * `AutomationCurve` 编辑、外壳提交，见 `utils/automation.ts`。
 */
const spec: Spec = {
  min: 0,
  max: 127,
  step: 1,
  digits: 0,
  get: (n) => n.velocity,
  patch: (v) => ({ velocity: Math.round(v) }),
};

const props = defineProps<{
  notes: Note[];
  selectedIndices: number[];
  /** 当前选中的目标键：`"velocity"` 或自动化目标键（`gain_db` / `inst:cutoff` / `fx0:mix` …）。 */
  targetKey: string;
  /** 可选目标（**外壳已收窄**；力度由本组件自己加在最前面）。 */
  targets: AutomationTarget[];
  /** 片段在时间轴上的起点（拍）：音符坐标是相对片段的，这里换算成绝对时间。 */
  offsetBeats: number;
  /** 泳道内容高度（条形的最大高度）。 */
  contentHeight: number;
  /** 非 `velocity` 目标时的曲线点（来自 `Track.automation`）。 */
  curvePoints: AutomationPoint[];
}>();

const emit = defineEmits<{
  (e: "update:targetKey", value: string): void;
  (e: "select", indices: number[]): void;
  (e: "apply", patch: Partial<Note>): void;
  (e: "delete", indices: number[]): void;
  (e: "applyCurve", points: AutomationPoint[]): void;
  /** 下拉底部的「＋ 添加参数…」：本组件不认识工程，交回外壳去打开参数页。 */
  (e: "browse"): void;
}>();

const timeline = useTimelineStore();
const layout = useLayoutStore();
const i18n = useI18n();

/** 当前选中目标的定义；`velocity` 由本组件自己提供，不在 `props.targets` 里。 */
const curveTarget = computed(() => props.targets.find((t) => t.key === props.targetKey) ?? null);

/** 刻度列极值：非 `velocity` 目标取自它的值域，`力度` 仍是 0 / 127。 */
const scaleMin = computed(() => curveTarget.value?.min ?? spec.min);
const scaleMax = computed(() => curveTarget.value?.max ?? spec.max);

/** 下拉选项：`力度` 单独一组（逐音符），其余按 `props.targets` 的 `group` 分组。 */
const chooserOptions = computed(() => [
  {
    id: "velocity",
    label: i18n.t("lane.velocity"),
    hint: i18n.t("lane.velocityHint"),
    group: i18n.t("auto.group.note"),
  },
  ...props.targets.map((t) => ({ id: t.key, label: t.label, group: t.group })),
]);

function onChoose(id: string): void {
  emit("update:targetKey", id);
}

/** 「＋ 添加参数…」：交回外壳（`EditView`）打开参数页。 */
function onBrowse(): void {
  emit("browse");
}

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
  const { min, max } = spec;
  const ratio = (value - min) / (max - min || 1);
  return Math.max(2, Math.min(1, Math.max(0, ratio)) * props.contentHeight);
}

/** 渲染用数值：拖动中的已选音符显示预览值。 */
function valueOf(index: number, note: Note): number {
  if (dragValue.value !== null && props.selectedIndices.includes(index)) return dragValue.value;
  return spec.get(note);
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
  if (!el) return spec.min;
  const rect = el.getBoundingClientRect();
  const usable = Math.max(1, rect.height - 6);
  const ratio = Math.min(1, Math.max(0, 1 - (clientY - rect.top - 3) / usable));
  const { min, max, step } = spec;
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
    emit("apply", spec.patch(value));
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
  const value = dragValue.value ?? spec.get(primary);
  return { count: indices.length, value: value.toFixed(spec.digits) };
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
    <!-- 头部：目标选择 + 读数 -->
    <div class="lane-head">
      <!-- 目标选择：下拉（力度 + 该轨道收窄后的插件参数），说明文字在菜单里 -->
      <div class="lane-chooser">
        <Md3Select
          :model-value="targetKey"
          :options="chooserOptions"
          :label="i18n.t('lane.property')"
          :action="{ label: i18n.t('auto.browse') }"
          @select="onChoose"
          @action="onBrowse"
        />
      </div>

      <span v-if="curveTarget" class="readout muted">{{ curveTarget.label }}</span>
      <span v-else-if="readout" class="readout">
        {{ i18n.t("lane.selected", { count: readout.count }) }} · {{ i18n.t("lane.velocity") }}
        <strong class="tabular">{{ readout.value }}</strong>
      </span>
      <span v-else class="readout muted">{{ i18n.t("lane.velocityHint") }}</span>

      <span class="lane-spacer" />

      <!-- 右上角的收起按钮：与编排视图自动化车道的那颗同款（由顶栏开关再打开） -->
      <button
        class="lane-close"
        type="button"
        :aria-label="i18n.t('lane.hide')"
        @click="layout.showControllerLane = false"
      >
        <XMarkIcon class="lane-close-icon" />
      </button>
    </div>

    <!-- 泳道本体：左侧 60px 与卷帘的琴键列对齐 -->
    <div class="lane-row">
      <div class="lane-gutter">
        <span class="max">{{ scaleMax }}</span>
        <span class="min">{{ scaleMin }}</span>
      </div>

      <div class="lane-clip">
        <!-- 力度：逐音符直角条（横位置与宽度与上方音符一一对应） -->
        <div
          v-if="targetKey === 'velocity'"
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
              :title="i18n.t('lane.velocity')"
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

        <!-- 其余目标：与编排视图的自动化车道**同一份数据**的曲线（左侧 60px 已有刻度列，故 insetPx = 0） -->
        <AutomationCurve
          v-if="curveTarget"
          :points="curvePoints"
          :min="curveTarget.min"
          :max="curveTarget.max"
          :step="curveTarget.step"
          :digits="curveTarget.digits"
          :unit="curveTarget.unit"
          :height="contentHeight + 6"
          :inset-px="0"
          @apply="(p: AutomationPoint[]) => emit('applyCurve', p)"
        />
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
/* 目标下拉：压到与泳道头部同高的小尺寸（`Md3Select` 默认是 34px 的宽药丸） */
.lane-chooser {
  flex: 0 0 auto;
  display: inline-flex;
  min-width: 0;
}
.lane-chooser :deep(.md3-select) {
  width: 120px;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.lane-chooser :deep(.caret) {
  width: 13px;
  height: 13px;
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
/* 把右上角的收起按钮顶到最右 */
.lane-spacer {
  flex: 1 1 auto;
}
/* 收起按钮：与编排视图自动化车道的那颗同款（22px 药丸 + 14px 图标） */
.lane-close {
  flex: 0 0 auto;
  width: 22px;
  height: 22px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.lane-close:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.lane-close:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.lane-close-icon {
  width: 14px;
  height: 14px;
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
/* 曲线目标的样式在 `AutomationCurve` 里（曲线画布整块搬过去了）。 */
</style>
