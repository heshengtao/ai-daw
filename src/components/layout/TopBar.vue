<script setup lang="ts">
/**
 * 唯一的顶部栏：窗口拖拽区 + 左/右栏开关 + 工程名 + 导航 + 工具按钮。
 * 所有按钮都是图标 + tooltip，窄屏逐级隐藏，绝不换行。
 */
import { computed } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Squares2X2Icon,
  PencilSquareIcon,
  AdjustmentsHorizontalIcon,
  CircleStackIcon,
  ComputerDesktopIcon,
  SunIcon,
  MoonIcon,
  CommandLineIcon,
  ArrowDownTrayIcon,
  ArrowUpTrayIcon,
  PlusIcon,
} from "@heroicons/vue/24/outline";
import WindowControls from "./WindowControls.vue";
import SegmentedNav from "@/components/common/SegmentedNav.vue";
import type { NavItem } from "@/components/common/nav";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useTheme } from "@/composables/useTheme";
import { isLinux, isMacOS, isTauri, isWindows } from "@/utils/platform";

const emit = defineEmits<{
  (e: "demo"): void;
  (e: "new"): void;
  (e: "export"): void;
}>();

const layout = useLayoutStore();
const projectStore = useProjectStore();
const { theme, cycleTheme } = useTheme();

const isMac = isMacOS();
const showWindowControls = isTauri() && (isWindows() || isLinux());

const navItems: NavItem[] = [
  { label: "编排", path: "/arrange", icon: Squares2X2Icon },
  { label: "编辑", path: "/edit", icon: PencilSquareIcon },
  { label: "混音", path: "/mix", icon: AdjustmentsHorizontalIcon },
  { label: "浏览", path: "/browse", icon: CircleStackIcon },
];

/** 工程名（空工程时给一个可读的默认文案）。 */
const projectName = computed(() => {
  const name = projectStore.project?.name;
  return !name || name === "untitled" ? "未命名工程" : name;
});

function onMouseDown(e: MouseEvent): void {
  if (!isTauri()) return;
  if (e.buttons !== 1) return;
  const target = e.target as HTMLElement;
  if (target.closest("button, a, input, select")) return;
  const win = getCurrentWindow();
  if (e.detail === 2) {
    void win.toggleMaximize();
  } else {
    void win.startDragging();
  }
}
</script>

<template>
  <header
    class="top-bar"
    :class="isMac ? 'pad-mac' : ''"
    @mousedown="onMouseDown"
  >
    <!-- 左栏开关 -->
    <button
      class="bar-btn"
      :class="{ on: layout.showTrackHeaders }"
      :title="layout.showTrackHeaders ? '收起左侧轨道列表' : '展开左侧轨道列表'"
      @click="layout.showTrackHeaders = !layout.showTrackHeaders"
    >
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <rect x="2.8" y="4.8" width="18.4" height="14.4" rx="3" stroke="currentColor" stroke-width="1.6" />
        <line x1="9.6" y1="4.8" x2="9.6" y2="19.2" stroke="currentColor" stroke-width="1.6" />
        <path v-if="layout.showTrackHeaders" d="M5.8 4.8h3.8v14.4H5.8a3 3 0 0 1-3-3V7.8a3 3 0 0 1 3-3z" fill="currentColor" />
      </svg>
    </button>

    <span class="bar-sep" />

    <!-- 名称：粗体工程名 + 灰字应用名 -->
    <div class="name-block">
      <span class="name-strong">{{ projectName }}</span>
      <span class="name-muted">daw-party</span>
    </div>

    <!-- 视图导航（窄屏隐藏） -->
    <SegmentedNav :items="navItems" class="bar-nav" />

    <div class="flex-1" />

    <div class="bar-actions">
      <button class="bar-btn" title="载入示例工程" @click="emit('demo')">
        <ArrowDownTrayIcon class="w-[18px] h-[18px]" />
      </button>
      <button class="bar-btn" title="新建工程" @click="emit('new')">
        <PlusIcon class="w-[18px] h-[18px]" />
      </button>
      <button class="bar-btn" title="导出 WAV" @click="emit('export')">
        <ArrowUpTrayIcon class="w-[18px] h-[18px]" />
      </button>

      <span class="bar-sep" />

      <button class="bar-btn" title="切换主题" @click="cycleTheme">
        <SunIcon v-if="theme === 'light'" class="w-[18px] h-[18px]" />
        <MoonIcon v-else-if="theme === 'dark'" class="w-[18px] h-[18px]" />
        <ComputerDesktopIcon v-else class="w-[18px] h-[18px]" />
      </button>
      <button
        class="bar-btn"
        :class="{ on: layout.devDrawer }"
        title="开发者日志 (Ctrl/Cmd+Shift+D)"
        @click="layout.toggleDevDrawer()"
      >
        <CommandLineIcon class="w-[18px] h-[18px]" />
      </button>

      <!-- 右栏开关 -->
      <button
        class="bar-btn"
        :class="{ on: layout.showInspector }"
        :title="layout.showInspector ? '收起右侧检查器' : '展开右侧检查器'"
        @click="layout.showInspector = !layout.showInspector"
      >
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <rect x="2.8" y="4.8" width="18.4" height="14.4" rx="3" stroke="currentColor" stroke-width="1.6" />
          <line x1="14.4" y1="4.8" x2="14.4" y2="19.2" stroke="currentColor" stroke-width="1.6" />
          <path v-if="layout.showInspector" d="M14.4 4.8h3.8a3 3 0 0 1 3 3v8.4a3 3 0 0 1-3 3h-3.8z" fill="currentColor" />
        </svg>
      </button>

      <WindowControls v-if="showWindowControls" class="h-full" />
    </div>
  </header>
</template>

<style scoped>
.top-bar {
  height: 44px;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  background-color: rgb(var(--md-surface-container));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
  user-select: none;
}
/* macOS：给红黄绿让位 */
.pad-mac {
  padding-left: 78px;
}
.bar-sep {
  width: 1px;
  height: 18px;
  margin: 0 4px;
  background-color: rgb(var(--md-outline-variant) / 0.55);
  flex: 0 0 auto;
}
.name-block {
  display: flex;
  align-items: baseline;
  gap: 7px;
  min-width: 0;
  flex: 0 1 auto;
}
.name-strong {
  font-weight: 700;
  font-size: 13px;
  letter-spacing: -0.1px;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.name-muted {
  font-size: 12px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.bar-nav {
  margin-left: 8px;
  flex: 0 1 auto;
  min-width: 0;
}
.bar-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: 0 0 auto;
}
/* 顶栏专用的小号图标按钮 */
.bar-btn {
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.15s ease, color 0.15s ease;
}
.bar-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.bar-btn:active {
  transform: scale(0.94);
}
.bar-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.bar-btn.on {
  color: rgb(var(--md-primary));
}

/* 窄屏：逐级让位，先收导航、再收次要按钮，永远不换行 */
@media (max-width: 1100px) {
  .bar-nav {
    display: none;
  }
}
@media (max-width: 720px) {
  .name-muted {
    display: none;
  }
}

@media (prefers-reduced-motion: reduce) {
  .bar-btn:active {
    transform: none;
  }
}
</style>
