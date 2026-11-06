<script setup lang="ts">
/**
 * 右侧面板：只负责渲染当前标签的内容。
 * 标签栏（标签 / 「+」/ 开关）在顶栏里，与本面板同宽、同一行。
 */
import {
  AdjustmentsHorizontalIcon,
  AdjustmentsVerticalIcon,
  BookmarkSquareIcon,
  CircleStackIcon,
  MusicalNoteIcon,
  PuzzlePieceIcon,
  SparklesIcon,
  ChatBubbleLeftRightIcon,
  ListBulletIcon,
  ChevronRightIcon,
} from "@heroicons/vue/24/outline";
import InspectorPanel from "./InspectorPanel.vue";
import MixerPanel from "./panes/MixerPanel.vue";
import PluginEditorPane from "./panes/PluginEditorPane.vue";
import PluginsPane from "./panes/PluginsPane.vue";
import ManagerPane from "./panes/ManagerPane.vue";
import SoundLibraryPane from "./panes/SoundLibraryPane.vue";
import OpsPane from "./panes/OpsPane.vue";
import AiPanel from "./panes/AiPanel.vue";
import EmptyStack from "@/components/onboarding/EmptyStack.vue";
import { defineAsyncComponent } from "vue";
import { PICKABLE_PANES, useLayoutStore, type PaneId } from "@/stores/layout";
import { useI18n } from "@/i18n";

// 写歌智能体带着 markdown 渲染（marked + dompurify），按需加载、别拖累首屏。
const SongAgentPanel = defineAsyncComponent(() => import("./agent/SongAgentPanel.vue"));

const layout = useLayoutStore();
const i18n = useI18n();

const ICONS: Record<PaneId, unknown> = {
  inspector: AdjustmentsHorizontalIcon,
  mixer: AdjustmentsVerticalIcon,
  editor: MusicalNoteIcon,
  plugins: CircleStackIcon,
  manager: PuzzlePieceIcon,
  sounds: BookmarkSquareIcon,
  ops: ListBulletIcon,
  agent: ChatBubbleLeftRightIcon,
  ai: SparklesIcon,
};
</script>

<template>
  <aside class="right-panel" data-tour="panels">
    <div class="pane-body">
      <!-- 换标签时做一次淡入 + 轻微上浮：面板内容整块替换，不加过渡会像「闪」了一下 -->
      <transition name="pane" mode="out-in">
        <!-- 没有标签时：列出所有可以打开的面板 -->
        <div v-if="layout.tabs.length === 0" class="empty" key="empty">
          <EmptyStack class="empty-art" />
          <p class="empty-title">{{ i18n.t("right.emptyTitle") }}</p>
          <p class="empty-sub">{{ i18n.t("right.emptyHint") }}</p>
          <ul class="empty-list">
            <li v-for="p in PICKABLE_PANES" :key="p.id">
              <button class="empty-item" @click="layout.openTab(p.id)">
                <span class="empty-icon-wrap">
                  <component :is="ICONS[p.id]" class="empty-icon" />
                </span>
                <span class="empty-text">
                  <span class="empty-label">{{ i18n.t(p.labelKey) }}</span>
                  <span class="empty-hint">{{ i18n.t(p.hintKey) }}</span>
                </span>
                <ChevronRightIcon class="empty-arrow" />
              </button>
            </li>
          </ul>
        </div>

        <InspectorPanel v-else-if="layout.activeTab === 'inspector'" />
        <MixerPanel v-else-if="layout.activeTab === 'mixer'" />
        <PluginEditorPane v-else-if="layout.activeTab === 'editor'" />
        <PluginsPane v-else-if="layout.activeTab === 'plugins'" />
        <ManagerPane v-else-if="layout.activeTab === 'manager'" />
        <SoundLibraryPane v-else-if="layout.activeTab === 'sounds'" />
        <OpsPane v-else-if="layout.activeTab === 'ops'" />
        <SongAgentPanel v-else-if="layout.activeTab === 'agent'" />
        <AiPanel v-else-if="layout.activeTab === 'ai'" />
      </transition>
    </div>
  </aside>
</template>

<style scoped>
.right-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background-color: var(--app-card);
  overflow: hidden;
}
.pane-body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
}

/* ---- 空态：可打开面板列表 ---- */
.empty {
  flex: 1;
  min-height: 0;
  width: 100%;
  display: flex;
  flex-direction: column;
  padding: 26px 14px 20px;
  overflow-y: auto;
}
.empty-art {
  width: 132px;
  margin: 0 auto 12px;
  opacity: 0.95;
}
.empty-title {
  margin: 0;
  text-align: center;
  font-size: 13px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.empty-sub {
  margin: 4px 0 16px;
  text-align: center;
  font-size: 11.5px;
  color: rgb(var(--md-on-surface-variant));
}
.empty-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.empty-item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 9px 10px;
  border: 0;
  border-radius: 14px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard);
}
.empty-item:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.empty-item:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 图标放在一个圆角小方块里：比裸图标更像「入口」，也为将来换图标留了底 */
.empty-icon-wrap {
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  transition: transform var(--md-duration-medium-1) var(--md-ease-spring-fast);
}
.empty-item:hover .empty-icon-wrap {
  transform: scale(1.06);
}
.empty-icon {
  width: 16px;
  height: 16px;
}
.empty-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}
.empty-label {
  font-size: 12.5px;
  font-weight: 500;
}
.empty-hint {
  font-size: 11px;
  line-height: 1.4;
  color: rgb(var(--md-on-surface-variant));
}
.empty-arrow {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-muted));
  opacity: 0;
  transform: translateX(-3px);
  transition:
    opacity var(--md-duration-short-2) var(--md-ease-standard),
    transform var(--md-duration-medium-1) var(--md-ease-spring-fast);
}
.empty-item:hover .empty-arrow {
  opacity: 1;
  transform: translateX(0);
}
</style>
