<script setup lang="ts">
/**
 * 检查器面板：编辑**当前选中轨道**。
 *
 * 四组内容：
 * - 轨道名（可直接改，Enter/失焦提交）；
 * - 乐器（挂载 / 卸载内置合成器）；
 * - 乐器参数（来自 `data/plugins.ts` 的参数表，拖动时实时监听，松手提交）；
 * - 混音参数（与混音台同一份状态，改哪边都同步）。
 *
 * 未选中轨道时退回工程概览。
 */
import { computed, reactive, ref, watch } from "vue";
import { ArrowDownIcon, ArrowUpIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import Md3Slider from "@/components/common/Md3Slider.vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import PluginBrowser from "@/components/common/PluginBrowser.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { useParamBrowserStore } from "@/stores/paramBrowser";
import { usePluginInterface } from "@/composables/usePluginInterface";
import { canonicalPluginId } from "@/data/plugins";
import type { Send } from "@/api/types";
import { useI18n } from "@/i18n";

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const i18n = useI18n();
const pluginInterface = usePluginInterface();
/** 参数页（模态）的打开状态：挂载在 `App.vue` 根部，检查器只负责「打开哪条轨道、预选哪个插件」。 */
const paramBrowser = useParamBrowserStore();

const track = computed(() => selection.track);

/** 工程概览（未选中轨道时显示）。 */
const overview = computed(() => {
  const p = store.project;
  if (!p) return [];
  const instruments = p.tracks.filter((t) => t.instrument).length;
  const clips = p.tracks.reduce((n, t) => n + t.clips.length, 0);
  const notes = p.tracks.reduce((n, t) => n + t.clips.reduce((m, c) => m + c.notes.length, 0), 0);
  return [
    { label: i18n.t("inspector.overview.project"), value: p.name },
    { label: i18n.t("inspector.overview.tracks"), value: String(p.tracks.length) },
    { label: i18n.t("inspector.overview.instruments"), value: String(instruments) },
    { label: i18n.t("inspector.overview.clips"), value: String(clips) },
    { label: i18n.t("inspector.overview.notes"), value: String(notes) },
    { label: i18n.t("inspector.overview.master"), value: `${p.master.gain_db.toFixed(1)} dB` },
  ];
});

// ---- 轨道名 ----

const nameDraft = ref("");

watch(
  () => track.value?.name,
  (name) => {
    if (name != null) nameDraft.value = name;
  },
  { immediate: true },
);

async function commitName(): Promise<void> {
  const t = track.value;
  if (!t) return;
  const next = nameDraft.value.trim();
  if (!next || next === t.name) {
    nameDraft.value = t.name;
    return;
  }
  try {
    await store.renameTrack(t.id, next);
  } catch (err) {
    layout.notify(i18n.t("error.rename", { error: String(err) }));
    nameDraft.value = t.name;
  }
}

// ---- 乐器 ----

async function setInstrument(pluginId: string): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setTrackInstrument(t.id, pluginId);
    layout.notify(
      pluginId
        ? i18n.t("inspector.loaded", { name: plugins.name(pluginId) })
        : i18n.t("inspector.unloaded"),
    );
  } catch (err) {
    layout.notify(i18n.t("error.loadInstrument", { error: String(err) }));
  }
}

/**
 * 音色下拉的候选项：第一项是「无乐器」，其余按分类分组（顺序即内置 → 外部）。
 *
 * 下拉是**按 id 精确匹配**显示当前值的，所以这里必须用**归一后的 id**：默认分轨挂的是引擎
 * 注册表里的预设 id（`builtin.synth.lead` 等），而清单里只有 `builtin.synth`——不归一就匹配不上，
 * 触发器会退回占位符「—」，看起来像「这条轨道没有乐器」。
 *
 * 同理，当前乐器若根本不在清单里（插件被卸载 / 这机器扫不到），也把它补进去显示名字。
 */
const timbreOptions = computed<SelectOption[]>(() => {
  const current = canonicalPluginId(track.value?.instrument?.plugin_id ?? "");
  const list: SelectOption[] = plugins.instruments.map((p) => ({
    id: p.id,
    label: p.name,
    hint: p.note,
    group: p.category,
  }));
  if (current && !list.some((o) => o.id === current)) {
    list.unshift({ id: current, label: plugins.name(current), hint: "", group: "" });
  }
  return [{ id: "", label: i18n.t("instrument.none"), group: "" }, ...list];
});

