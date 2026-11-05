<script setup lang="ts">
/**
 * **AI 写歌（YuE2）**子页（侧栏内整页，从列表的卡片进来，左上角返回）。
 *
 * 排版统一用 `main.css` 里的 MD3 表单原语（`m3-card` / `m3-row` / `m3-field` /
 * `m3-input` / `m3-btn`）和共享控件（`Md3Select` / `Md3Segmented` / `Md3Switch` /
 * `NumberField`），不再在页面里另造一套输入框 / 按钮。
 *
 * 两段状态：引擎（分钟级加载，`engine://yue-engine`）与任务（提交后轮询，`engine://yue-job`）。
 * 安装包**不带** audio.cpp 引擎：一份都没装时先弹安装向导，选后端后跳下载管理器。
 *
 * 用户只关心三件事，界面也只暴露这三件。
 * - **写歌**：风格 + 歌词 → 整首歌（谱和语义码在后台顺带存下来，不占一个选项）；
 * - **翻唱**：给一段参考录音（工程内轨道 / 音频文件），SheetSage2 听成 ABC 谱，
 *   YuE2 再按这份谱用你的风格重唱。参考音频那一块的排版与「音色替换」一致；
 * - **续写**：选一首在这里生成的曲子，接着往下写（YuE2-Studio 的 "Extend a song"）。
 *   续写吃的是语义码，外部音频没有码——界面直说，并引导去「翻唱」。
 *
 * 翻唱里「转谱」不是一步：主按钮点下去自动转，转完自动唱。谱面只是中间产物，
 * 收在「进阶」里——只有想改音符的人才需要看见它。 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import EngineSetupDialog from "@/components/common/EngineSetupDialog.vue";
import SetupDialog from "@/components/common/SetupDialog.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import NumberField from "@/components/common/NumberField.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import type {
  ContinuableTrack,
  ModelStatus,
  RuntimeStatus,
  YueComposeArgs,
  YueSampling,
} from "@/api/types";
import { pickAudioToOpen, pickLoraFile } from "@/api/dialogs";
import { yueContinuable, yueImportAbc, yueMidiAbc, yueRecord, yueScore } from "@/api/tauri";
import { useModelChoice } from "@/composables/useModelChoice";
import { useModelDownloadPrompt } from "@/composables/useModelDownloadPrompt";
import { useSetupSession } from "@/composables/useSetup";
import { buildPlan, engineReason, pickRecommendedEngine } from "@/core/aiDeps";
import { useDownloadsStore } from "@/stores/downloads";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useProjectStore } from "@/stores/project";
import { useRuntimesStore } from "@/stores/runtimes";
import { useSelectionStore } from "@/stores/selection";
import type { AiIntent } from "@/stores/aiIntent";
import { useYueStore } from "@/stores/yue";
import { humanSize } from "@/utils/models";
import type { SetupPlan } from "@/types/setup";

const props = defineProps<{ intent?: AiIntent | null }>();
const emit = defineEmits<{ (e: "back"): void }>();

const yue = useYueStore();
const models = useModelsStore();
const runtimes = useRuntimesStore();
const layout = useLayoutStore();
const downloads = useDownloadsStore();
const selection = useSelectionStore();
const project = useProjectStore();
const i18n = useI18n();

const style = ref("");
const lyrics = ref("");
const abc = ref("");
const duration = ref(120);
const steps = ref(32);
const seed = ref(-1);
const cfg = ref(-1);
/** 步数 / CFG / 种子是进阶参数，默认收起，普通用户只看见风格、歌词和时长。 */
const advancedOpen = ref(false);
/** 14 个采样旋钮再收一层：绝大多数人一个都不用改。 */
const samplingOpen = ref(false);
/** 续写：选中的那条轨道的语义码文件 + 要加长多少秒。 */
const prefixFile = ref<string | null>(null);
const continueSeconds = ref(30);
/** 读回谱面 / 记录时的忙标（都是几毫秒的本地读，防连点）。 */
const loadingScore = ref(false);
/** 安装向导：一份引擎都没有时强制弹出，装了以后也能从这里换后端。 */
const setupOpen = ref(false);
/** 页面根节点：进来时把共用的右侧滚动容器拉回顶部。 */
const pageEl = ref<HTMLElement | null>(null);

/** 权重档：与列表卡片共用「记住的那一档」（选过就不被「谁已下载」顶掉）。 */
const yueChoice = useModelChoice("compose", () =>
  models.catalog.filter((m) => m.feature === "compose"),
);
const modelId = yueChoice.id;
/** 下拉选项：与其它 AI 子页同一套（名称 + 「已下载 / 需下载 · 体积」）。 */
const modelOptions = yueChoice.selectOptions;
const chosenModel = yueChoice.current;

/**
 * 这份权重属于哪个 audio.cpp **家族**（`yue2` / `ace_step`）。
 *
 * 家族决定三件事：引擎配置里的 `family`、请求体的形状（YuE2 走「谱 → 语义码 → 音频」，
 * ACE-Step 走「描述 + 歌词 → 整首」）、以及这一页能开哪几个开关。清单还没加载出来时
 * 按 YuE2 算——那是能开满的一条路，不会出现「明明能用却藏起来」。 */
const family = computed(() => chosenModel.value?.family || "yue2");
/** 谱面 / 续写 / 翻唱 / LoRA 都是 YuE2 专有的：别的家族一律不显示，免得点出一堆必然失败的操作。 */
const isYue2 = computed(() => family.value === "yue2");

// ---- audio.cpp 引擎后端（与「设置 → AI」同一份清单） ----
const enginePacks = computed(() => runtimes.items.filter((r) => r.group === "audiocpp"));
const engineInstalled = computed(() => enginePacks.value.some((r) => r.installed));
const currentEngine = computed(() => enginePacks.value.find((r) => r.selected) ?? null);
/** 引擎包正在下载（管理器可能在后台继续，不在本页显示进度条）。 */
const engineDownloading = computed(() => runtimes.busyId?.startsWith("audiocpp-") ?? false);

function deviceOf(item: RuntimeStatus): string {
  return item.id.startsWith("audiocpp-") ? item.id.slice("audiocpp-".length) : item.id;
}

function deviceLabel(device: string | undefined): string {
  const key = `engine.device.${device || "cpu"}`;
  const text = i18n.t(key);
  return text === key ? device || "CPU" : text;
}

/** 引擎包的界面名字（`audiocpp-cuda12` → 「CUDA 12」）。 */
function engineLabelOf(pack: RuntimeStatus): string {
  return deviceLabel(deviceOf(pack));
}

const engineChoices = computed(() =>
  enginePacks.value
    .filter((pack) => pack.usable)
    .map((pack) => ({ kind: pack.id, label: engineLabelOf(pack) })),
);

/**
 * 组一份「写歌还要准备什么」的计划：缺哪份引擎 + 缺哪个权重档。
 *
 * 装引擎和装权重本来是两次确认、两次跳转（用户中间还得再点一次「开始」）；收进同一份
 * 计划后是**一次确认、一条队列**，合计体积也就有了意义（引擎几百 MB + 权重几个 GB）。 */
function buildYuePlan(override?: string): SetupPlan | null {
  const usable = enginePacks.value.filter((pack) => pack.usable);
  const engine = override
    ? (usable.find((pack) => pack.id === override) ?? null)
    : pickRecommendedEngine(usable);
  const wanted: ModelStatus[] = [];
  const model = chosenModel.value;
  if (model && !model.installed) wanted.push(model);
  return buildPlan({
    feature: "yue",
    title: i18n.t("setup.title.yue"),
    lead: (backend, why) => i18n.t("setup.lead", { backend, reason: why }),
    engine: engine && !engine.installed ? engine : null,
    engineLabel: engine ? engineLabelOf(engine) : "",
    reason: engineReason(i18n.t, engine),
    choices: engineChoices.value,
    models: wanted,
  });
}

/**
 * 「准备运行环境」会话：缺引擎 / 缺权重时弹向导，确认后跳管理器并串行下载。
 *
 * audio.cpp 引擎是独立进程（每次启动现挑后端），装完**不需要重启**，所以这里只需要
 * 「装完把后端切过去」。 */
const yueSetup = useSetupSession({
  feature: "yue",
  build: () => buildYuePlan(),
  applyBackend: async (plan) => {
    if (plan.backend?.kind) await runtimes.select(plan.backend.kind);
    return false;
  },
});

/** 模板里用的几个 ref（`script setup` 只自动解包顶层 ref）。 */
const setupPlanOpen = yueSetup.open;
const setupPlan = yueSetup.plan;
const setupForced = yueSetup.forced;

/** 换一份引擎重算计划；没有要下的就直接切过去。 */
async function chooseEngine(kind: string): Promise<void> {
  const next = buildYuePlan(kind);
  if (next) {
    yueSetup.setPlan(next);
    return;
  }
  yueSetup.setPlan(null);
  await runtimes.select(kind);
}

/** 引擎与权重都齐了才自动开头；缺东西时先走向导。 */
watch(
  () => [runtimes.loading, enginePacks.value.length, engineInstalled.value] as const,
  ([loading, count]) => {
    if (loading || count === 0) return;
    // 首次进入自动弹一次（同一「功能 + 推荐引擎」只弹一次；点过「稍后」就不再打扰）。
    yueSetup.maybePrompt();
  },
  { immediate: true },
);

