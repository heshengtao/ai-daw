<script setup lang="ts">
/**
 * 音频片段的波形：把 `audio_waveform` 回来的 min/max 包络画成一列列竖线。
 *
 * canvas 骨架照抄 `Oscilloscope.vue`（devicePixelRatio + CSS 变量取色 + ResizeObserver 按需重绘）。
 *
 * **取的是整段素材**（不是片段窗口），按**路径**缓存一次：裁切 / 移动片段都不再打后端，
 * 也不会出现「松手后波形反复变化」。
 *
 * 画法：横轴是**拍**（画布宽 = 方框宽 = `viewLength * zoom`）。
 * 1. 每个像素 → 拍 → 秒（`beatsToSeconds`，与引擎同一套分段换算）；
 * 2. 换算到**素材内的时刻** τ = (时间轴秒 − 片段起点) × `rate` + 素材偏移；
 * 3. τ 落在素材内就画对应桶——方框是**素材上的一个窗口**，所以拖边缘 = 收起 / 展开，
 *    露出的永远是**同一段素材**（内容不会跟着方框跑）。
 *
 * **`rate` 必须参与换算**：时间轴 1 秒 = 素材 `rate` 秒（与引擎的播放语义一致）——
 * 漏掉它，改了播放速度的片段会画出与听到的完全对不上的波形（`rate = 2` 只画出前一半）。
 */
import { onMounted, onUnmounted, ref, watch } from "vue";
import { audioWaveform, type WaveformSlice } from "@/api/tauri";
import { useProjectStore } from "@/stores/project";
import { useTimelineStore } from "@/stores/timeline";
import { beatsToSeconds } from "@/utils/tempo";

/** 整段素材的桶数：一次取完、按路径缓存。8192 桶 ≈ 长素材里短片段也够看清细节。 */
const BUCKETS = 8192;

const props = defineProps<{
  path: string;
  /** 素材内的起始帧（crop 起点）——决定素材第 0 帧相对方框起点的位置。 */
  offsetFrames?: number;
  /** 播放速度倍率（1 = 原速）；时间轴 1 秒 = 素材 `rate` 秒。 */
  rate?: number;
  /** 方框**内容锚点**（拍）：方框起点在工程时间轴上的位置（移动跟着走、裁切钉在原地）。 */
  sourceStart: number;
  /** 当前显示的方框起点（拍）；拖动中是预览值。 */
  viewStart: number;
  /** 当前显示的方框长度（拍）；拖动中是预览值。 */
  viewLength: number;
}>();

const project = useProjectStore();
const timeline = useTimelineStore();

const canvasEl = ref<HTMLCanvasElement | null>(null);
const slice = ref<WaveformSlice | null>(null);

/** 路径 → 整段素材的包络（切分出来的两半共用同一份）。 */
const cache = new Map<string, WaveformSlice>();

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

  const s = slice.value;
  const transport = project.project?.transport;
  if (!s || s.peaks.length < 2 || !(s.sample_rate > 0) || s.frames === 0 || !transport) return;

  const buckets = Math.floor(s.peaks.length / 2);
  const dur = s.frames / s.sample_rate; // 整段素材时长（秒）
  // 片段起点所在的工程时刻，以及素材窗口的起始偏移（秒）。
  // 时间轴秒 → 素材秒：每秒时间轴吃 `rate` 秒素材（与引擎的播放语义一致）。
  const anchor = beatsToSeconds(props.sourceStart, transport);
  const offsetSec = (props.offsetFrames ?? 0) / s.sample_rate;
  const rate = props.rate ?? 1;

  const mid = h / 2;
  const scale = h / 2 - 1;
  ctx.fillStyle = waveColor(el);
  for (let x = 0; x < w; x += 1) {
    const beat = props.viewStart + x / timeline.zoom;
    const tau = (beatsToSeconds(beat, transport) - anchor) * rate + offsetSec;
    if (tau < 0 || tau > dur) continue; // 方框落在素材之外 → 留白
    const b = Math.min(buckets - 1, Math.floor((tau / dur) * buckets));
    const lo = s.peaks[b * 2] ?? 0;
    const hi = s.peaks[b * 2 + 1] ?? 0;
    const top = mid - Math.max(-1, Math.min(1, hi)) * scale;
    const bottom = mid - Math.max(-1, Math.min(1, lo)) * scale;
    ctx.fillRect(x, top, 1, Math.max(1, bottom - top));
  }
}

async function load(): Promise<void> {
  const cached = cache.get(props.path);
  if (cached) {
    slice.value = cached;
    draw();
    return;
  }
  try {
    const data = await audioWaveform(props.path, BUCKETS);
    cache.set(props.path, data);
    slice.value = data;
  } catch {
    // 素材缺失 / 不可解码：留白。片段本身仍然存在（后端按静音处理）。
    slice.value = null;
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
watch(
  () =>
    [
      props.viewStart,
      props.viewLength,
      props.sourceStart,
      props.offsetFrames,
      // `rate` 参与「时间轴秒 → 素材秒」的换算（见本文件顶部说明）：漏掉它，
      // 改了播放速度后波形不会重画，画出来的还是旧速度的映射（与听到的对不上）。
      props.rate,
      timeline.zoom,
      project.project?.transport,
    ] as const,
  () => draw(),
  { deep: true },
);
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
