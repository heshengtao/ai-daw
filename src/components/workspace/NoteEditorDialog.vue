<script setup lang="ts">
/**
 * 音符编辑模态框：扫弦 / 音高表达 / 逐音 MIDI。
 *
 * 三个标签页共用同一套 MD3 卡片 + 行布局（`.m3-card` / `.m3-row`）：
 * 左边是「这一项是什么」，右边是控件，需要解释的写在标题下面，不再把参数堆成网格。
 *
 * 音高一律按**半音**输入（0.5 = 四分之一音），音分只存在于引擎数据里；
 * 需要精确到音分时，0.01 个半音 = 1 音分，精度并没有丢失。
 */
import { computed, ref, watch } from "vue";
import ModalShell from "@/components/common/ModalShell.vue";
import NumberField from "@/components/common/NumberField.vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import PitchOffsetField from "@/components/common/PitchOffsetField.vue";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import { formatSemitones } from "@/utils/pitch";
import { firstLyricToken, splitLyrics } from "@/utils/lyrics";
import type { Note, NotePatch, PitchExpression } from "@/api/types";

const props = withDefaults(
  defineProps<{
    open: boolean;
    initialTab?: "strum" | "expr" | "midi" | "lyric";
  }>(),
  { initialTab: "expr" },
);

const emit = defineEmits<{ (e: "update:open", value: boolean): void }>();

const store = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const i18n = useI18n();

const tab = ref<"strum" | "expr" | "midi" | "lyric">("expr");

