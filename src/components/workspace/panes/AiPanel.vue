<script setup lang="ts">
/**
 * AI 面板：与「插件」面板同一种风格（一个可滚动容器 + 一组卡片条目，不套多层容器）。
 *
 * 五条一级条目：
 * - **MCP 服务器**：开关；开启后同一容器里多出一行接入地址 + 「复制」。
 * - **音频转 MIDI** / **人声降噪** / **乐器底噪降噪**：各自**一个**「开始」按钮；
 *   处理中该条**下边缘变成进度条**、按钮变成「停止」，可随时中止。
 * - **母带处理**：作用于**整个工程的离线混音**（不是某条轨道），三个参数（目标响度 /
 *   真峰值上限 / 立体声宽度）；同样是**一个**按钮 + 进度条。
 *
 * 降噪刻意分成两条：LocalVQE 是 16 kHz 的**语音**模型，直接拿它处理音乐会把乐器当噪声压掉
 * （而且 8 kHz 以上直接没了），所以另一条走内置 DSP（全带宽、只在接近底噪处衰减）。
 * 结果都放到源轨道**下面**的新轨道上，方便和原轨 A/B 对比。
 *
 * 母带与上面几条不同：**纯 DSP、不依赖任何模型**（`crates/mastering`），所以没有「模型 /
 * 内置 DSP」的路径之分；结果是一条名为「母带」的新轨道。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ClipboardDocumentIcon } from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Md3Select from "@/components/common/Md3Select.vue";
import ConfirmDialog from "@/components/common/ConfirmDialog.vue";
import { aiCancel, aiModels, aiStart, aiStatus, mcpStatus, onAiEvent, setMcpConfig } from "@/api/tauri";
import type { AiModels, AiStatus, McpStatus, ModelStatus } from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useI18n } from "@/i18n";

const KIND_MIDI = "to_midi";
/** 人声降噪 / 去混响：走 LocalVQE 模型。 */
const KIND_VOICE = "enhance";
/** 乐器 / 素材底噪降噪：始终走内置 DSP（全带宽）。 */
const KIND_MUSIC = "enhance_dsp";
/** 母带处理：作用于**整个工程的离线混音**，纯 DSP，不需要模型。 */
const KIND_MASTER = "master";
/** 音源分离：把所选轨道的音频拆成鼓/贝斯/其他/人声四条分轨（HTDemucs 模型，无 DSP 兜底）。 */
const KIND_SEPARATE = "separate";
/** 音源分离模型在清单里的 id（不带权重时在本卡片里直接下载）。 */
const SEPARATE_MODEL_ID = "htdemucs-ort-v1";

const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const modelsStore = useModelsStore();
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
  try {
    await navigator.clipboard.writeText(url);
    layout.notify(i18n.t("ai.copied"));
  } catch {
    layout.notify(i18n.t("ai.copyFailed"));
  }
}

// ---- 本地 AI ----
const ai = ref<AiStatus>({ running: false, kind: null, progress: 0, message: null });
/** 两个 ONNX 权重是否就位（决定条目上的「模型 / 内置 DSP」标签）。 */
const models = ref<AiModels>({ basic_pitch: false, localvqe: false, separate: false });
let unlisten: (() => void) | null = null;
/** 模型下载进度订阅的取消函数。 */
let stopModel: (() => void) | null = null;

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

/** 这个功能会走哪个后端：权重就位就是真模型，否则退回内置 DSP。
 *
 * 乐器降噪**始终**是内置 DSP——语音模型会把乐器当噪声压掉，所以那条路刻意不走模型。
 */
function backendLabel(kind: string): string {
  return hasModel(kind) ? i18n.t("ai.backendModel") : i18n.t("ai.backendDsp");
}
function hasModel(kind: string): boolean {
  if (kind === KIND_MIDI) return models.value.basic_pitch;
  if (kind === KIND_VOICE) return models.value.localvqe;
  if (kind === KIND_SEPARATE) return models.value.separate;
  return false;
}
const progressPct = computed(() => `${Math.round(ai.value.progress * 100)}%`);

