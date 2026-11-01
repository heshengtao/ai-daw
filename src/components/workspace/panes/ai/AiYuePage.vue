<script setup lang="ts">
/**
 * **AI 写歌（YuE2）**子页（侧栏内整页，从列表的卡片进来，左上角返回）。
 *
 * 排版统一走 `main.css` 里的 MD3 表单原语（`m3-card` / `m3-row` / `m3-field` /
 * `m3-input` / `m3-btn`）和共享控件（`Md3Select` / `Md3Segmented` / `Md3Switch` /
 * `NumberField`），不再在页面里另造一套输入框 / 按钮。
 *
 * 两段状态：引擎（分钟级加载，`engine://yue-engine`）与任务（提交后轮询，`engine://yue-job`）。
 * 安装包**不带** audio.cpp 引擎：一份都没装时先弹安装向导，选后端后跳下载管理器。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ArrowLeftIcon } from "@heroicons/vue/24/outline";
import EngineSetupDialog from "@/components/common/EngineSetupDialog.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import Md3Switch from "@/components/common/Md3Switch.vue";
import NumberField from "@/components/common/NumberField.vue";
import type { SegmentedOption } from "@/components/common/segmented";
import type { RuntimeStatus } from "@/api/types";
import { useModelChoice } from "@/composables/useModelChoice";
import { useI18n } from "@/i18n";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useRuntimesStore } from "@/stores/runtimes";
import { useSelectionStore } from "@/stores/selection";
import { useYueStore } from "@/stores/yue";
import { humanSize } from "@/utils/models";

const emit = defineEmits<{ (e: "back"): void }>();

const yue = useYueStore();
const models = useModelsStore();
const runtimes = useRuntimesStore();
const layout = useLayoutStore();
const selection = useSelectionStore();
const i18n = useI18n();

const style = ref("");
const lyrics = ref("");
const abc = ref("");
const cot = ref<"full" | "melody" | "off">("full");
const duration = ref(120);
const steps = ref(32);
const seed = ref(-1);
const cfg = ref(-1);
/** 步数 / CFG / 种子是进阶参数，默认收起，普通用户只看见风格、歌词和时长。 */
const advancedOpen = ref(false);
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

/** 一份引擎都没有 → 安装向导必须出现（安装包默认不带引擎）。 */
const needsEngineSetup = computed(
  () =>
    !runtimes.loading &&
    runtimes.busyId === null &&
    enginePacks.value.length > 0 &&
    !engineInstalled.value,
);
watch(
  needsEngineSetup,
  (needs) => {
    if (needs) setupOpen.value = true;
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

// ---- 创作参数 ----
const cotValue = computed({
  get: () => cot.value,
  set: (value: string) => {
    cot.value = value as "full" | "melody" | "off";
  },
});

const cotOptions = computed<SegmentedOption[]>(() => [
  { value: "full", label: i18n.t("yue.cotFullShort"), title: i18n.t("yue.cotFull") },
  { value: "melody", label: i18n.t("yue.cotMelodyShort"), title: i18n.t("yue.cotMelody") },
  { value: "off", label: i18n.t("yue.cotOffShort"), title: i18n.t("yue.cotOff") },
]);

const modelHint = computed(() => {
  const m = chosenModel.value;
  if (!m) return "";
  return `${m.license} · ${humanSize(m.approx_bytes)}`;
});

const canCompose = computed(
  () =>
    yue.ready &&
    !yue.busy &&
    chosenModel.value?.installed === true &&
    (style.value.trim().length > 0 || lyrics.value.trim().length > 0),
);

const elapsedText = computed(() => i18n.t("yue.elapsed", { seconds: Math.round(yue.job.elapsed) }));

/** 主按钮：按当前缺什么决定动作，避免一个永远点不动的「生成」。 */
type PrimaryKind = "engine" | "model" | "start" | "compose" | "stop";
const primaryKind = computed<PrimaryKind>(() => {
  if (yue.busy) return "stop";
  if (!engineInstalled.value) return "engine";
  if (chosenModel.value?.installed !== true) return "model";
  if (!yue.ready) return "start";
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
    case "start":
      return yue.starting ? i18n.t("yue.engineStarting") : i18n.t("yue.startEngine");
    default:
      return i18n.t("yue.generate");
  }
});
const primaryDisabled = computed(() => {
  if (primaryKind.value === "engine") return enginePacks.value.length === 0 || engineDownloading.value;
  if (primaryKind.value === "start") return yue.starting;
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
    case "start":
      return i18n.t("yue.stepStart");
    case "compose":
      return canCompose.value ? "" : i18n.t("ai.yueNeedInput");
    default:
      return "";
  }
});

function openModelManager(): void {
  models.requestFocus();
  layout.openTab("manager");
}

