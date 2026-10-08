<script setup lang="ts">
/**
 * 实时波形屏（示波器）。
 *
 * 数据来自音频回调推入的无锁环形缓冲（`scope_data`）。用 `requestAnimationFrame` 轮询：
 * 约 30 Hz，够看起来连续，又不会把 IPC 打满。
 *
 * 空闲（没有声音）时画一条平静的中线——屏幕是「亮着但没信号」，不是空白。
 */
import { onMounted, onUnmounted, ref } from "vue";
import { scopeData } from "@/api/tauri";

const props = withDefaults(
  defineProps<{
    /** 画多少个采样点。 */
    points?: number;
    height?: number;
  }>(),
  { points: 1024, height: 132 },
);

const canvasEl = ref<HTMLCanvasElement | null>(null);

/** 采样（复用同一个数组，避免每帧分配）。 */
let buffer = new Float32Array(props.points);
let raf = 0;
let lastPaint = 0;

/** 从元素上取主题色：canvas 只认具体颜色，不认 CSS 变量。 */
function themeColors(el: HTMLElement): { line: string; grid: string } {
  const styles = getComputedStyle(el);
  const read = (name: string, fallback: string) => {
    const value = styles.getPropertyValue(name).trim();
    return value ? `rgb(${value})` : fallback;
  };
  return {
    line: read("--md-primary", "#33d6c0"),
    grid: read("--md-outline-variant", "#3a3a3a"),
  };
}

function resize(el: HTMLCanvasElement): void {
  const dpr = window.devicePixelRatio || 1;
  const width = el.clientWidth;
  const height = el.clientHeight;
  if (el.width !== Math.round(width * dpr) || el.height !== Math.round(height * dpr)) {
    el.width = Math.round(width * dpr);
    el.height = Math.round(height * dpr);
  }
}

/** 拉一帧数据填进复用数组。 */
async function paint(): Promise<void> {
  const length = await scopeData(props.points);
  if (length.length !== buffer.length) buffer = new Float32Array(length.length);
  buffer.set(length);
}

function draw(el: HTMLCanvasElement): void {
  const ctx = el.getContext("2d");
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const width = el.width / dpr;
  const height = el.height / dpr;
  const { line, grid } = themeColors(el);

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  // 中线 + 上下刻度
  ctx.strokeStyle = grid;
  ctx.globalAlpha = 0.5;
  ctx.lineWidth = 1;
  for (const y of [height / 2, height / 4, (height * 3) / 4]) {
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  // 波形
  const mid = height / 2;
  const scale = height / 2 - 2;
  ctx.strokeStyle = line;
  ctx.lineWidth = 1.6;
  ctx.lineJoin = "round";
  ctx.beginPath();
  const count = buffer.length;
  for (let i = 0; i < count; i += 1) {
    const x = (i / (count - 1)) * width;
    const y = mid - Math.max(-1, Math.min(1, buffer[i])) * scale;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
}

function loop(): void {
  const el = canvasEl.value;
  const now = performance.now();
  // 约 30 Hz：比这更频繁只是白烧 CPU。
  if (el && now - lastPaint > 33) {
    lastPaint = now;
    resize(el);
    void paint().then(() => draw(el));
  }
  raf = requestAnimationFrame(loop);
}

onMounted(() => {
  raf = requestAnimationFrame(loop);
});
onUnmounted(() => cancelAnimationFrame(raf));
</script>

<template>
  <div class="scope" :style="{ height: `${height}px` }">
    <canvas ref="canvasEl" :aria-label="'output waveform'" />
  </div>
</template>

<style scoped>
.scope {
  position: relative;
  flex: 0 0 auto;
  width: 100%;
  border-radius: 14px;
  /* 屏幕该是「黑的那块」：比面板底色更深 */
  background-color: rgb(var(--md-scrim) / 0.55);
  border: 1px solid rgb(var(--md-outline-variant) / 0.45);
  overflow: hidden;
}
canvas {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