// ---- 音源分离：可选模型（4 分轨 / 6 分轨）；缺权重时先问、再下载并跳到「管理器」----
/** 「人类可读体积」。 */
function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

/** 清单里的分离模型（按清单顺序）。 */
const separateModels = computed(() => modelsStore.catalog.filter((m) => m.feature === "separate"));
/** 当前选中的分离模型 id。 */
const sepModelId = ref(SEPARATE_MODEL_ID);
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
    label: m.name,
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
    ? i18n.t("dialog.downloadModelMessage", { name: m.name, size: humanSize(m.approx_bytes) })
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

// 分离模型就位情况变化后，刷新一次「模型 / 内置 DSP」标签。
watch(
  () => separateModels.value.map((m) => `${m.id}:${m.installed}`).join(","),
  () => {
    void aiModels()
      .then((m) => (models.value = m))
      .catch(() => {});
  },
);

// ---- 母带处理（纯 DSP，作用于整个工程）----
/** 三个参数；默认值与后端 `crates/mastering` 的默认值一致。 */
const masterTarget = ref(-14);
const masterCeiling = ref(-1);
const masterWidth = ref(1);
/** 母带有东西可处理：工程里至少有一条轨道（且没有别的任务在跑）。 */
const canMaster = computed(() => (project.project?.tracks.length ?? 0) > 0 && !ai.value.running);

