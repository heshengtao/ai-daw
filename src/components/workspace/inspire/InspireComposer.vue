<script setup lang="ts">
/**
 * 灵感页左半：**创作面板**（版式参考 SUNO 的生成页：模式 / 模型 / 描述 / 歌词 / 生成）。
 *
 * 只借它的**结构**，不借它的商业件：没有「升级解锁」「积分」，也没有任何假按钮。
 * 控件全部是项目里已有的 MD3 原语（`m3-input` / `m3-chip` / `Md3Select` /
 * `Md3Segmented` / `NumberField`），所以深浅色、键盘焦点、禁用态与其它面板一致。
 *
 * 两种模式：
 * - **简单**：一句描述就生成（歌词由模型写）；
 * - **进阶**：多出歌词编辑器（带结构标签）、时长、步数、种子。
 * 切回简单**不会丢歌词**：只要歌词框里有字，它照样会被唱出来（旁边会提示一句）。
 *
 * 本地与云端共用一个下拉（`useMusicSource`）：云端服务存在时选它就走云端，否则走本地权重。
 * 本地生成前要先过三道门槛：引擎包装了没、权重下了没、引擎起了没——缺哪道就补哪道，
 * 不需要用户自己去别处找设置。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { SparklesIcon, TrashIcon } from "@heroicons/vue/24/outline";
import Md3Chip from "@/components/common/Md3Chip.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import NumberField from "@/components/common/NumberField.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import EngineSetupDialog from "@/components/common/EngineSetupDialog.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import { formatDuration } from "./format";
import { STYLE_PRESETS } from "./styles";
import { useI18n } from "@/i18n";
import { useInspireStore, MAX_BATCH } from "@/stores/inspire";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useRuntimesStore } from "@/stores/runtimes";
import { useYueStore } from "@/stores/yue";
import { useDownloadsStore } from "@/stores/downloads";
import { useMusicSource } from "@/composables/useMusicSource";
import { useModelDownloadPrompt } from "@/composables/useModelDownloadPrompt";
import { humanSize, modelName } from "@/utils/models";

const i18n = useI18n();
const inspire = useInspireStore();
const layout = useLayoutStore();
const models = useModelsStore();
const runtimes = useRuntimesStore();
const yue = useYueStore();
const downloads = useDownloadsStore();
const modelPrompt = useModelDownloadPrompt();
/** 解构出来当顶层 ref 用（模板里才会自动解包）。 */
const {
  open: promptOpen,
  message: promptMessage,
  require: requireModel,
  confirm: confirmDownload,
  background: backgroundDownload,
  cancel: cancelDownload,
} = modelPrompt;

const music = useMusicSource();
const { value: sourceValue, options: sourceOptions, provider: cloudProvider, localModel, isCloud } = music;

/**
 * 纯音乐开关**写进风格串**的标记。
 *
 * 两道保险缺一不可：歌词会被后端整份换成 `[instrumental]` 标签块（真正管住人声的那一步，
 * 见 `yue_engine::INSTRUMENTAL_LYRICS`），风格里再明确说一句「不要人声」。这里让用户
 * **看得见**这段文字（在风格框里），想改随时能改。
 */
const INSTRUMENTAL_TAG = "instrumental, no vocals";

/**
 * 当前选的是不是本地的 **ACE-Step**。
 *
 * ACE-Step 的器乐路径就是「歌词留空」，不需要、也不能靠风格里的否定式提示（会被规划器的
 * CoT 重写带偏，实测反而「乱弹」）。所以对它不加标签，让「纯音乐」= 和「不勾 + 空歌词」
 * 完全一样的请求。
 */
const aceLocalModel = computed(
  () => !isCloud.value && /^ace-step-/.test(localModel.value?.id ?? ""),
);

function promptMentionsInstrumental(text: string): boolean {
  return /\binstrumental\b/i.test(text);
}