async function startEngine(): Promise<void> {
  if (yue.starting) return;
  if (!engineInstalled.value) {
    setupOpen.value = true;
    return;
  }
  if (chosenModel.value?.installed !== true) {
    openModelManager();
    return;
  }
  await yue.start(modelId.value || null);
}

async function compose(): Promise<void> {
  if (!canCompose.value) return;
  await yue.compose({
    style: style.value.trim(),
    lyrics: lyrics.value,
    abc: abc.value,
    duration: duration.value,
    steps: steps.value,
    seed: seed.value,
    cot: cot.value,
    cfg_scale: cfg.value < 0 ? undefined : cfg.value,
    model: modelId.value || null,
    after_track: selection.trackId ?? null,
  });
}

async function runPrimary(): Promise<void> {
  switch (primaryKind.value) {
    case "stop":
      await yue.cancel();
      break;
    case "engine":
      setupOpen.value = true;
      break;
    case "model":
      openModelManager();
      break;
    case "start":
      await startEngine();
      break;
    case "compose":
      await compose();
      break;
  }
}

/** 强制向导里的「返回」：直接退回 AI 列表，不把人困在一张不能用的页面上。 */
function onSetupCancel(): void {
  if (!engineInstalled.value) emit("back");
}

let stopYue: (() => void) | null = null;

onMounted(async () => {
  stopYue = yue.watch();
  void models.load();
  // 右侧面板是三个 AI 子页共用的滚动容器：从列表进来时先回到顶部。
  pageEl.value?.closest(".pane")?.scrollTo({ top: 0 });
  // 引擎清单要等：一份都没有时要立刻弹安装向导。
  await Promise.all([yue.load(), runtimes.load()]);
});

onUnmounted(() => {
  stopYue?.();
});

// 生成完成 / 失败时弹一次通知。
watch(
  () => yue.job.state,
  (state, before) => {
    if (before !== "running") return;
    if ((state === "done" || state === "failed") && yue.job.message) {
      layout.notify(yue.job.message);
    }
  },
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
          <span class="m3-card-sub">{{ i18n.t("yue.composeHint") }}</span>
        </div>
      </div>

      <!-- 作曲方式：和时长一样，标签在左、控件靠右 -->
      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.mode") }}</span>
        </div>
        <div class="m3-row-control">
          <Md3Segmented
            v-model="cotValue"
            :options="cotOptions"
            :aria-label="i18n.t('yue.mode')"
          />
        </div>
      </div>
      <p class="m3-hint mode-hint">{{ i18n.t("yue.modeHint") }}</p>

      <div class="m3-row">
        <div class="m3-row-main">
          <span class="m3-row-label">{{ i18n.t("yue.duration") }}</span>
        </div>
        <div class="m3-row-control">
          <NumberField
            v-model="duration"
            :sr-label="i18n.t('yue.duration')"
            :min="15"
            :max="360"
            :step="5"
            input-width="64px"
          />
          <span class="m3-row-value">{{ i18n.t("yue.seconds") }}</span>
        </div>
      </div>

      <div class="m3-fields single">
        <label class="m3-field">
          <span class="m3-field-label">{{ i18n.t("yue.style") }}</span>
          <input
            v-model="style"
            class="m3-input"
            type="text"
            spellcheck="false"
            :placeholder="i18n.t('yue.styleHint')"
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
        <label class="m3-field">
          <span class="m3-field-label">{{ i18n.t("yue.abc") }}</span>
          <textarea
            v-model="abc"
            class="m3-textarea mono"
            spellcheck="false"
            :placeholder="i18n.t('yue.abcHint')"
          />
          <span class="m3-hint">{{ i18n.t("yue.abcExample") }}</span>
        </label>
      </div>

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
        <div class="m3-row">
          <div class="m3-row-main">
            <span class="m3-row-label">{{ i18n.t("yue.steps") }}</span>
            <span class="m3-row-sub">{{ i18n.t("yue.stepsHint") }}</span>
          </div>
          <div class="m3-row-control">
            <NumberField
              v-model="steps"
              :sr-label="i18n.t('yue.steps')"
              :min="1"
              :max="128"
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
      </template>

      <div v-if="yue.busy" class="progress-indeterminate" aria-hidden="true" />

      <!-- 和其他 AI 子页一样：主动作在卡片底部，不做悬浮/固定条 -->
      <div class="m3-card-actions">
        <span v-if="yue.busy" class="m3-hint">{{ elapsedText }}</span>
        <span v-else-if="yue.job.state === 'done'" class="m3-hint">{{ i18n.t("yue.added") }}</span>
        <span v-else-if="actionHint" class="m3-hint">{{ actionHint }}</span>
        <span class="spacer" />
        <button class="m3-btn filled" type="button" :disabled="primaryDisabled" @click="runPrimary">
          {{ primaryLabel }}
        </button>
      </div>
    </section>

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
</style>
