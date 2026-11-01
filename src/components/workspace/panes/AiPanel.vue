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
import AiYuePage from "@/components/workspace/panes/ai/AiYuePage.vue";
import AiSingingPage from "@/components/workspace/panes/ai/AiSingingPage.vue";
import AiSvcPage from "@/components/workspace/panes/ai/AiSvcPage.vue";
import "@/components/workspace/panes/ai/aiPage.css";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import EngineBackendPicker from "@/components/common/EngineBackendPicker.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import {
  aiBackendInfo,
  aiCancel,
  aiModels,
  aiSetBackend,
  aiStart,
  aiStatus,
  mcpStatus,
  onAiEvent,
  onSingingEvent,
  onSvcEvent,
  setMcpConfig,
  singingCancel,
  singingStatus,
  svcCancel,
  svcStatus,
} from "@/api/tauri";
import type {
  AiModels,
  AiStatus,
  BackendInfo,
  BackendPreference,
  McpStatus,
  ModelStatus,
  SingingStatus,
  SvcStatus,
} from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useRuntimesStore } from "@/stores/runtimes";
import { useModelChoice } from "@/composables/useModelChoice";
import { useYueStore } from "@/stores/yue";
import { useI18n } from "@/i18n";
import { copyText } from "@/utils/clipboard";
import { humanSize, missingBytes, modelName } from "@/utils/models";
import { restartApp } from "@/utils/restart";

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
const i18n = useI18n();

/**
 * 面板里正在显示的子页（`null` = 卡片列表）。
 *
 * 三个 AI 功能（写歌 / 歌声合成 / 音色替换）的表单都**占满侧栏**，而不是跳到工作区中间：
 * 它们是右栏的延续，左边的时间线要一直看得见。
 */
const aiPage = ref<null | "yue" | "singing" | "svc">(null);
/** 子页里自己会弹完成通知，列表这边让路（两边都在挂载状态）。 */
const inAiPage = computed(() => aiPage.value !== null);

// ---- 顶部标签：全部 / 生成 / 转换（「全部」里用分隔标题分组）----
const aiTab = ref<"all" | "generate" | "convert">("all");
const AI_TABS = ["all", "generate", "convert"] as const;
function aiTabLabel(tab: (typeof AI_TABS)[number]): string {
  return i18n.t(`ai.tab.${tab}`);
}

// ---- 运行后端：与设置页同一套（切换要重启）----
const backend = ref<BackendInfo | null>(null);
const backendBusy = ref(false);
const backendRestart = ref(false);
/** 重启提示正文：开发模式下补一句「不会自动重启」。 */
const backendRestartBody = computed(() => {
  const body = i18n.t("ai.backendRestartBody");
  return import.meta.env.DEV ? `${body} ${i18n.t("settings.ai.restartDevHint")}` : body;
});
/** audio.cpp 引擎的后端包（独立于 ONNX Runtime，切换只需重启引擎）。 */
const runtimes = useRuntimesStore();
/** 本机平台支持的后端（CPU 之外通常只有 DirectML / CoreML / CUDA 之一）。 */
const backendOptions = computed(() => backend.value?.options.filter((o) => o.available) ?? []);
/** 分段控件用的选项（文案走 i18n；缺运行时的给个悬停提示）。 */
const backendSegOptions = computed<SegmentedOption[]>(() =>
  backendOptions.value.map((o) => ({
    value: o.kind,
    label: backendLabelOf(o.kind, o.label),
    disabled: backendBusy.value,
    title: o.runtime_installed ? undefined : i18n.t("ai.backendMissing"),
  })),
);
function onBackendPick(value: string): void {
  void pickBackend(value as BackendPreference);
}
/** 当前后端的一句话说明（词典没这条就退回后端下发的中文）。 */
const backendNote = computed(() => {
  const current = backend.value?.options.find((o) => o.kind === backend.value?.preference);
  if (!current) return i18n.t("ai.backendHint");
  const key = `backend.${current.kind}.note`;
  const text = i18n.t(key);
  return text === key ? current.note : text;
});
function backendLabelOf(kind: string, fallback: string): string {
  const key = `backend.${kind}`;
  const text = i18n.t(key);
  return text === key ? fallback : text;
}

