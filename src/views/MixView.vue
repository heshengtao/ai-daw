<script setup lang="ts">
/**
 * 混音视图：通道条横向排列，第一列固定是主输出。
 *
 * 每一条的节奏参照 openDAW / 主流调音台：**名字 → 发送 → 声像 → 音量（读数）→ 推子 + 表 →
 * 静音/独奏**。插入效果**不铺在通道条上**——它们属于「设备链」，在检查器与右侧混音面板里编辑，
 * 这里只用一个角标显示条数。这样一眼看到的是「电平与路由」，而不是一列小方块。
 *
 * 拖动推子时调用 `monitorMix`（只改实时声音、不进历史），松手才提交一次正式编辑，
 * 因此一次拖动只产生**一条**撤销记录，同时又能实时听到变化。
 *
 * 宽度固定：名字再长也只截断，不会把通道条撑宽（`min-width: 0` + 省略号）。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import { MusicalNoteIcon, SpeakerWaveIcon, RectangleStackIcon } from "@heroicons/vue/24/outline";
import Md3Slider from "@/components/common/Md3Slider.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import ChannelMeter from "@/components/common/ChannelMeter.vue";
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

/** 推子旁边的刻度（dB）：只画 6 dB 一档，0 以上留空。 */
const SCALE = [0, -6, -12, -18, -24, -30, -36, -42, -48, -54];

/** 与引擎 `MixerChannel` 的夹紧范围保持一致。 */
const MIN_DB = -60;
const MAX_DB = 12;
/** 推子的量化步长（dB）。 */
const STEP_DB = 0.5;
/** 主输出在草稿状态里的哨兵 id。 */
const MASTER = -1;

const MASTER_DEFAULT: MixerChannel = { gain_db: 0, pan: 0, mute: false, solo: false };

interface Strip {
  id: number;
  /** 轨道名（用户数据，原样显示）。 */
  name: string;
  /** 主输出这类**内置条**的名字走 i18n（有 nameKey 就用它）。 */
  nameKey?: string;
  mix: MixerChannel;
  hue: number | null;
  master: boolean;
  /** 通道类型：主输出 / 有乐器 / 空轨（总线）。 */
  kind: "master" | "instrument" | "bus";
  inserts: PluginInstance[];
  /** 发送摘要：目标名 + 增益 + 是否推子前。 */
  sends: { target: number; name: string; gain: number; pre: boolean; mute: boolean }[];
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
    master: true,
    kind: "master",
    inserts: store.project?.master_effects ?? [],
    sends: [],
  },
  ...tracks.value.map((t, i) => ({
    id: t.id,
    name: t.name,
    mix: t.mixer,
    hue: trackHue(i),
    master: false,
    kind: t.instrument ? ("instrument" as const) : ("bus" as const),
    inserts: t.effects,
    sends: (t.sends ?? []).map((s) => ({
      target: s.target,
      name: trackName(s.target),
      gain: s.gain_db,
      pre: s.mode === "pre",
      mute: Boolean(s.mute),
    })),
  })),
]);

const anySolo = computed(() => tracks.value.some((t) => t.mixer.solo));

/** 被独奏压掉的轨道：视觉上变暗，与听感一致。 */
function dimmed(s: Strip): boolean {
  return anySolo.value && !s.master && !s.mix.solo && !s.mix.mute;
}

/** 某条通道的表（主输出单独一条）。 */
function meterOf(s: Strip) {
  return s.master ? meters.master : meters.track(s.id);
}

function resetClip(): void {
  void meters.clearClip();
}

/** 峰值 → 读数文本（-∞ 表示没信号）。 */
function peakText(s: Strip): string {
  const value = meterOf(s);
  const peak = Math.max(value.l, value.r);
  if (!(peak > 0)) return "-∞";
  const db = 20 * Math.log10(peak);
  return db < -60 ? "-∞" : db.toFixed(1);
}

/** 当前通道是否削波（表头锁存）。 */
function clipping(s: Strip): boolean {
  return meterOf(s).clipped;
}

/** 插入摘要：名字连起来做 tooltip。 */
function insertSummary(s: Strip): string {
  if (s.inserts.length === 0) return i18n.t("mix.noInserts");
  const chain = s.inserts.map((fx) => plugins.name(fx.plugin_id)).join(" → ");
  return i18n.t("mix.inserChain", { chain });
}

/** 点名字 / 角标：选中该轨并打开检查器（设备链在那边）。 */
function openChannel(s: Strip): void {
  if (!s.master) selection.selectTrack(s.id);
  layout.openTab("inspector");
}

/** 点发送摘要：去右侧混音面板改发送。 */
function openSends(s: Strip): void {
  if (!s.master) selection.selectTrack(s.id);
  layout.openTab("mixer");
}

