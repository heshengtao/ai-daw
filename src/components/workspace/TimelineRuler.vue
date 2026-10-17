<script setup lang="ts">
/**
 * 共用时间轴标尺：**编排视图与编辑视图用的是同一个组件、同一条时间轴**。
 *
 * - 小节号按拍号推导（4/4 → 每 4 拍一小节，6/8 → 每 3 拍一小节），与网格线严格对齐；
 * - 点击 / 按住拖动 → 定位播放头；正在播放时从新位置继续（这就是「在对应位置开始播放」）；
 * - 标尺上还能划出**播放范围**（cycle range）：它是引擎里的循环区间，重绘区间也跟着它走。
 *
 * 播放范围的三种手势（都落在标尺上）：
 * 1. **框选模式下直接拖动** → 划出范围（松手即生效，并自动打开循环）。鼠标模式在顶栏
 *    三段按钮里切（`layout.tool === "select"`），**不再需要按 Shift**；原地点击
 *    （移动不超过 3px）不算划范围，按定位播放头处理。
 * 2. 拖动范围**两端标记** → 改起点 / 终点；
 * 3. **右键范围** → 取消（回到「整首循环」）。
 *
 * 范围带**本身不可拖**（整段平移会抢走压在它上面的播放头拖动——播放头在范围内也随时能拖）；
 * 范围带上的按下交给标尺自己处理，语义与空白处一致。
 *
 * 吸附步长与播放头**同一个**（网格细分 `timeline.snapBeats`）——划范围与拖播放头的精度一致，
 * 不再默认只吸小节线（小节线仍是网格上的点，想卡小节照样卡得上）。
 *
 * 视觉：范围用编辑视图里 `.clip-band` 同一套淡色（「这一段被选中了」在两个视图里长得一样），
 * 两端与播放头是同一形状的**上方形下三角**标记，只靠上下位置区分——时间轴上一共三枚。
 *
 * 组件本身不带左侧留白：调用方负责让它与网格内容的横坐标原点对齐
 * （编辑视图里那 60px 琴键列由父容器单独占位）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useProjectStore } from "@/stores/project";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { capturePointer } from "@/utils/pointer";

const project = useProjectStore();
const timeline = useTimelineStore();
const transport = useTransportStore();
const layout = useLayoutStore();
const i18n = useI18n();

const el = ref<HTMLElement | null>(null);
const scrubbing = ref(false);

/** 指针 x → 拍（`el` 的左边缘就是工程起点）。 */
function beatAt(clientX: number): number {
  const node = el.value;
  if (!node) return 0;
  const rect = node.getBoundingClientRect();
  return timeline.snapBeats((clientX - rect.left) / timeline.zoom);
}

/** 指针 x → 未吸附的拍（播放范围拖动用，吸附另行处理）。 */
function rawBeatAt(clientX: number): number {
  const node = el.value;
  if (!node) return 0;
  const rect = node.getBoundingClientRect();
  return Math.max(0, (clientX - rect.left) / timeline.zoom);
}

// ---- 播放范围（cycle range） ----

/**
 * 框选模式下「点还是拖」的判定阈值（px）：按下后移动不超过这个距离算点击（定位播放头），
 * 超过才算划范围。没有阈值的话，想点一下定位播放头会顺手划出一小段范围。
 */
const SELECT_SLOP_PX = 3;

/** 范围拖动的状态机。`null` = 没在拖。 */
type RangeDrag = {
  mode: "create" | "start" | "end";
  /** `create`：按下点吸附后的拍；`start` / `end` 不用。 */
  anchor: number;
};

const rangeDrag = ref<RangeDrag | null>(null);
/** 框选模式下刚按下、还没跨过判定阈值：先记着，看用户是点还是拖。 */
const pending = ref<{ x: number; beat: number } | null>(null);

const rangeStart = computed(() => transport.loopStartBeats);
const rangeEnd = computed(() => transport.loopEndBeats);
const rangeStartPx = computed(() => rangeStart.value * timeline.zoom);
const rangeWidthPx = computed(() => Math.max(2, (rangeEnd.value - rangeStart.value) * timeline.zoom));

const rangeLabel = computed(() => transport.loopRangeLabel);
/** 范围带上的提示：小节区间 + 手势。 */
const rangeHint = computed(() => i18n.t("transport.rangeHint", { bars: rangeLabel.value }));

/**
 * 标尺 tooltip：没范围时按当前鼠标模式给对应提示——
 * 框选模式告诉用户「怎么划范围」，其余模式是「怎么定位播放头」。
 * （store 属性在 script 里已自动解包，直接取布尔 / 字符串。）
 */
const rulerTitle = computed(() =>
  transport.hasLoopRange
    ? undefined
    : layout.tool === "select"
      ? i18n.t("transport.rangeCreate")
      : i18n.t("transport.seek"),
);

