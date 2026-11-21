<script setup lang="ts">
/**
 * 生成记录里那条**播放进度条**（替代 SUNO 的点赞 / 点踩 / 分享）。
 *
 * 行为与走带一致：
 * - 点一下 / 按住拖 = 从那个位置开始试听（拖动中连续 seek，能「刮着听」）；
 * - 键盘同样可用（`role="slider"`，左右箭头 ±2%，PageUp/Down ±10%，Home/End 到两端）；
 * - 悬停 / 拖动时轨道变粗、露出滑块，平时只是一条安静的细线。
 *
 * 只管**表现与指针**，播放状态全在 `stores/inspire` 里：所以切歌、暂停、播完归零都不用在
 * 这里再写一份状态机。
 */
import { computed, ref } from "vue";
import { useI18n } from "@/i18n";
import { capturePointer, releasePointer } from "@/utils/pointer";
import { formatDuration } from "./format";

const props = defineProps<{
  /** 已播比例（0..1）。 */
  value: number;
  /** 已播秒数。 */
  position: number;
  /** 总长（秒）；`0` = 未知。 */
  total: number;
  /** 正在播放这一首。 */
  active?: boolean;
  /** 装载 / 解码中：进度条先按「不可拖」处理。 */
  busy?: boolean;
}>();

const emit = defineEmits<{ (e: "seek", ratio: number): void }>();

const i18n = useI18n();
const dragging = ref(false);

const pct = computed(() => {
  const value = Number.isFinite(props.value) ? props.value : 0;
  return Math.min(100, Math.max(0, value * 100));
});

function ratioAt(event: PointerEvent, el: HTMLElement): number {
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0) return 0;
  return Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
}

function onPointerDown(event: PointerEvent): void {
  const el = event.currentTarget as HTMLElement;
  if (props.busy) return;
  dragging.value = true;
  capturePointer(el, event.pointerId);
  emit("seek", ratioAt(event, el));
  event.preventDefault();
}

function onPointerMove(event: PointerEvent): void {
  if (!dragging.value) return;
  const el = event.currentTarget as HTMLElement;
  emit("seek", ratioAt(event, el));
}

function onPointerUp(event: PointerEvent): void {
  if (!dragging.value) return;
  dragging.value = false;
  releasePointer(event.currentTarget, event.pointerId);
}

function onKey(event: KeyboardEvent): void {
  const current = Math.min(1, Math.max(0, props.value));
  let next: number | null = null;
  switch (event.key) {
    case "ArrowLeft":
    case "ArrowDown":
      next = current - 0.02;
      break;
    case "ArrowRight":
    case "ArrowUp":
      next = current + 0.02;
      break;
    case "PageDown":
      next = current - 0.1;
      break;
    case "PageUp":
      next = current + 0.1;
      break;
    case "Home":
      next = 0;
      break;
    case "End":
      next = 1;
      break;
    default:
      return;
  }
  event.preventDefault();
  emit("seek", Math.min(1, Math.max(0, next)));
}
</script>

<template>
  <div class="seek" :class="{ active, dragging, busy }">
    <div
      class="track"
      role="slider"
      tabindex="0"
      :aria-label="i18n.t('inspire.seek')"
      :aria-valuemin="0"
      :aria-valuemax="100"
      :aria-valuenow="Math.round(pct)"
      :aria-valuetext="`${formatDuration(position)} / ${formatDuration(total)}`"
      :aria-disabled="busy"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @keydown="onKey"
    >
      <span class="rail" aria-hidden="true">
        <span class="fill" :style="{ width: `${pct}%` }" />
      </span>
      <span class="knob" :style="{ left: `${pct}%` }" aria-hidden="true" />
    </div>
    <span class="clock">
      {{ formatDuration(position) }}<span class="clock-sep"> / </span>{{ formatDuration(total) }}
    </span>
  </div>
</template>

<style scoped>
.seek {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
  /* 列表在宽窗口上能有 1200px 宽：进度条铺满整行会让「已播到哪」离标题和时钟太远，
     用户得横着扫一眼才看得懂。收在一个舒服的宽度里，右边留白。 */
  max-width: 640px;
}

/* 轨道：热区比视觉高度大（14px），细线居中；这样点起来不费劲 */
.track {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
  height: 14px;
  display: flex;
  align-items: center;
  border-radius: 999px;
  cursor: pointer;
  touch-action: none;
}
.track:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}

.rail {
  position: relative;
  display: block;
  width: 100%;
  height: 4px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface) / 0.14);
  overflow: hidden;
  transition: height var(--md-duration-short-2) var(--md-ease-standard);
}
.fill {
  display: block;
  height: 100%;
  border-radius: inherit;
  background-color: rgb(var(--md-primary));
  /* 播放中不跟着 timeupdate 抖：宽度由 rAF 每帧更新，这里只做过渡的兜底 */
  transition: width 80ms linear;
}
.seek.active .rail,
.seek:hover .rail,
.seek.dragging .rail {
  height: 6px;
}

.knob {
  position: absolute;
  top: 50%;
  width: 11px;
  height: 11px;
  margin-left: -5.5px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  box-shadow: 0 1px 3px rgb(0 0 0 / 0.35);
  transform: translateY(-50%) scale(0);
  transform-origin: center;
  transition: transform var(--md-duration-short-2) var(--md-ease-spring-fast);
  pointer-events: none;
}
.seek:hover .knob,
.seek.dragging .knob,
.track:focus-visible .knob {
  transform: translateY(-50%) scale(1);
}

.clock {
  flex: 0 0 auto;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.seek.active .clock {
  color: rgb(var(--md-on-surface-variant));
}
.clock-sep {
  opacity: 0.5;
}

.seek.busy .track {
  cursor: progress;
}

@media (prefers-reduced-motion: reduce) {
  .rail,
  .fill,
  .knob {
    transition: none;
  }
}
</style>
