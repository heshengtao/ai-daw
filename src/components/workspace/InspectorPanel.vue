<script setup lang="ts">
/**
 * 检查器面板：编辑**当前选中轨道**。
 *
 * 只做三组内容，外加一组片段参数：
 * - 轨道名（可直接改，Enter/失焦提交）；
 * - 乐器（挂载 / 卸载音色，参数在插件自己的界面里编）；
 * - 效果器链（增删 / 换顺序，参数同样不在展开）；
 * - 音频片段的速度 / 升降调（选中音频片段时才出现）。
 *
 * 混音（增益 / 声像 / 静音 / 独奏）与发送**不在这里**——它们归「调音台」面板。
 * 未选中轨道时退回工程概览。
 */
import { computed, ref, watch } from "vue";
import { ArrowDownIcon, ArrowUpIcon, XMarkIcon } from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import PluginBrowser from "@/components/common/PluginBrowser.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { useParamBrowserStore } from "@/stores/paramBrowser";
import { usePluginInterface } from "@/composables/usePluginInterface";
import { canonicalPluginId } from "@/data/plugins";
import { useI18n } from "@/i18n";
import type { Note, NotePatch } from "@/api/types";

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

// ---- 音频片段：播放速度 / 升降调（与工程 BPM 解耦；无音频片段时整块灰显） ----

const MIN_RATE = 0.25;
const MAX_RATE = 4;
const MAX_PITCH = 24;

/** 选中的是不是音频片段。 */
const audioClip = computed(() => !!selection.clip?.audio);

const shownRate = computed(() => selection.clip?.audio?.rate ?? 1);
const shownPitch = computed(() => selection.clip?.audio?.pitch_semitones ?? 0);

/** 提交速度（`rate`）；升降调沿用当前值。 */
async function commitRate(value: number): Promise<void> {
  const c = selection.clip;
  const t = track.value;
  if (!c || !t || !c.audio) return;
  try {
    await store.setClipRate(t.id, c.id, value, c.audio.pitch_semitones ?? 0);
  } catch (err) {
    layout.notify(i18n.t("error.setClipRate", { error: String(err) }));
  }
}

/** 提交升降调（半音）；速度沿用当前值。 */
async function commitPitch(value: number): Promise<void> {
  const c = selection.clip;
  const t = track.value;
  if (!c || !t || !c.audio) return;
  try {
    await store.setClipRate(t.id, c.id, c.audio.rate ?? 1, value);
  } catch (err) {
    layout.notify(i18n.t("error.setClipRate", { error: String(err) }));
  }
}

// ---- 音符（有音符选中时才出现）：扫弦偏移 / 连奏 ----
//
// 「起始偏移」是**每个音符**的字段（拍）：同一个 `start` 上的一组音，给它们写递增偏移就是扫弦
// （下扫 = 低音先响）。缺省 0 = 同时发声。卷帘里音符仍画在**记谱位置**上（网格保持诚实），
// 偏移只在这里读数与调整——这样量化、对齐、复制粘贴都不会把扫弦搞乱。

/** 起点归组用的键：1e-6 拍以内的起点算「同一格」（浮点上的同一格不该被拆成两组）。 */
function startKey(startBeat: number): string {
  return (Math.round(startBeat * 1e6) / 1e6).toFixed(6);
}

/** 选中的音符（只取拿得到稳定 id 的——改写按 id 定位）。 */
const pickedNotes = computed(() => {
  const c = selection.clip;
  if (!c) return [];
  const out: { index: number; id: number; note: Note }[] = [];
  for (const index of selection.noteIndices) {
    const note = c.notes[index];
    if (note && typeof note.id === "number") out.push({ index, id: note.id, note });
  }
  return out;
});

/**
 * 参与扫弦的音符：选中音符所在**起点**上的**全部**音符。
 *
 * 展开成整组是有意的——选和弦里任意一个音再点「应用」就该把整组扫开，而不是只动那一个。
 */
const strumGroup = computed(() => {
  const c = selection.clip;
  if (!c || pickedNotes.value.length === 0) return [];
  const starts = new Set(pickedNotes.value.map((p) => startKey(c.start + p.note.start)));
  const out: { id: number; note: Note; startBeat: number }[] = [];
  for (const note of c.notes) {
    if (typeof note.id !== "number") continue;
    const startBeat = c.start + note.start;
    if (starts.has(startKey(startBeat))) out.push({ id: note.id, note, startBeat });
  }
  return out;
});

/** 扫弦间隔（ms）：每根弦之间差多少毫秒。0 = 同时发声。 */
const strumMs = ref(28);
/** 扫弦方向：下扫 = 低音先响（吉他上最常用的那一种）。 */
const strumDirection = ref("down");

const strumDirections = computed(() => [
  { id: "down", label: i18n.t("inspector.strumDown") },
  { id: "up", label: i18n.t("inspector.strumUp") },
]);

