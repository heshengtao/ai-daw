<script setup lang="ts">
/**
 * 关闭工程前的「未保存改动」三选一：保存 / 不保存 / 取消。
 *
 * 关闭守卫（`App.vue`）会在窗口关闭请求时打开它，并等待用户的选择——
 * 因此这里只负责把选择以 `choose` 事件抛给上层，由上层决定是否真正退出。
 */
import { ExclamationTriangleIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

defineProps<{ open: boolean }>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "choose", value: "save" | "discard" | "cancel"): void;
}>();

const i18n = useI18n();

function choose(value: "save" | "discard" | "cancel"): void {
  emit("choose", value);
  emit("update:open", false);
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    choose("cancel");
  }
}
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="choose('cancel')">
        <div
          class="dialog"
          role="alertdialog"
          aria-modal="true"
          :aria-label="i18n.t('app.unsavedExitTitle')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <ExclamationTriangleIcon class="warn" />
            <h2 class="title">{{ i18n.t("app.unsavedExitTitle") }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="choose('cancel')">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <p class="message">{{ i18n.t("app.unsavedExit") }}</p>
          </div>

          <footer class="foot">
            <button class="btn-text" @click="choose('cancel')">{{ i18n.t("common.cancel") }}</button>
            <button class="btn-text danger" @click="choose('discard')">{{ i18n.t("common.dontSave") }}</button>
            <button class="btn-filled" @click="choose('save')">{{ i18n.t("common.save") }}</button>
          </footer>
        </div>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 130;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background-color: rgb(var(--md-scrim) / 0.42);
}
.dialog {
  width: 400px;
  max-width: 100%;
  display: flex;
  flex-direction: column;
  border-radius: 24px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-3);
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 16px 16px 8px 22px;
}
.warn {
  width: 22px;
  height: 22px;
  flex: 0 0 auto;
  color: rgb(var(--md-error));
}
.title {
  flex: 1 1 auto;
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
  flex: 0 0 auto;
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
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.icon-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.body {
  padding: 4px 22px 8px;
}
.message {
  margin: 0;
  font-size: 14px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
}
.foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px 16px;
}
.btn-text {
  height: 36px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.btn-text:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
.btn-text.danger {
  color: rgb(var(--md-error));
}
.btn-text.danger:hover {
  background-color: rgb(var(--md-error) / 0.08);
}
.btn-filled {
  height: 36px;
  padding: 0 20px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 13px;
  font-weight: 500;
  cursor: pointer;
  transition: filter 0.14s ease;
}
.btn-filled:hover {
  filter: brightness(1.06);
}
.btn-text:focus-visible,
.btn-filled:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.16s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .icon-btn,
  .btn-text,
  .btn-filled,
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>
