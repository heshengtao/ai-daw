<script setup lang="ts">
/**
 * AI 功能：与「插件」面板同一种风格（一个可滚动容器 + 一组卡片条目，不套多层容器）。
 *
 * 一级条目，绝大多数**依赖本地模型**（`crates/local-ai` 的 ONNX 权重）：
 * - **MCP 服务器**：开关；开启后同一容器里多出一行接入地址 + 「复制」。
 * - **命令行与 Skills**：入口——右侧按钮跳到「设置 → 命令行」。
 * - **音频转 MIDI** / **人声降噪**：各自**一个**「开始」按钮；处理中该条**下边缘变成
 *   进度条**、按钮变成「停止」，可随时中止。权重缺失时退回内置 DSP，条目上的标签会标明。
 * - **音源分离**：HTDemucs，可选 4 / 6 分轨；没有内置兜底，缺权重时先问再下载。
 * - **节拍检测**：Beat This!，推出全局 BPM + 拍号。
 * - **音乐生成**：只是**入口**——按一下就跳到独立的「AI 音乐生成」面板。音乐生成有
 *   描述 / 歌词 / 时长 / 种子一整套表单和一次动辄几分钟的推理，塞进这个列表里会挤成
 *
 * **不依赖模型的传统 DSP 不在这里**：乐器底噪降噪与母带处理都搬到了「调音台」面板
 * （那条降噪走内置 DSP——LocalVQE 是 16 kHz 的语音模型，直接拿来处理音乐会把乐器当噪声压掉）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { BoltIcon, ClipboardDocumentIcon } from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import {
  aiCancel,
  aiModels,
  aiStart,
  aiStatus,
  mcpStatus,
  onAiEvent,
  onSingingEvent,
  setMcpConfig,
  singingCancel,
  singingModels,
  singingRender,
  singingStatus,
  voiceList,
  voiceSave,
} from "@/api/tauri";
import type {
  AiModels,
  AiStatus,
  McpStatus,
  ModelStatus,
  SingingModels,
  SingingStatus,
  VoiceEntry,
} from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useSoundMemoriesStore } from "@/stores/soundMemories";
import { useI18n } from "@/i18n";
import { copyText } from "@/utils/clipboard";
import { pickAudioToOpen } from "@/api/dialogs";
import { humanSize, missingBytes, modelName } from "@/utils/models";

const KIND_MIDI = "to_midi";
/** 人声降噪 / 去混响：走 LocalVQE 模型。 */
const KIND_VOICE = "enhance";
/** 音源分离：把所选轨道的音频拆成鼓/贝斯/其他/人声四条分轨（HTDemucs 模型，无 DSP 兜底）。 */
const KIND_SEPARATE = "separate";
/** 节拍检测：Beat This! 推出全局 BPM + 拍号，并把片段首重拍对齐到小节线。 */
const KIND_TEMPO = "detect_tempo";

const router = useRouter();
const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const modelsStore = useModelsStore();
const soundMemories = useSoundMemoriesStore();
const i18n = useI18n();

// ---- MCP ----
const status = ref<McpStatus | null>(null);
const saving = ref(false);
const mcpOn = computed(() => status.value?.enabled ?? false);
const endpoint = computed(() => status.value?.url ?? null);

async function refresh(): Promise<void> {
  try {
    status.value = await mcpStatus();
  } catch {
    status.value = null;
  }
}

async function toggleMcp(): Promise<void> {
  if (saving.value) return;
  saving.value = true;
  try {
    status.value = await setMcpConfig(!mcpOn.value, status.value?.port ?? 50562);
  } catch (err) {
    layout.notify(String(err));
  } finally {
    saving.value = false;
  }
}

async function copyEndpoint(): Promise<void> {
  const url = endpoint.value;
  if (!url) return;
  if (await copyText(url)) layout.notify(i18n.t("ai.copied"));
  else layout.notify(i18n.t("copy.failed"));
}

// ---- 本地 AI ----
const ai = ref<AiStatus>({ running: false, kind: null, progress: 0, message: null });
/** 两个 ONNX 权重是否就位（决定条目上的「模型 / 内置 DSP」标签）。 */
const models = ref<AiModels>({
  basic_pitch: false,
  localvqe: false,
  separate: false,
  beat_this: false,
});
let unlisten: (() => void) | null = null;
/** 模型下载进度订阅的取消函数。 */
let stopModel: (() => void) | null = null;
/** 歌声合成状态订阅的取消函数。 */
let stopSingingEvent: (() => void) | null = null;

/** 目标轨道（选中轨道，否则首轨）与「有音频片段」。 */
const track = computed(() => {
  const tracks = project.project?.tracks ?? [];
  if (selection.trackId === null) return tracks[0] ?? null;
  return tracks.find((t) => t.id === selection.trackId) ?? null;
});
const hasAudio = computed(() => (track.value?.clips ?? []).some((c) => c.audio));
const canRun = computed(() => Boolean(track.value) && hasAudio.value && !ai.value.running);

