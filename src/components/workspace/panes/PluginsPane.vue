<script setup lang="ts">
/**
 * 插件面板内容：可插入的乐器与效果（与设置页的「插件管理」共用同一份清单）。
 *
 * 选中轨道后，点右侧的按钮就能把它装上去——这是「右侧边栏拿插件」的那条路径：
 * - **乐器**：互斥的「换成它」（一轨只有一件乐器）；
 * - **效果器**：**可多选**，点一下加入该轨的效果链、再点一下移出，与乐器互不干扰。
 *
 * 清单按分类分组；音色多起来之后要精挑细选，请走检查器里的「浏览全部音色…」。
 */
import { computed, ref } from "vue";
import { CheckIcon, PlusIcon } from "@heroicons/vue/24/outline";
import { usePluginsStore } from "@/stores/plugins";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginInterface } from "@/composables/usePluginInterface";
import { useI18n } from "@/i18n";
import { canonicalPluginId, type PluginInfo } from "@/data/plugins";

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const i18n = useI18n();
const pluginInterface = usePluginInterface();

const track = computed(() => selection.track);

/** 来源筛选：全部 / 内置 / 外部。 */
type SourceFilter = "all" | "builtin" | "external";

const SOURCES: { id: SourceFilter; labelKey: string }[] = [
  { id: "all", labelKey: "common.all" },
  { id: "builtin", labelKey: "common.builtin" },
  { id: "external", labelKey: "plugins.sourceExternal" },
];

const source = ref<SourceFilter>("all");

function matchesSource(p: PluginInfo): boolean {
  if (source.value === "builtin") return p.builtin;
  if (source.value === "external") return !p.builtin;
  return true;
}

/** 按分类分组（按来源筛选后，只保留有内容的分类）。 */
const groups = computed(() =>
  plugins.categories.map((category) => ({
    category,
    items: plugins.all.filter((p) => p.category === category && matchesSource(p)),
  })).filter((g) => g.items.length > 0),
);

/** 某条轨道上这个插件在不在效果链里（返回链内序号）。 */
function fxIndex(pluginId: string): number {
  return track.value?.effects.findIndex((f) => f.plugin_id === pluginId) ?? -1;
}

/**
 * 该插件是否已作用于当前轨道。
 *
 * **乐器**看「是不是当前乐器」（一轨只有一个，互斥）；**效果器**看「在不在效果链里」
 * （可多选、可重复，与乐器不互斥）。
 */
function isLoaded(it: PluginInfo): boolean {
  if (!track.value) return false;
  return it.kind === "effect"
    ? fxIndex(it.id) >= 0
    : // 归一后比较：默认分轨挂的是注册表预设 id（builtin.synth.lead 等），清单里只有 builtin.synth。
      canonicalPluginId(track.value.instrument?.plugin_id ?? "") === it.id;
}

/**
 * 点一下加入 / 再点一下取消。
 *
 * 效果器走**效果链**（多选、不占乐器位）；乐器仍然是互斥的「换成它」。
 */
async function toggle(it: PluginInfo): Promise<void> {
  const t = track.value;
  if (!t || !it.loadable) return;
  try {
    if (it.kind === "effect") {
      const index = fxIndex(it.id);
      if (index >= 0) {
        await store.removeEffect(t.id, index);
        layout.notify(i18n.t("plugins.fxRemoved", { name: it.name }));
      } else {
        await store.addEffect(t.id, it.id);
        layout.notify(i18n.t("plugins.fxAdded", { name: it.name }));
      }
      return;
    }
    await store.setTrackInstrument(t.id, it.id);
    layout.notify(i18n.t("plugins.loaded", { track: t.name }));
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  }
}

/**
 * 一键打开「这个东西的界面」（共用 `usePluginInterface`）：
 * 内置合成器与内置效果器开我们自己的窗口，外部插件开它的原生窗口，
 * 其余（没有原生界面的）切到侧边栏的插件界面标签。
 */
async function openInterface(it: PluginInfo): Promise<void> {
  const t = track.value;
  if (!t) {
    layout.notify(i18n.t("plugins.selectTrackFirst"));
    return;
  }
  if (it.kind !== "effect") {
    await pluginInterface.open(t.id, it.id, true);
    return;
  }
  // 效果器不在链上就先加进去——「打开界面」本身就意味着要用它。
  if (fxIndex(it.id) < 0) await store.addEffect(t.id, it.id);
  await pluginInterface.openEffect(t.id, it.id);
}

/** 「界面」按钮的提示文本。 */
function interfaceTitle(it: PluginInfo): string {
  return i18n.t("plugins.openInterfaceTitle", { name: it.name });
}

