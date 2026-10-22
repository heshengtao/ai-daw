<script setup lang="ts">
/**
 * 顶栏：左侧工程名 + 工具区；右侧是「右侧面板的标签栏 + 面板开关」。
 * 顶栏横跨"工作区 + 右侧面板"两列，右端恒等于窗口右端——
 * 因此面板开关固定在顶栏最右，展开/收起时鼠标不用移动。
 */
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ClockIcon,
  FolderOpenIcon,
  ArrowUpTrayIcon,
  PlusIcon,
  Cog6ToothIcon,
  Squares2X2Icon,
  PencilSquareIcon,
  AdjustmentsHorizontalIcon,
  MusicalNoteIcon,
} from "@heroicons/vue/24/outline";
import Tooltip from "@/components/common/Tooltip.vue";
import PanelIcon from "@/components/common/PanelIcon.vue";
import PanelTabs from "@/components/workspace/PanelTabs.vue";
import FloppyDiskIcon from "@/components/common/icons/FloppyDiskIcon.vue";
import GitHubIcon from "@/components/common/icons/GitHubIcon.vue";
import PianoIcon from "@/components/common/icons/PianoIcon.vue";
import KeyboardIcon from "@/components/common/icons/KeyboardIcon.vue";
import type { NavItem } from "@/components/common/nav";
import { managerOpen } from "@/api/tauri";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { lastWorkspace } from "@/router";
import { isMacOS, isTauri } from "@/utils/platform";

const emit = defineEmits<{
  (e: "import"): void;
  (e: "importAudio"): void;
  (e: "new"): void;
  (e: "history"): void;
  (e: "save"): void;
  (e: "export"): void;
  (e: "rename"): void;
  (e: "shortcuts"): void;
}>();

const route = useRoute();
const router = useRouter();
const layout = useLayoutStore();
const projectStore = useProjectStore();
const i18n = useI18n();

const isMac = isMacOS();

const navItems: NavItem[] = [
  { labelKey: "topbar.view.arrange", path: "/arrange", icon: Squares2X2Icon },
  { labelKey: "topbar.view.edit", path: "/edit", icon: PianoIcon },
  { labelKey: "topbar.view.mix", path: "/mix", icon: AdjustmentsHorizontalIcon },
];

const isSettings = computed(() => route.path.startsWith("/settings"));

function isActive(path: string): boolean {
  return route.path === path || route.path.startsWith(`${path}/`);
}

const projectName = computed(() => {
  const name = projectStore.project?.name;
  return !name || name === "untitled" ? i18n.t("settings.untitled") : name;
});

/** 有未保存改动时在工程名旁显示一个小圆点。 */
const dirty = computed(() => projectStore.dirty);

/** 音轨栏收起（或处于设置页，侧栏本就不渲染）时，顶栏顶到窗口最左，macOS 下要给红绿灯让位。 */
const showMacPad = computed(() => isMac && (isSettings.value || !layout.showTrackHeaders));

/** 仓库地址：顶栏入口用它，Issue / PR 也指向这里。 */
const REPO_URL = "https://github.com/heshengtao/ai-daw";

/** 设置按钮：进入设置，或从设置返回上一个工作区视图。 */
function toggleSettings(): void {
  if (isSettings.value) void router.push(lastWorkspace.value);
  else void router.push("/settings");
}

