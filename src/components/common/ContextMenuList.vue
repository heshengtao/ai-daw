<script setup lang="ts">
/**
 * 右键菜单的**一层**（根层与二级菜单共用同一份渲染）。
 *
 * 观感参考 `D:\ai-cut` 的 `MenuPanel.vue`：行首图标 + 键帽 + AI 徽标，行高 32px；
 * 子菜单悬停 90ms 才展开（斜着划过时不会一路弹开），靠右放不下就翻到左边。
 *
 * 递归渲染自己：菜单项里带 `children` 就自动成为可展开项，调用方只描述结构，
 * 不用关心层级、翻转、贴边这些排版细节。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { CheckIcon, ChevronRightIcon } from "@heroicons/vue/24/outline";
import type { ContextMenuItem } from "./contextMenu";

/** 触发行在视口里的位置（子菜单的锚点）。 */
interface Anchor {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

const props = withDefaults(
  defineProps<{
    items: ContextMenuItem[];
    /** 根层：鼠标位置。 */
    x?: number;
    y?: number;
    /** 二级菜单：触发行的视口矩形。 */
    anchor?: Anchor | null;
    /** 层级（0 = 根层）；只用来抬 z-index，后开的子菜单永远压在父菜单上。 */
    depth?: number;
  }>(),
  { x: 0, y: 0, anchor: null, depth: 0 },
);

const emit = defineEmits<{
  (e: "select", id: string): void;
  (e: "back"): void;
}>();

const panelEl = ref<HTMLElement | null>(null);
const position = ref({ left: 0, top: 0 });
const placed = ref(false);
const activeId = ref<string | null>(null);
const subAnchor = ref<Anchor | null>(null);
let hoverTimer: number | null = null;

const activeSub = computed<ContextMenuItem | null>(() => {
  const item = props.items.find((candidate) => candidate.id === activeId.value);
  return item && item.children && item.children.length > 0 ? item : null;
});

/** 贴着锚点 / 鼠标落位，并夹在窗口内；右边放不下就翻到触发行左侧。 */
function measure(): void {
  const el = panelEl.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const margin = 8;
  const maxLeft = Math.max(margin, window.innerWidth - rect.width - margin);
  const maxTop = Math.max(margin, window.innerHeight - rect.height - margin);

  let left = props.anchor ? props.anchor.right + 2 : props.x;
  let top = props.anchor ? props.anchor.top - 6 : props.y;

  if (props.anchor && left + rect.width > window.innerWidth - margin) {
    left = props.anchor.left - rect.width - 2;
  }

  position.value = {
    left: Math.min(Math.max(margin, left), maxLeft),
    top: Math.min(Math.max(margin, top), maxTop),
  };
  placed.value = true;
}

/** 当前层可聚焦的行（禁用项跳过）。 */
function rows(): HTMLButtonElement[] {
  return Array.from(
    panelEl.value?.querySelectorAll<HTMLButtonElement>(".ctx-item:not(.disabled)") ?? [],
  );
}

function focusFirst(): void {
  rows()[0]?.focus({ preventScroll: true });
}

function moveFocus(delta: number): void {
  const list = rows();
  if (list.length === 0) return;
  const index = list.indexOf(document.activeElement as HTMLButtonElement);
  const next = index < 0 ? 0 : (index + delta + list.length) % list.length;
  const row = list[next];
  row?.focus();
  row?.scrollIntoView({ block: "nearest" });
}

onMounted(async () => {
  await nextTick();
  measure();
  // 等 `placed` 把面板从 visibility:hidden 切回来，再抢焦点。
  await nextTick();
  focusFirst();
});

onBeforeUnmount(() => {
  if (hoverTimer !== null) window.clearTimeout(hoverTimer);
});

// 根层位置 / 子菜单锚点变了：重新量一次（内容更新时也重排）。
watch(
  () => [props.anchor, props.x, props.y, props.items] as const,
  async () => {
    placed.value = false;
    await nextTick();
    measure();
  },
  { flush: "post" },
);

function pick(item: ContextMenuItem): void {
  if (item.disabled) return;
  emit("select", item.id);
}

function openWithRect(item: ContextMenuItem, rect: Anchor): void {
  if (item.disabled || !item.children?.length) return;
  subAnchor.value = rect;
  activeId.value = item.id;
}

function openNow(item: ContextMenuItem, event: Event): void {
  if (item.disabled || !item.children?.length) return;
  if (activeId.value === item.id) {
    activeId.value = null;
    return;
  }
  const el = event.currentTarget as HTMLElement | null;
  if (el) openWithRect(item, el.getBoundingClientRect());
}

/** 悬停一小会儿再展开，斜着划过子菜单时不会一路弹开。 */
function scheduleOpen(item: ContextMenuItem, event: Event): void {
  if (item.disabled || !item.children?.length) return;
  const el = event.currentTarget as HTMLElement | null;
  if (!el || activeId.value === item.id) return;
  if (hoverTimer !== null) window.clearTimeout(hoverTimer);
  const rect = el.getBoundingClientRect();
  hoverTimer = window.setTimeout(() => openWithRect(item, rect), 90);
}

function onRowEnter(item: ContextMenuItem, event: Event): void {
  if (hoverTimer !== null) {
    window.clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  if (item.children?.length) {
    scheduleOpen(item, event);
    return;
  }
  if (activeId.value !== null) activeId.value = null;
}

/** 子菜单按 ← 退回本层：关掉子菜单并把焦点还给触发它的那一行。 */
function closeSub(): void {
  const id = activeId.value;
  activeId.value = null;
  if (!id) return;
  void nextTick(() => {
    panelEl.value
      ?.querySelector<HTMLButtonElement>(`[data-submenu-id="${id}"]`)
      ?.focus({ preventScroll: true });
  });
}

function onKeyDown(event: KeyboardEvent): void {
  // Esc 交给外层宿主关闭；其余按键只作用于当前这一层，避免触发时间线的全局快捷键。
  if (event.key === "Escape") return;
  event.stopPropagation();

  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    moveFocus(event.key === "ArrowDown" ? 1 : -1);
    return;
  }
  if (event.key === "ArrowRight") {
    const el = document.activeElement as HTMLButtonElement | null;
    const id = el?.dataset.submenuId;
    if (!id || !el) return;
    const item = props.items.find((candidate) => candidate.id === id);
    if (item) {
      event.preventDefault();
      openWithRect(item, el.getBoundingClientRect());
    }
    return;
  }
  if (event.key === "ArrowLeft" && props.depth > 0) {
    event.preventDefault();
    emit("back");
  }
}
</script>

<template>
  <div
    ref="panelEl"
    class="ctx-menu"
    role="menu"
    :style="{
      left: `${position.left}px`,
      top: `${position.top}px`,
      zIndex: 200 + depth * 2,
      visibility: placed ? 'visible' : 'hidden',
    }"
    @contextmenu.prevent.stop
    @scroll.passive="activeId = null"
    @keydown="onKeyDown"
  >
    <template v-for="(item, i) in items" :key="item.id">
      <div v-if="item.divided && i > 0" class="ctx-sep" role="separator" />

      <button
        class="ctx-item"
        :class="{
          disabled: item.disabled,
          danger: item.danger,
          ai: item.ai,
          on: item.children?.length && activeId === item.id,
        }"
        type="button"
        :role="item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'"
        :tabindex="item.disabled ? -1 : 0"
        :title="item.hint ?? item.label"
        :aria-checked="item.checked"
        :aria-disabled="item.disabled || undefined"
        :aria-haspopup="item.children?.length ? 'menu' : undefined"
        :aria-expanded="item.children?.length ? activeId === item.id : undefined"
        :data-submenu-id="item.children?.length ? item.id : undefined"
        @mouseenter="onRowEnter(item, $event)"
        @click="item.children?.length ? openNow(item, $event) : pick(item)"
      >
        <component :is="item.icon" v-if="item.icon" class="ctx-icon" />
        <span v-else class="ctx-icon ctx-icon-empty" />

        <span class="ctx-label">{{ item.label }}</span>

        <CheckIcon v-if="item.checked" class="ctx-check" />

        <span v-if="item.badge" class="ctx-badge">{{ item.badge }}</span>

        <span v-if="item.keys?.length" class="ctx-keys">
          <template v-for="(key, index) in item.keys" :key="key">
            <span v-if="index > 0" class="ctx-plus">+</span>
            <kbd class="ctx-key">{{ key }}</kbd>
          </template>
        </span>

        <ChevronRightIcon v-if="item.children?.length" class="ctx-chevron" />
      </button>

      <ContextMenuList
        v-if="item.children?.length && activeSub?.id === item.id && subAnchor"
        :items="item.children"
        :depth="depth + 1"
        :x="0"
        :y="0"
        :anchor="subAnchor"
        @select="emit('select', $event)"
        @back="closeSub"
      />
    </template>
  </div>
