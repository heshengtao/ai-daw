<script setup lang="ts">
/**
 * 混音视图：通道条横向排列，第一列固定是主输出。
 *
 * 一个通道条从上到下是主流调音台的顺序：**名字 → 插入 → 发送 → 声像 → 推子 + 表 → 读数 →
 * 静音/独奏**。这样眼睛不用来回跳：越往下越是「末端」的操作。
 *
 * 拖动推子时调用 `monitorMix`（只改实时声音、不进历史），松手才提交一次正式编辑，
 * 因此一次拖动只产生**一条**撤销记录，同时又能实时听到变化。
 *
 * 宽度是**固定 112 px**：名字再长也只截断，绝不会把通道条撑宽——这一点由 `min-width: 0`
 * 与省略号共同保证（flex 项默认 `min-width: auto`，长名字会把整条撑开）。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import Md3Slider from "@/components/common/Md3Slider.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import ChannelMeter from "@/components/common/ChannelMeter.vue";
import InsertSlot from "@/components/common/InsertSlot.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { useMetersStore } from "@/stores/meters";
import { trackHue } from "@/utils/trackColor";
import { capturePointer } from "@/utils/pointer";
import { useI18n } from "@/i18n";
import type { MixerChannel, PluginInstance } from "@/api/types";

const store = useProjectStore();
const i18n = useI18n();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const meters = useMetersStore();

/** 与引擎 `MixerChannel` 的夹紧范围保持一致。 */
const MIN_DB = -60;
const MAX_DB = 12;
/** 推子的量化步长（dB）。 */
const STEP_DB = 0.5;
/** 主输出在草稿状态里的哨兵 id。 */
const MASTER = -1;
/** 通道条上最多直接显示几个插入 / 发送槽，多出来的折成「+N」。 */
const MAX_SLOTS = 3;

const MASTER_DEFAULT: MixerChannel = { gain_db: 0, pan: 0, mute: false, solo: false };

interface Strip {
  id: number;
  /** 轨道名（用户数据，原样显示）。 */
  name: string;
  /** 主输出这类**内置条**的名字走 i18n（有 nameKey 就用它）。 */
  nameKey?: string;
  mix: MixerChannel;
  hue: number | null;
  instrument: boolean;
  master: boolean;
  inserts: PluginInstance[];
  /** 发送摘要：目标名 + 增益。 */
  sends: { target: number; name: string; gain: number }[];
}

/** 推子的显示名：内置条翻译，轨道条用原文。 */
function stripName(s: Strip): string {
  return s.nameKey ? i18n.t(s.nameKey) : s.name;
}

const tracks = computed(() => store.project?.tracks ?? []);
const masterMix = computed<MixerChannel>(() => store.project?.master ?? MASTER_DEFAULT);

function trackName(id: number): string {
  return tracks.value.find((t) => t.id === id)?.name ?? `#${id}`;
}

const strips = computed<Strip[]>(() => [
  {
    id: MASTER,
    name: "",
    nameKey: "mix.master",
    mix: masterMix.value,
    hue: null,
    instrument: true,
    master: true,
    inserts: store.project?.master_effects ?? [],
    sends: [],
  },
  ...tracks.value.map((t, i) => ({
    id: t.id,
    name: t.name,
    mix: t.mixer,
    hue: trackHue(i),
    instrument: Boolean(t.instrument),
    master: false,
    inserts: t.effects,
    sends: (t.sends ?? []).map((s) => ({
      target: s.target,
      name: trackName(s.target),
      gain: s.gain_db,
    })),
  })),
]);

const anySolo = computed(() => tracks.value.some((t) => t.mixer.solo));

/** 被独奏压掉的轨道：视觉上变暗，与听感一致。 */
function dimmed(s: Strip): boolean {
  return anySolo.value && !s.master && !s.mix.solo && !s.mix.mute;
}

function pluginName(id: string): string {
  return plugins.name(id);
}

/** 某条通道的表（主输出单独一条）。 */
function meterOf(s: Strip) {
  return s.master ? meters.master : meters.track(s.id);
}

function resetClip(): void {
  void meters.clearClip();
}

// ---- 推子拖动 ----

/** 正在拖动的通道；拖动期间显示草稿值而不是工程值。 */
const dragging = ref<number | null>(null);
const draft = reactive({ gain_db: 0, pan: 0 });

function shownGain(s: Strip): number {
  return dragging.value === s.id ? draft.gain_db : s.mix.gain_db;
}

/** 拖动中的声像草稿（否则滑块的把手不会跟手）。 */
function shownPan(s: Strip): number {
  return dragging.value === s.id ? draft.pan : s.mix.pan;
}