/** 发送小标签：`推子前` 用一个点标记，哑掉画删除线。 */
function sendGain(s: { gain: number }): string {
  return `${s.gain > 0 ? "+" : ""}${s.gain.toFixed(1)}`;
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
        <div class="strip-head">
          <span
            class="kind-badge"
            :class="{ master: s.master }"
            :style="s.hue !== null ? { '--track-hue': String(s.hue) } : undefined"
            aria-hidden="true"
          >
            <SpeakerWaveIcon v-if="s.kind === 'master'" class="kind-icon" />
            <MusicalNoteIcon v-else-if="s.kind === 'instrument'" class="kind-icon" />
            <RectangleStackIcon v-else class="kind-icon" />
          </span>
        </div>

        <button
          class="strip-name"
          :title="stripName(s)"
          :aria-label="s.master ? i18n.t('mix.master') : i18n.t('mix.selectNamed', { name: s.name })"
          @click="s.master ? selection.clear() : selection.selectTrack(s.id)"
        >
          <span class="name-text">{{ stripName(s) }}</span>
          <span
            v-if="s.inserts.length > 0"
            class="fx-badge"
            :title="insertSummary(s)"
            role="button"
            tabindex="0"
            @click.stop="openChannel(s)"
            @keydown.enter.stop="openChannel(s)"
          >
            {{ s.inserts.length }}
          </span>
        </button>

        <!-- 发送：一行摘要，点开去右侧面板改（通道条上不摆控件） -->
        <div v-if="!s.master" class="field">
          <span class="field-label">{{ i18n.t("mix.sends") }}</span>
          <div class="field-value">
            <button
              v-for="send in s.sends.slice(0, 1)"
              :key="send.target"
              class="chip"
              :class="{ off: send.mute }"
              :title="`${i18n.t('mix.sendTo', { name: send.name })} · ${sendGain(send)} dB${send.pre ? ' · ' + i18n.t('mixer.pre') : ''}`"
              @click="openSends(s)"
            >
              <span v-if="send.pre" class="pre-dot" aria-hidden="true" />
              {{ send.name }}
            </button>
            <button
              v-if="s.sends.length > 1"
              class="chip more"
              :title="i18n.t('mix.sendTo', { name: s.sends[1]?.name ?? '' })"
              @click="openSends(s)"
            >
              +{{ s.sends.length - 1 }}
            </button>
            <button
              v-if="s.sends.length === 0"
              class="chip add"
              :aria-label="i18n.t('mixer.addSend')"
              :title="i18n.t('mixer.addSend')"
              @click="openSends(s)"
            >
              +
            </button>
          </div>
        </div>

        <div v-if="!s.master" class="field">
          <span class="field-label">{{ i18n.t("mix.output") }}</span>
          <span class="out-value" :title="i18n.t('mix.outputMasterHint')">
            {{ i18n.t("mix.master") }}
          </span>
        </div>

        <div v-if="!s.master" class="field">
          <span class="field-label">{{ i18n.t("mix.pan") }}</span>
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

        <!-- 读数：左边是推子值，右边是当前峰值（削波变红）——与 openDAW 的「两格数字」一致 -->
        <div class="field readout">
          <span class="field-label">{{ i18n.t("mix.volume") }}</span>
          <span class="num tabular">{{ shownGain(s).toFixed(1) }}</span>
          <span class="num peak tabular" :class="{ clip: clipping(s) }">{{ peakText(s) }}</span>
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
          <div class="fader-scale" aria-hidden="true">
            <span
              v-for="db in SCALE"
              :key="db"
              :style="{ bottom: `${faderRatio(db) * 100}%` }"
              >{{ db }}</span
            >
          </div>
          <ChannelMeter :value="meterOf(s)" :label="stripName(s)" @reset="resetClip" />
        </div>

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
  gap: 6px;
  padding: 12px;
  min-width: 100%;
  box-sizing: border-box;
}
.strip {
  /* 固定宽度 + min-width: 0：长轨道名只会被省略号截断，不会把通道条撑宽。 */
  flex: 0 0 132px;
  width: 132px;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 4px;
  padding: 8px 7px;
  border-radius: 16px;
  background-color: var(--app-inset);
  border: 1px solid rgb(var(--md-outline-variant) / 0.25);
  transition: border-color 0.15s ease, opacity 0.15s ease;
  overflow: hidden;
}
.strip.selected {
  border-color: rgb(var(--md-primary) / 0.65);
}
.strip.dimmed {
  opacity: 0.5;
}
.strip.master {
  background-color: rgb(var(--md-primary) / 0.07);
  border-color: rgb(var(--md-primary) / 0.3);
}