function clearRange(): void {
  void transport.clearLoopRange();
}

/** 范围两端标记上的按下：改起点 / 终点（带子本身不拦事件，交给标尺定位播放头）。 */
function onRangeDown(e: PointerEvent, mode: "start" | "end"): void {
  if (e.button !== 0) return;
  capturePointer(e.currentTarget as HTMLElement, e.pointerId);
  rangeDrag.value = { mode, anchor: 0 };
}

/** 双击标尺非 0 位置 → 在该处插入关键点（初值继承该处生效值）。 */
function addKeyframe(e: MouseEvent): void {
  const beat = beatAt(e.clientX);
  if (beat <= 0) return;
  void project.setKeyframeAt(beat, {});
}

/** 右键关键点徽标 → 删除它。 */
function removeKeyframe(beat: number): void {
  void project.removeKeyframeAt(beat);
}

const keyframes = computed(() => project.project?.transport.keyframes ?? []);

function onPointerDown(e: PointerEvent): void {
  if (e.button !== 0) return;
  capturePointer(e.currentTarget, e.pointerId);
  if (layout.tool === "select") {
    // 框选模式：按下先记着——拖动（跨过 3px 阈值）划范围，原地松手按点击定位播放头。
    // 不立刻写范围：用户可能只是想点一下，原来的范围应当原样留着。
    pending.value = { x: e.clientX, beat: timeline.snapBeats(rawBeatAt(e.clientX)) };
    return;
  }
  scrubbing.value = true;
  void transport.seekBeats(beatAt(e.clientX));
}

function onPointerMove(e: PointerEvent): void {
  // 框选模式：先判定「点还是拖」，跨过阈值才真正开始划范围。
  if (pending.value && !rangeDrag.value) {
    if (Math.abs(e.clientX - pending.value.x) <= SELECT_SLOP_PX) return;
    rangeDrag.value = { mode: "create", anchor: pending.value.beat };
    pending.value = null;
  }
  const drag = rangeDrag.value;
  if (drag) {
    const beat = timeline.snapBeats(rawBeatAt(e.clientX));
    if (drag.mode === "create") {
      // 划到哪就画到哪：从第一次真正移动开始才写范围。
      if (beat !== drag.anchor) {
        void transport.setLoopRange(Math.min(drag.anchor, beat), Math.max(drag.anchor, beat));
      }
      return;
    }
    // 改起止：保留至少一个网格细分的宽度，免得拖成空区间被引擎当成「没范围」。
    const min = timeline.snap;
    if (drag.mode === "start") {
      void transport.setLoopRange(Math.min(beat, rangeEnd.value - min), rangeEnd.value);
    } else {
      void transport.setLoopRange(rangeStart.value, Math.max(beat, rangeStart.value + min));
    }
    return;
  }
  if (!scrubbing.value) return;
  void transport.seekBeats(beatAt(e.clientX));
}

function onPointerUp(): void {
  // 框选模式下没拖够阈值：按点击处理，定位播放头。
  if (pending.value) {
    void transport.seekBeats(pending.value.beat);
    pending.value = null;
  }
  rangeDrag.value = null;
  scrubbing.value = false;
}

const playheadPx = computed(() => transport.positionBeats * timeline.zoom);

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
    ref="el"
    class="ruler"
    :class="{ scrubbing }"
    role="slider"
    :aria-label="i18n.t('transport.seek')"
    :title="rulerTitle"
    :aria-valuenow="Math.round(transport.positionBeats * 100) / 100"
    :aria-valuemin="0"
    tabindex="0"
    @pointerdown="onPointerDown"
    @pointermove="onPointerMove"
    @dblclick="addKeyframe"
    @keydown.left.prevent="transport.seekBeats(Math.max(0, transport.positionBeats - timeline.snap))"
    @keydown.right.prevent="transport.seekBeats(transport.positionBeats + timeline.snap)"
  >
    <span
      v-for="m in timeline.barMarks"
      :key="m.bar"
      class="bar-mark"
      :style="{ width: `${m.width}px` }"
    >
      {{ m.bar }}
    </span>

    <!--
      播放范围：**与编辑视图里 `.clip-band` 同一套淡色**（6% 主色底 + 45% 两侧边线）——
      「这一段是被选中的」在两个视图里长得一样，用户不必重新学一遍。
      两端各一枚可拖的标记（改起止）；**带子本身不可拖**——整段平移会抢走压在它上面的
      播放头拖动（播放头在范围内也随时能拖），所以带子上的按下不拦事件，交给标尺自己
      按「定位播放头 / 框选模式划新范围」处理；右键取消。
    -->
    <div
      v-if="transport.hasLoopRange"
      class="range"
      :style="{ left: `${rangeStartPx}px`, width: `${rangeWidthPx}px` }"
      :title="rangeHint"
      @contextmenu.prevent.stop="clearRange"
    >
      <span class="tick range-tick start" @pointerdown.stop="onRangeDown($event, 'start')" />
      <span class="tick range-tick end" @pointerdown.stop="onRangeDown($event, 'end')" />
    </div>

    <button
      v-for="kf in keyframes"
      :key="kf.beat"
      class="kf-mark"
      :style="{ left: `${kf.beat * timeline.zoom}px` }"
      :title="`${kf.tempo.toFixed(0)} BPM · ${kf.time_sig_num}/${kf.time_sig_den}`"
      @contextmenu.prevent.stop="removeKeyframe(kf.beat)"
    >
      <span class="kf-badge">{{ kf.tempo.toFixed(0) }}</span>
      <span class="kf-meter">{{ kf.time_sig_num }}/{{ kf.time_sig_den }}</span>
    </button>

    <!--
      播放头：与范围标记**同一形状**（上方形下三角），只是整体往下挪——三个标记靠
      上下位置区分，不用换颜色也不用换形状，一眼就能数出「开始 / 结束 / 现在」。
    -->
    <span class="tick playhead-tick" :style="{ left: `${playheadPx}px` }" />
  </div>