/** 打开纯音乐：风格里补上器乐标记（ACE-Step 除外，见上）。 */
function addInstrumentalIntent(): void {
  if (aceLocalModel.value) return;
  const current = prompt.value.trim();
  if (promptMentionsInstrumental(current)) return;
  prompt.value = current ? `${current}, ${INSTRUMENTAL_TAG}` : INSTRUMENTAL_TAG;
}

/**
 * 关掉纯音乐：把风格里的器乐标记撤掉。
 *
 * **不认位置**：打开后用户可能又点了风格预设，预设是往末尾追加的，标记就被挤到中间去了
 * （`jazz, instrumental, no vocals, lo-fi`）。所以按整段短语找、删掉后顺手收拾逗号。
 * 短语本身很具体（`instrumental, no vocals`），不会误伤用户写的普通风格词。
 */
function removeInstrumentalIntent(): void {
  const pattern = /instrumental\s*,\s*no\s+vocals/i;
  if (!pattern.test(prompt.value)) return;
  prompt.value = prompt.value
    .replace(pattern, "")
    .replace(/\s*,\s*,\s*/g, ", ")
    .replace(/^\s*,\s*/, "")
    .replace(/\s*,\s*$/, "")
    .replace(/\s{2,}/g, " ")
    .trim();
}
/** 歌词里可插入的结构标签（与 YuE2 / 云端服务都认的那套一致）。 */
const SECTION_TAGS = ["[Verse]", "[Chorus]", "[Bridge]", "[Outro]"];

/** 草稿：切走再回来不该从零开始（与「设置页」那种一次性表单不同，这里是长期在用的工作台）。 */
const DRAFT_KEY = "ai-daw-inspire-draft";
interface Draft {
  prompt: string;
  lyrics: string;
  duration: number;
  steps: number;
  seed: number;
  instrumental: boolean;
  advanced: boolean;
  /** 一批生成几首（1 = 单首）。 */
  count: number;
}

function loadDraft(): Partial<Draft> {
  try {
    const raw = JSON.parse(localStorage.getItem(DRAFT_KEY) ?? "{}") as Partial<Draft>;
    return raw && typeof raw === "object" ? raw : {};
  } catch {
    return {};
  }
}

const draft = loadDraft();
const advanced = ref(draft.advanced === true);
const prompt = ref(typeof draft.prompt === "string" ? draft.prompt : "");
const lyrics = ref(typeof draft.lyrics === "string" ? draft.lyrics : "");
const duration = ref(typeof draft.duration === "number" ? draft.duration : 120);
const steps = ref(typeof draft.steps === "number" ? draft.steps : 32);
const seed = ref(typeof draft.seed === "number" ? draft.seed : -1);
/** 一批几首：串行跑，每首换一个种子（固定种子 +1），拿到 N 个不同的 take。 */
const count = ref(
  typeof draft.count === "number" ? Math.min(MAX_BATCH, Math.max(1, draft.count)) : 1,
);

/** 纯音乐 / 有人声。默认有人声——与改动前「歌词留空 = 模型写词」的行为一致。 */
const instrumental = ref(draft.instrumental === true);

// 开关：打开就往风格里补器乐标记，关掉就只撤我们补的那一段（用户自己写的
// 「instrumental」不动）。**不在挂载时跑删除**——草稿里可能本来就是用户手写的标签。
watch(instrumental, (on, was) => {
  if (on) addInstrumentalIntent();
  else if (was) removeInstrumentalIntent();
});
// 换模型（本地权重 / 云端服务）时把标记补回去：它标的是「这首要不要人声」。
watch(
  localModel,
  () => {
    if (instrumental.value) addInstrumentalIntent();
  },
  { immediate: true },
);
// 切到 / 切走 ACE-Step：它不吃否定式标签，切过去时把之前加的那段撤掉。
watch(aceLocalModel, (ace) => {
  if (ace) removeInstrumentalIntent();
  else if (instrumental.value) addInstrumentalIntent();
});