/** 某个功能是否正在处理。 */
function busy(kind: string): boolean {
  return ai.value.running && ai.value.kind === kind;
}

/** 这个功能会走哪个后端：权重就位就是真模型，否则退回内置 DSP。 */
function backendLabel(kind: string): string {
  return hasModel(kind) ? i18n.t("ai.backendModel") : i18n.t("ai.backendDsp");
}
function hasModel(kind: string): boolean {
  if (kind === KIND_MIDI) return models.value.basic_pitch;
  if (kind === KIND_VOICE) return models.value.localvqe;
  if (kind === KIND_SEPARATE) return models.value.separate;
  if (kind === KIND_TEMPO) return models.value.beat_this;
  return false;
}
const progressPct = computed(() => `${Math.round(ai.value.progress * 100)}%`);

// ---- 音源分离：可选模型（4 分轨 / 6 分轨）；缺权重时先问、再下载并跳到「管理器」----
/** 清单里的分离模型（按清单顺序）。 */
const separateModels = computed(() => modelsStore.catalog.filter((m) => m.feature === "separate"));
/** 当前选中的分离模型 id；`null` = 还没定，由下面的 watcher 按「谁已下载」自动选。 */
const sepModelId = ref<string | null>(null);
/** 当前选中的分离模型条目（清单没加载出来时为 null）。 */
const sepModel = computed(
  () => separateModels.value.find((m) => m.id === sepModelId.value) ?? separateModels.value[0] ?? null,
);
/** 选中模型是否就位。 */
const separateModelReady = computed(() => sepModel.value?.installed ?? false);
/** 选中模型是否正在下载。 */
const sepDownloading = computed(() =>
  sepModel.value ? modelsStore.installing(sepModel.value.id) : false,
);
/** 下拉选项：名称 + 「已下载 / 需下载 · 体积」。 */
const sepOptions = computed(() =>
  separateModels.value.map((m) => ({
    id: m.id,
    label: modelName(m),
    hint: m.installed
      ? i18n.t("ai.modelReady")
      : i18n.t("ai.modelNeedsDownload", { size: humanSize(m.approx_bytes) }),
  })),
);

/** 待下载的分离模型（用于 MD3 确认框）。 */
const pendingModel = ref<ModelStatus | null>(null);
/** 确认框是否打开。 */
const promptOpen = ref(false);
/** 确认框正文。 */
const downloadPromptMessage = computed(() => {
  const m = pendingModel.value;
  return m
    ? i18n.t("dialog.downloadModelMessage", { name: modelName(m), size: humanSize(missingBytes(m)) })
    : "";
});