</template>

<style scoped>
/* 与 `D:\ai-cut` 的 MenuPanel 同一套观感：238px 起、32px 行高、6px 内边距、
   图标 16px、键帽 18px；子菜单固定定位，不被父级滚动容器裁切。 */
.ctx-menu {
  position: fixed;
  z-index: 200;
  min-width: 238px;
  max-width: 330px;
  max-height: min(72vh, 620px);
  padding: 6px;
  display: flex;
  flex-direction: column;
  gap: 1px;
  overflow-y: auto;
  overscroll-behavior: contain;
  border-radius: 14px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow:
    var(--md-elevation-3),
    0 0 0 1px rgb(var(--md-outline-variant) / 0.3);
  user-select: none;
  scrollbar-width: thin;
}
.ctx-menu::-webkit-scrollbar {
  width: 8px;
}
.ctx-menu::-webkit-scrollbar-thumb {
  border: 2px solid transparent;
  border-radius: 999px;
  background-clip: content-box;
  background-color: rgb(var(--md-outline) / 0.5);
}
.ctx-item {
  width: 100%;
  min-height: 32px;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 8px 0 9px;
  border: 0;
  border-radius: 9px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font: inherit;
  font-size: 12.5px;
  line-height: 1.3;
  text-align: left;
  cursor: pointer;
  outline: none;
  transition:
    background-color 0.1s ease,
    color 0.1s ease;
}
.ctx-item:hover:not(.disabled),
.ctx-item:focus-visible:not(.disabled),
.ctx-item.on:not(.disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha, 0.08));
}
.ctx-item:focus-visible {
  box-shadow: inset 0 0 0 2px rgb(var(--md-primary) / 0.7);
}
.ctx-item.disabled {
  color: rgb(var(--md-on-surface-muted));
  opacity: 0.5;
  cursor: default;
}
.ctx-item.disabled:hover {
  background-color: transparent;
}
.ctx-item.danger {
  color: rgb(var(--md-error));
}
.ctx-item.danger:not(.disabled):hover,
.ctx-item.danger:not(.disabled):focus-visible {
  background-color: rgb(var(--md-error) / 0.12);
}
.ctx-item.ai .ctx-icon {
  color: rgb(var(--md-primary));
}
.ctx-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  opacity: 0.9;
}
.ctx-icon-empty {
  display: block;
}
.ctx-label {
  flex: 1 1 auto;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ctx-check {
  width: 14px;
  height: 14px;
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
}
.ctx-badge {
  flex: 0 0 auto;
  padding: 1px 5px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 0.3px;
  line-height: 14px;
}
.ctx-keys {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex: 0 0 auto;
}
.ctx-key {
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
.ctx-plus {
  color: rgb(var(--md-on-surface-muted));
  font-size: 9px;
}
.ctx-chevron {
  width: 13px;
  height: 13px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-muted));
}
.ctx-sep {
  height: 1px;
  margin: 4px 6px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-outline-variant) / 0.55);
}
</style>