/** CUDA 组件 id → 界面上的名字（面板里没有运行时清单，用固定名称）。 */
function partLabel(id: string): string {
  switch (id) {
    case "cudart":
    case "cudart13":
      return id.endsWith("13") ? "CUDA Runtime 13" : "CUDA Runtime";
    case "cublas":
    case "cublas13":
      return id.endsWith("13") ? "cuBLAS 13" : "cuBLAS";
    case "cufft":
    case "cufft13":
      return id.endsWith("13") ? "cuFFT 12" : "cuFFT";
    case "cudnn":
    case "cudnn13":
      return "cuDNN 9";
    default:
      return id;
  }
}

/** CUDA 看着能用、实际会回落 CPU 的两种情况：组件缺 / provider 没注册成功。 */
const backendWarning = computed(() => {
  const info = backend.value;
  if (!info || !info.preference.startsWith("cuda")) return "";
  const cuda = info.options.find((o) => o.kind === info.preference);
  if (!cuda || !cuda.available) return "";
  if (!cuda.runtime_installed) return i18n.t("ai.backendCoreMissing");
  const missing = cuda.missing ?? [];
  if (missing.length > 0) {
    return i18n.t("ai.backendPartsMissing", { parts: missing.map(partLabel).join("、") });
  }
  // 启动后还没建会话时运行时尚未加载，`ep_ready` 为 false 属于正常；只有加载过
  // 但 provider 仍没注册成功才提示。
  if (!info.ep_ready && info.loaded_runtime) return i18n.t("ai.backendNotActive");
  return "";
});

async function loadBackendInfo(): Promise<void> {
  try {
    backend.value = await aiBackendInfo();
  } catch {
    backend.value = null;
  }
}

async function pickBackend(kind: BackendPreference): Promise<void> {
  if (!backend.value || backend.value.preference === kind || backendBusy.value) return;
  const option = backend.value.options.find((o) => o.kind === kind);
  if (!option) return;
  // 本体没装、或 CUDA 组件没装齐：切过去也只会静默跑 CPU，先去下载管理器补齐。
  const missing = option.missing ?? [];
  if (!option.runtime_installed || missing.length > 0) {
    layout.notify(
      missing.length > 0
        ? i18n.t("ai.backendPartsMissing", { parts: missing.map(partLabel).join("、") })
        : i18n.t("ai.backendMissing"),
    );
    layout.openTab("manager");
    return;
  }
  backendBusy.value = true;
  try {
    backend.value = await aiSetBackend(kind);
    if (backend.value?.restart_required) {
      backendRestart.value = true;
    } else {
      const loaded = backend.value?.loaded_runtime === option.runtime;
      layout.notify(
        i18n.t(
          loaded ? "settings.ai.backendAppliedCurrent" : "settings.ai.backendAppliedNext",
          { backend: backendLabelOf(kind, option.label) },
        ),
      );
    }
  } catch (err) {
    layout.notify(String(err));
  } finally {
    backendBusy.value = false;
  }
}

// ---- YuE2 写歌 ----
/** 已装 / 未装的 YuE2 权重都在下拉里；选过哪一档跨会话记住（卡片与子页共用）。 */
const yueChoice = useModelChoice("compose", () =>
  modelsStore.catalog.filter((m) => m.feature === "compose"),
);
const yueDraftModel = yueChoice.id;
const chosenYueModel = yueChoice.current;
/** 下拉选项与其它模型卡片同一套（名称 + 「已下载 / 需下载 · 体积」）。 */
const yueModelOptions = yueChoice.selectOptions;

/** 「开始」：引擎没装先进页（页里强制选引擎）；模型没下就弹下载确认。 */
function startYue(): void {
  // 安装包默认不带引擎：先让写歌页的安装向导处理，别先去下几 GB 权重。
  if (!runtimes.items.some((r) => r.group === "audiocpp" && r.installed)) {
    aiPage.value = "yue";
    return;
  }
  const model = chosenYueModel.value;
  if (model && !model.installed) {
    pendingModel.value = model;
    promptOpen.value = true;
    return;
  }
  aiPage.value = "yue";
}

// ---- YuE2 本地写歌 ----
/** 引擎子进程 + 一次生成任务（表单在模态框里，条目只留入口与状态）。 */
const yue = useYueStore();
/** 后端 id → 界面上的显卡名字。 */
function deviceLabel(device: string): string {
  switch (device) {
    case "cuda13":
      return "CUDA 13";
    case "cuda12":
      return "CUDA 12";
    case "vulkan":
      return "Vulkan";
    case "metal":
      return "Metal";
    default:
      return "CPU";
  }
}