const setupOpen = ref(false);
const preparing = ref(false);
const lyricsEl = ref<HTMLTextAreaElement | null>(null);
/** 「启动引擎→等就绪」的轮询句柄：离开页面要收掉，否则会一直等下去。 */
let waitTimer: number | null = null;

const modeValue = computed<string>({
  get: () => (advanced.value ? "advanced" : "simple"),
  set: (value) => {
    advanced.value = value === "advanced";
  },
});

const modeOptions = computed<SegmentedOption[]>(() => [
  { value: "simple", label: i18n.t("inspire.modeSimple"), title: i18n.t("inspire.modeSimpleHint") },
  {
    value: "advanced",
    label: i18n.t("inspire.modeAdvanced"),
    title: i18n.t("inspire.modeAdvancedHint"),
  },
]);

// ---- 本地引擎 / 权重状态 ----
const enginePacks = computed(() => runtimes.items.filter((r) => r.group === "audiocpp"));
const engineInstalled = computed(() => enginePacks.value.some((pack) => pack.installed));
const modelReady = computed(() => localModel.value?.installed === true);
const weightDownloading = computed(() =>
  localModel.value ? models.installing(localModel.value.id) : false,
);
/** ACE-Step 的 **Turbo** 档：蒸馏成 8 步，再多也不会更细。 */
const turboModel = computed(() => {
  const model = localModel.value;
  return model?.family === "ace_step" && !/base|sft/i.test(model.id);
});

/**
 * 步数上限跟着「档位」走：YuE2 常用 32 步；ACE-Step 的 Turbo 是 8 步上限，
 * Base/SFT 才是质量档，32–64 步才有意义。
 */
const stepsMax = computed(() => {
  const model = localModel.value;
  if (model?.family !== "ace_step") return 128;
  return turboModel.value ? 8 : 64;
});

/**
 * 步数的**默认值**也跟着档位走：切到 Turbo → 8，切到别的模型 → 32。
 *
 * 只在「档位」变化时重置（YuE2 的 q8 ↔ bf16 之间来回切不会把你手填的数冲掉）；
 * 挂载时不覆盖草稿里的值。
 */
const stepsPreset = computed(() => (turboModel.value ? 8 : 32));
watch(stepsPreset, (value, before) => {
  if (before === undefined || before === value) return;
  steps.value = value;
});

/** 云端服务允许的最长时长（没拉到预设时按 300 秒兜底）。 */
const maxSeconds = computed(() => {
  const preset = music.cloudMusic.presets.find((item) => item.kind === cloudProvider.value?.kind);
  return preset?.max_seconds ?? 300;
});
watch(maxSeconds, (max) => {
  if (duration.value > max) duration.value = max;
});

/** 底部那行状态：说清「现在能不能生成、缺什么」。 */
const statusLine = computed(() => {
  if (inspire.crafting) {
    const at = inspire.batch;
    if (at) {
      return i18n.t("inspire.batchProgress", {
        index: Math.min(at.total, at.done + 1),
        total: at.total,
        time: formatDuration(inspire.elapsed),
      });
    }
    return `${inspire.craftLine} · ${formatDuration(inspire.elapsed)}`;
  }
  if (isCloud.value) {
    return cloudProvider.value
      ? i18n.t("inspire.hintCloud", { name: cloudProvider.value.name })
      : i18n.t("inspire.hintNoProvider");
  }
  const model = localModel.value;
  if (!model) return i18n.t("inspire.hintNoModel");
  if (weightDownloading.value) return i18n.t("inspire.hintDownloading");
  if (!modelReady.value) {
    return i18n.t("inspire.hintNeedDownload", { size: humanSize(model.approx_bytes) });
  }
  if (!engineInstalled.value) return i18n.t("inspire.hintNoEngine");
  if (!yue.ready) {
    return i18n.t(yue.starting ? "inspire.hintEngineLoading" : "inspire.hintEngineStopped");
  }
  return i18n.t("inspire.hintReady", { model: modelName(model) });
});

