<script setup lang="ts">
/** 应用根：状态初始化、工程读写、试听、快捷键与布局组合。 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRoute } from "vue-router";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ExclamationTriangleIcon, ArrowPathIcon } from "@heroicons/vue/24/outline";
import AppShell from "@/components/layout/AppShell.vue";
import TrackHeaderList from "@/components/workspace/TrackHeaderList.vue";
import RightPanel from "@/components/workspace/RightPanel.vue";
import PanelResizer from "@/components/workspace/PanelResizer.vue";
import ToastHost from "@/components/common/ToastHost.vue";
import ProjectDialog from "@/components/common/ProjectDialog.vue";
import ShortcutsDialog from "@/components/common/ShortcutsDialog.vue";
import UnsavedChangesDialog from "@/components/common/UnsavedChangesDialog.vue";
import ParameterBrowser from "@/components/common/ParameterBrowser.vue";
import { useProjectSync } from "@/composables/useProjectSync";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useEngineEventsStore } from "@/stores/engineEvents";
import { useTransportStore } from "@/stores/transport";
import { usePluginsStore } from "@/stores/plugins";
import { usePresetsStore } from "@/stores/presets";
import { useSelectionStore } from "@/stores/selection";
import { useParamBrowserStore } from "@/stores/paramBrowser";
import { useImportingStore } from "@/stores/importing";
import { useTimelineStore } from "@/stores/timeline";
import { useI18n } from "@/i18n";
import { exportWav, onPluginsEvent } from "@/api/tauri";
import { pickAudioToOpen, pickProjectToOpen, pickProjectToSave, pickWavToSave } from "@/api/dialogs";
import { isTauri } from "@/utils/platform";
import { shouldDeferToBrowser } from "@/utils/editable";

const project = useProjectStore();
const layout = useLayoutStore();
const events = useEngineEventsStore();
const transport = useTransportStore();
const plugins = usePluginsStore();
const presets = usePresetsStore();
const selection = useSelectionStore();
const paramBrowser = useParamBrowserStore();
const importing = useImportingStore();
const timeline = useTimelineStore();
const i18n = useI18n();
const route = useRoute();

// 每个窗口（主窗口 + 合成器/效果器等二级窗口）都订阅引擎事件、节流重拉工程。
// 二级窗口改工程（例如合成器窗口切预设换掉整条效果链）时，主窗口也能立刻跟上——
// 否则主窗口会拿着一条过时的效果链，按过时序号打开效果界面（见 open_effect_window）。
useProjectSync();

/** 设置页独占中栏：不显示音轨栏与右侧面板。 */
const isSettings = computed(() => route.path.startsWith("/settings"));

/** 独立窗口的路由前缀。 */
const BARE_PREFIXES = ["/synth", "/effect", "/blank"];

