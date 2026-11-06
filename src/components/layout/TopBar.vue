<script setup lang="ts">
/**
 * 顶栏：左侧工程名，中间是**带文字的四个视图**，右侧是动作区。
 *
 * 这一版跟上一版的差别只有一件事：**把「是什么」写出来**。
 * 以前右边挤了十多个纯图标按钮（历史 / 导入 / 导入音频 / 新建 / 导出 / 保存 / 四个视图 /
 * 快捷键 / GitHub / 设置），新用户得逐个悬停才知道是什么。现在：
 *
 * - 四个视图做成带图标 + 文字的胶囊（当前项有滑块），一眼看出「我在哪个工作区」；
 * - 文件类动作收进「文件」菜单，每一项都有完整名称和快捷键；
 * - 「帮助」菜单放快速上手 / 快捷键 / 仓库；
 * - 只把两件事留在明面上：**保存**和**AI 写歌**（这是这个应用的主张）。
 *
 * 顶栏横跨「工作区 + 右侧面板」两列，右端恒等于窗口右端——
 * 因此面板开关固定在顶栏最右，展开/收起时鼠标不用移动。
 */
import { computed, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ClockIcon,
  FolderOpenIcon,
  ArrowUpTrayIcon,
  ArrowDownTrayIcon,
  PlusIcon,
  Cog6ToothIcon,
  Squares2X2Icon,
  PencilSquareIcon,
  AdjustmentsHorizontalIcon,
  DocumentTextIcon,
  QuestionMarkCircleIcon,
  SparklesIcon,
  SquaresPlusIcon,
} from "@heroicons/vue/24/outline";
import Tooltip from "@/components/common/Tooltip.vue";
import MenuButton from "@/components/common/MenuButton.vue";
import type { MenuSection } from "@/components/common/menu";
import PanelIcon from "@/components/common/PanelIcon.vue";
import PanelTabs from "@/components/workspace/PanelTabs.vue";
import FloppyDiskIcon from "@/components/common/icons/FloppyDiskIcon.vue";
import GitHubIcon from "@/components/common/icons/GitHubIcon.vue";
import PianoIcon from "@/components/common/icons/PianoIcon.vue";
import KeyboardIcon from "@/components/common/icons/KeyboardIcon.vue";
import type { NavItem } from "@/components/common/nav";
import { openExternal } from "@/utils/external";
import { useLayoutStore } from "@/stores/layout";
import { useOnboardingStore } from "@/stores/onboarding";
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
  (e: "saveAs"): void;
  (e: "export"): void;
  (e: "rename"): void;
  (e: "shortcuts"): void;
}>();

const route = useRoute();
const router = useRouter();
const layout = useLayoutStore();
const onboarding = useOnboardingStore();
const projectStore = useProjectStore();
const i18n = useI18n();

const isMac = isMacOS();

const navItems: NavItem[] = [
  { labelKey: "topbar.view.arrange", path: "/arrange", icon: Squares2X2Icon },
  { labelKey: "topbar.view.edit", path: "/edit", icon: PianoIcon },
  { labelKey: "topbar.view.score", path: "/score", icon: DocumentTextIcon },
  { labelKey: "topbar.view.mix", path: "/mix", icon: AdjustmentsHorizontalIcon },
];

const fileOpen = ref(false);
const helpOpen = ref(false);

const isSettings = computed(() => route.path.startsWith("/settings"));

function isActive(path: string): boolean {
  return route.path === path || route.path.startsWith(`${path}/`);
}

/** 当前视图在胶囊里的位置：滑块靠它平移（设置页时不高亮任何一格）。 */
const navIndex = computed(() => {
  if (isSettings.value) return -1;
  return navItems.findIndex((item) => isActive(item.path));
});

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

/** AI 写歌 = 打开右侧的写歌智能体面板。 */
function openAgent(): void {
  layout.openTab("agent");
}

/** 「文件」菜单：动作、名称、快捷键一次说清。 */
const fileSections = computed<MenuSection[]>(() => [
  {
    items: [
      { id: "new", label: i18n.t("topbar.menu.new"), icon: PlusIcon },
      { id: "open", label: i18n.t("topbar.import"), icon: FolderOpenIcon },
      { id: "history", label: i18n.t("topbar.history"), icon: ClockIcon },
    ],
  },
  {
    title: i18n.t("topbar.menu.importGroup"),
    items: [
      { id: "import", label: i18n.t("topbar.menu.importFiles"), icon: ArrowDownTrayIcon },
    ],
  },
  {
    title: i18n.t("topbar.menu.saveGroup"),
    items: [
      { id: "save", label: i18n.t("topbar.saveLabel"), hint: isMac ? "⌘S" : "Ctrl+S", icon: FloppyDiskIcon },
      { id: "saveAs", label: i18n.t("topbar.menu.saveAs"), hint: isMac ? "⇧⌘S" : "Ctrl+Shift+S", icon: FloppyDiskIcon },
      { id: "export", label: i18n.t("topbar.exportLabel"), icon: ArrowUpTrayIcon },
    ],
  },
]);

