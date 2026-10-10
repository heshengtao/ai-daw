<script setup lang="ts">
/**
 * 插件浏览器（模态框）：`kind` 决定浏览**音色**还是**效果器**。
 *
 * 两者找法一样——搜索（名称、分类、关键词、说明都能命中，中英文都行）、按来源筛选、排序——
 * 但**选中语义不同**：
 * - **音色**：一轨只有一件，所以是「换成它」（`pick`）；
 * - **效果器**：效果链是**可多选**的，所以是「点一下加入、再点一下移出」（`toggle`），
 *   而且同一条效果器可以在链上出现多次，所以这里显示的是**链上的个数**。
 *
 * 外壳（遮罩 / 对话框 / 标题栏）与工具条（搜索药丸 / chips / 排序分段）现在是共享件
 * `ModalShell` / `ListToolbar`——这两个模态长得一样，不该各写一份；本组件只留自己的业务：
 * 搜索 / 筛选 / 排序的**取值逻辑**、结果列表、页脚。
 *
 * 面板由 `ModalShell` Teleport 到 body；Escape 与点遮罩关闭也在那边。
 */
import { computed, nextTick, ref, watch } from "vue";
import { CheckIcon, NoSymbolIcon } from "@heroicons/vue/24/outline";
import type { PluginInfo } from "@/data/plugins";
import { usePluginsStore } from "@/stores/plugins";
import { useI18n } from "@/i18n";
import ModalShell from "@/components/common/ModalShell.vue";
import ListToolbar from "@/components/common/ListToolbar.vue";

const plugins = usePluginsStore();
const i18n = useI18n();

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** 浏览音色还是效果器。 */
    kind?: "instrument" | "effect";
    /** 当前插件 id（音色用它打勾）。 */
    current?: string | null;
    /** 当前效果链（效果器用它显示「链上有几个」并打勾）。 */
    chain?: { plugin_id: string }[];
  }>(),
  { kind: "instrument", current: null, chain: () => [] },
);

const emit = defineEmits<{
  (e: "update:open", value: boolean): void;
  /** 音色：换成它（空串 = 卸载）。 */
  (e: "pick", id: string): void;
  /** 效果器：加入 / 移出效果链。 */
  (e: "toggle", id: string): void;
}>();

/** 是不是效果器模式。 */
const isEffect = computed(() => props.kind === "effect");

/** 链上某个效果器出现了几次（效果链允许同一条重复）。 */
function chainCount(pluginId: string): number {
  return props.chain.filter((f) => f.plugin_id === pluginId).length;
}

/** 是否已作用于当前轨道：音色看「是不是当前乐器」，效果器看「在不在链上」。 */
function isChosen(id: string): boolean {
  return isEffect.value ? chainCount(id) > 0 : props.current === id;
}

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

/** 效果器模式下「内置 / 外部」那两个标签换一套说法（乐器叫「内置乐器」，效果器叫「内置效果」）。 */
const tags = computed(() =>
  isEffect.value
    ? [
        { id: "all" as TagKey, labelKey: "browser.tagAll" },
        { id: "builtin" as TagKey, labelKey: "browser.tagBuiltinFx" },
        { id: "external" as TagKey, labelKey: "browser.tagExternalFx" },
      ]
    : TAGS,
);

const query = ref("");
const tag = ref<TagKey>("all");
const sort = ref<SortKey>("category");
/** 共享工具条的实例：打开时用它把焦点放进搜索框。 */
const toolbar = ref<InstanceType<typeof ListToolbar> | null>(null);

/** 工具条那一组 chip 的描述（只有一组，所以 `ListToolbar` 的渲染与改动前一致）。 */
const chipGroups = computed(() => [
  {
    id: "tag",
    label: i18n.t("browser.filterBySource"),
    chips: tags.value.map((c) => ({ id: c.id, label: i18n.t(c.labelKey) })),
    active: tag.value,
  },
]);

const sortOptions = computed(() => SORTS.map((s) => ({ id: s.id, label: i18n.t(s.labelKey) })));

const results = computed<PluginInfo[]>(() => {
  const q = query.value.trim().toLowerCase();
  let list = (isEffect.value ? plugins.effects : plugins.instruments).filter((it) => {
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

function onChip(payload: { groupId: string; chipId: string }): void {
  if (payload.groupId === "tag") tag.value = payload.chipId as TagKey;
}

function onSort(value: string): void {
  sort.value = value as SortKey;
}

function pick(id: string): void {
  if (isEffect.value) {
    // 效果器**不关窗**：多选是常态，关掉反而要反复打开。
    emit("toggle", id);
    return;
  }
  emit("pick", id);
  close();
}

// 每次打开都重置到干净状态并聚焦搜索框——打开就是来搜的。
watch(
  () => props.open,
  (v) => {
    if (!v) return;
    query.value = "";
    tag.value = "all";
    sort.value = "category";
    void nextTick(() => toolbar.value?.focusSearch());
  },
);
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t(isEffect ? 'browser.titleFx' : 'browser.title')"
    @update:open="close"
  >
    <template #toolbar>
      <ListToolbar
        ref="toolbar"
        :query="query"
        :placeholder="i18n.t(isEffect ? 'browser.searchPlaceholderFx' : 'browser.searchPlaceholder')"
        :search-label="i18n.t(isEffect ? 'browser.searchPlaceholderFx' : 'browser.searchLabel')"
        :clear-label="i18n.t('browser.clearSearch')"
        :chip-groups="chipGroups"
        :sort-label="i18n.t('browser.sort')"
        :sort-aria-label="i18n.t('browser.sortBy')"
        :sorts="sortOptions"
        :active-sort="sort"
        @update:query="query = $event"
        @update:active-chip="onChip"
        @update:active-sort="onSort"
      />
    </template>

    <div class="body">
      <ul v-if="results.length > 0" class="list">
        <li v-for="it in results" :key="it.id">
          <button class="row" :class="{ on: isChosen(it.id) }" @click="pick(it.id)">
            <span class="row-main">
              <span class="row-top">
                <span class="row-name">{{ it.name }}</span>
                <span class="badge" :class="{ builtin: it.builtin }">{{ it.category }}</span>
                <!-- 效果链允许同一条重复，所以给的是**个数**而不是「已选」。 -->
                <span v-if="isEffect && chainCount(it.id) > 1" class="badge count">
                  {{ i18n.t("browser.onChain", { n: chainCount(it.id) }) }}
                </span>
              </span>
              <span class="row-note">{{ it.note }}</span>
            </span>
            <CheckIcon v-if="isChosen(it.id)" class="row-check" />
          </button>
        </li>
      </ul>

      <div v-else class="empty">
        <NoSymbolIcon class="empty-icon" />
        <p class="empty-text">
          {{ i18n.t(isEffect ? "browser.emptyFx" : "browser.empty") }}
        </p>
        <p class="empty-hint">{{ i18n.t("browser.emptyHint") }}</p>
      </div>
    </div>

    <template #foot>
      <footer class="foot">
        <span class="count">{{
          i18n.t(isEffect ? "browser.countFx" : "browser.count", {
            shown: results.length,
            total: (isEffect ? plugins.effects : plugins.instruments).length,
          })
        }}</span>
        <!-- 音色可以一键卸载；效果器的「移出」就是再点一次那一行（可多选）。 -->
        <button v-if="!isEffect && current" class="clear-btn" @click="pick('')">
          {{ i18n.t("browser.unload") }}
        </button>
      </footer>
    </template>
  </ModalShell>
</template>

<style scoped>
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
.badge.count {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
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
</style>
