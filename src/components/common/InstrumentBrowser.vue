<script setup lang="ts">
/**
 * 音色浏览器（模态框）：音色多起来之后，光靠下拉列表就不够用了。
 *
 * 提供三种找法：
 * - **搜索**：名称、分类、关键词、说明都能命中（中英文都行）；
 * - **筛选**：按分类（合成器 / 预设音色 / 外部插件）；
 * - **排序**：分类顺序 / 名称 A→Z / 内置优先。
 *
 * 面板用 Teleport 到 body，并自己管焦点与 Escape。
 */
import { computed, nextTick, ref, watch } from "vue";
import { MagnifyingGlassIcon, XMarkIcon, CheckIcon, NoSymbolIcon } from "@heroicons/vue/24/outline";
import type { PluginInfo } from "@/data/plugins";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";

const plugins = usePluginsStore();
const i18n = useI18n();

const props = defineProps<{
  open: boolean;
  /** 当前插件 id（用于打勾）。 */
  current: string | null;
}>();

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  (e: "pick", id: string): void;
}>();

type SortKey = "category" | "name" | "builtin";

const SORTS: { id: SortKey; labelKey: string }[] = [
  { id: "category", labelKey: "browser.sortCategory" },
  { id: "name", labelKey: "browser.sortName" },
  { id: "builtin", labelKey: "browser.sortBuiltin" },
];

/** 三个 TAG：全部 / 内置乐器 / 外部乐器。 */
type TagKey = "all" | "builtin" | "external";

const TAGS: { id: TagKey; labelKey: string }[] = [
  { id: "all", labelKey: "browser.tagAll" },
  { id: "builtin", labelKey: "browser.tagBuiltin" },
  { id: "external", labelKey: "browser.tagExternal" },
];

const query = ref("");
const tag = ref<TagKey>("all");
const sort = ref<SortKey>("category");
const searchEl = ref<HTMLInputElement | null>(null);

const results = computed<PluginInfo[]>(() => {
  const q = query.value.trim().toLowerCase();
  let list = plugins.instruments.filter((it) => {
    if (tag.value === "builtin" && !it.builtin) return false;
    if (tag.value === "external" && it.builtin) return false;
    if (!q) return true;
    const haystack = [it.name, it.id, it.category, it.note, ...it.tags].join(" ").toLowerCase();
    return haystack.includes(q);
  });
  if (sort.value === "name") {
    list = [...list].sort((a, b) => a.name.localeCompare(b.name, "zh-Hans-CN"));
  } else if (sort.value === "builtin") {
    list = [...list].sort((a, b) => Number(b.builtin) - Number(a.builtin));
  }
  return list;
});

function close(): void {
  emit("update:open", false);
}

function pick(id: string): void {
  emit("pick", id);
  close();
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.stopPropagation();
    close();
  }
}

// 每次打开都重置到干净状态并聚焦搜索框——打开就是来搜的。
watch(
  () => props.open,
  (v) => {
    if (!v) return;
    query.value = "";
    tag.value = "all";
    sort.value = "category";
    void nextTick(() => searchEl.value?.focus());
  },
);
</script>