/** 音名（歌词行左侧的元信息）。 */
const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
function noteName(pitch: number): string {
  return `${NOTE_NAMES[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
}

watch(
  () => props.open,
  (open) => {
    if (open) tab.value = props.initialTab;
  },
  { immediate: true },
);

const pickedNotes = computed(() => {
  const c = selection.clip;
  if (!c) return [] as { index: number; id: number; note: Note }[];
  const out: { index: number; id: number; note: Note }[] = [];
  for (const index of selection.noteIndices) {
    const note = c.notes[index];
    if (note && typeof note.id === "number") out.push({ index, id: note.id, note });
  }
  return out;
});

function startKey(startBeat: number): string {
  return (Math.round(startBeat * 1e6) / 1e6).toFixed(6);
}

const clip = computed(() => selection.clip);
const track = computed(() => selection.track);

const tabs = computed<{ id: "strum" | "expr" | "midi" | "lyric"; label: string }[]>(() => [
  { id: "strum", label: i18n.t("noteEditor.tabStrum") },
  { id: "expr", label: i18n.t("noteEditor.tabExpr") },
  { id: "midi", label: i18n.t("noteEditor.tabMidi") },
  { id: "lyric", label: i18n.t("noteEditor.tabLyric") },
]);

// ---------------------------------------------------------------------------
// 扫弦
// ---------------------------------------------------------------------------

const strumMs = ref(28);
const strumDirection = ref("down");

const strumDirections = computed<SelectOption[]>(() => [
  { id: "down", label: i18n.t("inspector.strumDown") },
  { id: "up", label: i18n.t("inspector.strumUp") },
]);

/** 参与扫弦的音符：选中音符所在起点上的**全部**音符。 */
const strumGroup = computed(() => {
  const c = clip.value;
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

const offsetRange = computed(() => {
  const offsets = strumGroup.value.map((entry) => entry.note.offset ?? 0);
  if (offsets.length === 0) return null;
  return { min: Math.min(...offsets), max: Math.max(...offsets) };
});

const strumScope = computed(() => {
  if (strumGroup.value.length === 0) return i18n.t("noteEditor.noNotes");
  const bits = [i18n.t("inspector.strumScope", { count: strumGroup.value.length })];
  if (offsetRange.value) {
    bits.push(
      i18n.t("inspector.offsetRange", {
        min: offsetRange.value.min.toFixed(3),
        max: offsetRange.value.max.toFixed(3),
      }),
    );
  }
  return bits.join(" · ");
});

async function applyStrum(): Promise<void> {
  const c = clip.value;
  const t = track.value;
  if (!c || !t || strumGroup.value.length === 0) return;
  try {
    await store.strumNotes(t.id, c.id, strumMs.value, strumDirection.value as "down" | "up");
    layout.notify(i18n.t("inspector.strumApplied", { count: strumGroup.value.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 音高表达
// ---------------------------------------------------------------------------

const exprBend = ref(0);
const exprBendTo = ref(0);
const exprBendTime = ref(0);
const exprVibrato = ref(0);
const exprRate = ref(5.5);
const exprDelay = ref(0);
const exprFade = ref(0);

watch(
  // 返回**字符串**（而不是数组）：watch 按引用比较，数组每次求值都是新对象，
  // 工程快照刷新就会把用户还没应用的编辑冲掉。
  () => `${props.open}:${pickedNotes.value[0]?.id ?? -1}`,
  () => {
    if (!props.open) return;
    const expr = pickedNotes.value[0]?.note.expr;
    exprBend.value = expr?.bend ?? 0;
    exprBendTo.value = expr?.bend_to ?? 0;
    exprBendTime.value = expr?.bend_time ?? 0;
    exprVibrato.value = expr?.vibrato ?? 0;
    exprRate.value = expr?.vibrato_rate || 5.5;
    exprDelay.value = expr?.vibrato_delay ?? 0;
    exprFade.value = expr?.vibrato_fade ?? 0;
  },
  { immediate: true },
);

function exprPayload(): PitchExpression {
  return {
    bend: Math.round(exprBend.value),
    bend_to: Math.round(exprBendTo.value),
    bend_time: exprBendTime.value,
    vibrato: Math.round(exprVibrato.value),
    vibrato_rate: exprRate.value,
    vibrato_delay: exprDelay.value,
    vibrato_fade: exprFade.value,
  };
}

type Preset = "bend" | "prebend" | "vibrato" | "slideDown" | "clear";

const presets = computed<{ id: Preset; label: string; hint: string }[]>(() => [
  { id: "bend", label: i18n.t("noteEditor.presetBend"), hint: i18n.t("noteEditor.presetBendHint") },
  { id: "prebend", label: i18n.t("noteEditor.presetPrebend"), hint: i18n.t("noteEditor.presetPrebendHint") },
  { id: "vibrato", label: i18n.t("noteEditor.presetVibrato"), hint: i18n.t("noteEditor.presetVibratoHint") },
  { id: "slideDown", label: i18n.t("noteEditor.presetSlide"), hint: i18n.t("noteEditor.presetSlideHint") },
  { id: "clear", label: i18n.t("noteEditor.presetClear"), hint: i18n.t("noteEditor.presetClearHint") },
]);

function applyPreset(preset: Preset): void {
  switch (preset) {
    case "bend":
      exprBend.value = 0;
      exprBendTo.value = 200;
      exprBendTime.value = 0.25;
      exprVibrato.value = 0;
      break;
    case "prebend":
      exprBend.value = 200;
      exprBendTo.value = 200;
      exprBendTime.value = 0;
      exprVibrato.value = 35;
      exprRate.value = 5.5;
      exprDelay.value = 0.5;
      exprFade.value = 0.5;
      break;
    case "vibrato":
      exprBend.value = 0;
      exprBendTo.value = 0;
      exprBendTime.value = 0;
      exprVibrato.value = 35;
      exprRate.value = 5.5;
      exprDelay.value = 0.25;
      exprFade.value = 0.25;
      break;
    case "slideDown":
      exprBend.value = 0;
      exprBendTo.value = -200;
      exprBendTime.value = 0.3;
      exprVibrato.value = 0;
      break;
    case "clear":
      exprBend.value = 0;
      exprBendTo.value = 0;
      exprBendTime.value = 0;
      exprVibrato.value = 0;
      exprRate.value = 5.5;
      exprDelay.value = 0;
      exprFade.value = 0;
      break;
  }
}

/** 与引擎 `PitchExpression::cents_at` 同语义，用来画预览曲线。 */
function centsAt(expr: PitchExpression, t: number): number {
  const bendTime = expr.bend_time ?? 0;
  const from = expr.bend ?? 0;
  const to = expr.bend_to ?? 0;
  let cents =
    bendTime <= 0 ? to : from + (to - from) * Math.max(0, Math.min(1, t / bendTime));
  const vibrato = expr.vibrato ?? 0;
  if (vibrato !== 0) {
    const delay = Math.max(0, expr.vibrato_delay ?? 0);
    if (t >= delay) {
      const elapsed = t - delay;
      const fade =
        (expr.vibrato_fade ?? 0) > 0
          ? Math.max(0, Math.min(1, elapsed / (expr.vibrato_fade ?? 1)))
          : 1;
      const rate = (expr.vibrato_rate ?? 0) > 0 ? expr.vibrato_rate! : 5.5;
      cents += vibrato * fade * Math.sin(2 * Math.PI * rate * elapsed);
    }
  }
  return cents;
}

/** 预览长度：选中音符的时值（至少 1 拍）。 */
const previewBeats = computed(() =>
  Math.max(1, pickedNotes.value[0]?.note.duration ?? 2),
);

/** 预览曲线的 SVG path（viewBox 300×60）。 */
const previewPath = computed(() => {
  const expr = exprPayload();
  const samples = 96;
  const values: number[] = [];
  for (let i = 0; i < samples; i += 1) {
    values.push(centsAt(expr, (i / (samples - 1)) * previewBeats.value));
  }
  const maxAbs = Math.max(100, ...values.map((v) => Math.abs(v)));
  const scale = Math.ceil(maxAbs / 100) * 100;
  const mid = 30;
  const points = values.map((cents, i) => {
    const x = (i / (samples - 1)) * 300;
    const y = mid - (cents / scale) * 24;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  return `M ${points.join(" L ")}`;
});

const exprSummary = computed(() => {
  const expr = exprPayload();
  const bend = expr.bend ?? 0;
  const bendTo = expr.bend_to ?? 0;
  const bendTime = expr.bend_time ?? 0;
  const vibrato = expr.vibrato ?? 0;
  const vibratoRate = expr.vibrato_rate || 5.5;
  const vibratoDelay = expr.vibrato_delay ?? 0;
  const bits: string[] = [];
  if (bend !== bendTo || bend !== 0) {
    if (bend === bendTo) {
      bits.push(i18n.t("noteEditor.summaryPrebend", { semis: formatSemitones(bend) }));
    } else {
      bits.push(
        i18n.t("noteEditor.summaryBend", {
          from: formatSemitones(bend),
          to: formatSemitones(bendTo),
          time: bendTime,
        }),
      );
    }
  }
  if (vibrato) {
    bits.push(
      i18n.t("noteEditor.summaryVibrato", {
        semis: formatSemitones(vibrato),
        rate: vibratoRate,
        delay: vibratoDelay,
      }),
    );
  }
  return bits.length > 0 ? bits.join(i18n.t("noteEditor.summaryJoin")) : i18n.t("noteEditor.none");
});

async function applyExpr(): Promise<void> {
  const c = clip.value;
  const t = track.value;
  if (!c || !t || pickedNotes.value.length === 0) return;
  const expr = exprPayload();
  const patches: NotePatch[] = pickedNotes.value.map((entry) => ({ id: entry.id, expr }));
  try {
    await store.updateNotes(t.id, c.id, patches);
    layout.notify(i18n.t("inspector.exprApplied", { count: patches.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

async function clearExpr(): Promise<void> {
  const c = clip.value;
  const t = track.value;
  if (!c || !t || pickedNotes.value.length === 0) return;
  applyPreset("clear");
  const patches: NotePatch[] = pickedNotes.value.map((entry) => ({ id: entry.id, expr: {} }));
  try {
    await store.updateNotes(t.id, c.id, patches);
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// MIDI
// ---------------------------------------------------------------------------

const notePressure = ref(0);
const noteChannel = ref("auto");

const channelOptions = computed<SelectOption[]>(() => [
  { id: "auto", label: i18n.t("inspector.channelAuto") },
  ...Array.from({ length: 16 }, (_, i) => ({ id: String(i), label: String(i + 1) })),
]);

watch(
  // 同上：按「打开状态 + 音符 id」的字符串载入，避免快照刷新重置未应用的草稿。
  () => `${props.open}:${pickedNotes.value[0]?.id ?? -1}`,
  () => {
    if (!props.open) return;
    const note = pickedNotes.value[0]?.note;
    if (!note) return;
    notePressure.value = note.pressure ?? 0;
    noteChannel.value = note.channel == null ? "auto" : String(note.channel);
  },
  { immediate: true },
);

async function applyMidi(): Promise<void> {
  const c = clip.value;
  const t = track.value;
  if (!c || !t || pickedNotes.value.length === 0) return;
  const channel = noteChannel.value === "auto" ? undefined : Number(noteChannel.value);
  const patches: NotePatch[] = pickedNotes.value.map((entry) => ({
    id: entry.id,
    pressure: Math.round(notePressure.value),
    ...(channel === undefined ? {} : { channel }),
  }));
  try {
    await store.updateNotes(t.id, c.id, patches);
    layout.notify(i18n.t("inspector.midiApplied", { count: patches.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 歌词
// ---------------------------------------------------------------------------

/** 选中音符的歌词草稿（按 id 存，避免快照刷新冲掉未应用的编辑）。 */
const lyricDraft = ref<Record<number, string>>({});
/** 「按顺序填入」用的一行文本。 */
const lyricFill = ref("");

/** 按起点排序的选中音符（逐音填词就是按演唱顺序）。 */
const lyricRows = computed(() =>
  [...pickedNotes.value]
    .sort((a, b) => a.note.start - b.note.start)
    .map((entry) => ({
      id: entry.id,
      pitch: entry.note.pitch,
      start: entry.note.start,
      value: lyricDraft.value[entry.id] ?? entry.note.lyric ?? "",
    })),
);

watch(
  // 打开状态 + 选中音符 id 的字符串（不用数组，理由同上面几个 watcher）。
  () => `${props.open}:${pickedNotes.value.map((p) => p.id).join(",")}`,
  () => {
    if (!props.open) return;
    lyricFill.value = "";
    const next: Record<number, string> = {};
    for (const p of pickedNotes.value) next[p.id] = p.note.lyric ?? "";
    lyricDraft.value = next;
  },
  { immediate: true },
);

/** 把一行歌词按顺序填进选中音符（够不着的音符保持原样）。 */
function distributeLyrics(): void {
  const tokens = splitLyrics(lyricFill.value);
  if (tokens.length === 0) return;
  const rows = lyricRows.value;
  const next = { ...lyricDraft.value };
  for (let i = 0; i < rows.length; i += 1) next[rows[i].id] = tokens[i] ?? "";
  lyricDraft.value = next;
}

async function applyLyrics(): Promise<void> {
  const c = clip.value;
  const t = track.value;
  if (!c || !t || pickedNotes.value.length === 0) return;
  const patches: NotePatch[] = [];
  for (const entry of pickedNotes.value) {
    // 一个音只唱一个记号：多写的字由「按顺序填入」处理，逐音输入这里只取第一个。
    const next = firstLyricToken(lyricDraft.value[entry.id] ?? "");
    const prev = entry.note.lyric ?? "";
    if (next !== prev) patches.push({ id: entry.id, lyric: next });
  }
  if (patches.length === 0) return;
  try {
    await store.updateNotes(t.id, c.id, patches);
    layout.notify(i18n.t("noteEditor.lyricApplied", { count: patches.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

// ---------------------------------------------------------------------------
// 应用当前标签页
// ---------------------------------------------------------------------------

async function applyActive(): Promise<void> {
  if (tab.value === "strum") await applyStrum();
  else if (tab.value === "expr") await applyExpr();
  else if (tab.value === "lyric") await applyLyrics();
  else await applyMidi();
  emit("update:open", false);
}
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('noteEditor.title')"
    width="680px"
    @update:open="emit('update:open', $event)"
  >
    <template #toolbar>
      <div class="tabs-bar">
        <div class="m3-seg" role="tablist" :aria-label="i18n.t('noteEditor.title')">
          <button
            v-for="item in tabs"
            :key="item.id"
            class="m3-seg-btn"
            :class="{ on: tab === item.id }"
            type="button"
            role="tab"
            :aria-selected="tab === item.id"
            @click="tab = item.id"
          >
            {{ item.label }}
          </button>
        </div>
      </div>
    </template>

    <!-- 扫弦 -->
    <div v-if="tab === 'strum'" class="m3-dialog-body">
      <p class="m3-hint">{{ i18n.t("noteEditor.strumLead") }}</p>

      <section class="m3-card">
        <div class="m3-card-head">
          <span class="m3-card-title">{{ i18n.t("noteEditor.tabStrum") }}</span>
          <span class="m3-card-sub">{{ strumScope }}</span>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.strum") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.strumIntervalHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="strumMs"
              :sr-label="i18n.t('inspector.strum')"
              :min="0"
              :max="500"
              :step="1"
            />
            <span class="unit">ms</span>
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.strumDirection") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.strumDirectionHint") }}</span>
          </div>
          <div class="m3-row-control select-control">
            <Md3Select
              v-model="strumDirection"
              :options="strumDirections"
              :label="i18n.t('inspector.strumDirection')"
            />
          </div>
        </div>
      </section>
    </div>

    <!-- 音高表达 -->
    <div v-else-if="tab === 'expr'" class="m3-dialog-body">
      <p class="m3-hint">{{ i18n.t("noteEditor.exprLead") }}</p>

      <div class="presets">
        <button
          v-for="preset in presets"
          :key="preset.id"
          class="m3-chip"
          type="button"
          :title="preset.hint"
          @click="applyPreset(preset.id)"
        >
          {{ preset.label }}
        </button>
      </div>

      <section class="m3-card">
        <div class="m3-card-head">
          <div class="m3-row-main">
            <span class="m3-card-title">{{ i18n.t("noteEditor.bendGroup") }}</span>
            <span class="m3-card-sub">{{ i18n.t("noteEditor.bendHint") }}</span>
          </div>
          <span class="m3-card-sub head-summary">{{ exprSummary }}</span>
        </div>

        <div class="preview">
          <svg viewBox="0 0 300 60" preserveAspectRatio="none" aria-hidden="true">
            <line x1="0" y1="30" x2="300" y2="30" class="baseline" />
            <path :d="previewPath" class="curve" />
          </svg>
          <div class="preview-axis">
            <span>{{ i18n.t("noteEditor.previewGroup") }}</span>
            <span>0 – {{ previewBeats.toFixed(2) }} {{ i18n.t("noteEditor.beats") }}</span>
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprBend") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.bendStartHint") }}</span>
          </div>
          <div class="m3-row-control">
            <PitchOffsetField
              v-model="exprBend"
              :label="i18n.t('inspector.exprBend')"
              :step="0.25"
            />
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprBendTo") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.bendTargetHint") }}</span>
          </div>
          <div class="m3-row-control">
            <PitchOffsetField
              v-model="exprBendTo"
              :label="i18n.t('inspector.exprBendTo')"
              :step="0.25"
            />
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprBendTime") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.bendTimeHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="exprBendTime"
              :sr-label="i18n.t('inspector.exprBendTime')"
              :min="0"
              :max="8"
              :step="0.0625"
              :decimals="3"
            />
            <span class="unit">{{ i18n.t("noteEditor.beats") }}</span>
          </div>
        </div>
      </section>

      <section class="m3-card">
        <div class="m3-card-head">
          <span class="m3-card-title">{{ i18n.t("noteEditor.vibratoGroup") }}</span>
          <span class="m3-card-sub">{{ i18n.t("noteEditor.vibratoHint") }}</span>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprVibrato") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.vibratoDepthHint") }}</span>
          </div>
          <div class="m3-row-control">
            <PitchOffsetField
              v-model="exprVibrato"
              :min="0"
              :max="1200"
              :step="0.05"
              :label="i18n.t('inspector.exprVibrato')"
            />
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprRate") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.vibratoRateHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="exprRate"
              :sr-label="i18n.t('inspector.exprRate')"
              :min="0"
              :max="20"
              :step="0.1"
              :decimals="2"
            />
            <span class="unit">Hz</span>
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprDelay") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.vibratoDelayHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="exprDelay"
              :sr-label="i18n.t('inspector.exprDelay')"
              :min="0"
              :max="8"
              :step="0.0625"
              :decimals="3"
            />
            <span class="unit">{{ i18n.t("noteEditor.beats") }}</span>
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.exprFade") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.vibratoFadeHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="exprFade"
              :sr-label="i18n.t('inspector.exprFade')"
              :min="0"
              :max="8"
              :step="0.0625"
              :decimals="3"
            />
            <span class="unit">{{ i18n.t("noteEditor.beats") }}</span>
          </div>
        </div>
      </section>
    </div>

    <!-- 歌词（歌声合成） -->
    <div v-else-if="tab === 'lyric'" class="m3-dialog-body">
      <p class="m3-hint">{{ i18n.t("noteEditor.lyricLead") }}</p>

      <section class="m3-card">
        <div class="m3-card-head">
          <span class="m3-card-title">{{ i18n.t("noteEditor.tabLyric") }}</span>
          <span class="m3-card-sub">{{ i18n.t("noteEditor.lyricScope", { count: lyricRows.length }) }}</span>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("noteEditor.lyricFill") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.lyricFillHint") }}</span>
          </div>
          <div class="m3-row-control lyric-fill">
            <input
              v-model="lyricFill"
              class="lyric-input"
              type="text"
              spellcheck="false"
              :placeholder="i18n.t('noteEditor.lyricFillPlaceholder')"
              @keydown.enter.prevent="distributeLyrics"
            />
            <button class="m3-btn text" type="button" @click="distributeLyrics">
              {{ i18n.t("noteEditor.lyricFillAction") }}
            </button>
          </div>
        </div>
      </section>

      <section class="m3-card">
        <div class="m3-card-head">
          <span class="m3-card-title">{{ i18n.t("noteEditor.lyricPerNote") }}</span>
          <span class="m3-card-sub">{{ i18n.t("noteEditor.lyricPerNoteHint") }}</span>
        </div>
        <div class="lyric-list">
          <div v-for="row in lyricRows" :key="row.id" class="lyric-row">
            <span class="lyric-meta">{{ noteName(row.pitch) }} · {{ row.start.toFixed(2) }}</span>
            <input
              v-model="lyricDraft[row.id]"
              class="lyric-input"
              type="text"
              spellcheck="false"
              maxlength="24"
              :placeholder="i18n.t('noteEditor.lyricPlaceholder')"
            />
          </div>
          <p v-if="lyricRows.length === 0" class="m3-hint">{{ i18n.t("noteEditor.noNotes") }}</p>
        </div>
      </section>

      <p class="m3-hint">{{ i18n.t("noteEditor.lyricTip") }}</p>
    </div>

    <!-- MIDI -->
    <div v-else class="m3-dialog-body">
      <p class="m3-hint">{{ i18n.t("noteEditor.midiLead") }}</p>

      <section class="m3-card">
        <div class="m3-card-head">
          <span class="m3-card-title">{{ i18n.t("noteEditor.tabMidi") }}</span>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.pressure") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.pressureHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="notePressure"
              :sr-label="i18n.t('inspector.pressure')"
              :min="0"
              :max="127"
              :step="1"
            />
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("inspector.channel") }}</span>
            <span class="m3-row-sub">{{ i18n.t("noteEditor.channelHint") }}</span>
          </div>
          <div class="m3-row-control select-control">
            <Md3Select
              v-model="noteChannel"
              :options="channelOptions"
              :label="i18n.t('inspector.channel')"
            />
          </div>
        </div>
      </section>

      <p class="m3-hint">{{ i18n.t("inspector.midiHint") }}</p>
    </div>

    <template #foot>
      <div class="m3-dialog-foot">
        <button
          v-if="tab === 'expr'"
          class="m3-btn text"
          type="button"
          @click="clearExpr"
        >
          {{ i18n.t("inspector.exprClear") }}
        </button>
        <span class="spacer" />
        <button class="m3-btn text" type="button" @click="emit('update:open', false)">
          {{ i18n.t("common.cancel") }}
        </button>
        <button class="m3-btn filled" type="button" @click="applyActive">
          {{ i18n.t("noteEditor.apply") }}
        </button>
      </div>
    </template>
  </ModalShell>
</template>

<style scoped>
.tabs-bar {
  padding: 0 16px 12px;
}

/* 单位跟在步进器后面 */
.unit {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}

.select-control {
  width: 210px;
}

/* 歌词：与面板其它输入保持同一密度。 */
.lyric-fill {
  display: flex;
  align-items: center;
  gap: 8px;
}
.lyric-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: 240px;
  overflow-y: auto;
}
.lyric-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.lyric-meta {
  flex: 0 0 96px;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  font-variant-numeric: tabular-nums;
}
.lyric-input {
  flex: 1 1 auto;
  min-width: 0;
  padding: 6px 9px;
  border: none;
  border-radius: 8px;
  outline: none;
  font-family: inherit;
  font-size: 12px;
  color: rgb(var(--md-on-surface));
  background-color: rgb(var(--md-surface-container-high) / 0.7);
}
.lyric-input:focus {
  box-shadow: inset 0 0 0 1px rgb(var(--md-primary) / 0.6);
}

.presets {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.head-summary {
  max-width: 52%;
  text-align: right;
}

.preview {
  padding: 8px 10px 6px;
  border-radius: 12px;
  background-color: rgb(var(--md-surface-container-lowest));
}
.preview svg {
  display: block;
  width: 100%;
  height: 62px;
}
.baseline {
  stroke: rgb(var(--md-outline-variant));
  stroke-dasharray: 3 3;
  stroke-width: 1;
}
.curve {
  fill: none;
  stroke: rgb(var(--md-primary));
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
}
.preview-axis {
  display: flex;
  justify-content: space-between;
  font-size: 10px;
  color: rgb(var(--md-on-surface-muted));
}

.spacer {
  flex: 1 1 auto;
}

@media (max-width: 560px) {
  .select-control {
    width: 160px;
  }
}
</style>
