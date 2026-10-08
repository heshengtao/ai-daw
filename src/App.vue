<script setup lang="ts">
/** 应用根：状态初始化、命令动作、试听、快捷键与布局组合。 */
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

const project = useProjectStore();
const layout = useLayoutStore();
const events = useEngineEventsStore();

const aiActive = ref(false);
let pollTimer: number | null = null;

const playback = usePlayback({
  onError: (message, err) => layout.notify(`${message}：${String(err)}`),
});

const loaded = computed(() => project.project !== null);
const isEmpty = computed(() => (project.project?.tracks.length ?? 0) === 0);

function onResizeLeft(delta: number): void {
  layout.trackWidth = Math.min(360, Math.max(160, layout.trackWidth + delta));
}
function onResizeRight(delta: number): void {
  layout.inspectorWidth = Math.min(440, Math.max(200, layout.inspectorWidth + delta));
}

async function onDemo(): Promise<void> {
  playback.stop();
  try {
    await project.loadDemo();
    layout.notify("已载入示例工程");
  } catch (err) {
    layout.notify(`载入失败：${String(err)}`);
  }
}

async function onNew(): Promise<void> {
  playback.stop();
  try {
    await project.clearAll();
    layout.notify("已新建空工程");
  } catch (err) {
    layout.notify(`新建失败：${String(err)}`);
  }
}

function onKey(e: KeyboardEvent): void {
  if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "d") {
    e.preventDefault();
    layout.toggleDevDrawer();
  } else if (e.key === "Escape" && layout.devDrawer) {
    layout.devDrawer = false;
  } else if (e.code === "Space" && !(e.target instanceof HTMLInputElement)) {
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
    @demo="onDemo"
    @new="onNew"
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

    <EmptyState v-else-if="isEmpty" @demo="onDemo" @new="onNew" />

    <template v-else>
      <TrackHeaderList v-if="layout.showTrackHeaders" :style="{ width: `${layout.trackWidth}px`, flex: '0 0 auto' }" />
      <PanelResizer v-if="layout.showTrackHeaders" side="left" @resize="onResizeLeft" />

      <div class="workspace-main">
        <router-view />
      </div>

      <PanelResizer v-if="layout.showInspector" side="right" @resize="onResizeRight" />
      <InspectorPanel v-if="layout.showInspector" :style="{ width: `${layout.inspectorWidth}px`, flex: '0 0 auto' }" />
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
  display: flex;
  align-items: center;
  justify-content: center;
}
.center-card {
  width: 380px;
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
