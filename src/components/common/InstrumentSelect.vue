<script setup lang="ts">
/**
 * 音色选择器：一个**分类下拉菜单**。
 *
 * 为什么不用一排 chips：音色会越来越多，横排按钮很快就铺不下、也扫不过来。
 * 下拉里按分类分组，底部再给一个「浏览全部音色…」入口打开模态框
 * （那里有搜索、分类筛选与排序）。
 *
 * 下拉面板用 Teleport 到 body + fixed 定位：检查器是可滚动容器，
 * 面板挂在里面会被裁掉。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { ChevronDownIcon, MagnifyingGlassIcon, CheckIcon, NoSymbolIcon } from "@heroicons/vue/24/outline";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";
import InstrumentBrowser from "@/components/common/InstrumentBrowser.vue";

const i18n = useI18n();

const plugins = usePluginsStore();

const props = defineProps<{
  /** 当前插件 id；空串 / null 表示「无乐器」。 */
  modelValue: string | null;
  disabled?: boolean;
}>();

const emit = defineEmits<{ (e: "update:modelValue", value: string): void }>();

const open = ref(false);
const browserOpen = ref(false);
const triggerEl = ref<HTMLElement | null>(null);
const menuStyle = ref<Record<string, string>>({});

const currentName = computed(
  () => plugins.info(props.modelValue)?.name ?? i18n.t("instrument.none"),
);

/** 按分类分组（只保留有内容的分类）。 */
const groups = computed(() =>
  plugins.categories.map((category) => ({
    category,
    items: plugins.instruments.filter((i) => i.category === category),
  })).filter((g) => g.items.length > 0),
);

function place(): void {
  const el = triggerEl.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const width = Math.max(268, r.width);
  const spaceBelow = window.innerHeight - r.bottom - 12;
  const spaceAbove = r.top - 12;
  const openUp = spaceBelow < 220 && spaceAbove > spaceBelow;
  const maxHeight = Math.max(160, Math.min(340, openUp ? spaceAbove : spaceBelow));
  menuStyle.value = {
    left: `${Math.max(8, Math.min(r.left, window.innerWidth - width - 8))}px`,
    width: `${width}px`,
    maxHeight: `${maxHeight}px`,
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
  if (id !== (props.modelValue ?? "")) emit("update:modelValue", id);
}

/** 从浏览器里选中后同样关闭下拉。 */
function pickFromBrowser(id: string): void {
  close();
  if (id !== (props.modelValue ?? "")) emit("update:modelValue", id);
}

/** 点击面板与触发器之外的地方就收起。 */
function onWindowPointerDown(e: PointerEvent): void {
  if (!open.value) return;
  const target = e.target as Node;
  if (triggerEl.value?.contains(target)) return;
  if (document.getElementById("instrument-select-menu")?.contains(target)) return;
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

watch(open, (v) => {
  if (!v) return;
  window.addEventListener("scroll", onViewportChange, true);
  window.addEventListener("resize", onViewportChange);
});

onMounted(() => {
  window.addEventListener("pointerdown", onWindowPointerDown, true);
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  window.removeEventListener("pointerdown", onWindowPointerDown, true);
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("scroll", onViewportChange, true);
  window.removeEventListener("resize", onViewportChange);
});
</script>

<template>
  <div class="select">
    <button
      ref="triggerEl"
      class="trigger"
      type="button"
      :disabled="disabled"
      :aria-expanded="open"
      aria-haspopup="listbox"
      :aria-label="i18n.t('instrument.choose')"
      @click="toggle"
    >
      <span class="trigger-name">{{ currentName }}</span>
      <ChevronDownIcon class="trigger-caret" :class="{ up: open }" />
    </button>

    <Teleport to="body">
      <transition name="drop">
        <div
          v-if="open"
          id="instrument-select-menu"
          class="menu"
          :style="menuStyle"
          role="listbox"
          :aria-label="i18n.t('instrument.list')"
        >
          <button
            class="item none"
            type="button"
            role="option"
            :aria-selected="!modelValue"
            @click="pick('')"
          >
            <NoSymbolIcon class="item-icon" />
            <span class="item-name">{{ i18n.t("instrument.none") }}</span>
            <CheckIcon v-if="!modelValue" class="item-check" />
          </button>

          <div v-for="g in groups" :key="g.category" class="group">
            <div class="group-title">{{ g.category }}</div>
            <button
              v-for="it in g.items"
              :key="it.id"
              class="item"
              type="button"
              role="option"
              :aria-selected="modelValue === it.id"
              @click="pick(it.id)"
            >
              <span class="item-main">
                <span class="item-name">{{ it.name }}</span>
                <span class="item-note">{{ it.note }}</span>
              </span>
              <CheckIcon v-if="modelValue === it.id" class="item-check" />
            </button>
          </div>

          <div class="menu-foot">
            <button class="browse" type="button" @click="((browserOpen = true), close())">
              <MagnifyingGlassIcon class="browse-icon" />
              {{ i18n.t("instrument.browse") }}
            </button>
          </div>
        </div>
      </transition>
    </Teleport>

    <InstrumentBrowser
      v-model:open="browserOpen"
      :current="modelValue"
      @pick="pickFromBrowser"
    />
  </div>
</template>

<style scoped>
.select {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
}
.trigger {
  width: 100%;
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 0 10px 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 10px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s ease, background-color 0.15s ease;
}
.trigger:hover:not(:disabled) {
  border-color: rgb(var(--md-primary) / 0.7);
}
.trigger:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.trigger:disabled {
  opacity: 0.5;
  cursor: default;
}
.trigger-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.trigger-caret {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
  transition: transform 0.18s ease;
}
.trigger-caret.up {
  transform: rotate(180deg);
}
</style>

<style>
/* 面板 Teleport 到 body，因此不能写在 scoped 里 */
#instrument-select-menu.menu {
  position: fixed;
  z-index: 90;
  display: flex;
  flex-direction: column;
  padding: 6px;
  overflow-y: auto;
  border-radius: 14px;
  background-color: rgb(var(--md-surface-container-high));
  border: 1px solid rgb(var(--md-outline-variant) / 0.5);
  box-shadow: var(--md-elevation-3);
}
#instrument-select-menu .group-title {
  padding: 8px 10px 4px;
  font-size: 10px;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
}
#instrument-select-menu .item {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border: 0;
  border-radius: 10px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
#instrument-select-menu .item:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
#instrument-select-menu .item.none {
  color: rgb(var(--md-on-surface-variant));
}
#instrument-select-menu .item-icon {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
}
#instrument-select-menu .item-main {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  flex: 1 1 auto;
}
#instrument-select-menu .item-name {
  font-size: 12.5px;
  font-weight: 500;
}
#instrument-select-menu .item.none .item-name {
  font-weight: 400;
}
#instrument-select-menu .item-note {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
#instrument-select-menu .item-check {
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
}
#instrument-select-menu .menu-foot {
  position: sticky;
  bottom: -6px;
  margin-top: 6px;
  padding: 6px 0 0;
  background-color: rgb(var(--md-surface-container-high));
}
#instrument-select-menu .browse {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 7px;
  height: 32px;
  border: 1px dashed rgb(var(--md-outline-variant));
  border-radius: 10px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
#instrument-select-menu .browse:hover {
  background-color: rgb(var(--md-primary) / 0.08);
}
#instrument-select-menu .browse-icon {
  width: 14px;
  height: 14px;
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
