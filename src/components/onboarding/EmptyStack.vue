<script setup lang="ts">
/** 空面板小插图：几张叠在一起的面板卡片。用于右侧面板「还没打开面板」的空态。 */
</script>

<template>
  <svg class="art" viewBox="0 0 180 140" role="img" aria-hidden="true" focusable="false">
    <!-- 后面两张：只露一角，表示「这里可以放很多东西」 -->
    <rect
      x="30"
      y="26"
      width="108"
      height="74"
      rx="14"
      transform="rotate(-8 84 63)"
      style="fill: rgb(var(--md-tertiary)); opacity: 0.16"
    />
    <rect
      x="42"
      y="30"
      width="108"
      height="74"
      rx="14"
      transform="rotate(5 96 67)"
      style="fill: rgb(var(--md-secondary)); opacity: 0.2"
    />

    <!-- 前面这张：标题条 + 两行内容 -->
    <g class="front">
      <rect
        x="34"
        y="38"
        width="112"
        height="76"
        rx="14"
        style="fill: rgb(var(--md-surface-container-lowest)); stroke: rgb(var(--md-outline-variant)); stroke-opacity: 0.7"
        stroke-width="1.5"
      />
      <rect x="46" y="50" width="42" height="6" rx="3" style="fill: rgb(var(--md-on-surface)); opacity: 0.35" />
      <rect x="46" y="66" width="88" height="12" rx="6" style="fill: rgb(var(--md-primary)); opacity: 0.16" />
      <rect x="46" y="84" width="88" height="12" rx="6" style="fill: rgb(var(--md-primary)); opacity: 0.1" />
      <rect x="52" y="70" width="30" height="4" rx="2" style="fill: rgb(var(--md-primary)); opacity: 0.7" />
      <rect x="52" y="88" width="20" height="4" rx="2" style="fill: rgb(var(--md-primary)); opacity: 0.45" />
    </g>

    <!-- AI 小徽标 -->
    <g class="badge">
      <circle cx="132" cy="104" r="15" style="fill: rgb(var(--md-primary))" />
      <path
        d="M132 94l2.7 7.3 7.3 2.7-7.3 2.7-2.7 7.3-2.7-7.3-7.3-2.7 7.3-2.7z"
        style="fill: rgb(var(--md-on-primary))"
      />
    </g>
  </svg>
</template>

<style scoped>
.art {
  width: 100%;
  height: auto;
  display: block;
}
.front {
  animation: art-front 6s ease-in-out infinite alternate;
  transform-box: fill-box;
  transform-origin: center;
}
.badge {
  animation: art-badge 3.4s ease-in-out infinite;
  transform-box: fill-box;
  transform-origin: center;
}
@keyframes art-front {
  from {
    transform: translateY(0);
  }
  to {
    transform: translateY(-4px);
  }
}
@keyframes art-badge {
  0%,
  100% {
    opacity: 0.75;
    transform: scale(0.96);
  }
  50% {
    opacity: 1;
    transform: scale(1.04);
  }
}
@media (prefers-reduced-motion: reduce) {
  .front,
  .badge {
    animation: none;
  }
}
</style>
