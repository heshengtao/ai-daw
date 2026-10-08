<script setup lang="ts">
/**
 * 混音视图：通道条靠左排列、占满整个高度。
 * 第一列固定是主输出，其后按工程顺序排列音轨——新增的轨道出现在最右边。
 *
 * 可执行的操作：
 * - 拖动推子 → 改增益；拖动声像滑块 → 改声像；
 * - M / S → 静音 / 独奏（独奏时其余轨道自动静音，与播放器语义一致）。
 *
 * 拖动过程中调用 `monitorMix`（只改实时声音、不进历史），松手时才提交一次正式编辑，
 * 因此一次拖动只产生**一条**撤销记录，同时又能实时听到变化。
 */
import { computed, onMounted, onUnmounted, reactive, ref } from "vue";
import Md3Slider from "@/components/common/Md3Slider.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { trackHue } from "@/utils/trackColor";
import { capturePointer } from "@/utils/pointer";
import { useI18n } from "@/i18n";
import type { MixerState } from "@/api/types";

const store = useProjectStore();
const i18n = useI18n();
const selection = useSelectionStore();
const layout = useLayoutStore();

/** 与引擎 `MixerChannel` 的夹紧范围保持一致。 */
const MIN_DB = -60;
const MAX_DB = 12;
/** 推子的量化步长（dB）。 */
const STEP_DB = 0.5;
/** 主输出在草稿状态里的哨兵 id。 */
const MASTER = -1;

const MASTER_DEFAULT: MixerState = { gain_db: 0, pan: 0, mute: false, solo: false };

interface Strip {
  id: number;
  /** 轨道名（用户数据，原样显示）。 */
  name: string;
  /** 主输出这类**内置条**的名字走 i18n（有 nameKey 就用它）。 */
  nameKey?: string;
  mix: MixerState;
  hue: number | null;
  instrument: boolean;
  master: boolean;
}

/** 推子的显示名：内置条翻译，轨道条用原文。 */
function stripName(s: Strip): string {
  return s.nameKey ? i18n.t(s.nameKey) : s.name;
}

const tracks = computed(() => store.project?.tracks ?? []);
const masterMix = computed<MixerState>(() => store.project?.master ?? MASTER_DEFAULT);

const strips = computed<Strip[]>(() => [
  { id: MASTER, name: "", nameKey: "mix.master", mix: masterMix.value, hue: null, instrument: true, master: true },
  ...tracks.value.map((t, i) => ({
    id: t.id,
    name: t.name,
    mix: t.mixer,
    hue: trackHue(i),
    instrument: Boolean(t.instrument),
    master: false,
  })),
]);

const anySolo = computed(() => tracks.value.some((t) => t.mixer.solo));

/** 被独奏压掉的轨道：视觉上变暗，与听感一致。 */
function dimmed(s: Strip): boolean {
  return anySolo.value && !s.master && !s.mix.solo && !s.mix.mute;
}

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
});
onUnmounted(() => {
  window.removeEventListener("pointerup", endGainDrag);
  window.removeEventListener("pointercancel", endGainDrag);
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
        <button
          class="strip-name text-label-md"
          :aria-label="s.master ? i18n.t('mix.master') : i18n.t('mix.selectNamed', { name: s.name })"
          @click="s.master ? selection.clear() : selection.selectTrack(s.id)"
        >
          {{ stripName(s) }}
        </button>

        <div v-if="!s.master" class="pan-row">
          <span class="pan-label">PAN</span>
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
          <span class="pan-value tabular">{{ shownPan(s).toFixed(2) }}</span>
        </div>

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

        <span class="strip-gain text-label-sm tabular">{{ shownGain(s).toFixed(1) }}</span>

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

        <span
          class="strip-tag"
          :class="{ on: s.instrument && !s.master, master: s.master }"
          :style="s.hue !== null ? { '--track-hue': String(s.hue) } : undefined"
        >
          {{ s.master ? i18n.t("mix.masterOut") : s.instrument ? i18n.t("mix.instrument") : i18n.t("mix.empty") }}
        </span>
      </div>

      <p v-if="tracks.length === 0" class="mix-empty text-body-sm">
        还没有音轨——新增轨道后会出现在右侧。
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
  gap: 10px;
  padding: 14px;
  min-width: 100%;
  box-sizing: border-box;
}
.strip {
  flex: 0 0 124px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 12px 10px;
  border-radius: 22px;
  background-color: var(--app-inset);
  border: 1px solid rgb(var(--md-outline-variant) / 0.35);
  transition: border-color 0.15s ease, opacity 0.15s ease;
}
.strip.selected {
  border-color: rgb(var(--md-primary) / 0.65);
}
.strip.dimmed {
  opacity: 0.55;
}
.strip.master {
  background-color: rgb(var(--md-primary-container) / 0.55);
  border-color: rgb(var(--md-primary) / 0.5);
}
.strip-name {
  max-width: 100%;
  border: 0;
  padding: 2px 8px;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  cursor: pointer;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font: inherit;
}
.strip-name:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
/* 声像：小号自绘滑块 + 数值 */
.pan-row {
  width: 100%;
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 5px;
}
.pan-label {
  font-size: 8.5px;
  letter-spacing: 0.6px;
  color: rgb(var(--md-on-surface-variant));
}
.pan-slider {
  height: 18px;
}
.pan-value {
  font-size: 9px;
  color: rgb(var(--md-on-surface-variant));
  min-width: 24px;
  text-align: right;
}
/* 推子：自绘轨道 + 已推部分 + 手柄，随高度拉伸；可拖动 */
.fader {
  position: relative;
  width: 22px;
  flex: 1 1 auto;
  min-height: 90px;
  display: flex;
  justify-content: center;
  cursor: ns-resize;
  touch-action: none;
  outline: none;
}
.fader:focus-visible {
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.5);
  border-radius: 999px;
}
.fader-rail {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-on-surface) / 0.14);
}
.fader-fill {
  position: absolute;
  bottom: 0;
  width: 6px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary) / 0.55);
}
.fader-knob {
  position: absolute;
  width: 26px;
  height: 12px;
  margin-bottom: -6px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  box-shadow: var(--md-elevation-1);
}
.strip.master .fader-knob {
  background-color: rgb(var(--md-on-primary-container));
}
.strip-gain {
  color: rgb(var(--md-on-surface-variant));
  font-variant-numeric: tabular-nums;
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
.mini:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.strip-tag {
  font-size: 10px;
  padding: 3px 10px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
  white-space: nowrap;
}
.strip-tag.on {
  background-color: hsl(var(--track-hue, 0) 45% 34%);
  color: hsl(var(--track-hue, 0) 70% 90%);
}
.strip-tag.master {
  background-color: rgb(var(--md-primary) / 0.22);
  color: rgb(var(--md-on-primary-container));
}
.tabular {
  font-variant-numeric: tabular-nums;
}
.mix-empty {
  align-self: center;
  padding: 0 8px;
  color: rgb(var(--md-on-surface-muted));
}
@media (prefers-reduced-motion: reduce) {
  .strip,
  .mini {
    transition: none;
  }
}
</style>
