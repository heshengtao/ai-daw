<script setup lang="ts">
/**
 * 应用外壳：左侧音轨栏（全高，与顶栏融合）+ 右侧区域（顶栏 + [工作区 | 右侧面板]）+ 底部传输条。
 * 顶栏横跨"工作区 + 右侧面板"两列，因此它的最右端**永远等于窗口最右端**——
 * 面板开关按钮固定放在那里，展开/收起时鼠标不用移动。
 */
import TopBar from "./TopBar.vue";
import TopProgressBar from "./TopProgressBar.vue";
import BottomBar from "./BottomBar.vue";
import WindowControls from "./WindowControls.vue";
import DevDrawer from "./DevDrawer.vue";
import ToolDock from "./ToolDock.vue";
import { isLinux, isTauri, isWindows } from "@/utils/platform";

defineProps<{
  aiActive: boolean;
  /** 顶边进度条的状态：由 `App.vue` 决定现在该显示哪个长任务（导入 / 导出）。 */
  jobBar: { active: boolean; pct: number | null; label: string };
}>();
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

/** 只有 Windows / Linux 需要自绘窗口按钮。 */
const showWindowControls = isTauri() && (isWindows() || isLinux());
</script>

<template>
  <div class="app-root" :class="{ win: showWindowControls }">
    <!-- 窗口按钮：固定在窗口右上角，独立于各栏布局 -->
    <WindowControls v-if="showWindowControls" class="win-layer" />

    <div class="app-body">
      <!-- 左侧音轨栏：从窗口最顶端开始，与顶栏融为一体 -->
      <slot name="left" />

      <!-- 顶栏横跨"工作区 + 右侧面板"，保证其右端始终是窗口右端 -->
      <div class="app-stack">
        <TopBar
          @import="emit('import')"
          @import-audio="emit('importAudio')"
          @new="emit('new')"
          @history="emit('history')"
          @save="emit('save')"
          @export="emit('export')"
          @rename="emit('rename')"
          @shortcuts="emit('shortcuts')"
        />

        <!-- 长任务进度条（导入 / 导出共用）：绝对定位贴住顶栏下边缘，不占布局 -->
        <TopProgressBar v-bind="jobBar" />

        <div class="app-row">
          <main class="app-main">
            <slot />
            <DevDrawer />
            <!-- 浮动工具岛：中栏右下角的鼠标模式 + 底部窗格开关 -->
            <ToolDock />
          </main>

          <slot name="right" />
        </div>
      </div>
    </div>

    <BottomBar :ai-active="aiActive" />
  </div>
</template>

<style scoped>
.app-root {
  position: relative;
  display: flex;
  flex-direction: column;
  height: 100vh;
  overflow: hidden;
  background-color: var(--app-bg);
  color: rgb(var(--md-on-surface));
  /* 窗口按钮占位宽度：由 .win 打开 */
  --win-reserve: 0px;
}
.app-root.win {
  --win-reserve: 138px;
}
/* 窗口按钮层：贴窗口右上角，压在所有面板之上 */
.win-layer {
  position: absolute;
  top: 0;
  right: 0;
  height: 52px;
  z-index: 60;
}
.app-body {
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
}
.app-stack {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.app-row {
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
}
.app-main {
  position: relative;
  flex: 1;
  min-height: 0;
  min-width: 0;
  display: flex;
  overflow: hidden;
  background-color: var(--app-card);
}
</style>