/** `#/effect?track=2` → `/effect`（同步读，不经过 router）。 */
function hashPath(): string {
  return window.location.hash.replace(/^#/, "").split("?")[0] ?? "";
}

/**
 * 独立窗口（合成器面板 / 效果器面板 / 插件界面宿主页）：只渲染自己的内容，不套工作区外壳。
 *
 * **同时看 `location.hash`**：`route` 要等 router 异步解析完才有值，在那之前它是 `/`，
 * 于是独立窗口会先渲染**一帧主界面**再跳成自己的内容（打开窗口时的闪屏）。
 * hash 是同步可读的，第一帧就能判断对。
 */
const isBareWindow = computed(() =>
  [hashPath(), route.path].some((p) => BARE_PREFIXES.some((prefix) => p.startsWith(prefix))),
);

const aiActive = ref(false);
/** 最近被 AI 代理改动过的轨道（用于音轨栏高亮）。 */
const aiTracks = ref<number[]>([]);
const exporting = ref(false);
let pollTimer: number | null = null;
let unlistenPlugins: (() => void) | null = null;
let unlistenImport: (() => void) | null = null;
let unlistenClose: (() => void) | null = null;
let autosaveTimer: number | null = null;

const loaded = computed(() => project.project !== null);

/** 用于文件名的工程名。 */
const fileBase = computed(() => {
  const name = project.project?.name;
  return !name || name === "untitled" ? "untitled" : name;
});

/** 主窗口才管「保存/退出提示/自动缓存」；插件独立窗口不参与。 */
const appWindow = isTauri() ? getCurrentWindow() : null;
const isMainWindow = appWindow?.label === "main";

/**
 * 「主」窗口：真正的应用窗口（浏览器预览里没有多窗口这回事，也按主窗口对待）。
 *
 * 二级窗口（合成器 / 效果器面板、插件界面宿主页）**不**订阅传输、也**不**做工程初始化：
 * 它们根本不显示走带，订阅了却会在 `mode` 从 preview 切回 realtime 时把**自己那份**
 * 循环区间 / 循环开关补推给引擎。多一扇窗就多一份全局状态写入——它的快照一旦比主窗口旧，
 * 用户刚划好的循环段就被冲掉了。工程写入（`localizeStarterTracks`）同理只该由主窗口做。
 */
const isPrimaryWindow = !isTauri() || isMainWindow;

/** 工程名对话框（重命名 / 保存共用）。 */
const projectDialogOpen = ref(false);
const projectDialogMode = ref<"rename" | "save">("rename");

/** 快捷键速查表模态框。 */
const shortcutsOpen = ref(false);

/** 退出确认模态框：关闭守卫等待用户三选一（保存 / 不保存 / 取消）。 */
const unsavedOpen = ref(false);
let unsavedResolve: ((choice: "save" | "discard" | "cancel") => void) | null = null;

/** 打开退出确认并返回用户的选择。 */
function askUnsavedChanges(): Promise<"save" | "discard" | "cancel"> {
  return new Promise((resolve) => {
    unsavedResolve = resolve;
    unsavedOpen.value = true;
  });
}

function onUnsavedChoice(choice: "save" | "discard" | "cancel"): void {
  unsavedOpen.value = false;
  unsavedResolve?.(choice);
  unsavedResolve = null;
}

/** 对话框的初始名：未命名工程留空，让它显示占位符。 */
const projectBaseName = computed(() => {
  const name = project.project?.name;
  return !name || name === "untitled" ? "" : name;
});

function onResizeLeft(delta: number): void {
  // 左栏上限为窗口的 1/4（由 layout store 统一夹紧）。
  layout.resizeTrack(delta);
}
function onResizeRight(delta: number): void {
  // 右栏上限为「扣掉左栏后剩余区域的 2/3」。
  layout.resizeInspector(delta);
}

/** 窗口尺寸变化后重新夹紧左右面板宽度。 */
function onWindowResize(): void {
  layout.clampPanes();
}

async function onImport(): Promise<void> {
  await transport.stop();
  try {
    const path = await pickProjectToOpen();
    if (!path) return;
    await project.openFrom(path);
    void project.cacheSession();
    layout.notify(i18n.t("app.imported"));
  } catch (err) {
    layout.notify(i18n.t("error.import", { error: String(err) }));
  }
}

/**
 * 导入音频片段：落在**选中轨道**（没有选中就第一条）的播放头处，并对齐到网格。
 *
 * 片段的拍长由后端按素材自然时长换算（导入时不变调）；之后拖宽窄或改速度才会变调。
 */
async function onImportAudio(): Promise<void> {
  try {
    const path = await pickAudioToOpen();
    if (!path) return;
    const tracks = project.project?.tracks ?? [];
    const track = selection.trackId ?? tracks[0]?.id;
    if (track === undefined) {
      layout.notify(i18n.t("error.importAudioNoTrack"));
      return;
    }
    await project.importAudio(path, track, timeline.snapBeats(transport.positionBeats));
  } catch (err) {
    layout.notify(i18n.t("error.importAudio", { error: String(err) }));
  }
}

async function onNew(): Promise<void> {
  await transport.stop();
  try {
    await project.newProject();
    void project.cacheSession();
    layout.notify(i18n.t("app.created"));
  } catch (err) {
    layout.notify(i18n.t("error.newProject", { error: String(err) }));
  }
}

/** 把工程名转成安全的文件名（去掉路径分隔符等非法字符）。 */
function safeFilename(name: string): string {
  const cleaned = name.replace(/[\\/:*?"<>|]/g, "-").trim();
  return cleaned || "untitled";
}

/** 打开工程名对话框：重命名。 */
function openRenameDialog(): void {
  projectDialogMode.value = "rename";
  projectDialogOpen.value = true;
}

/** 打开工程名对话框：保存（可选改名 + 选路径）。 */
function openSaveDialog(): void {
  projectDialogMode.value = "save";
  projectDialogOpen.value = true;
}

/** 把工程写到指定路径；成功返回 true 并清除未保存标记。 */
async function writeProject(path: string): Promise<boolean> {
  try {
    const written = await project.saveTo(path);
    layout.notify(i18n.t("app.saved", { path: written }));
    void project.cacheSession();
    return true;
  } catch (err) {
    layout.notify(i18n.t("error.save", { error: String(err) }));
    return false;
  }
}

/**
 * 保存：已经保存过的工程直接写回原路径；从未保存过的弹出「工程名 + 选择位置」。
 *
 * 返回是否真的写盘成功（退出提示用它决定能不能闭合窗口）。
 */
async function onSave(): Promise<boolean> {
  if (project.currentPath) return writeProject(project.currentPath);
  openSaveDialog();
  return false;
}

/** 工程名对话框提交：重命名模式下直接改名；保存模式下改名后继续选路径并写盘。 */
async function onProjectDialogSubmit(name: string): Promise<void> {
  projectDialogOpen.value = false;
  if (projectDialogMode.value === "rename") {
    try {
      await project.renameProject(name);
    } catch (err) {
      layout.notify(i18n.t("error.rename", { error: String(err) }));
    }
    return;
  }
  // 保存模式：名字变了先改名（这样写进文件的就是新名字）。
  try {
    if (name !== (project.project?.name ?? "")) await project.renameProject(name);
  } catch (err) {
    layout.notify(i18n.t("error.rename", { error: String(err) }));
    return;
  }
  const path = await pickProjectToSave(`${safeFilename(name)}.json`);
  if (!path) return;
  await writeProject(path);
}

async function onExport(): Promise<void> {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const path = await pickWavToSave(`${fileBase.value}.wav`);
    if (!path) return;
    const written = await exportWav(path, 44100);
    layout.notify(i18n.t("app.exported", { path: written }));
  } catch (err) {
    layout.notify(i18n.t("error.export", { error: String(err) }));
  } finally {
    exporting.value = false;
  }
}

async function onUndo(): Promise<void> {
  try {
    if (!(await project.undo())) layout.notify(i18n.t("app.nothingToUndo"));
  } catch (err) {
    layout.notify(i18n.t("error.undo", { error: String(err) }));
  }
}

async function onRedo(): Promise<void> {
  try {
    if (!(await project.redo())) layout.notify(i18n.t("app.nothingToRedo"));
  } catch (err) {
    layout.notify(i18n.t("error.redo", { error: String(err) }));
  }
}

function onKey(e: KeyboardEvent): void {
  // 焦点在输入框/文本域里，或用户选了一段文字：这些按键归浏览器（否则文本域里打不出 `1`/`2`/`3`，
  // 在别处选中文字按 Cmd+C 也会被当成 DAW 快捷键）。见 `utils/editable.ts`。
  if (shouldDeferToBrowser(e)) return;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.shiftKey && e.key.toLowerCase() === "d") {
    e.preventDefault();
    layout.toggleDevDrawer();
    return;
  }
  if (mod && e.key.toLowerCase() === "z") {
    e.preventDefault();
    void (e.shiftKey ? onRedo() : onUndo());
    return;
  }
  // 切换音轨栏（与顶栏/音轨栏里的按钮同一个动作）。
  if (mod && !e.shiftKey && e.key.toLowerCase() === "b") {
    e.preventDefault();
    layout.showTrackHeaders = !layout.showTrackHeaders;
    return;
  }
  // 保存：Cmd/Ctrl+S 直接写回原路径（没保存过则弹名/路径）；Cmd/Ctrl+Shift+S 另存为。
  if (mod && e.shiftKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    openSaveDialog();
    return;
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    void onSave();
    return;
  }
  if (e.key === "Escape" && layout.devDrawer) {
    layout.devDrawer = false;
    return;
  }
  // 鼠标模式：与顶栏的三段按钮同一顺序 —— 1 = 绘制，2 = 框选，3 = 分隔。
  if (!mod && e.key === "1") {
    e.preventDefault();
    layout.setTool("draw");
    return;
  }
  if (!mod && e.key === "2") {
    e.preventDefault();
    layout.setTool("select");
    return;
  }
  if (!mod && e.key === "3") {
    e.preventDefault();
    layout.setTool("split");
    return;
  }
  // 快捷键速查表：`?` 或 Ctrl/Cmd+/。
  if (!mod && e.key === "?") {
    e.preventDefault();
    shortcutsOpen.value = true;
    return;
  }
  if (mod && (e.key === "/" || e.key === "?")) {
    e.preventDefault();
    shortcutsOpen.value = !shortcutsOpen.value;
    return;
  }
  // 传输快捷键：空格播放/暂停，Home 回到起点。
  if (e.code === "Space") {
    e.preventDefault();
    void transport.toggle();
    return;
  }
  if (e.key === "Home") {
    e.preventDefault();
    void transport.rewind();
  }
}

/**
 * 主窗口关闭守卫：有未保存改动时先问一句。
 *
 * 应用内 MD3 模态框三选一：保存 → 写盘后关闭；不保存 → 直接关闭（此刻工作状态已进会话缓存，
 * 下次打开照样还原）；取消 → 什么都不做。从未保存过时「保存」会先弹出工程名/路径对话框，
 * 让用户先落盘，而不是把窗口关掉。
 */
async function setupCloseGuard(): Promise<(() => void) | null> {
  const win = appWindow;
  if (!win || !isMainWindow) return null;
  return win.onCloseRequested(async (event) => {
    if (!project.dirty) return;
    event.preventDefault();
    // 先把当前状态落进缓存，「不保存」也不会真的丢东西。
    try {
      await project.cacheSession();
    } catch {
      // 缓存失败不阻断退出。
    }
    const choice = await askUnsavedChanges();
    if (choice === "cancel") return;
    if (choice === "save") {
      // 还没保存过：先让用户起名并选路径；保存完再关。
      if (!project.currentPath) {
        openSaveDialog();
        return;
      }
      if (!(await writeProject(project.currentPath))) return;
    }
    await win.destroy();
  });
}

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("resize", onWindowResize);
  layout.clampPanes();
  // 每一步都独立：合成器那个二级窗口里，某一步失败不该把它拖成「永远空白」。
  // 传输状态只有主窗口订阅：二级窗口不显示走带，订阅了反而会把本窗口的循环区间写回引擎。
  await Promise.allSettled(
    isPrimaryWindow
      ? [events.start(), transport.init(), project.reload()]
      : [events.start(), project.reload()],
  );
  // 初始工程是在界面起来之前建的（那时还不知道界面语言），这里按当前语言把默认分轨名改过来。
  // 后端只在工程仍是默认模板时才改——用户动过就一个字都不动；改名不进撤销栈。
  // 只由主窗口做：二级窗口同时来一遍是多余的工程写入。
  if (isPrimaryWindow) await project.localizeStarterTracks().catch(() => {});
  // 用户预设：跨工程，取一次就够（保存/删除后会各自刷新）。
  void presets.refresh();
  // 插件目录：先取一次当前值，再等后端的后台扫描完成推送。
  await plugins.refresh().catch(() => {});
  unlistenPlugins = await onPluginsEvent(() => void plugins.refresh());
  // 音频导入解码进度：顶栏下边缘的进度条据此更新。
  unlistenImport = importing.watchProgress();
  pollTimer = window.setInterval(() => {
    // 只统计**代理**发起的事件：人类自己的编辑不该点亮「AI 正在写」。
    aiActive.value = events.agentActive();
    aiTracks.value = events.agentTracks();
  }, 300);
  // 主窗口独有：还原上次的工程路径、装上退出守卫、定期把工作状态写进会话缓存。
  if (isMainWindow) {
    await project.loadPath();
    unlistenClose = await setupCloseGuard();
    autosaveTimer = window.setInterval(() => {
      if (project.dirty) void project.cacheSession();
    }, 10_000);
  }
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("resize", onWindowResize);
  if (pollTimer !== null) window.clearInterval(pollTimer);
  if (autosaveTimer !== null) window.clearInterval(autosaveTimer);
  unlistenClose?.();
  unlistenPlugins?.();
  unlistenImport?.();
  events.stop();
  transport.dispose();
});
</script>

