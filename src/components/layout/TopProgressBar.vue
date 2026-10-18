<script setup lang="ts">
/**
 * 顶栏下边缘的**长任务进度条**（导入 / 导出共用同一条）。
 *
 * 确定进度（有总量）按百分比填充；总量未知（导出时的「准备中」「编码中」）走不确定态滑动动画。
 * 挂在 `.app-stack` 里、绝对定位贴住 52px 顶栏的底边，因此**不占布局**（出现/消失不会让工作区跳动）。
 *
 * 只认 props，不认 store：`App.vue` 决定「现在该显示哪个任务」（导入优先），本组件只管画。
 */
withDefaults(
  defineProps<{
    /** 有没有在跑的长任务（false 时整条隐藏）。 */
    active: boolean;
    /** 进度百分比（0–100）；`null` = 总量未知（不确定态）。 */
    pct: number | null;
    /** 无障碍名 / 悬浮提示（例如「正在渲染…」）。 */
    label: string;
  }>(),
  { pct: null },
);
</script>

<template>
  <div
    v-if="active"
    class="job-bar"
    role="progressbar"
    :aria-label="label"
    :title="label"
    :aria-valuenow="pct ?? undefined"
    aria-valuemin="0"
    aria-valuemax="100"
  >
    <div
      class="job-fill"
      :class="{ indeterminate: pct === null }"
      :style="pct === null ? undefined : { width: `${pct}%` }"
    />
  </div>
</template>

<style scoped>
.job-bar {
  position: absolute;
  top: 52px;
  left: 0;
  right: 0;
  height: 3px;
  z-index: 40;
  overflow: hidden;
  background-color: rgb(var(--md-primary) / 0.16);
  pointer-events: none;
}
.job-fill {
  height: 100%;
  background-color: rgb(var(--md-primary));
  transition: width 0.12s linear;
}
/* 总量未知：一段定宽色块来回滑动，表示「在跑但说不准还有多久」。 */
.job-fill.indeterminate {
  width: 40%;
  animation: job-slide 1.1s ease-in-out infinite;
}
@keyframes job-slide {
  from {
    margin-left: -40%;
  }
  to {
    margin-left: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .job-fill {
    transition: none;
  }
  .job-fill.indeterminate {
    animation-duration: 2.4s;
  }
}
</style>