const engineLine = computed(() => {
  const e = yue.engine;
  if (engineDownloading.value) return i18n.t("yue.engineDownloading");
  if (e?.state === "starting") return e.message ?? i18n.t("yue.engineStarting");
  if (e?.state === "error") return e.message ?? e.log ?? i18n.t("yue.engineError");
  if (e?.state === "ready") {
    return `${i18n.t("yue.engineReady", { device: deviceLabel(e.device), model: e.model })} · ${i18n.t("yue.readyHint")}`;
  }
  if (!enginePacks.value.length) {
    return runtimes.loading ? i18n.t("common.loading") : i18n.t("yue.noEnginePacks");
  }
  return engineInstalled.value ? i18n.t("yue.engineStopped") : i18n.t("yue.engineMissingHint");
});

/** 当前后端（引擎跑着就用它报的，没跑就用选中的那份）。 */
const engineDevice = computed(
  () => yue.engine?.device || (currentEngine.value ? deviceOf(currentEngine.value) : ""),
);
/** CUDA 性能预设只对 CUDA 后端有意义：跑 CPU / Vulkan 时不显示这一行。 */
const cudaActive = computed(() => engineDevice.value.startsWith("cuda"));

const engineBadge = computed(() => {
  if (!engineInstalled.value) return i18n.t("engine.notInstalledHint");
  return deviceLabel(engineDevice.value);
});

const engineBackendLine = computed(() => {
  if (engineDownloading.value) return i18n.t("yue.engineDownloading");
  if (!enginePacks.value.length) {
    return runtimes.loading ? i18n.t("common.loading") : i18n.t("yue.noEnginePacks");
  }
  if (!engineInstalled.value) return i18n.t("engine.notInstalledHint");
  const item = currentEngine.value;
  if (!item) return i18n.t("engine.notInstalledHint");
  return i18n.t("engine.current", {
    name: deviceLabel(deviceOf(item)),
    mode: i18n.t(item.automatic ? "engine.auto" : "engine.manual"),
  });
});

/** CUDA 性能预设：开关状态在 store 里，写入后下次启动引擎生效。 */
const cudaPerf = computed({
  get: () => yue.cudaPerf,
  set: (value: boolean) => void yue.setCudaPerf(value),
});

// ---- 创作方式：创作（从零写）/ 翻唱（参考录音 → ABC 谱 → 按谱唱） ----
const mode = ref<"create" | "cover">("create");
const modeValue = computed({
  get: () => mode.value,
  set: (value: string) => {
    mode.value = value as "create" | "cover";
  },
});

const modeOptions = computed<SegmentedOption[]>(() => [
  { value: "create", label: i18n.t("yue.modeCreate"), title: i18n.t("yue.modeCreateHint") },
  { value: "cover", label: i18n.t("yue.modeCover"), title: i18n.t("yue.modeCoverHint") },
]);

/**
 * 翻唱手里有一份 ABC 谱。要是它和续写前缀同时被发出去，引擎会**先重新规划一份谱**再吃前缀
 * （`cot` 会被后端改成 `full`），接出来的段落就跟原曲脱节了。切到翻唱时把前缀丢干净。 */
watch(mode, (value) => {
  if (value === "cover") clearPrefix();
});

/** 别的家族没有翻唱这条路：切过去时把模式收回「创作」，别留一个点不动的翻唱表单。 */
watch(
  isYue2,
  (yue2) => {
    if (!yue2) {
      mode.value = "create";
      // 续写是 YuE2 专有的：切到 ACE-Step 后前缀会被引擎**静默忽略**（等于又写一首歌），
      // 所以在源头就丢掉它。`continueTrackId` 在这个 watch 里还不能碰（声明在后面，
      // immediate 回调会在 setup 阶段就先跑一次）。
      prefixFile.value = null;
    }
  },
  { immediate: true },
);

// 权重清单通常在页面打开后才加载完：选中的若是 ACE-Step，刚套上的「翻唱」意图会失效，
// 这时明确提醒一次，别让用户以为按钮没反应。
watch(isYue2, (yue2, before) => {
  if (before === true && !yue2 && props.intent?.mode === "cover") {
    layout.notify(i18n.t("menu.coverNeedsYue2"));
  }
});

/**
 * 步数跟着家族走：YuE2 常用 32 步，ACE-Step 的 Turbo 档只要 8 步（再多也不会更细）。
 * 只在家族真的换掉时给回默认值，不覆盖用户自己填的数。 */
const stepsMax = computed(() => (isYue2.value ? 128 : 8));
watch(
  family,
  (value, before) => {
    if (before !== undefined && before === value) return;
    steps.value = value === "yue2" ? 32 : 8;
  },
  { immediate: true },
);

/**
 * 进阶：要不要让模型先写谱（`cot`）。
 *
 * 默认写（结构最完整）；关掉 = `cot=off`，跳过符号阶段直接出音频，最快但结构最弱。
 * 翻唱不看这个开关——它必须按参考录音转出来的旋律谱唱（`cot=melody`）。 */
const planScore = ref(true);

/** 实际发给引擎的符号规划路线（只有 YuE2 认这个字段；别的家族发什么都不会读）。 */
const effectiveCot = computed(() => {
  if (!isYue2.value) return "off";
  // 续写：前缀只在不重新规划谱的路径上被接受（引擎硬约束）。
  if (continueOn.value) return "off";
  if (mode.value === "cover") return "melody";
  return planScore.value ? "full" : "off";
});

const modelHint = computed(() => {
  const m = chosenModel.value;
  if (!m) return "";
  return `${m.license} · ${humanSize(m.approx_bytes)}`;
});

/**
 * 输入够不够？
 *
 * YuE2 风格 / 歌词给一样就能写；ACE-Step **必须要一句描述**——它就是那句描述的正文，
 * 空着引擎直接回「requires text_input」。 */
const hasInput = computed(() =>
  isYue2.value
    ? style.value.trim().length > 0 || lyrics.value.trim().length > 0
    : style.value.trim().length > 0,
);

const canCompose = computed(
  () =>
    !yue.busy &&
    !yue.transcribing &&
    chosenModel.value?.installed === true &&
    hasInput.value &&
    // 续写总长要留在引擎的单次上限内（600 秒），否则前缀会比 token 上限还长。
    !continueTooLong.value &&
    // 翻唱的谱是**自动**来的（转谱 / MIDI 换算），不用先手动准备。
    (mode.value === "create" || coverHasScoreOrSource.value),
);

/** 主按钮：按当前缺什么决定动作，避免一个永远点不动的「生成」。 */
type PrimaryKind = "engine" | "model" | "coverModel" | "compose" | "stop";
const primaryKind = computed<PrimaryKind>(() => {
  // 转谱 / 听写也是这个按钮发起的（见 `compose()`），跑起来就变成「停止」。
  if (yue.busy || yue.transcribing || yue.lyricizing) return "stop";
  if (!engineInstalled.value) return "engine";
  if (chosenModel.value?.installed !== true) return "model";
  // 翻唱要额外一份 SheetSage2：缺了就让这个按钮去下载，不再单独占一行。
  if (
    mode.value === "cover" &&
    coverSource.value !== "midi" &&
    !coverInstalled.value &&
    !abc.value.trim()
  ) {
    return "coverModel";
  }
  // 引擎没起也算「生成」：后端会先把它拉起来（转谱刚停过它，这一趟要连贯）。
  return "compose";
});
const primaryLabel = computed(() => {
  switch (primaryKind.value) {
    case "stop":
      return i18n.t("yue.stop");
    case "engine":
      return i18n.t("yue.engineChoose");
    case "model":
      return i18n.t("yue.downloadModel");
    case "coverModel":
      return i18n.t("yue.coverDownloadModel");
    default:
      // 翻唱没有谱时，这一次点下去会先把参考音频转成谱——按钮上就说清楚。
      if (mode.value === "cover" && !abc.value.trim()) return i18n.t("yue.coverGenerate");
      return i18n.t("yue.generate");
  }
});
const primaryDisabled = computed(() => {
  if (primaryKind.value === "engine") return enginePacks.value.length === 0 || engineDownloading.value;
  if (primaryKind.value === "compose") return !canCompose.value;
  return false;
});

/** 底部常驻动作条左侧的一句话：告诉用户下一步该做什么。 */
const actionHint = computed(() => {
  switch (primaryKind.value) {
    case "engine":
      return engineDownloading.value ? i18n.t("yue.engineDownloading") : i18n.t("yue.stepEngine");
    case "model":
      return i18n.t("yue.stepModel");
    case "coverModel":
      return i18n.t("yue.coverNeedModel");
    case "compose":
      if (continueTooLong.value) {
        return i18n.t("yue.continueTooLong", { total: continueTotal.value });
      }
      if (!canCompose.value) {
        if (mode.value === "cover" && !coverHasScoreOrSource.value) {
          return i18n.t("yue.coverNoTrack");
        }
        return i18n.t(isYue2.value ? "ai.yueNeedInput" : "yue.needPrompt");
      }
      // 引擎被停过（转谱 / 听写 / 上次取消）：这一下会连加载带生成一起跑。
      if (!yue.ready) return i18n.t("yue.willLoadEngine");
      return "";
    default:
      return "";
  }
});