<template>
  <!-- 独立窗口：窗口里只有它自己的内容 -->
  <router-view v-if="isBareWindow" />

  <AppShell
    v-else
    :ai-active="aiActive"
    @import="onImport"
    @import-audio="onImportAudio"
    @new="onNew"
    @save="onSave"
    @export="onExport"
    @rename="openRenameDialog"
    @shortcuts="shortcutsOpen = true"
  >
    <!-- 左：音轨栏（全高，与顶栏融合） -->
    <template #left>
      <template v-if="!isSettings">
        <TrackHeaderList
          v-if="layout.showTrackHeaders"
          :agent-tracks="aiTracks"
          :style="{ width: `${layout.trackWidth}px`, flex: '0 0 auto' }"
        />
        <PanelResizer v-if="layout.showTrackHeaders" side="left" @resize="onResizeLeft" />
      </template>
    </template>

    <!-- 中：工作区 -->
    <div v-if="isSettings" class="workspace-main">
      <router-view />
    </div>

    <!-- 启动失败：必须让用户看见，并能重试（而不是停在"载入中"）。 -->
    <div v-else-if="project.loadError" class="center-state">
      <div class="card-elevated center-card" role="alert">
        <ExclamationTriangleIcon class="w-7 h-7" :style="{ color: 'rgb(var(--md-error))' }" />
        <h2 class="text-title-md">{{ i18n.t("app.loadFailed") }}</h2>
        <p class="text-body-sm center-reason">{{ project.loadError }}</p>
        <button class="btn-filled" @click="project.reload()">
          <ArrowPathIcon class="w-5 h-5" />
          {{ i18n.t("app.retry") }}
        </button>
      </div>
    </div>

    <div v-else-if="!loaded" class="center-state">
      <p class="text-body-md center-muted">{{ i18n.t("app.loading") }}</p>
    </div>

    <div v-else class="workspace-main">
      <router-view />
    </div>

    <!-- 右：标签页面板（全高） -->
    <template #right>
      <template v-if="!isSettings">
        <PanelResizer v-if="layout.showInspector" side="right" @resize="onResizeRight" />
        <RightPanel
          v-if="layout.showInspector"
          :style="{ width: `${layout.inspectorWidth}px`, flex: '0 0 auto' }"
        />
      </template>
    </template>

    <ToastHost />
  </AppShell>

  <!-- 工程名对话框（重命名 / 保存共用）：Teleport 到 body，独立于窗口布局 -->
  <ProjectDialog
    v-model:open="projectDialogOpen"
    :mode="projectDialogMode"
    :initial-name="projectBaseName"
    @submit="onProjectDialogSubmit"
  />

  <!-- 快捷键速查表：⌘ 按钮 / `?` / Ctrl+Cmd+/ 打开 -->
  <ShortcutsDialog v-model:open="shortcutsOpen" />

  <!-- 关闭工程前的未保存改动确认：窗口关闭请求会打开它并等待选择 -->
  <UnsavedChangesDialog v-model:open="unsavedOpen" @choose="onUnsavedChoice" />

  <!-- 参数页（挑参数给自动化用）：挂在根部，车道下拉与检查器两个入口共用 -->
  <ParameterBrowser v-if="paramBrowser.isOpen" />
</template>

<style scoped>
.workspace-main {
  flex: 1;
  min-width: 0;
  display: flex;
  overflow: hidden;
}
.center-state {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: center;
}
.center-card {
  width: 380px;
  max-width: 100%;
  padding: 26px 24px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  text-align: center;
}
.center-reason {
  margin: 0;
  color: rgb(var(--md-on-surface-variant));
  word-break: break-word;
}
.center-muted {
  color: rgb(var(--md-on-surface-variant));
}
</style>