const canSubmit = computed(() => {
  if (inspire.crafting) return true;
  if (preparing.value) return false;
  const hasLyrics = !instrumental.value && Boolean(lyrics.value.trim());
  return Boolean(prompt.value.trim()) || hasLyrics;
});

const primaryLabel = computed(() => {
  if (inspire.crafting) return i18n.t("inspire.stop");
  if (preparing.value) return i18n.t("inspire.preparing");
  return i18n.t("inspire.create");
});

/** 徽章文字：本地是权重档名，云端是「服务 · 模型」。 */
function badgeOfLocal(): string {
  const model = localModel.value;
  return model ? modelName(model) : "";
}

function submitLocal(): void {
  const model = localModel.value;
  if (!model) return;
  void inspire.craftBatch(
    {
      prompt: prompt.value,
      lyrics: instrumental.value ? "" : lyrics.value,
      model: badgeOfLocal(),
      source: "local",
      duration: duration.value,
      target: model.id,
      steps: steps.value,
      seed: seed.value,
      instrumental: instrumental.value,
    },
    count.value,
  );
}

function submitCloud(): void {
  const provider = cloudProvider.value;
  if (!provider) return;
  void inspire.craftBatch(
    {
      prompt: prompt.value,
      lyrics: instrumental.value ? "" : lyrics.value,
      model: provider.model ? `${provider.name} · ${provider.model}` : provider.name,
      source: "cloud",
      duration: duration.value,
      target: provider.id,
      instrumental: instrumental.value,
    },
    count.value,
  );
}

/** 启动引擎并等它真的就绪（加载几 GB 权重是分钟级，期间按钮显示「准备中」）。 */
async function startEngineAndWait(modelId: string): Promise<boolean> {
  await yue.start(modelId);
  const started = Date.now();
  return await new Promise<boolean>((resolve) => {
    if (waitTimer !== null) window.clearInterval(waitTimer);
    waitTimer = window.setInterval(() => {
      const finish = (ok: boolean, message?: string): void => {
        if (waitTimer !== null) {
          window.clearInterval(waitTimer);
          waitTimer = null;
        }
        if (message) layout.notify(message);
        resolve(ok);
      };
      if (yue.ready) return finish(true);
      const state = yue.engine?.state;
      if (state === "error") {
        return finish(false, yue.engine?.message ?? yue.engine?.log ?? i18n.t("inspire.engineError"));
      }
      // 兜底：不该无限等（正常情况下 1-6 分钟）。超时就放开按钮，让用户自己再点一次。
      if (Date.now() - started > 15 * 60_000) return finish(false, i18n.t("inspire.engineTimeout"));
    }, 400);
  });
}

async function run(): Promise<void> {
  if (inspire.crafting) {
    await inspire.cancelCraft();
    return;
  }
  if (preparing.value || !canSubmit.value) return;
  inspire.clearError();

  if (isCloud.value) {
    submitCloud();
    return;
  }

  const model = localModel.value;
  if (!model) {
    layout.notify(i18n.t("inspire.hintNoModel"));
    downloads.jumpTo("model", "*");
    return;
  }
  // 缺权重：确认后下载（不跳页，进度看页顶下载条），下完再点生成。
  if (!model.installed) {
    requireModel(model);
    return;
  }
  // 一个引擎包都没装：先选一份（官方便携包 100 MB – 1 GB，不能替用户默认选）。
  if (!engineInstalled.value) {
    setupOpen.value = true;
    return;
  }
  if (!yue.ready) {
    preparing.value = true;
    try {
      const ok = await startEngineAndWait(model.id);
      if (!ok) return;
    } finally {
      preparing.value = false;
    }
  }
  submitLocal();
}