/** 增益（dB）→ 推子位置（0..1）。 */
function faderRatio(db: number): number {
  const clamped = Math.min(MAX_DB, Math.max(MIN_DB, db));
  return (clamped - MIN_DB) / (MAX_DB - MIN_DB);
}

function clampDb(db: number): number {
  return Math.min(MAX_DB, Math.max(MIN_DB, Math.round(db / STEP_DB) * STEP_DB));
}

/** 相对拖动的起点：记录按下时的指针位置与增益，避免按下瞬间跳变。 */
const dragBase = reactive({ startY: 0, gain_db: 0, height: 1 });

/** 把草稿值推给音频线程（只影响监听，不写工程）。 */
function monitor(s: Strip): void {
  if (s.master) return;
  void store.monitorMix(s.id, {
    gain_db: draft.gain_db,
    pan: draft.pan,
    mute: s.mix.mute,
    solo: s.mix.solo,
  });
}

function beginGainDrag(e: PointerEvent, s: Strip): void {
  if (e.button !== 0) return;
  const el = e.currentTarget as HTMLElement;
  capturePointer(el, e.pointerId);
  dragging.value = s.id;
  draft.gain_db = s.mix.gain_db;
  draft.pan = s.mix.pan;
  dragBase.startY = e.clientY;
  dragBase.gain_db = s.mix.gain_db;
  dragBase.height = Math.max(1, el.getBoundingClientRect().height);
}

function applyGainPointer(s: Strip, clientY: number): void {
  // 相对拖动：指针上移 = 增益变大。推子不因按下位置而跳变（与主流 DAW 一致）。
  const delta = ((dragBase.startY - clientY) / dragBase.height) * (MAX_DB - MIN_DB);
  const next = clampDb(dragBase.gain_db + delta);
  if (next === draft.gain_db) return;
  draft.gain_db = next;
  monitor(s);
}

function onGainMove(e: PointerEvent, s: Strip): void {
  if (dragging.value !== s.id) return;
  applyGainPointer(s, e.clientY);
}

async function endGainDrag(): Promise<void> {
  const id = dragging.value;
  if (id === null) return;
  const s = strips.value.find((x) => x.id === id);
  try {
    if (!s) return;
    if (s.master) await store.setMasterGain(draft.gain_db);
    else await store.setGain(s.id, draft.gain_db);
  } catch (err) {
    layout.notify(i18n.t("error.setGain", { error: String(err) }));
  } finally {
    dragging.value = null;
  }
}

// 指针可能在推子之外松开（尤其在捕获失败时），因此结束事件挂在 window 上。
onMounted(() => {
  window.addEventListener("pointerup", endGainDrag);
  window.addEventListener("pointercancel", endGainDrag);
  meters.start();
});
onUnmounted(() => {
  window.removeEventListener("pointerup", endGainDrag);
  window.removeEventListener("pointercancel", endGainDrag);
  meters.stop();
});

/** 声像滑块：拖动时监听，松手提交。 */
function onPanInput(s: Strip, value: number): void {
  dragging.value = s.id;
  draft.gain_db = s.mix.gain_db;
  draft.pan = value;
  monitor(s);
}

