<script setup lang="ts">
/**
 * 调音台面板：传统数字音频处理。
 *
 * 五块：
 * - **通道**：选中轨道的电平表 / 增益 / 声像 / 静音 / 独奏；
 * - **插入**：效果链的旁通与侧链来源（参数编辑在检查器 / 效果器窗口）；
 * - **发送**：推子前 / 推子后、增益、哑掉；
 * - **主输出**：表、总增益、插入链（限幅器挂这里），以及**母带处理**（离线，纯 DSP）；
 * - **处理**：逐轨的内置 DSP（底噪降噪）。
 *
 * 与整页混音台的分工：整页负责「一眼看全局」（推子 + 表 + 插入/发送摘要），这里负责
 * 「就地把这一条调好」。两边共用同一份数据与同一套组件。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { XMarkIcon } from "@heroicons/vue/24/outline";
import NumberField from "@/components/common/NumberField.vue";
import Md3Select, { type SelectOption } from "@/components/common/Md3Select.vue";
import ChannelMeter from "@/components/common/ChannelMeter.vue";
import InsertSlot from "@/components/common/InsertSlot.vue";
import { aiCancel, aiStart, aiStatus, onAiEvent } from "@/api/tauri";
import type { AiStatus, Send } from "@/api/types";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useLayoutStore } from "@/stores/layout";
import { usePluginsStore } from "@/stores/plugins";
import { useMetersStore } from "@/stores/meters";
import { useI18n } from "@/i18n";

/** 乐器 / 素材底噪降噪：内置 DSP（全带宽、只在接近底噪处衰减）。 */
const KIND_MUSIC = "enhance_dsp";
/** 母带处理：作用于**整个工程的离线混音**，纯 DSP，不需要模型。 */
const KIND_MASTER = "master";

const MIN_DB = -60;
const MAX_DB = 12;

const project = useProjectStore();
const selection = useSelectionStore();
const layout = useLayoutStore();
const plugins = usePluginsStore();
const meters = useMetersStore();
const i18n = useI18n();

const track = computed(() => selection.track);

// ---- 通道 ----

const shownGain = computed(() => track.value?.mixer.gain_db ?? 0);
const shownPan = computed(() => track.value?.mixer.pan ?? 0);

/** 本轨的表。 */
const channelLevel = computed(() =>
  track.value ? meters.track(track.value.id) : meters.track(-1),
);

async function commitGain(value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setGain(t.id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setGain", { error: String(err) }));
  }
}

async function commitPan(value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setPan(t.id, value);
  } catch (err) {
    layout.notify(i18n.t("error.setPan", { error: String(err) }));
  }
}

async function toggleMute(): Promise<void> {
  const t = track.value;
  if (!t) return;
  await project.setMute(t.id, !t.mixer.mute);
}

async function toggleSolo(): Promise<void> {
  const t = track.value;
  if (!t) return;
  await project.setSolo(t.id, !t.mixer.solo);
}

// ---- 插入 ----

/** 本轨的插入链。 */
const inserts = computed(() => track.value?.effects ?? []);

/** 可选的效果器（内置 + 已扫描到的外部插件）。 */
const fxOptions = computed<SelectOption[]>(() =>
  plugins.effects.map((e) => ({ id: e.id, label: e.name, hint: e.format?.toUpperCase() })),
);

/** 可作侧链来源的轨道（除自身外）。 */
const sidechainOptions = computed<SelectOption[]>(() => [
  { id: "", label: i18n.t("mixer.sidechainNone") },
  ...(project.project?.tracks ?? [])
    .filter((t) => t.id !== track.value?.id)
    .map((t) => ({ id: String(t.id), label: t.name })),
]);

function insertName(index: number): string {
  const fx = inserts.value[index];
  return fx ? plugins.name(fx.plugin_id) : "";
}