// ---- 采样旋钮（-1 = 用引擎默认，不发送） ----
interface KnobState {
  temperature: number;
  top_p: number;
  top_k: number;
  repetition_penalty: number;
  penalty_window: number;
  min_tokens: number;
  max_tokens: number;
}

function blankKnobs(): KnobState {
  return {
    temperature: -1,
    top_p: -1,
    top_k: -1,
    repetition_penalty: -1,
    penalty_window: -1,
    min_tokens: -1,
    max_tokens: -1,
  };
}

const abcKnobs = ref<KnobState>(blankKnobs());
const semanticKnobs = ref<KnobState>(blankKnobs());

/** `-1` 是「不填」；其余原样。返回 `undefined` 表示整组都不发。 */
function knobsToArgs(knobs: KnobState): YueSampling | undefined {
  const out: YueSampling = {};
  for (const [key, value] of Object.entries(knobs)) {
    if (value >= 0) out[key as keyof YueSampling] = value;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

// ---- 续写：只列「在这里生成过」的曲子 ----
//
// 续写只吃引擎的 `semantic_prefix`（codec indices，不是音频）。语义码每次生成都会落盘成
// `<音频>.semantic.json`，所以「哪些轨道能续写」由后端扫一遍文件说了算：外部导入的音频
// 根本不出现在下拉里——选不到，也就不会点出一个注定失败的续写。
const continueTrackId = ref<string | null>(null);
/** 能续写的轨道：id → {语义码路径, 前缀秒数}。 */
const continuable = ref<Record<string, ContinuableTrack>>({});

/** 只留「后端说有码 + 工程里还在」的轨道。 */
const continuableTracks = computed(() =>
  audioTracks.value.filter((t) => continuable.value[String(t.id)] !== undefined),
);
/** 下拉选项：名字 + 前缀时长，选之前就知道能接多长。 */
const continueTrackOptions = computed(() =>
  continuableTracks.value.map((t) => {
    const info = continuable.value[String(t.id)];
    return {
      id: String(t.id),
      label: i18n.t("yue.continueTrackLabel", {
        name: t.name,
        seconds: Math.round(info?.seconds ?? 0),
      }),
    };
  }),
);

const continueOn = computed(() => prefixFile.value !== null);
/** 前缀有多长（秒）。后端按「码长 ÷ 25 帧每秒」算好，扫一遍就有。 */
const prefixSeconds = computed(() =>
  continueTrackId.value ? (continuable.value[continueTrackId.value]?.seconds ?? 0) : 0,
);
const continueTotal = computed(() => Math.round(prefixSeconds.value) + continueSeconds.value);
/** 引擎单次最多渲染 600 秒：前缀 + 新增超过它就接不下去（前缀会比 token 上限还长）。 */
const continueTooLong = computed(() => continueOn.value && continueTotal.value > 600);
const continueLine = computed(() =>
  i18n.t("yue.continueOn", {
    from: Math.round(prefixSeconds.value),
    total: continueTotal.value,
  }),
);
const continueHint = computed(() => {
  if (continueTooLong.value) {
    return i18n.t("yue.continueTooLong", { total: continueTotal.value });
  }
  if (continueOn.value) return continueLine.value;
  if (!continueTrackId.value) return i18n.t("yue.continuePick");
  return i18n.t("yue.continueHint", { seconds: Math.round(prefixSeconds.value) });
});

/** 续写时把原曲的风格 / 歌词填回了表单：明说一句，别让人以为输入框被乱改。 */
const continueInherit = computed(() => {
  const id = continueTrackId.value;
  const info = id ? continuable.value[id] : undefined;
  if (!continueOn.value || !info) return "";
  const style = info.style?.trim() || "—";
  const lines = info.lyrics ? info.lyrics.split("\n").filter((line) => line.trim()).length : 0;
  return i18n.t("yue.continueInherit", { style, lines });
});

/** 重新扫一遍可续写轨道（工程变了 / 刚生成完时调）。 */
async function refreshContinuable(): Promise<void> {
  try {
    const list = await yueContinuable();
    const map: Record<string, ContinuableTrack> = {};
    for (const item of list) map[String(item.track)] = item;
    continuable.value = map;
  } catch {
    continuable.value = {};
  }
}

/** 换轨道 → 清掉上一次的前缀（码和轨道是一对一的）。 */
watch(continueTrackId, (value, before) => {
  if (before !== value) clearPrefix();
});

/**
 * 以上一段为前缀：总长 = 前缀 + 新增，否则引擎会把前缀算进新生成的长度里、把歌截短。
 *
 * 同时把**原曲的文本条件**填回表单：语义码只决定旋律骨架，风格与歌词仍然是语义解码器的
 * 条件。不沿用它们，接出来的段落会像另一首歌——这是「续写和原曲几乎毫无关联」的主要来源。
 * 权重也一并指回原曲那一档（换量化会让同一段码漂）。字段仍然可改，改了就按改的走。 */
function useTrackAsPrefix(): void {
  const id = continueTrackId.value;
  const info = id ? continuable.value[id] : undefined;
  if (!info) return;
  // **前缀必须配 `cot=off`**：引擎只在「不重新规划谱」或「手里有谱」时才接受语义码前缀
  // （实测报错：`semantic prefix requires abc or abc_file unless cot=off`）。`effectiveCot`
  // 里已按续写状态返回 off——续写就是接着往下长，不该再让模型重新写一份谱。
  duration.value = Math.min(600, Math.max(15, Math.round(continueTotal.value)));
  prefixFile.value = info.semantic;
  if (info.style?.trim()) style.value = info.style;
  if (info.lyrics?.trim()) lyrics.value = info.lyrics;
  if (info.model && models.catalog.some((m) => m.id === info.model)) {
    modelId.value = info.model;
  }
}

function clearPrefix(): void {
  prefixFile.value = null;
}

// ---- 翻唱：参考音频 / MIDI → ABC 谱（UI 与「音色替换」的参考声线一套：来源 + 具体对象） ----
/** 有音频片段的轨道（能「听」的那一类；续写也从这里挑）。 */
const audioTracks = computed(() =>
  (project.project?.tracks ?? []).filter((t) => t.clips.some((c) => c.audio)),
);
/** 有 MIDI 片段的轨道（旋律已经知道，零推理直接写谱）。 */
const midiTracks = computed(() =>
  (project.project?.tracks ?? []).filter((t) => t.clips.some((c) => !c.audio)),
);

/** 参考来源：工程内音频轨 / 音频文件 / 工程内 MIDI。 */
const coverSource = ref<"track" | "file" | "midi">("track");
const coverSourceChoice = computed({
  get: () => coverSource.value,
  set: (value: string | null) => {
    coverSource.value = value === "file" ? "file" : value === "midi" ? "midi" : "track";
  },
});
const coverSourceOptions = computed(() => [
  { id: "track", label: i18n.t("yue.coverSourceTrack") },
  { id: "file", label: i18n.t("yue.coverSourceFile") },
  { id: "midi", label: i18n.t("yue.coverSourceMidi") },
]);

/** 参考轨道（字符串 id，和 Md3Select 一致）。 */
const coverTrackId = ref<string | null>(null);
/**
 * 右键菜单指定的参考**片段**；为空 = 用这条轨道上的第一个音频片段。
 *
 * 转谱 / 抓歌词的接口都支持 `clip`，所以从音频块右键进来时只听这一块，不会被同轨道的
 * 其他音频带偏。
 */
const coverClipId = ref<string | null>(null);
const coverTrackOptions = computed(() =>
  audioTracks.value.map((t) => ({ id: String(t.id), label: t.name })),
);
/** 参考文件路径。 */
const coverFile = ref("");
/** MIDI 参考：轨道 + 片段。 */
const coverMidiTrackId = ref<string | null>(null);
const coverMidiClipId = ref<string | null>(null);
const coverMidiTrackOptions = computed(() =>
  midiTracks.value.map((t) => ({ id: String(t.id), label: t.name })),
);
/** 选中轨道里的 MIDI 片段（按起点排序，名字用「第 n 段 + 起拍」）。 */
const coverMidiClipOptions = computed(() => {
  const track = midiTracks.value.find((t) => String(t.id) === coverMidiTrackId.value);
  if (!track) return [];
  return track.clips
    .filter((c) => !c.audio)
    .slice()
    .sort((a, b) => a.start - b.start)
    .map((c, i) => ({
      id: String(c.id),
      label: i18n.t("yue.coverClipLabel", {
        index: i + 1,
        beat: Math.round(c.start) + 1,
        bars: Math.max(1, Math.round(c.length / 4)),
      }),
    }));
});
/** 最近一次从 MIDI 转出来的统计（没有转就为空）。 */
const midiScoreInfo = ref<{ bars: number; notes: number; lyrics: number } | null>(null);

/** 转谱用的模型条目：发音频才需要它；MIDI 参考不需要。 */
const coverModel = computed(() => models.catalog.find((m) => m.id === "sheetsage2-orig") ?? null);
const coverInstalled = computed(() => coverModel.value?.installed === true);

/**
 * 缺权重的下载确认框（SheetSage2 / 写歌权重共用）：确认即跳「管理器 → 模型」**并**开始下载，
 * 和其它 AI 卡片同一套；不再出现「跳过去却什么都不下」。
 */
const {
  open: modelAskOpen,
  message: modelAskMessage,
  require: requireModel,
  confirm: confirmModelDownload,
  background: backgroundModelDownload,
} = useModelDownloadPrompt();
/** 权重正在下载（卡片上要显示进度与「取消」）。 */
const coverDownloading = computed(() =>
  coverModel.value ? models.installing(coverModel.value.id) : false,
);

/**
 * 权重缺就弹确认框。
 *
 * 清单里没有这一条（老安装 / 离线）时退回旧行为：直接跳到管理器，让用户自己看缺什么。 */
function askCoverModel(): void {
  const model = coverModel.value;
  if (!model) {
    openModelManager();
    return;
  }
  requireModel(model);
}

const coverCanRun = computed(
  () =>
    coverSource.value !== "midi" &&
    coverInstalled.value &&
    !yue.transcribing &&
    !yue.busy &&
    (coverSource.value === "track" ? coverTrackId.value !== null : coverFile.value.trim() !== ""),
);

/** MIDI 参考选中了没有（有没有可转的东西）。 */
const midiPicked = computed(
  () => coverMidiTrackId.value !== null && coverMidiClipId.value !== null,
);

/** 翻唱能不能直接开跑：要么手里已经有谱，要么现在能从参考音频 / MIDI 得到一份。 */
const coverHasScoreOrSource = computed(
  () =>
    abc.value.trim().length > 0 ||
    (coverSource.value === "midi" ? midiPicked.value : coverCanRun.value),
);

const coverHint = computed(() => {
  if (yue.transcribeError) return yue.transcribeError;
  if (yue.transcribing) return i18n.t("yue.coverRunning");
  if (coverDownloading.value) return i18n.t("yue.coverDownloading");
  if (coverSource.value === "midi") {
    if (!midiPicked.value) return i18n.t("yue.coverNoMidi");
    if (midiScoreInfo.value) {
      return i18n.t("yue.coverMidiReady", {
        bars: midiScoreInfo.value.bars,
        notes: midiScoreInfo.value.notes,
        lyrics: midiScoreInfo.value.lyrics,
      });
    }
    return i18n.t("yue.coverMidiHint");
  }
  if (!coverInstalled.value) {
    const size = coverModel.value ? ` · ${humanSize(coverModel.value.approx_bytes)}` : "";
    return `${i18n.t("yue.coverNeedModel")}${size}`;
  }
  if (coverSource.value === "track" && coverTrackId.value === null) {
    return i18n.t("yue.coverNoTrack");
  }
  if (abc.value.trim()) {
    return i18n.t("yue.coverReady", { chars: abc.value.trim().length });
  }
  return i18n.t("yue.coverHint");
});

async function pickCoverFile(): Promise<void> {
  const path = await pickAudioToOpen();
  if (path) coverFile.value = path;
}

/// 把选中的 MIDI 片段写成 ABC：旋律本来就在工程里，不需要任何模型。
async function applyMidiScore(): Promise<boolean> {
  const track = coverMidiTrackId.value;
  const clip = coverMidiClipId.value;
  if (!track || !clip) return false;
  try {
    const score = await yueMidiAbc(Number(track), Number(clip));
    abc.value = score.abc;
    midiScoreInfo.value = {
      bars: score.bars,
      notes: score.notes,
      lyrics: score.lyrics ? score.lyrics.split(/\s+/).filter(Boolean).length : 0,
    };
    // 歌词框空着就自动带入——这正是「MIDI 里带歌词」最省事的地方；
    // 已经有词就不覆盖（那是用户写的东西）。
    if (!lyrics.value.trim() && score.lyrics) {
      lyrics.value = score.lyrics;
    }
    return true;
  } catch (err) {
    midiScoreInfo.value = null;
    layout.notify(String(err));
    return false;
  }
}

/** 音频参考的接口参数：右键指定的片段只在这条轨道上有效，否则退回「第一个音频片段」。 */
function audioReferenceArgs(): { track: number; clip: number | null } {
  const track = Number(coverTrackId.value);
  const row = audioTracks.value.find((t) => t.id === track);
  const wanted = coverClipId.value !== null ? Number(coverClipId.value) : null;
  const clip =
    wanted !== null && row?.clips.some((c) => c.id === wanted && c.audio) ? wanted : null;
  return { track, clip };
}

async function transcribeCover(): Promise<boolean> {
  if (!coverCanRun.value) return false;
  const args =
    coverSource.value === "track" ? audioReferenceArgs() : { audio: coverFile.value.trim() };
  const outcome = await yue.transcribe(args);
  if (!outcome) {
    if (yue.transcribeError) layout.notify(yue.transcribeError);
    return false;
  }
  // 转出来的谱直接进谱面框；翻唱按这份旋律谱唱（`effectiveCot` 已经固定 melody）。
  abc.value = outcome.abc;
  layout.notify(
    i18n.t("yue.coverDone", {
      chars: outcome.chars,
      seconds: Math.round(outcome.elapsed),
    }),
  );
  return true;
}

// ---- 歌词：音频参考用 ASR 听写，MIDI 参考直接读音符上的词 ----
//
// 同一个按钮、同一套状态。音频这条路要的是清单里的 ASR（不是转谱用的 SheetSage2），
// 所以它**不依赖 SheetSage2**；MIDI 那条零推理、毫秒回。
// 抓到词**直接覆盖**歌词框——用户要的就是「一键拿到词」，不再弹二次确认。
const {
  current: lyricsModel,
  id: lyricsModelId,
  ready: lyricsReady,
  downloading: lyricsDownloading,
  selectOptions: lyricsModelOptions,
} = useModelChoice("lyrics", () => models.catalog.filter((m) => m.feature === "lyrics"));

const lyricsCanRun = computed(() => {
  if (yue.lyricizing || yue.transcribing || yue.busy) return false;
  if (coverSource.value === "midi") return midiPicked.value;
  return coverSource.value === "track"
    ? coverTrackId.value !== null
    : coverFile.value.trim() !== "";
});

const lyricsHint = computed(() => {
  if (yue.lyricizing) return i18n.t("yue.lyricsRunning");
  if (yue.lyricError) return yue.lyricError;
  if (coverSource.value === "midi") {
    return midiPicked.value ? i18n.t("yue.lyricsMidiFetchHint") : i18n.t("yue.coverNoMidi");
  }
  const model = lyricsModel.value;
  if (!model) return i18n.t("yue.lyricsNoModel");
  if (!model.installed) {
    return i18n.t("yue.lyricsNeedModel", { size: humanSize(model.approx_bytes) });
  }
  return i18n.t("yue.lyricsFetchHint");
});

/** 写 ASR 权重：和其它 AI 卡片同一套「确认 → 跳管理器并开始下载」。 */
function askLyricsModel(): void {
  const model = lyricsModel.value;
  if (!model) {
    openModelManager();
    return;
  }
  requireModel(model);
}

/** 获取歌词：音频 → ASR 听写；MIDI → 音符上的词。抓到就覆盖歌词框。 */
async function getLyrics(): Promise<void> {
  if (!lyricsCanRun.value) return;

  // MIDI：旋律和词都在工程里，不用任何模型。
  if (coverSource.value === "midi") {
    const track = coverMidiTrackId.value;
    const clip = coverMidiClipId.value;
    if (!track || !clip) return;
    try {
      const score = await yueMidiAbc(Number(track), Number(clip));
      if (!score.lyrics.trim()) {
        layout.notify(i18n.t("yue.lyricsEmptyMidi"));
        return;
      }
      lyrics.value = score.lyrics;
      layout.notify(
        i18n.t("yue.lyricsDone", {
          lines: score.lyrics.split("\n").filter((line) => line.trim()).length,
          syllables: score.lyrics.split(/\s+/).filter(Boolean).length,
        }),
      );
    } catch (err) {
      layout.notify(String(err));
    }
    return;
  }

  if (!lyricsReady.value) {
    askLyricsModel();
    return;
  }
  const args =
    coverSource.value === "track" ? audioReferenceArgs() : { audio: coverFile.value.trim() };
  const outcome = await yue.lyrics({ ...args, model: lyricsModelId.value });
  if (!outcome) {
    if (yue.lyricError) layout.notify(yue.lyricError);
    return;
  }
  lyrics.value = outcome.text;
  layout.notify(
    i18n.t("yue.lyricsDone", { lines: outcome.lines.length, syllables: outcome.syllables }),
  );
}

// ---- 一条进度条：现在跑到哪一步 + 已经等了多久 ----
//
// 「转谱 → 生成」是**一次点击里的两段**（翻唱的谱只是中间产物）。以前每段各画一条
// 进度条，看着像两个任务在抢带宽；现在整页只有一条，把「在哪一步、等了多久」放在
// 动作条左下角——那正是用户盯着看的地方。
type Stage = "transcribe" | "lyrics" | "download" | "load" | "compose";

/** 选中的权重正在下载（进度在「管理器 → 模型」，这里只报阶段）。 */
const weightDownloading = computed(() =>
  modelId.value ? models.installing(modelId.value) : false,
);

/**
 * 同一时刻只可能有一个阶段；顺序 = 先转谱 / 听写，再下载，最后才是生成。
 *
 * 生成那一段还分「加载权重」和「真正在跑」：第一次生成（或刚被转谱停过引擎）时，
 * 前几分钟其实在加载几 GB 权重，这时候说「生成中」是在骗人。 */
const stage = computed<Stage | null>(() => {
  if (yue.transcribing) return "transcribe";
  if (yue.lyricizing) return "lyrics";
  if (weightDownloading.value || coverDownloading.value || lyricsDownloading.value) return "download";
  if (yue.busy) return yue.engine?.state === "starting" ? "load" : "compose";
  return null;
});

/** 本地计时：转谱 / 听写 / 下载都没有后端计时，只能从进入这一刻自己数。 */
const stageTicked = ref(0);
let stageTimer: number | null = null;
function stopStageTimer(): void {
  if (stageTimer !== null) {
    window.clearInterval(stageTimer);
    stageTimer = null;
  }
}
watch(
  stage,
  (value) => {
    stopStageTimer();
    stageTicked.value = 0;
    if (!value) return;
    const started = Date.now();
    stageTimer = window.setInterval(() => {
      stageTicked.value = Math.round((Date.now() - started) / 1000);
    }, 500);
  },
  { immediate: true },
);
onUnmounted(stopStageTimer);

/** 生成那一段用后端报的秒数（含加载权重的等待），其余用本地计时。 */
const stageSeconds = computed(() =>
  stage.value === "compose" || stage.value === "load"
    ? Math.round(yue.job.elapsed)
    : stageTicked.value,
);

/** 左下角那一行：`转谱中 · 已等 12 秒`。 */
const stageText = computed(() => {
  const value = stage.value;
  if (!value) return "";
  return i18n.t("yue.stageLine", {
    stage: i18n.t(`yue.stage.${value}`),
    seconds: stageSeconds.value,
  });
});

// ---- 动作 ----
function openModelManager(): void {
  downloads.jumpTo("model", "*");
}

async function startEngine(): Promise<void> {
  if (yue.starting) return;
  // 缺引擎 / 缺权重：先把「还差什么、一共多少」摆出来，确认后一条队列下完。
  if (!engineInstalled.value || chosenModel.value?.installed !== true) {
    if (!yueSetup.ensure()) return;
  }
  await startEngineNow();
}

/** 真的要启动引擎（调用前必须确认引擎与权重都在位）。 */
async function startEngineNow(): Promise<void> {
  if (yue.starting) return;
  const model = chosenModel.value;
  if (!model) {
    openModelManager();
    return;
  }
  // 兜底：计划刚被别处取消 / 清单晚到，权重仍然缺时退回单模型确认框。
  if (!model.installed) {
    requireModel(model);
    return;
  }
  await yue.start(modelId.value || null);
}

function composeArgs(): YueComposeArgs {
  // 续写只对 YuE2 有意义，而且只走「创作」这条没有外部谱的路；前缀在别的家族 / 翻唱里
  // 要么被静默忽略、要么被重新规划的谱带偏（`isYue2` / `mode` 的 watcher 也会清掉它，
  // 这里再兜一道，避免请求真的带着一个不会按预期生效的字段跑几分钟）。
  const extending =
    continueOn.value && continueTrackId.value !== null && isYue2.value && mode.value === "create";
  return {
    style: style.value.trim(),
    lyrics: lyrics.value,
    // **只有翻唱才把 ABC 谱发出去**：创作 / 续写时留着上一次翻唱的谱，会让引擎重新规划
    // 一份旋律，接出来的段落就跟原曲脱节了。
    abc: mode.value === "cover" ? abc.value : "",
    duration: extending ? Math.min(600, continueTotal.value) : duration.value,
    steps: steps.value,
    seed: seed.value,
    cot: effectiveCot.value,
    // 永远是整首：用户不选“跑到哪一步”。语义码每次都顺带存下来（续写 / 复现靠它）。
    stop_after: "audio",
    export_semantic: true,
    semantic_prefix_file: extending ? prefixFile.value : null,
    abc_sampling: knobsToArgs(abcKnobs.value) ?? null,
    semantic_sampling: knobsToArgs(semanticKnobs.value) ?? null,
    cfg_scale: cfg.value < 0 ? undefined : cfg.value,
    model: modelId.value || null,
    // 续写：新片段接在被续写的**那条轨道**上（后端按它的最后一个音频片段定位）；
    // 普通生成才是「插到当前选中轨道下面」。
    after_track: extending ? Number(continueTrackId.value) : (selection.trackId ?? null),
  };
}

async function compose(): Promise<void> {
  if (!canCompose.value) return;
  // 翻唱：还没有谱就先弄出一份——音频走 SheetSage2 转谱，MIDI 直接本地换算。
  if (mode.value === "cover" && !abc.value.trim()) {
    const ok = coverSource.value === "midi" ? await applyMidiScore() : await transcribeCover();
    if (!ok) return;
  }
  await yue.compose(composeArgs());
}

async function runPrimary(): Promise<void> {
  switch (primaryKind.value) {
    case "stop":
      // 一个按钮停一切：转谱 / 听写 / 写歌，谁在跑停谁。
      if (yue.transcribing) await yue.cancelTranscribe();
      if (yue.lyricizing) await yue.cancelLyrics();
      if (yue.busy) await yue.cancel();
      break;
    case "engine":
      await startEngine();
      break;
    case "model":
      openModelManager();
      break;
    case "coverModel":
      askCoverModel();
      break;
    case "compose":
      await compose();
      break;
  }
}

/**
 * 把模型这次写的 ABC 谱显式导入成 MIDI 轨。
 *
 * 生成结果默认只加一条音频轨（谱面草稿不再自动铺进编排）；想看看模型到底哼了什么、
 * 或者拿它的旋律当素材的人，再点这个按钮。 */
async function importScore(): Promise<void> {
  const path = yue.job.score;
  if (!path) return;
  try {
    layout.notify(await yueImportAbc(path));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 把落盘的谱读回谱面框（改完再生成 = 按谱演唱）。谱面在进阶里，读完顺手展开。 */
async function loadLastScore(quiet = false): Promise<void> {
  const path = yue.job.score;
  if (!path || loadingScore.value) return;
  loadingScore.value = true;
  try {
    abc.value = await yueScore(path);
    if (!quiet) {
      advancedOpen.value = true;
      layout.notify(i18n.t("yue.loadedScore"));
    }
  } catch (err) {
    layout.notify(String(err));
  } finally {
    loadingScore.value = false;
  }
}

/** 把生成记录里的参数填回表单（种子用记录里那个，才能复现同一条结果）。 */
async function loadRecord(): Promise<void> {
  const path = yue.job.record;
  if (!path) return;
  try {
    const record = JSON.parse(await yueRecord(path)) as {
      seed?: number;
      args?: Partial<YueComposeArgs>;
    };
    const args: Partial<YueComposeArgs> = record.args ?? {};
    style.value = args.style ?? "";
    lyrics.value = args.lyrics ?? "";
    abc.value = args.abc ?? "";
    if (typeof args.duration === "number") duration.value = args.duration;
    if (typeof args.steps === "number") steps.value = args.steps;
    // `cot` 是「路线」，界面拆成「创作/翻唱 + 先写谱：按记录还原」。
    if (typeof args.cot === "string") {
      if (args.cot === "melody" && args.abc) {
        mode.value = "cover";
      } else {
        mode.value = "create";
        planScore.value = args.cot !== "off";
      }
    }
    if (typeof args.cfg_scale === "number") cfg.value = args.cfg_scale;
    if (typeof record.seed === "number") seed.value = record.seed;
    layout.notify(i18n.t("yue.replayed"));
  } catch (err) {
    layout.notify(String(err));
  }
}

// ---- LoRA ----
const loraDraft = ref({ ...yue.lora });
const loraDirty = computed(
  () => JSON.stringify(loraDraft.value) !== JSON.stringify(yue.lora),
);
watch(
  () => yue.lora,
  (value) => {
    loraDraft.value = { ...value };
  },
  { deep: true },
);

async function pickAr(): Promise<void> {
  const path = await pickLoraFile();
  if (path) loraDraft.value = { ...loraDraft.value, ar_path: path };
}

async function pickNar(): Promise<void> {
  const path = await pickLoraFile();
  if (path) loraDraft.value = { ...loraDraft.value, nar_path: path };
}

async function saveLora(): Promise<void> {
  await yue.saveLora({ ...loraDraft.value });
  // 适配器在会话创建时合并，改完必须重启引擎。
  if (yue.ready) await yue.stop();
}

/** 强制向导里的「返回」：直接退回 AI 列表，不把人困在一张不能用的页面上。 */
function onSetupCancel(): void {
  if (!engineInstalled.value) emit("back");
}

let stopYue: (() => void) | null = null;

/**
 * 把右键菜单的意图落到表单上（挂载时 + 意图变化时）。
 *
 * 只预选「从哪块音频 / 哪段旋律来的」，用户进来后仍可改参考来源、轨道与片段；所以这里
 * 不锁任何控件。
 */
function applyIntent(request: AiIntent | null | undefined): void {
  if (!request || request.page !== "yue") return;
  // 翻唱是 YuE2 专有的：选了 ACE-Step 时不能假装切过去，直接说清楚。
  if (request.mode === "cover" && !isYue2.value) {
    layout.notify(i18n.t("menu.coverNeedsYue2"));
    return;
  }
  if (request.mode) mode.value = request.mode;
  const trackId = request.trackId ?? null;
  const clipId = request.clipId ?? null;

  // MIDI 参考：翻唱按这段旋律重唱（零推理写谱，选中即触发）。
  if (request.mode === "cover" && request.source === "midi") {
    coverSource.value = "midi";
    const track = midiTracks.value.find((t) => t.id === trackId);
    if (!track) return;
    coverMidiTrackId.value = String(track.id);
    const picked = clipId !== null && track.clips.some((c) => !c.audio && c.id === clipId);
    coverMidiClipId.value = picked ? String(clipId) : (coverMidiClipOptions.value[0]?.id ?? null);
    return;
  }

  // 音频参考：轨道上的**这一块**音频（没有就退回第一个音频片段）。
  const track = audioTracks.value.find((t) => t.id === trackId);
  if (!track) return;
  coverSource.value = "track";
  coverTrackId.value = String(track.id);
  coverClipId.value =
    clipId !== null && track.clips.some((c) => c.audio && c.id === clipId) ? String(clipId) : null;
}

onMounted(async () => {
  stopYue = yue.watch();
  void models.load();
  // 右侧面板是三个 AI 子页共用的滚动容器：从列表进来时先回到顶部。
  pageEl.value?.closest(".pane")?.scrollTo({ top: 0 });
  // 翻唱的参考默认跟当前选中的音频轨（与「音色替换」同一套直觉）。
  const preferred = audioTracks.value.find((t) => t.id === selection.trackId);
  const fallback = audioTracks.value[0];
  coverTrackId.value = String((preferred ?? fallback)?.id ?? "") || null;
  // 右键菜单送来的上下文（哪一块音频 / 哪段 MIDI）优先于「当前选中」。
  applyIntent(props.intent);
  // 引擎清单要等：一份都没有时要立刻弹安装向导。
  await Promise.all([yue.load(), runtimes.load()]);
  // 续写下拉只列「本机有语义码」的轨道——进来先扫一遍。
  await refreshContinuable();
});

// 页面已经打开时又从右键菜单进来（换了一块音频 / 一段旋律）：意图变了就重新套用。
watch(
  () => props.intent,
  (request) => applyIntent(request),
);

// 翻唱要 SheetSage2；缺就等用户真去点主按钮时再问——切个模式就弹框太吵。
// 切到「工程内 MIDI」：自动选第一条有 MIDI 的轨道 + 第一个片段，并立刻写谱。
watch(coverSource, (value, before) => {
  // 换了参考对象，旧谱就不再对应它了——清掉，免得「按上一段旋律唱」还以为是新的。
  if (before && before !== value) {
    abc.value = "";
    midiScoreInfo.value = null;
  }
  if (value !== "midi") return;
  if (!midiTracks.value.some((t) => String(t.id) === coverMidiTrackId.value)) {
    coverMidiTrackId.value = String(midiTracks.value[0]?.id ?? "") || null;
  }
  if (!coverMidiClipId.value) {
    coverMidiClipId.value = coverMidiClipOptions.value[0]?.id ?? null;
  }
});

// 换轨道 → 片段列表变了，回落到第一个。
watch(coverMidiTrackId, () => {
  const options = coverMidiClipOptions.value;
  if (!options.some((option) => option.id === coverMidiClipId.value)) {
    coverMidiClipId.value = options[0]?.id ?? null;
  }
});

// 选中片段 → 立刻写谱 + 带歌词（纯本地换算，不用点任何按钮）。
watch(coverMidiClipId, () => {
  if (coverSource.value !== "midi") return;
  void applyMidiScore();
});

// 参考轨道被删掉 / 换了工程：回落到仍然存在的第一个。
watch(
  () => audioTracks.value.map((t) => t.id).join(","),
  () => {
    if (!audioTracks.value.some((t) => String(t.id) === coverTrackId.value)) {
      const selected = audioTracks.value.find((t) => t.id === selection.trackId);
      coverTrackId.value = String((selected ?? audioTracks.value[0])?.id ?? "") || null;
    }
  },
);

onUnmounted(() => {
  stopYue?.();
});

// 生成完成 / 失败时弹一次通知。
/** 刚生成完的音频路径：等它出现在工程里，再把续写面板指过去。 */
const pendingContinueAudio = ref<string | null>(null);
watch(
  () => yue.job.state,
  (state, before) => {
    if (before !== "running") return;
    if ((state === "done" || state === "failed") && yue.job.message) {
      layout.notify(yue.job.message);
    }
    // 翻唱里谱是中间产物：只写谱跑完直接读回编辑器，省一次点击。
    // （创作模式没有谱面输入，结果用「把 ABC 谱导入工程」看。）
    if (mode.value === "cover" && state === "done" && yue.job.audio === null && yue.job.score) {
      void loadLastScore(true);
    }
    // 刚生成完：把续写面板自动指到这条新轨道（「接着刚才这首继续」是常态）。
    if (state === "done" && yue.job.audio) pendingContinueAudio.value = yue.job.audio;
  },
);

watch(
  audioTracks,
  async (tracks) => {
    // 工程变了：可续写列表可能跟着变（新歌加进来 / 旧歌被删）。
    await refreshContinuable();
    // 选中的那条没了或不再可续写：连前缀一起清掉。
    if (
      continueTrackId.value !== null &&
      !tracks.some((t) => String(t.id) === continueTrackId.value && continuable.value[continueTrackId.value ?? ""])
    ) {
      continueTrackId.value = null;
      clearPrefix();
    }
    const path = pendingContinueAudio.value;
    if (!path) return;
    const track = tracks.find((t) => t.clips.some((c) => c.audio?.path === path));
    if (!track) return;
    pendingContinueAudio.value = null;
    const id = String(track.id);
    if (mode.value === "create" && continuable.value[id]) {
      continueTrackId.value = id;
      // 刚生成的续写片段改变了「前缀指向的那段码」（新片段码更长）：旧前缀已经过期。
      // 清掉它，让用户重新点「以此续写」——否则下一次点生成会拿旧码配新总长。
      clearPrefix();
    }
  },
  { deep: true },
);
</script>

<template>
  <div ref="pageEl" class="ai-subpage yue-page">
    <header class="sub-head">
      <button
        class="m3-icon-btn"
        type="button"
        :aria-label="i18n.t('common.back')"
        @click="emit('back')"
      >
        <ArrowLeftIcon class="w-[15px] h-[15px]" />
      </button>
      <h1 class="sub-title">{{ i18n.t("yue.title") }}</h1>
      <span class="sub-badge" :class="{ on: yue.ready }">{{ engineBadge }}</span>
    </header>

    <!-- 引擎：起停 / 后端 / CUDA 预设 -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("yue.engineSection") }}</span>
          <span class="m3-card-sub">{{ engineLine }}</span>
        </div>
        <button v-if="yue.ready" class="m3-btn text" type="button" @click="yue.stop()">
          {{ i18n.t("yue.stopEngine") }}
        </button>
        <button
          v-else-if="!yue.starting"
          class="m3-btn text"
          type="button"
          :disabled="!enginePacks.length || engineDownloading"
          @click="startEngine"
        >
          {{ i18n.t(engineInstalled ? "yue.startEngine" : "yue.engineChoose") }}
        </button>
      </div>

      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.engineBackend") }}</span>
          <span class="m3-row-sub">{{ engineBackendLine }}</span>
        </div>
        <div class="m3-row-control">
          <button
            class="m3-btn text"
            type="button"
            :disabled="engineDownloading || yue.busy"
            @click="setupOpen = true"
          >
            {{ i18n.t(engineInstalled ? "yue.engineChange" : "yue.engineChoose") }}
          </button>
        </div>
      </div>

      <div v-if="cudaActive" class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.cudaPerf") }}</span>
          <span class="m3-row-sub">{{ i18n.t("yue.cudaPerfHint") }}</span>
        </div>
        <div class="m3-row-control">
          <Md3Switch v-model="cudaPerf" :label="i18n.t('yue.cudaPerf')" :disabled="yue.busy" />
        </div>
      </div>
    </section>

    <!-- 权重 -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("yue.weights") }}</span>
          <span class="m3-card-sub">{{ modelHint }}</span>
        </div>
      </div>
      <div class="m3-field block">
        <span class="m3-field-label">{{ i18n.t("yue.model") }}</span>
        <Md3Select
          v-model="modelId"
          :options="modelOptions"
          :label="i18n.t('yue.model')"
          :empty="i18n.t('yue.modelEmpty')"
          :disabled="yue.busy"
        />
      </div>
    </section>

    <!-- 创作参数 -->
    <section class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("yue.composeGroup") }}</span>
          <span class="m3-card-sub">{{ i18n.t(isYue2 ? "yue.composeHint" : "yue.onlyYue2") }}</span>
        </div>
      </div>

      <!-- 创作方式：创作（从零写）/ 翻唱（参考录音 → ABC 谱 → 按谱唱）。翻唱是 YuE2 专有的。 -->
      <template v-if="isYue2">
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.mode") }}</span>
          </div>
          <div class="m3-row-control">
            <Md3Segmented
              v-model="modeValue"
              :options="modeOptions"
              :aria-label="i18n.t('yue.mode')"
            />
          </div>
        </div>
        <p class="m3-hint mode-hint">
          {{ i18n.t(mode === "cover" ? "yue.modeCoverHint" : "yue.modeCreateHint") }}
        </p>
      </template>

      <!-- 翻唱：参考音频（来源 + 具体对象），排版与「音色替换」一致 -->
      <template v-if="mode === 'cover'">
        <div class="m3-field block">
          <span class="m3-field-label">{{ i18n.t("yue.coverSource") }}</span>
          <Md3Select
            v-model="coverSourceChoice"
            :options="coverSourceOptions"
            :label="i18n.t('yue.coverSource')"
            :disabled="yue.transcribing || yue.busy"
          />
        </div>

        <!-- 音频参考：轨道 / 文件 -->
        <template v-if="coverSource !== 'midi'">
          <div v-if="coverSource === 'track'" class="m3-field block">
            <span class="m3-field-label">{{ i18n.t("yue.coverAudio") }}</span>
            <Md3Select
              v-model="coverTrackId"
              :options="coverTrackOptions"
              :label="i18n.t('yue.coverAudio')"
              :empty="i18n.t('yue.coverNoTrack')"
              :disabled="yue.transcribing || yue.busy"
            />
          </div>
          <div v-else class="m3-field block">
            <span class="m3-field-label">{{ i18n.t("yue.coverAudio") }}</span>
            <div class="lora-line">
              <input
                v-model="coverFile"
                class="m3-input"
                type="text"
                spellcheck="false"
                :placeholder="i18n.t('yue.coverFileHint')"
              />
              <button class="m3-btn text" type="button" @click="pickCoverFile">
                {{ i18n.t("yue.coverPick") }}
              </button>
            </div>
          </div>

          <div class="m3-row">
            <div class="m3-row-main">
              <span class="m3-row-sub">{{ coverHint }}</span>
            </div>
            <div v-if="coverDownloading" class="m3-row-control">
              <button class="m3-btn text" type="button" @click="models.cancel()">
                {{ i18n.t("yue.coverCancel") }}
              </button>
            </div>
          </div>
        </template>

        <!-- MIDI 参考：旋律本来就在工程里，选中即写谱（零推理），歌词顺手带进歌词框 -->
        <template v-else>
          <div class="m3-field block">
            <span class="m3-field-label">{{ i18n.t("yue.coverMidiTrack") }}</span>
            <Md3Select
              v-model="coverMidiTrackId"
              :options="coverMidiTrackOptions"
              :label="i18n.t('yue.coverMidiTrack')"
              :empty="i18n.t('yue.coverNoMidi')"
              :disabled="yue.busy"
            />
          </div>
          <div class="m3-field block">
            <span class="m3-field-label">{{ i18n.t("yue.coverMidiClip") }}</span>
            <Md3Select
              v-model="coverMidiClipId"
              :options="coverMidiClipOptions"
              :label="i18n.t('yue.coverMidiClip')"
              :empty="i18n.t('yue.coverNoMidi')"
              :disabled="yue.busy"
            />
          </div>
          <div class="m3-row">
            <div class="m3-row-main">
              <span class="m3-row-sub">{{ coverHint }}</span>
            </div>
          </div>
        </template>

        <!-- 歌词：音频走 ASR 听写，MIDI 直接取音符上的词——同一个按钮、同一套状态。 -->
        <div v-if="coverSource !== 'midi'" class="m3-field block">
          <span class="m3-field-label">{{ i18n.t("yue.lyricsModel") }}</span>
          <Md3Select
            v-model="lyricsModelId"
            :options="lyricsModelOptions"
            :label="i18n.t('yue.lyricsModel')"
            :empty="i18n.t('yue.lyricsNoModel')"
            :disabled="yue.lyricizing || yue.busy"
          />
        </div>
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-sub">{{ lyricsHint }}</span>
          </div>
          <div class="m3-row-control">
            <button
              v-if="lyricsDownloading"
              class="m3-btn text"
              type="button"
              @click="models.cancel()"
            >
              {{ i18n.t("yue.coverCancel") }}
            </button>
            <button
              v-else-if="yue.lyricizing"
              class="m3-btn text"
              type="button"
              @click="yue.cancelLyrics()"
            >
              {{ i18n.t("yue.coverCancel") }}
            </button>
            <button
              v-else
              class="m3-btn text"
              type="button"
              :disabled="!lyricsCanRun"
              @click="getLyrics"
            >
              {{ i18n.t("yue.lyricsFetch") }}
            </button>
          </div>
        </div>
      </template>

      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.duration") }}</span>
        </div>
        <div class="m3-row-control">
          <NumberField
            v-model="duration"
            :sr-label="i18n.t('yue.duration')"
            :min="15"
            :max="600"
            :step="5"
            input-width="64px"
            :disabled="continueOn"
          />
          <span class="m3-row-value">{{ i18n.t("yue.seconds") }}</span>
        </div>
      </div>

      <div class="m3-fields single">
        <label class="m3-field">
          <!-- ACE-Step 把那句话当**提示词正文**（引擎回 `requires text_input`），不是标签，所以换个标签。 -->
          <span class="m3-field-label">{{ i18n.t(isYue2 ? "yue.style" : "yue.describe") }}</span>
          <input
            v-model="style"
            class="m3-input"
            type="text"
            spellcheck="false"
            :placeholder="i18n.t(isYue2 ? 'yue.styleHint' : 'yue.describeHint')"
          />
        </label>
        <label class="m3-field">
          <span class="m3-field-label">{{ i18n.t("yue.lyrics") }}</span>
          <textarea
            v-model="lyrics"
            class="m3-textarea"
            spellcheck="false"
            :placeholder="i18n.t('yue.lyricsHint')"
          />
        </label>
      </div>

      <!-- 续写：选一首在这里生成的曲子往下接（只有 YuE2 有语义码；翻唱有自己的流程，不混进来）。 -->
      <template v-if="mode === 'create' && isYue2">
        <div class="m3-field block">
          <span class="m3-field-label">{{ i18n.t("yue.continue") }}</span>
          <Md3Select
            v-model="continueTrackId"
            :options="continueTrackOptions"
            :label="i18n.t('yue.continue')"
            :empty="i18n.t('yue.continueNoTracks')"
            :disabled="yue.busy"
          />
          <span class="m3-hint">{{ i18n.t("yue.continueOnly") }}</span>
        </div>
        <div v-if="continueTrackId" class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-sub">{{ continueHint }}</span>
          </div>
          <div class="m3-row-control">
            <template v-if="continueOn">
              <NumberField
                v-model="continueSeconds"
                :sr-label="i18n.t('yue.continueAdd')"
                :min="15"
                :max="300"
                :step="15"
                input-width="64px"
              />
              <button class="m3-btn text" type="button" @click="clearPrefix">
                {{ i18n.t("yue.continueOff") }}
              </button>
            </template>
            <button
              v-else
              class="m3-btn text"
              type="button"
              :disabled="yue.busy"
              @click="useTrackAsPrefix"
            >
              {{ i18n.t("yue.continueUse") }}
            </button>
          </div>
        </div>
        <p v-if="continueInherit" class="m3-hint continue-inherit">{{ continueInherit }}</p>
      </template>

      <!-- 进阶参数默认收起：普通用户只需要风格 / 歌词 / 时长 -->
      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.advanced") }}</span>
          <span class="m3-row-sub">{{ i18n.t("yue.advancedHint") }}</span>
        </div>
        <div class="m3-row-control">
          <button class="m3-btn text" type="button" @click="advancedOpen = !advancedOpen">
            {{ i18n.t(advancedOpen ? "common.collapse" : "common.expand") }}
          </button>
        </div>
      </div>

      <template v-if="advancedOpen">
        <!-- 创作模式才有「写不写谱」的自由度；翻唱必须按转出来的谱唱；别的家族没有谱这一档。 -->
        <div v-if="mode === 'create' && isYue2" class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.planScore") }}</span>
            <span class="m3-row-sub">{{ i18n.t("yue.planScoreHint") }}</span>
          </div>
          <div class="m3-row-control">
            <Md3Switch
              v-model="planScore"
              :label="i18n.t('yue.planScore')"
              :disabled="yue.busy || continueOn"
            />
          </div>
        </div>

        <!-- 翻唱：谱是自动转出来的中间产物；想改音符的人才需要看到它 -->
        <template v-if="mode === 'cover'">
          <div class="m3-row">
            <div class="m3-row-main">
              <span class="m3-row-label">{{ i18n.t("yue.coverScore") }}</span>
              <span class="m3-row-sub">{{ i18n.t("yue.coverScoreNote") }}</span>
            </div>
            <div class="m3-row-control">
              <button
                v-if="coverSource !== 'midi'"
                class="m3-btn text"
                type="button"
                :disabled="!coverCanRun"
                @click="transcribeCover"
              >
                {{ i18n.t("yue.coverTranscribe") }}
              </button>
              <button
                v-if="abc.trim()"
                class="m3-btn text"
                type="button"
                :disabled="yue.busy || yue.transcribing"
                @click="abc = ''"
              >
                {{ i18n.t("yue.coverClearScore") }}
              </button>
            </div>
          </div>
          <label class="m3-field">
            <textarea
              v-model="abc"
              class="m3-textarea mono"
              spellcheck="false"
              :placeholder="i18n.t('yue.coverScoreHint')"
            />
          </label>
        </template>

        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.steps") }}</span>
            <span class="m3-row-sub">
              {{ i18n.t(isYue2 ? "yue.stepsHint" : "yue.stepsHintTurbo") }}
            </span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="steps"
              :sr-label="i18n.t('yue.steps')"
              :min="1"
              :max="stepsMax"
              input-width="64px"
            />
          </div>
        </div>
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.cfg") }}</span>
            <span class="m3-row-sub">{{ i18n.t("yue.cfgHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="cfg"
              :sr-label="i18n.t('yue.cfg')"
              :min="-1"
              :max="5"
              :step="0.05"
              :decimals="2"
              input-width="64px"
            />
          </div>
        </div>
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.seed") }}</span>
            <span class="m3-row-sub">{{ i18n.t("yue.seedHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="seed"
              :sr-label="i18n.t('yue.seed')"
              :min="-1"
              :max="2147483647"
              input-width="64px"
            />
          </div>
        </div>

        <!-- 14 个采样旋钮（YuE2 的谱规划 / 语义码两套）；-1 = 用引擎默认（不发这个字段）。 -->
        <template v-if="isYue2">
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.sampling") }}</span>
            <span class="m3-row-sub">{{ i18n.t("yue.samplingHint") }}</span>
          </div>
          <div class="m3-row-control">
            <button class="m3-btn text" type="button" @click="samplingOpen = !samplingOpen">
              {{ i18n.t(samplingOpen ? "common.collapse" : "common.expand") }}
            </button>
          </div>
        </div>

        <template v-if="samplingOpen">
          <p class="m3-hint mode-hint">{{ i18n.t("yue.samplingAbc") }}</p>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.temperature") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.temperature" :sr-label="i18n.t('yue.temperature')" :min="-1" :max="5" :step="0.05" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.topP") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.top_p" :sr-label="i18n.t('yue.topP')" :min="-1" :max="1" :step="0.01" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.topK") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.top_k" :sr-label="i18n.t('yue.topK')" :min="-1" :max="200" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.repPenalty") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.repetition_penalty" :sr-label="i18n.t('yue.repPenalty')" :min="-1" :max="4" :step="0.05" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.penaltyWindow") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.penalty_window" :sr-label="i18n.t('yue.penaltyWindow')" :min="-1" :max="4096" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.minTokens") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.min_tokens" :sr-label="i18n.t('yue.minTokens')" :min="-1" :max="8192" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.maxTokens") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="abcKnobs.max_tokens" :sr-label="i18n.t('yue.maxTokens')" :min="-1" :max="8192" input-width="64px" />
            </div>
          </div>

          <p class="m3-hint mode-hint">{{ i18n.t("yue.samplingSemantic") }}</p>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.temperature") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.temperature" :sr-label="i18n.t('yue.temperature')" :min="-1" :max="5" :step="0.05" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.topP") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.top_p" :sr-label="i18n.t('yue.topP')" :min="-1" :max="1" :step="0.01" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.topK") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.top_k" :sr-label="i18n.t('yue.topK')" :min="-1" :max="200" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.repPenalty") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.repetition_penalty" :sr-label="i18n.t('yue.repPenalty')" :min="-1" :max="4" :step="0.05" :decimals="2" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.penaltyWindow") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.penalty_window" :sr-label="i18n.t('yue.penaltyWindow')" :min="-1" :max="4096" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.minTokens") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.min_tokens" :sr-label="i18n.t('yue.minTokens')" :min="-1" :max="24000" input-width="64px" />
            </div>
          </div>
          <div class="m3-row">
            <div class="m3-row-main"><span class="m3-row-label">{{ i18n.t("yue.maxTokens") }}</span></div>
            <div class="m3-row-control">
              <NumberField v-model="semanticKnobs.max_tokens" :sr-label="i18n.t('yue.maxTokens')" :min="-1" :max="24000" input-width="64px" />
            </div>
          </div>
        </template>
        </template>
      </template>

      <!-- 整页只有这一条进度条：转谱 / 听写 / 下载 / 生成都走它，阶段和秒数在左下。 -->
      <div v-if="stage" class="progress-indeterminate" aria-hidden="true" />

      <!-- 和其它 AI 子页一样：主动作在卡片底部，不做悬浮固定。 -->
      <div class="m3-card-actions">
        <span v-if="stage" class="m3-hint">{{ stageText }}</span>
        <span v-else-if="yue.job.state === 'done'" class="m3-hint">{{ i18n.t("yue.added") }}</span>
        <span v-else-if="actionHint" class="m3-hint">{{ actionHint }}</span>
        <span class="spacer" />
        <button
          v-if="yue.job.state === 'done' && yue.job.record"
          class="m3-btn text"
          type="button"
          @click="loadRecord"
        >
          {{ i18n.t("yue.replay") }}
        </button>
        <button
          v-if="mode === 'cover' && yue.job.state === 'done' && yue.job.score"
          class="m3-btn text"
          type="button"
          :disabled="loadingScore"
          @click="loadLastScore(false)"
        >
          {{ i18n.t("yue.loadScore") }}
        </button>
        <button
          v-if="yue.job.state === 'done' && yue.job.score"
          class="m3-btn text"
          type="button"
          @click="importScore"
        >
          {{ i18n.t("yue.importScore") }}
        </button>
        <button class="m3-btn filled" type="button" :disabled="primaryDisabled" @click="runPrimary">
          {{ primaryLabel }}
        </button>
      </div>
    </section>

    <!-- LoRA：两半各一份（YuE2 的适配器是 AR / NAR 两半，别的家族没有） -->
    <section v-if="isYue2" class="m3-card">
      <div class="m3-card-head">
        <div class="m3-row-main">
          <span class="m3-card-title">{{ i18n.t("yue.lora") }}</span>
          <span class="m3-card-sub">{{ i18n.t("yue.loraHint") }}</span>
        </div>
        <button
          class="m3-btn text"
          type="button"
          :disabled="!loraDirty || yue.busy"
          @click="saveLora"
        >
          {{ i18n.t("common.save") }}
        </button>
      </div>

      <div class="m3-field block">
        <span class="m3-field-label">{{ i18n.t("yue.loraAr") }}</span>
        <div class="lora-line">
          <input
            v-model="loraDraft.ar_path"
            class="m3-input"
            type="text"
            spellcheck="false"
            :placeholder="i18n.t('yue.loraPath')"
          />
          <button class="m3-btn text" type="button" @click="pickAr">
            {{ i18n.t("yue.loraPick") }}
          </button>
        </div>
        <div class="lora-scale">
          <span class="m3-row-label">{{ i18n.t("yue.loraScale") }}</span>
          <NumberField
            v-model="loraDraft.ar_scale"
            :sr-label="i18n.t('yue.loraScale')"
            :min="0"
            :max="2"
            :step="0.05"
            :decimals="2"
            input-width="64px"
          />
        </div>
      </div>

      <div class="m3-field block">
        <span class="m3-field-label">{{ i18n.t("yue.loraNar") }}</span>
        <div class="lora-line">
          <input
            v-model="loraDraft.nar_path"
            class="m3-input"
            type="text"
            spellcheck="false"
            :placeholder="i18n.t('yue.loraPath')"
          />
          <button class="m3-btn text" type="button" @click="pickNar">
            {{ i18n.t("yue.loraPick") }}
          </button>
        </div>
        <div class="lora-scale">
          <span class="m3-row-label">{{ i18n.t("yue.loraScale") }}</span>
          <NumberField
            v-model="loraDraft.nar_scale"
            :sr-label="i18n.t('yue.loraScale')"
            :min="0"
            :max="2"
            :step="0.05"
            :decimals="2"
            input-width="64px"
          />
        </div>
      </div>
    </section>

    <!-- 缺权重时的应用内 MD3 确认框（不用系统原生框）：确认就既跳管理器又开始下载。 -->
    <ConfirmDialog
      v-model:open="modelAskOpen"
      :title="i18n.t('dialog.downloadModelTitle')"
      :message="modelAskMessage"
      :confirm-label="i18n.t('dialog.downloadModelOk')"
      :extra-label="i18n.t('setup.background')"
      @confirm="confirmModelDownload"
      @extra="backgroundModelDownload"
    />

    <!-- 缺引擎 / 缺权重：一份清单说清「还差什么、合计多少」，可跳转下载或留在后台。 -->
    <SetupDialog
      v-model:open="setupPlanOpen"
      :plan="setupPlan"
      :dismissible="!setupForced"
      @confirm="yueSetup.confirm()"
      @background="yueSetup.confirmBackground()"
      @cancel="yueSetup.cancel()"
      @choose="chooseEngine"
    />

    <EngineSetupDialog
      v-model:open="setupOpen"
      :dismissible="engineInstalled"
      @cancel="onSetupCancel"
    />
  </div>
</template>

<style scoped>
/* 侧栏只有 ~330 px：多行字段单列；数值控件统一 64px，和时长右边对齐。 */
.yue-page .single {
  grid-template-columns: minmax(0, 1fr);
}
.yue-page .block {
  padding: 8px 0;
}
.yue-page .mode-hint {
  margin: -2px 0 4px;
}
/* 续写「沿用原曲设置」那一行：比普通说明再弱一档，不抢「加长」控件的注意力。 */
.yue-page .continue-inherit {
  margin: -2px 0 4px;
  color: rgb(var(--md-on-surface-variant) / 0.85);
}
.yue-page .m3-input {
  width: 100%;
}
.yue-page .m3-textarea.mono {
  min-height: 64px;
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
.yue-page .progress-indeterminate {
  margin: 2px 0 6px;
}
.yue-page .spacer {
  flex: 1 1 auto;
}
.yue-page .lora-line {
  display: flex;
  align-items: center;
  gap: 4px;
}
.yue-page .lora-scale {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 4px;
}
</style>