/** 在光标处插入结构标签（独占一行）；不吞掉已经写好的词。 */
function insertTag(tag: string): void {
  const el = lyricsEl.value;
  const value = lyrics.value;
  const start = el?.selectionStart ?? value.length;
  const end = el?.selectionEnd ?? value.length;
  const before = value.slice(0, start);
  const after = value.slice(end);
  const lead = before && !before.endsWith("\n") ? "\n" : "";
  const insert = `${lead}${tag}\n`;
  lyrics.value = `${before}${insert}${after}`;
  const caret = before.length + insert.length;
  // 先等 v-model 把新值写进 DOM，再挪光标：太早设 `selectionStart` 会被旧长度夹住。
  void nextTick(() => {
    el?.focus();
    el?.setSelectionRange(caret, caret);
  });
}

/** 清空输入（不碰已经生成出来的记录）。 */
function discardDraft(): void {
  prompt.value = "";
  lyrics.value = "";
}

/**
 * 点芯片 = 切换这一段 tag：没有就追加、有就整段删掉。
 *
 * 选中态不用额外的 ref，直接看提示词里有没有这段子串——用户手改提示词时芯片状态
 * 不会和内容打架（删掉其中一个词，芯片自己就灭了）。
 */
function togglePreset(tags: string): void {
  const value = prompt.value;
  if (value.includes(tags)) {
    prompt.value = value
      .replace(tags, "")
      .replace(/(\s*,\s*){2,}/g, ", ")
      .replace(/^[\s,]+|[\s,]+$/g, "");
    return;
  }
  const trimmed = value.trim().replace(/[\s,]+$/, "");
  prompt.value = trimmed ? `${trimmed}, ${tags}` : tags;
}

// 草稿持久化：只在停下来之后写，别每敲一个字打一次 localStorage。
let draftTimer: number | null = null;
watch(
  [prompt, lyrics, duration, steps, seed, advanced, count],
  () => {
    if (draftTimer !== null) window.clearTimeout(draftTimer);
    draftTimer = window.setTimeout(() => {
      try {
        localStorage.setItem(
          DRAFT_KEY,
          JSON.stringify({
            prompt: prompt.value,
            lyrics: lyrics.value,
            duration: duration.value,
            steps: steps.value,
            seed: seed.value,
            instrumental: instrumental.value,
            advanced: advanced.value,
            count: count.value,
          } satisfies Draft),
        );
      } catch {
        // 存不下不影响创作。
      }
    }, 400);
  },
  { deep: false },
);

onMounted(() => {
  // 清单与预设：模型下拉要名称与「已下载 / 需下载」，云端的时长上限来自预设。
  void models.load();
  void runtimes.load();
  void yue.load();
  void music.load();
  void music.cloudMusic.loadPresets();
});

onUnmounted(() => {
  if (waitTimer !== null) window.clearInterval(waitTimer);
  if (draftTimer !== null) window.clearTimeout(draftTimer);
});
</script>

