<script setup lang="ts">
/** 应用外壳：标题栏 + 顶栏（导航/动作/导出）+ 传输条 + 主区插槽 + 状态栏 + 开发者抽屉。 */
import { computed, ref } from "vue";
import { documentDir, join } from "@tauri-apps/api/path";
import {
  Squares2X2Icon,
  PencilSquareIcon,
  AdjustmentsHorizontalIcon,
  CircleStackIcon,
  ComputerDesktopIcon,
  SunIcon,
  MoonIcon,
  ViewColumnsIcon,
  CodeBracketIcon,
  ArrowDownTrayIcon,
  ArrowUpTrayIcon,
  PlusIcon,
} from "@heroicons/vue/24/outline";
import TitleBar from "./TitleBar.vue";
import TransportBar from "./TransportBar.vue";
import StatusBar from "./StatusBar.vue";
import DevDrawer from "./DevDrawer.vue";
import SegmentedNav from "@/components/common/SegmentedNav.vue";
import type { NavItem } from "@/components/common/nav";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useTheme } from "@/composables/useTheme";
import { exportWav } from "@/api/tauri";

defineProps<{ playing: boolean; aiActive: boolean }>();
const emit = defineEmits<{
  (e: "play"): void;
  (e: "stop"): void;
  (e: "demo"): void;
  (e: "new"): void;
}>();

const layout = useLayoutStore();
const projectStore = useProjectStore();
const { theme, cycleTheme } = useTheme();
const exporting = ref(false);

const navItems: NavItem[] = [
  { label: "编排", path: "/arrange", icon: Squares2X2Icon },
  { label: "编辑", path: "/edit", icon: PencilSquareIcon },
  { label: "混音", path: "/mix", icon: AdjustmentsHorizontalIcon },
  { label: "浏览", path: "/browse", icon: CircleStackIcon },
];

const projectName = computed(() => projectStore.project?.name ?? "untitled");

/** 导出到「文稿」目录下的 daw-party-export.wav（文件对话框在后续版本接入）。 */
async function onExport(): Promise<void> {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const dir = await documentDir();
    const path = await join(dir, "daw-party-export.wav");
    const written = await exportWav(path, 44100);
    layout.notify(`已导出：${written}`);
  } catch (err) {
    layout.notify(`导出失败：${String(err)}`);
  } finally {
    exporting.value = false;
  }
}
</script>

<template>
  <div class="app-root">
    <TitleBar />

    <header class="app-bar">
      <div class="bar-left">
        <span class="app-title text-title-md">{{ projectName }}</span>
      </div>

      <SegmentedNav :items="navItems" class="bar-nav" />

      <div class="flex-1" />

      <div class="bar-actions">
        <button class="btn-tonal" title="载入示例" @click="emit('demo')">
          <ArrowDownTrayIcon class="w-4 h-4" />
          示例
        </button>
        <button class="btn-text" title="清空为新工程" @click="emit('new')">
          <PlusIcon class="w-4 h-4" />
          新建
        </button>
        <button class="btn-text" :disabled="exporting" title="导出 WAV" @click="onExport">
          <ArrowUpTrayIcon class="w-4 h-4" />
          {{ exporting ? "导出中…" : "导出" }}
        </button>
        <span class="divider" />
        <button
          class="btn-icon"
          :title="layout.mode === 'pro' ? '切换到简单模式' : '展开专业面板'"
          :style="layout.mode === 'pro' ? { color: 'rgb(var(--md-primary))' } : undefined"
          @click="layout.togglePro()"
        >
          <ViewColumnsIcon class="w-5 h-5" />
        </button>
        <button class="btn-icon" title="切换主题" @click="cycleTheme">
          <SunIcon v-if="theme === 'light'" class="w-5 h-5" />
          <MoonIcon v-else-if="theme === 'dark'" class="w-5 h-5" />
          <ComputerDesktopIcon v-else class="w-5 h-5" />
        </button>
        <button
          class="btn-icon"
          title="开发者抽屉 (Ctrl/Cmd+Shift+D)"
          :style="layout.devDrawer ? { color: 'rgb(var(--md-primary))' } : undefined"
          @click="layout.toggleDevDrawer()"
        >
          <CodeBracketIcon class="w-5 h-5" />
        </button>
      </div>
    </header>

    <TransportBar :playing="playing" :ai-active="aiActive" @play="emit('play')" @stop="emit('stop')" />

    <main class="app-main">
      <slot />
      <DevDrawer />
    </main>

    <StatusBar />
  </div>
</template>

<style scoped>
.app-root {
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  background-color: rgb(var(--md-surface));
  color: rgb(var(--md-on-surface));
}
.app-bar {
  height: 56px;
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 0 14px;
  background-color: rgb(var(--md-surface-container));
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.3);
  flex: 0 0 auto;
}
.bar-left {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}
.app-title {
  font-weight: 700;
  letter-spacing: -0.2px;
}
.bar-nav {
  margin-left: 6px;
}
.bar-actions {
  display: flex;
  align-items: center;
  gap: 4px;
}
.divider {
  width: 1px;
  height: 20px;
  margin: 0 6px;
  background-color: rgb(var(--md-outline-variant) / 0.6);
}
.app-main {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}
</style>
