<script setup lang="ts">
/**
 * 浮动工具岛（中栏右下角）：鼠标模式三选一 + 底部窗格开关，共 4 个按钮。
 *
 * 从顶栏挪下来，免得顶栏图标越堆越多。只在**有底部窗格**的视图（编排 / 编辑）出现——
 * 那两个视图才用得到鼠标模式与底部窗格；混音 / 设置页不渲染。
 */
import { computed } from "vue";
import { useRoute } from "vue-router";
import { PaintBrushIcon } from "@heroicons/vue/24/outline";
import Tooltip from "@/components/common/Tooltip.vue";
import PanelIcon from "@/components/common/PanelIcon.vue";
import MarqueeIcon from "@/components/common/icons/MarqueeIcon.vue";
import SplitIcon from "@/components/common/icons/SplitIcon.vue";
import { useLayoutStore, type Tool } from "@/stores/layout";
import { useI18n } from "@/i18n";

const route = useRoute();
const layout = useLayoutStore();
const i18n = useI18n();

function isActive(path: string): boolean {
  return route.path === path || route.path.startsWith(`${path}/`);
}

/** 只有编排（自动化车道）和编辑（控制器泳道）才有底部窗格。 */
const hasBottomLane = computed(() => isActive("/arrange") || isActive("/edit"));

/** 当前视图的底部窗格是否打开。 */
const bottomLaneOn = computed(() =>
  isActive("/edit") ? layout.showControllerLane : layout.showAutomation,
);

/**
 * 底部窗格的高度（px）：两个视图**共用**一份 `laneHeight`，窗格自身的固定开销也一致
 * （1px 顶边框 + 30px 表头），所以切换页面时窗格高度不变、浮岛也就不会上下跳。
 */
const LANE_CHROME = 31;

/** 当前视图的底部窗格整体高度（px）；未展开或该视图没有窗格时为 0。 */
const paneHeight = computed(() => {
  if (isActive("/edit")) return layout.showControllerLane ? layout.laneHeight + LANE_CHROME : 0;
  if (isActive("/arrange")) return layout.showAutomation ? layout.laneHeight + LANE_CHROME : 0;
  return 0;
});

/** 浮岛离容器底部的距离：窗格展开时抬到它上方（含 1px 分隔线），收起时贴底。 */
const dockBottom = computed(() => `${paneHeight.value + (paneHeight.value > 0 ? 1 : 0) + 14}px`);

/** 开关**当前视图**的底部窗格。 */
function toggleBottomLane(): void {
  if (isActive("/edit")) layout.showControllerLane = !layout.showControllerLane;
  else layout.showAutomation = !layout.showAutomation;
}

/** 三种鼠标模式（顺序即展示顺序）：绘制 / 框选 / 分隔。 */
const toolOptions: { id: Tool; icon: unknown; labelKey: string }[] = [
  { id: "draw", icon: PaintBrushIcon, labelKey: "topbar.toolDraw" },
  { id: "select", icon: MarqueeIcon, labelKey: "topbar.toolSelect" },
  { id: "split", icon: SplitIcon, labelKey: "topbar.toolSplit" },
];
</script>

<template>
  <div
    v-if="hasBottomLane"
    class="tool-dock"
    role="toolbar"
    :aria-label="i18n.t('topbar.toolGroup')"
    :style="{ bottom: dockBottom }"
  >
    <Tooltip v-for="t in toolOptions" :key="t.id" :text="i18n.t(t.labelKey)" place="above">
      <button
        class="dock-btn"
        :class="{ on: layout.tool === t.id }"
        :aria-label="i18n.t(t.labelKey)"
        :aria-pressed="layout.tool === t.id"
        @click="layout.setTool(t.id)"
      >
        <component :is="t.icon" class="dock-icon" />
      </button>
    </Tooltip>

    <span class="dock-sep" />

    <Tooltip :text="i18n.t('topbar.bottomLane')" place="above">
      <button
        class="dock-btn"
        :class="{ on: bottomLaneOn }"
        :aria-label="i18n.t('topbar.bottomLaneLabel')"
        :aria-pressed="bottomLaneOn"
        @click="toggleBottomLane"
      >
        <PanelIcon side="bottom" :filled="bottomLaneOn" class="dock-icon" />
      </button>
    </Tooltip>
  </div>
</template>

<style scoped>
/* 浮岛：贴中栏右下角，压在工作区内容之上 */
.tool-dock {
  position: absolute;
  right: 14px;
  z-index: 40;
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 5px;
  border-radius: 999px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.5);
  background-color: rgb(var(--md-surface-container-high) / 0.92);
  -webkit-backdrop-filter: blur(12px);
  backdrop-filter: blur(12px);
  box-shadow: var(--md-elevation-2);
  user-select: none;
}
.dock-btn {
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
  transition: background-color 0.15s ease, color 0.15s ease, transform 0.15s ease;
}
.dock-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.dock-btn:active {
  transform: scale(0.94);
}
.dock-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.dock-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.dock-icon {
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
}
.dock-sep {
  width: 1px;
  height: 20px;
  margin: 0 3px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-outline-variant) / 0.55);
}
@media (prefers-reduced-motion: reduce) {
  .dock-btn {
    transition: none;
  }
  .dock-btn:active {
    transform: none;
  }
}
</style>
