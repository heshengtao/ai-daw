<script setup lang="ts">
/**
 * **音色替换**子页（侧栏内整页，从列表的卡片进来，左上角返回）。
 *
 * 一段人声 + 一条参考声线 → 同一条旋律、换一个音色。与歌声合成的差别只在输入：
 * 这里给的是**已经有唱词的人声**，不需要 MIDI 与歌词，但需要按音域差做整体移调。 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import { onSvcEvent, svcCancel, svcRender, svcStatus } from "@/api/tauri";
import type { SvcStatus } from "@/api/types";
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

const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();

const downloads = useDownloadsStore();
const models = useModelsStore();
const i18n = useI18n();

const svc = ref<SvcStatus>({ running: false, progress: 0, message: null });

/** 有音频片段的轨道（既能当要换的目标，也能当参考声线）。 */
const audioTracks = computed(() =>
  (project.project?.tracks ?? []).filter((t) => t.clips.some((c) => c.audio)),
);

const busy = computed(() => svc.value.running);
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

/** 目标：要换音色的那条人声。 */
const targetId = ref<string | null>(null);
const target = computed(
  () => audioTracks.value.find((t) => String(t.id) === targetId.value) ?? audioTracks.value[0] ?? null,
);
const targetOptions = computed(() =>
  audioTracks.value.map((t) => ({ id: String(t.id), label: t.name })),
);

/** 音色替换（fp16_hq / fp32）：与列表卡片共用「记住的那一档」。 */
const {
  current: model,
  id: modelId,
  ready,
  downloading,
  selectOptions: modelOptions,
} = useModelChoice("voice_swap", () => models.catalog.filter((m) => m.feature === "voice_swap"));

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

/** 整体移调（半音）：男女声对唱时按音域差调。 */
const shift = ref(0);
const shiftOptions = computed(() =>
  [-12, -7, -5, -2, 0, 2, 5, 7, 12].map((v) => ({
    id: String(v),
    label: v === 0 ? i18n.t("ai.pitchOriginal") : v > 0 ? `+${v}` : String(v),
  })),
);
const shiftChoice = computed({
  get: () => String(shift.value),
  set: (v: string) => {
    shift.value = Number(v);
  },
});

const progressPct = computed(() => `${Math.round(svc.value.progress * 100)}%`);
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
  if (busy.value) return `${i18n.t("ai.voiceSwapBusy")} · ${progressPct.value}`;
  if (!audioTracks.value.length) return i18n.t("ai.voiceSwapNeedsAudio");
  return ready.value ? i18n.t("ai.voiceSwapNote") : i18n.t("ai.voiceSwapNeedsModel");
});

/**
 * 「去下载」：确认后跳到下载管理器**并立刻开始下载**。
 *
 * 以前这里只切页签 + 请求聚焦，什么也不下，用户到了管理器还得自己再点一次「下载」。 */
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
  if (current && !requireDownload(current)) return;
  downloads.jumpTo("model", current?.id ?? "*");
  models.requestFocus(current?.id ?? "*");
}

async function render(): Promise<void> {
  const t = target.value;
  if (!t || busy.value) return;
  if (!ready.value) {
    downloadModel();
    return;
  }
  if (!hasTone.value) {
    layout.notify(i18n.t("ai.voiceSwapNeedsRef"));
    return;
  }
  try {
    await svcRender({
      track: t.id,
      reference_voice: toneVoiceId.value,
      reference_track: toneTrackId.value,
      model: model.value?.id ?? null,
      steps: steps.value,
      seed: 0,
      pitch_shift: shift.value,
    });
  } catch (err) {
    layout.notify(String(err));
  }
}

