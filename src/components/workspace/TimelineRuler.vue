<script setup lang="ts">
/**
 * 共用时间轴标尺：**编排视图与编辑视图用的是同一个组件、同一条时间轴**。
 *
 * - 小节号按拍号推导（4/4 → 每 4 拍一小节，6/8 → 每 3 拍一小节），与网格线严格对齐；
 * - 点击 / 按住拖动 → 定位播放头；正在播放时从新位置继续（这就是「在对应位置开始播放」）；
 * - 顶部画一根播放头，与下方网格里的播放头同一横坐标。
 *
 * 组件本身不带左侧留白：调用方负责让它与网格内容的横坐标原点对齐
 * （编辑视图里那 60px 琴键列由父容器单独占位）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useProjectStore } from "@/stores/project";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import { capturePointer } from "@/utils/pointer";

const project = useProjectStore();
const timeline = useTimelineStore();
const transport = useTransportStore();

const el = ref<HTMLElement | null>(null);
const scrubbing = ref(false);

/** 指针 x → 拍（`el` 的左边缘就是工程起点）。 */
function beatAt(clientX: number): number {
  const node = el.value;
  if (!node) return 0;
  const rect = node.getBoundingClientRect();
  return timeline.snapBeats((clientX - rect.left) / timeline.zoom);
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
  scrubbing.value = true;
  void transport.seekBeats(beatAt(e.clientX));
}

function onPointerMove(e: PointerEvent): void {
  if (!scrubbing.value) return;
  void transport.seekBeats(beatAt(e.clientX));
}

function onPointerUp(): void {
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
    aria-label="时间轴：点击或拖动定位播放头"
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

    <span class="ruler-playhead" :style="{ left: `${playheadPx}px` }" />
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
.ruler-playhead {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  margin-left: -1px;
  background-color: rgb(var(--md-primary));
  pointer-events: none;
}
/*
 * 播放头顶端的箭头：只有一条细线时很难看出它落在哪一拍，加个箭头一眼就能指到位置。
 * 用边框拼三角形（不依赖字体与图标资源）。
 */
.ruler-playhead::before {
  content: "";
  position: absolute;
  top: 0;
  left: 50%;
  width: 0;
  height: 0;
  transform: translateX(-50%);
  border-left: 5px solid transparent;
  border-right: 5px solid transparent;
  border-top: 9px solid rgb(var(--md-primary));
}
/* 关键点徽标：显示该点的 BPM 与拍号；右键删除。 */
.kf-mark {
  position: absolute;
  top: 2px;
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