<template>
  <aside class="composer" :aria-label="i18n.t('inspire.title')">
    <header class="composer-head">
      <h1 class="composer-title">{{ i18n.t("inspire.title") }}</h1>
      <Md3Segmented v-model="modeValue" :options="modeOptions" :aria-label="i18n.t('inspire.mode')" />
    </header>

    <div class="composer-scroll">
      <!-- 模型：本地权重 + 云端服务共用一份列表（与「AI 写歌」同一个下拉） -->
      <div class="field">
        <span class="field-label">{{ i18n.t("inspire.model") }}</span>
        <Md3Select
          v-model="sourceValue"
          :options="sourceOptions"
          :label="i18n.t('inspire.model')"
          :disabled="inspire.crafting"
          @action="music.onAction"
        />
      </div>

      <!-- 描述：一句话就是一首歌 -->
      <label class="field">
        <span class="field-label">{{ i18n.t("inspire.style") }}</span>
        <div class="tags preset-tags">
          <Md3Chip
            v-for="preset in STYLE_PRESETS"
            :key="preset.id"
            size="sm"
            :selected="prompt.includes(preset.tags)"
            :disabled="inspire.crafting"
            @click="togglePreset(preset.tags)"
          >
            {{ i18n.t(preset.label) }}
          </Md3Chip>
        </div>
        <textarea
          v-model="prompt"
          class="m3-textarea prompt-input"
          spellcheck="false"
          :rows="4"
          :placeholder="i18n.t('inspire.stylePlaceholder')"
          :disabled="inspire.crafting"
        />
        <p class="hint">{{ i18n.t("inspire.styleHint") }}</p>
      </label>

      <!-- 进阶：歌词 + 结构标签 -->
      <section v-if="advanced" class="field">
        <div class="field-head">
          <span class="field-label">{{ i18n.t("inspire.lyrics") }}</span>
          <span class="field-sub">{{ i18n.t("inspire.lyricsHint") }}</span>
        </div>
        <div class="tags">
          <Md3Chip
            v-for="tag in SECTION_TAGS"
            :key="tag"
            size="sm"
            variant="assist"
            :disabled="inspire.crafting"
            @click="insertTag(tag)"
          >
            {{ tag }}
          </Md3Chip>
        </div>
        <textarea
          ref="lyricsEl"
          v-model="lyrics"
          class="m3-textarea lyrics-input"
          spellcheck="false"
          :rows="9"
          :placeholder="i18n.t('inspire.lyricsPlaceholder')"
          :disabled="inspire.crafting || instrumental"
        />
      </section>

      <!-- 简单模式：没开纯音乐就给一个紧凑的歌词框（开了就不出现——歌词不参与生成）；
           歌词内容一直保留，切回「有人声」时会原样回来 -->
      <label v-else-if="!instrumental" class="field">
        <div class="field-head">
          <span class="field-label">{{ i18n.t("inspire.lyrics") }}</span>
          <span class="field-sub">{{ i18n.t("inspire.lyricsOptional") }}</span>
        </div>
        <textarea
          v-model="lyrics"
          class="m3-textarea lyrics-input simple-lyrics"
          spellcheck="false"
          :rows="3"
          :placeholder="i18n.t('inspire.lyricsSimplePlaceholder')"
          :disabled="inspire.crafting"
        />
      </label>

      <!-- 纯音乐：一个开关，左标签右开关（开 = 纯音乐，不再靠「歌词是否为空」猜） -->
      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("inspire.instrumental") }}</span>
          <span class="m3-row-sub">{{ i18n.t("inspire.instrumentalSwitchHint") }}</span>
        </div>
        <div class="m3-row-control">
          <Md3Switch
            v-model="instrumental"
            :label="i18n.t('inspire.instrumental')"
            :disabled="inspire.crafting"
          />
        </div>
      </div>
      <!-- 打开纯音乐后：说清楚「歌词去哪了、为什么这样能行」——歌词换成标签块 + 风格写明器乐 -->
      <p v-if="instrumental" class="instrumental-hint">
        {{ i18n.t("inspire.instrumentalHint") }}
      </p>

      <!-- 进阶参数：时长 / 数量 / 步数 / 种子 -->
      <div v-if="advanced" class="params">
        <label class="param">
          <span class="param-label">{{ i18n.t("inspire.duration") }}</span>
          <NumberField
            v-model="duration"
            :sr-label="i18n.t('inspire.duration')"
            :min="15"
            :max="isCloud ? maxSeconds : 600"
            :step="5"
            input-width="54px"
            :disabled="inspire.crafting"
          />
        </label>
        <!-- 一批几首：串行跑，每首换一个种子；1 = 现在的单首行为 -->
        <label class="param">
          <span class="param-label">{{ i18n.t("inspire.takes") }}</span>
          <NumberField
            v-model="count"
            :sr-label="i18n.t('inspire.takes')"
            :min="1"
            :max="MAX_BATCH"
            input-width="42px"
            :disabled="inspire.crafting"
          />
        </label>
        <label v-if="!isCloud" class="param">
          <span class="param-label">{{ i18n.t("inspire.steps") }}</span>
          <NumberField
            v-model="steps"
            :sr-label="i18n.t('inspire.steps')"
            :min="1"
            :max="stepsMax"
            input-width="54px"
            :disabled="inspire.crafting"
          />
        </label>
        <label v-if="!isCloud" class="param">
          <span class="param-label">{{ i18n.t("inspire.seed") }}</span>
          <NumberField
            v-model="seed"
            :sr-label="i18n.t('inspire.seed')"
            :min="-1"
            :max="2147483647"
            input-width="72px"
            :disabled="inspire.crafting"
          />
        </label>
      </div>

      <p v-if="advanced" class="hint">{{ i18n.t("inspire.seedHint") }}</p>
      <!-- 数量 > 1：说清楚是「串行排队」，不是一次并行出 N 首 -->
      <p v-if="advanced && count > 1" class="hint">
        {{ i18n.t("inspire.batchHint", { count }) }}
      </p>
      <!-- 歌词留空意味着什么，本地与云端不一样：云端是显式的「纯音乐」开关，本地是让模型自己写词 -->
      <p v-if="!instrumental" class="hint">{{ i18n.t("inspire.lyricsAuto") }}</p>
    </div>

    <footer class="composer-foot">
      <div class="foot-row">
        <Tooltip :text="i18n.t('inspire.discard')">
          <button
            class="discard"
            type="button"
            :aria-label="i18n.t('inspire.discard')"
            :disabled="inspire.crafting || (!prompt && !lyrics)"
            @click="discardDraft"
          >
            <TrashIcon class="discard-icon" />
          </button>
        </Tooltip>

        <button
          class="create"
          :class="{ stop: inspire.crafting }"
          type="button"
          :disabled="!canSubmit"
          @click="run"
        >
          <SparklesIcon v-if="!inspire.crafting" class="create-icon" />
          <span class="create-label">{{ primaryLabel }}</span>
        </button>
      </div>

      <p class="status" :class="{ error: Boolean(inspire.error) }">{{ statusLine }}</p>
    </footer>

    <ConfirmDialog
      v-model:open="promptOpen"
      :title="i18n.t('inspire.downloadTitle')"
      :message="promptMessage"
      :confirm-label="i18n.t('inspire.downloadConfirm')"
      :extra-label="i18n.t('setup.background')"
      @confirm="confirmDownload()"
      @extra="backgroundDownload()"
      @cancel="cancelDownload()"
    />

    <EngineSetupDialog v-model:open="setupOpen" :dismissible="engineInstalled" />
  </aside>