/** 已归一化的当前乐器 id（下拉的选中值）。 */
const currentTimbre = computed(() => canonicalPluginId(track.value?.instrument?.plugin_id ?? ""));

/** 「浏览全部音色…」模态框的开关。 */
const browserOpen = ref(false);

/** 下拉或浏览器里选中一个音色：空串表示卸载。 */
function onPickTimbre(id: string): void {
  // 选中的还是当前这件乐器（只是 id 归一的差别）就别写回：那会把工程里空着的 params 固化下来，
  // 默认分轨挂着的预设音色会被重置成「默认正弦」。
  if (id === currentTimbre.value) return;
  void setInstrument(id);
}

// ---- 参数 ----

// 乐器参数不在这里列——检查器只给一个「打开界面」入口，编辑都去插件的独立界面
// （内置合成器有完整的合成器面板，外部插件开它自己的原生窗口 / 参数面板）。

// ---- 效果器链 ----
//
// **参数不在这里展开**：链上效果一多，检查器会被几十个滑块塞满。这里只做「链管理」
// （增删、换顺序）+ 一个「打开界面」入口，编辑去效果器自己的界面
// （内置效果器有独立面板窗口；外部插件切到侧边栏的「插件界面」标签）。

/** 从下拉里选中一个效果器 → 追加到链尾。 */
async function addFx(pluginId: string): Promise<void> {
  const t = track.value;
  if (!t || !pluginId) return;
  try {
    await store.addEffect(t.id, pluginId);
    layout.notify(i18n.t("inspector.fxAdded", { name: plugins.name(pluginId) }));
  } catch (err) {
    layout.notify(i18n.t("error.addEffect", { error: String(err) }));
  }
}

/**
 * 浏览器里点一条效果器：**点一下加入、再点一下移出**（与「插件」面板一致）。
 *
 * 效果链允许同一条重复，所以这里移出的是**最后一条**同名效果器。
 */
async function toggleFx(pluginId: string): Promise<void> {
  const t = track.value;
  if (!t) return;
  // 从后往前找（`findLastIndex` 在当前的 TS 目标库里没有）。
  let index = -1;
  for (let i = t.effects.length - 1; i >= 0; i -= 1) {
    if (t.effects[i]?.plugin_id === pluginId) {
      index = i;
      break;
    }
  }
  if (index >= 0) await removeFx(index);
  else await addFx(pluginId);
}

/** 「浏览全部效果器…」模态框的开关。 */
const fxBrowserOpen = ref(false);

/** 效果器下拉的候选项。 */
const fxOptions = computed(() =>
  plugins.effects.map((e) => ({ id: e.id, label: e.name, hint: e.format?.toUpperCase() })),
);

async function removeFx(index: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.removeEffect(t.id, index);
  } catch (err) {
    layout.notify(i18n.t("error.removeEffect", { error: String(err) }));
  }
}

async function moveFx(from: number, to: number): Promise<void> {
  const t = track.value;
  if (!t || to < 0 || to >= t.effects.length) return;
  try {
    await store.moveEffect(t.id, from, to);
  } catch (err) {
    layout.notify(i18n.t("error.moveEffect", { error: String(err) }));
  }
}

// ---- 混音 ----

const MIN_DB = -60;
const MAX_DB = 12;

/** 拖动中的草稿（滑块是受控组件，没有草稿把手不会跟手）。 */
const gainDraft = ref<number | null>(null);
const panDraft = ref<number | null>(null);

const shownGain = computed(() => gainDraft.value ?? track.value?.mixer.gain_db ?? 0);
const shownPan = computed(() => panDraft.value ?? track.value?.mixer.pan ?? 0);

