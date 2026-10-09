<script setup lang="ts">
/**
 * 音频片段的波形：把 `audio_waveform` 回来的 min/max 包络画成一列列竖线。
 *
 * canvas 骨架照抄 `Oscilloscope.vue`（devicePixelRatio + CSS 变量取色 + ResizeObserver 按需重绘）。
 * 包络按**路径**缓存（固定 512 桶）：拖动/缩放片段时只重画，不再向后端请求。
 */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { audioWaveform } from "@/api/tauri";

/** 固定桶数：换宽度只影响绘制，不影响请求，因此拖拽时不会反复打后端。 */
const BUCKETS = 512;

const props = defineProps<{ path: string }>();

const canvasEl = ref<HTMLCanvasElement | null>(null);
const peaks = ref<number[]>([]);

/** 路径 → 包络（`[min, max, min, max, …]`）。 */
const cache = new Map<string, number[]>();

function themeColor(el: HTMLElement, name: string, fallback: string): string {
  const v = getComputedStyle(el).getPropertyValue(name).trim();
  return v ? `rgb(${v})` : fallback;
}

/** 波形颜色：优先用宿主给的 `--wave-color`（片段里按轨道色系），否则退回主色。 */
function waveColor(el: HTMLElement): string {
  const styles = getComputedStyle(el);
  const custom = styles.getPropertyValue("--wave-color").trim();
  if (custom) return custom;
  return themeColor(el, "--md-primary", "#6750a4");
}

function draw(): void {
  const el = canvasEl.value;
  if (!el) return;
  const ctx = el.getContext("2d");
  if (!ctx) return;
  const w = el.clientWidth;
  const h = el.clientHeight;
  if (w === 0 || h === 0) return;

  const dpr = window.devicePixelRatio || 1;
  if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
    el.width = Math.round(w * dpr);
    el.height = Math.round(h * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  const data = peaks.value;
  if (data.length < 2) return;
  const buckets = Math.floor(data.length / 2);
  const mid = h / 2;
  const scale = h / 2 - 1;
  ctx.fillStyle = waveColor(el);
  for (let x = 0; x < w; x += 1) {
    const b = Math.min(buckets - 1, Math.floor((x / w) * buckets));
    const lo = data[b * 2] ?? 0;
    const hi = data[b * 2 + 1] ?? 0;
    const top = mid - Math.max(-1, Math.min(1, hi)) * scale;
    const bottom = mid - Math.max(-1, Math.min(1, lo)) * scale;
    ctx.fillRect(x, top, 1, Math.max(1, bottom - top));
  }
}

async function load(): Promise<void> {
  const cached = cache.get(props.path);
  if (cached) {
    peaks.value = cached;
    draw();
    return;
  }
  try {
    const data = await audioWaveform(props.path, BUCKETS);
    cache.set(props.path, data);
    peaks.value = data;
  } catch {
    // 素材缺失/不可解码：留白。片段本身仍然存在（后端按静音处理）。
    peaks.value = [];
  }
  draw();
}

let ro: ResizeObserver | null = null;
onMounted(() => {
  void load();
  ro = new ResizeObserver(() => draw());
  if (canvasEl.value) ro.observe(canvasEl.value);
});
onUnmounted(() => ro?.disconnect());
watch(() => props.path, () => void load());
</script>

<template>
  <canvas ref="canvasEl" class="clip-wave" aria-hidden="true" />
</template>

<style scoped>
.clip-wave {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
