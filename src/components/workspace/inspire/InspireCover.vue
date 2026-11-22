<script setup lang="ts">
/**
 * 生成记录的**封面**：按曲子 id 生成一张程序化图案，中间压一个常显的播放/暂停键。
 *
 * 为什么不用波形当封面：波形是「素材」的语言，不是「作品」的语言——一行八九十条记录
 * 全是一模一样的横线，用户既分不出哪首是哪首，也看不出这一格能点。这里换成一幅由 id
 * 决定的抽象画（同一首歌永远是同一张），再叠一个**一直看得见**的播放键，点哪里就不用猜了。
 *
 * 图案完全本地生成（不下载、不写盘）：一个哈希播种的 PRNG 决定色相偏移、色块位置与旋转角，
 * 所以「同一首歌同一张封面」在重开应用、换机器后都一样。
 */
import { computed } from "vue";
import { ArrowPathIcon, PlayIcon, PauseIcon } from "@heroicons/vue/24/solid";
import { useI18n } from "@/i18n";

const props = defineProps<{
  /** 稳定的种子：曲子的 id（同一首永远同一张）。 */
  seed: string;
  /** 轨道色相（度），与编排页的片段同色。 */
  hue: number;
  playing?: boolean;
  loading?: boolean;
}>();

const emit = defineEmits<{ (e: "toggle"): void }>();
const i18n = useI18n();