</template>

<style scoped>
.composer {
  /* 宽度按**中栏**的宽度取比例（不是视口）：左右两栏都开着时中栏只有 800–1100px，
     写死 424 会把右边的记录列表挤成一条缝。`flex: 0 0 auto` + `width` 是刻意的——
     `flex: 0 0 clamp(...)` 这种写法浏览器会当非法值丢掉（实测会塌成内容宽度）。 */
  flex: 0 0 auto;
  width: clamp(300px, 42%, 424px);
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background-color: var(--app-inset);
  border-right: 1px solid rgb(var(--md-outline-variant) / 0.4);
}

.composer-head {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 18px 20px 14px;
}
.composer-title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  letter-spacing: 0.1px;
  color: rgb(var(--md-on-surface));
  flex: 0 0 auto;
}
/* 模式胶囊推到右边，像 SUNO 一样和标题同一行 */
.composer-head :deep(.md3-seg) {
  margin-left: auto;
}

.composer-scroll {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  padding: 2px 20px 16px;
  display: flex;
  flex-direction: column;
  gap: 16px;
  scrollbar-width: thin;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}
.field-label {
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.4px;
  color: rgb(var(--md-on-surface-muted));
}
.field-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.field-sub {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}

/* 多行输入：沿用 m3-textarea 的描边 / 底色，只把高度钉住（侧栏宽度固定，不需要 resize 把手） */
.prompt-input {
  min-height: 96px;
  resize: none;
}
.lyrics-input {
  min-height: 190px;
  resize: none;
  font-family: var(--font-mono, inherit);
  line-height: 1.7;
}
/* 简单模式只给 3 行：够写副歌，又不把「一句描述就能生成」的轻量感挤掉 */
.simple-lyrics {
  min-height: 76px;
}
/* 文本框里的结构标签用等宽字体更整齐，但正文别跟着变窄 */
.lyrics-input::placeholder {
  font-family: inherit;
}

