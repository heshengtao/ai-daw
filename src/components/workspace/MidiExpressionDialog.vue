<script setup lang="ts">
/**
 * 「演奏技巧」配置向导。
 *
 * 新手只需要回答两个问题：**要哪个技巧** + **插件怎么触发它**。
 * 原始 MIDI 事件、MPE、能力检测都在「高级 MIDI」标签页里，默认不出现。
 *
 * 布局与音符设置共用同一套 MD3 卡片 / 行（`.m3-card` / `.m3-row`）：
 * 左边是技巧清单，右边是「这个技巧」的卡片，参数不再散落成一堆输入框。
 *
 * 这里仍然不内置任何厂商键位——「按一个键」是让用户按插件手册选一个音，
 * 不替用户猜。
 */
import { computed, reactive, ref, watch } from "vue";
import { PlusIcon } from "@heroicons/vue/24/outline";
import ModalShell from "@/components/common/ModalShell.vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import NumberField from "@/components/common/NumberField.vue";
import MidiAdvancedPane from "@/components/workspace/MidiAdvancedPane.vue";
import { useProjectStore } from "@/stores/project";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import * as api from "@/api/tauri";
import type {
  ArticulationEvent,
  ArticulationMapSummary,
  ArticulationSwitch,
  MidiEvent,
  Track,
} from "@/api/types";

const props = withDefaults(
  defineProps<{
    open: boolean;
    track: Track;
    initialTab?: "techniques" | "library" | "advanced";
  }>(),
  { initialTab: "techniques" },
);

const emit = defineEmits<{ (e: "update:open", value: boolean): void }>();

const store = useProjectStore();
const layout = useLayoutStore();
const i18n = useI18n();

type Tab = "techniques" | "library" | "advanced";

const tab = ref<Tab>("techniques");

const tabs = computed<{ id: Tab; label: string }[]>(() => [
  { id: "techniques", label: i18n.t("midi.studioTechniques") },
  { id: "library", label: i18n.t("midi.studioLibrary") },
  { id: "advanced", label: i18n.t("midi.studioAdvanced") },
]);

// ---------------------------------------------------------------------------
// 技巧草稿
// ---------------------------------------------------------------------------

type Trigger = "keyswitch" | "cc" | "program" | "events";

interface TechniqueRow {
  name: string;
  trigger: Trigger;
  /** keyswitch 音高 / program 号；`null` = 还没选。 */
  value: number | null;
  ccController: number;
  ccValue: number;
  /** 导入的多消息序列（只读展示；保存时原样写回）。 */
  events?: ArticulationEvent[];
  /** 提前量（拍）；0 = 引擎默认 1/64 拍。 */
  lead: number;
  momentary: boolean;
}

const rows = reactive<TechniqueRow[]>([]);
const selected = ref(-1);
const defaultTechnique = ref("");
const newName = ref("");

const SUGGESTIONS = computed(() => [
  i18n.t("midi.suggest.harmonics"),
  i18n.t("midi.suggest.mute"),
  i18n.t("midi.suggest.slide"),
  i18n.t("midi.suggest.hammer"),
  i18n.t("midi.suggest.legato"),
  i18n.t("midi.suggest.staccato"),
]);

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