async function toggleInsert(index: number): Promise<void> {
  const t = track.value;
  const fx = inserts.value[index];
  if (!t || !fx) return;
  try {
    await project.setEffectEnabled(t.id, index, !(fx.enabled ?? true));
  } catch (err) {
    layout.notify(String(err));
  }
}

async function addInsert(pluginId: string): Promise<void> {
  const t = track.value;
  if (!t || !pluginId) return;
  try {
    await project.addEffect(t.id, pluginId);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function removeInsert(index: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.removeEffect(t.id, index);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function setSidechain(index: number, value: string): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setEffectSidechain(t.id, index, value === "" ? null : Number(value));
  } catch (err) {
    layout.notify(String(err));
  }
}

// ---- 发送（aux send → 总线） ----

/** 本轨的发送列表。 */
const sends = computed<Send[]>(() => track.value?.sends ?? []);

/** 可选的发送目标：除自己以外的所有轨道（含总线）。 */
const sendTargets = computed<SelectOption[]>(() =>
  (project.project?.tracks ?? [])
    .filter((t) => t.id !== track.value?.id)
    .map((t) => ({ id: String(t.id), label: t.name })),
);

/** 发送目标的显示名。 */
function targetName(id: number): string {
  return (project.project?.tracks ?? []).find((t) => t.id === id)?.name ?? `#${id}`;
}

/** 新增一路发送到 `targetId`（已在列表里则忽略）。 */
async function addSend(targetId: string): Promise<void> {
  const t = track.value;
  if (!t) return;
  const target = Number(targetId);
  if (!Number.isFinite(target) || sends.value.some((s) => s.target === target)) return;
  try {
    await project.setSends(t.id, [...sends.value, { target, gain_db: 0 }]);
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 提交一路发送的增益（一次编辑 = 一条撤销记录）。 */
async function commitSendGain(target: number, value: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setSends(
      t.id,
      sends.value.map((s) => (s.target === target ? { ...s, gain_db: value } : s)),
    );
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 切换一路发送的取点（推子前 / 推子后）。 */
async function setSendMode(target: number, mode: "pre" | "post"): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setSends(
      t.id,
      sends.value.map((s) => (s.target === target ? { ...s, mode } : s)),
    );
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 哑掉 / 打开一路发送。 */
async function toggleSendMute(target: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setSends(
      t.id,
      sends.value.map((s) => (s.target === target ? { ...s, mute: !s.mute } : s)),
    );
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

/** 移除一路发送。 */
async function removeSend(target: number): Promise<void> {
  const t = track.value;
  if (!t) return;
  try {
    await project.setSends(t.id, sends.value.filter((s) => s.target !== target));
  } catch (err) {
    layout.notify(i18n.t("error.setSends", { error: String(err) }));
  }
}

// ---- 主输出 ----

const shownMasterGain = computed(() => project.project?.master.gain_db ?? 0);

async function commitMasterGain(value: number): Promise<void> {
  try {
    await project.setMasterGain(value);
  } catch (err) {
    layout.notify(i18n.t("error.setGain", { error: String(err) }));
  }
}

/** 主总线插入链。 */
const masterInserts = computed(() => project.project?.master_effects ?? []);

async function addMasterInsert(pluginId: string): Promise<void> {
  if (!pluginId) return;
  try {
    await project.addMasterEffect(pluginId);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function removeMasterInsert(index: number): Promise<void> {
  try {
    await project.removeMasterEffect(index);
  } catch (err) {
    layout.notify(String(err));
  }
}

async function toggleMasterInsert(index: number): Promise<void> {
  const fx = masterInserts.value[index];
  if (!fx) return;
  try {
    await project.setMasterEffectEnabled(index, !(fx.enabled ?? true));
  } catch (err) {
    layout.notify(String(err));
  }
}

/** 展开哪个主总线效果器的参数（主总线没有「检查器」可去，就在地改）。 */
const expandedMaster = ref<number | null>(null);

function toggleMasterParams(index: number): void {
  expandedMaster.value = expandedMaster.value === index ? null : index;
}

/** 某个主总线效果器的参数表。 */
function masterParams(index: number) {
  const fx = masterInserts.value[index];
  if (!fx) return [];
  return plugins.paramsOf(fx.plugin_id).map((spec) => ({
    spec,
    value: fx.params[spec.id] ?? spec.default,
  }));
}

async function commitMasterParam(index: number, param: string, value: number): Promise<void> {
  try {
    await project.setMasterEffectParam(index, param, value);
  } catch (err) {
    layout.notify(String(err));
  }
}

// ---- 内置 DSP 任务 ----

const ai = ref<AiStatus>({ running: false, kind: null, progress: 0, message: null });
let unlisten: (() => void) | null = null;

/** 某个功能是否正在处理。 */
function busy(kind: string): boolean {
  return ai.value.running && ai.value.kind === kind;
}

const progressPct = computed(() => `${Math.round(ai.value.progress * 100)}%`);

/** 当前轨道有没有音频片段（降噪作用于它）。 */
const hasAudio = computed(() => (track.value?.clips ?? []).some((c) => c.audio));

/** 降噪可跑：选中轨道且它带音频，且没有别的任务在跑。 */
const canEnhance = computed(() => Boolean(track.value) && hasAudio.value && !ai.value.running);

/** 母带有东西可处理：工程里至少有一条轨道（且没有别的任务在跑）。 */
const canMaster = computed(() => (project.project?.tracks.length ?? 0) > 0 && !ai.value.running);

/** 母带三个参数；默认值与后端 `crates/mastering` 的默认值一致。 */
const masterTarget = ref(-14);
const masterCeiling = ref(-1);
const masterWidth = ref(1);

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

async function runEnhance(): Promise<void> {
  const t = track.value;
  if (!t || !canEnhance.value) return;
  try {
    await aiStart(KIND_MUSIC, t.id);
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

/** 任务跑完时把摘要 / 错误作为通知弹一次。 */
watch(
  () => ai.value.running,
  (running, wasRunning) => {
    if (wasRunning && !running && ai.value.message) layout.notify(ai.value.message);
  },
);

onMounted(async () => {
  ai.value = await aiStatus().catch(() => ai.value);
  unlisten = await onAiEvent((s) => (ai.value = s));
  meters.start();
});
onUnmounted(() => {
  unlisten?.();
  meters.stop();
});
</script>

<template>
  <div class="pane">
    <section class="group">
      <div class="group-title">
        {{ i18n.t("mixer.channel") }}<template v-if="track"> · {{ track.name }}</template>
      </div>
      <div class="card">
        <p v-if="!track" class="hint">{{ i18n.t("mixer.selectTrackHint") }}</p>
        <template v-else>
          <div class="row meter-row">
            <span class="row-label">{{ i18n.t("mixer.level") }}</span>
            <div class="meter-wrap">
              <ChannelMeter
                :value="channelLevel"
                :label="track.name"
                @reset="meters.clearClip()"
              />
            </div>
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("mixer.gain") }}</span>
            <NumberField
              :model-value="shownGain"
              :sr-label="i18n.t('mixer.gain')"
              :min="MIN_DB"
              :max="MAX_DB"
              :step="0.5"
              :decimals="1"
              @update:model-value="commitGain"
            />
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("mixer.pan") }}</span>
            <NumberField
              :model-value="shownPan"
              :sr-label="i18n.t('mixer.pan')"
              :min="-1"
              :max="1"
              :step="0.02"
              :decimals="2"
              @update:model-value="commitPan"
            />
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("mixer.muteSolo") }}</span>
            <div class="strip-buttons">
              <button
                class="mini"
                :class="{ on: track.mixer.mute }"
                :aria-pressed="track.mixer.mute"
                :aria-label="i18n.t('mixer.mute')"
                @click="toggleMute"
              >
                M
              </button>
              <button
                class="mini solo"
                :class="{ on: track.mixer.solo }"
                :aria-pressed="track.mixer.solo"
                :aria-label="i18n.t('mixer.solo')"
                @click="toggleSolo"
              >
                S
              </button>
            </div>
          </div>
        </template>
      </div>
    </section>

    <section v-if="track" class="group">
      <div class="group-title">{{ i18n.t("mixer.inserts") }}</div>
      <div class="card">
        <p v-if="inserts.length === 0" class="hint">{{ i18n.t("mixer.insertsEmpty") }}</p>
        <div v-for="(fx, i) in inserts" :key="i" class="slot-row">
          <InsertSlot
            class="slot-line"
            :name="insertName(i)"
            :enabled="fx.enabled ?? true"
            :sidechain="fx.sidechain ? targetName(fx.sidechain.track) : null"
            @toggle="toggleInsert(i)"
            @open="layout.openTab('inspector')"
          />
          <Md3Select
            v-if="plugins.info(fx.plugin_id)?.sidechain"
            class="slot-sc"
            :model-value="fx.sidechain ? String(fx.sidechain.track) : ''"
            :options="sidechainOptions"
            :label="i18n.t('mixer.sidechain')"
            :placeholder="i18n.t('mixer.sidechainNone')"
            :disabled="false"
            @update:model-value="(v: string) => setSidechain(i, v)"
          />
          <button
            class="mini danger"
            :aria-label="i18n.t('inspector.fxRemove')"
            @click="removeInsert(i)"
          >
            <XMarkIcon class="mini-icon" />
          </button>
        </div>
        <div class="row">
          <span class="row-label">{{ i18n.t("inspector.add") }}</span>
          <Md3Select
            :model-value="null"
            :options="fxOptions"
            :label="i18n.t('inspector.addFx')"
            :placeholder="i18n.t('inspector.chooseFx')"
            @update:model-value="addInsert"
          />
        </div>
      </div>
    </section>

    <section class="group">
      <div class="group-title">{{ i18n.t("mixer.sends") }}</div>
      <div class="card">
        <p v-if="!track" class="hint">{{ i18n.t("mixer.selectTrackHint") }}</p>
        <template v-else>
          <p v-if="sends.length === 0" class="hint">{{ i18n.t("mixer.sendsEmpty") }}</p>
          <div v-for="send in sends" :key="send.target" class="send-block">
            <div class="row send-row">
              <span class="row-label send-name" :title="targetName(send.target)">
                {{ targetName(send.target) }}
              </span>
              <span class="send-ctl">
                <button
                  class="tap"
                  :class="{ on: (send.mode ?? 'post') === 'pre' }"
                  :title="i18n.t('mixer.sendTapHint')"
                  :aria-label="i18n.t('mixer.sendTap')"
                  @click="setSendMode(send.target, (send.mode ?? 'post') === 'pre' ? 'post' : 'pre')"
                >
                  {{ (send.mode ?? "post") === "pre" ? i18n.t("mixer.pre") : i18n.t("mixer.post") }}
                </button>
                <NumberField
                  :model-value="send.gain_db"
                  :sr-label="i18n.t('mixer.sendGain')"
                  :min="MIN_DB"
                  :max="MAX_DB"
                  :step="0.5"
                  :decimals="1"
                  @update:model-value="(v: number) => commitSendGain(send.target, v)"
                />
                <button
                  class="mini"
                  :class="{ on: send.mute }"
                  :aria-pressed="Boolean(send.mute)"
                  :aria-label="i18n.t('mixer.sendMute')"
                  @click="toggleSendMute(send.target)"
                >
                  M
                </button>
                <button
                  class="mini danger"
                  :aria-label="i18n.t('mixer.removeSend')"
                  @click="removeSend(send.target)"
                >
                  <XMarkIcon class="mini-icon" />
                </button>
              </span>
            </div>
          </div>
          <div class="row">
            <span class="row-label">{{ i18n.t("mixer.addSend") }}</span>
            <Md3Select
              :model-value="null"
              :options="sendTargets"
              :label="i18n.t('mixer.chooseSend')"
              :placeholder="i18n.t('mixer.chooseSend')"
              @update:model-value="addSend"
            />
          </div>
        </template>
      </div>
    </section>

    <section class="group">
      <div class="group-title">{{ i18n.t("mixer.master") }}</div>
      <div class="card" :class="{ busy: busy(KIND_MASTER) }">
        <div class="row meter-row">
          <span class="row-label">{{ i18n.t("mixer.level") }}</span>
          <div class="meter-wrap master">
            <ChannelMeter
              :value="meters.master"
              :label="i18n.t('mix.master')"
              @reset="meters.clearClip()"
            />
          </div>
        </div>
        <div class="row">
          <span class="row-label">{{ i18n.t("mixer.masterGain") }}</span>
          <NumberField
            :model-value="shownMasterGain"
            :sr-label="i18n.t('mixer.masterGain')"
            :min="MIN_DB"
            :max="MAX_DB"
            :step="0.5"
            :decimals="1"
            @update:model-value="commitMasterGain"
          />
        </div>
        <div class="row">
          <span class="row-label">{{ i18n.t("mixer.masterInserts") }}</span>
          <span class="row-value">{{ masterInserts.length }}</span>
        </div>
        <div v-for="(fx, i) in masterInserts" :key="i" class="master-slot">
          <div class="slot-row">
            <InsertSlot
              class="slot-line"
              :name="plugins.name(fx.plugin_id)"
              :enabled="fx.enabled ?? true"
              :sidechain="fx.sidechain ? targetName(fx.sidechain.track) : null"
              @toggle="toggleMasterInsert(i)"
              @open="toggleMasterParams(i)"
            />
            <button
              class="mini danger"
              :aria-label="i18n.t('inspector.fxRemove')"
              @click="removeMasterInsert(i)"
            >
              <XMarkIcon class="mini-icon" />
            </button>
          </div>
          <div v-if="expandedMaster === i" class="param-grid">
            <div v-for="p in masterParams(i)" :key="p.spec.id" class="param-row">
              <span class="param-label">{{ p.spec.name }}</span>
              <NumberField
                :model-value="p.value"
                :sr-label="p.spec.name"
                :min="p.spec.min"
                :max="p.spec.max"
                :step="p.spec.step"
                :decimals="p.spec.digits ?? 2"
                @update:model-value="(v: number) => commitMasterParam(i, p.spec.id, v)"
              />
            </div>
          </div>
        </div>
        <div class="row">
          <span class="row-label">{{ i18n.t("mixer.addInsert") }}</span>
          <Md3Select
            :model-value="null"
            :options="fxOptions"
            :label="i18n.t('mixer.addInsert')"
            :placeholder="i18n.t('inspector.chooseFx')"
            @update:model-value="addMasterInsert"
          />
        </div>
        <div class="row">
          <span class="row-label">{{ i18n.t("ai.master") }}</span>
          <button
            v-if="!busy(KIND_MASTER)"
            class="pill"
            :disabled="!canMaster"
            @click="runMaster"
          >
            {{ i18n.t("ai.runMaster") }}
          </button>
          <button v-else class="pill" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <p class="hint">{{ i18n.t("ai.masterNote") }}</p>
        <div class="row">
          <span class="row-label">{{ i18n.t("ai.masterTarget") }}</span>
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
        <div class="row">
          <span class="row-label">{{ i18n.t("ai.masterCeiling") }}</span>
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
        <div class="row">
          <span class="row-label">{{ i18n.t("ai.masterWidth") }}</span>
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
        <div v-if="busy(KIND_MASTER)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </div>
    </section>

    <section class="group">
      <div class="group-title">{{ i18n.t("mixer.processing") }}</div>
      <div class="card" :class="{ busy: busy(KIND_MUSIC) }">
        <div class="row">
          <span class="row-label">{{ i18n.t("ai.enhanceMusic") }}</span>
          <button v-if="!busy(KIND_MUSIC)" class="pill" :disabled="!canEnhance" @click="runEnhance">
            {{ i18n.t("ai.runEnhance") }}
          </button>
          <button v-else class="pill" @click="stop">{{ i18n.t("ai.stop") }}</button>
        </div>
        <p class="hint">
          {{ hasAudio ? i18n.t("ai.musicNote") : i18n.t("ai.needsAudio") }}
        </p>
        <div v-if="busy(KIND_MUSIC)" class="progress">
          <div class="progress-fill" :style="{ width: progressPct }" />
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.pane {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px;
  gap: 14px;
}
.group {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.group-title {
  font-size: 10.5px;
  letter-spacing: 0.8px;
  text-transform: uppercase;
  color: rgb(var(--md-on-surface-variant));
  padding-left: 4px;
}
.card {
  position: relative;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 6px;
  border-radius: 16px;
  background-color: var(--app-inset);
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  min-height: 34px;
  padding: 0 10px;
  border-radius: 12px;
  font-size: 12px;
}
.row-label {
  color: rgb(var(--md-on-surface-variant));
  flex: 0 0 auto;
  white-space: nowrap;
}
.row-value {
  color: rgb(var(--md-on-surface));
  font-variant-numeric: tabular-nums;
}
.meter-row {
  min-height: 56px;
}
.meter-wrap {
  height: 44px;
  display: flex;
  align-items: stretch;
}
.meter-wrap.master {
  height: 52px;
}
.slot-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 8px;
  min-height: 30px;
}
.master-slot {
  display: flex;
  flex-direction: column;
}
.param-grid {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 2px 8px 6px 26px;
}
.param-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  min-height: 28px;
  font-size: 11.5px;
}
.param-label {
  color: rgb(var(--md-on-surface-variant));
  white-space: nowrap;
}
.slot-line {
  flex: 1 1 auto;
  min-width: 0;
}
.slot-sc {
  flex: 0 0 108px;
}
.send-block {
  display: flex;
  flex-direction: column;
}
.send-row {
  gap: 8px;
}
.send-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.send-ctl {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 auto;
}
.tap {
  height: 22px;
  padding: 0 7px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 6px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10px;
  cursor: pointer;
}
.tap.on {
  color: rgb(var(--md-on-primary));
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
}
.strip-buttons {
  display: flex;
  gap: 5px;
}
.mini {
  width: 24px;
  height: 24px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 8px;
  padding: 0;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10.5px;
  font-weight: 700;
  cursor: pointer;
  transition: background-color 0.14s ease, color 0.14s ease, border-color 0.14s ease;
}
.mini:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
}
.mini-icon {
  width: 14px;
  height: 14px;
  display: block;
}
.mini.on {
  background-color: rgb(var(--md-error));
  border-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.mini.solo.on {
  background-color: rgb(var(--md-primary));
  border-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.mini.danger {
  color: rgb(var(--md-error));
  border-color: rgb(var(--md-error) / 0.5);
}
.mini.danger:hover {
  background-color: rgb(var(--md-error) / 0.12);
}
.pill {
  flex: 0 0 auto;
  height: 30px;
  padding: 0 14px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 11.5px;
  cursor: pointer;
  white-space: nowrap;
  transition: filter 0.14s ease;
}
.pill:hover:not(:disabled) {
  filter: brightness(1.08);
}
.pill:disabled {
  opacity: 0.45;
  cursor: default;
}
.pill:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 2px;
}
.hint {
  margin: 0 8px;
  font-size: 11px;
  line-height: 1.5;
  color: rgb(var(--md-on-surface-muted));
}
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
  background-color: rgb(var(--md-primary));
  transition: width 120ms linear;
}
</style>
