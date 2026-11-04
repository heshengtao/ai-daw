<script setup lang="ts">
/**
 * **歌声合成**子页（侧栏内整页，从列表的卡片进来，左上角返回）。
 *
 * 输入是「MIDI 音符 + 歌词 + 一条参考声线」，输出是一条新音频轨。歌词逐音填在钢琴卷帘里；
 * 歌词模式下，这里只放入口——再把一个整串歌词框抄一遍只会让人不知道哪份生效。
 *
 * 参考声线两路：工程内的音频轨，或人声音色库（见 `useTonePicker`）。 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import { onSingingEvent, singingCancel, singingRender, singingStatus } from "@/api/tauri";
import type { SingingStatus } from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useDownloadsStore } from "@/stores/downloads";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useI18n } from "@/i18n";
import { useTonePicker } from "@/composables/useTonePicker";
import { useModelChoice } from "@/composables/useModelChoice";
import { useModelDownloadPrompt } from "@/composables/useModelDownloadPrompt";

const emit = defineEmits<{ (e: "back"): void }>();

const router = useRouter();
const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();

const downloads = useDownloadsStore();
const models = useModelsStore();
const i18n = useI18n();

const singing = ref<SingingStatus>({ running: false, progress: 0, message: null });

/** 有音符的 MIDI 轨道（歌声合成的输入）。 */
const midiTracks = computed(() =>
  (project.project?.tracks ?? []).filter(
    (t) => !t.clips.every((c) => c.audio) && t.clips.some((c) => (c.notes?.length ?? 0) > 0),
  ),
);
/** 有音频片段的轨道（可当参考声线）。 */
const audioTracks = computed(() =>
  (project.project?.tracks ?? []).filter((t) => t.clips.some((c) => c.audio)),
);

const busy = computed(() => singing.value.running);
const {
  source,
  tone,
  sourceOptions,
  sourceChoice,
  toneOptions,
  toneVoice,
  toneVoiceId,
  toneTrackId,
  hasTone,
  importing: toneImporting,
  saveOpen: toneSaveOpen,
  draftName: toneDraft,
  refresh: refreshVoices,
  beginSave,
  confirmSave,
  cancelSave,
  importFile,
  openLibrary,
} = useTonePicker(audioTracks, () => busy.value);

/** 目标 MIDI 轨道（`Md3Select` 是字符串模型）。 */
const singTrackId = ref<string | null>(null);
const singTrack = computed(
  () => midiTracks.value.find((t) => String(t.id) === singTrackId.value) ?? midiTracks.value[0] ?? null,
);
const singTrackOptions = computed(() =>
  midiTracks.value.map((t) => ({ id: String(t.id), label: t.name })),
);

/** 歌声合成（fp16 / fp32）：与列表卡片共用「记住的那一档」。 */
const {
  current: model,
  id: modelId,
  ready,
  downloading,
  selectOptions: modelOptions,
} = useModelChoice("sing", () => models.catalog.filter((m) => m.feature === "sing"));

const steps = ref(32);
const stepOptions = [
  { id: "16", label: "16" },
  { id: "24", label: "24" },
  { id: "32", label: "32" },
  { id: "48", label: "48" },
];
const stepChoice = computed({
  get: () => String(steps.value),
  set: (v: string) => {
    steps.value = Number(v);
  },
});

const progressPct = computed(() => `${Math.round(singing.value.progress * 100)}%`);
/** 进度条：下载走下载器百分比（拿不到就条纹），推理走自己的百分比。 */
const barIndeterminate = computed(() => downloading.value && models.progressPct === null);
const barStyle = computed(() => {
  if (downloading.value) {
    return models.progressPct === null ? undefined : { width: `${models.progressPct}%` };
  }
  return { width: progressPct.value };
});

const statusLine = computed(() => {
  if (downloading.value) return i18n.t("ai.downloadingModel");
  if (busy.value) return `${i18n.t("ai.singingBusy")} · ${progressPct.value}`;
  if (!midiTracks.value.length) return i18n.t("ai.singingNeedsMidi");
  return ready.value ? i18n.t("ai.singingNote") : i18n.t("ai.singingNeedsModel");
});

/**
 * 「去下载」：确认后跳到下载管理器**并立刻开始下载**。
 *
 * 以前这里只 `openTab("manager")` + 请求聚焦，什么也不下；用户到了管理器还要自己
 * 找到那一行再点一次「下载」，很容易以为按钮坏了。 */
