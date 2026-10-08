<script setup lang="ts">
/** 应用根：状态初始化、工程读写、试听、快捷键与布局组合。 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { ExclamationTriangleIcon, ArrowPathIcon } from "@heroicons/vue/24/outline";
import AppShell from "@/components/layout/AppShell.vue";
import TrackHeaderList from "@/components/workspace/TrackHeaderList.vue";
import InspectorPanel from "@/components/workspace/InspectorPanel.vue";
import PanelResizer from "@/components/workspace/PanelResizer.vue";
import EmptyState from "@/components/common/EmptyState.vue";
import ToastHost from "@/components/common/ToastHost.vue";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useEngineEventsStore } from "@/stores/engineEvents";
import { usePlayback } from "@/composables/usePlayback";
import { exportWav } from "@/api/tauri";
import { pickProjectToOpen, pickProjectToSave, pickWavToSave } from "@/api/dialogs";

const project = useProjectStore();
const layout = useLayoutStore();
const events = useEngineEventsStore();

const aiActive = ref(false);
const exporting = ref(false);
let pollTimer: number | null = null;

const playback = usePlayback({
  onError: (message, err) => layout.notify(`${message}：${String(err)}`),
});

const loaded = computed(() => project.project !== null);
/** 首屏引导只在「还没开始」且工程为空时出现。 */
const showIntro = computed(() => !project.started && (project.project?.tracks.length ?? 0) === 0);

/** 用于文件名的工程名。 */
const fileBase = computed(() => {
  const name = project.project?.name;
  return !name || name === "untitled" ? "untitled" : name;
});

function onResizeLeft(delta: number): void {
  layout.trackWidth = Math.min(360, Math.max(160, layout.trackWidth + delta));
}
function onResizeRight(delta: number): void {
  layout.inspectorWidth = Math.min(440, Math.max(200, layout.inspectorWidth + delta));
}

async function onImport(): Promise<void> {
  playback.stop();
  try {
    const path = await pickProjectToOpen();
    if (!path) return;
    await project.openFrom(path);
    layout.notify("已导入工程");
  } catch (err) {
    layout.notify(`导入失败：${String(err)}`);
  }
}

async function onNew(): Promise<void> {
  playback.stop();
  try {
    await project.newProject();
    layout.notify("已新建空工程");
  } catch (err) {
    layout.notify(`新建失败：${String(err)}`);
  }
}

async function onSave(): Promise<void> {
  try {
    const path = await pickProjectToSave(`${fileBase.value}.json`);
    if (!path) return;
    const written = await project.saveTo(path);
    layout.notify(`已保存：${written}`);
  } catch (err) {
    layout.notify(`保存失败：${String(err)}`);
  }
}

async function onExport(): Promise<void> {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const path = await pickWavToSave(`${fileBase.value}.wav`);
    if (!path) return;
    const written = await exportWav(path, 44100);
    layout.notify(`已导出：${written}`);
  } catch (err) {
    layout.notify(`导出失败：${String(err)}`);
  } finally {
    exporting.value = false;
  }
}

async function onUndo(): Promise<void> {
  try {
    if (!(await project.undo())) layout.notify("没有可撤销的操作");
  } catch (err) {
    layout.notify(`撤销失败：${String(err)}`);
  }
}

async function onRedo(): Promise<void> {
  try {
    if (!(await project.redo())) layout.notify("没有可重做的操作");
  } catch (err) {
    layout.notify(`重做失败：${String(err)}`);
  }
}

function onKey(e: KeyboardEvent): void {
  if (e.target instanceof HTMLInputElement) return;
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
  if (e.key === "Escape" && layout.devDrawer) {
    layout.devDrawer = false;
    return;
  }
  if (e.code === "Space") {
    e.preventDefault();
    if (playback.playing.value) playback.stop();
    else void playback.play();
  }
}

onMounted(async () => {
  window.addEventListener("keydown", onKey);
  await events.start();
  await project.reload();
  pollTimer = window.setInterval(() => {
    aiActive.value = events.recentlyActive();
  }, 300);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  if (pollTimer !== null) window.clearInterval(pollTimer);
  events.stop();
});
</script>

<template>
  <AppShell
    :playing="playback.playing.value"
    :ai-active="aiActive"
    @play="playback.play()"
    @stop="playback.stop()"
    @import="onImport"
    @new="onNew"
    @save="onSave"
    @export="onExport"
  >
    <!-- 启动失败：必须让用户看见，并能重试（而不是停在"载入中"）。 -->
    <div v-if="project.loadError" class="center-state">
      <div class="card-elevated center-card" role="alert">
        <ExclamationTriangleIcon class="w-7 h-7" :style="{ color: 'rgb(var(--md-error))' }" />
        <h2 class="text-title-md">无法载入工程</h2>
        <p class="text-body-sm center-reason">{{ project.loadError }}</p>
        <button class="btn-filled" @click="project.reload()">
          <ArrowPathIcon class="w-5 h-5" />
          重试
        </button>
      </div>
    </div>

    <div v-else-if="!loaded" class="center-state">
      <p class="text-body-md center-muted">正在载入工程…</p>
    </div>

    <EmptyState v-else-if="showIntro" @import="onImport" @new="onNew" />

    <template v-else>
      <TrackHeaderList
        v-if="layout.showTrackHeaders"
        :style="{ width: `${layout.trackWidth}px`, flex: '0 0 auto' }"
      />
      <PanelResizer v-if="layout.showTrackHeaders" side="left" @resize="onResizeLeft" />

      <div class="workspace-main">
        <router-view />
      </div>

      <PanelResizer v-if="layout.showInspector" side="right" @resize="onResizeRight" />
      <InspectorPanel
        v-if="layout.showInspector"
        :style="{ width: `${layout.inspectorWidth}px`, flex: '0 0 auto' }"
      />
    </template>

    <ToastHost />
  </AppShell>
</template>

<style scoped>
.workspace-main {
  flex: 1;
  min-width: 0;
  display: flex;
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
