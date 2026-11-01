<script setup lang="ts">
/**
 * 右键菜单：固定定位、贴边翻转、点外面 / 滚轮 / Esc 关闭。
 *
 * 只负责「摆在哪、画什么、点了哪一项」，不关心业务；编排与钢琴卷帘共用同一个组件。
 * 用 `Teleport` 挂到 `body`：时间轴的滚动容器有 `overflow: auto`，留在里面会被裁掉。
 */
import { nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { ContextMenuItem } from "./contextMenu";

const props = defineProps<{
  open: boolean;
  x: number;
  y: number;
  items: ContextMenuItem[];
}>();

const emit = defineEmits<{
  (e: "select", id: string): void;
  (e: "close"): void;
}>();

const menuEl = ref<HTMLElement | null>(null);
const left = ref(0);
const top = ref(0);

/** 贴边时翻转 / 收进视口，避免菜单被窗口边缘截断。 */
async function place(): Promise<void> {
  left.value = props.x;
  top.value = props.y;
  await nextTick();
  const el = menuEl.value;
  if (!el) return;
  const margin = 8;
  const rect = el.getBoundingClientRect();
  left.value = Math.max(margin, Math.min(props.x, window.innerWidth - rect.width - margin));
  top.value = Math.max(margin, Math.min(props.y, window.innerHeight - rect.height - margin));
}

watch(
  () => [props.open, props.x, props.y] as const,
  () => {
    if (props.open) void place();
  },
  { immediate: true, flush: "post" },
);

function pick(item: ContextMenuItem): void {
  if (item.disabled) return;
  emit("select", item.id);
  emit("close");
}

function onPointerDown(e: PointerEvent): void {
  if (!props.open) return;
  const el = menuEl.value;
  if (el && e.target instanceof Node && el.contains(e.target)) return;
  emit("close");
}

function onKeyDown(e: KeyboardEvent): void {
  if (props.open && e.key === "Escape") {
    e.preventDefault();
    emit("close");
  }
}

/** 滚动时间轴 / 缩放窗口后菜单的锚点就失效了，直接收起。 */
function onDismiss(): void {
  if (props.open) emit("close");
}

onMounted(() => {
  window.addEventListener("pointerdown", onPointerDown, true);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("resize", onDismiss);
  window.addEventListener("scroll", onDismiss, true);
});
onUnmounted(() => {
  window.removeEventListener("pointerdown", onPointerDown, true);
  window.removeEventListener("keydown", onKeyDown);
  window.removeEventListener("resize", onDismiss);
  window.removeEventListener("scroll", onDismiss, true);
});
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      ref="menuEl"
      class="ctx-menu"
      role="menu"
      :style="{ left: `${left}px`, top: `${top}px` }"
      @contextmenu.prevent
    >
      <template v-for="(item, i) in items" :key="item.id">
        <div v-if="item.divided && i > 0" class="ctx-sep" role="separator" />
        <button
          class="ctx-item"
          type="button"
          role="menuitem"
          :class="{ danger: item.danger, disabled: item.disabled }"
          :aria-disabled="item.disabled"
          :disabled="item.disabled"
          @click="pick(item)"
        >
          <span class="ctx-label">{{ item.label }}</span>
          <span v-if="item.shortcut" class="ctx-shortcut">{{ item.shortcut }}</span>
        </button>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
/* 与 `D:\ai-cut` 的右键菜单同一套观感：14px 圆角 + 一圈极淡描边（用阴影画，避免
   双层边框）、6px 内边距、34px 行高、13px 字号；快捷键做成小键帽。 */
.ctx-menu {
  position: fixed;
  z-index: 200;
  min-width: 208px;
  max-width: 280px;
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border-radius: 14px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow:
    var(--md-elevation-3),
    0 0 0 1px rgb(var(--md-outline-variant) / 0.3);
  user-select: none;
}
.ctx-item {
  width: 100%;
  height: 34px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font: inherit;
  font-size: 13px;
  text-align: left;
  cursor: pointer;
  transition: background-color 0.12s ease;
}
.ctx-item:hover:not(.disabled),
.ctx-item:focus-visible:not(.disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha, 0.08));
  outline: none;
}
.ctx-item.danger {
  color: rgb(var(--md-error));
}
.ctx-item.danger:hover:not(.disabled) {
  background-color: rgb(var(--md-error) / 0.12);
}
.ctx-item.disabled {
  color: rgb(var(--md-on-surface-muted));
  opacity: 0.55;
  cursor: default;
}
.ctx-label {
  flex: 1 1 auto;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ctx-shortcut {
  flex: 0 0 auto;
  min-width: 18px;
  height: 18px;
  padding: 0 4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  font-family: inherit;
  font-size: 10px;
  font-weight: 600;
  line-height: 1;
}
.ctx-item.danger .ctx-shortcut {
  color: rgb(var(--md-error) / 0.7);
}
.ctx-sep {
  height: 1px;
  margin: 4px 6px;
  background-color: rgb(var(--md-outline-variant) / 0.55);
}
</style>