/** 条目上的小徽标：未启动 / 加载中 / 就绪 / 出错。 */
const yueBadge = computed(() => {
  const state = yue.engine?.state ?? "stopped";
  if (state === "ready") return deviceLabel(yue.engine?.device ?? "cpu");
  if (state === "starting") return i18n.t("ai.yueBadgeLoading");
  if (state === "error") return i18n.t("ai.yueBadgeError");
  return i18n.t("ai.yueBadgeStopped");
});

/** 条目 / 对话框里那一行说明。 */
const yueStatusLine = computed(() => {
  const engine = yue.engine;
  if (yue.busy) {
    return i18n.t("ai.yueElapsed", { seconds: Math.round(yue.job.elapsed) });
  }
  if (!engine || engine.state === "stopped") return i18n.t("ai.yueHint");
  if (engine.state === "error") {
    return engine.message ?? engine.log ?? i18n.t("ai.yueEngineError");
  }
  if (engine.state === "starting") {
    return engine.message ?? i18n.t("ai.yueEngineStarting");
  }
  return `${i18n.t("ai.yueEngineReady", {
    device: deviceLabel(engine.device),
    model: engine.model,
  })} · ${i18n.t("yue.readyHint")}`;
});


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
/** 音色替换状态订阅的取消函数。 */
let stopSvcEvent: (() => void) | null = null;
/** YuE2 引擎 / 任务状态订阅的取消函数。 */
let stopYueEvent: (() => void) | null = null;

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
/** 分离模型（4 分轨 / 6 分轨）：选过哪一档跨会话记住。 */
const {
  current: sepModel,
  id: sepModelId,
  ready: separateModelReady,
  downloading: sepDownloading,
  selectOptions: sepOptions,
} = useModelChoice("separate", () => modelsStore.catalog.filter((m) => m.feature === "separate"));

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

// ---- 歌声合成 / 音色替换：列表里只留「选模型 + 开始」，参数都在各自独立的页面里 ----
/** 有音符的 MIDI 轨道（歌声合成的输入）。 */
const midiTracks = computed(() =>
  (project.project?.tracks ?? []).filter(
    (t) => !t.clips.every((c) => c.audio) && t.clips.some((c) => (c.notes?.length ?? 0) > 0),
  ),
);
/** 有音频片段的轨道（可当参考声线，也是音色替换的输入）。 */
const audioTracks = computed(() =>
  (project.project?.tracks ?? []).filter((t) => t.clips.some((c) => c.audio)),
);

const singing = ref<SingingStatus>({ running: false, progress: 0, message: null });
/** 歌声合成（fp16 / fp32）：选过哪一档跨会话记住。 */
const {
  current: singModel,
  id: singModelId,
  ready: singReady,
  downloading: singDownloading,
  selectOptions: singModelOptions,
} = useModelChoice("sing", () => modelsStore.catalog.filter((m) => m.feature === "sing"));
const singBusy = computed(() => singing.value.running);

/** 点「开始」：模型没下就先问下载；没有清单条目也先进页（页里能直接去下载）。 */
function startSinging(): void {
  const model = singModel.value;
  if (model && !model.installed) {
    pendingModel.value = model;
    promptOpen.value = true;
    return;
  }
  aiPage.value = "singing";
}

async function stopSinging(): Promise<void> {
  try {
    await singingCancel();
  } catch {
    // 状态事件会纠正。
  }
}

const svc = ref<SvcStatus>({ running: false, progress: 0, message: null });
/** 音色替换（fp16_hq / fp32）：选过哪一档跨会话记住。 */
const {
  current: svcModel,
  id: svcModelId,
  ready: svcReady,
  downloading: svcDownloading,
  selectOptions: svcModelOptions,
} = useModelChoice("voice_swap", () => modelsStore.catalog.filter((m) => m.feature === "voice_swap"));
const svcBusy = computed(() => svc.value.running);

/** 点「开始」：同歌声合成，进音色替换页。 */
function startSvc(): void {
  const model = svcModel.value;
  if (model && !model.installed) {
    pendingModel.value = model;
    promptOpen.value = true;
    return;
  }
  aiPage.value = "svc";
}

