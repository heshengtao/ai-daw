<script setup lang="ts">
/**
 * 通用 **MD3 确认框**：标题 + 一句话 + 「取消 / 确认」。
 *
 * 与 `UnsavedChangesDialog.vue` 同一套外壳（`Teleport` → 遮罩 → 圆角对话框），
 * 但内容泛化成「一次确认」——用于「是否下载模型」这类需要用户点一下才继续的动作。
 * **不用系统原生对话框**（原生框样式与 MD3 对不上）。
 */
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

withDefaults(
  defineProps<{
    open: boolean;
    /** 标题（同时作为对话框的 `aria-label`）。 */
    title: string;
    /** 正文一句话。 */
    message: string;
    /** 确认按钮文案。 */
    confirmLabel: string;
    /** 取消按钮文案；不给就用「取消」。 */
    cancelLabel?: string;
    /** 确认按钮用危险色（删除类操作用）。 */
    danger?: boolean;
  }>(),
  { cancelLabel: undefined, danger: false },
);

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "confirm"): void;
  (e: "cancel"): void;
}>();

const i18n = useI18n();

function cancel(): void {
  emit("cancel");
  emit("update:open", false);
}

function confirm(): void {
  emit("confirm");
  emit("update:open", false);
}

/** Escape 关闭；`stopPropagation` 免得这一个按键再触发外层全局快捷键。 */
function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    cancel();
  }
}
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="cancel">
        <div
          class="dialog"
          role="alertdialog"
          aria-modal="true"
          :aria-label="title"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ title }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="cancel">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <p class="message">{{ message }}</p>
          </div>

          <footer class="foot">
            <button class="btn-text" @click="cancel">{{ cancelLabel ?? i18n.t("common.cancel") }}</button>
            <button class="btn-filled" :class="{ danger }" @click="confirm">{{ confirmLabel }}</button>
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
  z-index: 140;
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
.btn-filled.danger {
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
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
