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
import { computed, ref, watch } from "vue";
import Md3Slider from "@/components/common/Md3Slider.vue";
import InstrumentSelect from "@/components/common/InstrumentSelect.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import PluginParamList from "@/components/common/PluginParamList.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { usePluginInterface } from "@/composables/usePluginInterface";
import { useI18n } from "@/i18n";

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const i18n = useI18n();
const pluginInterface = usePluginInterface();

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

// ---- 参数 ----

// 乐器参数不在这里列——检查器只给一个「打开界面」入口，编辑都去插件的独立界面
// （内置合成器有完整的合成器面板，外部插件开它自己的原生窗口 / 参数面板）。

// ---- 效果器链 ----

/** 链上效果器的参数表同样是懒加载。 */
watch(
  () => (track.value?.effects ?? []).map((f) => f.plugin_id).join("|"),
  () => {
    for (const fx of track.value?.effects ?? []) void plugins.ensureParams(fx.plugin_id);
  },
  { immediate: true },
);

/** 拖动中的效果器参数草稿由 `PluginParamList` 内部管理，这里只处理提交。 */
async function onFxCommit(index: number, id: string, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await store.setEffectParam(t.id, index, id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setEffectParam", { error: String(err) }));
  }
}

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
              class="name-input"
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
            <InstrumentSelect
              :model-value="track.instrument?.plugin_id ?? ''"
              @update:model-value="setInstrument"
            />
          </div>

          <!-- 参数不在这里展开：留一个入口，编辑去插件的独立界面。 -->
          <div v-if="track.instrument" class="row">
            <span class="row-value plugin-name">{{ plugins.name(track.instrument.plugin_id) }}</span>
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
              <div class="strip-buttons">
                <button class="mini" :disabled="i === 0" :aria-label="i18n.t('inspector.fxUp')" @click="moveFx(i, i - 1)">
                  ↑
                </button>
                <button
                  class="mini"
                  :disabled="i === track.effects.length - 1"
                  :aria-label="i18n.t('inspector.fxDown')"
                  @click="moveFx(i, i + 1)"
                >
                  ↓
                </button>
                <button class="mini" :aria-label="i18n.t('inspector.fxRemove')" @click="removeFx(i)">×</button>
              </div>
            </div>

            <PluginParamList
              :plugin-id="fx.plugin_id"
              :values="fx.params"
              @commit="(id: string, v: number) => onFxCommit(i, id, v)"
            />
          </div>

          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.add") }}</span>
            <Md3Select
              class="fx-select"
              :model-value="null"
              :options="fxOptions"
              :label="i18n.t('inspector.addFx')"
              :placeholder="i18n.t('inspector.chooseFx')"
              :empty="i18n.t('fx.none')"
              @select="addFx"
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
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-high));
  color: rgb(var(--md-on-surface));
  font-size: 12px;
  outline: none;
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}
.name-input:focus {
  border-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.22);
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
.fx-select {
  flex: 1 1 auto;
  min-width: 0;
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