async function onPanCommit(s: Strip, value: number): Promise<void> {
  try {
    await store.setPan(s.id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setPan", { error: String(err) }));
  } finally {
    dragging.value = null;
  }
}

async function toggleMute(s: Strip): Promise<void> {
  try {
    await store.setMute(s.id, !s.mix.mute);
  } catch (err) {
    layout.notify(i18n.t("error.toggleMute", { error: String(err) }));
  }
}

async function toggleSolo(s: Strip): Promise<void> {
  try {
    await store.setSolo(s.id, !s.mix.solo);
  } catch (err) {
    layout.notify(i18n.t("error.toggleSolo", { error: String(err) }));
  }
}

// ---- 插入 / 发送 ----

/** 点插入槽：选中该轨并打开检查器（参数编辑在那边）。 */
function openInsert(s: Strip): void {
  if (!s.master) selection.selectTrack(s.id);
  layout.openTab("inspector");
}

/** 旁通 / 启用一个插入。 */
async function toggleInsert(s: Strip, index: number): Promise<void> {
  const fx = s.inserts[index];
  if (!fx) return;
  try {
    if (s.master) await store.setMasterEffectEnabled(index, !(fx.enabled ?? true));
    else await store.setEffectEnabled(s.id, index, !(fx.enabled ?? true));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 「+」：同样打开检查器，在那边挑插件（不在这里塞一个二级弹窗）。 */
function addInsert(s: Strip): void {
  openInsert(s);
}

/** 发送摘要的显示名：过长的目标名截断由 CSS 负责。 */
function sendLabel(s: { name: string; gain: number }): string {
  const gain = s.gain > 0 ? `+${s.gain.toFixed(1)}` : s.gain.toFixed(1);
  return `${s.name} ${gain}`;
}
</script>

<template>
  <div class="mix">
    <div class="mix-strips">
      <div
        v-for="s in strips"
        :key="s.id"
        class="strip"
        :class="{
          master: s.master,
          selected: !s.master && selection.trackId === s.id,
          dimmed: dimmed(s),
        }"
      >
        <button
          class="strip-name"
          :title="stripName(s)"
          :aria-label="s.master ? i18n.t('mix.master') : i18n.t('mix.selectNamed', { name: s.name })"
          @click="s.master ? selection.clear() : selection.selectTrack(s.id)"
        >
          <span
            v-if="!s.master"
            class="hue"
            :style="s.hue !== null ? { '--track-hue': String(s.hue) } : undefined"
          />
          <span class="name-text">{{ stripName(s) }}</span>
        </button>

        <!-- 插入：最多 3 个，多出来的折成 +N -->
        <div class="slots" :class="{ master: s.master }">
          <InsertSlot
            v-for="(fx, i) in s.inserts.slice(0, MAX_SLOTS)"
            :key="`${fx.plugin_id}-${i}`"
            :name="pluginName(fx.plugin_id)"
            :enabled="fx.enabled ?? true"
            :sidechain="fx.sidechain ? trackName(fx.sidechain.track) : null"
            @toggle="toggleInsert(s, i)"
            @open="openInsert(s)"
          />
          <span v-if="s.inserts.length > MAX_SLOTS" class="more">
            +{{ s.inserts.length - MAX_SLOTS }}
          </span>
          <button
            class="add-insert"
            :aria-label="i18n.t('mix.addInsert')"
            :title="i18n.t('mix.addInsert')"
            @click="addInsert(s)"
          >
            +
          </button>
        </div>

        <!-- 发送：只读摘要（编辑在右侧混音面板），让「有没有送去总线」一眼可见 -->
        <div v-if="s.sends.length > 0" class="sends">
          <span
            v-for="send in s.sends.slice(0, 2)"
            :key="send.target"
            class="send-chip"
            :title="i18n.t('mix.sendTo', { name: send.name })"
          >
            {{ sendLabel(send) }}
          </span>
          <span v-if="s.sends.length > 2" class="more">+{{ s.sends.length - 2 }}</span>
        </div>

        <div v-if="!s.master" class="pan-row">
          <Md3Slider
            class="pan-slider"
            :model-value="shownPan(s)"
            :min="-1"
            :max="1"
            :step="0.02"
            :label="i18n.t('mix.panNamed', { name: stripName(s) })"
            @update:model-value="(v: number) => onPanInput(s, v)"
            @commit="(v: number) => onPanCommit(s, v)"
          />
        </div>

        <div class="fader-row">
          <div
            class="fader"
            role="slider"
            tabindex="0"
            :aria-label="i18n.t('mix.gainNamed', { name: stripName(s) })"
            :aria-valuemin="MIN_DB"
            :aria-valuemax="MAX_DB"
            :aria-valuenow="s.mix.gain_db"
            @pointerdown="beginGainDrag($event, s)"
            @pointermove="onGainMove($event, s)"
          >
            <div class="fader-rail" />
            <div class="fader-fill" :style="{ height: `${faderRatio(shownGain(s)) * 100}%` }" />
            <div class="fader-knob" :style="{ bottom: `${faderRatio(shownGain(s)) * 100}%` }" />
          </div>
          <ChannelMeter
            :value="meterOf(s)"
            :label="stripName(s)"
            @reset="resetClip"
          />
        </div>

        <span class="strip-gain tabular">{{ shownGain(s).toFixed(1) }}</span>

        <div v-if="!s.master" class="strip-buttons">
          <Tooltip :text="s.mix.mute ? i18n.t('tracks.unmute') : i18n.t('tracks.mute')">
            <button
              class="mini"
              :class="{ on: s.mix.mute }"
              :aria-pressed="s.mix.mute"
              :aria-label="i18n.t('mix.muteNamed', { name: stripName(s) })"
              @click="toggleMute(s)"
            >
              M
            </button>
          </Tooltip>
          <Tooltip :text="s.mix.solo ? i18n.t('tracks.unsolo') : i18n.t('tracks.solo')">
            <button
              class="mini solo"
              :class="{ on: s.mix.solo }"
              :aria-pressed="s.mix.solo"
              :aria-label="i18n.t('mix.soloNamed', { name: stripName(s) })"
              @click="toggleSolo(s)"
            >
              S
            </button>
          </Tooltip>
        </div>
        <div v-else class="strip-tag">{{ i18n.t("mix.masterOut") }}</div>
      </div>

      <p v-if="tracks.length === 0" class="mix-empty text-body-sm">
        {{ i18n.t("mix.emptyTracks") }}
      </p>
    </div>
  </div>
</template>

<style scoped>
/* 靠左、占满全高；通道多时横向滚动 */
.mix {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow-x: auto;
  overflow-y: hidden;
  background-color: transparent;
}
.mix-strips {
  display: flex;
  align-items: stretch;
  gap: 8px;
  padding: 12px;
  min-width: 100%;
  box-sizing: border-box;
}
.strip {
  /* 固定宽度 + min-width: 0：长轨道名只会被省略号截断，不会把通道条撑宽。 */
  flex: 0 0 112px;
  width: 112px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 5px;
  padding: 8px 7px 7px;
  border-radius: 18px;
  background-color: var(--app-inset);
  border: 1px solid rgb(var(--md-outline-variant) / 0.35);
  transition: border-color 0.15s ease, opacity 0.15s ease;
  overflow: hidden;
}
.strip.selected {
  border-color: rgb(var(--md-primary) / 0.65);
}
.strip.dimmed {
  opacity: 0.45;
}
.strip.master {
  background-color: rgb(var(--md-primary) / 0.08);
  border-color: rgb(var(--md-primary) / 0.35);
}

/* 名字：单行省略，永远是 112px 里的一行 */
.strip-name {
  display: flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  width: 100%;
  height: 20px;
  padding: 0 2px;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  font-size: 11px;
  font-weight: 600;
  text-align: left;
  cursor: pointer;
}
.strip-name:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.name-text {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.hue {
  flex: 0 0 auto;
  width: 3px;
  height: 12px;
  border-radius: 2px;
  background-color: hsl(var(--track-hue, 210) 60% 55%);
}

/* 插入槽 */
.slots {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
  min-height: 18px;
}
.add-insert {
  height: 16px;
  padding: 0;
  border: 1px dashed rgb(var(--md-outline-variant) / 0.8);
  border-radius: 5px;
  background: transparent;
  color: rgb(var(--md-on-surface-muted));
  font-size: 11px;
  line-height: 14px;
  cursor: pointer;
}
.add-insert:hover {
  color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary) / 0.6);
}
.more {
  font-size: 9px;
  color: rgb(var(--md-on-surface-muted));
  text-align: center;
}

/* 发送摘要 */
.sends {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.send-chip {
  font-size: 9px;
  line-height: 14px;
  padding: 0 4px;
  border-radius: 4px;
  color: rgb(var(--md-on-surface-variant));
  background-color: rgb(var(--md-on-surface) / 0.05);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 声像 */
.pan-row {
  min-width: 0;
  padding: 0 1px;
}
.pan-slider {
  width: 100%;
  min-width: 0;
}

/* 推子 + 表 */
.fader-row {
  flex: 1 1 auto;
  min-height: 70px;
  display: flex;
  align-items: stretch;
  justify-content: center;
  gap: 6px;
}
.fader {
  position: relative;
  width: 26px;
  min-width: 26px;
  flex: 0 0 auto;
  cursor: ns-resize;
  touch-action: none;
}
.fader-rail {
  position: absolute;
  left: 50%;
  top: 4px;
  bottom: 4px;
  width: 4px;
  margin-left: -2px;
  border-radius: 2px;
  background-color: rgb(var(--md-on-surface) / 0.12);
}
.fader-fill {
  position: absolute;
  left: 50%;
  bottom: 4px;
  width: 4px;
  margin-left: -2px;
  border-radius: 2px;
  background-color: rgb(var(--md-primary) / 0.55);
}
.fader-knob {
  position: absolute;
  left: 50%;
  width: 22px;
  height: 12px;
  margin-left: -11px;
  margin-bottom: -6px;
  border-radius: 4px;
  background-color: rgb(var(--md-primary));
  box-shadow: 0 1px 3px rgb(0 0 0 / 0.25);
}

.strip-gain {
  text-align: center;
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-variant));
}

.strip-buttons {
  display: flex;
  justify-content: center;
  gap: 4px;
}
.mini {
  width: 26px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 6px;
  padding: 0;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 9.5px;
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
.strip-tag {
  text-align: center;
  font-size: 8.5px;
  letter-spacing: 0.5px;
  color: rgb(var(--md-primary));
}
.mix-empty {
  align-self: center;
  margin: 0 12px;
  color: rgb(var(--md-on-surface-muted));
}
</style>
