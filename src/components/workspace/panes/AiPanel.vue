<script setup lang="ts">
/**
 * AI 面板：与「插件」面板同一种风格（一个可滚动容器 + 一组卡片条目，不套多层容器）。
 *
 * 四条一级条目：
 * - **MCP 服务器**：开关；开启后同一容器里多出一行接入地址 + 「复制」。
 * - **音频转 MIDI** / **人声降噪** / **乐器底噪降噪**：各自**一个**「开始」按钮；
 *   处理中该条**下边缘变成进度条**、按钮变成「停止」，可随时中止。
 *
 * 降噪刻意分成两条：LocalVQE 是 16 kHz 的**语音**模型，直接拿它处理音乐会把乐器当噪声压掉
 * （而且 8 kHz 以上直接没了），所以另一条走内置 DSP（全带宽、只在接近底噪处衰减）。
 * 结果都放到源轨道**下面**的新轨道上，方便和原轨 A/B 对比。
 *
 * 本地能力优先走 ONNX 模型（Basic Pitch 转 MIDI、LocalVQE 降噪，见 `crates/local-ai/src/models/`），
 * 权重缺失时自动退回内置 DSP；作用于**当前选中轨道**的音频片段。条目上会标明实际走哪条路径。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { ClipboardDocumentIcon } from "@heroicons/vue/24/outline";
import { aiCancel, aiModels, aiStart, aiStatus, mcpStatus, onAiEvent, setMcpConfig } from "@/api/tauri";
import type { AiModels, AiStatus, McpStatus } from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";

const KIND_MIDI = "to_midi";
/** 人声降噪 / 去混响：走 LocalVQE 模型。 */
const KIND_VOICE = "enhance";
/** 乐器 / 素材底噪降噪：始终走内置 DSP（全带宽）。 */
const KIND_MUSIC = "enhance_dsp";

const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
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
const models = ref<AiModels>({ basic_pitch: false, localvqe: false });
let unlisten: (() => void) | null = null;

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
  return false;
}
const progressPct = computed(() => `${Math.round(ai.value.progress * 100)}%`);

/** 条目下的说明：先说这条能力**适合什么素材**，跑不了时说明缺什么。
 *
 * **不**把上一次任务的结果写进来——几条能力共用同一份说明，会导致「转 MIDI 的结果
 * 出现在别的条目下面」。任务结果/错误统一走通知（见下面的 watcher）。 */
function noteFor(kind: string): string {
  if (!track.value) return i18n.t("ai.noTrack");
  if (!hasAudio.value) return i18n.t("ai.needsAudio");
  if (kind === KIND_MIDI) return i18n.t("ai.midiNote");
  if (kind === KIND_VOICE) return i18n.t("ai.voiceNote");
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
});
onUnmounted(() => unlisten?.());
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
    </ul>
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
