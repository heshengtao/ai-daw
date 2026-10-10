<script setup lang="ts">
/**
 * 自动化曲线画布：刻度列 + 网格画布 + 折线（两端补抽头）+ 可拖动的关键点 + 播放头。
 *
 * 从编排视图的自动化车道里整块搬出来，两条车道共用：编排视图底部的自动化车道（原点 26px 的
 * 留白）与钢琴卷帘下方的参数车道（原点 0）。组件本身**不认识轨道与工程**——它只画一组
 * `points`，把编辑结果通过 `apply` 交回外壳去提交。
 *
 * 语义与引擎完全一致：
 * - 关键点按拍存放，两点之间**线性插值**，首点之前 / 末点之后**夹住**最近点的值；
 * - 横向与调用方共用的时间轴（`stores/timeline`）对齐：坐标原点从 `insetPx` 起算并按共享滚动
 *   位置平移，所以曲线的小节线严格压在编排网格的小节线上。
 *
 * 交互（一次拖拽只提交一次 → 一条撤销记录）：
 * - 在空白处点击 / 拖动 → 在该处加一个点并开始拖动；
 * - 拖动已有的点 → 改拍与值（拍吸附到网格，值取整到参数步长）；
 * - 右键点点 → 删除该点。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { capturePointer } from "@/utils/pointer";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import type { AutomationPoint } from "@/api/types";

/** 曲线上下各留的内边距（px）：极值处的点不会被裁掉。 */
const PAD = 9;

const props = defineProps<{
  points: AutomationPoint[];
  min: number;
  max: number;
  step: number;
  digits: number;
  unit?: string;
  /** 车道本体高度（px）。 */
  height: number;
  /**
   * 画布左边缘相对第 0 拍的偏移：编排 26（TIMELINE_INSET_PX），卷帘车道 0。
   * 为 0 时不画自带的刻度列（那类宿主的刻度列在自己那边）。
   */
  insetPx: number;
}>();

const emit = defineEmits<{ (e: "apply", points: AutomationPoint[]): void }>();

const timeline = useTimelineStore();
const transport = useTransportStore();

/** 拖动中的草稿（升序）；松手才 emit。 */
const draft = ref<AutomationPoint[] | null>(null);
const shown = computed(() => draft.value ?? props.points);

const bodyEl = ref<HTMLElement | null>(null);

// ---------------------------------------------------------------------------
// 坐标换算
// ---------------------------------------------------------------------------

/** 参数值 → 车道内的 y（px，顶部为最大值）。 */
function yOf(value: number): number {
  const ratio = (value - props.min) / (props.max - props.min || 1);
  const usable = Math.max(1, props.height - PAD * 2);
  return PAD + (1 - Math.min(1, Math.max(0, ratio))) * usable;
}

/** 指针 y → 参数值（取整到步长，并夹在区间内）。 */
function valueFromY(clientY: number): number {
  const el = bodyEl.value;
  if (!el) return props.min;
  const rect = el.getBoundingClientRect();
  const usable = Math.max(1, props.height - PAD * 2);
  const ratio = Math.min(1, Math.max(0, 1 - (clientY - rect.top - PAD) / usable));
  const raw = props.min + ratio * (props.max - props.min);
  const snapped = Math.round(raw / props.step) * props.step;
  return Math.min(props.max, Math.max(props.min, snapped));
}

