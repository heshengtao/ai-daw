<script setup lang="ts">
/**
 * 高级 MIDI：能力检测、MPE、原始事件表。
 *
 * 这是「给高级用户 / AI 代理」的一层，藏在「高级 MIDI」后面；新手只需要摘要卡和配置向导。
 * 三块内容都走同一套 MD3 卡片 + 行（`.m3-card` / `.m3-row`），
 * 开关、下拉、步进器的位置在三块之间保持一致。
 */
import { computed, reactive, ref, watch } from "vue";
import { PlusIcon, TrashIcon } from "@heroicons/vue/24/outline";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import NumberField from "@/components/common/NumberField.vue";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import * as api from "@/api/tauri";
import type { MidiCapabilities, MidiEvent, MidiEventAt, Track } from "@/api/types";

const props = defineProps<{ track: Track }>();

const store = useProjectStore();
const layout = useLayoutStore();
const i18n = useI18n();

// ---------------------------------------------------------------------------
// 能力检测
// ---------------------------------------------------------------------------

const caps = ref<MidiCapabilities | null>(null);
const capsLoading = ref(false);

async function refreshCaps(): Promise<void> {
  capsLoading.value = true;
  try {
    caps.value = await api.trackMidiCapabilities(props.track.id);
  } catch (err) {
    layout.notify(String(err));
  } finally {
    capsLoading.value = false;
  }
}

const badges = computed(() => {
  const transport = caps.value?.transport;
  if (!transport) return [];
  return [
    { label: "MIDI IN", on: transport.midi_input },
    { label: "Bend", on: transport.channel_pitch_bend },
    { label: "Aftertouch", on: transport.channel_pressure },
    { label: "Poly AT", on: transport.poly_pressure },
    { label: "Program", on: transport.program_change },
    { label: "Per-note ch", on: transport.per_note_channel },
    { label: "NE Tuning", on: transport.note_expression.tuning },
    { label: "NE Volume", on: transport.note_expression.volume },
  ];
});

// ---------------------------------------------------------------------------
// MPE
// ---------------------------------------------------------------------------

const mpeZone = ref<"lower" | "upper">("lower");
const mpeChannels = ref(15);
const mpeZoneOptions = computed<SelectOption[]>(() => [
  { id: "lower", label: i18n.t("midi.mpeZoneLower") },
  { id: "upper", label: i18n.t("midi.mpeZoneUpper") },
]);

const mpeOn = computed(() => !!props.track.meta?.mpe);

watch(
  // 返回**字符串**（而不是数组）：watch 按引用比较，数组每次求值都是新对象，
  // 工程快照一刷新就会误触发；拼接成字符串后内容不变就不会回调。
  () => [props.track.meta?.mpe?.zone, props.track.meta?.mpe?.channels].join(":"),
  (signature) => {
    const [zone, channels] = signature.split(":");
    if (!zone || !channels) return;
    mpeZone.value = zone === "upper" ? "upper" : "lower";
    mpeChannels.value = Number(channels);
  },
  { immediate: true },
);