/* 名字：单行省略 */
.strip-name {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  width: 100%;
  height: 22px;
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
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
/* 通道身份：一块带轨道色的图标（openDAW 的「设备图标」位） */
.strip-head {
  display: flex;
  justify-content: center;
  padding: 2px 0 4px;
}
.kind-badge {
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 9px;
  background-color: hsl(var(--track-hue, 210) 60% 55% / 0.14);
  color: hsl(var(--track-hue, 210) 62% 68%);
}
.kind-badge.master {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.kind-icon {
  width: 15px;
  height: 15px;
}
.out-value {
  flex: 1 1 auto;
  min-width: 0;
  height: 15px;
  line-height: 15px;
  padding: 0 4px;
  border-radius: 4px;
  background-color: rgb(var(--md-on-surface) / 0.04);
  color: rgb(var(--md-on-surface-variant));
  font-size: 8.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.hue.master {
  background-color: rgb(var(--md-primary));
}
/* 插入条数角标：不是按钮排，只是一个数字 */
.fx-badge {
  flex: 0 0 auto;
  min-width: 14px;
  height: 14px;
  padding: 0 3px;
  border-radius: 4px;
  background-color: rgb(var(--md-on-surface) / 0.08);
  color: rgb(var(--md-on-surface-variant));
  font-size: 9px;
  line-height: 14px;
  text-align: center;
  cursor: pointer;
}
.fx-badge:hover {
  color: rgb(var(--md-primary));
  background-color: rgb(var(--md-primary) / 0.14);
}

/* 字段行：标签在左（28px 固定），值在右——openDAW 的排版节奏 */
.field {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 18px;
  padding: 0 2px;
}
.field-label {
  flex: 0 0 26px;
  font-size: 8.5px;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.field-value {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 3px;
  overflow: hidden;
}
.chip {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  min-width: 0;
  height: 15px;
  padding: 0 4px;
  border: 0;
  border-radius: 4px;
  background-color: rgb(var(--md-on-surface) / 0.07);
  color: rgb(var(--md-on-surface-variant));
  font-size: 8.5px;
  line-height: 15px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.chip:hover {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
}
.chip.off {
  opacity: 0.45;
  text-decoration: line-through;
}
.chip.more,
.chip.add {
  flex: 0 0 auto;
  padding: 0 3px;
  color: rgb(var(--md-on-surface-muted));
}
.chip.add {
  border: 1px dashed rgb(var(--md-outline-variant) / 0.8);
  background: transparent;
}
.pre-dot {
  width: 3px;
  height: 3px;
  border-radius: 50%;
  background-color: rgb(var(--md-primary));
}
.pan-slider {
  flex: 1 1 auto;
  min-width: 0;
}
.readout {
  gap: 2px;
}
.num {
  flex: 1 1 0;
  min-width: 0;
  height: 16px;
  border-radius: 4px;
  background-color: rgb(var(--md-on-surface) / 0.06);
  color: rgb(var(--md-on-surface));
  font-size: 9px;
  line-height: 16px;
  text-align: center;
  overflow: hidden;
}
.num.peak {
  color: rgb(var(--md-on-surface-variant));
}
.num.peak.clip {
  color: rgb(var(--md-on-primary));
  background-color: rgb(var(--md-error));
}

/* 推子 + 表 */
.fader-row {
  flex: 1 1 auto;
  min-height: 120px;
  display: flex;
  align-items: stretch;
  justify-content: center;
  gap: 2px;
  padding: 2px 2px 0;
}
/* 刻度：与导轨上下各留 4px 对齐 */
.fader-scale {
  position: relative;
  flex: 0 0 15px;
  width: 15px;
  margin: 4px 0;
  font-size: 7px;
  line-height: 1;
  color: rgb(var(--md-on-surface-muted));
}
.fader-scale span {
  position: absolute;
  right: 0;
  transform: translateY(50%);
  white-space: nowrap;
}
.fader {
  position: relative;
  width: 24px;
  min-width: 24px;
  flex: 0 0 auto;
  cursor: ns-resize;
  touch-action: none;
}
/* 导轨 */
.fader-rail {
  position: absolute;
  left: 50%;
  top: 4px;
  bottom: 4px;
  width: 4px;
  margin-left: -2px;
  border-radius: 2px;
  background-color: rgb(var(--md-on-surface) / 0.1);
}
.fader-fill {
  position: absolute;
  left: 50%;
  bottom: 4px;
  width: 4px;
  margin-left: -2px;
  border-radius: 2px;
  background-color: rgb(var(--md-primary) / 0.5);
}
.fader-knob {
  position: absolute;
  left: 50%;
  width: 18px;
  height: 12px;
  margin-left: -9px;
  margin-bottom: -6px;
  border-radius: 3px;
  background-color: rgb(var(--md-surface-container-high, 40 40 44));
  border: 1px solid rgb(var(--md-outline-variant) / 0.9);
  box-shadow: 0 1px 3px rgb(0 0 0 / 0.35);
}
.fader-knob::after {
  content: "";
  position: absolute;
  left: 2px;
  right: 2px;
  top: 50%;
  height: 1px;
  background-color: rgb(var(--md-on-surface) / 0.55);
}

.strip-buttons {
  display: flex;
  justify-content: center;
  gap: 4px;
}
.mini {
  width: 28px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
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