/** 点「开始分离」：权重缺就弹 MD3 确认框，确认后下载并跳到「管理器 → 可下载 → 模型」。 */
async function runSeparation(): Promise<void> {
  const t = track.value;
  const model = sepModel.value;
  if (!t || !model || !canRun.value) return;
  if (!model.installed) {
    pendingModel.value = model;
    promptOpen.value = true;
    return;
  }
  try {
    await aiStart(KIND_SEPARATE, t.id, undefined, model.id);
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 打开右侧的「写歌智能体」面板。 */
function openAgent(): void {
  layout.openTab("agent");
}

/** 跳到「设置 → 命令行」子页。 */
function openCliSettings(): void {
  void router.push({ name: "settings", query: { section: "cli" } });
}

/** 确认框点了「下载」：跳到「管理器 → 模型」并开始下载。 */
async function confirmDownloadModel(): Promise<void> {
  const model = pendingModel.value;
  if (!model) return;
  layout.openTab("manager");
  modelsStore.requestFocus();
  try {
    await modelsStore.download(model.id);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function cancelModelDownload(): Promise<void> {
  await modelsStore.cancel();
}

// ---- 歌声合成（SoulX-Singer）：MIDI + 歌词 + 参考声线 → 新音频轨 ----
const singing = ref<SingingStatus>({ running: false, progress: 0, message: null });
const singingInfo = ref<SingingModels>({ installed: false, dir: null, approx_bytes: 0 });
const singModel = computed(
  () => modelsStore.catalog.find((m) => m.feature === "sing") ?? null,
);
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
/** 下拉用**字符串 id**（`Md3Select` 是字符串模型）；参考声线是 `voice:<id>` / `track:<id>`。 */
const singTrackId = ref<string | null>(null);
const singRef = ref<string>("");
const singSteps = ref(32);
/** 人声音色库（跨工程）。 */
const voices = ref<VoiceEntry[]>([]);
/** 参考声线的来源：工程内轨道 / 人声音色库。 */
const refSource = ref<"track" | "voice">("track");
const singBusy = computed(() => singing.value.running);
const singDownloading = computed(() =>
  singModel.value ? modelsStore.installing(singModel.value.id) : false,
);
const singProgressPct = computed(() => `${Math.round(singing.value.progress * 100)}%`);
const singReady = computed(() => singingInfo.value.installed);
const singTrack = computed(
  () => midiTracks.value.find((t) => String(t.id) === singTrackId.value) ?? midiTracks.value[0] ?? null,
);
const singOptions = computed(() =>
  midiTracks.value.map((t) => ({ id: String(t.id), label: t.name })),
);
const sourceOptions = computed(() => [
  { id: "track", label: i18n.t("ai.refSourceTrack") },
  { id: "voice", label: i18n.t("ai.refSourceVoice") },
]);
const sourceChoice = computed({
  get: () => refSource.value,
  set: (v: string) => {
    refSource.value = v === "voice" ? "voice" : "track";
    // 换来源时立刻落到该来源的第一条，避免下拉显示空值。
    singRef.value =
      refSource.value === "voice"
        ? (voices.value[0] ? `voice:${voices.value[0].id}` : "")
        : (audioTracks.value[0] ? `track:${audioTracks.value[0].id}` : "");
  },
});

/** 当前来源下可选的参考（轨道 / 音色库）。 */
const refOptions = computed(() =>
  refSource.value === "voice"
    ? voices.value.map((v) => ({
        id: `voice:${v.id}`,
        label: v.name,
        hint: `${Math.round(v.seconds)}s${v.description ? ` · ${v.description}` : ""}`,
      }))
    : audioTracks.value.map((t) => ({ id: `track:${t.id}`, label: t.name })),
);
const singRefVoiceId = computed(() =>
  singRef.value.startsWith("voice:") ? singRef.value.slice("voice:".length) : null,
);
const singRefTrackId = computed(() =>
  singRef.value.startsWith("track:") ? Number(singRef.value.slice("track:".length)) : null,
);
/** 当前选中的音色库条目。 */
const singVoice = computed(() =>
  voices.value.find((v) => v.id === singRefVoiceId.value) ?? null,
);
/** 有可用的参考声线（选中的那条还在）。 */
const hasTone = computed(() => refOptions.value.some((o) => o.id === singRef.value));

const stepOptions = [
  { id: "16", label: "16" },
  { id: "24", label: "24" },
  { id: "32", label: "32" },
  { id: "48", label: "48" },
];
const stepChoice = computed({
  get: () => String(singSteps.value),
  set: (v: string) => {
    singSteps.value = Number(v);
  },
});

// ---- 存进 / 移出人声音色库 ----
const voiceSaveOpen = ref(false);
const voiceDraftName = ref("");
/** 待保存的来源：工程内轨道，或一个刚选的音频文件。 */
const voiceSource = ref<{ track?: number; path?: string } | null>(null);

async function refreshVoices(): Promise<void> {
  try {
    voices.value = await voiceList();
  } catch {
    voices.value = [];
  }
}

/** 把当前选中的工程内轨道存成音色（默认用轨道名）。 */
function beginSaveVoice(): void {
  if (refSource.value !== "track") return;
  const track = audioTracks.value.find((t) => t.id === singRefTrackId.value);
  voiceSource.value = track ? { track: track.id } : null;
  voiceDraftName.value = track?.name ?? i18n.t("ai.voiceDefaultName");
  voiceSaveOpen.value = true;
}

async function confirmSaveVoice(): Promise<void> {
  const source = voiceSource.value;
  const name = voiceDraftName.value.trim();
  if (!source || !name) return;
  try {
    const entry = await voiceSave({
      track: source.track ?? null,
      path: source.path ?? null,
      name,
    });
    await refreshVoices();
    refSource.value = "voice";
    singRef.value = `voice:${entry.id}`;
    voiceSaveOpen.value = false;
    voiceSource.value = null;
    layout.notify(i18n.t("ai.voiceSaved", { name: entry.name }));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 打开侧边栏的「音色库」：人声和乐器在同一个库里，只靠标签区分。 */
function openVoiceLibrary(): void {
  soundMemories.kind = "voice";
  layout.openTab("sounds");
}

/** 从音频文件直接存一条人声音色（与「存入音色库」互补：一个从文件、一个从工程轨道）。 */
async function importVoice(): Promise<void> {
  if (singBusy.value) return;
  try {
    const path = await pickAudioToOpen();
    if (!path) return;
    const stem = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "";
    const entry = await voiceSave({ path, name: stem || i18n.t("ai.voiceDefaultName") });
    await refreshVoices();
    refSource.value = "voice";
    singRef.value = `voice:${entry.id}`;
    layout.notify(i18n.t("ai.voiceSaved", { name: entry.name }));
  } catch (err) {
    layout.notify(String(err));
  }
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

/** 选中轨变化后把 MIDI 轨与参考声线的默认值收敛到合法值。 */
watch(
  () =>
    [
      midiTracks.value.map((t) => t.id).join(","),
      audioTracks.value.map((t) => t.id).join(","),
      voices.value.map((v) => v.id).join(","),
      selection.trackId,
    ].join("|"),
  () => {
    if (!midiTracks.value.some((t) => String(t.id) === singTrackId.value)) {
      const selected = midiTracks.value.find((t) => t.id === selection.trackId);
      singTrackId.value = String((selected ?? midiTracks.value[0])?.id ?? "") || null;
    }
    // 所选来源下没有可选条目时，自动切到另一边（用户可能刚导入第一条人声）。
    if (refSource.value === "voice" && voices.value.length === 0 && audioTracks.value.length > 0) {
      refSource.value = "track";
    } else if (
      refSource.value === "track" &&
      audioTracks.value.length === 0 &&
      voices.value.length > 0
    ) {
      refSource.value = "voice";
    }
    if (!hasTone.value) {
      singRef.value = refOptions.value[0]?.id ?? "";
    }
  },
  { immediate: true },
);

/** 点「生成」：模型缺就弹确认框（下载后跳到管理器），否则直接渲染。 */
async function runSinging(): Promise<void> {
  const t = singTrack.value;
  if (!t || singBusy.value) return;
  if (!singReady.value) {
    if (!singModel.value) return;
    pendingModel.value = singModel.value;
    promptOpen.value = true;
    return;
  }
  if (!hasTone.value) {
    layout.notify(i18n.t("ai.singingNeedsTone"));
    return;
  }
  try {
    await singingRender({
      track: t.id,
      reference_voice: singRefVoiceId.value,
      reference_track: singRefTrackId.value,
      steps: singSteps.value,
      seed: 0,
    });
  } catch (err) {
    layout.notify(String(err));
  }
}

async function stopSinging(): Promise<void> {
  try {
    await singingCancel();
  } catch {
    // 状态事件会纠正。
  }
}


// ---- 节拍检测：只依赖一个模型（Beat This! final0），缺权重时同样先问再下载 ----
/** 清单里节拍检测的模型（`feature = detect_tempo`）。 */
const tempoModel = computed(
  () => modelsStore.catalog.find((m) => m.feature === KIND_TEMPO) ?? null,
);
/** 权重是否就位（mel 前端随包 + final0 已下载 → 后端才认）。 */
const tempoReady = computed(() => models.value.beat_this);
/** 模型是否正在下载。 */
const tempoDownloading = computed(() =>
  tempoModel.value ? modelsStore.installing(tempoModel.value.id) : false,
);

/** 点「检测」：权重缺就弹 MD3 确认框，确认后下载并跳到「管理器 → 可下载 → 模型」。 */
async function runTempo(): Promise<void> {
  const t = track.value;
  if (!t || !canRun.value) return;
  const model = tempoModel.value;
  if (!tempoReady.value) {
    if (!model) return;
    pendingModel.value = model;
    promptOpen.value = true;
    return;
  }
  try {
    await aiStart(KIND_TEMPO, t.id);
  } catch (err) {
    layout.notify(String(err));
  }
}

// 分离模型就位情况变化后：默认选中「已下载」的那个（都没下载就退回第一个），并刷新标签。
// 同时覆盖节拍检测的模型（任一模型下载完成后都要刷新 AI 模型可用状态）。
watch(
  () => modelsStore.catalog.map((m) => `${m.id}:${m.installed}`).join(","),
  () => {
    const list = separateModels.value;
    const current = list.find((m) => m.id === sepModelId.value);
    // 选中的已就位就留着；否则优先跟着已下载的那个，都没有再退回第一个。
    if (!current?.installed && list.length > 0) {
      sepModelId.value = (list.find((m) => m.installed) ?? list[0])!.id;
    }
    void aiModels()
      .then((m) => (models.value = m))
      .catch(() => {});
    void singingModels()
      .then((m) => (singingInfo.value = m))
      .catch(() => {});
  },
  { immediate: true },
);

/** 条目下的说明：先说这条能力**适合什么素材**，跑不了时说明缺什么。
 *
 * **不**把上一次任务的结果写进来——几条能力共用同一份说明，会导致「转 MIDI 的结果
 * 出现在别的条目下面」。任务结果/错误统一走通知（见下面的 watcher）。 */
function noteFor(kind: string): string {
  if (!track.value) return i18n.t("ai.noTrack");
  if (!hasAudio.value) return i18n.t("ai.needsAudio");
  if (kind === KIND_MIDI) return i18n.t("ai.midiNote");
  if (kind === KIND_SEPARATE) {
    return separateModelReady.value ? i18n.t("ai.separateNote") : i18n.t("ai.separateNeedsModel");
  }
  if (kind === KIND_TEMPO) {
    return tempoReady.value ? i18n.t("ai.tempoNote") : i18n.t("ai.tempoNeedsModel");
  }
  return i18n.t("ai.voiceNote");
}

/** 任务跑完时把摘要 / 错误作为通知弹一次（而不是塞进两条能力的描述里）。 */
watch(
  () => ai.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && ai.value.message) layout.notify(ai.value.message);
  },
);

async function run(kind: string): Promise<void> {
  const t = track.value;
  if (!t || !canRun.value) return;
  try {
    await aiStart(kind, t.id);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function stop(): Promise<void> {
  try {
    await aiCancel();
  } catch {
    // 停止失败不打断用户；状态事件会纠正。
  }
}

onMounted(async () => {
  await refresh();
  ai.value = await aiStatus().catch(() => ai.value);
  models.value = await aiModels().catch(() => models.value);
  unlisten = await onAiEvent((s) => (ai.value = s));
  singing.value = await singingStatus().catch(() => singing.value);
  singingInfo.value = await singingModels().catch(() => singingInfo.value);
  await refreshVoices();
  // 从音色库点「用作参考」过来：直接落在刚选的那条人声上（只消费一次）。
  const picked = soundMemories.consumeVoicePick();
  if (picked) {
    refSource.value = "voice";
    singRef.value = `voice:${picked}`;
  }
  stopSingingEvent = await onSingingEvent((s) => (singing.value = s));
  // 分离卡片的「未安装 → 下载」需要知道模型清单与下载进度。
  void modelsStore.load();
  stopModel = modelsStore.watchProgress();
});
onUnmounted(() => {
  unlisten?.();
  stopModel?.();
  stopSingingEvent?.();
});

// 合成跑完时把摘要 / 错误作为通知弹一次。
watch(
  () => singing.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && singing.value.message) layout.notify(singing.value.message);
  },
);
</script>

<template>
  <div class="pane">
    <ul class="list">
      <!-- MCP 服务器：接入地址并入同一个容器 -->
      <!-- 写歌智能体：对话式编曲入口 -->
      <li class="item">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">{{ i18n.t("panes.agent") }}</span>
            <span class="item-note">{{ i18n.t("panes.agentHint") }}</span>
          </div>
          <button class="run-btn" @click="openAgent">{{ i18n.t("ai.run") }}</button>
        </div>
      </li>

      <li class="item">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">{{ i18n.t("ai.mcpServer") }}</span>
            <span class="item-note">{{ mcpOn ? i18n.t("ai.mcpOn") : i18n.t("ai.mcpOff") }}</span>
          </div>
          <button
            class="switch"
            role="switch"
            :class="{ on: mcpOn }"
            :aria-checked="mcpOn"
            :disabled="saving"
            :aria-label="mcpOn ? i18n.t('ai.mcpToggleOff') : i18n.t('ai.mcpToggleOn')"
            @click="toggleMcp"
          >
            <span class="knob" />
          </button>
        </div>
        <div v-if="endpoint" class="item-row">
          <div class="item-main">
            <span class="item-note">{{ i18n.t("ai.endpoint") }}</span>
            <span class="item-name endpoint">{{ endpoint }}</span>
          </div>
          <button class="iface-btn" @click="copyEndpoint">
            <ClipboardDocumentIcon class="w-[13px] h-[13px]" />
            {{ i18n.t("ai.copy") }}
          </button>
        </div>
      </li>

      <!-- 命令行与 Skills：入口，跳到「设置 → 命令行」 -->
      <li class="item">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">{{ i18n.t("ai.cli") }}</span>
            <span class="item-note">{{ i18n.t("ai.cliHint") }}</span>
          </div>
          <button class="iface-btn" @click="openCliSettings">
            <BoltIcon class="w-[13px] h-[13px]" />
            {{ i18n.t("ai.openCli") }}
          </button>
        </div>
      </li>

      <!-- 音频转 MIDI：只要一个按钮（处理中变成「停止」） -->
      <li class="item" :class="{ busy: busy(KIND_MIDI) }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.toMidi") }}
              <span class="badge" :class="{ on: hasModel(KIND_MIDI) }">{{ backendLabel(KIND_MIDI) }}</span>
            </span>
            <span class="item-note">{{ busy(KIND_MIDI) ? i18n.t("ai.busy") : noteFor(KIND_MIDI) }}</span>
          </div>
          <button v-if="!busy(KIND_MIDI)" class="run-btn" :disabled="!canRun" @click="run(KIND_MIDI)">
            {{ i18n.t("ai.run") }}
          </button>
          <button v-else class="run-btn" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <div v-if="busy(KIND_MIDI)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </li>

      <!-- 人声降噪 / 去混响：LocalVQE（16 kHz 语音模型） -->
      <li class="item" :class="{ busy: busy(KIND_VOICE) }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.enhanceVoice") }}
              <span class="badge" :class="{ on: hasModel(KIND_VOICE) }">
                {{ backendLabel(KIND_VOICE) }}
              </span>
            </span>
            <span class="item-note">{{ busy(KIND_VOICE) ? i18n.t("ai.busy") : noteFor(KIND_VOICE) }}</span>
          </div>
          <button
            v-if="!busy(KIND_VOICE)"
            class="run-btn"
            :disabled="!canRun"
            @click="run(KIND_VOICE)"
          >
            {{ i18n.t("ai.runEnhance") }}
          </button>
          <button v-else class="run-btn" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <div v-if="busy(KIND_VOICE)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </li>

      <!-- 音源分离：HTDemucs 模型（可选 4 分轨 / 6 分轨；缺权重时先问再下载，无内置兜底） -->
      <li class="item" :class="{ busy: busy(KIND_SEPARATE) || sepDownloading }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.separate") }}
              <span class="badge" :class="{ on: separateModelReady }">
                {{ i18n.t(separateModelReady ? "ai.backendModel" : "ai.modelMissing") }}
              </span>
            </span>
            <span class="item-note">
              {{
                sepDownloading
                  ? i18n.t("ai.downloadingModel")
                  : busy(KIND_SEPARATE)
                    ? i18n.t("ai.busy")
                    : noteFor(KIND_SEPARATE)
              }}
            </span>
          </div>

          <!-- 正在下载模型：可停止 -->
          <button v-if="sepDownloading" class="run-btn" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <!-- 正在分离：可停止 -->
          <button v-else-if="busy(KIND_SEPARATE)" class="run-btn" @click="stop">
            {{ i18n.t("ai.stop") }}
          </button>
          <!-- 开始：权重缺时先弹「是否下载」并跳转到「管理器」 -->
          <button v-else class="run-btn" :disabled="!canRun" @click="runSeparation">
            {{ separateModelReady ? i18n.t("ai.runSeparate") : i18n.t("ai.downloadAndRun") }}
          </button>
        </div>

        <!-- 模型选择 -->
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.separateModel") }}</span>
          <Md3Select
            v-model="sepModelId"
            :options="sepOptions"
            :label="i18n.t('ai.separateModel')"
            :disabled="ai.running || sepDownloading"
          />
        </div>

        <!-- 进度条：模型下载 或 分离处理，贴在条目下边缘 -->
        <div v-if="sepDownloading" class="progress">
          <div
            class="progress-fill"
            :class="{ indeterminate: modelsStore.progressPct === null }"
            :style="modelsStore.progressPct === null ? undefined : { width: `${modelsStore.progressPct}%` }"
          />
        </div>
        <div v-else-if="busy(KIND_SEPARATE)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </li>

      <!-- 节拍检测：Beat This!（一个全局 BPM + 拍号，并把片段首重拍对齐到小节线） -->
      <li class="item" :class="{ busy: busy(KIND_TEMPO) || tempoDownloading }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.tempo") }}
              <span class="badge" :class="{ on: tempoReady }">
                {{ i18n.t(tempoReady ? "ai.backendModel" : "ai.modelMissing") }}
              </span>
            </span>
            <span class="item-note">
              {{
                tempoDownloading
                  ? i18n.t("ai.downloadingModel")
                  : busy(KIND_TEMPO)
                    ? i18n.t("ai.tempoBusy")
                    : noteFor(KIND_TEMPO)
              }}
            </span>
            <!-- 适用范围常驻显示：避免用户拿自由速度 / 氛围音试完觉得「这功能没用」。 -->
            <span class="item-hint">{{ i18n.t("ai.tempoScope") }}</span>
          </div>

          <!-- 正在下载模型 / 正在检测：可停止 -->
          <button v-if="tempoDownloading" class="run-btn" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else-if="busy(KIND_TEMPO)" class="run-btn" @click="stop">
            {{ i18n.t("ai.stop") }}
          </button>
          <!-- 开始：权重缺时先弹「是否下载」并跳转到「管理器」 -->
          <button v-else class="run-btn" :disabled="!canRun" @click="runTempo">
            {{ tempoReady ? i18n.t("ai.runTempo") : i18n.t("ai.downloadAndDetect") }}
          </button>
        </div>

        <!-- 进度条：模型下载 或 检测处理，贴在条目下边缘 -->
        <div v-if="tempoDownloading" class="progress">
          <div
            class="progress-fill"
            :class="{ indeterminate: modelsStore.progressPct === null }"
            :style="modelsStore.progressPct === null ? undefined : { width: `${modelsStore.progressPct}%` }"
          />
        </div>
        <div v-else-if="busy(KIND_TEMPO)" class="progress">
          <!-- 进度**不确定**：Beat This! 的推理是一整段不可打断的调用（crate 没有进度回调），
               给百分比只会「停在 5%、然后突然满」。用不定长条纹诚实表示「在跑」。 -->
          <div class="progress-fill indeterminate" />
        </div>
      </li>

      <!-- 歌声合成：SoulX-Singer 的 9 张 ONNX 图（MIDI + 歌词 + 参考声线 → 新音频轨） -->
      <li class="item" :class="{ busy: singBusy || singDownloading }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.singing") }}
              <span class="badge" :class="{ on: singReady }">
                {{ i18n.t(singReady ? "ai.backendModel" : "ai.modelMissing") }}
              </span>
            </span>
            <span class="item-note">
              {{
                singDownloading
                  ? i18n.t("ai.downloadingModel")
                  : singBusy
                    ? i18n.t("ai.singingBusy")
                    : singReady
                      ? i18n.t("ai.singingNote")
                      : i18n.t("ai.singingNeedsModel")
              }}
            </span>
            <span v-if="!midiTracks.length" class="item-hint">
              {{ i18n.t("ai.singingNeedsMidi") }}
            </span>
            <span v-else-if="!audioTracks.length" class="item-hint">
              {{ i18n.t("ai.singingNeedsTone") }}
            </span>
            <span v-else class="item-hint">{{ i18n.t("ai.singingToneHint") }}</span>
          </div>

          <button v-if="singDownloading" class="run-btn" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else-if="singBusy" class="run-btn" @click="stopSinging">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else class="run-btn" :disabled="!singTrack || !hasTone" @click="runSinging">
            {{ singReady ? i18n.t("ai.runSinging") : i18n.t("ai.downloadAndSing") }}
          </button>
        </div>

        <!-- 输入：MIDI 轨 / 歌词入口 / 参考声线 / 步数 -->
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.singingTrack") }}</span>
          <Md3Select
            v-model="singTrackId"
            :options="singOptions"
            :label="i18n.t('ai.singingTrack')"
            :empty="i18n.t('ai.singingNeedsMidi')"
            :disabled="singBusy"
          />
        </div>

        <!-- 歌词在钢琴卷帘里逐音填（这里只放入口，不再重复一个整串歌词框） -->
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.lyrics") }}</span>
          <span class="item-hint lyric-hint">{{ i18n.t("ai.lyricsInEditor") }}</span>
          <button
            class="iface-btn"
            :disabled="!singTrack || singBusy"
            @click="openLyricsEditor"
          >
            {{ i18n.t("ai.goFillLyrics") }}
          </button>
        </div>

        <!-- 参考声线：先选来源（工程轨道 / 音色库），再选具体那条 -->
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.refSource") }}</span>
          <Md3Select
            v-model="sourceChoice"
            :options="sourceOptions"
            :label="i18n.t('ai.refSource')"
            :disabled="singBusy"
          />
        </div>
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.singingTone") }}</span>
          <Md3Select
            v-model="singRef"
            :options="refOptions"
            :label="i18n.t('ai.singingTone')"
            :empty="
              refSource === 'voice' ? i18n.t('ai.voiceLibraryEmpty') : i18n.t('ai.singingNeedsTone')
            "
            :disabled="singBusy"
          />
        </div>

        <div class="tone-actions">
          <span class="item-hint">
            <template v-if="refSource === 'track'">
              {{ i18n.t("ai.voiceSaveHint") }}
            </template>
            <template v-else-if="singVoice">
              {{ i18n.t("ai.voiceFromLibrary", { seconds: Math.round(singVoice.seconds) }) }}
            </template>
            <template v-else>{{ i18n.t("ai.voiceLibraryEmpty") }}</template>
          </span>
          <div class="tone-buttons">
            <button
              v-if="refSource === 'track'"
              class="iface-btn"
              :disabled="singBusy || !singRefTrackId"
              @click="beginSaveVoice"
            >
              {{ i18n.t("ai.voiceSave") }}
            </button>
            <button class="iface-btn" :disabled="singBusy" @click="importVoice">
              {{ i18n.t("ai.voiceImport") }}
            </button>
            <button class="iface-btn" :disabled="singBusy" @click="openVoiceLibrary">
              {{ i18n.t("ai.manageVoices") }}
            </button>
          </div>
        </div>

        <!-- 存进音色库：内联命名，不弹窗 -->
        <div v-if="voiceSaveOpen" class="param-field">
          <span class="param-name">{{ i18n.t("ai.voiceName") }}</span>
          <input
            v-model="voiceDraftName"
            class="voice-name"
            type="text"
            maxlength="60"
            spellcheck="false"
            :placeholder="i18n.t('ai.voiceDefaultName')"
            @keydown.enter.prevent="confirmSaveVoice"
          />
          <button class="iface-btn" @click="confirmSaveVoice">{{ i18n.t("common.save") }}</button>
          <button class="iface-btn" @click="voiceSaveOpen = false">{{ i18n.t("common.cancel") }}</button>
        </div>
        <div class="param-field">
          <span class="param-name">{{ i18n.t("ai.steps") }}</span>
          <Md3Select
            v-model="stepChoice"
            :options="stepOptions"
            :label="i18n.t('ai.steps')"
            :disabled="singBusy"
          />
        </div>

        <!-- 进度条：模型下载 或 合成，贴在条目下边缘 -->
        <div v-if="singDownloading" class="progress">
          <div
            class="progress-fill"
            :class="{ indeterminate: modelsStore.progressPct === null }"
            :style="modelsStore.progressPct === null ? undefined : { width: `${modelsStore.progressPct}%` }"
          />
        </div>
        <div v-else-if="singBusy" class="progress">
          <div class="progress-fill" :style="{ width: singProgressPct }" />
        </div>
      </li>

    </ul>

    <!-- 「是否下载该模型」用应用内 MD3 模态框（不用系统原生框）。 -->
    <ConfirmDialog
      v-model:open="promptOpen"
      :title="i18n.t('dialog.downloadModelTitle')"
      :message="downloadPromptMessage"
      :confirm-label="i18n.t('dialog.downloadModelOk')"
      @confirm="confirmDownloadModel"
    />
  </div>
