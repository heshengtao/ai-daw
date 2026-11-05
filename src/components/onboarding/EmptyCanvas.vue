<script setup lang="ts">
/** 空网格小插图：一条虚线轨道 + 拖出来的片段 + 光标。用于「从这里开始」浮卡与空音轨栏。 */
</script>

<template>
  <svg class="art" viewBox="0 0 240 150" role="img" aria-hidden="true" focusable="false">
    <!-- 淡淡的网格底 -->
    <g style="stroke: rgb(var(--md-outline-variant)); stroke-width: 1; opacity: 0.5">
      <line x1="20" y1="26" x2="20" y2="128" />
      <line x1="86" y1="26" x2="86" y2="128" />
      <line x1="152" y1="26" x2="152" y2="128" />
      <line x1="218" y1="26" x2="218" y2="128" />
    </g>

    <!-- 空轨道：虚线框 -->
    <rect
      x="12"
      y="46"
      width="216"
      height="56"
      rx="14"
      style="fill: rgb(var(--md-primary)); fill-opacity: 0.04; stroke: rgb(var(--md-outline-variant)); stroke-width: 1.5; stroke-dasharray: 7 6"
    />

    <!-- 拖出来的片段（实线 + 左侧色条） -->
    <g class="ghost">
      <rect x="36" y="58" width="92" height="32" rx="10" style="fill: rgb(var(--md-primary)); fill-opacity: 0.2" />
      <rect x="36" y="58" width="4" height="32" rx="2" style="fill: rgb(var(--md-primary))" />
      <g style="fill: rgb(var(--md-primary)); opacity: 0.75">
        <rect x="50" y="72" width="14" height="4" rx="2" />
        <rect x="70" y="66" width="20" height="4" rx="2" />
        <rect x="96" y="76" width="18" height="4" rx="2" />
      </g>
    </g>

    <!-- 拖拽虚线 + 光标 -->
    <path
      d="M136 74h26"
      style="stroke: rgb(var(--md-primary)); stroke-width: 1.5; stroke-dasharray: 4 4; opacity: 0.7"
    />
    <g class="cursor" style="fill: rgb(var(--md-on-surface))">
      <path
        d="M166 66l16 6.6-6.6 1.9 3.6 7.3-3.6 1.8-3.6-7.4-5.4 4.2z"
        style="stroke: rgb(var(--md-surface-container-lowest)); stroke-width: 1.4; stroke-linejoin: round"
      />
    </g>

    <!-- 右上角的「新增」小胶囊 -->
    <g class="plus">
      <rect x="176" y="20" width="52" height="22" rx="11" style="fill: rgb(var(--md-primary)); opacity: 0.16" />
      <path
        d="M202 26v10M197 31h10"
        style="stroke: rgb(var(--md-primary)); stroke-width: 2; stroke-linecap: round"
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
.ghost {
  animation: art-ghost 4.5s ease-in-out infinite alternate;
  transform-box: fill-box;
  transform-origin: left center;
}
.cursor {
  animation: art-cursor 4.5s ease-in-out infinite alternate;
}
.plus {
  animation: art-plus 3.2s ease-in-out infinite;
  transform-box: fill-box;
  transform-origin: center;
}
@keyframes art-ghost {
  from {
    opacity: 0.55;
  }
  to {
    opacity: 1;
  }
}
@keyframes art-cursor {
  from {
    transform: translateX(-6px);
  }
  to {
    transform: translateX(0);
  }
}
@keyframes art-plus {
  0%,
  100% {
    opacity: 0.6;
  }
  50% {
    opacity: 1;
  }
}
@media (prefers-reduced-motion: reduce) {
  .ghost,
  .cursor,
  .plus {
    animation: none;
  }
}
</style>