const {
  open: downloadOpen,
  message: downloadMessage,
  require: requireDownload,
  confirm: confirmDownload,
  background: backgroundDownload,
  cancel: cancelDownload,
} = useModelDownloadPrompt();

function downloadModel(): void {
  const current = model.value;
  // 没装 → 弹确认框（`require` 返回 false），确认后跳转并下载。
  if (current && !requireDownload(current)) return;
  downloads.jumpTo("model", current?.id ?? "*");
  models.requestFocus(current?.id ?? "*");
}

/** 跳到钢琴卷帘的歌词模式，并把当前 MIDI 片段选中。 */
function openLyricsEditor(): void {
  const t = singTrack.value;
  if (!t) return;
  const clip = t.clips.find((c) => !c.audio && (c.notes?.length ?? 0) > 0);
  selection.selectTrack(t.id);
  if (clip) selection.selectClip(t.id, clip.id);
  layout.setTool("lyric");
  void router.push("/edit");
}

async function render(): Promise<void> {
  const t = singTrack.value;
  if (!t || busy.value) return;
  if (!ready.value) {
    downloadModel();
    return;
  }
  if (!hasTone.value) {
    layout.notify(i18n.t("ai.singingNeedsTone"));
    return;
  }
  try {
    await singingRender({
      track: t.id,
      reference_voice: toneVoiceId.value,
      reference_track: toneTrackId.value,
      model: model.value?.id ?? null,
      steps: steps.value,
      seed: 0,
    });
  } catch (err) {
    layout.notify(String(err));
  }
}

async function stop(): Promise<void> {
  try {
    await singingCancel();
  } catch {
    // 状态事件会纠正。
  }
}

let stopStatus: (() => void) | null = null;
let stopProgress: (() => void) | null = null;

onMounted(async () => {
  void models.load();
  stopProgress = models.watchProgress();
  await refreshVoices();
  // 默认落在当前选中的那条 MIDI 轨上。
  singTrackId.value =
    String((midiTracks.value.find((t) => t.id === selection.trackId) ?? midiTracks.value[0])?.id ?? "") ||
    null;
  // 参考声线默认跟着当前选中的音频轨（选中轨不是音频轨时保持第一条）。
  const selectedRef = audioTracks.value.find((t) => t.id === selection.trackId);
  if (selectedRef) {
    source.value = "track";
    tone.value = `track:${selectedRef.id}`;
  }
  singing.value = await singingStatus().catch(() => singing.value);
  stopStatus = await onSingingEvent((s) => (singing.value = s));
});

onUnmounted(() => {
  stopStatus?.();
  stopProgress?.();
});

// 目标轨道消失后回落到合法值。
watch(
  () => midiTracks.value.map((t) => t.id).join(","),
  () => {
    if (!midiTracks.value.some((t) => String(t.id) === singTrackId.value)) {
      const selected = midiTracks.value.find((t) => t.id === selection.trackId);
      singTrackId.value = String((selected ?? midiTracks.value[0])?.id ?? "") || null;
    }
  },
);

// 合成跑完时把摘要 / 错误作为通知弹一次。
watch(
  () => singing.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && singing.value.message) layout.notify(singing.value.message);
  },
);
</script>

