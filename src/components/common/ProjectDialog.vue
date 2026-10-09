<script setup lang="ts">
/**
 * 工程名对话框：重命名（顶栏「编辑」按钮）与「保存工程」共用。
 *
 * `mode` 只影响标题和确认按钮文案；提交时统一把一个**非空**工程名交给上层——
 * 重命名模式下它就是最终名字；保存模式下上层拿到名字后继续弹出文件选择框。
 */
import { nextTick, ref, watch } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

const props = withDefaults(
  defineProps<{
    open: boolean;
    mode?: "rename" | "save";
    /** 打开时的初始名字（未命名工程传空串）。 */
    initialName?: string;
  }>(),
  { mode: "rename", initialName: "" },
);

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "submit", name: string): void;
}>();

const i18n = useI18n();
const name = ref("");
const inputEl = ref<HTMLInputElement | null>(null);

// 每次打开都重置为初始名并聚焦，方便直接改名。
watch(
  () => props.open,
  (v) => {
    if (!v) return;
    name.value = props.initialName;
    void nextTick(() => {
      inputEl.value?.focus();
      inputEl.value?.select();
    });
  },
);

function close(): void {
  emit("update:open", false);
}

function submit(): void {
  const trimmed = name.value.trim();
  if (!trimmed) return;
  emit("submit", trimmed);
  close();
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    close();
  }
}
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="close">
        <div
          class="dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="i18n.t(mode === 'save' ? 'projectDialog.saveTitle' : 'projectDialog.renameTitle')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">
              {{ i18n.t(mode === "save" ? "projectDialog.saveTitle" : "projectDialog.renameTitle") }}
            </h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="close">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <label class="field-label" for="project-name-input">
              {{ i18n.t("projectDialog.nameLabel") }}
            </label>
            <input
              id="project-name-input"
              ref="inputEl"
              v-model="name"
              class="field md3-field"
              type="text"
              spellcheck="false"
              autocomplete="off"
              :placeholder="i18n.t('settings.untitled')"
              :aria-label="i18n.t('projectDialog.nameLabel')"
              @keydown.enter.prevent="submit"
            />
            <p v-if="mode === 'save'" class="hint">{{ i18n.t("projectDialog.saveHint") }}</p>
          </div>

          <footer class="foot">
            <button class="btn-text" @click="close">{{ i18n.t("common.cancel") }}</button>
            <button class="btn-filled" :disabled="!name.trim()" @click="submit">
              {{ i18n.t(mode === "save" ? "projectDialog.confirmSave" : "common.save") }}
            </button>
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
  justify-content: space-between;
  gap: 12px;
  padding: 16px 16px 8px 22px;
}
.title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
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
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 4px 22px 8px;
}
.field-label {
  font-size: 12px;
  color: rgb(var(--md-on-surface-variant));
}
.field {
  width: 100%;
  height: 40px;
  padding: 0 14px;
  font-size: 14px;
  color: rgb(var(--md-on-surface));
}
.hint {
  margin: 0;
  font-size: 11.5px;
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
.btn-filled:disabled {
  opacity: 0.4;
  cursor: default;
  filter: none;
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