async function stopSvc(): Promise<void> {
  try {
    await svcCancel();
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

// 任一模型下载完成后刷新「哪些能力已经能走真模型」的标签。
// 注意：**不在这里改用户选的档**——选择归用户，只有清单里彻底没有那一档时才回落。
watch(
  () => modelsStore.catalog.map((m) => `${m.id}:${m.installed}`).join(","),
  () => {
    void aiModels()
      .then((m) => (models.value = m))
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
  stopSingingEvent = await onSingingEvent((s) => (singing.value = s));
  svc.value = await svcStatus().catch(() => svc.value);
  stopSvcEvent = await onSvcEvent((s) => (svc.value = s));
  // 分离卡片的「未安装 → 下载」需要知道模型清单与下载进度。
  void modelsStore.load();
  stopModel = modelsStore.watchProgress();
  // 运行后端（与设置页同一套）+ YuE2 的默认权重档。
  void loadBackendInfo();
  // audio.cpp 引擎的后端包：AI 面板里的选择器和设置页读同一个 store。
  // 这里必须自己拉一次——选择器在清单为空时仍会渲染，但不能指望它替父组件加载。
  void runtimes.load();
  void modelsStore.load();
  // YuE2：引擎状态 + 任务状态（写歌卡片的徽标 / 进度都靠它）。
  void yue.load();
  stopYueEvent = yue.watch();
});
onUnmounted(() => {
  unlisten?.();
  stopModel?.();
  stopSingingEvent?.();
  stopSvcEvent?.();
  stopYueEvent?.();
});

// 合成跑完时把摘要 / 错误作为通知弹一次。
watch(
  () => singing.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && singing.value.message && !inAiPage.value) {
      layout.notify(singing.value.message);
    }
  },
);

// 音色替换跑完时同样只弹一次通知。
watch(
  () => svc.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && svc.value.message && !inAiPage.value) {
      layout.notify(svc.value.message);
    }
  },
);

// YuE2 写歌跑完 / 失败时弹一次通知（音频轨已经进工程了）。
watch(
  () => yue.job.state,
  (state, before) => {
    if (before !== "running") return;
    if (state === "done") {
      if (yue.job.message && !inAiPage.value) layout.notify(yue.job.message);
      void yue.load();
    } else if (state === "failed" && yue.job.message && !inAiPage.value) {
      layout.notify(yue.job.message);
    }
  },
);
</script>

