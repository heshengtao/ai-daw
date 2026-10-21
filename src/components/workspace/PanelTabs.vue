<script setup lang="ts">
/**
 * 面板标签栏：标签 + 「+」打开选择器。
 * 作为顶栏的一部分渲染（与顶栏同一行），不再单独占一行。
 */
import { onMounted, onUnmounted, ref } from "vue";
import {
  PlusIcon,
  XMarkIcon,
  AdjustmentsHorizontalIcon,
  AdjustmentsVerticalIcon,
  BookmarkSquareIcon,
  CircleStackIcon,
  MusicalNoteIcon,
  PuzzlePieceIcon,
  SparklesIcon,
  ChatBubbleLeftRightIcon,
} from "@heroicons/vue/24/outline";
import Tooltip from "@/components/common/Tooltip.vue";
import { PANES, PICKABLE_PANES, useLayoutStore, type PaneId } from "@/stores/layout";
import { useI18n } from "@/i18n";

const layout = useLayoutStore();
const i18n = useI18n();
const pickerOpen = ref(false);
const pickerRoot = ref<HTMLElement | null>(null);

/** 「+」里能挑的面板（不含「插件界面」，理由见 `stores/layout` 里的说明）。 */
const PICKABLE = PICKABLE_PANES;

const ICONS: Record<PaneId, unknown> = {
  inspector: AdjustmentsHorizontalIcon,
  mixer: AdjustmentsVerticalIcon,
  editor: MusicalNoteIcon,
  plugins: CircleStackIcon,
  manager: PuzzlePieceIcon,
  sounds: BookmarkSquareIcon,
  agent: ChatBubbleLeftRightIcon,
  ai: SparklesIcon,
};

function labelOf(id: PaneId): string {
  const key = PANES.find((p) => p.id === id)?.labelKey;
  return key ? i18n.t(key) : id;
}

function isOpen(id: PaneId): boolean {
  return layout.tabs.includes(id);
}

function pick(id: PaneId): void {
  // 用户是从列表里「选中」了这个面板——按习惯应当直接切过去。
  layout.openTab(id);
  pickerOpen.value = false;
}

function onDocPointer(e: PointerEvent): void {
  if (!pickerOpen.value) return;
  if (pickerRoot.value && !pickerRoot.value.contains(e.target as Node)) pickerOpen.value = false;
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape" && pickerOpen.value) pickerOpen.value = false;
}

onMounted(() => {
  document.addEventListener("pointerdown", onDocPointer);
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  document.removeEventListener("pointerdown", onDocPointer);
  window.removeEventListener("keydown", onKey);
});
</script>

<template>
  <div class="tabs" role="tablist">
    <div
      v-for="id in layout.tabs"
      :key="id"
      class="tab"
      :class="{ on: id === layout.activeTab }"
      role="tab"
      :aria-selected="id === layout.activeTab"
      @click="layout.activeTab = id"
    >
      <component :is="ICONS[id]" class="tab-icon" />
      <span class="tab-label">{{ labelOf(id) }}</span>
      <button class="tab-close" :aria-label="i18n.t('panes.close', { name: labelOf(id) })" @click.stop="layout.closeTab(id)">
        <XMarkIcon class="w-[13px] h-[13px]" />
      </button>
    </div>
  </div>

  <!-- 「+」紧跟最后一个标签，符合浏览器习惯 -->
  <div ref="pickerRoot" class="picker-root">
    <Tooltip :text="i18n.t('panes.addTooltip')" align="end">
      <button
        class="tab-add"
        :class="{ on: pickerOpen }"
        :aria-expanded="pickerOpen"
        :aria-label="i18n.t('panes.addTooltip')"
        @click="pickerOpen = !pickerOpen"
      >
        <PlusIcon class="w-[16px] h-[16px]" />
      </button>
    </Tooltip>

    <transition name="pop">
      <div v-if="pickerOpen" class="picker" role="menu">
        <div class="picker-title">{{ i18n.t("panes.title") }}</div>
        <button
          v-for="p in PICKABLE"
          :key="p.id"
          class="picker-item"
          role="menuitem"
          :disabled="isOpen(p.id) && layout.activeTab === p.id"
          @click="pick(p.id)"
        >
          <component :is="ICONS[p.id]" class="picker-icon" />
          <span class="picker-text">
            <span class="picker-label">{{ i18n.t(p.labelKey) }}</span>
            <span class="picker-hint">{{ i18n.t(p.hintKey) }}</span>
          </span>
          <span v-if="isOpen(p.id)" class="picker-badge">{{ i18n.t("panes.opened") }}</span>
        </button>
      </div>
    </transition>
  </div>
</template>

<style scoped>
.tabs {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 0 1 auto;
  min-width: 0;
  overflow-x: auto;
  scrollbar-width: none;
}
.tabs::-webkit-scrollbar {
  display: none;
}
.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 32px;
  padding: 0 6px 0 10px;
  border-radius: 10px;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  white-space: nowrap;
  flex: 0 0 auto;
  transition: background-color 0.15s ease, color 0.15s ease;
}
.tab:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.tab.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.tab-icon {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
}
.tab-label {
  font-size: 12px;
  font-weight: 500;
}
.tab-close {
  width: 20px;
  height: 20px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: inherit;
  opacity: 0.5;
  cursor: pointer;
  transition: opacity 0.15s ease, background-color 0.15s ease;
}
.tab:hover .tab-close {
  opacity: 1;
}
.tab-close:hover {
  background-color: rgb(var(--md-on-surface) / 0.12);
}
.tab-add {
  width: 28px;
  height: 28px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.15s ease, color 0.15s ease;
}
.tab-add:hover,
.tab-add.on {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.tab-add:focus-visible,
.tab-close:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}

/* ---- 新建标签选择器 ---- */
.picker-root {
  position: relative;
  flex: 0 0 auto;
}
.picker {
  position: absolute;
  top: calc(100% + 8px);
  right: 0;
  z-index: 60;
  width: 236px;
  padding: 8px;
  border-radius: 16px;
  background-color: rgb(var(--md-surface-container-high));
  border: 1px solid rgb(var(--md-outline-variant) / 0.4);
  box-shadow: var(--md-elevation-3);
}
.picker-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.5px;
  color: rgb(var(--md-on-surface-muted));
  padding: 4px 8px 8px;
}
.picker-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.15s ease;
}
.picker-item:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.picker-item:disabled {
  opacity: 0.5;
  cursor: default;
}
.picker-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
}
.picker-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}
.picker-label {
  font-size: 13px;
  font-weight: 500;
}
.picker-hint {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.picker-badge {
  font-size: 10px;
  padding: 2px 7px;
  border-radius: 999px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
}

.pop-enter-active {
  transition: opacity 0.16s ease, transform 0.22s cubic-bezier(0.34, 1.42, 0.5, 1);
}
.pop-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}
.pop-enter-from,
.pop-leave-to {
  opacity: 0;
  transform: translateY(-6px) scale(0.96);
}

@media (prefers-reduced-motion: reduce) {
  .tab,
  .tab-close,
  .tab-add,
  .picker-item,
  .pop-enter-active,
  .pop-leave-active {
    transition: none;
  }
}
</style>