<template>
  <div class="ai-subpage">
    <header class="sub-head">
      <button class="sub-icon" :aria-label="i18n.t('common.back')" @click="emit('back')">
        <ArrowLeftIcon class="w-[15px] h-[15px]" />
      </button>
      <h1 class="sub-title">{{ i18n.t("ai.singingShort") }}</h1>
      <span class="sub-badge" :class="{ on: ready }">
        {{ i18n.t(ready ? "ai.backendModel" : "ai.modelMissing") }}
      </span>
    </header>

    <p class="sub-status">{{ statusLine }}</p>

    <section class="sub-card">
      <label class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.singingTrack") }}</span>
        <Md3Select
          v-model="singTrackId"
          :options="singTrackOptions"
          :label="i18n.t('ai.singingTrack')"
          :empty="i18n.t('ai.singingNeedsMidi')"
          :disabled="busy"
        />
      </label>

      <!-- 歌词在钢琴卷帘里逐音填（这里只放入口） -->
      <div class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.lyrics") }}</span>
        <div class="sub-inline">
          <span class="sub-hint">{{ i18n.t("ai.lyricsInEditor") }}</span>
          <span class="sub-spacer" />
          <button class="sub-ghost" :disabled="!singTrack || busy" @click="openLyricsEditor">
            {{ i18n.t("ai.goFillLyrics") }}
          </button>
        </div>
      </div>

      <!-- 参考声线：先选来源（工程轨道 / 音色库），再选具体那条 -->
      <label class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.refSource") }}</span>
        <Md3Select
          v-model="sourceChoice"
          :options="sourceOptions"
          :label="i18n.t('ai.refSource')"
          :disabled="busy"
        />
      </label>
      <label class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.singingTone") }}</span>
        <Md3Select
          v-model="tone"
          :options="toneOptions"
          :label="i18n.t('ai.singingTone')"
          :empty="source === 'voice' ? i18n.t('ai.voiceLibraryEmpty') : i18n.t('ai.singingNeedsTone')"
          :disabled="busy"
        />
      </label>

      <div class="sub-inline">
        <span class="sub-hint">
          <template v-if="source === 'track'">{{ i18n.t("ai.voiceSaveHint") }}</template>
          <template v-else-if="toneVoice">
            {{ i18n.t("ai.voiceFromLibrary", { seconds: Math.round(toneVoice.seconds) }) }}
          </template>
          <template v-else>{{ i18n.t("ai.voiceLibraryEmpty") }}</template>
        </span>
        <span class="sub-spacer" />
        <button
          v-if="source === 'track'"
          class="sub-ghost"
          :disabled="busy || toneImporting || !toneTrackId"
          @click="beginSave"
        >
          {{ i18n.t("ai.voiceSave") }}
        </button>
        <button class="sub-ghost" :disabled="busy || toneImporting" @click="importFile">
          {{ toneImporting ? i18n.t("ai.voiceImporting") : i18n.t("ai.voiceImport") }}
        </button>
        <button class="sub-ghost" :disabled="busy" @click="openLibrary">
          {{ i18n.t("ai.manageVoices") }}
        </button>
      </div>

      <!-- 存进音色库：内联命名，不弹窗 -->
      <div v-if="toneSaveOpen" class="sub-inline">
        <input
          v-model="toneDraft"
          class="sub-input"
          style="max-width: 150px"
          type="text"
          maxlength="60"
          spellcheck="false"
          :placeholder="i18n.t('ai.voiceDefaultName')"
          @keydown.enter.prevent="confirmSave"
        />
        <button class="sub-ghost" @click="confirmSave">{{ i18n.t("common.save") }}</button>
        <button class="sub-ghost" @click="cancelSave">{{ i18n.t("common.cancel") }}</button>
      </div>

      <div class="sub-row">
        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.singingModel") }}</span>
          <Md3Select v-model="modelId" :options="modelOptions" :label="i18n.t('ai.singingModel')" :disabled="busy" />
        </label>
        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.steps") }}</span>
          <Md3Select v-model="stepChoice" :options="stepOptions" :label="i18n.t('ai.steps')" :disabled="busy" />
        </label>
      </div>

      <div class="sub-actions">
        <span v-if="busy" class="sub-hint">{{ progressPct }}</span>
        <button v-if="!busy" class="sub-primary" :disabled="!singTrack" @click="render">
          {{ ready ? i18n.t("ai.runSinging") : i18n.t("ai.downloadAndSing") }}
        </button>
        <button v-else class="sub-primary" @click="stop">{{ i18n.t("ai.stop") }}</button>
      </div>

      <div v-if="downloading || busy" class="sub-progress">
        <div class="sub-progress-fill" :class="{ indeterminate: barIndeterminate }" :style="barStyle" />
      </div>
    </section>

    <!-- 缺权重：确认后跳到管理器并**直接开始下载**（不再只是切个页签）；也可留在后台 -->
    <ConfirmDialog
      v-model:open="downloadOpen"
      :title="i18n.t('dialog.downloadModelTitle')"
      :message="downloadMessage"
      :confirm-label="i18n.t('dialog.downloadModelOk')"
      :extra-label="i18n.t('setup.background')"
      @confirm="confirmDownload"
      @extra="backgroundDownload"
      @cancel="cancelDownload"
    />
  </div>
</template>
