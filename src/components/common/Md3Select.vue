<script setup lang="ts">
/**
 * MD3 下拉框：一颗药丸按钮 + 弹出菜单（**不是**原生 `<select>`）。
 *
 * 原生下拉的样式由系统接管，和 MD3 的圆角 / 触感完全对不上；这里自己画，行为仍然是最朴素的
 * 「选一项 → 关掉」。
 *
 * 菜单 Teleport 到 body 并用 fixed 定位：检查器是可滚动容器，绝对定位的面板会被裁掉。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { ArrowRightIcon, CheckIcon, ChevronDownIcon } from "@heroicons/vue/24/outline";
import { useI18n } from "@/i18n";

export interface SelectOption {
  id: string;
  label: string;
  /** 第二行小字（可选）。 */
  hint?: string;
  /** 分组标题（可选）：同 group 的选项渲染在同一个标题下。 */
  group?: string;
}

const props = withDefaults(
  defineProps<{
    modelValue?: string | null;
    options: SelectOption[];
    /** 无障碍名（也是没选中时显示在按钮上的占位文案）。 */
    label: string;
    placeholder?: string;
    /** 候选项为空时的提示文案；不给就用通用文案。 */
    empty?: string;
    disabled?: boolean;
    /** 菜单底部的一个动作按钮（例如「浏览全部音色」）。 */
    action?: { label: string };
  }>(),
  { placeholder: "—" },
);

const emit = defineEmits<{
  (e: "update:modelValue", value: string): void;
  (e: "select", value: string): void;
  (e: "action"): void;
}>();

const { t } = useI18n();

/** 没有候选项时展开菜单显示的空态文案（比如「一个效果器插件都没有」）。 */
const emptyText = computed(() => props.empty ?? t("select.empty"));

/** 选项按 group 折叠成区块（保持与传入顺序一致）。 */
const groups = computed<{ title: string; items: SelectOption[] }[]>(() => {
  const out: { title: string; items: SelectOption[] }[] = [];
  const index = new Map<string, number>();
  for (const o of props.options) {
    const key = o.group ?? "";
    let at = index.get(key);
    if (at === undefined) {
      at = out.length;
      index.set(key, at);
      out.push({ title: key, items: [] });
    }
    out[at]!.items.push(o);
  }
  return out;
});

const open = ref(false);
const triggerEl = ref<HTMLElement | null>(null);
const menuEl = ref<HTMLElement | null>(null);
const menuStyle = ref<Record<string, string>>({});

const selected = computed(() => props.options.find((o) => o.id === props.modelValue) ?? null);
const text = computed(() => selected.value?.label ?? props.placeholder);

/** 贴着触发器放；下方空间不够就翻到上方。 */
function place(): void {
  const el = triggerEl.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const width = Math.max(200, r.width);
  const spaceBelow = window.innerHeight - r.bottom - 12;
  const spaceAbove = r.top - 12;
  const openUp = spaceBelow < 180 && spaceAbove > spaceBelow;
  menuStyle.value = {
    left: `${Math.max(8, Math.min(r.left, window.innerWidth - width - 8))}px`,
    width: `${width}px`,
    maxHeight: `${Math.max(140, Math.min(320, openUp ? spaceAbove : spaceBelow))}px`,
    ...(openUp ? { bottom: `${window.innerHeight - r.top + 6}px` } : { top: `${r.bottom + 6}px` }),
  };
}

function toggle(): void {
  if (props.disabled) return;
  open.value = !open.value;
  if (open.value) void nextTick(place);
}

function close(): void {
  open.value = false;
}

function pick(id: string): void {
  close();
  if (id === props.modelValue) return;
  emit("update:modelValue", id);
  emit("select", id);
}

/** 点击菜单底部的动作行：先收起菜单，再通知父组件（例如打开「浏览全部音色」）。 */
function runAction(): void {
  close();
  emit("action");
}

function onDocPointer(e: PointerEvent): void {
  if (!open.value) return;
  const target = e.target as Node;
  if (triggerEl.value?.contains(target) || menuEl.value?.contains(target)) return;
  close();
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape" && open.value) {
    e.stopPropagation();
    close();
  }
}

function onViewportChange(): void {
  if (open.value) place();
}

watch(open, (isOpen) => {
  if (!isOpen) return;
  window.addEventListener("scroll", onViewportChange, true);
  window.addEventListener("resize", onViewportChange);
});