function noteLabel(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 1} (${note})`;
}

const noteOptions = computed<SelectOption[]>(() => {
  const out: SelectOption[] = [{ id: "", label: i18n.t("midi.notSet") }];
  for (let note = 0; note < 128; note += 1) {
    out.push({
      id: String(note),
      label: noteLabel(note),
      group: `C${Math.floor(note / 12) - 1}`,
    });
  }
  return out;
});

const triggerLabels = computed<Record<Trigger, string>>(() => ({
  keyswitch: i18n.t("midi.triggerKeyswitch"),
  cc: i18n.t("midi.triggerCc"),
  program: i18n.t("midi.triggerProgram"),
  events: i18n.t("midi.triggerEvents"),
}));

/** 手动配置只能选这三种；导入的映射行不参与这个选择。 */
const triggerOptions = computed<SelectOption[]>(() => [
  { id: "keyswitch", label: i18n.t("midi.triggerKeyswitch") },
  { id: "cc", label: i18n.t("midi.triggerCc") },
  { id: "program", label: i18n.t("midi.triggerProgram") },
]);

function fromSwitch(name: string, value: ArticulationSwitch): TechniqueRow {
  const events = value.events ?? [];
  const trigger: Trigger =
    events.length > 0
      ? "events"
      : value.keyswitch != null
        ? "keyswitch"
        : value.cc != null
          ? "cc"
          : "program";
  return {
    name,
    trigger,
    value: value.keyswitch ?? value.program ?? null,
    ccController: value.cc?.controller ?? 1,
    ccValue: value.cc?.value ?? 0,
    events: events.length ? events : undefined,
    lead: value.lead_beats ?? 0,
    momentary: value.momentary ?? false,
  };
}

function sync(track: Track): void {
  const map = track.meta?.articulations ?? {};
  rows.splice(0, rows.length, ...Object.entries(map).map(([name, value]) => fromSwitch(name, value)));
  defaultTechnique.value = track.meta?.default_articulation ?? "";
  selected.value = rows.length > 0 ? 0 : -1;
}

watch(
  // 拼成字符串再比较：watch 对数组按引用比较，每次工程快照刷新都会误触发，
  // 把用户还没保存的技巧草稿重置掉。
  () =>
    `${props.track.id}:${JSON.stringify(props.track.meta?.articulations ?? {})}:${
      props.track.meta?.default_articulation ?? ""
    }`,
  () => sync(props.track),
  { immediate: true },
);

const current = computed(() => (selected.value >= 0 ? rows[selected.value] : null));

function triggerLabel(trigger: Trigger): string {
  return triggerLabels.value[trigger] ?? trigger;
}

function addRow(name = ""): void {
  rows.push({
    name,
    trigger: "keyswitch",
    value: null,
    ccController: 1,
    ccValue: 0,
    lead: 0,
    momentary: false,
  });
  selected.value = rows.length - 1;
}

async function removeRow(index: number): Promise<void> {
  const [row] = rows.splice(index, 1);
  if (!row) return;
  if (selected.value >= rows.length) selected.value = rows.length - 1;
  const ok = await persistMap(
    i18n.t("midi.articulationDeleted", {
      name: row.name || i18n.t("midi.untitled"),
    }),
  );
  if (!ok) sync(props.track);
}

function addSuggestion(name: string): void {
  if (rows.some((row) => row.name === name)) return;
  addRow(name);
}

function addCustom(): void {
  const name = newName.value.trim();
  if (!name) return;
  addSuggestion(name);
  newName.value = "";
}

function setTrigger(id: string): void {
  if (!current.value) return;
  current.value.trigger = id as Trigger;
  if (id !== "events") current.value.events = undefined;
}

/** 导入的切换消息压缩成给初学者看的一行摘要（隐藏自动补的 NoteOff）。 */
function eventSummary(row: TechniqueRow): string {
  return (row.events ?? [])
    .filter((item) => !("NoteOff" in item.event))
    .map((item) => {
      const event = item.event;
      if ("NoteOn" in event) {
        return `${i18n.t("midi.triggerKeyswitch")} ${noteLabel(event.NoteOn.note)}`;
      }
      if ("ControlChange" in event) {
        return `${i18n.t("midi.cc")} ${event.ControlChange.controller} = ${event.ControlChange.value}`;
      }
      if ("ProgramChange" in event) {
        return `${i18n.t("midi.triggerProgram")} ${event.ProgramChange.program}`;
      }
      if ("ChannelPressure" in event) return `触后 ${event.ChannelPressure.value}`;
      if ("PolyPressure" in event) {
        return `复音触后 ${event.PolyPressure.note} = ${event.PolyPressure.value}`;
      }
      return "MIDI";
    })
    .join(" · ");
}

/** 把导入行转成手动配置：清空序列，交给用户选按键 / CC / 音色。 */
function convertToManual(): void {
  const row = current.value;
  if (!row) return;
  row.events = undefined;
  row.trigger = "keyswitch";
  row.value = null;
}

// ---------------------------------------------------------------------------
// 映射库
// ---------------------------------------------------------------------------

const libraryQuery = ref("");
const libraryResults = ref<ArticulationMapSummary[]>([]);
const libraryMode = ref<"replace" | "merge">("replace");
const libraryLoading = ref(false);
const libraryError = ref("");
const confirmClear = ref(false);
const showGuide = ref(false);
const saving = ref(false);

const mapSource = computed(() => props.track.meta?.articulation_source);
const mapCount = computed(() => Object.keys(props.track.meta?.articulations ?? {}).length);
const hasMap = computed(() => mapCount.value > 0);
const sourceLabel = computed(() => {
  const source = mapSource.value;
  if (!source) return "";
  return [source.manufacturer, source.product, source.patch]
    .map((part) => part?.trim())
    .filter((part): part is string => Boolean(part))
    .join(" · ");
});

/** 清除整份映射：给“想重新导入”一个明确入口。 */
async function clearMap(): Promise<void> {
  try {
    await store.setArticulations(props.track.id, {}, undefined, undefined);
    confirmClear.value = false;
    showGuide.value = false;
    layout.notify(i18n.t("midi.mapCleared"));
    selectTab("library");
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  }
}

/** 工程里标了奏法、但当前映射里没有对应项的标签。 */
const unknownTags = computed(() => {
  const map = props.track.meta?.articulations ?? {};
  const tags = store.project?.note_tags ?? {};
  const unknown = new Set<string>();
  for (const [key, tag] of Object.entries(tags)) {
    if (!tag.articulation) continue;
    if (!key.startsWith(`${props.track.id}:`)) continue;
    if (!map[tag.articulation]) unknown.add(tag.articulation);
  }
  return [...unknown];
});

async function searchLibrary(): Promise<void> {
  libraryLoading.value = true;
  libraryError.value = "";
  try {
    libraryResults.value = await api.listArticulationMaps(libraryQuery.value, 100);
  } catch (err) {
    libraryError.value = String(err);
    libraryResults.value = [];
  } finally {
    libraryLoading.value = false;
  }
}

function selectTab(id: Tab): void {
  tab.value = id;
  confirmClear.value = false;
  if (id === "library" && libraryResults.value.length === 0) void searchLibrary();
}

async function importLibrary(entry: ArticulationMapSummary): Promise<void> {
  libraryLoading.value = true;
  try {
    await store.importArticulationMap(props.track.id, entry.id, libraryMode.value);
    layout.notify(
      i18n.t("midi.libraryImported", {
        patch: entry.patch,
        count: entry.articulation_count,
      }),
    );
    showGuide.value = false;
    tab.value = "techniques";
  } catch (err) {
    libraryError.value = String(err);
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
  } finally {
    libraryLoading.value = false;
  }
}

watch(
  () => props.open,
  (open) => {
    if (!open) return;
    tab.value = props.initialTab;
    if (tab.value === "library") void searchLibrary();
  },
  { immediate: true },
);

/** 把当前技巧草稿展开成要试听的 MIDI 事件。 */
function rowEvents(row: TechniqueRow): MidiEvent[] {
  if (row.trigger === "events") return (row.events ?? []).map((item) => item.event);
  if (row.trigger === "keyswitch") {
    return row.value == null ? [] : [{ NoteOn: { note: row.value, velocity: 1 } }];
  }
  if (row.trigger === "cc") {
    return [{ ControlChange: { controller: row.ccController, value: row.ccValue } }];
  }
  if (row.trigger === "program") {
    return row.value == null ? [] : [{ ProgramChange: { program: row.value } }];
  }
  return [];
}

/** 试听奏法：先发切换序列，再弹一个 C4 听结果（不写工程）。 */
async function audition(): Promise<void> {
  const row = current.value;
  if (!row) return;
  const events = rowEvents(row);
  if (events.length === 0) return;
  try {
    await api.auditionMidiEvents(props.track.id, events);
    window.setTimeout(() => void api.auditionNote(props.track.id, 60, 100, true), 80);
    window.setTimeout(() => void api.auditionNote(props.track.id, 60, 100, false), 560);
  } catch {
    // 试听失败不影响配置。
  }
}

/** 把当前草稿转成可写入工程的映射。 */
function buildMap(): Record<string, ArticulationSwitch> {
  const map: Record<string, ArticulationSwitch> = {};
  for (const row of rows) {
    const name = row.name.trim();
    if (!name) continue;
    if (row.trigger === "events" && !(row.events?.length ?? 0)) continue;
    if (row.trigger !== "cc" && row.trigger !== "events" && row.value == null) continue;
    map[name] = {
      ...(row.trigger === "keyswitch" ? { keyswitch: row.value! } : {}),
      ...(row.trigger === "cc"
        ? { cc: { controller: row.ccController, value: row.ccValue } }
        : {}),
      ...(row.trigger === "program" ? { program: row.value! } : {}),
      ...(row.trigger === "events" && row.events?.length ? { events: row.events } : {}),
      ...(row.lead > 0 ? { lead_beats: row.lead } : {}),
      ...(row.momentary ? { momentary: true } : {}),
    };
  }
  return map;
}

/**
 * 把当前草稿写入工程。
 *
 * 删除是立刻生效的（用户理解中的“删除”），所以删除后也走这里；
 * 失败时返回 false，让调用方从工程快照恢复草稿。
 */
async function persistMap(message?: string): Promise<boolean> {
  if (saving.value) return false;
  saving.value = true;
  const map = buildMap();
  const names = Object.keys(map);
  const defaultName = defaultTechnique.value.trim();
  const defaultToSave = defaultName && map[defaultName] ? defaultName : undefined;
  try {
    await store.setArticulations(
      props.track.id,
      map,
      defaultToSave,
      names.length ? mapSource.value : undefined,
    );
    confirmClear.value = false;
    layout.notify(message ?? i18n.t("midi.articulationsApplied", { count: names.length }));
    return true;
  } catch (err) {
    layout.notify(i18n.t("error.updateNotes", { error: String(err) }));
    return false;
  } finally {
    saving.value = false;
  }
}

async function save(): Promise<void> {
  if (await persistMap()) emit("update:open", false);
}

const defaultOptions = computed<SelectOption[]>(() => [
  { id: "", label: i18n.t("midi.none") },
  ...rows
    .filter((row) => row.name.trim())
    .map((row) => ({ id: row.name, label: row.name })),
]);

const remainingSuggestions = computed(() =>
  SUGGESTIONS.value.filter((name) => !rows.some((row) => row.name === name)),
);
</script>

<template>
  <ModalShell
    :open="open"
    :title="i18n.t('midi.studioTitle')"
    width="760px"
    @update:open="emit('update:open', $event)"
  >
    <template #toolbar>
      <div class="tabs-bar">
        <div class="m3-seg" role="tablist" :aria-label="i18n.t('midi.studioTitle')">
          <button
            v-for="item in tabs"
            :key="item.id"
            class="m3-seg-btn"
            :class="{ on: tab === item.id }"
            type="button"
            role="tab"
            :aria-selected="tab === item.id"
            @click="selectTab(item.id)"
          >
            {{ item.label }}
          </button>
        </div>
      </div>
    </template>

    <div v-if="tab === 'techniques'" class="m3-dialog-body">
      <div v-if="!hasMap || showGuide" class="guide m3-card">
        <div class="guide-step" :class="{ done: hasMap }">
          <span class="guide-num">1</span>
          <div class="guide-copy">
            <span class="guide-title">{{ i18n.t("midi.guideStep1Title") }}</span>
            <span class="guide-body">{{ i18n.t("midi.guideStep1Body") }}</span>
            <div class="guide-actions">
              <button class="m3-btn tonal" type="button" @click="selectTab('library')">
                {{ i18n.t("midi.guideStep1Action") }}
              </button>
              <span v-if="hasMap" class="guide-done">
                {{ i18n.t("midi.guideDone", { count: mapCount }) }}
              </span>
              <button
                v-if="hasMap"
                class="m3-btn text"
                type="button"
                @click="showGuide = false"
              >
                {{ i18n.t("midi.hideGuide") }}
              </button>
            </div>
          </div>
        </div>
        <div class="guide-step" :class="{ done: hasMap }">
          <span class="guide-num">2</span>
          <div class="guide-copy">
            <span class="guide-title">{{ i18n.t("midi.guideStep2Title") }}</span>
            <span class="guide-body">{{ i18n.t("midi.guideStep2Body") }}</span>
          </div>
        </div>
        <div class="guide-step">
          <span class="guide-num">3</span>
          <div class="guide-copy">
            <span class="guide-title">{{ i18n.t("midi.guideStep3Title") }}</span>
            <span class="guide-body">{{ i18n.t("midi.guideStep3Body") }}</span>
          </div>
        </div>
      </div>

      <div v-else class="map-status m3-card">
        <div class="map-status-main">
          <span class="map-status-title">
            {{ i18n.t("midi.guideDone", { count: mapCount }) }}
          </span>
          <span class="map-status-sub">{{ sourceLabel }}</span>
        </div>
        <button class="m3-btn text" type="button" @click="showGuide = true">
          {{ i18n.t("midi.showGuide") }}
        </button>
        <button class="m3-btn text" type="button" @click="selectTab('library')">
          {{ i18n.t("midi.mapReimport") }}
        </button>
        <button class="m3-btn text danger" type="button" @click="confirmClear = true">
          {{ i18n.t("midi.mapClear") }}
        </button>
      </div>

      <div v-if="unknownTags.length || confirmClear" class="studio-banners">
        <div v-if="confirmClear" class="m3-card banner warn-banner">
          <span class="banner-body">{{ i18n.t("midi.clearMapConfirm") }}</span>
          <div class="banner-actions">
            <button class="m3-btn text" type="button" @click="confirmClear = false">
              {{ i18n.t("common.cancel") }}
            </button>
            <button class="m3-btn filled" type="button" @click="clearMap">
              {{ i18n.t("midi.mapClearConfirm") }}
            </button>
          </div>
        </div>
        <div v-if="unknownTags.length" class="m3-card banner warn-banner">
          <span class="banner-title">{{ i18n.t("midi.unknownTags") }}</span>
          <span class="banner-body">{{ unknownTags.join(" · ") }}</span>
        </div>
      </div>
      <div class="studio">
        <!-- 左：技巧清单 -->
        <aside class="m3-card list-card">
          <div class="m3-card-head">
            <span class="m3-card-title">{{ i18n.t("midi.studioTechniques") }}</span>
            <span class="m3-card-sub">{{ rows.length }}</span>
          </div>
          <p class="m3-hint list-hint">{{ i18n.t("midi.studioHint") }}</p>

          <p v-if="rows.length === 0" class="m3-hint empty">{{ i18n.t("midi.noTechniques") }}</p>
          <button
            v-for="(row, index) in rows"
            :key="index"
            class="tech"
            :class="{ active: selected === index }"
            type="button"
            @click="selected = index"
          >
            <span class="tech-name">{{ row.name || i18n.t("midi.untitled") }}</span>
            <span class="tech-trigger">{{ triggerLabel(row.trigger) }}</span>
          </button>

          <div class="add-row">
            <input
              v-model="newName"
              class="m3-input add-input"
              type="text"
              spellcheck="false"
              :placeholder="i18n.t('midi.customPlaceholder')"
              :aria-label="i18n.t('midi.addCustom')"
              @keydown.enter.prevent="addCustom"
            />
            <button
              class="m3-icon-btn"
              type="button"
              :aria-label="i18n.t('midi.addCustom')"
              @click="addCustom"
            >
              <PlusIcon class="icon-svg" />
            </button>
          </div>

          <div v-if="remainingSuggestions.length" class="suggestions">
            <span class="m3-field-label">{{ i18n.t("midi.suggestions") }}</span>
            <div class="chips">
              <button
                v-for="name in remainingSuggestions"
                :key="name"
                class="m3-chip"
                type="button"
                @click="addSuggestion(name)"
              >
                {{ name }}
              </button>
            </div>
          </div>
        </aside>

        <!-- 右：当前技巧 -->
        <section v-if="current" class="editor-col">
          <div class="m3-card">
            <div class="m3-card-head">
              <span class="m3-card-title">{{ i18n.t("midi.currentTechnique") }}</span>
              <button
                class="m3-btn text danger"
                type="button"
                :disabled="saving"
                :title="i18n.t('midi.deleteImmediately')"
                @click="removeRow(selected)"
              >
                {{ i18n.t("midi.remove") }}
              </button>
            </div>
            <p class="m3-hint delete-hint">{{ i18n.t("midi.deleteImmediately") }}</p>

            <div class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.name") }}</span>
              </div>
              <div class="m3-row-control">
                <input
                  v-model="current.name"
                  class="m3-input name-input"
                  type="text"
                  spellcheck="false"
                  :aria-label="i18n.t('midi.name')"
                />
              </div>
            </div>

            <template v-if="current.trigger === 'events'">
              <div class="m3-row">
                <div class="m3-row-main">
                  <span class="m3-row-label">{{ i18n.t("midi.importedSwitch") }}</span>
                  <span class="m3-row-sub">{{ i18n.t("midi.importedSwitchHint") }}</span>
                </div>
                <div class="m3-row-control">
                  <span class="event-summary">{{ eventSummary(current) }}</span>
                </div>
              </div>
              <div class="m3-row">
                <div class="m3-row-main">
                  <span class="m3-row-sub">{{ i18n.t("midi.convertToManualHint") }}</span>
                </div>
                <div class="m3-row-control">
                  <button class="m3-btn text" type="button" @click="convertToManual">
                    {{ i18n.t("midi.convertToManual") }}
                  </button>
                </div>
              </div>
            </template>

            <div v-else class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.triggerQuestion") }}</span>
                <span class="m3-row-sub">{{ i18n.t("midi.triggerHint") }}</span>
              </div>
              <div class="m3-row-control">
                <div class="m3-seg compact" role="group" :aria-label="i18n.t('midi.triggerQuestion')">
                  <button
                    v-for="option in triggerOptions"
                    :key="option.id"
                    class="m3-seg-btn"
                    :class="{ on: current.trigger === option.id }"
                    type="button"
                    @click="setTrigger(option.id)"
                  >
                    {{ option.label }}
                  </button>
                </div>
              </div>
            </div>

            <div v-if="current.trigger === 'keyswitch'" class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.keyswitch") }}</span>
                <span class="m3-row-sub">{{ i18n.t("midi.keyswitchHint") }}</span>
              </div>
              <div class="m3-row-control select-control">
                <Md3Select
                  :model-value="current.value == null ? '' : String(current.value)"
                  :options="noteOptions"
                  :label="i18n.t('midi.keyswitch')"
                  @update:model-value="current.value = $event === '' ? null : Number($event)"
                />
              </div>
            </div>

            <template v-if="current.trigger === 'cc'">
              <div class="m3-row">
                <div class="m3-row-main">
                  <span class="m3-row-label">{{ i18n.t("midi.cc") }}</span>
                  <span class="m3-row-sub">{{ i18n.t("midi.ccHint") }}</span>
                </div>
                <div class="m3-row-control">
                  <NumberField
                    v-model="current.ccController"
                    :sr-label="i18n.t('midi.cc')"
                    :min="0"
                    :max="127"
                    :step="1"
                  />
                </div>
              </div>
              <div class="m3-row">
                <div class="m3-row-main">
                  <span class="m3-row-label">{{ i18n.t("midi.ccValue") }}</span>
                  <span class="m3-row-sub">{{ i18n.t("midi.ccValueHint") }}</span>
                </div>
                <div class="m3-row-control">
                  <NumberField
                    v-model="current.ccValue"
                    :sr-label="i18n.t('midi.ccValue')"
                    :min="0"
                    :max="127"
                    :step="1"
                  />
                </div>
              </div>
            </template>

            <div v-if="current.trigger === 'program'" class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.program") }}</span>
                <span class="m3-row-sub">{{ i18n.t("midi.programHint") }}</span>
              </div>
              <div class="m3-row-control">
                <NumberField
                  :model-value="current.value ?? 0"
                  :sr-label="i18n.t('midi.program')"
                  :min="0"
                  :max="127"
                  :step="1"
                  @update:model-value="current.value = $event"
                />
              </div>
            </div>
          </div>

          <div class="m3-card">
            <div class="m3-card-head">
              <span class="m3-card-title">{{ i18n.t("midi.moreOptions") }}</span>
            </div>

            <div class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.lead") }}</span>
                <span class="m3-row-sub">{{ i18n.t("midi.leadHint") }}</span>
              </div>
              <div class="m3-row-control">
                <NumberField
                  v-model="current.lead"
                  :sr-label="i18n.t('midi.lead')"
                  :min="0"
                  :max="4"
                  :step="0.01"
                  :decimals="3"
                />
              </div>
            </div>

            <div class="m3-row">
              <div class="m3-row-main">
                <span class="m3-row-label">{{ i18n.t("midi.momentary") }}</span>
                <span class="m3-row-sub">{{ i18n.t("midi.momentaryHint") }}</span>
              </div>
              <div class="m3-row-control">
                <Md3Switch
                  v-model="current.momentary"
                  :label="i18n.t('midi.momentary')"
                />
              </div>
            </div>
          </div>

          <div class="editor-actions">
            <button class="m3-btn tonal" type="button" @click="audition">
              {{ i18n.t("midi.audition") }}
            </button>
          </div>
        </section>

        <section v-else class="m3-card empty-editor">
          <p class="m3-hint">{{ i18n.t("midi.pickTechnique") }}</p>
        </section>
      </div>
    </div>

    <div v-else-if="tab === 'library'" class="m3-dialog-body">
      <div class="library">
        <div class="library-intro">
          <span class="guide-num">1</span>
          <div class="guide-copy">
            <span class="guide-title">{{ i18n.t("midi.guideStep1Title") }}</span>
            <span class="guide-body">{{ i18n.t("midi.libraryStepHint") }}</span>
          </div>
        </div>

        <div class="library-toolbar">
          <input
            v-model="libraryQuery"
            class="m3-input"
            type="search"
            spellcheck="false"
            :placeholder="i18n.t('midi.librarySearchPlaceholder')"
            :aria-label="i18n.t('midi.librarySearch')"
            @keydown.enter.prevent="searchLibrary"
          />
          <button class="m3-btn tonal" type="button" :disabled="libraryLoading" @click="searchLibrary">
            {{ i18n.t("midi.librarySearch") }}
          </button>
        </div>

        <div class="library-mode">
          <span class="m3-field-label">{{ i18n.t("midi.libraryMode") }}</span>
          <div class="m3-seg compact" role="group" :aria-label="i18n.t('midi.libraryMode')">
            <button
              class="m3-seg-btn"
              :class="{ on: libraryMode === 'replace' }"
              type="button"
              @click="libraryMode = 'replace'"
            >
              {{ i18n.t("midi.libraryReplace") }}
            </button>
            <button
              class="m3-seg-btn"
              :class="{ on: libraryMode === 'merge' }"
              type="button"
              @click="libraryMode = 'merge'"
            >
              {{ i18n.t("midi.libraryMerge") }}
            </button>
          </div>
        </div>

        <p class="m3-hint">{{ i18n.t("midi.libraryHint") }}</p>
        <p v-if="libraryError" class="m3-hint library-error">{{ libraryError }}</p>
        <p v-else-if="libraryLoading" class="m3-hint">{{ i18n.t("midi.libraryLoading") }}</p>
        <p v-else-if="libraryResults.length === 0" class="m3-hint">
          {{ i18n.t("midi.libraryEmpty") }}
        </p>
        <ul v-else class="library-list">
          <li v-for="entry in libraryResults" :key="entry.id" class="library-item">
            <div class="library-info">
              <span class="library-title">{{ entry.patch }}</span>
              <span class="library-sub">{{ entry.manufacturer }} · {{ entry.product }}</span>
              <span class="library-desc">
                {{ entry.description || i18n.t("midi.libraryCount", { count: entry.articulation_count }) }}
              </span>
            </div>
            <button
              class="m3-btn filled"
              type="button"
              :disabled="libraryLoading"
              @click="importLibrary(entry)"
            >
              {{ i18n.t("midi.libraryImport") }}
            </button>
          </li>
        </ul>
      </div>
    </div>

    <MidiAdvancedPane v-else :track="track" class="m3-dialog-body" />

    <template #foot>
      <div class="m3-dialog-foot">
        <template v-if="tab === 'techniques'">
          <label class="default-pick">
            <span class="m3-field-label">{{ i18n.t("midi.default") }}</span>
            <Md3Select
              v-model="defaultTechnique"
              :options="defaultOptions"
              :label="i18n.t('midi.default')"
            />
          </label>
          <span class="save-hint">{{ i18n.t("midi.saveHint") }}</span>
          <span class="spacer" />
          <button class="m3-btn text" type="button" @click="emit('update:open', false)">
            {{ i18n.t("common.cancel") }}
          </button>
          <button class="m3-btn filled" type="button" :disabled="saving" @click="save">
            {{ i18n.t("midi.saveApply") }}
          </button>
        </template>
        <template v-else>
          <span class="spacer" />
          <button class="m3-btn text" type="button" @click="emit('update:open', false)">
            {{ i18n.t("common.close") }}
          </button>
        </template>
      </div>
    </template>
  </ModalShell>
</template>

<style scoped>
.tabs-bar {
  padding: 0 16px 12px;
}

.studio {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  gap: 12px;
  align-items: start;
}

/* ---- 左栏清单 ---- */
.list-card {
  gap: 2px;
}
.list-hint {
  padding: 0 0 6px;
}
.empty {
  padding: 8px 2px;
}
.tech {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: 12px;
  background: transparent;
  color: rgb(var(--md-on-surface));
  text-align: left;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease;
}
.tech:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.tech.active {
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
.tech:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.tech-name {
  font-size: 12.5px;
  font-weight: 600;
}
.tech-trigger {
  font-size: 10.5px;
  opacity: 0.75;
}
.add-row {
  display: flex;
  align-items: center;
  gap: 4px;
  margin-top: 4px;
}
.add-input {
  flex: 1 1 auto;
  width: 100%;
  height: 34px;
  font-size: 12px;
}
.suggestions {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.3);
}
.chips {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

/* ---- 右栏编辑 ---- */
.editor-col {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}
.name-input {
  width: 210px;
}
.select-control {
  width: 210px;
}
.m3-seg.compact {
  padding: 2px;
  gap: 1px;
}
.m3-seg.compact .m3-seg-btn {
  height: 30px;
  padding: 0 11px;
  font-size: 11.5px;
}
.m3-seg.compact .m3-seg-btn.on {
  box-shadow: none;
}
.editor-actions {
  display: flex;
  justify-content: flex-end;
}
.empty-editor {
  min-height: 150px;
  justify-content: center;
  align-items: center;
  text-align: center;
}

/* ---- 页脚 ---- */
.default-pick {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 0 1 280px;
  min-width: 0;
}
.default-pick > .m3-field-label {
  white-space: nowrap;
}
.default-pick :deep(.md3-select) {
  flex: 1 1 auto;
  min-width: 0;
}
.spacer {
  flex: 1 1 auto;
}

.icon-svg {
  width: 15px;
  height: 15px;
}

/* ---- 三步指引 ---- */
.guide {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-bottom: 12px;
  padding: 12px;
}
.guide-step {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}
.guide-num {
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background-color: rgb(var(--md-surface-container-highest));
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  font-weight: 700;
}
.guide-step.done .guide-num {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.guide-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.guide-title {
  font-size: 12.5px;
  font-weight: 600;
}
.guide-body {
  font-size: 11px;
  opacity: 0.75;
  overflow-wrap: anywhere;
}
.guide-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 4px;
}
.guide-done {
  font-size: 11px;
  color: rgb(var(--md-primary));
}
.map-status {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  padding: 10px 12px;
}
.map-status-main {
  display: flex;
  flex: 1 1 220px;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.map-status-title {
  font-size: 12.5px;
  font-weight: 600;
}
.map-status-sub {
  font-size: 11px;
  opacity: 0.7;
  overflow-wrap: anywhere;
}
.library-intro {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 2px 0 6px;
}
.save-hint {
  font-size: 11px;
  opacity: 0.65;
}
.delete-hint {
  padding: 0 0 2px;
}

/* ---- 映射库 ---- */
.studio-banners {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 12px;
}
.banner {
  display: flex;
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
}
.banner-actions {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}
.banner-title {
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  white-space: nowrap;
  opacity: 0.7;
}
.banner-body {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 12px;
  overflow-wrap: anywhere;
}
.warn-banner {
  background-color: rgb(var(--md-error-container) / 0.55);
  color: rgb(var(--md-on-error-container));
}
.event-summary {
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 11px;
  overflow-wrap: anywhere;
}
.library {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.library-toolbar {
  display: flex;
  gap: 8px;
}
.library-toolbar .m3-input {
  flex: 1 1 auto;
}
.library-mode {
  display: flex;
  align-items: center;
  gap: 10px;
}
.library-error {
  color: rgb(var(--md-error));
}
.library-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin: 0;
  padding: 0;
  max-height: 46vh;
  overflow: auto;
  list-style: none;
}
.library-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  border-radius: 12px;
  background-color: rgb(var(--md-surface-container-high) / 0.5);
}
.library-info {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.library-title {
  font-size: 13px;
  font-weight: 600;
}
.library-sub,
.library-desc {
  font-size: 11px;
  opacity: 0.7;
  overflow-wrap: anywhere;
}

@media (max-width: 680px) {
  .studio {
    grid-template-columns: minmax(0, 1fr);
  }
  .m3-seg.compact .m3-seg-btn {
    padding: 0 8px;
  }
}
</style>
