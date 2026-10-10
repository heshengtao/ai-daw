<script setup lang="ts">
/**
 * 快捷键速查表（模态框）：把散落在各处的快捷键集中展示。
 * 打开方式：顶栏的 ⌘ 按钮，或 Ctrl/Cmd+/ 与 `?`。
 */
import { computed } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";
import { isMacOS } from "@/utils/platform";

defineProps<{ open: boolean }>();
const emit = defineEmits<{ (e: "update:open", value: boolean): void }>();

const i18n = useI18n();

/** 修饰键在 macOS 用符号，其它平台用文字。 */
const mod = isMacOS() ? "⌘" : "Ctrl";
const shift = isMacOS() ? "⇧" : "Shift";

interface Item {
  keys: string[];
  labelKey: string;
}
interface Group {
  titleKey: string;
  items: Item[];
}

/** 分组展示；文案走 i18n，随语言切换即时更新。 */
const groups = computed<Group[]>(() => [
  {
    titleKey: "shortcuts.group.transport",
    items: [
      { keys: ["Space"], labelKey: "shortcuts.playPause" },
      { keys: ["Home"], labelKey: "shortcuts.toStart" },
    ],
  },
  {
    titleKey: "shortcuts.group.edit",
    items: [
      { keys: [mod, "Z"], labelKey: "shortcuts.undo" },
      { keys: [mod, shift, "Z"], labelKey: "shortcuts.redo" },
      { keys: ["Delete"], labelKey: "shortcuts.delete" },
      { keys: [mod, "C"], labelKey: "shortcuts.copy" },
      { keys: [mod, "V"], labelKey: "shortcuts.paste" },
    ],
  },
  {
    titleKey: "shortcuts.group.project",
    items: [
      { keys: [mod, "S"], labelKey: "shortcuts.save" },
      { keys: [mod, shift, "S"], labelKey: "shortcuts.saveAs" },
      { keys: [mod, "B"], labelKey: "shortcuts.toggleTracks" },
    ],
  },
  {
    titleKey: "shortcuts.group.tools",
    items: [
      { keys: ["1"], labelKey: "shortcuts.toolDraw" },
      { keys: ["2"], labelKey: "shortcuts.toolSelect" },
      { keys: ["3"], labelKey: "shortcuts.toolSplit" },
      { keys: [mod, i18n.t("shortcuts.wheel")], labelKey: "shortcuts.zoom" },
      { keys: [mod, shift, "D"], labelKey: "shortcuts.devLog" },
    ],
  },
]);

function close(): void {
  emit("update:open", false);
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
          :aria-label="i18n.t('shortcuts.title')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ i18n.t("shortcuts.title") }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="close">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="body">
            <section v-for="g in groups" :key="g.titleKey" class="group">
              <h3 class="group-title">{{ i18n.t(g.titleKey) }}</h3>
              <div v-for="it in g.items" :key="it.labelKey" class="row">
                <span class="row-label">{{ i18n.t(it.labelKey) }}</span>
                <span class="keys">
                  <template v-for="(k, i) in it.keys" :key="i">
                    <span v-if="i > 0" class="plus">+</span>
                    <kbd class="key">{{ k }}</kbd>
                  </template>
                </span>
              </div>
            </section>
          </div>
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
  width: 480px;
  max-width: 100%;
  max-height: 80vh;
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
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 22px 20px;
}
.group + .group {
  margin-top: 18px;
}
.group-title {
  margin: 0 0 6px;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.4px;
  color: rgb(var(--md-primary));
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  min-height: 34px;
  padding: 3px 0;
}
.row + .row {
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.25);
}
.row-label {
  font-size: 13px;
  color: rgb(var(--md-on-surface));
}
.keys {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex: 0 0 auto;
}
.key {
  min-width: 22px;
  height: 22px;
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  font-family: inherit;
  font-weight: 600;
  line-height: 1;
}
.plus {
  color: rgb(var(--md-on-surface-muted));
  font-size: 10px;
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
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>