/** 参与扫弦的音符当前偏移范围（读数）。 */
const offsetRange = computed(() => {
  const offsets = strumGroup.value.map((entry) => entry.note.offset ?? 0);
  if (offsets.length === 0) return null;
  return { min: Math.min(...offsets), max: Math.max(...offsets) };
});

/** 连奏状态：整组都已连奏 / 都没有 / 混合。 */
const legatoState = computed<"on" | "off" | "mixed">(() => {
  const values = pickedNotes.value.map((p) => !!p.note.legato);
  if (values.length === 0) return "off";
  if (values.every(Boolean)) return "on";
  return values.some(Boolean) ? "mixed" : "off";
});

/**
 * 应用扫弦：把**起点相同**的一组音按音高依次错开。
 *
 * 手势（分组、排序、毫秒→拍）在引擎里做（`Command::StrumNotes`），界面只给参数——
 * 这样界面与 AI 侧的 `strum_notes` 工具永远是同一套语义。间隔填 0 就是清掉扫弦。
 */
async function applyStrum(): Promise<void> {
  const c = selection.clip;
  const t = track.value;
  if (!c || !t || strumGroup.value.length === 0) return;
  try {
    await store.strumNotes(t.id, c.id, strumMs.value, strumDirection.value as "down" | "up");
    layout.notify(i18n.t("inspector.strumApplied", { count: strumGroup.value.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

/** 把选中的音符整体设为 / 取消连奏。 */
async function setLegato(on: boolean): Promise<void> {
  const c = selection.clip;
  const t = track.value;
  if (!c || !t || pickedNotes.value.length === 0) return;
  const patches: NotePatch[] = pickedNotes.value.map((entry) => ({ id: entry.id, legato: on }));
  try {
    await store.updateNotes(t.id, c.id, patches);
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
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

      <!-- 音符：只在该片段里有音符被选中时出现 -->
      <section v-if="pickedNotes.length > 0" class="group">
        <div class="group-title">{{ i18n.t("inspector.note") }}</div>
        <div class="card">
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.noteSelected") }}</span>
            <span class="row-value">{{ pickedNotes.length }}</span>
          </div>

          <!-- 扫弦：同一起点的一组音按音高依次错开（间隔 0 = 同时发声） -->
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.strum") }}</span>
            <NumberField
              v-model="strumMs"
              :sr-label="i18n.t('inspector.strum')"
              :min="0"
              :max="500"
              :step="1"
            />
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.strumDirection") }}</span>
            <Md3Select
              v-model="strumDirection"
              :options="strumDirections"
              :label="i18n.t('inspector.strumDirection')"
            />
          </div>
          <div class="row">
            <span class="row-value">
              {{ i18n.t("inspector.strumScope", { count: strumGroup.length }) }}
            </span>
            <button class="open-editor small" @click="applyStrum">
              {{ i18n.t("inspector.strumApply") }}
            </button>
          </div>
          <p v-if="offsetRange" class="hint">
            {{
              i18n.t("inspector.offsetRange", {
                min: offsetRange.min.toFixed(3),
                max: offsetRange.max.toFixed(3),
              })
            }}
          </p>

          <!-- 连奏：NoteOff 延到同音高的下一个音（重复同一个音不断音） -->
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.legato") }}</span>
            <div class="strip-buttons">
              <button
                class="pill"
                :class="{ on: legatoState === 'on' }"
                :aria-pressed="legatoState === 'on'"
                @click="setLegato(true)"
              >
                {{ i18n.t("inspector.legatoOn") }}
              </button>
              <button
                class="pill"
                :class="{ on: legatoState === 'off' }"
                :aria-pressed="legatoState === 'off'"
                @click="setLegato(false)"
              >
                {{ i18n.t("inspector.legatoOff") }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <section class="group">
        <div class="group-title">{{ i18n.t("inspector.audioClip") }}</div>
        <div class="card">
          <p v-if="!audioClip" class="hint">{{ i18n.t("inspector.audioClipHint") }}</p>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.rate") }}</span>
            <NumberField
              :model-value="shownRate"
              :sr-label="i18n.t('inspector.rate')"
              :min="MIN_RATE"
              :max="MAX_RATE"
              :step="0.01"
              :decimals="2"
              :disabled="!audioClip"
              @update:model-value="commitRate"
            />
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("inspector.pitch") }}</span>
            <NumberField
              :model-value="shownPitch"
              :sr-label="i18n.t('inspector.pitch')"
              :min="-MAX_PITCH"
              :max="MAX_PITCH"
              :step="1"
              :disabled="!audioClip"
              @update:model-value="commitPitch"
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
/* 二选一的文字药丸（连奏 开 / 关）：选中的那颗用主色实心。 */
.pill {
  height: 26px;
  padding: 0 12px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.pill:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.pill.on {
  border-color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.pill:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
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
</style>