/** 在系统浏览器里打开 GitHub 仓库（浏览器调试时退回新标签页）。 */
async function openRepo(): Promise<void> {
  if (!isTauri()) {
    window.open(REPO_URL, "_blank", "noopener,noreferrer");
    return;
  }
  try {
    await managerOpen(REPO_URL);
  } catch {
    window.open(REPO_URL, "_blank", "noopener,noreferrer");
  }
}

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
  <header class="top-bar" :class="{ 'pad-mac': showMacPad }" @mousedown="onMouseDown">
    <!-- 左：工程名 + 工具区 -->
    <div class="bar-main">
      <Tooltip v-if="!isSettings && !layout.showTrackHeaders" :text="i18n.t('topbar.showTracks')">
        <button class="bar-btn" :aria-label="i18n.t('topbar.showTracksLabel')" @click="layout.showTrackHeaders = true">
          <PanelIcon side="left" :filled="false" class="bar-icon" />
        </button>
      </Tooltip>

      <div class="name-block">
        <span class="name-strong">{{ projectName }}</span>
        <span v-if="dirty" class="name-dirty" :title="i18n.t('topbar.unsaved')" aria-hidden="true" />
      </div>
      <Tooltip :text="i18n.t('topbar.rename')">
        <button class="bar-btn name-edit" :aria-label="i18n.t('topbar.renameLabel')" @click="emit('rename')">
          <PencilSquareIcon class="bar-icon" />
        </button>
      </Tooltip>

      <div class="flex-1" />

      <div class="bar-actions">
        <Tooltip :text="i18n.t('topbar.history')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.historyLabel')" @click="emit('history')">
            <ClockIcon class="bar-icon" />
          </button>
        </Tooltip>
        <Tooltip :text="i18n.t('topbar.import')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.importLabel')" @click="emit('import')">
            <FolderOpenIcon class="bar-icon" />
          </button>
        </Tooltip>
        <Tooltip :text="i18n.t('topbar.importAudio')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.importAudioLabel')" @click="emit('importAudio')">
            <MusicalNoteIcon class="bar-icon" />
          </button>
        </Tooltip>
        <Tooltip :text="i18n.t('topbar.new')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.new')" @click="emit('new')">
            <PlusIcon class="bar-icon" />
          </button>
        </Tooltip>
        <Tooltip :text="i18n.t('topbar.export')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.exportLabel')" @click="emit('export')">
            <ArrowUpTrayIcon class="bar-icon" />
          </button>
        </Tooltip>
        <Tooltip :text="i18n.t('topbar.save')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.saveLabel')" @click="emit('save')">
            <FloppyDiskIcon class="bar-icon" />
          </button>
        </Tooltip>

        <span class="bar-sep" />

        <Tooltip v-for="item in navItems" :key="item.path" :text="i18n.t(item.labelKey)" align="end">
          <button
            class="bar-btn"
            :class="{ on: isActive(item.path) && !isSettings }"
            :aria-label="i18n.t(item.labelKey)"
            :aria-current="isActive(item.path) && !isSettings ? 'page' : undefined"
            @click="router.push(item.path)"
          >
            <component :is="item.icon" class="bar-icon" />
          </button>
        </Tooltip>

        <span class="bar-sep" />

        <Tooltip :text="i18n.t('shortcuts.title')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('shortcuts.title')" @click="emit('shortcuts')">
            <KeyboardIcon class="bar-icon" />
          </button>
        </Tooltip>

        <Tooltip :text="i18n.t('topbar.github')" align="end">
          <button class="bar-btn" :aria-label="i18n.t('topbar.github')" @click="openRepo">
            <GitHubIcon class="bar-icon" />
          </button>
        </Tooltip>

        <Tooltip :text="isSettings ? i18n.t('topbar.backToWorkspace') : i18n.t('settings.title')" align="end">
          <button
            class="bar-btn"
            :class="{ on: isSettings }"
            :aria-label="isSettings ? i18n.t('topbar.backToWorkspace') : i18n.t('settings.title')"
            :aria-current="isSettings ? 'page' : undefined"
            @click="toggleSettings"
          >
            <Cog6ToothIcon class="bar-icon" />
          </button>
        </Tooltip>
      </div>
    </div>

    <!-- 右：右侧面板的标签栏（与下方面板同宽、同一行）+ 面板开关 -->
    <div
      v-if="!isSettings"
      class="bar-tabs"
      :class="{ 'has-panel': layout.showInspector }"
      :style="layout.showInspector ? { width: `${layout.inspectorWidth}px` } : undefined"
    >
      <PanelTabs v-if="layout.showInspector" />

      <Tooltip
        class="tabs-toggle"
        :text="layout.showInspector ? i18n.t('topbar.hidePanel') : i18n.t('topbar.showPanel')"
        align="end"
      >
        <button
          class="bar-btn"
          :class="{ on: layout.showInspector }"
          :aria-label="layout.showInspector ? i18n.t('topbar.hidePanel') : i18n.t('topbar.showPanel')"
          @click="layout.showInspector = !layout.showInspector"
        >
          <PanelIcon side="right" :filled="layout.showInspector" class="bar-icon" />
        </button>
      </Tooltip>
    </div>
  </header>
</template>

<style scoped>
.top-bar {
  height: 52px;
  display: flex;
  align-items: center;
  padding: 0;
  background-color: var(--app-card);
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.4);
  flex: 0 0 auto;
  user-select: none;
}
.bar-main {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 10px;
}
/* 音轨栏收起后顶栏顶到窗口最左：macOS 下给红绿灯让位 */
.top-bar.pad-mac .bar-main {
  padding-left: 78px;
}

/* ---- 右侧面板的标签区 ---- */
.bar-tabs {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  /* 顶栏右端始终是窗口右端：右内边距恒为 12px + 窗口按钮预留 */
  padding-right: calc(12px + var(--win-reserve));
}
.bar-tabs.has-panel {
  padding-left: 8px;
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.4);
}
/* 开关永远贴在这一段的最右 */
.tabs-toggle {
  margin-left: auto;
}

.name-block {
  display: flex;
  align-items: baseline;
  gap: 7px;
  min-width: 0;
  flex: 0 1 auto;
}
.name-strong {
  font-weight: 500;
  font-size: 14px;
  letter-spacing: -0.1px;
  color: rgb(var(--md-on-surface));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 未保存标记：紧贴工程名的小圆点 */
.name-dirty {
  width: 7px;
  height: 7px;
  flex: 0 0 auto;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  align-self: center;
}
/* 重命名按钮比普通图标按钮稍小，紧跟在工程名后面（用复合选择器压过 .bar-btn 的尺寸） */
.bar-btn.name-edit {
  width: 28px;
  height: 28px;
}
.bar-btn.name-edit .bar-icon {
  width: 16px;
  height: 16px;
}
.bar-actions {
  display: flex;
  align-items: center;
  gap: 1px;
  flex: 0 0 auto;
}
.bar-sep {
  width: 1px;
  height: 20px;
  margin: 0 5px;
  background-color: rgb(var(--md-outline-variant) / 0.55);
  flex: 0 0 auto;
}
/* MD3 图标按钮：圆形，无原生外观 */
.bar-btn {
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
.bar-icon {
  width: 18px;
  height: 18px;
  flex: 0 0 auto;
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
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
/* 禁用（当前视图暂时没有底部窗格）：变灰、不可点、悬停/按下都不亮 */
.bar-btn:disabled,
.bar-btn:disabled:hover,
.bar-btn:disabled:active {
  opacity: 0.38;
  cursor: default;
  color: rgb(var(--md-on-surface-variant));
  background-color: transparent;
  transform: none;
}

@media (prefers-reduced-motion: reduce) {
  .bar-btn {
    transition: none;
  }
  .bar-btn:active {
    transform: none;
  }
}
</style>
