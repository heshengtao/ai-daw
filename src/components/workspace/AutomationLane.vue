<script setup lang="ts">
/**
 * 自动化曲线编辑器：编排视图底部的一条车道（对标钢琴卷帘下方的控制器泳道）。
 *
 * 编辑对象是**当前选中轨道**上的一条参数曲线（`gain_db` / `pan`）。语义与引擎完全一致：
 * - 关键点按拍存放，两点之间**线性插值**，首点之前 / 末点之后**夹住**最近点的值；
 * - 有曲线的参数**覆盖**推子值（读取模式）；
 * - 空曲线 = 删除该车道，参数回到推子值。
 *
 * 横向与编排视图共用同一条时间轴：坐标原点与网格一样从 `TIMELINE_INSET_PX` 起算并按
 * 共享滚动位置平移，所以曲线的小节线严格压在编排网格的小节线上。
 *
 * 交互（一次拖拽只提交一次 → 一条撤销记录）：
 * - 在空白处点击 / 拖动 → 在该处加一个点并开始拖动；
 * - 拖动已有的点 → 改拍与值（拍吸附到网格，值取整到参数步长）；
 * - 右键点点 → 删除该点；
 * - 表头的「清空」→ 删掉整条曲线。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import { capturePointer } from "@/utils/pointer";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import { useLayoutStore } from "@/stores/layout";
import type { AutomationPoint } from "@/api/types";

/** 时间轴在 T=0 左侧预留的空白（与编排视图一致，曲线才能与网格对齐）。 */
const TIMELINE_INSET_PX = 26;
/** 曲线上下各留的内边距（px）：极值处的点不会被裁掉。 */
const PAD = 9;

const props = defineProps<{ height: number }>();

/** 可自动化的参数（与引擎 `LaneParam` 一致）。 */
type Param = "gain_db" | "pan";

interface Spec {
  /** i18n key（渲染时翻译）。 */
  labelKey: string;
  min: number;
  max: number;
  /** 取值步长。 */
  step: number;
  /** 读数小数位。 */
  digits: number;
  /** 显示单位（空字符串 = 无）。 */
  unit: string;
}

/** 取值范围与混音台一致：增益 dB、声像 -1..1。 */
const SPECS: Record<Param, Spec> = {
  gain_db: { labelKey: "auto.gain", min: -60, max: 12, step: 0.5, digits: 1, unit: "dB" },
  pan: { labelKey: "auto.pan", min: -1, max: 1, step: 0.05, digits: 2, unit: "" },
};
const PARAMS: Param[] = ["gain_db", "pan"];

const store = useProjectStore();
const selection = useSelectionStore();
const timeline = useTimelineStore();
const transport = useTransportStore();
const layout = useLayoutStore();
const i18n = useI18n();

const param = ref<Param>("gain_db");
const spec = computed(() => SPECS[param.value]);

const track = computed(() => selection.track);
const trackId = computed(() => track.value?.id ?? null);

/** 工程里该参数的车道（升序）。 */
const stored = computed<AutomationPoint[]>(() => {
  const lane = track.value?.automation?.find((l) => l.param === param.value);
  return lane?.points ?? [];
});

/** 拖动中的草稿（保持升序）；松手才提交。 */
const draft = ref<AutomationPoint[] | null>(null);
const shown = computed(() => draft.value ?? stored.value);

const bodyEl = ref<HTMLElement | null>(null);

const chooserOptions = computed(() =>
  PARAMS.map((p) => ({ id: p, label: i18n.t(SPECS[p].labelKey) })),
);

// ---------------------------------------------------------------------------
// 坐标换算
// ---------------------------------------------------------------------------

/** 参数值 → 车道内的 y（px，顶部为最大值）。 */
function yOf(value: number): number {
  const { min, max } = spec.value;
  const ratio = (value - min) / (max - min || 1);
  const usable = Math.max(1, props.height - PAD * 2);
  return PAD + (1 - Math.min(1, Math.max(0, ratio))) * usable;
}