<template>
  <div class="pane">
    <!-- 三个子页占满侧栏（从卡片进来，左上角返回到列表） -->
    <AiYuePage v-if="aiPage === 'yue'" @back="aiPage = null" />
    <AiSingingPage v-else-if="aiPage === 'singing'" @back="aiPage = null" />
    <AiSvcPage v-else-if="aiPage === 'svc'" @back="aiPage = null" />

    <template v-else>
    <!-- 顶部标签：全部 / 生成 / 转换（全部里用分隔标题分组） -->
    <div class="ai-tabs" role="tablist" :aria-label="i18n.t('ai.tabLabel')">
      <button
        v-for="t in AI_TABS"
        :key="t"
        class="ai-tab"
        :class="{ on: aiTab === t }"
        role="tab"
        :aria-selected="aiTab === t"
        @click="aiTab = t"
      >
        {{ aiTabLabel(t) }}
      </button>
    </div>
    <ul class="list">
      <!-- MCP 服务器：接入地址并入同一个容器 -->
      <!-- 写歌智能体：对话式编曲入口 -->
      <li class="item" v-if="aiTab === 'all'" :style="{ order: 10 }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">{{ i18n.t("panes.agent") }}</span>
            <span class="item-note">{{ i18n.t("panes.agentHint") }}</span>
          </div>
          <button class="run-btn" @click="openAgent">{{ i18n.t("ai.run") }}</button>
        </div>
      </li>

      <!-- YuE2 本地写歌：引擎 + 一次生成（表单在模态框里，条目只留入口与状态） -->
      <li
        class="item"
        v-if="aiTab !== 'convert'"
        :style="{ order: 50 }"
        :class="{ busy: yue.busy }"
      >
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.yueTitle") }}
              <span class="badge" :class="{ on: yue.ready }">{{ yueBadge }}</span>
            </span>
            <span class="item-note">{{ yueStatusLine }}</span>
          </div>
        </div>
        <div class="item-row">
          <Md3Select
            v-model="yueDraftModel"
            class="model-select"
            :options="yueModelOptions"
            :label="i18n.t('ai.yueModel')"
          />
          <button class="run-btn" :disabled="yue.starting" @click="startYue">
            {{ chosenYueModel?.installed ? i18n.t("ai.yueOpen") : i18n.t("ai.yueDownloadAndWrite") }}
          </button>
        </div>
        <div v-if="yue.busy" class="progress">
          <div class="progress-fill indeterminate" />
        </div>
      </li>

      <li class="item" v-if="aiTab === 'all'" :style="{ order: 20 }">
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
      <li class="item" v-if="aiTab === 'all'" :style="{ order: 30 }">
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

      <!-- 运行后端：与设置页同一套；换后端 = 换进程里的运行时，要重启 -->
      <li class="item" v-if="aiTab === 'all'" :style="{ order: 40 }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">{{ i18n.t("ai.backend") }}</span>
            <span class="item-note">{{ backendNote }}</span>
          </div>
        </div>
        <div class="item-row">
          <Md3Segmented
            :model-value="backend?.preference ?? ''"
            :options="backendSegOptions"
            size="sm"
            :aria-label="i18n.t('ai.backend')"
            @update:model-value="onBackendPick"
          />
        </div>
        <div class="item-row" v-if="backendWarning">
          <span class="item-note warn">{{ backendWarning }}</span>
        </div>
      </li>

      <!-- audio.cpp 引擎后端：与 ONNX Runtime 那套相互独立；切换只重启引擎，不用重启应用 -->
      <li class="item" v-if="aiTab === 'all'" :style="{ order: 41 }">
        <EngineBackendPicker compact />
      </li>

      <li class="ai-sep sep-generate" v-if="aiTab === 'all'">
        {{ i18n.t("ai.tab.generate") }}
      </li>

      <!-- 音频转 MIDI：只要一个按钮（处理中变成「停止」） -->
      <li
        class="item"
        v-if="aiTab !== 'generate'"
        :style="{ order: 100 }"
        :class="{ busy: busy(KIND_MIDI) }"
      >
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
      <li
        class="item"
        v-if="aiTab !== 'generate'"
        :style="{ order: 90 }"
        :class="{ busy: busy(KIND_VOICE) }"
      >
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
      <li class="ai-sep sep-convert" v-if="aiTab === 'all'">
        {{ i18n.t("ai.tab.convert") }}
      </li>

      <li
        class="item"
        v-if="aiTab !== 'generate'"
        :style="{ order: 80 }"
        :class="{ busy: busy(KIND_SEPARATE) || sepDownloading }"
      >
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
      <li
        class="item"
        v-if="aiTab !== 'generate'"
        :style="{ order: 110 }"
        :class="{ busy: busy(KIND_TEMPO) || tempoDownloading }"
      >
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

      <!-- 歌声合成：参数在独立页里，列表只留「选模型 + 开始」 -->
      <li
        class="item"
        v-if="aiTab !== 'convert'"
        :style="{ order: 60 }"
        :class="{ busy: singBusy || singDownloading }"
      >
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
          </div>
        </div>
        <div class="item-row">
          <Md3Select
            v-model="singModelId"
            class="model-select"
            :options="singModelOptions"
            :label="i18n.t('ai.singingModel')"
          />
          <button v-if="singDownloading" class="run-btn" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else-if="singBusy" class="run-btn" @click="stopSinging">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else class="run-btn" @click="startSinging">
            {{ singReady ? i18n.t("ai.runSinging") : i18n.t("ai.downloadAndSing") }}
          </button>
        </div>
        <div v-if="singDownloading" class="progress">
          <div
            class="progress-fill"
            :class="{ indeterminate: modelsStore.progressPct === null }"
            :style="modelsStore.progressPct === null ? undefined : { width: `${modelsStore.progressPct}%` }"
          />
        </div>
        <div v-else-if="singBusy" class="progress">
          <div class="progress-fill indeterminate" />
        </div>
      </li>

      <!-- 音色替换：同上，参数在独立页里 -->
      <li
        class="item"
        v-if="aiTab !== 'convert'"
        :style="{ order: 70 }"
        :class="{ busy: svcBusy || svcDownloading }"
      >
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.voiceSwap") }}
              <span class="badge" :class="{ on: svcReady }">
                {{ i18n.t(svcReady ? "ai.backendModel" : "ai.modelMissing") }}
              </span>
            </span>
            <span class="item-note">
              {{
                svcDownloading
                  ? i18n.t("ai.downloadingModel")
                  : svcBusy
                    ? i18n.t("ai.voiceSwapBusy")
                    : svcReady
                      ? i18n.t("ai.voiceSwapNote")
                      : i18n.t("ai.voiceSwapNeedsModel")
              }}
            </span>
            <span v-if="!audioTracks.length" class="item-hint">
              {{ i18n.t("ai.voiceSwapNeedsAudio") }}
            </span>
          </div>
        </div>
        <div class="item-row">
          <Md3Select
            v-model="svcModelId"
            class="model-select"
            :options="svcModelOptions"
            :label="i18n.t('ai.singingModel')"
          />
          <button v-if="svcDownloading" class="run-btn" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else-if="svcBusy" class="run-btn" @click="stopSvc">
            {{ i18n.t("ai.stop") }}
          </button>
          <button v-else class="run-btn" @click="startSvc">
            {{ svcReady ? i18n.t("ai.runVoiceSwap") : i18n.t("ai.downloadAndSwap") }}
          </button>
        </div>
        <div v-if="svcDownloading" class="progress">
          <div
            class="progress-fill"
            :class="{ indeterminate: modelsStore.progressPct === null }"
            :style="modelsStore.progressPct === null ? undefined : { width: `${modelsStore.progressPct}%` }"
          />
        </div>
        <div v-else-if="svcBusy" class="progress">
          <div class="progress-fill indeterminate" />
        </div>
      </li>
    </ul>
    </template>

    <!-- 「是否下载该模型」用应用内 MD3 模态框（不用系统原生框）。 -->
    <ConfirmDialog
      v-model:open="promptOpen"
      :title="i18n.t('dialog.downloadModelTitle')"
      :message="downloadPromptMessage"
      :confirm-label="i18n.t('dialog.downloadModelOk')"
      @confirm="confirmDownloadModel"
    />


    <!-- 换运行时 = 换进程里的 dylib：重启才生效 -->
    <ConfirmDialog
      v-model:open="backendRestart"
      :title="i18n.t('ai.backendRestartTitle')"
      :message="backendRestartBody"
      :confirm-label="i18n.t('ai.backendRestartNow')"
      @confirm="restartApp()"
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
/* 警告：后端看着能用、实际会回落 CPU（CUDA 组件缺失 / provider 没注册成功）。 */
.item-note.warn {
  color: rgb(var(--md-error));
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

/* ---- 顶部标签 / 分组标题 / 运行后端 ---- */
.ai-tabs {
  display: flex;
  gap: 6px;
  padding: 0 12px 10px;
}
.ai-tab {
  height: 28px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 12px;
  cursor: pointer;
}
.ai-tab:hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
}
.ai-tab.on {
  background-color: rgb(var(--md-primary) / 0.16);
  color: rgb(var(--md-primary));
  font-weight: 500;
}
.ai-sep {
  margin: 6px 0 0;
  padding: 0 2px;
  font-size: 11px;
  letter-spacing: 0.04em;
  color: rgb(var(--md-on-surface-variant) / 0.9);
}
.ai-sep.sep-generate {
  order: 45;
}
.ai-sep.sep-convert {
  order: 75;
}
.ai-seg {
  display: inline-flex;
  gap: 2px;
  padding: 2px;
  border-radius: 999px;
  background-color: rgb(var(--md-surface-container-highest));
}
.ai-seg-btn {
  height: 24px;
  padding: 0 10px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
}
.ai-seg-btn.on {
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.ai-seg-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.model-select {
  flex: 1 1 auto;
  min-width: 0;
}
.yue-form {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 4px 22px 14px;
}
.yue-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.yue-input {
  width: 100%;
  min-width: 0;
  padding: 8px 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 10px;
  background-color: rgb(var(--md-surface-container-highest) / 0.6);
  color: rgb(var(--md-on-surface));
  font-size: 13px;
  font-family: inherit;
}
.yue-input:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.yue-lyrics {
  min-height: 150px;
  resize: vertical;
  line-height: 1.5;
}
.yue-row {
  display: flex;
  gap: 10px;
}
.yue-row .yue-field {
  flex: 1 1 0;
}
.yue-note {
  margin: 0;
  font-size: 12px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-variant));
  word-break: break-word;
}
.yue-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  padding: 10px 18px 16px;
}
</style>