onMounted(() => {
  window.addEventListener("pointerdown", onDocPointer, true);
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  window.removeEventListener("pointerdown", onDocPointer, true);
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("scroll", onViewportChange, true);
  window.removeEventListener("resize", onViewportChange);
});
</script>

<template>
  <button
    ref="triggerEl"
    class="md3-select"
    type="button"
    :disabled="disabled"
    :aria-expanded="open"
    aria-haspopup="listbox"
    :aria-label="label"
    @click="toggle"
  >
    <span class="value" :class="{ placeholder: !selected }">{{ text }}</span>
    <ChevronDownIcon class="caret" :class="{ up: open }" />
  </button>

  <Teleport to="body">
    <transition name="drop">
      <div
        v-if="open"
        ref="menuEl"
        class="md3-select-menu"
        :style="menuStyle"
        role="listbox"
        :aria-label="label"
      >
        <p v-if="options.length === 0" class="empty">{{ emptyText }}</p>
        <template v-for="g in groups" :key="g.title">
          <!-- 分组标题：`listbox` 里合法子项只有 `option`，标题标记成 presentation 免得成为杂项节点。 -->
          <p v-if="g.title" class="group-title" role="presentation">{{ g.title }}</p>
          <button
            v-for="o in g.items"
            :key="o.id"
            class="item"
            type="button"
            role="option"
            :aria-selected="o.id === modelValue"
            @click="pick(o.id)"
          >
            <span class="item-main">
              <span class="item-label">{{ o.label }}</span>
              <span v-if="o.hint" class="item-hint">{{ o.hint }}</span>
            </span>
            <CheckIcon v-if="o.id === modelValue" class="item-check" />
          </button>
        </template>

        <template v-if="action">
          <span class="menu-sep" aria-hidden="true" />
          <button class="item action" type="button" @click="runAction">
            <span class="item-main"><span class="item-label">{{ action.label }}</span></span>
            <ArrowRightIcon class="item-check" />
          </button>
        </template>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.md3-select {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  height: 34px;
  padding: 0 10px 0 12px;
  border: 1px solid rgb(var(--md-outline) / 0.7);
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}
.md3-select:hover:not(:disabled) {
  border-color: rgb(var(--md-primary) / 0.8);
}
.md3-select:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.md3-select:disabled {
  opacity: 0.5;
  cursor: default;
}
.value {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.value.placeholder {
  color: rgb(var(--md-on-surface-variant));
}
.caret {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
  transition: transform 0.18s ease;
}
.caret.up {
  transform: rotate(180deg);
}
</style>

<style>
/* 菜单 Teleport 到 body，所以不能写在 scoped 里 */
.md3-select-menu {
  position: fixed;
  /* 必须在**模态遮罩之上**：参数页里的下拉（插件筛选 / 每页）就挂在这个模态里，
     而 `ModalShell` 的 `.scrim` 是 120、其它对话框是 130。菜单是 Teleport 到 body 的
     `.scrim` 的兄弟节点，压在它下面的话既看不见、点击还会被遮罩的 `pointerdown.self` 吃掉。 */
  z-index: 140;
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px;
  overflow-y: auto;
  border-radius: 16px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.5);
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-3);
}
.md3-select-menu .group-title {
  margin: 4px 0 2px;
  padding: 2px 10px;
  font-size: 10px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-muted));
}
.md3-select-menu .item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.md3-select-menu .item:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.md3-select-menu .item-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1 1 auto;
}
.md3-select-menu .item-label {
  font-size: 12.5px;
}
.md3-select-menu .item-hint {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.md3-select-menu .item-check {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
}
.md3-select-menu .menu-sep {
  height: 1px;
  margin: 4px 6px;
  background-color: rgb(var(--md-outline-variant) / 0.5);
}
.md3-select-menu .item.action .item-label {
  color: rgb(var(--md-primary));
}
.md3-select-menu .empty {
  margin: 0;
  padding: 14px 12px;
  font-size: 11.5px;
  line-height: 1.6;
  text-align: center;
  color: rgb(var(--md-on-surface-muted));
}
.drop-enter-active,
.drop-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}
.drop-enter-from,
.drop-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.98);
}
@media (prefers-reduced-motion: reduce) {
  .drop-enter-active,
  .drop-leave-active {
    transition: opacity 0.12s ease;
  }
}
</style>
