<script setup lang="ts">
/**
 * 参数页（模态）：把某条轨道上**全部插件的全部参数**列出来，每行一个绑定 / 解绑按钮。
 *
 * 为什么要有它：自动化车道的目标下拉只列「增益 / 声像 + 已绑定 + 已有曲线」，那么"把某个插件
 * 参数加进来"就需要一个能翻页 / 查找 / 排序 / 筛选的挑选器——就是这一页。
 *
 * 外壳与工具条都是共享件（`ModalShell` / `ListToolbar`），本组件不再复制那份样式；**插件筛选**
 * 那个 `Md3Select` 是本页特有的，所以放在工具条旁边，而不是塞进 `ListToolbar`。
 *
 * 取数：`trackId` → 该轨道的乐器 + 每个效果器调 `ensureParams`（外部插件异步）→
 * `automationTargetsFor`（**全量**口径）拿目标 → 搜索/筛选/排序 → 切片翻页。
 * 绑定状态以 `useParamBindingsStore().isBound()`（**生效**状态）为准，与车道下拉一致。
 */
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { NoSymbolIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import ModalShell from "@/components/common/ModalShell.vue";
import ListToolbar from "@/components/common/ListToolbar.vue";
import { useI18n } from "@/i18n";
import { useProjectStore } from "@/stores/project";
import { usePluginsStore } from "@/stores/plugins";
import { useParamBindingsStore } from "@/stores/paramBindings";
import { useParamBrowserStore } from "@/stores/paramBrowser";
import {
  automationTargetsFor,
  defaultBoundKeys,
  laneHasPoints,
  type AutomationTarget,
} from "@/utils/automation";
import { canonicalPluginId, EFFECT_DEFAULTS, effectiveParams } from "@/data/plugins";
import type { Track } from "@/api/types";

const i18n = useI18n();
const project = useProjectStore();
const plugins = usePluginsStore();
const bindings = useParamBindingsStore();
const browser = useParamBrowserStore();

/** 绑定状态筛选。 */
type StatusChip = "all" | "bound" | "unbound" | "used";
/** 排序方式。 */
type SortKey = "plugin" | "name" | "boundFirst";
/** 插件筛选：`all` / 乐器 / 效果器 i（槽位下标）。 */
type PluginFilter = "all" | "inst" | `fx${number}`;

const PAGE_SIZES = [16, 32, 64, 0] as const;

/** 作用域轨道。 */
const trackId = computed(() => browser.trackId);
const track = computed<Track | null>(
  () => project.project?.tracks.find((t) => t.id === trackId.value) ?? null,
);

const query = ref("");
const status = ref<StatusChip>("all");
/** 「仅可自动化」默认**开**：先看到能真正自动化的参数，想看全部再去点掉。 */
const onlyAutomatable = ref(true);
const sort = ref<SortKey>("plugin");
const pluginFilter = ref<PluginFilter>(initialPluginFilter());
const page = ref(1);
/** 每页行数；`0` = 全部（不分页）。 */
const pageSize = ref(16);
const loading = ref(true);
const toolbar = ref<InstanceType<typeof ListToolbar> | null>(null);

/**
 * 打开时插件筛选预选哪一个：`pluginId` 命中乐器 → 乐器；命中某个效果器 → 那个槽位；否则「全部」。
 *
 * 按**当前**槽位算，不缓存（重排效果链后槽位就变了，缓存就会指错——设计 §7 的取舍）。
 */
function initialPluginFilter(): PluginFilter {
  const t = track.value;
  const pid = browser.pluginId;
  if (!t || !pid) return "all";
  const want = canonicalPluginId(pid);
  if (t.instrument && canonicalPluginId(t.instrument.plugin_id) === want) return "inst";
  const at = t.effects.findIndex((fx) => canonicalPluginId(fx.plugin_id) === want);
  return at >= 0 ? `fx${at}` : "all";
}

/** 某插件是否声明该参数可自动化（缺省视为可自动化）。 */
function isAutomatable(t: AutomationTarget): boolean {
  return t.automatable !== false;
}

/** 增益 / 声像 / 常用 MIDI CC 由 `curateTargets` 钉住：永远在车道下拉里，**不可能解绑**。 */
function isPinned(key: string): boolean {
  return key === "gain_db" || key === "pan" || key.startsWith("cc:");
}

/** 生效的绑定状态（与车道下拉同一个判定）。 */
function isBound(t: AutomationTarget): boolean {
  const id = trackId.value;
  return id !== null && bindings.isBound(id, t.key);
}

/**
 * 行上显示的绑定态：增益 / 声像被 `curateTargets` **钉住**（永远在车道下拉里、不可能解绑），
 * 而 `bindings.isBound()` 对它们返回 `false`——所以这两行必须**强制当成已绑定**，否则按钮会显示
 * 「绑定」、点了却什么也不发生。其余行走生效状态（`isBound`）。
 */
function rowBound(t: AutomationTarget): boolean {
  return isPinned(t.key) || isBound(t);
}

/** 目标键 → 原始参数 id（去掉 `inst:` / `fx<i>:` 前缀）。 */
function paramId(key: string): string {
  return key.replace(/^(inst:|fx\d+:)/, "");
}

/** 目标属于哪个插件筛选组（增益 / 声像归「轨道」，只在「全部」下露出）。 */
function groupOf(key: string): PluginFilter {
  if (key.startsWith("inst:")) return "inst";
  const fx = /^fx(\d+):/.exec(key);
  return fx ? (`fx${fx[1]}` as PluginFilter) : "all";
}

/** 单位与范围（行里"这个参数是什么量"的那一格）。 */
function rangeText(t: AutomationTarget): string {
  return `${t.min}–${t.max}${t.unit ? ` ${t.unit}` : ""}`;
}

/** 插件参数表里该参数的默认值（实例里没存过时的兜底）。 */
function specDefault(pluginId: string, id: string): number | null {
  return plugins.paramsOf(pluginId).find((p) => p.id === id)?.default ?? null;
}

/**
 * 该目标在轨道上的**当前值**：读实例参数；内置合成器/效果器的实例只存改过的值，所以先摊平
 * 出厂补丁再取，最后才落到参数表默认值。取不到给 `null`（渲染成 `—`）。
 */
function currentValue(key: string): number | null {
  const t = track.value;
  if (!t) return null;
  if (key === "gain_db") return t.mixer.gain_db;
  if (key === "pan") return t.mixer.pan;

  const inst = /^inst:(.+)$/.exec(key);
  if (inst) {
    const id = inst[1]!;
    const plug = t.instrument;
    if (!plug) return null;
    const merged = effectiveParams(plug.plugin_id, plug.params);
    return merged[id] ?? specDefault(plug.plugin_id, id);
  }

  const fx = /^fx(\d+):(.+)$/.exec(key);
  if (fx) {
    const plug = t.effects[Number(fx[1])];
    if (!plug) return null;
    const id = fx[2]!;
    const base = EFFECT_DEFAULTS[canonicalPluginId(plug.plugin_id)] ?? {};
    return plug.params[id] ?? base[id] ?? specDefault(plug.plugin_id, id);
  }
  return null;
}

function valueText(t: AutomationTarget): string {
  const v = currentValue(t.key);
  if (v === null) return "—";
  return `${v.toFixed(t.digits)}${t.unit ? ` ${t.unit}` : ""}`;
}

/** 插件的全部目标（全量口径：精选表之外的、不可自动化的都在）。 */
const allTargets = computed<AutomationTarget[]>(() =>
  automationTargetsFor(track.value, plugins.paramsOf, i18n.t),
);

/** 工具条的两组 chip：绑定状态 + 仅可自动化（互相独立、叠加生效）。 */
const chipGroups = computed(() => [
  {
    id: "status",
    label: i18n.t("browser.filter.status"),
    chips: [
      { id: "all", label: i18n.t("browser.filter.all") },
      { id: "bound", label: i18n.t("browser.filter.bound") },
      { id: "unbound", label: i18n.t("browser.filter.unbound") },
      { id: "used", label: i18n.t("browser.filter.used") },
    ],
    active: status.value,
  },
  {
    id: "automatable",
    label: i18n.t("browser.filter.automatableGroup"),
    chips: [{ id: "on", label: i18n.t("browser.filter.automatable") }],
    // 关掉时这组没有选中项——列表里才会出现"插件声明不可自动化"的行。
    active: onlyAutomatable.value ? "on" : "",
  },
]);

const sorts = computed(() => [
  { id: "plugin", label: i18n.t("browser.sortPluginOrder") },
  { id: "name", label: i18n.t("browser.sortParamName") },
  { id: "boundFirst", label: i18n.t("browser.sortBoundFirst") },
]);

/** 插件筛选的下拉项：全部 / 乐器 / 效果器 1..n。 */
const pluginOptions = computed(() => {
  const t = track.value;
  if (!t) return [];
  const out = [{ id: "all", label: i18n.t("common.all") }];
  if (t.instrument) out.push({ id: "inst", label: i18n.t("auto.group.instrument") });
  t.effects.forEach((_, i) => out.push({ id: `fx${i}`, label: i18n.t("auto.group.effect", { n: i + 1 }) }));
  return out;
});

const pageSizeOptions = computed(() =>
  PAGE_SIZES.map((n) => ({ id: String(n), label: n === 0 ? i18n.t("common.all") : String(n) })),
);

/** 搜索命中面：显示名 / 目标键 / 原始 id / 单位。 */
function haystack(t: AutomationTarget): string {
  return [t.label, t.key, paramId(t.key), t.unit ?? ""].join(" ").toLowerCase();
}

/** 搜索 + 筛选 + 排序之后的行（翻页之前的全量）。 */
const filtered = computed<AutomationTarget[]>(() => {
  const q = query.value.trim().toLowerCase();
  let list = allTargets.value.filter((t) => {
    if (onlyAutomatable.value && !isAutomatable(t)) return false;
    if (pluginFilter.value !== "all" && groupOf(t.key) !== pluginFilter.value) return false;
    if (status.value === "bound" && !rowBound(t)) return false;
    if (status.value === "unbound" && rowBound(t)) return false;
    if (status.value === "used" && !laneHasPoints(track.value, t.key)) return false;
    return !q || haystack(t).includes(q);
  });
  if (sort.value === "name") {
    list = [...list].sort((a, b) => a.label.localeCompare(b.label, "zh-Hans-CN"));
  } else if (sort.value === "boundFirst") {
    // `sort` 是稳定的：同状态内保持插件顺序。
    list = [...list].sort((a, b) => Number(rowBound(b)) - Number(rowBound(a)));
  }
  return list;
});

const shownCount = computed(() => filtered.value.length);
const pageCount = computed(() =>
  pageSize.value === 0 ? 1 : Math.max(1, Math.ceil(shownCount.value / pageSize.value)),
);

/** 当前页的行。 */
const pageItems = computed(() => {
  if (pageSize.value === 0) return filtered.value;
  const start = (page.value - 1) * pageSize.value;
  return filtered.value.slice(start, start + pageSize.value);
});

/** 兜底：绑定/解绑等操作让结果变少时，别停在一个越界的页上。 */
watch(pageCount, (n) => {
  if (page.value > n) page.value = n;
});

// ---- 交互（任一变化都回到第 1 页） ----

function onQuery(value: string): void {
  query.value = value;
  page.value = 1;
}

function onSort(value: string): void {
  sort.value = value as SortKey;
  page.value = 1;
}

function onChip(payload: { groupId: string; chipId: string }): void {
  if (payload.groupId === "status") status.value = payload.chipId as StatusChip;
  else if (payload.groupId === "automatable") onlyAutomatable.value = !onlyAutomatable.value;
  page.value = 1;
}

function onPluginFilter(value: string): void {
  pluginFilter.value = value as PluginFilter;
  page.value = 1;
}

function onPageSize(value: string): void {
  pageSize.value = Number(value);
  page.value = 1;
}

/** 绑定 / 解绑：没有显式记录时 `toggle` 会先把默认精选实体化再改，所以要把精选集传进去。 */
function toggleBind(t: AutomationTarget): void {
  const id = trackId.value;
  if (id === null || !isAutomatable(t) || isPinned(t.key)) return;
  bindings.toggle(id, t.key, defaultBoundKeys(track.value));
}

/** 打开时对该轨道的乐器与**每个**效果器确保参数表已加载（外部插件是异步的）。 */
async function ensureParams(): Promise<void> {
  const t = track.value;
  if (!t) {
    loading.value = false;
    return;
  }
  loading.value = true;
  try {
    const ids = [t.instrument?.plugin_id, ...t.effects.map((fx) => fx.plugin_id)];
    await Promise.all(ids.map((id) => plugins.ensureParams(id)));
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  // 打开就是来找参数的：焦点直接进搜索框（与插件浏览器同一约定）。
  void nextTick(() => toolbar.value?.focusSearch());
  void ensureParams();
});
</script>

<template>
  <ModalShell :open="true" :title="i18n.t('browser.titleParams')" @update:open="browser.close()">
    <template #toolbar>
      <ListToolbar
        ref="toolbar"
        :query="query"
        :placeholder="i18n.t('browser.searchParams')"
        :search-label="i18n.t('browser.searchLabelParams')"
        :clear-label="i18n.t('browser.clearSearch')"
        :chip-groups="chipGroups"
        :sort-label="i18n.t('browser.sort')"
        :sort-aria-label="i18n.t('browser.sortBy')"
        :sorts="sorts"
        :active-sort="sort"
        @update:query="onQuery"
        @update:active-chip="onChip"
        @update:active-sort="onSort"
      />

      <!-- 插件筛选是本页特有的，所以放在共享工具条**旁边**，而不是塞进去。 -->
      <div class="plugin-filter">
        <Md3Select
          :model-value="pluginFilter"
          :options="pluginOptions"
          :label="i18n.t('browser.pluginFilter')"
          @select="onPluginFilter"
        />
      </div>
    </template>

    <div class="body">
      <p v-if="loading" class="state">{{ i18n.t("browser.loading") }}</p>

      <ul v-else-if="pageItems.length > 0" class="list">
        <li v-for="t in pageItems" :key="t.key">
          <!-- 行是 `<div>`：行内还有一个绑定按钮，按钮绝不能嵌进按钮。 -->
          <div class="row" :class="{ muted: !isAutomatable(t) }">
            <span class="row-main">
              <span class="row-top">
                <span class="row-name">{{ t.label }}</span>
                <span class="badge">{{ t.group }}</span>
                <span class="badge mono">{{ rangeText(t) }}</span>
                <span v-if="!isAutomatable(t)" class="badge warn">
                  {{ i18n.t("browser.notAutomatable") }}
                </span>
                <span v-else-if="isPinned(t.key)" class="badge pin">
                  {{ i18n.t("browser.alwaysBound") }}
                </span>
              </span>
              <span class="row-note">{{ paramId(t.key) }} · {{ t.hint }}</span>
            </span>

            <span class="row-value" :title="i18n.t('browser.currentValue')">{{ valueText(t) }}</span>

            <!-- 增益 / 声像：钉在车道下拉里，所以永远显示「已绑定」但不可点。 -->
            <button
              class="bind"
              :class="{ on: rowBound(t) }"
              :disabled="!isAutomatable(t) || isPinned(t.key)"
              :aria-pressed="rowBound(t)"
              @click="toggleBind(t)"
            >
              {{ rowBound(t) ? i18n.t("browser.unbind") : i18n.t("browser.bind") }}
            </button>
          </div>
        </li>
      </ul>

      <div v-else class="empty">
        <NoSymbolIcon class="empty-icon" />
        <p class="empty-text">{{ i18n.t("browser.emptyParams") }}</p>
      </div>
    </div>

    <template #foot>
      <footer class="foot">
        <div class="pager">
          <button class="pager-btn" :disabled="page <= 1" @click="page -= 1">
            {{ i18n.t("browser.page.prev") }}
          </button>
          <span class="pager-pos">{{ i18n.t("browser.page.of", { page, pages: pageCount }) }}</span>
          <button class="pager-btn" :disabled="page >= pageCount" @click="page += 1">
            {{ i18n.t("browser.page.next") }}
          </button>
        </div>

        <div class="foot-right">
          <span class="page-size-label">{{ i18n.t("browser.pageSize") }}</span>
          <div class="page-size">
            <Md3Select
              :model-value="String(pageSize)"
              :options="pageSizeOptions"
              :label="i18n.t('browser.pageSize')"
              @select="onPageSize"
            />
          </div>
          <span class="count">
            {{ i18n.t("browser.countParams", { total: allTargets.length, shown: shownCount }) }}
          </span>
        </div>
      </footer>
    </template>
  </ModalShell>
</template>

<style scoped>
/* ---- 脚本特有：插件筛选那一行（与工具条同一个 22px 边距） ---- */
.plugin-filter {
  display: flex;
  align-items: center;
  padding: 0 22px 12px;
}
.plugin-filter :deep(.md3-select) {
  width: 220px;
}
/* ---- 列表 ---- */
.body {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 0 14px 8px;
}
.state {
  margin: 0;
  padding: 40px 16px;
  text-align: center;
  font-size: 12.5px;
  color: rgb(var(--md-on-surface-variant));
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
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 12px;
  border-radius: 14px;
  color: rgb(var(--md-on-surface));
  transition: background-color 0.14s ease;
}
.row:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.row.muted {
  color: rgb(var(--md-on-surface-muted));
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
  flex-wrap: wrap;
  gap: 6px;
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
.badge.mono {
  font-variant-numeric: tabular-nums;
}
.badge.warn {
  background-color: rgb(var(--md-error) / 0.14);
  color: rgb(var(--md-error));
}
.badge.pin {
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
.row-value {
  flex: 0 0 auto;
  min-width: 68px;
  text-align: right;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  color: rgb(var(--md-on-surface-variant));
}
.bind {
  flex: 0 0 auto;
  height: 28px;
  padding: 0 14px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.8);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.bind:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.bind.on {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.bind.on:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.85);
}
.bind:disabled {
  opacity: 0.45;
  cursor: default;
}
.bind:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
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
/* ---- 页脚：翻页 ---- */
.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 22px 14px;
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.35);
}
.pager {
  display: flex;
  align-items: center;
  gap: 8px;
}
.pager-btn {
  height: 28px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11.5px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.pager-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.pager-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.pager-pos {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.foot-right {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}
.page-size {
  width: 96px;
  flex: 0 0 auto;
}
.page-size-label {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.count {
  font-size: 11px;
  color: rgb(var(--md-on-surface-muted));
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