async function applyMpe(): Promise<void> {
  try {
    await store.setMpe(props.track.id, { zone: mpeZone.value, channels: mpeChannels.value });
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

async function disableMpe(): Promise<void> {
  try {
    await store.setMpe(props.track.id, null);
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

function toggleMpe(on: boolean): void {
  void (on ? applyMpe() : disableMpe());
}

// ---------------------------------------------------------------------------
// 通用 MIDI 事件表
// ---------------------------------------------------------------------------

type EventKind = "cc" | "program" | "channelPressure" | "polyPressure" | "noteExpression";

interface EventRow {
  beat: number;
  channel: number;
  kind: EventKind;
  a: number;
  b: number;
}

const eventKindOptions = computed<SelectOption[]>(() => [
  { id: "cc", label: i18n.t("midi.event.cc") },
  { id: "program", label: i18n.t("midi.event.program") },
  { id: "channelPressure", label: i18n.t("midi.event.channelPressure") },
  { id: "polyPressure", label: i18n.t("midi.event.polyPressure") },
  { id: "noteExpression", label: i18n.t("midi.event.noteExpression") },
]);

const eventRows = reactive<EventRow[]>([]);
/** 表里不认识的类型（例如手写的 NoteOn）原样保留，应用时追加回去。 */
const unknownEvents = ref<MidiEventAt[]>([]);

function classify(event: MidiEvent): EventKind | null {
  if ("ControlChange" in event) return "cc";
  if ("ProgramChange" in event) return "program";
  if ("ChannelPressure" in event) return "channelPressure";
  if ("PolyPressure" in event) return "polyPressure";
  if ("NoteExpression" in event) return "noteExpression";
  return null;
}

function syncEvents(track: Track): void {
  const list = track.meta?.midi_events ?? [];
  const rows: EventRow[] = [];
  const unknown: MidiEventAt[] = [];
  for (const item of list) {
    const kind = classify(item.event);
    if (!kind) {
      unknown.push(item);
      continue;
    }
    const row: EventRow = { beat: item.beat, channel: item.channel ?? 0, kind, a: 0, b: 0 };
    if (kind === "cc" && "ControlChange" in item.event) {
      row.a = item.event.ControlChange.controller;
      row.b = item.event.ControlChange.value;
    } else if (kind === "program" && "ProgramChange" in item.event) {
      row.a = item.event.ProgramChange.program;
    } else if (kind === "channelPressure" && "ChannelPressure" in item.event) {
      row.a = item.event.ChannelPressure.value;
    } else if (kind === "polyPressure" && "PolyPressure" in item.event) {
      row.a = item.event.PolyPressure.note;
      row.b = item.event.PolyPressure.value;
    } else if (kind === "noteExpression" && "NoteExpression" in item.event) {
      row.a = item.event.NoteExpression.note;
      row.b = Math.round(item.event.NoteExpression.value * 127);
    }
    rows.push(row);
  }
  eventRows.splice(0, eventRows.length, ...rows);
  unknownEvents.value = unknown;
}

watch(
  // 用内容的 JSON 签名 + 轨道 id 拼成**字符串**：工程快照每次刷新都会重建事件数组，
  // 但内容没变时不该清空用户刚添加、还没点「应用」的条目。
  () => `${props.track.id}:${JSON.stringify(props.track.meta?.midi_events ?? [])}`,
  () => syncEvents(props.track),
  { immediate: true },
);

function addEvent(): void {
  eventRows.push({ beat: 0, channel: 0, kind: "cc", a: 64, b: 127 });
}

function removeEvent(index: number): void {
  eventRows.splice(index, 1);
}

function buildEvent(row: EventRow): MidiEvent {
  switch (row.kind) {
    case "program":
      return { ProgramChange: { program: row.a } };
    case "channelPressure":
      return { ChannelPressure: { value: row.a } };
    case "polyPressure":
      return { PolyPressure: { note: row.a, value: row.b } };
    case "noteExpression":
      return {
        NoteExpression: {
          note: row.a,
          kind: "brightness",
          value: Math.max(0, Math.min(1, row.b / 127)),
        },
      };
    case "cc":
    default:
      return { ControlChange: { controller: row.a, value: row.b } };
  }
}

/** `a` 的含义随事件类型变化：控制器号 / 音高 / 音色号 / 压力。 */
function valueALabel(kind: EventKind): string {
  switch (kind) {
    case "cc":
      return i18n.t("midi.controller");
    case "program":
      return i18n.t("midi.program");
    case "channelPressure":
      return i18n.t("midi.pressure");
    case "polyPressure":
    case "noteExpression":
      return i18n.t("midi.note");
    default:
      return i18n.t("midi.valueA");
  }
}

const needsValueB = (kind: EventKind): boolean =>
  kind === "cc" || kind === "polyPressure" || kind === "noteExpression";

async function applyEvents(): Promise<void> {
  const events: MidiEventAt[] = [
    ...eventRows.map((row) => ({
      beat: row.beat,
      channel: row.channel,
      event: buildEvent(row),
    })),
    ...unknownEvents.value,
  ];
  try {
    await store.setMidiEvents(props.track.id, events);
    layout.notify(i18n.t("midi.eventsApplied", { count: events.length }));
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}
</script>

<template>
  <div class="pane">
    <!-- 兼容性 -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("midi.capabilities") }}</span>
          <span class="m3-card-sub">{{ i18n.t("midi.capabilitiesHint") }}</span>
        </div>
        <button class="m3-btn text" type="button" :disabled="capsLoading" @click="refreshCaps">
          {{ capsLoading ? i18n.t("midi.checking") : i18n.t("midi.check") }}
        </button>
      </div>

      <div v-if="badges.length" class="chips">
        <span v-for="badge in badges" :key="badge.label" class="chip" :class="{ off: !badge.on }">
          {{ badge.label }}
        </span>
      </div>
      <p v-else class="m3-hint">{{ i18n.t("midi.checkFirst") }}</p>

      <ul v-if="caps?.degraded?.length" class="notice">
        <li v-for="(message, index) in caps.degraded" :key="index">{{ message }}</li>
      </ul>
      <p v-if="caps?.note" class="m3-hint">{{ caps.note }}</p>
    </section>

    <!-- MPE -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("midi.mpe") }}</span>
          <span class="m3-card-sub">{{ i18n.t("midi.mpeHint") }}</span>
        </div>
        <Md3Switch
          :model-value="mpeOn"
          :label="mpeOn ? i18n.t('midi.mpeDisable') : i18n.t('midi.mpeEnable')"
          @update:model-value="toggleMpe"
        />
      </div>

      <template v-if="mpeOn">
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("midi.mpeZone") }}</span>
          </div>
          <div class="m3-row-control select-control">
            <Md3Select
              v-model="mpeZone"
              :options="mpeZoneOptions"
              :label="i18n.t('midi.mpeZone')"
            />
          </div>
        </div>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("midi.mpeChannels") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="mpeChannels"
              :sr-label="i18n.t('midi.mpeChannels')"
              :min="1"
              :max="15"
              :step="1"
            />
          </div>
        </div>

        <div class="m3-card-actions">
          <button class="m3-btn tonal" type="button" @click="applyMpe">
            {{ i18n.t("midi.apply") }}
          </button>
        </div>
      </template>
    </section>

    <!-- 事件表 -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("midi.events") }}</span>
          <span class="m3-card-sub">{{ i18n.t("midi.eventsHint") }}</span>
        </div>
        <button class="m3-btn text" type="button" @click="addEvent">
          <PlusIcon class="icon-svg" />
          {{ i18n.t("midi.addEvent") }}
        </button>
      </div>

      <p v-if="eventRows.length === 0" class="m3-hint empty">{{ i18n.t("midi.empty") }}</p>

      <div v-for="(row, index) in eventRows" :key="index" class="event">
        <div class="event-head">
          <div class="event-kind">
            <Md3Select
              v-model="row.kind"
              :options="eventKindOptions"
              :label="i18n.t('midi.type')"
            />
          </div>
          <button
            class="m3-icon-btn danger"
            type="button"
            :aria-label="i18n.t('midi.remove')"
            @click="removeEvent(index)"
          >
            <TrashIcon class="icon-svg" />
          </button>
        </div>

        <div class="event-grid">
          <label class="m3-field">
            <span class="m3-field-label">{{ i18n.t("midi.beat") }}</span>
            <NumberField
              v-model="row.beat"
              :sr-label="i18n.t('midi.beat')"
              :min="0"
              :max="9999"
              :step="0.25"
              :decimals="3"
            />
          </label>
          <label class="m3-field">
            <span class="m3-field-label">{{ i18n.t("midi.channel") }}</span>
            <NumberField
              v-model="row.channel"
              :sr-label="i18n.t('midi.channel')"
              :min="0"
              :max="15"
              :step="1"
            />
          </label>
          <label class="m3-field">
            <span class="m3-field-label">{{ valueALabel(row.kind) }}</span>
            <NumberField
              v-model="row.a"
              :sr-label="valueALabel(row.kind)"
              :min="0"
              :max="127"
              :step="1"
            />
          </label>
          <label v-if="needsValueB(row.kind)" class="m3-field">
            <span class="m3-field-label">{{ i18n.t("midi.value") }}</span>
            <NumberField
              v-model="row.b"
              :sr-label="i18n.t('midi.value')"
              :min="0"
              :max="127"
              :step="1"
            />
          </label>
        </div>
      </div>

      <div v-if="eventRows.length > 0" class="m3-card-actions">
        <button class="m3-btn filled" type="button" @click="applyEvents">
          {{ i18n.t("midi.apply") }}
        </button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 0 0 8px;
}
.chip {
  font-size: 10.5px;
  line-height: 20px;
  padding: 0 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
  white-space: nowrap;
}
.chip.off {
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-muted));
  text-decoration: line-through;
}
.notice {
  margin: 4px 0 8px;
  padding: 8px 10px;
  border-radius: 12px;
  background-color: rgb(var(--md-tertiary-container));
  color: rgb(var(--md-on-tertiary-container));
  font-size: 11px;
  line-height: 1.5;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.select-control {
  width: 200px;
}

.empty {
  padding: 8px 2px;
}

.event {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 8px 0;
  padding: 10px 12px;
  border-radius: 14px;
  background-color: rgb(var(--md-surface-container-high));
}
.event-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.event-kind {
  flex: 1 1 auto;
  min-width: 0;
}
.event-kind :deep(.md3-select) {
  max-width: 260px;
}
.event-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(104px, 1fr));
  gap: 10px;
}

.card-actions {
  display: flex;
  justify-content: flex-end;
}

.icon-svg {
  width: 15px;
  height: 15px;
}

@media (max-width: 560px) {
  .select-control {
    width: 150px;
  }
}
</style>