async function commitGain(value: number): Promise<void> {
  const t = track.value;
  gainDraft.value = null;
  if (!t) return;
  try {
    await store.setGain(t.id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setGain", { error: String(err) }));
  }
}

async function commitPan(value: number): Promise<void> {
  const t = track.value;
  panDraft.value = null;
  if (!t) return;
  try {
    await store.setPan(t.id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setPan", { error: String(err) }));
  }
}

// ---- 发送（aux send → 总线） ----

/** 每路发送的增益草稿（拖动中显示，松手提交）。 */
const sendDrafts = reactive<Record<number, number>>({});

/** 本轨的发送列表。 */
const sends = computed<Send[]>(() => track.value?.sends ?? []);

/** 可选的发送目标：除自己以外的所有轨道（含总线）。 */
const sendTargets = computed<SelectOption[]>(() =>
  (store.project?.tracks ?? [])
    .filter((t) => t.id !== track.value?.id)
    .map((t) => ({ id: String(t.id), label: t.name })),
);

/** 发送目标的显示名。 */
function targetName(id: number): string {
  return (store.project?.tracks ?? []).find((t) => t.id === id)?.name ?? `#${id}`;
}

/** 拖动中的发送增益（否则滑块把手不跟手）。 */
function shownSendGain(send: Send): number {
  return sendDrafts[send.target] ?? send.gain_db;
}

/** 新增一路发送到 `targetId`（已在列表里则忽略）。 */
async function addSend(targetId: string): Promise<void> {
  const t = track.value;
  if (!t) return;
  const target = Number(targetId);
  if (!Number.isFinite(target) || sends.value.some((s) => s.target === target)) return;
  try {
    await store.setSends(t.id, [...sends.value, { target, gain_db: 0 }]);
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 提交一路发送的增益（一次编辑 = 一条撤销记录）。 */
async function commitSendGain(target: number, value: number): Promise<void> {
  const t = track.value;
  delete sendDrafts[target];
  if (!t) return;
  try {
    await store.setSends(
      t.id,
      sends.value.map((s) => (s.target === target ? { ...s, gain_db: value } : s)),
    );
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 移除一路发送。 */
async function removeSend(target: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setSends(t.id, sends.value.filter((s) => s.target !== target));
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

async function toggleMute(): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setMute(t.id, !t.mixer.mute);
}

async function toggleSolo(): Promise<void> {
  const t = track.value;
  if (!t) return;
  await store.setSolo(t.id, !t.mixer.solo);
}
</script>

<template>
  <div class="pane">
    <template v-if="track">
      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.track") }}</div>
        <div class="card">
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.name") }}</span>
            <input
              v-model="nameDraft"
              class="name-input md3-field"
              type="text"
              spellcheck="false"
              :aria-label="i18n.t('inspector.trackName')"
              @keydown.enter.prevent="commitName"
              @blur="commitName"
            />
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.notes") }}</span>
            <span class="row-value">
              {{ track.clips.reduce((n, c) => n + c.notes.length, 0) }} · {{ track.clips.length }}
            </span>
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.instrument") }}</div>
        <div class="card">
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.timbre") }}</span>
            <Md3Select
              :model-value="currentTimbre"
              :options="timbreOptions"
              :label="i18n.t('inspector.timbre')"
              :action="{ label: i18n.t('instrument.browse') }"
              @select="onPickTimbre"
              @action="browserOpen = true"
            />
          </div>

          <!-- 参数不在这里展开：留一个入口，编辑去插件的独立界面。 -->
          <div v-if="track.instrument" class="row">
            <span class="row-value plugin-name">{{ plugins.name(track.instrument.plugin_id) }}</span>
            <button
              class="open-editor"
              :aria-label="i18n.t('inspector.paramsFor', { name: plugins.name(track.instrument.plugin_id) })"
              @click="paramBrowser.open({ trackId: track!.id, pluginId: track!.instrument!.plugin_id })"
            >
              {{ i18n.t("inspector.params") }}
            </button>
            <button
              class="open-editor"
              :aria-label="i18n.t('inspector.openInterface', { name: plugins.name(track.instrument.plugin_id) })"
              @click="pluginInterface.open(track!.id, track!.instrument!.plugin_id)"
            >
              {{ i18n.t("panel.openEditor") }}
            </button>
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.fxChain") }}</div>
        <div class="card">
          <p v-if="track.effects.length === 0" class="hint">{{ i18n.t("inspector.fxEmpty") }}</p>

          <div v-for="(fx, i) in track.effects" :key="i" class="fx">
            <div class="row">
              <span class="row-label">{{ i + 1 }}</span>
              <span class="fx-name">{{ plugins.name(fx.plugin_id) }}</span>
              <button
                class="open-editor small"
                :aria-label="i18n.t('inspector.paramsFor', { name: plugins.name(fx.plugin_id) })"
                @click="paramBrowser.open({ trackId: track!.id, pluginId: fx.plugin_id })"
              >
                {{ i18n.t("inspector.params") }}
              </button>
              <button
                class="open-editor small"
                :aria-label="i18n.t('inspector.openInterface', { name: plugins.name(fx.plugin_id) })"
                @click="pluginInterface.openEffect(track!.id, fx.plugin_id)"
              >
                {{ i18n.t("panel.openEditor") }}
              </button>
              <div class="strip-buttons">
                <button class="mini" :disabled="i === 0" :aria-label="i18n.t('inspector.fxUp')" @click="moveFx(i, i - 1)">
                  <ArrowUpIcon class="mini-icon" />
                </button>
                <button
                  class="mini"
                  :disabled="i === track.effects.length - 1"
                  :aria-label="i18n.t('inspector.fxDown')"
                  @click="moveFx(i, i + 1)"
                >
                  <ArrowDownIcon class="mini-icon" />
                </button>
                <button class="mini danger" :aria-label="i18n.t('inspector.fxRemove')" @click="removeFx(i)">
                  <XMarkIcon class="mini-icon" />
                </button>
              </div>
            </div>
          </div>

          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.add") }}</span>
            <Md3Select
              :model-value="null"
              :options="fxOptions"
              :label="i18n.t('inspector.addFx')"
              :placeholder="i18n.t('inspector.chooseFx')"
              :empty="i18n.t('fx.none')"
              :action="{ label: i18n.t('inspector.browseFx') }"
              @select="addFx"
              @action="fxBrowserOpen = true"
            />
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.mix") }}</div>
        <div class="card">
          <div class="row param-row">
            <span class="row-label">{{ i18n.t("inspector.gain") }}</span>
            <Md3Slider
              class="param-slider"
              :model-value="shownGain"
              :min="MIN_DB"
              :max="MAX_DB"
              :step="0.5"
              :label="i18n.t('inspector.gain')"
              @update:model-value="(v: number) => (gainDraft = v)"
              @commit="commitGain"
            />
            <span class="row-value tabular">{{ shownGain.toFixed(1) }} dB</span>
          </div>
          <div class="row param-row">
            <span class="row-label">{{ i18n.t("inspector.pan") }}</span>
            <Md3Slider
              class="param-slider"
              :model-value="shownPan"
              :min="-1"
              :max="1"
              :step="0.02"
              :label="i18n.t('inspector.pan')"
              @update:model-value="(v: number) => (panDraft = v)"
              @commit="commitPan"
            />
            <span class="row-value tabular">{{ shownPan.toFixed(2) }}</span>
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.muteSolo") }}</span>
            <div class="strip-buttons">
              <button
                class="mini"
                :class="{ on: track.mixer.mute }"
                :aria-pressed="track.mixer.mute"
                :aria-label="i18n.t('inspector.mute')"
                @click="toggleMute"
              >
                M
              </button>
              <button
                class="mini solo"
                :class="{ on: track.mixer.solo }"
                :aria-pressed="track.mixer.solo"
                :aria-label="i18n.t('inspector.solo')"
                @click="toggleSolo"
              >
                S
              </button>
            </div>
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.sends") }}</div>
        <div class="card">
          <p v-if="sends.length === 0" class="hint">{{ i18n.t("inspector.sendsEmpty") }}</p>
          <div v-for="send in sends" :key="send.target" class="row send-row">
            <span class="row-label send-name" :title="targetName(send.target)">
              {{ targetName(send.target) }}
            </span>
            <Md3Slider
              class="param-slider"
              :model-value="shownSendGain(send)"
              :min="MIN_DB"
              :max="MAX_DB"
              :step="0.5"
              :label="i18n.t('inspector.sendGain')"
              @update:model-value="(v: number) => (sendDrafts[send.target] = v)"
              @commit="(v: number) => commitSendGain(send.target, v)"
            />
            <span class="row-value tabular">{{ shownSendGain(send).toFixed(1) }} dB</span>
            <button
              class="mini danger"
              :aria-label="i18n.t('inspector.removeSend')"
              @click="removeSend(send.target)"
            >
              <XMarkIcon class="mini-icon" />
            </button>
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.addSend") }}</span>
            <Md3Select
              :model-value="null"
              :options="sendTargets"
              :label="i18n.t('inspector.chooseSend')"
              :placeholder="i18n.t('inspector.chooseSend')"
              @update:model-value="addSend"
            />
          </div>
        </div>
      </section>

      <PluginBrowser
        v-model:open="browserOpen"
        kind="instrument"
        :current="currentTimbre"
        @pick="onPickTimbre"
      />

      <PluginBrowser
        v-model:open="fxBrowserOpen"
        kind="effect"
        :chain="track.effects"
        @toggle="toggleFx"
      />
    </template>

    <template v-else>
      <dl class="rows">
        <div v-for="row in overview" :key="row.label" class="row">
          <dt>{{ row.label }}</dt>
          <dd>{{ row.value }}</dd>
        </div>
      </dl>
      <p class="hint">{{ i18n.t("inspector.selectTrackHint") }}</p>
    </template>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px;
  gap: 14px;
}
.group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.group-title {
  font-size: 10.5px;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
  padding-left: 4px;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px;
  border-radius: 16px;
  background-color: var(--app-inset);
}
.rows {
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 34px;
  padding: 0 10px;
  border-radius: 12px;
  font-size: 12px;
}
.row-label {
  color: rgb(var(--md-on-surface-variant));
  flex: 0 0 auto;
  white-space: nowrap;
}
.row-value {
  color: rgb(var(--md-on-surface));
  font-weight: 500;
  flex: 0 0 auto;
  font-variant-numeric: tabular-nums;
}
.rows .row {
  background-color: var(--app-inset);
}
.rows dt {
  color: rgb(var(--md-on-surface-variant));
}
.rows dd {
  margin: 0;
  color: rgb(var(--md-on-surface));
  font-weight: 500;
}
/* 参数行：标签固定宽度 + 滑块撑满 + 数值固定宽度，三行对齐 */
.param-row {
  display: grid;
  grid-template-columns: 52px 1fr 62px;
  gap: 8px;
}
.param-slider {
  height: 20px;
}
.name-input {
  flex: 1 1 auto;
  min-width: 0;
}
.strip-buttons {
  display: flex;
  gap: 5px;
}
.mini {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 8px;
  padding: 0;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10.5px;
  font-weight: 700;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.mini:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
/* 图标用 SVG（不靠字形，天然居中）。 */
.mini-icon {
  width: 14px;
  height: 14px;
  display: block;
}
.mini:disabled {
  opacity: 0.35;
  cursor: default;
}
.mini:disabled:hover {
  background-color: transparent;
}
/* 删除键：红色（危险动作的惯例）。 */
.mini.danger {
  color: rgb(var(--md-error));
  border-color: rgb(var(--md-error) / 0.5);
}
.mini.danger:hover:not(:disabled) {
  background-color: rgb(var(--md-error) / 0.12);
}
.mini.on {
  background-color: rgb(var(--md-error));
  border-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.mini.solo.on {
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
/* 发送行：与增益/声像**同一套列宽**（52 / 1fr / 62），滑块左边缘因此严格对齐；
   第 4 列是删除键。不要套 `.param-row`——那是 3 列，第 4 个元素会被挤到下一行。 */
.send-row {
  display: grid;
  grid-template-columns: 52px 1fr 62px 24px;
  gap: 8px;
  align-items: center;
}
/* 发送目标名可能很长：单行省略号，不挤坏滑块。 */
.send-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hint {
  margin: 0 8px;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
/* 效果器链 */
.fx {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 2px 0;
  border-bottom: 1px solid rgb(var(--md-outline-variant) / 0.25);
}
.fx:last-of-type {
  border-bottom: 0;
}
.fx-name {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
/* 乐器卡片里的「打开界面」：一颗药丸按钮，右对齐 */
.plugin-name {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}
.open-editor {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: filter 0.14s ease;
}
/* 效果器行里的那颗更小：一行要放下名称 + 界面 + 三个小按钮 */
.open-editor.small {
  height: 24px;
  padding: 0 10px;
  font-size: 10.5px;
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.open-editor:hover {
  filter: brightness(1.08);
}
.open-editor:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.tabular {
  font-variant-numeric: tabular-nums;
}
</style>
