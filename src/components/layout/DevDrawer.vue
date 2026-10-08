<script setup lang="ts">
/**
 * 开发者抽屉：原始引擎事件流。
 * 默认关闭，`Cmd/Ctrl+Shift+D` 呼出，Esc 关闭。不属于面向用户的界面。
 */
import { XMarkIcon, TrashIcon } from "@heroicons/vue/24/outline";
import Tooltip from "@/components/common/Tooltip.vue";
import { useEngineEventsStore } from "@/stores/engineEvents";
import { useLayoutStore } from "@/stores/layout";

const layout = useLayoutStore();
const events = useEngineEventsStore();
</script>

<template>
  <transition name="drawer">
    <aside v-if="layout.devDrawer" class="dev-drawer" role="complementary" aria-label="Developer drawer">
      <header class="dev-head">
        <div class="flex items-center gap-2">
          <span class="text-title-sm">Developer</span>
          <span class="tag">engine events</span>
        </div>
        <div class="flex items-center gap-1">
          <Tooltip text="清空日志" align="end">
            <button class="btn-icon" aria-label="清空日志" @click="events.clear()">
              <TrashIcon class="w-4 h-4" />
            </button>
          </Tooltip>
          <Tooltip text="关闭 (Esc)" align="end">
            <button class="btn-icon" aria-label="关闭" @click="layout.devDrawer = false">
              <XMarkIcon class="w-4 h-4" />
            </button>
          </Tooltip>
        </div>
      </header>
      <p class="dev-note">引擎事件原始流（AI/引擎调试用，非面向用户）。</p>
      <div class="dev-log">
        <p v-if="events.lines.length === 0" class="dev-empty">暂无事件。执行命令后会在此出现。</p>
        <code v-for="(line, i) in [...events.lines].reverse()" :key="i" class="dev-line">{{ line }}</code>
      </div>
    </aside>
  </transition>
</template>

<style scoped>
.dev-drawer {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  width: 380px;
  display: flex;
  flex-direction: column;
  background-color: rgb(var(--md-surface-container));
  border-left: 1px solid rgb(var(--md-outline-variant) / 0.4);
  box-shadow: var(--md-elevation-3);
  z-index: 50;
}
.dev-head {
  height: 44px;
  padding: 0 8px 0 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.tag {
  font-size: 10px;
  letter-spacing: 0.6px;
  padding: 2px 7px;
  border-radius: 999px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.dev-note {
  margin: 8px 14px;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.dev-log {
  flex: 1;
  overflow-y: auto;
  padding: 0 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.dev-line {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 11px;
  line-height: 1.45;
  word-break: break-all;
  padding: 6px 8px;
  border-radius: 8px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
}
.dev-empty {
  font-size: 12px;
  color: rgb(var(--md-on-surface-muted));
}
.drawer-enter-active,
.drawer-leave-active {
  transition: transform 0.28s cubic-bezier(0.2, 0, 0, 1), opacity 0.2s ease;
}
.drawer-enter-from,
.drawer-leave-to {
  transform: translateX(100%);
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .drawer-enter-active,
  .drawer-leave-active {
    transition: none;
  }
}
</style>