/** FNV-1a：短字符串散列，够用且稳定。 */
function hash(value: string): number {
  let h = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** mulberry32：小而稳的 PRNG，同一个种子永远同一条序列。 */
function random(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const wrap = (value: number): number => ((value % 360) + 360) % 360;

interface Blob {
  cx: number;
  cy: number;
  r: number;
  fill: string;
}

/** 一张封面要用的全部参数：底色、若干软色块、一组装饰线的角度。 */
const art = computed(() => {
  const rnd = random(hash(props.seed));
  const base = wrap(props.hue + rnd() * 24 - 12);
  const blobs: Blob[] = [];
  const count = 4 + Math.floor(rnd() * 3);
  for (let i = 0; i < count; i += 1) {
    const hue = wrap(base + rnd() * 150 - 75);
    const sat = Math.round(45 + rnd() * 40);
    const light = Math.round(38 + rnd() * 34);
    const alpha = (0.32 + rnd() * 0.42).toFixed(2);
    blobs.push({
      cx: Math.round(-10 + rnd() * 120),
      cy: Math.round(-10 + rnd() * 120),
      r: Math.round(22 + rnd() * 46),
      fill: `hsl(${hue} ${sat}% ${light}% / ${alpha})`,
    });
  }
  const angle = Math.round(rnd() * 180);
  const lines = Array.from({ length: 3 }, () => Math.round(rnd() * 100));
  // 一两道细圆环：没有它，几张封面看起来会像同一张磨砂玻璃的不同滤镜。
  const rings = Array.from({ length: 1 + Math.floor(rnd() * 2) }, () => ({
    cx: Math.round(10 + rnd() * 80),
    cy: Math.round(10 + rnd() * 80),
    r: Math.round(18 + rnd() * 40),
    opacity: (0.16 + rnd() * 0.2).toFixed(2),
  }));
  const gradientAngle = Math.round(rnd() * 360);
  return {
    from: `hsl(${wrap(base + 18)} 62% 30%)`,
    to: `hsl(${wrap(base - 26)} 68% 12%)`,
    blobs,
    angle,
    lines,
    rings,
    gradientAngle,
  };
});
</script>

<template>
  <button
    class="cover"
    type="button"
    :style="{ '--track-hue': String(hue) }"
    :aria-label="playing ? i18n.t('inspire.pause') : i18n.t('inspire.play')"
    :aria-pressed="playing === true"
    @click="emit('toggle')"
  >
    <svg class="art" viewBox="0 0 100 100" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
      <defs>
        <linearGradient
          :id="`bg-${seed}`"
          x1="0"
          y1="0"
          x2="1"
          y2="1"
          :gradientTransform="`rotate(${art.gradientAngle} 0.5 0.5)`"
        >
          <stop offset="0%" :stop-color="art.from" />
          <stop offset="100%" :stop-color="art.to" />
        </linearGradient>
        <filter :id="`soft-${seed}`" x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="7" />
        </filter>
        <radialGradient :id="`vig-${seed}`" cx="0.5" cy="0.46" r="0.72">
          <stop offset="55%" stop-color="hsl(0 0% 0% / 0)" />
          <stop offset="100%" stop-color="hsl(0 0% 0% / 0.55)" />
        </radialGradient>
      </defs>
      <rect width="100" height="100" :fill="`url(#bg-${seed})`" />
      <g :filter="`url(#soft-${seed})`">
        <circle v-for="(blob, i) in art.blobs" :key="i" :cx="blob.cx" :cy="blob.cy" :r="blob.r" :fill="blob.fill" />
      </g>
      <circle
        v-for="(ring, i) in art.rings"
        :key="`ring-${i}`"
        :cx="ring.cx"
        :cy="ring.cy"
        :r="ring.r"
        fill="none"
        stroke="hsl(0 0% 100%)"
        stroke-width="0.5"
        :opacity="ring.opacity"
      />
      <!-- 一道细斜纹：让图案不至于糊成一团色块 -->
      <g :transform="`rotate(${art.angle} 50 50)`" opacity="0.16">
        <line
          v-for="(y, i) in art.lines"
          :key="i"
          x1="-10"
          :y1="y"
          x2="110"
          :y2="y"
          stroke="hsl(0 0% 100%)"
          stroke-width="0.6"
        />
      </g>
      <rect class="vignette" width="100" height="100" :fill="`url(#vig-${seed})`" />
    </svg>

    <!-- 播放键常显：它是这一格的主动作，不该等 hover 才出现 -->
    <span class="play" aria-hidden="true">
      <ArrowPathIcon v-if="loading" class="icon spin" />
      <PauseIcon v-else-if="playing" class="icon" />
      <PlayIcon v-else class="icon play-icon" />
    </span>
  </button>
</template>

<style scoped>
.cover {
  position: relative;
  display: block;
  width: 100%;
  height: 100%;
  padding: 0;
  border: 0;
  overflow: hidden;
  cursor: pointer;
  background: hsl(var(--track-hue) 45% 26%);
  transition: transform var(--md-duration-short-2) var(--md-ease-spring-fast);
}
.cover:hover {
  transform: scale(1.02);
}
.cover:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.art {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
}
/* 四角压暗：中间那枚播放键才立得起来 */
.vignette {
  pointer-events: none;
}
.play {
  position: absolute;
  left: 50%;
  top: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  /* 尺寸由调用方给（列表窄的时候会收一档），默认 32px */
  width: var(--cover-play, 32px);
  height: var(--cover-play, 32px);
  margin: calc(var(--cover-play, 32px) / -2) 0 0 calc(var(--cover-play, 32px) / -2);
  border-radius: 999px;
  color: #fff;
  background-color: rgb(0 0 0 / 0.42);
  box-shadow: 0 2px 10px rgb(0 0 0 / 0.35);
  backdrop-filter: blur(3px);
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    transform var(--md-duration-short-2) var(--md-ease-spring-fast);
}
.cover:hover .play,
.cover:focus-visible .play {
  background-color: rgb(0 0 0 / 0.56);
  transform: scale(1.06);
}
.play .icon {
  width: calc(var(--cover-play, 32px) * 0.47);
  height: calc(var(--cover-play, 32px) * 0.47);
  filter: drop-shadow(0 1px 2px rgb(0 0 0 / 0.45));
}
/* 三角形重心偏左，补 1px 才在视觉上居中 */
.play-icon {
  margin-left: 1px;
}
.spin {
  animation: spin 1s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
@media (prefers-reduced-motion: reduce) {
  .cover,
  .play {
    transition: none;
  }
  .spin {
    animation: none;
  }
}
</style>
