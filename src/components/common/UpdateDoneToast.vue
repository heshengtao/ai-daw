<script setup lang="ts">
/**
 * 「更新完成」提示：升级后首次打开时在底部弹一条 snackbar，不挡界面。
 *
 * 版本对比在 `stores/updater.ts::detectJustUpdated` 里做（localStorage 记住上次运行版本）；
 * 这里只负责展示，并把「查看更新日志」指到当前版本对应的 GitHub Release 页。
 * 没有交互 12 秒后自动收起，也可以点「×」立即关掉。
 */
import { computed, onUnmounted, watch } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import Logo from "@/components/common/Logo.vue";
import { releaseNotesUrl } from "@/api/updater";
import { openExternal } from "@/utils/external";
import { useUpdaterStore } from "@/stores/updater";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";

/** 自动收起时间：够读完一句话并点到按钮，又不至于一直占着屏幕。 */
const AUTO_DISMISS_MS = 12_000;

const updater = useUpdaterStore();
const layout = useLayoutStore();
const i18n = useI18n();

/** 普通 toast 也在底部时错开，免得两条叠在一起。 */
const bottom = computed(() => (layout.toast ? "100px" : "44px"));

let timer: number | null = null;

function clearTimer(): void {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
}

watch(
  () => updater.justUpdatedVersion,
  (version) => {
    clearTimer();
    if (!version) return;
    timer = window.setTimeout(() => updater.dismissJustUpdated(), AUTO_DISMISS_MS);
  },
  { immediate: true },
);

function dismiss(): void {
  clearTimer();
  updater.dismissJustUpdated();
}

/** 打开当前版本的 GitHub 发布页，然后收起提示。 */
function viewNotes(): void {
  const version = updater.justUpdatedVersion;
  if (version) void openExternal(releaseNotesUrl(version));
  dismiss();
}

onUnmounted(clearTimer);
</script>

<template>
  <transition name="toast">
    <div
      v-if="updater.justUpdatedVersion"
      class="snackbar"
      :style="{ bottom }"
      role="status"
      aria-live="polite"
    >
      <Logo filled />
      <span class="text">
        {{ i18n.t("updateDone.version", { version: updater.justUpdatedVersion }) }}
      </span>
      <button class="action" @click="viewNotes">{{ i18n.t("updateDone.view") }}</button>
      <button class="close" :aria-label="i18n.t('common.close')" @click="dismiss">
        <XMarkIcon class="w-[16px] h-[16px]" />
      </button>
    </div>
  </transition>
</template>

<style scoped>
.snackbar {
  position: fixed;
  left: 50%;
  transform: translateX(-50%);
  z-index: 80;
  max-width: min(560px, 82vw);
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px 9px 14px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface));
  box-shadow: var(--md-elevation-3);
}
.text {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12.5px;
  letter-spacing: 0.2px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.action {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 12.5px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.action:hover {
  background-color: rgb(var(--md-primary) / var(--md-state-hover-alpha));
}
.close {
  flex: 0 0 auto;
  width: 28px;
  height: 28px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.close:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.action:focus-visible,
.close:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity 0.22s ease, transform 0.22s cubic-bezier(0.2, 0, 0, 1);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translate(-50%, 8px);
}
@media (prefers-reduced-motion: reduce) {
  .action,
  .close,
  .toast-enter-active,
  .toast-leave-active {
    transition: none;
  }
}
@media (max-width: 520px) {
  .text {
    white-space: normal;
  }
}
</style>