</template>

<style scoped>
/* 与 `PluginsPane.vue` 同一种骨架：一个可滚动容器 + 一组卡片条目。 */
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 0;
}
.list {
  list-style: none;
  margin: 0;
  padding: 0 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.item {
  position: relative;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  border-radius: 14px;
  background-color: var(--app-inset);
}
/* 一条条目内部的一行。 */
.item-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.item-main {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
  flex: 1 1 auto;
}
.item-name {
  font-size: 13px;
  font-weight: 500;
  color: rgb(var(--md-on-surface));
}
.item-note {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
/* 适用范围（比说明更弱一档）：常驻显示，不随任务状态变化。 */
.item-hint {
  font-size: 11px;
  line-height: 1.45;
  color: rgb(var(--md-on-surface-variant) / 0.78);
}
/* 后端标签：权重就位时点亮（走真模型），否则是灰的内置 DSP。 */
.badge {
  margin-left: 6px;
  padding: 0 6px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 600;
  line-height: 15px;
  vertical-align: 1px;
  background-color: var(--app-inset);
  color: rgb(var(--md-on-surface-variant));
}
.badge.on {
  background-color: rgb(var(--md-primary) / 0.14);
  color: rgb(var(--md-primary));
}
.endpoint {
  font-weight: 400;
  word-break: break-all;
  font-size: 12px;
}
/* 模型选择行：名称在左、下拉靠右。`Md3Select` 是 `width:100%`，配 `space-between`
   正好占满剩余宽度——**不要**去掉它，否则标签与下拉会各占一行。 */
.param-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}
.param-name {
  /* `flex: 0 0 auto` 不能少：`Md3Select` 是 `width:100%`，标签不锁宽就会被它挤成
     二十来像素、末尾被省略号吃掉（「模型」显示成「模..」）。检查器的 `.row-label` 同理。 */
  flex: 0 0 auto;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 歌词输入：多行、等宽感、与面板的整体密度一致。 */
.lyrics-input {
  width: 100%;
  box-sizing: border-box;
  padding: 8px 10px;
  border: none;
  border-radius: 10px;
  outline: none;
  resize: vertical;
  min-height: 66px;
  font-family: inherit;
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface));
  background-color: rgb(var(--md-surface-container-high) / 0.6);
}
.lyrics-input:focus {
  box-shadow: inset 0 0 0 1px rgb(var(--md-primary) / 0.6);
}
.lyrics-input:disabled {
  opacity: 0.6;
}
/* 参考声线下面的一行动作（存进音色库 / 删除）。 */
.lyric-hint {
  flex: 1 1 auto;
  min-width: 0;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tone-actions {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
}
.tone-actions .item-hint {
  width: 100%;
}
.tone-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.voice-name {
  flex: 1 1 auto;
  min-width: 0;
  box-sizing: border-box;
  height: 30px;
  padding: 0 10px;
  border: none;
  border-radius: 10px;
  outline: none;
  font-family: inherit;
  font-size: 12px;
  color: rgb(var(--md-on-surface));
  background-color: rgb(var(--md-surface-container-high) / 0.6);
}
.voice-name:focus {
  box-shadow: inset 0 0 0 1px rgb(var(--md-primary) / 0.6);
}
/* 进度条贴在条目**下边缘**。 */
.progress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 3px;
  background-color: rgb(var(--md-primary) / 0.15);
}
.progress-fill {
  height: 100%;
  width: 0;
  background-color: rgb(var(--md-primary));
  transition: width 120ms linear;
}
/* 总量未知（如模型下载的校验/安装阶段）时的不定长条纹。 */
.progress-fill.indeterminate {
  width: 35%;
  animation: ai-indeterminate 1.1s ease-in-out infinite;
}
@keyframes ai-indeterminate {
  0% {
    margin-left: -35%;
  }
  100% {
    margin-left: 100%;
  }
}
@media (prefers-reduced-motion: reduce) {
  .progress-fill.indeterminate {
    width: 100%;
    animation: none;
  }
}
/* 启动 / 停止开关 */
.switch {
  flex: 0 0 auto;
  width: 40px;
  height: 22px;
  padding: 2px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  transition: background-color 0.16s ease;
}
.switch.on {
  background-color: rgb(var(--md-primary));
}
.switch:disabled {
  opacity: 0.5;
  cursor: default;
}
.switch .knob {
  width: 18px;
  height: 18px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface));
  box-shadow: var(--md-elevation-1);
  transform: translateX(0);
  transition: transform 0.16s ease;
}
.switch.on .knob {
  transform: translateX(18px);
}
.switch:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
/* 运行 / 停止键：与「调音台」面板的运行键**逐字同一套**（实心主色 + on-primary、30px 药丸）。
   同一件事在两个面板里必须长得一样——用 `.iface-btn` 那套描边样式会和调音台对不上。 */
.run-btn {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 14px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: filter 0.14s ease;
}
.run-btn:hover:not(:disabled) {
  filter: brightness(1.08);
}
.run-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.run-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
/* 按钮：与插件面板的「界面」按钮同一套。 */
.iface-btn {
  flex: 0 0 auto;
  height: 26px;
  padding: 0 10px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-primary));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: background-color 0.14s ease;
}
.iface-btn:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.08);
}
.iface-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.iface-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
</style>