<template>
  <Teleport to="body">
    <transition name="fade">
      <div v-if="open" class="scrim" @pointerdown.self="close">
        <div
          class="dialog"
          role="dialog"
          aria-modal="true"
          :aria-label="i18n.t('browser.title')"
          tabindex="-1"
          @keydown="onKey"
        >
          <header class="head">
            <h2 class="title">{{ i18n.t("browser.title") }}</h2>
            <button class="icon-btn" :aria-label="i18n.t('common.close')" @click="close">
              <XMarkIcon class="w-[18px] h-[18px]" />
            </button>
          </header>

          <div class="toolbar">
            <label class="search">
              <MagnifyingGlassIcon class="search-icon" />
              <input
                ref="searchEl"
                v-model="query"
                class="search-input"
                type="text"
                spellcheck="false"
                autocomplete="off"
                :placeholder="i18n.t('browser.searchPlaceholder')"
                :aria-label="i18n.t('browser.searchLabel')"
              />
              <button v-if="query" class="search-clear" :aria-label="i18n.t('browser.clearSearch')" @click="query = ''">
                <XMarkIcon class="w-[14px] h-[14px]" />
              </button>
            </label>

            <div class="chips" role="group" :aria-label="i18n.t('browser.filterBySource')">
              <button
                v-for="c in TAGS"
                :key="c.id"
                class="chip"
                :class="{ on: tag === c.id }"
                :aria-pressed="tag === c.id"
                @click="tag = c.id"
              >
                {{ i18n.t(c.labelKey) }}
              </button>
            </div>

            <div class="sorts">
              <span class="sorts-label">{{ i18n.t("browser.sort") }}</span>
              <div class="seg" role="group" :aria-label="i18n.t('browser.sortBy')">
                <button
                  v-for="s in SORTS"
                  :key="s.id"
                  class="seg-btn"
                  :class="{ on: sort === s.id }"
                  :aria-pressed="sort === s.id"
                  @click="sort = s.id"
                >
                  {{ i18n.t(s.labelKey) }}
                </button>
              </div>
            </div>
          </div>

          <div class="body">
            <ul v-if="results.length > 0" class="list">
              <li v-for="it in results" :key="it.id">
                <button class="row" :class="{ on: current === it.id }" @click="pick(it.id)">
                  <span class="row-main">
                    <span class="row-top">
                      <span class="row-name">{{ it.name }}</span>
                      <span class="badge" :class="{ builtin: it.builtin }">{{ it.category }}</span>
                    </span>
                    <span class="row-note">{{ it.note }}</span>
                  </span>
                  <CheckIcon v-if="current === it.id" class="row-check" />
                </button>
              </li>
            </ul>

            <div v-else class="empty">
              <NoSymbolIcon class="empty-icon" />
              <p class="empty-text">{{ i18n.t("browser.empty") }}</p>
              <p class="empty-hint">{{ i18n.t("browser.emptyHint") }}</p>
            </div>
          </div>

          <footer class="foot">
            <span class="count">{{
              i18n.t("browser.count", { shown: results.length, total: plugins.instruments.length })
            }}</span>
            <button v-if="current" class="clear-btn" @click="pick('')">{{ i18n.t("browser.unload") }}</button>
          </footer>
        </div>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.scrim {
  position: fixed;
  inset: 0;
  z-index: 120;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  background-color: rgb(var(--md-scrim) / 0.42);
}
.dialog {
  width: 620px;
  max-width: 100%;
  max-height: 78vh;
  display: flex;
  flex-direction: column;
  border-radius: 24px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: var(--md-elevation-3);
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 16px 10px 22px;
}
.title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: rgb(var(--md-on-surface));
}
.icon-btn {
  width: 34px;
  height: 34px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.icon-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* ---- 工具条 ---- */
.toolbar {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 0 22px 12px;
}
.search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 40px;
  padding: 0 10px 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container));
}
.search:focus-within {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.2);
}
.search-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  color: rgb(var(--md-on-surface-variant));
}
.search-input {
  flex: 1 1 auto;
  min-width: 0;
  border: 0;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 13px;
  outline: none;
}
.search-clear {
  width: 24px;
  height: 24px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.search-clear:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.chip {
  height: 28px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.chip.on {
  background-color: rgb(var(--md-secondary-container));
  border-color: transparent;
  color: rgb(var(--md-on-secondary-container));
}
.chip:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.sorts {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sorts-label {
  font-size: 10.5px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-variant));
}
.seg {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container));
}
.seg-btn {
  height: 24px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.seg-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.seg-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* ---- 列表 ---- */
.body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 0 14px 8px;
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.row {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 0;
  border-radius: 14px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.14s ease;
}
.row:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.row.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.row:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: -2px;
}
.row-main {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1 1 auto;
}
.row-top {
  display: flex;
  align-items: center;
  gap: 8px;
}
.row-name {
  font-size: 13px;
  font-weight: 500;
}
.badge {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.badge.builtin {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.row-note {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.row.on .row-note {
  color: rgb(var(--md-on-secondary-container));
}
.row-check {
  width: 17px;
  height: 17px;
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
}
.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 44px 16px;
  color: rgb(var(--md-on-surface-muted));
}
.empty-icon {
  width: 26px;
  height: 26px;
}
.empty-text {
  margin: 0;
  font-size: 13px;
  color: rgb(var(--md-on-surface-variant));
}
.empty-hint {
  margin: 0;
  font-size: 11.5px;
}
/* ---- 页脚 ---- */
.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 22px 14px;
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.count {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
}
.clear-btn {
  height: 30px;
  padding: 0 14px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.clear-btn:hover {
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.16s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>
