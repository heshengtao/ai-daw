<script setup lang="ts">
/**
 * 顶栏下边缘的音频导入进度条。
 *
 * 确定进度（容器头给了总帧数）按百分比填充；总帧数未知时走不确定态滑动动画。挂在
 * `.app-stack` 里、绝对定位贴住 52px 顶栏的底边，因此**不占布局**（出现/消失不会让工作区跳动）。
 */
import { useImportingStore } from "@/stores/importing";
import { useI18n } from "@/i18n";

const importing = useImportingStore();
const i18n = useI18n();
</script>

<template>
  <div
    v-if="importing.active"
    class="import-bar"
    role="progressbar"
    :aria-label="i18n.t('import.progressLabel')"
    :aria-valuenow="importing.progressPct ?? undefined"
    aria-valuemin="0"
    aria-valuemax="100"
  >
    <div
      class="import-fill"
      :class="{ indeterminate: importing.progressPct === null }"
      :style="importing.progressPct === null ? undefined : { width: `${importing.progressPct}%` }"
    />
  </div>
</template>

<style scoped>
.import-bar {
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
.import-fill {
  height: 100%;
  background-color: rgb(var(--md-primary));
  transition: width 0.12s linear;
}
/* 总帧数未知：一段定宽色块来回滑动，表示「在跑但说不准还有多久」。 */
.import-fill.indeterminate {
  width: 40%;
  animation: import-slide 1.1s ease-in-out infinite;
}
@keyframes import-slide {
  from {
    margin-left: -40%;
  }
  to {
    margin-left: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .import-fill {
    transition: none;
  }
  .import-fill.indeterminate {
    animation-duration: 2.4s;
  }
}
</style>