/**
 * 「界面」按钮上写什么。
 *
 * 统一叫**插件界面**：不管打开的是内置合成器/效果器的面板、外部插件自带的原生界面，
 * 还是退回参数表，对用户来说都是「这个东西的界面」。
 */
function interfaceLabel(it: PluginInfo): string {
  return it.hasEditor || it.builtin ? i18n.t("plugins.pluginEditor") : i18n.t("plugins.params");
}

/** 加载按钮的提示：效果器是「加入/移出效果链」，乐器是「装到这条轨道」。 */
function toggleLabel(it: PluginInfo): string {
  if (it.kind !== "effect") return i18n.t("plugins.loadNamed", { name: it.name });
  return isLoaded(it)
    ? i18n.t("plugins.fxRemove", { name: it.name })
    : i18n.t("plugins.fxAdd", { name: it.name });
}
</script>

<template>
  <div class="pane">
    <div class="target-row">
      <p v-if="track" class="target">
        {{ i18n.t("plugins.loadTo") }}<strong>{{ track.name }}</strong>
      </p>
      <p v-else class="target muted">{{ i18n.t("plugins.selectTrackFirst") }}</p>

      <!-- 来源快速筛选：全部 / 内置 / 外部 -->
      <div class="src-chips" role="group" :aria-label="i18n.t('plugins.sourceFilter')">
        <button
          v-for="s in SOURCES"
          :key="s.id"
          class="src-chip"
          :class="{ on: source === s.id }"
          :aria-pressed="source === s.id"
          @click="source = s.id"
        >
          {{ i18n.t(s.labelKey) }}
        </button>
      </div>
    </div>

    <div v-for="g in groups" :key="g.category" class="group">
      <div class="group-title">{{ g.category }}</div>
      <ul class="list">
        <li v-for="it in g.items" :key="it.id" class="item">
          <div class="item-main">
            <span class="item-name">{{ it.name }}</span>
            <span class="item-note">{{ it.note }}</span>
          </div>

          <span v-if="it.builtin" class="item-kind builtin">{{ i18n.t("common.builtin") }}</span>

          <button
            v-if="it.loadable"
            class="iface-btn"
            :title="interfaceTitle(it)"
            :aria-label="interfaceTitle(it)"
            :disabled="!track"
            @click="openInterface(it)"
          >
            {{ interfaceLabel(it) }}
          </button>

          <button
            v-if="it.loadable"
            class="load-btn"
            :class="{ on: isLoaded(it) }"
            :disabled="!track || (it.kind === 'instrument' && isLoaded(it))"
            :title="toggleLabel(it)"
            :aria-label="toggleLabel(it)"
            :aria-pressed="it.kind === 'effect' ? isLoaded(it) : undefined"
            @click="toggle(it)"
          >
            <CheckIcon v-if="isLoaded(it)" class="w-[14px] h-[14px]" />
            <PlusIcon v-else class="w-[14px] h-[14px]" />
          </button>
          <span v-else class="soon">{{ i18n.t("plugins.pending") }}</span>
        </li>
      </ul>
    </div>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding-bottom: 12px;
}
/* 「加载到：X」一行 + 右侧的来源筛选 chips（窄面板里允许换行，不挤变形） */
.target-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin: 12px 12px 0;
}
.target {
  margin: 0;
  font-size: 11.5px;
  color: rgb(var(--md-on-surface-variant));
}
.target strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
}
.target.muted {
  color: rgb(var(--md-on-surface-muted));
}
.src-chips {
  display: flex;
  gap: 4px;
  flex: 0 0 auto;
}
.src-chip {
  height: 24px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  white-space: nowrap;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.src-chip:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.src-chip.on {
  border-color: transparent;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.src-chip:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.group {
  margin-top: 12px;
}
.group-title {
  padding: 0 12px 6px;
  font-size: 10px;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
}
.list {
  list-style: none;
  margin: 0;
  padding: 0 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px;
  border-radius: 14px;
  background-color: var(--app-inset);
}
.item-main {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1 1 auto;
}
.item-name {
  font-size: 13px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.item-note {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.item-kind {
  flex: 0 0 auto;
  font-size: 10px;
  padding: 3px 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.item-kind.builtin {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
/* 打开界面：自带原生界面的开窗口，其余切到侧边栏参数面板 */
.iface-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease;
}
.iface-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.08);
}
.iface-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.iface-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.load-btn {
  width: 30px;
  height: 30px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.load-btn:disabled {
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  cursor: default;
}
.load-btn.on:disabled {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.load-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.soon {
  flex: 0 0 auto;
  font-size: 10px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
