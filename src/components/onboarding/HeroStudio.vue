<script setup lang="ts">
/**
 * 欢迎页主插图：一台「正在写歌」的工作站。
 *
 * 纯 SVG + 主题 token，不引位图：亮色 / 深色 / 五种强调色都会自动跟着变，
 * 打包体积也只有几 KB。两处慢速浮动由 CSS 驱动，`prefers-reduced-motion` 下自动停。
 */
</script>

<template>
  <svg class="hero" viewBox="0 0 440 360" role="img" aria-hidden="true" focusable="false">
    <defs>
      <linearGradient id="hero-bg" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0%" style="stop-color: rgb(var(--md-primary-container))" />
        <stop offset="55%" style="stop-color: rgb(var(--md-surface-container-high))" />
        <stop offset="100%" style="stop-color: rgb(var(--md-tertiary-container))" />
      </linearGradient>
      <linearGradient id="hero-clip" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0%" style="stop-color: rgb(var(--md-primary)); stop-opacity: 0.34" />
        <stop offset="100%" style="stop-color: rgb(var(--md-primary)); stop-opacity: 0.14" />
      </linearGradient>
    </defs>

    <!-- 底：柔和的圆角画布 + 两团气氛色 -->
    <rect x="0" y="0" width="440" height="360" rx="28" fill="url(#hero-bg)" />
    <circle cx="368" cy="52" r="86" style="fill: rgb(var(--md-tertiary)); opacity: 0.16" />
    <circle cx="54" cy="322" r="104" style="fill: rgb(var(--md-primary)); opacity: 0.14" />

    <!-- 工作站窗口 -->
    <g class="float">
      <rect
        x="44"
        y="104"
        width="352"
        height="192"
        rx="20"
        style="fill: rgb(var(--md-scrim)); opacity: 0.18"
      />
      <rect
        x="36"
        y="96"
        width="368"
        height="196"
        rx="20"
        style="fill: rgb(var(--md-surface-container-high)); stroke: rgb(var(--md-outline-variant)); stroke-opacity: 0.55"
        stroke-width="1.5"
      />

      <!-- 标题栏 -->
      <circle cx="58" cy="116" r="3.5" style="fill: rgb(var(--md-outline)); opacity: 0.5" />
      <circle cx="70" cy="116" r="3.5" style="fill: rgb(var(--md-outline)); opacity: 0.35" />
      <circle cx="82" cy="116" r="3.5" style="fill: rgb(var(--md-outline)); opacity: 0.25" />
      <rect x="300" y="110" width="64" height="12" rx="6" style="fill: rgb(var(--md-primary)); opacity: 0.18" />

      <!-- 三条音轨 -->
      <line x1="44" y1="164" x2="396" y2="164" style="stroke: rgb(var(--md-outline-variant)); opacity: 0.45" />
      <line x1="44" y1="212" x2="396" y2="212" style="stroke: rgb(var(--md-outline-variant)); opacity: 0.45" />
      <line x1="44" y1="260" x2="396" y2="260" style="stroke: rgb(var(--md-outline-variant)); opacity: 0.45" />

      <!-- 第一轨：两块旋律片段 -->
      <g class="pop-a">
        <rect x="60" y="138" width="86" height="20" rx="7" fill="url(#hero-clip)" />
        <rect x="60" y="138" width="4" height="20" rx="2" style="fill: rgb(var(--md-primary))" />
        <rect x="158" y="138" width="118" height="20" rx="7" fill="url(#hero-clip)" />
        <rect x="158" y="138" width="4" height="20" rx="2" style="fill: rgb(var(--md-primary))" />
      </g>

      <!-- 第二轨：音频波形 -->
      <g class="pop-b">
        <rect x="60" y="186" width="150" height="20" rx="7" style="fill: rgb(var(--md-tertiary)); opacity: 0.24" />
        <rect x="60" y="186" width="4" height="20" rx="2" style="fill: rgb(var(--md-tertiary))" />
        <g style="fill: rgb(var(--md-tertiary)); opacity: 0.85">
          <rect v-for="(h, i) in [7, 12, 16, 9, 14, 6, 11, 17, 8, 13, 5, 10, 15, 7, 12, 9]" :key="i"
            :x="74 + i * 8" :y="196 - h / 2" width="3" :height="h" rx="1.5" />
        </g>
      </g>

      <!-- 第三轨：MIDI 音符（小方块） -->
      <g class="pop-c">
        <rect x="60" y="234" width="176" height="20" rx="7" style="fill: rgb(var(--md-secondary)); opacity: 0.18" />
        <g style="fill: rgb(var(--md-secondary)); opacity: 0.8">
          <rect v-for="(n, i) in [
            { x: 72, w: 12, y: 6 },
            { x: 92, w: 8, y: 2 },
            { x: 108, w: 16, y: 9 },
            { x: 132, w: 10, y: 4 },
            { x: 150, w: 14, y: 7 },
            { x: 172, w: 8, y: 3 },
            { x: 188, w: 18, y: 8 },
            { x: 214, w: 10, y: 5 },
          ]" :key="i" :x="n.x" :y="238 + n.y" :width="n.w" height="4" rx="2" />
        </g>
      </g>

      <!-- 播放头 -->
      <g class="playhead">
        <rect x="232" y="100" width="2" height="188" style="fill: rgb(var(--md-primary)); opacity: 0.9" />
        <path d="M225 100h16l-8 11z" style="fill: rgb(var(--md-primary))" />
      </g>
    </g>

    <!-- AI 标记：主色圆 + 四角星 -->
    <g class="spark spark-1">
      <circle cx="372" cy="82" r="24" style="fill: rgb(var(--md-primary))" />
      <path
        d="M372 66l4.4 11.6L388 82l-11.6 4.4L372 98l-4.4-11.6L356 82l11.6-4.4z"
        style="fill: rgb(var(--md-on-primary))"
      />
    </g>
    <g class="spark spark-2">
      <path
        d="M318 52l3 8 8 3-8 3-3 8-3-8-8-3 8-3z"
        style="fill: rgb(var(--md-tertiary)); opacity: 0.85"
      />
    </g>
    <g class="spark spark-3">
      <path
        d="M410 134l2.4 6.4 6.6 2.4-6.6 2.4-2.4 6.4-2.4-6.4-6.6-2.4 6.6-2.4z"
        style="fill: rgb(var(--md-primary)); opacity: 0.6"
      />
    </g>

    <!-- 两枚飘在外面的音符，压住四角 -->
    <g class="note note-1" style="fill: rgb(var(--md-primary)); opacity: 0.5">
      <ellipse cx="22" cy="86" rx="8" ry="6" transform="rotate(-18 22 86)" />
      <rect x="28" y="58" width="2.6" height="28" rx="1.3" />
      <path d="M30 58c6 1 9 4 10 9-3-3-6-4-10-4z" />
    </g>
    <g class="note note-2" style="fill: rgb(var(--md-tertiary)); opacity: 0.45">
      <ellipse cx="418" cy="300" rx="7" ry="5" transform="rotate(-18 418 300)" />
      <rect x="423" y="276" width="2.4" height="24" rx="1.2" />
    </g>
  </svg>