async function runMaster(): Promise<void> {
  if (!canMaster.value) return;
  try {
    await aiStart(KIND_MASTER, 0, {
      target_lufs: masterTarget.value,
      ceiling_dbtp: masterCeiling.value,
      stereo_width: masterWidth.value,
    });
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 条目下的说明：先说这条能力**适合什么素材**，跑不了时说明缺什么。
 *
 * **不**把上一次任务的结果写进来——几条能力共用同一份说明，会导致「转 MIDI 的结果
 * 出现在别的条目下面」。任务结果/错误统一走通知（见下面的 watcher）。 */
function noteFor(kind: string): string {
  // 母带作用于整个工程，与「选中轨道有没有音频」无关。
  if (kind === KIND_MASTER) {
    return canMaster.value || busy(KIND_MASTER) ? i18n.t("ai.masterNote") : i18n.t("ai.noTrack");
  }
  if (!track.value) return i18n.t("ai.noTrack");
  if (!hasAudio.value) return i18n.t("ai.needsAudio");
  if (kind === KIND_MIDI) return i18n.t("ai.midiNote");
  if (kind === KIND_VOICE) return i18n.t("ai.voiceNote");
  if (kind === KIND_SEPARATE) {
    return separateModelReady.value ? i18n.t("ai.separateNote") : i18n.t("ai.separateNeedsModel");
  }
  return i18n.t("ai.musicNote");
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
  // 分离卡片的「未安装 → 下载」需要知道模型清单与下载进度。
  void modelsStore.load();
  stopModel = modelsStore.watchProgress();
});
onUnmounted(() => {
  unlisten?.();
  stopModel?.();
});
</script>

<template>
  <div class="pane">
    <ul class="list">
      <!-- MCP 服务器：接入地址并入同一个容器 -->
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
          <button v-if="!busy(KIND_MIDI)" class="iface-btn" :disabled="!canRun" @click="run(KIND_MIDI)">
            {{ i18n.t("ai.run") }}
          </button>
          <button v-else class="iface-btn danger" @click="stop">{{ i18n.t("ai.stop") }}</button>
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
            class="iface-btn"
            :disabled="!canRun"
            @click="run(KIND_VOICE)"
          >
            {{ i18n.t("ai.runEnhance") }}
          </button>
          <button v-else class="iface-btn danger" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <div v-if="busy(KIND_VOICE)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </li>

      <!-- 乐器 / 素材底噪降噪：内置 DSP（全带宽，对音乐温和） -->
      <li class="item" :class="{ busy: busy(KIND_MUSIC) }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.enhanceMusic") }}
              <span class="badge" :class="{ on: hasModel(KIND_MUSIC) }">
                {{ backendLabel(KIND_MUSIC) }}
              </span>
            </span>
            <span class="item-note">{{ busy(KIND_MUSIC) ? i18n.t("ai.busy") : noteFor(KIND_MUSIC) }}</span>
          </div>
          <button
            v-if="!busy(KIND_MUSIC)"
            class="iface-btn"
            :disabled="!canRun"
            @click="run(KIND_MUSIC)"
          >
            {{ i18n.t("ai.runEnhance") }}
          </button>
          <button v-else class="iface-btn danger" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <div v-if="busy(KIND_MUSIC)" class="progress">
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
                {{ i18n.t(separateModelReady ? "ai.backendModel" : "ai.backendDsp") }}
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
          <button v-if="sepDownloading" class="iface-btn danger" @click="cancelModelDownload">
            {{ i18n.t("ai.stop") }}
          </button>
          <!-- 正在分离：可停止 -->
          <button v-else-if="busy(KIND_SEPARATE)" class="iface-btn danger" @click="stop">
            {{ i18n.t("ai.stop") }}
          </button>
          <!-- 开始：权重缺时先弹「是否下载」并跳转到「管理器」 -->
          <button v-else class="iface-btn" :disabled="!canRun" @click="runSeparation">
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

      <!-- 母带处理：整个工程的离线混音（纯 DSP，无模型） -->
      <li class="item" :class="{ busy: busy(KIND_MASTER) }">
        <div class="item-row">
          <div class="item-main">
            <span class="item-name">
              {{ i18n.t("ai.master") }}
              <span class="badge">{{ i18n.t("ai.backendDsp") }}</span>
            </span>
            <span class="item-note">
              {{ busy(KIND_MASTER) ? i18n.t("ai.busy") : noteFor(KIND_MASTER) }}
            </span>
          </div>
          <button
            v-if="!busy(KIND_MASTER)"
            class="iface-btn"
            :disabled="!canMaster"
            @click="runMaster"
          >
            {{ i18n.t("ai.runMaster") }}
          </button>
          <button v-else class="iface-btn danger" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>

        <!-- 三个参数：名称（单位）在左，步进器靠右；步进器里只有数值与 −/+。
             `sr-label` 让读屏仍能念出名称（界面上不重复渲染）。 -->
        <div class="params">
          <div class="param-field">
            <span class="param-name">{{ i18n.t("ai.masterTarget") }}</span>
            <NumberField
              v-model="masterTarget"
              :sr-label="i18n.t('ai.masterTarget')"
              :min="-40"
              :max="0"
              :step="0.5"
              :decimals="1"
              :disabled="ai.running"
            />
          </div>
          <div class="param-field">
            <span class="param-name">{{ i18n.t("ai.masterCeiling") }}</span>
            <NumberField
              v-model="masterCeiling"
              :sr-label="i18n.t('ai.masterCeiling')"
              :min="-12"
              :max="0"
              :step="0.1"
              :decimals="1"
              :disabled="ai.running"
            />
          </div>
          <div class="param-field">
            <span class="param-name">{{ i18n.t("ai.masterWidth") }}</span>
            <NumberField
              v-model="masterWidth"
              :sr-label="i18n.t('ai.masterWidth')"
              :min="0"
              :max="2"
              :step="0.1"
              :decimals="1"
              :disabled="ai.running"
            />
          </div>
        </div>

        <div v-if="busy(KIND_MASTER)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
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
/* 母带参数：一行一个「名称（单位）在左 + 步进器靠右」。 */
.params {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.param-field {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
}
.param-name {
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
.iface-btn.danger {
  color: rgb(var(--md-error));
  border-color: rgb(var(--md-error) / 0.5);
}
.iface-btn.danger:hover:not(:disabled) {
  background-color: rgb(var(--md-error) / 0.12);
}
.iface-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
</style>
