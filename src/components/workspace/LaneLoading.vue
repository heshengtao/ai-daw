<script setup lang="ts">
/**
 * 异步下窗格的加载占位：保持与真实窗格完全一致的高度，避免「乐谱还在下载时编排网格
 * 突然长高、下载完又缩回去」的跳动。
 */
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";

const i18n = useI18n();
const layout = useLayoutStore();

/** 1px 顶边框 + 30px 表头。 */
const LANE_CHROME = 31;
</script>

<template>
  <div class="lane-loading" :style="{ height: `${layout.laneHeight + LANE_CHROME}px` }">
    <span>{{ i18n.t("lane.loading") }}</span>
  </div>
</template>

<style scoped>
.lane-loading {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  justify-content: center;
  border-top: 1px solid var(--grid-line-strong);
  background-color: var(--app-inset);
  font-size: 12px;
  color: rgb(var(--md-on-surface-muted));
  user-select: none;
}
</style>