async function stop(): Promise<void> {
  try {
    await svcCancel();
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
  // 目标默认跟当前选中的音频轨。
  targetId.value =
    String((audioTracks.value.find((t) => t.id === selection.trackId) ?? audioTracks.value[0])?.id ?? "") ||
    null;
  // 参考声线：选中轨就是目标时退到另一条，别把「要换的那条」当参考。
  const preferred = audioTracks.value.find((t) => t.id === selection.trackId && String(t.id) !== targetId.value);
  const fallback = audioTracks.value.find((t) => String(t.id) !== targetId.value);
  if (preferred || fallback) {
    source.value = "track";
    tone.value = `track:${(preferred ?? fallback)!.id}`;
  }
  svc.value = await svcStatus().catch(() => svc.value);
  stopStatus = await onSvcEvent((s) => (svc.value = s));
});

onUnmounted(() => {
  stopStatus?.();
  stopProgress?.();
});

// 目标轨道消失后回落到合法值。
watch(
  () => audioTracks.value.map((t) => t.id).join(","),
  () => {
    if (!audioTracks.value.some((t) => String(t.id) === targetId.value)) {
      const selected = audioTracks.value.find((t) => t.id === selection.trackId);
      targetId.value = String((selected ?? audioTracks.value[0])?.id ?? "") || null;
    }
  },
);

// 替换跑完时把摘要 / 错误作为通知弹一次。
watch(
  () => svc.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && svc.value.message) layout.notify(svc.value.message);
  },
);
</script>

<template>
  <div class="ai-subpage">
    <header class="sub-head">
      <button class="sub-icon" :aria-label="i18n.t('common.back')" @click="emit('back')">
        <ArrowLeftIcon class="w-[15px] h-[15px]" />
      </button>
      <h1 class="sub-title">{{ i18n.t("ai.voiceSwapShort") }}</h1>
      <span class="sub-badge" :class="{ on: ready }">
        {{ i18n.t(ready ? "ai.backendModel" : "ai.modelMissing") }}
      </span>
    </header>

    <p class="sub-status">{{ statusLine }}</p>

    <section class="sub-card">
      <label class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.voiceSwapTarget") }}</span>
        <Md3Select
          v-model="targetId"
          :options="targetOptions"
          :label="i18n.t('ai.voiceSwapTarget')"
          :empty="i18n.t('ai.voiceSwapNeedsAudio')"
          :disabled="busy"
        />
      </label>

      <!-- 参考：工程内轨道 / 人声音色 -->
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
          :empty="source === 'voice' ? i18n.t('ai.voiceLibraryEmpty') : i18n.t('ai.voiceSwapNeedsRef')"
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

      <label class="sub-field">
        <span class="sub-name">{{ i18n.t("ai.singingModel") }}</span>
        <Md3Select v-model="modelId" :options="modelOptions" :label="i18n.t('ai.singingModel')" :disabled="busy" />
      </label>

      <div class="sub-pair">
        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.steps") }}</span>
          <Md3Select v-model="stepChoice" :options="stepOptions" :label="i18n.t('ai.steps')" :disabled="busy" />
        </label>
        <label class="sub-field">
          <span class="sub-name">{{ i18n.t("ai.pitchShift") }}</span>
          <Md3Select
            v-model="shiftChoice"
            :options="shiftOptions"
            :label="i18n.t('ai.pitchShift')"
            :disabled="busy"
          />
        </label>
      </div>

      <div class="sub-actions">
        <span v-if="busy" class="sub-hint">{{ progressPct }}</span>
        <button v-if="!busy" class="sub-primary" :disabled="!target" @click="render">
          {{ ready ? i18n.t("ai.runVoiceSwap") : i18n.t("ai.downloadAndSwap") }}
        </button>
        <button v-else class="sub-primary" @click="stop">{{ i18n.t("ai.stop") }}</button>
      </div>

      <div v-if="downloading || busy" class="sub-progress">
        <div class="sub-progress-fill" :class="{ indeterminate: barIndeterminate }" :style="barStyle" />
      </div>
    </section>

    <!-- 缺权重：确认后跳到管理器并**直接开始下载**；也可以留在本页后台下 -->
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
