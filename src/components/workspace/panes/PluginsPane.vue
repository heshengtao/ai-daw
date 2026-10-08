<script setup lang="ts">
/**
 * 插件面板内容：可插入的乐器与效果（与设置页的「插件管理」共用同一份清单）。
 *
 * 选中轨道后可以直接把插件加载到该轨道上——这是「右侧边栏拿插件」的那条路径。
 * 清单按分类分组；音色多起来之后要精挑细选，请走检查器里的「浏览全部音色…」。
 */
import { computed } from "vue";
import { CheckIcon, PlusIcon } from "@heroicons/vue/24/outline";
import { usePluginsStore } from "@/stores/plugins";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginInterface } from "@/composables/usePluginInterface";
import { useI18n } from "@/i18n";
import { canonicalPluginId, SYNTH_ID, type PluginInfo } from "@/data/plugins";

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const i18n = useI18n();
const pluginInterface = usePluginInterface();

const track = computed(() => selection.track);

/** 按分类分组（只保留有内容的分类）。 */
const groups = computed(() =>
  plugins.categories.map((category) => ({
    category,
    items: plugins.all.filter((p) => p.category === category),
  })).filter((g) => g.items.length > 0),
);

/** 该插件是否已挂到当前轨道上。 */
function isLoaded(pluginId: string): boolean {
  return track.value?.instrument?.plugin_id === pluginId;
}

async function load(pluginId: string, loadable: boolean): Promise<void> {
  const t = track.value;
  if (!t || !loadable) return;
  try {
    await store.setTrackInstrument(t.id, pluginId);
    layout.notify(i18n.t("plugins.loaded", { track: t.name }));
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  }
}

/**
 * 一键打开「这个东西的界面」（共用 `usePluginInterface`）：
 * 自带原生界面的开窗口，其余（内置合成器 / 无界面插件）切到侧边栏的插件界面标签。
 */
async function openInterface(it: PluginInfo): Promise<void> {
  const t = track.value;
  if (!t) {
    layout.notify(i18n.t("plugins.selectTrackFirst"));
    return;
  }
  await pluginInterface.open(t.id, it.id, true);
}

/**
 * 「界面」按钮上写什么。
 *
 * 统一叫**插件界面**：不管打开的是内置合成器的面板、外部插件自带的原生界面，还是
 * 退回参数表，对用户来说都是「这台乐器的界面」。内置合成器与外部插件不再用两套叫法。
 */
function interfaceLabel(it: PluginInfo): string {
  return it.hasEditor || canonicalPluginId(it.id) === SYNTH_ID
    ? i18n.t("plugins.pluginEditor")
    : i18n.t("plugins.params");
}
</script>

<template>
  <div class="pane">
    <p v-if="track" class="target">
      加载到：<strong>{{ track.name }}</strong>
    </p>
    <p v-else class="target muted">先选中一条轨道，再加载插件。</p>

    <div v-for="g in groups" :key="g.category" class="group">
      <div class="group-title">{{ g.category }}</div>
      <ul class="list">
        <li v-for="it in g.items" :key="it.id" class="item">
          <div class="item-main">
            <span class="item-name">{{ it.name }}</span>
            <span class="item-note">{{ it.note }}</span>
          </div>

          <span v-if="it.builtin" class="item-kind builtin">内置</span>

          <button
            v-if="it.loadable"
            class="iface-btn"
            :title="i18n.t('plugins.openInterfaceTitle', { name: it.name })"
            :aria-label="i18n.t('plugins.openInterfaceTitle', { name: it.name })"
            :disabled="!track"
            @click="openInterface(it)"
          >
            {{ interfaceLabel(it) }}
          </button>

          <button
            v-if="it.loadable"
            class="load-btn"
            :class="{ on: isLoaded(it.id) }"
            :disabled="!track || isLoaded(it.id)"
            :aria-label="`加载 ${it.name}`"
            @click="load(it.id, it.loadable)"
          >
            <CheckIcon v-if="isLoaded(it.id)" class="w-[14px] h-[14px]" />
            <PlusIcon v-else class="w-[14px] h-[14px]" />
          </button>
          <span v-else class="soon">待接入</span>
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
.target {
  margin: 12px 12px 0;
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