/** 指针 y → 参数值（取整到步长，并夹在区间内）。 */
function valueFromY(clientY: number): number {
  const el = bodyEl.value;
  if (!el) return spec.value.min;
  const rect = el.getBoundingClientRect();
  const usable = Math.max(1, props.height - PAD * 2);
  const ratio = Math.min(1, Math.max(0, 1 - (clientY - rect.top - PAD) / usable));
  const { min, max, step } = spec.value;
  const raw = min + ratio * (max - min);
  const snapped = Math.round(raw / step) * step;
  return Math.min(max, Math.max(min, snapped));
}

/** 指针 x → 拍（已含预留空白与滚动；吸附到网格）。 */
function beatAt(clientX: number): number {
  const el = bodyEl.value;
  if (!el) return 0;
  const rect = el.getBoundingClientRect();
  const beat = (clientX - rect.left - TIMELINE_INSET_PX + timeline.scrollLeft) / timeline.zoom;
  return Math.max(0, timeline.snapBeats(beat));
}

/** 在折线上求值（镜像引擎 `lane_value_at`：线性插值、两端夹住）。 */
function laneValueAt(points: AutomationPoint[], beat: number): number {
  const first = points[0]!;
  if (beat <= first.beat) return first.value;
  const last = points[points.length - 1]!;
  if (beat >= last.beat) return last.value;
  for (let i = 0; i + 1 < points.length; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (beat >= a.beat && beat <= b.beat) {
      const span = b.beat - a.beat;
      if (span <= 1e-9) return b.value;
      return a.value + (b.value - a.value) * ((beat - a.beat) / span);
    }
  }
  return last.value;
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

const canvasStyle = computed(() => ({
  left: `${TIMELINE_INSET_PX}px`,
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
  const s = spec.value;
  return `${v.toFixed(s.digits)}${s.unit ? ` ${s.unit}` : ""}`;
}

/** 刻度列上的极值标签（短一些，塞得进 26px 的留白）。 */
function scaleLabel(v: number): string {
  return spec.value.digits === 1 ? v.toFixed(0) : v.toFixed(1);
}

function pointTitle(p: AutomationPoint): string {
  return `${p.beat.toFixed(2)} · ${formatValue(p.value)}`;
}

/** 播放头处的曲线值（有曲线时给一个即时读数）。 */
const valueAtPlayhead = computed(() => {
  const pts = shown.value;
  if (pts.length === 0) return null;
  return laneValueAt(pts, transport.positionBeats);
});

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
 * 所以进出这条车道的每个点都要先摊平成普通对象。
 */
function plain(p: AutomationPoint): AutomationPoint {
  return { beat: p.beat, value: p.value };
}

function sortedCopy(points: AutomationPoint[]): AutomationPoint[] {
  return points.map(plain).sort((a, b) => a.beat - b.beat);
}

function onChoose(id: string): void {
  draft.value = null;
  mode.value = null;
  param.value = id as Param;
}

function onBodyPointerDown(e: PointerEvent): void {
  if (e.button !== 0 || !track.value) return;
  if ((e.target as HTMLElement).closest(".auto-point")) return;
  const rect = bodyEl.value?.getBoundingClientRect();
  // 左侧留白是刻度列，不是时间轴——别在那儿加点。
  if (!rect || e.clientX < rect.left + TIMELINE_INSET_PX) return;
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
  if (e.button !== 0 || !track.value) return;
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

async function onPointerUp(): Promise<void> {
  if (mode.value !== "move") return;
  mode.value = null;
  const draftPoints = draft.value;
  draft.value = null;
  dragIndex.value = -1;
  const id = trackId.value;
  if (id === null || draftPoints === null) return;
  const points = draftPoints.map(plain);
  try {
    await store.setAutomation(id, param.value, points);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

async function onPointContextMenu(e: MouseEvent, index: number): Promise<void> {
  e.preventDefault();
  e.stopPropagation();
  const id = trackId.value;
  if (id === null) return;
  const points = sortedCopy(shown.value);
  points.splice(index, 1);
  try {
    await store.setAutomation(id, param.value, points);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

async function clearLane(): Promise<void> {
  const id = trackId.value;
  if (id === null) return;
  try {
    await store.setAutomation(id, param.value, []);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
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
  <div class="auto-lane">
    <!-- 表头：参数选择 + 读数 + 清空 / 收起 -->
    <div class="auto-head">
      <div class="auto-chooser">
        <Md3Select
          :model-value="param"
          :options="chooserOptions"
          :label="i18n.t('auto.param')"
          @select="onChoose"
        />
      </div>
      <span class="auto-name text-label-sm">{{ i18n.t("auto.title") }}</span>

      <span v-if="!track" class="auto-readout muted">{{ i18n.t("auto.noTrack") }}</span>
      <span v-else-if="shown.length === 0" class="auto-readout muted">{{ i18n.t("auto.hint") }}</span>
      <span v-else class="auto-readout">
        {{ i18n.t("auto.points", { count: shown.length }) }}
        <template v-if="valueAtPlayhead !== null">
          · {{ i18n.t("auto.atPlayhead") }}
          <strong class="tabular">{{ formatValue(valueAtPlayhead) }}</strong>
        </template>
      </span>

      <span class="auto-spacer" />

      <button v-if="track && shown.length > 0" class="auto-btn" @click="clearLane">
        {{ i18n.t("auto.clear") }}
      </button>
      <button
        class="auto-btn icon"
        type="button"
        :aria-label="i18n.t('auto.hide')"
        @click="layout.showAutomation = false"
      >
        <XMarkIcon class="auto-icon" />
      </button>
    </div>

    <!-- 车道本体：左侧 26px 刻度列，右侧曲线画布 -->
    <div
      ref="bodyEl"
      class="auto-body"
      :class="{ empty: !track }"
      :style="{ height: `${props.height}px` }"
      @pointerdown="onBodyPointerDown"
      @pointermove="onPointerMove"
      @contextmenu.prevent
    >
      <div class="auto-scale" aria-hidden="true">
        <span>{{ scaleLabel(spec.max) }}</span>
        <span>{{ scaleLabel(spec.min) }}</span>
      </div>

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
  </div>
</template>

<style scoped>
.auto-lane {
  flex: 0 0 auto;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background-color: var(--app-card);
  border-top: 1px solid var(--grid-line-strong);
  user-select: none;
  -webkit-user-select: none;
}
.auto-head {
  height: 30px;
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
/* 参数下拉：压到与表头同高的小尺寸（`Md3Select` 默认是 34px 的宽药丸） */
.auto-chooser {
  flex: 0 0 auto;
  display: inline-flex;
  min-width: 0;
}
.auto-chooser :deep(.md3-select) {
  width: 128px;
  height: 22px;
  padding: 0 8px 0 10px;
  gap: 6px;
  font-size: 11px;
}
.auto-chooser :deep(.caret) {
  width: 13px;
  height: 13px;
}
.auto-name {
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.auto-readout {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.auto-readout.muted {
  color: rgb(var(--md-on-surface-muted));
}
.auto-readout strong {
  color: rgb(var(--md-on-surface));
  margin-left: 4px;
}
.auto-spacer {
  flex: 1 1 auto;
}
.auto-btn {
  flex: 0 0 auto;
  height: 22px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.auto-btn:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.auto-btn.icon {
  width: 22px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.auto-icon {
  width: 14px;
  height: 14px;
}
/* 车道本体 */
.auto-body {
  position: relative;
  overflow: hidden;
  background-color: var(--app-card);
  cursor: crosshair;
}
.auto-body.empty {
  cursor: default;
}
/* 左侧刻度列：宽度 = 编排视图留白，槽位与上方 lanes 的留白连成一条 */
.auto-scale {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  width: 26px;
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
/* 画布：与编排网格同一原点、同一滚动位置 */
.auto-canvas {
  position: absolute;
  top: 0;
  bottom: 0;
}
.auto-body:not(.empty) .auto-canvas {
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
.tabular {
  font-variant-numeric: tabular-nums;
}
</style>