</template>

<style scoped>
.hero {
  width: 100%;
  height: auto;
  display: block;
}

/* 窗口整体上下轻浮，音符反向慢慢飘：幅度只有 6px，不抢文字。 */
.float {
  animation: hero-float 7s ease-in-out infinite alternate;
}
.note-1 {
  animation: hero-drift 9s ease-in-out infinite alternate;
}
.note-2 {
  animation: hero-drift 11s ease-in-out infinite alternate-reverse;
}
.spark {
  transform-box: fill-box;
  transform-origin: center;
}
.spark-1 {
  animation: hero-twinkle 3.6s ease-in-out infinite;
}
.spark-2 {
  animation: hero-twinkle 3.6s ease-in-out 0.7s infinite;
}
.spark-3 {
  animation: hero-twinkle 4.4s ease-in-out 1.4s infinite;
}

@keyframes hero-float {
  from {
    transform: translateY(0);
  }
  to {
    transform: translateY(-6px);
  }
}
@keyframes hero-drift {
  from {
    transform: translateY(0) rotate(0deg);
  }
  to {
    transform: translateY(-8px) rotate(3deg);
  }
}
@keyframes hero-twinkle {
  0%,
  100% {
    opacity: 0.55;
    transform: scale(0.92);
  }
  50% {
    opacity: 1;
    transform: scale(1.06);
  }
}

@media (prefers-reduced-motion: reduce) {
  .float,
  .note-1,
  .note-2,
  .spark {
    animation: none;
  }
}
</style>