/** 指针 x → 拍（已含预留空白与滚动；吸附到网格）。 */
function beatAt(clientX: number): number {
  const el = bodyEl.value;
  if (!el) return 0;
  const rect = el.getBoundingClientRect();
  const beat = (clientX - rect.left - props.insetPx + timeline.scrollLeft) / timeline.zoom;
  return Math.max(0, timeline.snapBeats(beat));
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

const canvasStyle = computed(() => ({
  left: `${props.insetPx}px`,
  width: `${timeline.contentWidth}px`,
  transform: `translateX(${-timeline.scrollLeft}px)`,
  ...timeline.gridStyle,
}));

/**
 * 折线的绘制点：**在两端各补一个抽头**。
 *
 * 引擎在首点之前 / 末点之后按最近点夹住（水平直线），所以补上 0 拍与时间轴末端两个点，
 * 画出来的形状才和实际听到的一致。
 */
const linePoints = computed(() => {
  const pts = shown.value;
  if (pts.length === 0) return [] as { x: number; y: number }[];
  const z = timeline.zoom;
  const out: { x: number; y: number }[] = [{ x: 0, y: yOf(pts[0]!.value) }];
  for (const p of pts) out.push({ x: p.beat * z, y: yOf(p.value) });
  out.push({ x: timeline.totalBeats * z, y: yOf(pts[pts.length - 1]!.value) });
  return out;
});

const polyline = computed(() => linePoints.value.map((p) => `${p.x},${p.y}`).join(" "));

function pointStyle(p: AutomationPoint): Record<string, string> {
  return { left: `${p.beat * timeline.zoom}px`, top: `${yOf(p.value)}px` };
}

function formatValue(v: number): string {
  return `${v.toFixed(props.digits)}${props.unit ? ` ${props.unit}` : ""}`;
}

/** 刻度列上的极值标签（短一些，窄留白里也塞得进）。 */
function scaleLabel(v: number): string {
  return props.digits === 1 ? v.toFixed(0) : v.toFixed(1);
}

function pointTitle(p: AutomationPoint): string {
  return `${p.beat.toFixed(2)} · ${formatValue(p.value)}`;
}

// ---------------------------------------------------------------------------
// 指针
// ---------------------------------------------------------------------------

const mode = ref<"move" | null>(null);
/** 正在拖动的点在 `draft` 中的下标。 */
const dragIndex = ref(-1);

/**
 * 关键点的**纯数据副本**。
 *
 * 工程数据是 Vue 的深层响应式代理：若直接把它的点放进拖动草稿，拖动就等于**就地改写工程**
 * （绕过命令与撤销历史），而且代理对象无法被 `structuredClone`（浏览器预览的 mock 会炸）。
 * 所以进出曲线的每个点都要先摊平成普通对象。
 */
function plain(p: AutomationPoint): AutomationPoint {
  return { beat: p.beat, value: p.value };
}

function sortedCopy(points: AutomationPoint[]): AutomationPoint[] {
  return points.map(plain).sort((a, b) => a.beat - b.beat);
}

function onBodyPointerDown(e: PointerEvent): void {
  if (e.button !== 0) return;
  if ((e.target as HTMLElement).closest(".auto-point")) return;
  const rect = bodyEl.value?.getBoundingClientRect();
  // 左侧留白是刻度列，不是时间轴——别在那儿加点。
  if (!rect || e.clientX < rect.left + props.insetPx) return;
  capturePointer(e.currentTarget, e.pointerId);
  const beat = beatAt(e.clientX);
  const value = valueFromY(e.clientY);
  const points = sortedCopy(shown.value);
  // 落在已有点的拍上就复用它（避免同拍重复点），否则按拍序插入。
  const at = points.findIndex((p) => Math.abs(p.beat - beat) < 1e-6);
  let index: number;
  if (at >= 0) {
    points[at] = { beat, value };
    index = at;
  } else {
    const inserted = points.findIndex((p) => p.beat > beat);
    index = inserted < 0 ? points.length : inserted;
    points.splice(index, 0, { beat, value });
  }
  draft.value = points;
  dragIndex.value = index;
  mode.value = "move";
}

function onPointPointerDown(e: PointerEvent, index: number): void {
  if (e.button !== 0) return;
  e.stopPropagation();
  capturePointer(e.currentTarget, e.pointerId);
  draft.value = sortedCopy(shown.value);
  dragIndex.value = index;
  mode.value = "move";
}

function onPointerMove(e: PointerEvent): void {
  if (mode.value !== "move" || !draft.value) return;
  const moved = draft.value[dragIndex.value];
  if (!moved) return;
  moved.beat = beatAt(e.clientX);
  moved.value = valueFromY(e.clientY);
  // 位置可能越过邻居：整理顺序后按对象引用重新定位。
  draft.value.sort((a, b) => a.beat - b.beat);
  dragIndex.value = draft.value.indexOf(moved);
}

/** 松手：把草稿摊平后交给外壳提交（提交失败由外壳提示）。 */
function onPointerUp(): void {
  if (mode.value !== "move") return;
  mode.value = null;
  const draftPoints = draft.value;
  draft.value = null;
  dragIndex.value = -1;
  if (draftPoints === null) return;
  emit("apply", draftPoints.map(plain));
}

function onPointContextMenu(e: MouseEvent, index: number): void {
  e.preventDefault();
  e.stopPropagation();
  const points = sortedCopy(shown.value);
  points.splice(index, 1);
  emit("apply", points);
}

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
  <div
    ref="bodyEl"
    class="auto-body"
    :style="{ height: `${props.height}px` }"
    @pointerdown="onBodyPointerDown"
    @pointermove="onPointerMove"
    @contextmenu.prevent
  >
    <!-- 刻度列：宽度 = 左侧留白，槽位与上方 lanes 的留白连成一条（没有留白就不画） -->
    <div
      v-if="props.insetPx > 0"
      class="auto-scale"
      :style="{ width: `${props.insetPx}px` }"
      aria-hidden="true"
    >
      <span>{{ scaleLabel(props.max) }}</span>
      <span>{{ scaleLabel(props.min) }}</span>
    </div>

    <!-- 画布：与调用方网格同一原点、同一滚动位置 -->
    <div class="auto-canvas" :style="canvasStyle">
      <svg class="auto-svg" :width="timeline.contentWidth" :height="props.height" aria-hidden="true">
        <polyline v-if="linePoints.length > 1" class="auto-line" :points="polyline" />
      </svg>

      <div
        v-for="(p, i) in shown"
        :key="i"
        class="auto-point"
        :class="{ dragging: mode === 'move' && i === dragIndex }"
        :style="pointStyle(p)"
        :title="pointTitle(p)"
        @pointerdown="onPointPointerDown($event, i)"
        @contextmenu="onPointContextMenu($event, i)"
      />

      <div class="auto-playhead" :style="{ left: `${transport.positionBeats * timeline.zoom}px` }" />
    </div>
  </div>
</template>

<style scoped>
.auto-body {
  position: relative;
  overflow: hidden;
  background-color: var(--app-card);
  cursor: crosshair;
}
/* 刻度列：宽度内联取自 `insetPx`（= 宿主留白的宽度），槽位与宿主的留白连成一条 */
.auto-scale {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  align-items: flex-end;
  padding: 3px 4px 3px 0;
  font-size: 8.5px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
  border-right: 1px solid var(--grid-line-row);
  pointer-events: none;
}
.auto-canvas {
  position: absolute;
  top: 0;
  bottom: 0;
  cursor: crosshair;
}
.auto-svg {
  position: absolute;
  top: 0;
  left: 0;
  pointer-events: none;
  overflow: visible;
}
.auto-line {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
}
/* 关键点：小圆点，命中区用透明描边撑大 */
.auto-point {
  position: absolute;
  width: 12px;
  height: 12px;
  margin: -6px 0 0 -6px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-surface));
  cursor: grab;
  touch-action: none;
}
.auto-point:hover,
.auto-point.dragging {
  box-shadow: 0 0 0 2px rgb(var(--md-surface)), 0 0 0 4px rgb(var(--md-primary) / 0.4);
}
.auto-point.dragging {
  cursor: grabbing;
}
.auto-playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  margin-left: -1px;
  background-color: rgb(var(--md-primary) / 0.55);
  pointer-events: none;
}
</style>
