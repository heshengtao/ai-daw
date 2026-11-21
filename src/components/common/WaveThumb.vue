<script setup lang="ts">
/**
 * 生成记录里的**波形缩略图**（当封面用）。
 *
 * 没有封面图生成，也不该拿一张假图凑数——一首歌在这个 DAW 里的身份就是它的波形。
 * 颜色取工程里那条轨道的色相（与编排页的片段同色），于是「列表里的这一条」和
 * 「时间线上的那一条轨道」一眼能对上。
 *
 * 与 `ClipWaveform` 的差别：那边要把波形映射到片段方框、跟着缩放和平移重画；
 * 这里是固定宽高的小图，只看**整段素材**，所以按路径缓存一次包络就够了（列表滚动时不打后端）。
 */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { audioWaveform, type WaveformSlice } from "@/api/tauri";

const props = defineProps<{
  path: string;
  /** 轨道色相（度）。 */
  hue: number;
}>();

/** 缩略图用**低**桶数：小图上再多细节也是糊的，少取点后端也轻松。 */
const BUCKETS = 320;

/** 路径 → 包络（整个应用的列表共用一份：同一首歌在多处出现只读一次）。 */
const cache = new Map<string, WaveformSlice>();

const canvasEl = ref<HTMLCanvasElement | null>(null);
const slice = ref<WaveformSlice | null>(null);
const failed = ref(false);
let disposed = false;

function draw(): void {
  const el = canvasEl.value;
  if (!el) return;
  const ctx = el.getContext("2d");
  if (!ctx) return;
  const width = el.clientWidth;
  const height = el.clientHeight;
  if (width === 0 || height === 0) return;

  const dpr = window.devicePixelRatio || 1;
  if (el.width !== Math.round(width * dpr) || el.height !== Math.round(height * dpr)) {
    el.width = Math.round(width * dpr);
    el.height = Math.round(height * dpr);
  }
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  const data = slice.value;
  if (!data || data.peaks.length < 2) {
    // 还没加载 / 素材不可解码：画一条淡的中线，别让这一格空成黑洞。
    ctx.fillStyle = `hsl(${props.hue} 55% 55% / 0.3)`;
    ctx.fillRect(0, Math.round(height / 2) - 0.5, width, 1);
    return;
  }

  const buckets = Math.floor(data.peaks.length / 2);
  const mid = height / 2;
  const scale = height / 2 - 1;
  ctx.fillStyle = `hsl(${props.hue} 55% 55% / 0.72)`;
  for (let x = 0; x < width; x += 1) {
    const bucket = Math.min(buckets - 1, Math.floor((x / width) * buckets));
    const lo = data.peaks[bucket * 2] ?? 0;
    const hi = data.peaks[bucket * 2 + 1] ?? 0;
    const top = mid - Math.max(-1, Math.min(1, hi)) * scale;
    const bottom = mid - Math.max(-1, Math.min(1, lo)) * scale;
    ctx.fillRect(x, top, 1, Math.max(1, bottom - top));
  }
}

async function load(): Promise<void> {
  failed.value = false;
  const path = props.path;
  const cached = cache.get(path);
  if (cached) {
    slice.value = cached;
    draw();
    return;
  }
  try {
    const data = await audioWaveform(path, BUCKETS);
    if (disposed || path !== props.path) return; // 期间换了记录
    cache.set(path, data);
    slice.value = data;
  } catch {
    if (disposed || path !== props.path) return;
    // 文件被删 / 解不开：留一条中线，记录本身还在（播放会给出真正的错误）。
    failed.value = true;
    slice.value = null;
  }
  draw();
}

let observer: ResizeObserver | null = null;
onMounted(() => {
  void load();
  observer = new ResizeObserver(() => draw());
  if (canvasEl.value) observer.observe(canvasEl.value);
});
onUnmounted(() => {
  disposed = true;
  observer?.disconnect();
});
watch(
  () => props.path,
  () => {
    slice.value = null;
    void load();
  },
);
watch(
  () => props.hue,
  () => draw(),
);
</script>

<template>
  <canvas ref="canvasEl" class="wave-thumb" aria-hidden="true" />
</template>

<style scoped>
.wave-thumb {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
