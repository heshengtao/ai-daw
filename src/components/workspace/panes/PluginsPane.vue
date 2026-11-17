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
import { CheckIcon, PlusIcon, TrashIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import { pickSoundfontsToImport } from "@/api/dialogs";
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

/** 正在导入 SoundFont（复制 + 解析要几秒，按钮期间显示「导入中」）。 */
const importing = ref(false);
/** 待确认删除的库（null = 没开确认框）。 */
const pendingRemove = ref<{ id: string; name: string } | null>(null);
const removeOpen = computed(() => pendingRemove.value !== null);

/** 导入 SF2/SF3：可多选，逐个导入；失败的第一条原因直接提示。 */
async function importSoundfonts(): Promise<void> {
  try {
    const paths = await pickSoundfontsToImport();
    if (paths.length === 0) return;
    importing.value = true;
    const { imported, failed } = await plugins.importFonts(paths);
    if (imported.length > 0) {
      layout.notify(i18n.t("plugins.soundfontImported", { count: imported.length }));
    }
    if (failed.length > 0) {
      layout.notify(i18n.t("error.load", { error: failed[0] }));
    }
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  } finally {
    importing.value = false;
  }
}

/** 删除库要先确认：文件是**真的从磁盘删掉**的。 */
function askRemoveFont(entry: { id: string; name: string }): void {
  pendingRemove.value = { id: entry.id, name: entry.name };
}

async function confirmRemoveFont(): Promise<void> {
  const target = pendingRemove.value;
  pendingRemove.value = null;
  if (!target) return;
  try {
    await plugins.removeFont(target.id);
    layout.notify(i18n.t("plugins.soundfontRemoved", { name: target.name }));
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  }
}

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

/** 被跳过插件的显示名：取包名（去掉 `.vst3` / `.clap` 后缀）。 */
function skippedName(path: string): string {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return name.replace(/\.(vst3|clap|component)$/i, "");
}

/** 重新扫描（坏插件只会被再次跳过，不会再连累宿主）。 */
async function retryScan(): Promise<void> {
  try {
    await plugins.rescan();
  } catch (err) {
    layout.notify(i18n.t("error.load", { error: String(err) }));
  }
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

    <!-- 用户 SoundFont：一个库展开成多条音色，下面的清单按分类平铺 -->
    <div class="sf-box">
      <div class="sf-head">
        <span class="sf-title">{{ i18n.t("plugins.soundfontTitle") }}</span>
        <button class="sf-import" :disabled="importing" @click="importSoundfonts">
          <PlusIcon class="w-[14px] h-[14px]" />
          {{ importing ? i18n.t("plugins.soundfontImporting") : i18n.t("plugins.soundfontImport") }}
        </button>
      </div>
      <p v-if="!plugins.soundfonts.length" class="sf-empty">
        {{ i18n.t("plugins.soundfontEmpty") }}
      </p>
      <ul v-else class="sf-list">
        <li v-for="f in plugins.soundfonts" :key="f.id" class="sf-item">
          <div class="sf-main">
            <span class="sf-name" :title="f.file">{{ f.name }}</span>
            <span class="sf-note">
              {{ f.format.toUpperCase() }} ·
              {{ i18n.t("plugins.soundfontPresets", { count: f.presets.length }) }}
            </span>
          </div>
          <button
            class="sf-remove"
            :title="i18n.t('plugins.soundfontRemove', { name: f.name })"
            :aria-label="i18n.t('plugins.soundfontRemove', { name: f.name })"
            @click="askRemoveFont(f)"
          >
            <TrashIcon class="w-[14px] h-[14px]" />
          </button>
        </li>
      </ul>
    </div>

    <!-- 扫描器整体没跑起来（探针缺失等）：说清楚原因，而不是静默显示「没有插件」 -->
    <div v-if="plugins.scanError" class="scan-warn" role="alert">
      <div class="scan-warn-main">
        <span class="scan-warn-title">{{ i18n.t("plugins.scanFailed") }}</span>
        <span class="scan-warn-hint">{{ plugins.scanError }}</span>
      </div>
      <button class="scan-warn-retry" :disabled="plugins.loading" @click="retryScan">
        {{ i18n.t("plugins.scanSkippedRetry") }}
      </button>
    </div>

    <!-- 扫描时崩溃 / 超时 / 读不出来的插件：列出来，给一个重扫入口 -->
    <div v-if="plugins.skipped.length" class="scan-warn" role="status">
      <div class="scan-warn-main">
        <span class="scan-warn-title">
          {{ i18n.t("plugins.scanSkipped", { count: plugins.skipped.length }) }}
        </span>
        <span class="scan-warn-hint">{{ i18n.t("plugins.scanSkippedHint") }}</span>
        <ul class="scan-warn-list">
          <li v-for="s in plugins.skipped" :key="s.path" :title="s.reason">
            {{ skippedName(s.path) }}
          </li>
        </ul>
      </div>
      <button class="scan-warn-retry" :disabled="plugins.loading" @click="retryScan">
        {{ i18n.t("plugins.scanSkippedRetry") }}
      </button>
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

    <ConfirmDialog
      :open="removeOpen"
      :title="i18n.t('plugins.soundfontRemoveTitle')"
      :message="i18n.t('plugins.soundfontRemoveMessage', { name: pendingRemove?.name ?? '' })"
      :confirm-label="i18n.t('common.delete')"
      danger
      @confirm="confirmRemoveFont"
      @cancel="pendingRemove = null"
      @update:open="(open: boolean) => { if (!open) pendingRemove = null }"
    />
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
.scan-warn {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  margin: 12px 12px 0;
  padding: 10px 12px;
  border-radius: 12px;
  background-color: rgb(var(--md-error-container));
  color: rgb(var(--md-on-error-container));
}
.scan-warn-main {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1 1 auto;
}
.scan-warn-title {
  font-size: 12px;
  font-weight: 600;
}
.scan-warn-hint {
  font-size: 11px;
  opacity: 0.8;
}
.scan-warn-list {
  list-style: none;
  margin: 2px 0 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  font-size: 11px;
}
.scan-warn-list li {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.scan-warn-retry {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  border: 1px solid currentColor;
  border-radius: 999px;
  background: transparent;
  color: inherit;
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}
.scan-warn-retry:disabled {
  opacity: 0.5;
  cursor: default;
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

/* ---- 用户 SoundFont：导入入口 + 已导入的库 ---- */
.sf-box {
  margin: 10px 12px 0;
  padding: 8px 10px 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 12px;
  background-color: rgb(var(--md-surface-container));
}
.sf-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.sf-title {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.4px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
}
.sf-import {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.9);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}
.sf-import:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.08);
}
.sf-import:disabled {
  opacity: 0.5;
  cursor: default;
}
.sf-empty {
  margin: 8px 0 0;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
.sf-list {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.sf-item {
  display: flex;
  align-items: center;
  gap: 8px;
}
.sf-main {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
  flex: 1 1 auto;
}
.sf-name {
  font-size: 12px;
  color: rgb(var(--md-on-surface));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sf-note {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}
.sf-remove {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.sf-remove:hover {
  background-color: rgb(var(--md-error) / 0.12);
  color: rgb(var(--md-error));
}
</style>