</template>

<style scoped>
/* 与音轨栏表头同高（40px）、内凹底色 + 底部分隔线；点击即定位。 */
.ruler {
  position: relative;
  height: 40px;
  flex: 0 0 auto;
  display: flex;
  cursor: pointer;
  background-color: var(--app-inset);
  border-bottom: 1px solid var(--grid-line-strong);
  user-select: none;
  -webkit-user-select: none;
  touch-action: none;
  overflow: hidden;
}
.ruler.scrubbing {
  cursor: ew-resize;
}
.ruler:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: -2px;
}
.bar-mark {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  padding-left: 8px;
  font-size: 10px;
  letter-spacing: 0.5px;
  white-space: nowrap;
  color: rgb(var(--md-on-surface-variant));
  border-left: 1px solid var(--grid-line-strong);
  pointer-events: none;
}
/*
 * 播放范围：**与编辑视图 `.clip-band` 完全同一套淡色**——「这一段被选中了」在两个视图里
 * 长得一样，不必重新学。**带子本身不可拖**（光标继承标尺：定位播放头），只有两端标记
 * 能拖；右键删除在本工程已是既有约定（关键点徽标同理）。
 */
.range {
  position: absolute;
  top: 0;
  bottom: 0;
  z-index: 2;
  background-color: rgb(var(--md-primary) / 0.06);
  border-left: 1px solid rgb(var(--md-primary) / 0.45);
  border-right: 1px solid rgb(var(--md-primary) / 0.45);
  touch-action: none;
}
/*
 * 时间轴标记：**上方形、下三角**（一面朝下插进时间轴的小旗）。
 * 用边框拼三角形，不依赖字体与图标资源。范围两端与播放头共用这一形状——
 * 只靠上下位置区分谁是谁，所以三个标记叠在一起也数得清。
 */
.tick {
  position: absolute;
  width: 12px;
}
.tick::before {
  content: "";
  display: block;
  width: 12px;
  height: 13px;
  background-color: rgb(var(--md-primary));
}
.tick::after {
  content: "";
  display: block;
  width: 0;
  height: 0;
  border-left: 6px solid transparent;
  border-right: 6px solid transparent;
  border-top: 9px solid rgb(var(--md-primary));
}
/* 范围两端：贴顶边，中心对准范围边线（`-6px` = 半个标记宽）。按下时换成 grabbing。 */
.range-tick {
  top: 0;
  z-index: 3;
  cursor: ew-resize;
}
.range-tick:active {
  cursor: grabbing;
}
.range-tick.start {
  left: -6px;
}
.range-tick.end {
  right: -6px;
}
/* 播放头：同一形状，整体往下挪 18px，三角尖正好落在标尺下边缘——
   与范围标记错开一行，三个标记同时出现时不打架。 */
.playhead-tick {
  top: 18px;
  margin-left: -6px;
  z-index: 5;
  pointer-events: none;
}
/* 关键点徽标：显示该点的 BPM 与拍号；右键删除。压在范围带与标记之上（信息比标记重要）。 */
.kf-mark {
  position: absolute;
  top: 2px;
  z-index: 4;
  transform: translateX(-50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  padding: 2px 6px;
  border: 0;
  border-radius: 8px;
  background-color: rgb(var(--md-tertiary-container));
  color: rgb(var(--md-on-tertiary-container));
  font-size: 9.5px;
  line-height: 1.1;
  cursor: pointer;
}
.kf-mark .kf-meter {
  font-size: 8px;
  opacity: 0.75;
}
</style>