const helpSections = computed<MenuSection[]>(() => [
  {
    items: [
      { id: "tour", label: i18n.t("topbar.quickStart"), icon: SquaresPlusIcon },
      { id: "shortcuts", label: i18n.t("shortcuts.title"), icon: KeyboardIcon },
    ],
  },
  {
    items: [{ id: "github", label: i18n.t("topbar.github"), icon: GitHubIcon }],
  },
]);

function onFileAction(id: string): void {
  if (id === "new") emit("new");
  else if (id === "open") emit("import");
  else if (id === "import") emit("importAudio");
  else if (id === "history") emit("history");
  else if (id === "save") emit("save");
  else if (id === "saveAs") emit("saveAs");
  else if (id === "export") emit("export");
}

/** 快速上手：先回到编排视图（引导要指着那块网格讲），再开始。 */
function onHelpAction(id: string): void {
  if (id === "tour") {
    if (route.path !== "/arrange") void router.push("/arrange");
    window.setTimeout(() => onboarding.startTour(), route.path === "/arrange" ? 0 : 260);
    return;
  }
  if (id === "shortcuts") emit("shortcuts");
  else if (id === "github") void openRepo();
}

/** 在系统浏览器里打开 GitHub 仓库（浏览器调试时退回新标签页）。 */
async function openRepo(): Promise<void> {
  await openExternal(REPO_URL);
}

/**
 * 顶栏里这些元素自己处理点击/输入，按下时不能当拖窗口用。
 * 必须包含非 <button> 的可交互元素（如面板标签的 role="tab"）：
 * Windows 下 startDragging 会进入原生拖拽循环并吞掉后续 click，标签就点不动了。
 */
const NO_DRAG_SELECTOR =
  'button, a, input, select, textarea, [contenteditable], [role="tab"], [role="button"], [role="menuitem"], [role="switch"], [role="checkbox"]';

function onMouseDown(e: MouseEvent): void {
  if (!isTauri()) return;
  if (e.buttons !== 1) return;
  const target = e.target as HTMLElement;
  if (target.closest(NO_DRAG_SELECTOR)) return;
  const win = getCurrentWindow();
  if (e.detail === 2) {
    void win.toggleMaximize();
  } else {
    void win.startDragging();
  }
}
</script>

<template>
  <header class="top-bar" :class="{ 'pad-mac': showMacPad, 'is-settings': isSettings }" @mousedown="onMouseDown">
    <!-- 左：工程名 -->
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

      <!-- 中：四个视图，带文字 + 滑块 -->
      <nav
        class="view-nav"
        :class="{ 'is-settings': isSettings }"
        data-tour="views"
        role="tablist"
        :aria-label="i18n.t('topbar.viewGroup')"
      >
        <span
          class="view-thumb"
          :class="{ hidden: navIndex < 0 }"
          :style="{ '--idx': Math.max(navIndex, 0) }"
          aria-hidden="true"
        />
        <button
          v-for="item in navItems"
          :key="item.path"
          class="view-btn"
          role="tab"
          :class="{ on: isActive(item.path) && !isSettings }"
          :aria-selected="isActive(item.path) && !isSettings"
          @click="router.push(item.path)"
        >
          <component :is="item.icon" class="view-icon" />
          <span class="view-label">{{ i18n.t(item.labelKey) }}</span>
        </button>
      </nav>

      <div class="flex-1" />

      <div class="bar-actions">
        <Tooltip :text="i18n.t('topbar.aiWriteHint')" align="end" :delay="400">
          <button class="ai-cta" data-tour="ai" @click="openAgent">
            <SparklesIcon class="ai-icon" />
            <span class="ai-label">{{ i18n.t("topbar.aiWrite") }}</span>
          </button>
        </Tooltip>

        <MenuButton
          v-model:open="fileOpen"
          :sections="fileSections"
          :label="i18n.t('topbar.menu.file')"
          align="end"
          @select="onFileAction"
        >
          <template #trigger="{ open }">
            <Tooltip :text="i18n.t('topbar.menu.file')" align="end">
              <button class="bar-btn" :class="{ on: open }" :aria-label="i18n.t('topbar.menu.file')" :aria-expanded="open">
                <FolderOpenIcon class="bar-icon" />
              </button>
            </Tooltip>
          </template>
        </MenuButton>

        <MenuButton
          v-model:open="helpOpen"
          :sections="helpSections"
          :label="i18n.t('topbar.help')"
          align="end"
          @select="onHelpAction"
        >
          <template #trigger="{ open }">
            <Tooltip :text="i18n.t('topbar.help')" align="end">
              <button class="bar-btn" :class="{ on: open }" :aria-label="i18n.t('topbar.help')" :aria-expanded="open">
                <QuestionMarkCircleIcon class="bar-icon" />
              </button>
            </Tooltip>
          </template>
        </MenuButton>

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
/* 设置页不渲染右侧标签区，顶栏工具区会顶到窗口右端：
   同样要让出 Windows / Linux 自绘窗口按钮的位置，避免重叠。 */