.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
/* 风格芯片：一行放不下就换行，横向不滚（面板只有 400 多 px）。 */
.preset-tags {
  margin-bottom: 2px;
}
.tag {
  height: 24px;
  padding: 0 9px;
  font-size: 10.5px;
  font-family: var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace);
}

.params {
  display: flex;
  flex-wrap: wrap;
  gap: 14px;
}
.param {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.param-label {
  font-size: 10.5px;
  color: rgb(var(--md-on-surface-muted));
}

.hint {
  margin: 0;
  font-size: 10.5px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}

/* 纯器乐说明行：告诉用户「歌词已清空、风格里已写明器乐意图」 */
.instrumental-hint {
  margin: -4px 0 2px;
  font-size: 11px;
  line-height: 1.45;
  color: rgb(var(--md-on-surface-muted));
}
.instrumental-hint.on {
  color: rgb(var(--md-primary));
}

/* 简单模式下「歌词还在」的提示条：整条可点，直接切回进阶 */
.carried {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 12px;
  border: 1px dashed rgb(var(--md-outline-variant) / 0.8);
  border-radius: var(--md-shape-corner-md);
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  text-align: start;
  cursor: pointer;
  transition: border-color var(--md-duration-short-2) var(--md-ease-standard),
    background-color var(--md-duration-short-2) var(--md-ease-standard);
}
.carried:hover {
  border-color: rgb(var(--md-primary) / 0.6);
  background-color: rgb(var(--md-primary) / 0.06);
}
.carried-text {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.carried-link {
  flex: 0 0 auto;
  color: rgb(var(--md-primary));
  font-weight: 600;
}

/* ---- 底部动作区 ---- */
.composer-foot {
  flex: 0 0 auto;
  padding: 10px 20px 16px;
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.35);
  background-color: var(--app-inset);
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.foot-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

/* 丢弃草稿：安静的小按钮（SUNO 左下角那个垃圾桶的位置） */
.discard {
  flex: 0 0 auto;
  width: 42px;
  height: 44px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    color var(--md-duration-short-2) var(--md-ease-standard),
    opacity var(--md-duration-short-2) var(--md-ease-standard);
}
.discard:hover:not(:disabled) {
  background-color: rgb(var(--md-on-surface) / 0.06);
  color: rgb(var(--md-on-surface));
}
.discard:disabled {
  opacity: 0.38;
  cursor: default;
}
.discard:focus-visible,
.create:focus-visible,
.carried:focus-visible,
.tag:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.discard-icon {
  width: 17px;
  height: 17px;
}

/* 主按钮：MD3 filled，占满剩余宽度；生成中换成 tonal 的「停止」 */
.create {
  flex: 1 1 auto;
  min-width: 0;
  height: 44px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-family: inherit;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    box-shadow var(--md-duration-short-2) var(--md-ease-standard),
    opacity var(--md-duration-short-2) var(--md-ease-standard);
  box-shadow: var(--md-elevation-1);
}
.create:hover:not(:disabled) {
  box-shadow: var(--md-elevation-2);
}
.create:disabled {
  opacity: 0.38;
  cursor: default;
  box-shadow: none;
}
.create.stop {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
  box-shadow: none;
}
.create-icon {
  width: 17px;
  height: 17px;
}
.create-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.status {
  margin: 0;
  font-size: 10.5px;
  line-height: 1.4;
  color: rgb(var(--md-on-surface-muted));
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.status.error {
  color: rgb(var(--md-error));
  white-space: normal;
}

@media (prefers-reduced-motion: reduce) {
  .create,
  .discard,
  .carried {
    transition: none;
  }
}
</style>
