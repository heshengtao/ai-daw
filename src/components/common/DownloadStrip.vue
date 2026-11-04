<script setup lang="ts">
/**
 * **全局下载条**：AI 页面里任意位置都能看到「正在下什么、下到哪、还剩几项」。
 *
 * 为什么不是每条卡片各画一根进度条：一次「准备运行环境」会串着下引擎和权重，
 * 进度会从卡片 A 跳到卡片 B；给用户一根始终在顶部的条，比让他在两处之间找更清楚。
 * 「在后台下载」之后用户仍留在本页，靠的就是这条。
 *
 * 失败时不弹框：条子变成红的一行 + 「重试剩余」——用户可能正在别处，别用模态框打断。
 */
import { computed } from "vue";
import { useDownloadsStore } from "@/stores/downloads";
import { useI18n } from "@/i18n";

const downloads = useDownloadsStore();
const i18n = useI18n();

const progress = computed(() => downloads.progress);

/** `第 1/2 项 · 42%`：总大小未知时百分比位显示省略号。 */
const step = computed(() => {
  const value = progress.value;
  if (!value) return "";
  return i18n.t("setup.busyStep", {
    index: value.index,
    total: value.total,
    pct: value.pct === null ? "…" : `${value.pct}%`,
  });
});

/** 还没开始的那几项（让用户知道「后面还排着」）。 */
const queued = computed(() => Math.max(0, downloads.pending));

const failure = computed(() => (downloads.running ? null : downloads.failure));
</script>

<template>
  <div v-if="downloads.running && progress" class="strip" role="status" aria-live="polite">
    <div class="bar">
      <div
        class="fill"
        :class="{ indeterminate: progress.pct === null }"
        :style="progress.pct === null ? undefined : { width: `${progress.pct}%` }"
      />
    </div>
    <span class="text">
      {{ step }} · {{ progress.label }}
      <span v-if="queued" class="queued">{{ i18n.t("setup.queued", { count: queued }) }}</span>
    </span>
    <button class="action" type="button" @click="downloads.cancel()">
      {{ i18n.t("common.cancel") }}
    </button>
  </div>

  <div v-else-if="failure" class="strip error" role="status">
    <span class="text error-text">
      {{ i18n.t("setup.failed", { name: failure.task.label, error: failure.message }) }}
    </span>
    <button v-if="downloads.retryCount > 0" class="action" type="button" @click="downloads.retry()">
      {{ i18n.t("setup.retryRest", { count: downloads.retryCount }) }}
    </button>
    <button class="action" type="button" @click="downloads.failure = null">
      {{ i18n.t("common.close") }}
    </button>
  </div>
</template>

<style scoped>
.strip {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 8px;
  margin: 8px 12px 0;
  padding: 6px 10px;
  border-radius: 10px;
  background-color: rgb(var(--md-primary) / 0.08);
}
.strip.error {
  background-color: rgb(var(--md-error) / 0.12);
}
.bar {
  width: 72px;
  height: 4px;
  flex: 0 0 auto;
  overflow: hidden;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
}
.fill {
  height: 100%;
  border-radius: inherit;
  background-color: rgb(var(--md-primary));
  transition: width 0.2s ease;
}
.fill.indeterminate {
  width: 35%;
  animation: strip-slide 1.2s ease-in-out infinite;
}
@keyframes strip-slide {
  0% {
    transform: translateX(-100%);
  }
  100% {
    transform: translateX(300%);
  }
}
.text {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.error-text {
  color: rgb(var(--md-error));
  white-space: normal;
}
.queued {
  margin-left: 6px;
  opacity: 0.75;
}
.action {
  flex: 0 0 auto;
  padding: 0;
  border: 0;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 10.5px;
  cursor: pointer;
}
.action:hover {
  text-decoration: underline;
}
@media (prefers-reduced-motion: reduce) {
  .fill.indeterminate {
    width: 100%;
    animation: none;
  }
}
</style>