.top-bar.is-settings .bar-main {
  padding-right: calc(12px + var(--win-reserve));
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
  max-width: 180px;
}
/* 未保存标记：紧贴工程名的小圆点 */
.name-dirty {
  width: 7px;
  height: 7px;
  flex: 0 0 auto;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  align-self: center;
  animation: dirty-in var(--md-duration-medium-1) var(--md-ease-spring-fast);
}
@keyframes dirty-in {
  from {
    transform: scale(0);
  }
  to {
    transform: scale(1);
  }
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

/* ---- 视图胶囊：图标 + 文字，当前项有滑块 ---- */
.view-nav {
  position: relative;
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  align-items: center;
  height: 36px;
  margin-left: 8px;
  padding: 3px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  border: 1px solid rgb(var(--md-outline-variant) / 0.35);
  flex: 0 0 auto;
}
.view-thumb {
  position: absolute;
  top: 3px;
  bottom: 3px;
  left: 3px;
  width: calc((100% - 6px) / 4);
  border-radius: 999px;
  background-color: rgb(var(--md-secondary-container));
  box-shadow: var(--md-elevation-1);
  transform: translateX(calc(var(--idx, 0) * 100%));
  transition:
    transform var(--md-duration-medium-2) var(--md-ease-spring-fast),
    opacity var(--md-duration-short-2) var(--md-ease-standard);
}
.view-thumb.hidden {
  opacity: 0;
}
.view-btn {
  position: relative;
  z-index: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-family: inherit;
  font-size: 12px;
  font-weight: 500;
  white-space: nowrap;
  cursor: pointer;
  transition: color var(--md-duration-short-2) var(--md-ease-standard);
}
.view-btn:hover {
  color: rgb(var(--md-on-surface));
}
.view-btn.on {
  color: rgb(var(--md-on-secondary-container));
  font-weight: 600;
}
.view-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.view-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
}
.view-nav.is-settings .view-btn {
  color: rgb(var(--md-on-surface-variant));
}

/* ---- 动作区 ---- */
.bar-actions {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: 0 0 auto;
}
/* AI 写歌：整个顶栏唯一的实心按钮——它是这个应用的主张，值得占位置。 */
.ai-cta {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  height: 34px;
  padding: 0 15px 0 12px;
  margin-right: 4px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-family: inherit;
  font-size: 12.5px;
  font-weight: 600;
  letter-spacing: 0.1px;
  cursor: pointer;
  box-shadow: 0 2px 10px -2px rgb(var(--md-primary) / 0.55);
  transition:
    transform var(--md-duration-short-2) var(--md-ease-standard),
    box-shadow var(--md-duration-medium-1) var(--md-ease-standard),
    filter var(--md-duration-short-2) var(--md-ease-standard);
}
.ai-cta:hover {
  filter: brightness(1.06);
  box-shadow: 0 4px 16px -2px rgb(var(--md-primary) / 0.6);
  transform: translateY(-1px);
}
.ai-cta:active {
  transform: scale(0.97);
}
.ai-cta:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.ai-icon {
  width: 17px;
  height: 17px;
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
  transition:
    background-color var(--md-duration-short-2) var(--md-ease-standard),
    color var(--md-duration-short-2) var(--md-ease-standard),
    transform var(--md-duration-short-2) var(--md-ease-standard);
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

/* 窄窗口：先把文字收起来，保住图标和点击区 */
@media (max-width: 1180px) {
  .view-label {
    display: none;
  }
  .view-btn {
    padding: 0 8px;
  }
  .view-nav {
    grid-template-columns: repeat(4, 40px);
  }
  .view-thumb {
    width: 40px;
  }
}
@media (max-width: 900px) {
  .ai-label {
    display: none;
  }
  .ai-cta {
    width: 34px;
    padding: 0;
    justify-content: center;
  }
}

@media (prefers-reduced-motion: reduce) {
  .bar-btn,
  .view-thumb,
  .ai-cta {
    transition: none;
  }
  .bar-btn:active,
  .ai-cta:hover {
    transform: none;
  }
  .name-dirty {
    animation: none;
  }
}
</style>
