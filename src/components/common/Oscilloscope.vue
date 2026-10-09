<script setup lang="ts">
/**
 * 一个周期的波形屏（教学用）。
 * 数据是按参数解析算出来的一个基频周期，不是真实音频——转旋钮能看到波形本身的变化。
 */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { sampleOnePeriod, type WaveParams } from "@/utils/waveform";

const props = withDefaults(
  defineProps<{ params: WaveParams; points?: number; height?: number }>(),
  { points: 256, height: 150 },
);

const canvasEl = ref<HTMLCanvasElement | null>(null);

function themeColors(el: HTMLElement): { line: string; grid: string } {
  const styles = getComputedStyle(el);
  const read = (name: string, fallback: string) => {
    const v = styles.getPropertyValue(name).trim();
    return v ? `rgb(${v})` : fallback;
  };
  return { line: read("--md-primary", "#6750a4"), grid: read("--md-outline-variant", "#cac4d0") };
}

function draw(): void {
  const el = canvasEl.value;
  if (!el) return;
  const ctx = el.getContext("2d");
  if (!ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const { line, grid } = themeColors(el);

  ctx.strokeStyle = grid;
  ctx.globalAlpha = 0.5;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, h / 2);
  ctx.lineTo(w, h / 2);
  ctx.stroke();
  ctx.globalAlpha = 1;

  const data = sampleOnePeriod(props.params, props.points);
  const mid = h / 2;
  const scale = h / 2 - 3;
  ctx.strokeStyle = line;
  ctx.lineWidth = 1.8;
  ctx.lineJoin = "round";
  ctx.beginPath();
  for (let i = 0; i < data.length; i += 1) {
    const x = (i / (data.length - 1)) * w;
    const y = mid - Math.max(-1, Math.min(1, data[i]!)) * scale;
    if (i === 0) ctx.moveTo(x, y);
    else ctx.lineTo(x, y);
  }
  ctx.stroke();
}

// 波形纯粹是解析算出来的，只在参数或尺寸变化时才会不同——没有动画需求，
// 因此不用 rAF 帧循环，改为「按需重绘」：挂载时画一次、参数变时重画、容器尺寸变时重画。
let ro: ResizeObserver | null = null;

onMounted(() => {
  draw();
  ro = new ResizeObserver(() => draw());
  if (canvasEl.value) ro.observe(canvasEl.value);
});
onUnmounted(() => ro?.disconnect());
watch(() => JSON.stringify(props.params), draw);
</script>

<template>
  <div class="scope" :style="{ height: `${height}px` }">
    <canvas ref="canvasEl" aria-label="一个周期的波形" />
  </div>
</template>

<style scoped>
.scope {
  position: relative;
  width: 100%;
  border-radius: 20px;
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
