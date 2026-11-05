<script setup lang="ts">
/**
 * 底部条：**两翼 + 正中播放键**。
 *
 * 结构上只有三个 flex 直接子项：左翼、播放键、右翼。两翼都是 `flex: 1 1 0`——
 * 基数为 0、增长相同，所以两翼宽度**永远相等**，播放键作为中间项被钉在底栏中线上，
 * 与两翼各塞了多少控件无关（不再靠"左右按钮数量相等"这种脆弱前提）。
 *
 * 两翼内部又是**贴住播放键**排布（左翼 `flex-end`、右翼 `flex-start`），于是底栏读作
 * 一整条连续的控件带：左边是编辑 / 设备 / 工程读数，中间是传输，右边是时间与节拍。
 * 富余空间只留在两端的外缘，不会再在"循环"和"时间读数"之间凭空裂出一道缝。
 *
 * 分组（用 1px 竖线隔开）：
 * - 左翼：撤销 / 重做 ｜ 录音模式 + 输入设备 ｜ 工程小节数 ｜ 回到起点 + 录音；
 * - 中：播放 / 暂停；
 * - 右翼：停止 + 循环（+ 循环范围）｜ 位置读数 ｜ 输出设备 ｜ 拍号 / BPM / 节拍器 ｜ AI 忙碌提示 / 离线重试。
 *
 * 输入设备在左、输出设备在右：既是「进 → 出」的直觉，也让两翼宽度接近，
 * 播放键居中时两侧留下的空桨不至于一边倒（左翼内容原本比右翼重得多）。
 *
 * 只在"有事要说"时才出现的状态：AI 忙碌胶囊（空闲时整块不渲染）、音频离线重试按钮。
 * 采样率、实时/离线这类常驻信息不进底栏——设备提示里已有规格，设置页也有。
 *
 * 传输直接调用 `transport` store：实时播放走原生音频线程，设备不可用时自动降级为
 * 离线试听，两种模式下这些按钮的行为完全一致。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import {
  PlayIcon,
  PauseIcon,
  StopIcon,
  BackwardIcon,
  ArrowUturnLeftIcon,
  ArrowUturnRightIcon,
  ArrowPathIcon,
  XMarkIcon,
  SpeakerWaveIcon,
  MicrophoneIcon,
} from "@heroicons/vue/24/outline";
import Md3Select from "@/components/common/Md3Select.vue";
import Md3Segmented from "@/components/common/Md3Segmented.vue";
import KeyboardIcon from "@/components/common/icons/KeyboardIcon.vue";
import MetronomeIcon from "@/components/common/icons/MetronomeIcon.vue";
import NumberField from "@/components/common/NumberField.vue";
import TimeSignatureField from "@/components/common/TimeSignatureField.vue";
import Tooltip from "@/components/common/Tooltip.vue";
import { retryAudio } from "@/api/tauri";
import { useEngineEventsStore } from "@/stores/engineEvents";
import { useLayoutStore } from "@/stores/layout";
import { useMidiStore } from "@/stores/midi";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import { useI18n } from "@/i18n";
import { barBeatAt, beatsToSeconds, meterAt, tempoAt } from "@/utils/tempo";
import type { SegmentedOption } from "@/components/common/segmented";

const props = defineProps<{ aiActive: boolean }>();

const store = useProjectStore();
const transport = useTransportStore();
const timeline = useTimelineStore();
const layout = useLayoutStore();
const selection = useSelectionStore();
const midi = useMidiStore();
const i18n = useI18n();
const aiEvents = useEngineEventsStore();

/** 录音目标轨道：选中轨道，否则首轨。 */
const recordTrack = computed(() => selection.track?.id ?? store.project?.tracks[0]?.id ?? null);

/** 底栏录音模式小开关：音频 / MIDI。录制中只锁住显示，不影响正在进行的这一次。 */
const recordModeOptions = computed<SegmentedOption[]>(() => [
  {
    value: "audio",
    label: i18n.t("bottom.recordModeAudio"),
    disabled: transport.recording,
    title: i18n.t("bottom.recordModeAudioHint"),
  },
  {
    value: "midi",
    label: i18n.t("bottom.recordModeMidi"),
    disabled: transport.recording,
    title: i18n.t("bottom.recordModeMidiHint"),
  },
]);

const recordMode = computed({
  get: () => transport.recordMode,
  set: (value: string) => transport.setRecordMode(value === "midi" ? "midi" : "audio"),
});

/** 录音键的提示：模式 + 状态一起说清楚。 */
const recordHint = computed(() => {
  if (transport.recording) return i18n.t("bottom.recordStop");
  return transport.recordMode === "midi" ? i18n.t("bottom.recordMidi") : i18n.t("bottom.record");
});

/**
 * 输入设备下拉选项：首项固定「无输入设备」（id 为空串 = 关闭输入流、释放麦克风），
 * 其后是本机设备。设备项带上连接类型 / 默认标记，避免「蓝牙耳机」这类设备看着像纯输出。
 */
const inputOptions = computed(() => [
  {
    id: "",
    label: i18n.t("bottom.recordInputNone"),
    hint: i18n.t("bottom.recordInputNoneHint"),
  },
  ...transport.inputDevices.map((d) => {
    const parts: string[] = [];
    if (d.transport) parts.push(i18n.t(`bottom.inputTransport.${d.transport}`));
    if (d.is_default) parts.push(i18n.t("bottom.inputDefault"));
    parts.push(`${d.sample_rate} Hz · ${d.channels}ch`);
    return { id: d.name, label: d.name, hint: parts.join(" · ") };
  }),
]);

/** 电平表填充宽度（0..100%）。 */
const levelPct = computed(() => `${Math.round(Math.min(1, transport.inputLevelPeak) * 100)}%`);

/** MIDI 输入设备下拉：录音模式为 MIDI 时替换输入组里的「麦克风 + 音频输入」块。 */
const midiOptions = computed(() => [
  {
    id: "",
    label: i18n.t("midi.inputNone"),
    hint: i18n.t("midi.inputNoneHint"),
  },
  ...midi.inputs.map((d) => ({ id: d.name, label: d.name })),
]);

const midiDevice = computed({
  get: () => midi.input ?? "",
  set: (id: string) => void midi.select(id === "" ? null : id),
});

/** 有硬件音符正在响：给 MIDI 设备块一点活指示（没有电平表可用）。 */
const midiActive = computed(() => midi.heldNotes.length > 0);

/** 输出电平表填充宽度（0..100%）。 */
const outputLevelPct = computed(() => `${Math.round(Math.min(1, transport.outputLevelPeak) * 100)}%`);

/** 输出设备下拉：第一项固定「跟随系统默认」，其后是本机设备（带默认标记与规格）。 */
const outputOptions = computed(() => [
  { id: "", label: i18n.t("bottom.outputDefault"), hint: i18n.t("bottom.outputDefaultHint") },
  ...transport.outputDevices.map((d) => {
    const parts: string[] = [`${d.sample_rate} Hz · ${d.channels}ch`];
    if (d.is_default) parts.push(i18n.t("bottom.inputDefault"));
    return { id: d.name, label: d.name, hint: parts.join(" · ") };
  }),
]);

let levelTimer: number | null = null;

/** 切设备时重建输入流；空 id（「无输入设备」）= 关闭输入流、释放麦克风。 */
async function onSelectInput(name: string): Promise<void> {
  await transport.selectInput(name === "" ? null : name);
}

/** 切输出设备（空 id = 跟随系统默认设备）。 */
async function onSelectOutput(id: string): Promise<void> {
  await transport.selectOutput(id === "" ? null : id);
}

/** 展开输出设备菜单前刷新一次：应用运行期间插上的设备也能被看到。 */
function onPickOutput(): void {
  void transport.refreshOutputDevices();
}

/** 录音开关：开始前把起点吸附到网格。 */
async function onToggleRecord(): Promise<void> {
  if (transport.recording) {
    await transport.stopRecording();
    return;
  }
  if (recordTrack.value === null) return;
  await transport.startRecording(recordTrack.value, timeline.snapBeats(transport.positionBeats));
}

/** 停止键：录制中先收尾（写片段 / WAV），再回到起点。 */
async function onStop(): Promise<void> {
  if (transport.recording) {
    await transport.stopRecording();
    await transport.stop();
    return;
  }
  await transport.stop();
}

onMounted(async () => {
  // 只拉设备列表，**不自动打开输入流**：麦克风只在用户显式选设备或开始录音时才被占用。
  // MIDI 端口相反：它不涉及隐私、也不抢独占设备，选过的端口启动时就自动接回来。
  await Promise.all([
    transport.refreshInputDevices(),
    transport.refreshOutputDevices(),
    // MIDI 枚举 / 订阅失败不该拦住电平表与底栏其余功能。
    midi.init().catch(() => {}),
  ]);
  // 电平表轮询：只在底部条存在时跑。输入流没开 / 没在播放时 store 会直接归零，不发 IPC。
  levelTimer = window.setInterval(() => {
    void transport.pollInputLevel();
    void transport.pollOutputLevel();
  }, 120);
});
onUnmounted(() => {
  if (levelTimer !== null) window.clearInterval(levelTimer);
  midi.dispose();
});

/** 当前工程的传输设置（分段换算用）。 */
const settings = computed(
  () => store.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] },
);

/** 播放头位置（拍）：BPM / 拍号 / 读数都按它取生效值。 */
const atBeat = computed(() => transport.positionBeats);

/** 播放头处的生效拍号（随播放头移动）。 */
const timeSigNum = computed(() => meterAt(atBeat.value, settings.value).num);
const timeSigDen = computed(() => meterAt(atBeat.value, settings.value).den);

const tempo = ref(tempoAt(0, settings.value));

// 播放头 / 工程变化时，BPM 输入框跟随该处的生效值；靠「与生效值比较」避免自触发回写。
watch(
  [() => store.project?.transport, atBeat],
  () => {
    tempo.value = tempoAt(atBeat.value, settings.value);
  },
  { immediate: true, deep: true },
);

// 步进器已把值夹在 20..300，store 还会再校验一次。值等于该处生效值时说明是上面的同步，不回写。
watch(tempo, (t) => {
  if (t === tempoAt(atBeat.value, settings.value)) return;
  void store.applyTempoAt(atBeat.value, t);
});

/** 拍号变化：写在对准的关键点或基准（第 0 拍）上。 */
function onTimeSignature(num: number, den: number): void {
  void store.applyMeterAt(atBeat.value, num, den);
}

/** 播放头是否正对准某个关键点（显示徽标）。 */
const onKeyframe = computed(() => store.keyframeAt(atBeat.value) !== null);

/** 工程内容长度（小节）。 */
const bars = computed(() => {
  const beats = timeline.contentBeats;
  if (beats <= 0) return 0;
  // 内容占的是 `[0, beats)`：正好收在小节线上时，**那条线属于下一小节的开头、不算占用**
  // （12 小节整的片段必须读 12，不是 13）。所以看的是「最后一个被占用的瞬间」落在第几小节。
  return barBeatAt(Math.max(0, beats - 1e-9), settings.value).bar;
});

/** 当前位置（小节.拍），随实时位置推进，按分段读数。 */
const positionText = computed(() => {
  const p = barBeatAt(transport.positionBeats, settings.value);
  return `${p.bar}.${p.beatInBar}`;
});

/**
 * 播放头的**时间**位置（秒）。
 *
 * 工程按拍记时间、速度可以随关键点变化，所以不能拿「拍数 × 当前 BPM」直接算——
 * 必须走 `beatsToSeconds` 的分段换算，读数才和听到的位置一致。
 */
const positionSeconds = computed(() => beatsToSeconds(transport.positionBeats, settings.value));

/** 时 / 分 / 秒三段（分秒补零，小时不补：超过一小时才占位，读数仍是等宽对齐）。 */
const timeParts = computed(() => {
  const total = Math.max(0, Math.floor(positionSeconds.value));
  return {
    hours: Math.floor(total / 3600),
    minutes: String(Math.floor(total / 60) % 60).padStart(2, "0"),
    seconds: String(total % 60).padStart(2, "0"),
  };
});

/** 纯文本形式（tooltip / aria 用）。 */
const timeText = computed(
  () => `${timeParts.value.hours}:${timeParts.value.minutes}:${timeParts.value.seconds}`,
);

/** 读数的完整说明：时间 + 小节.拍（底栏只放时间，小节读数留给 tooltip）。 */
const timeHint = computed(() =>
  i18n.t("bottom.timeTitle", { time: timeText.value, bar: positionText.value }),
);

const isPlaying = computed(() => transport.playing);
const isAiActive = computed(() => props.aiActive);
const isPreview = computed(() => transport.mode === "preview");

/** 取消循环范围（回到整首循环）。给一句提示，免得用户以为「循环被关掉了」。 */
async function clearLoopRange(): Promise<void> {
  await transport.clearLoopRange();
  layout.notify(i18n.t("bottom.loopRangeCleared"));
}

/**
 * AI 胶囊文案：只在代理真的在写的时候出现（空闲时整块不渲染，底栏不留常驻状态灯）。
 */
const aiText = computed(() => {
  const agent = aiEvents.activeAgent();
  return agent ? i18n.t("bottom.aiBusyNamed", { agent }) : i18n.t("bottom.aiBusy");
});

/** AI 胶囊 tooltip：本次由代理改动的处数与涉及的轨道；还没有改动时就说「正在写」。 */
const aiHint = computed(() => {
  const actions = aiEvents.agentActions;
  if (actions.length === 0) return aiText.value;
  const agent = actions[actions.length - 1].agent;
  const tracks = [...new Set(actions.map((a) => a.track).filter((t): t is number => t !== null))].sort(
    (a, b) => a - b,
  );
  const base = i18n.t("bottom.aiRecent", { agent, n: actions.length });
  return tracks.length > 0 ? `${base} · ${i18n.t("bottom.aiTracks", { tracks: tracks.join(", ") })}` : base;
});

/** 离线提示：把后端给出的原因（可能缺失）拼成「原因 + 点击重试」。 */
const offlineHint = computed(() =>
  i18n.t("bottom.audioOfflineHint", {
    error: transport.error ?? i18n.t("bottom.audioUnavailable"),
  }),
);

/** 是否有一次「重试音频」请求在途：用于合并重复点击。 */
const retrying = ref(false);

/** 重试输出设备：重新探测并重建实时宿主；结果由传输事件推回来，无需手动刷新。 */
function onRetryAudio(): void {
  // 连点/重复触发时合并为一次在途请求，避免给音频线程灌入多条 Retry。
  if (retrying.value) return;
  retrying.value = true;
  // 失败无需在此处理：可用性由传输事件/轮询推回，界面会继续显示离线提示。这里只兜住
  // promise，避免未处理的 rejection（沿用本项目静默吞掉后台失败的做法，见 App.vue）。
  void retryAudio()
    .catch(() => {})
    .finally(() => {
      retrying.value = false;
    });
}
</script>

<template>
  <footer class="bottom-bar" data-tour="transport">
    <!--
      左翼：编辑历史 → 录音输入 → 输出 → 工程读数 → 传输左簇。
      `flex-end` 让它**贴住播放键**，富余空间全部留给窗口左缘。
    -->
    <div class="flank left">
      <Tooltip :text="i18n.t('bottom.undo')" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.undoDepth === 0"
          :aria-label="i18n.t('bottom.undoLabel')"
          @click="store.undo()"
        >
          <ArrowUturnLeftIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>
      <Tooltip :text="i18n.t('bottom.redo')" place="above" align="center">
        <button
          class="t-btn"
          :disabled="store.redoDepth === 0"
          :aria-label="i18n.t('bottom.redoLabel')"
          @click="store.redo()"
        >
          <ArrowUturnRightIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <span class="t-sep" />

      <span v-if="onKeyframe" class="keyframe-pill" :title="i18n.t('bottom.keyframeHint')">
        {{ i18n.t("bottom.keyframe") }}
      </span>

      <!--
        输入组：录音模式 + 输入设备。模式决定这一块是「麦克风 + 电平」还是「MIDI 端口」，
        两者必须挨着才读得懂，所以合成一组。窄窗口只藏设备下拉，模式开关留着。
      -->
      <div class="input-group">
        <Tooltip :text="i18n.t('bottom.recordModeHint')" place="above" align="center">
          <Md3Segmented
            v-model="recordMode"
            :options="recordModeOptions"
            size="sm"
            :aria-label="i18n.t('bottom.recordModeLabel')"
            class="record-mode"
          />
        </Tooltip>

        <Tooltip
          v-if="transport.recordMode === 'audio'"
          class="device-tip"
          :text="i18n.t('bottom.recordLevel')"
          place="above"
          align="center"
        >
          <span class="device-block" :class="{ idle: !transport.inputActive }">
            <MicrophoneIcon class="pick-icon" />
            <Md3Select
              :model-value="transport.inputDevice ?? ''"
              :options="inputOptions"
              :label="i18n.t('bottom.recordInputLabel')"
              :placeholder="i18n.t('bottom.recordInputNone')"
              @update:model-value="onSelectInput"
            >
              <template #background>
                <span class="level-fill" :style="{ width: levelPct }" aria-hidden="true" />
              </template>
            </Md3Select>
          </span>
        </Tooltip>

        <Tooltip
          v-else
          class="device-tip"
          :text="i18n.t('midi.inputLabel')"
          place="above"
          align="center"
        >
          <span
            class="device-block midi"
            :class="{ idle: !midi.input, live: midiActive }"
            @pointerdown="midi.refresh()"
          >
            <KeyboardIcon class="pick-icon" />
            <Md3Select
              v-model="midiDevice"
              :options="midiOptions"
              :label="i18n.t('midi.inputLabel')"
              :placeholder="i18n.t('midi.inputNone')"
            />
          </span>
        </Tooltip>
      </div>

      <span class="t-sep" />

      <span class="readout">
        <span class="strong tabular">{{ bars }}</span>
        <span class="readout-label">{{ i18n.t("bottom.bar") }}</span>
      </span>

      <span class="t-sep" />

      <!-- 传输左簇：回到起点 + 录音（与右簇的「停止 + 循环」等宽对称） -->
      <Tooltip :text="i18n.t('bottom.toStart')" place="above" align="center">
        <button class="t-btn" :aria-label="i18n.t('bottom.toStartLabel')" @click="transport.rewind()">
          <BackwardIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <Tooltip :text="recordHint" place="above" align="center">
        <button
          class="rec-btn"
          :class="{ on: transport.recording, midi: transport.recordMode === 'midi' }"
          :disabled="recordTrack === null"
          :aria-pressed="transport.recording"
          :aria-label="
            transport.recording
              ? i18n.t('bottom.recordStopLabel')
              : transport.recordMode === 'midi'
                ? i18n.t('bottom.recordMidi')
                : i18n.t('bottom.recordLabel')
          "
          @click="onToggleRecord"
        >
          <span class="rec-dot" />
        </button>
      </Tooltip>
    </div>

    <!-- 正中：播放 / 暂停。两翼等宽 → 它恒落在底栏中线上。 -->
    <Tooltip
      class="play-tip"
      :text="isPlaying ? i18n.t('bottom.pause') : i18n.t('bottom.play')"
      place="above"
      align="center"
    >
      <button
        class="play-btn"
        :class="{ playing: isPlaying }"
        :aria-label="isPlaying ? i18n.t('bottom.pauseLabel') : i18n.t('bottom.playLabel')"
        @click="transport.toggle()"
      >
        <PauseIcon v-if="isPlaying" class="w-[18px] h-[18px]" />
        <PlayIcon v-else class="w-[20px] h-[20px]" />
      </button>
    </Tooltip>

    <!--
      右翼：传输右簇（贴住播放键）→ 位置读数 → 拍号 / BPM / 节拍器 → 只在需要时出现的状态。
    -->
    <div class="flank right">
      <Tooltip :text="i18n.t('bottom.stop')" place="above" align="center">
        <button class="t-btn" :aria-label="i18n.t('bottom.stopLabel')" @click="onStop">
          <StopIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <Tooltip :text="i18n.t('bottom.loop')" place="above" align="center">
        <button
          class="t-btn"
          :class="{ on: transport.loop }"
          :aria-pressed="transport.loop"
          :aria-label="i18n.t('bottom.loopLabel')"
          @click="transport.setLoop(!transport.loop)"
        >
          <ArrowPathIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>

      <!--
        循环范围徽标：只在标尺上划出了范围时出现。右键标尺也能取消，但那条手势太隐晦，
        底栏给一个明面上的出口（点它即取消，并提示一下「回到整首循环」）。
      -->
      <Tooltip
        v-if="transport.hasLoopRange"
        :text="i18n.t('bottom.loopRangeClear')"
        place="above"
        align="center"
      >
        <button class="range-pill" :aria-label="i18n.t('bottom.loopRangeClear')" @click="clearLoopRange">
          <span class="tabular">{{ transport.loopRangeLabel }}</span>
          <XMarkIcon class="w-[12px] h-[12px]" />
        </button>
      </Tooltip>

      <span class="t-sep" />

      <!--
        播放头位置：**时 / 分 / 秒**三段一组，上下两行（数字 + 单位）的 MD3 读数块。
        底栏只有 60px 高，两行比「0时:01分:23秒」一行更省横向空间，也和右侧的拍号 / BPM
        落在同一条水平线上。小节.拍 放进 tooltip——那仍然是编排时最常用的定位口径。
      -->
      <Tooltip :text="timeHint" place="above" align="center">
        <span class="timecode" :class="{ on: isPlaying }" role="timer" :aria-label="timeHint">
          <!-- 不足一小时只显示「分:秒」：多数工程都在这段，省下的宽度留给拍号 / BPM。 -->
          <template v-if="timeParts.hours > 0">
            <span class="timecode-cell">
              <span class="timecode-num tabular">{{ timeParts.hours }}</span>
              <span class="timecode-unit">{{ i18n.t("bottom.hours") }}</span>
            </span>
            <span class="timecode-sep" aria-hidden="true">:</span>
          </template>
          <span class="timecode-cell">
            <span class="timecode-num tabular">{{ timeParts.minutes }}</span>
            <span class="timecode-unit">{{ i18n.t("bottom.minutes") }}</span>
          </span>
          <span class="timecode-sep" aria-hidden="true">:</span>
          <span class="timecode-cell sec">
            <span class="timecode-num tabular">{{ timeParts.seconds }}</span>
            <span class="timecode-unit">{{ i18n.t("bottom.seconds") }}</span>
          </span>
        </span>
      </Tooltip>

      <!--
        输出设备（同一套结构；电平就是下拉框的底色）：夹在位置读数与拍号之间——
        传输簇后先看时间，再是设备，最后才是拍号 / BPM 这类节拍设置。
        输入在左、输出在右，两翼宽度也因此接近，播放键居中时两侧不至于一边倒。
      -->
      <span class="t-sep out-sep" />

      <Tooltip
        class="device-tip output-tip"
        :text="i18n.t('bottom.outputLevel')"
        place="above"
        align="center"
      >
        <span class="device-block" @pointerdown="onPickOutput">
          <SpeakerWaveIcon class="pick-icon" />
          <Md3Select
            :model-value="transport.outputDevice ?? ''"
            :options="outputOptions"
            :label="i18n.t('bottom.outputLabel')"
            @update:model-value="onSelectOutput"
          >
            <template #background>
              <span class="level-fill" :style="{ width: outputLevelPct }" aria-hidden="true" />
            </template>
          </Md3Select>
        </span>
      </Tooltip>

      <span class="t-sep" />

      <TimeSignatureField :num="timeSigNum" :den="timeSigDen" @change="onTimeSignature" />
      <NumberField v-model="tempo" :min="20" :max="300" label="BPM" />
      <Tooltip :text="i18n.t('bottom.metronome')" place="above" align="center">
        <button
          class="t-btn"
          :class="{ on: transport.metronome }"
          :aria-pressed="transport.metronome"
          :aria-label="i18n.t('bottom.metronomeLabel')"
          @click="transport.setMetronome(!transport.metronome)"
        >
          <MetronomeIcon />
        </button>
      </Tooltip>

      <!--
        状态区：只在「有事要说」时渲染，且**贴住右缘**（`margin-left: auto`）——
        平时整块不存在，底栏右端就是空的；代理在写 / 音频离线时，提示才落在最右角，
        不会挤动中间的传输与读数。
      -->
      <span v-if="isAiActive || isPreview" class="status">
        <!-- 代理在写时才出现；空闲态不占位、不发光，底栏不留常驻状态灯。 -->
        <Tooltip v-if="isAiActive" :text="aiHint" place="above" align="end">
          <span class="ai-pill">
            <span class="dot" />
            <span class="ai-text">{{ aiText }}</span>
          </span>
        </Tooltip>

        <!-- 实时可用时这里什么都没有；离线才长出可点的重试按钮（原因在 tooltip 里）。 -->
        <Tooltip v-if="isPreview" :text="offlineHint" place="above" align="end">
          <button class="audio-retry" :aria-label="i18n.t('bottom.retryAudio')" @click="onRetryAudio">
            <ArrowPathIcon class="w-[13px] h-[13px]" />
            {{
              transport.previewPreparing
                ? i18n.t("bottom.preparingPreview")
                : i18n.t("bottom.offline")
            }}
          </button>
        </Tooltip>
      </span>
    </div>
  </footer>
</template>

<style scoped>
/*
 * 两翼 + 播放键。两翼 `flex: 1 1 0`（基数 0、增长相同）→ 宽度恒相等，
 * 中间项（播放键）因此**严格落在底栏中线上**，不论两翼各有多少控件。
 */
.bottom-bar {
  height: 60px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 14px;
  flex: 0 0 auto;
  background-color: var(--app-card);
  border-top: 1px solid rgb(var(--md-outline-variant) / 0.4);
  font-size: 11.5px;
  letter-spacing: 0.3px;
  color: rgb(var(--md-on-surface-variant));
}
/*
 * 翼内控件贴住播放键：左翼 `flex-end`、右翼 `flex-start`。
 * 于是底栏读作一整条连续的控件带，富余空间只落在窗口两端。
 */
.flank {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}
.flank.left {
  justify-content: flex-end;
}
.flank.right {
  justify-content: flex-start;
}
.t-btn {
  width: 32px;
  height: 32px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
  transition:
    background-color var(--md-ease-emphasized),
    color var(--md-ease-emphasized),
    transform var(--md-ease-emphasized);
}
.t-btn:not(:disabled):hover {
  background-color: rgb(var(--md-on-surface-variant) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
.t-btn:not(:disabled):active {
  transform: scale(0.92);
}
.t-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
/* 激活态（如循环开关打开）：与选中标签同一套配色。 */
.t-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
.t-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
.t-sep {
  width: 1px;
  height: 18px;
  margin: 0 4px;
  background-color: rgb(var(--md-outline-variant) / 0.6);
  flex: 0 0 auto;
}
/* 播放头对准关键点时的徽标：提醒「这里的改动只影响该关键点之后」。 */
.keyframe-pill {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-tertiary-container));
  color: rgb(var(--md-on-tertiary-container));
  font-size: 10.5px;
  letter-spacing: 0.4px;
  white-space: nowrap;
  flex: 0 0 auto;
}
/*
 * 循环范围徽标：显示「第 X–Y 小节」，点它取消范围。
 * 用**主色**而不是关键点那套三色——循环范围是运行时状态、跟关键点那种工程数据不是一回事，
 * 标尺上那条范围带也是同一个主色，两处能对上。
 */
.range-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 8px;
  margin-left: 2px;
  border: 0;
  border-radius: 999px;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  font-size: 10.5px;
  letter-spacing: 0.4px;
  white-space: nowrap;
  flex: 0 0 auto;
  cursor: pointer;
  transition: background-color var(--md-ease-emphasized);
}
.range-pill:hover {
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
.range-pill:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
/* 播放键：两翼等宽，所以它恒在底栏中线上；这里只需要固定尺寸、不参与伸缩。 */
.play-tip {
  flex: 0 0 auto;
}
.play-btn {
  position: relative;
  width: 44px;
  height: 44px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  cursor: pointer;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
  box-shadow: var(--md-elevation-1);
  transition:
    transform var(--md-ease-emphasized),
    background-color var(--md-ease-emphasized),
    box-shadow var(--md-ease-emphasized);
}
.play-btn:hover {
  transform: scale(1.07);
  box-shadow: var(--md-elevation-2);
}
.play-btn:active {
  transform: scale(0.93);
}
.play-btn.playing {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
/* 播放中：一圈缓慢扩散的光晕，把「正在走」这件事从数字之外也说出来。 */
.play-btn.playing::after {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: inherit;
  border: 2px solid rgb(var(--md-primary) / 0.5);
  animation: play-halo 2.2s var(--md-ease-standard-decel) infinite;
  pointer-events: none;
}
@keyframes play-halo {
  0% {
    transform: scale(1);
    opacity: 0.7;
  }
  70% {
    transform: scale(1.45);
    opacity: 0;
  }
  100% {
    transform: scale(1.45);
    opacity: 0;
  }
}
.play-btn:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 3px;
}
/* 录音键：空闲是暗红圆点，录制中点亮并放大。 */
.rec-btn {
  width: 32px;
  height: 32px;
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  cursor: pointer;
}
.rec-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.rec-dot {
  width: 14px;
  height: 14px;
  border-radius: 999px;
  background-color: rgb(var(--md-error));
  transition:
    box-shadow var(--md-ease-emphasized),
    transform var(--md-ease-emphasized);
}
.rec-btn.on .rec-dot {
  box-shadow: 0 0 0 4px rgb(var(--md-error) / 0.25);
  transform: scale(1.1);
}
/* MIDI 模式：红点外一圈主色细环，不看旁边小开关也知道这次录的是 MIDI。 */
.rec-btn.midi .rec-dot {
  box-shadow: 0 0 0 2px rgb(var(--md-primary) / 0.45);
}
.rec-btn.on.midi .rec-dot {
  box-shadow: 0 0 0 4px rgb(var(--md-error) / 0.25), 0 0 0 6px rgb(var(--md-primary) / 0.22);
}
/* 录音模式小开关：紧贴输入设备，字号再收一点，避免底栏被撑宽。 */
.record-mode {
  flex: 0 0 auto;
}
.record-mode :deep(.md3-seg-btn) {
  padding: 0 8px;
  font-size: 10.5px;
  letter-spacing: 0.2px;
}
/* 输入组：模式开关 + 设备块，两者共同决定「录什么」。 */
.input-group {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 0 1 auto;
}
/* Tooltip 的外壳默认按内容撑开、且带 `min-width:auto`；底栏变窄时它会把
   设备块钉在 158px，宁可把撤销键挤出窗口也不肯收缩。这里放开下限，
   让内部的 `.device-block` 继续按自己的规则缩。 */
.device-tip {
  min-width: 0;
  flex: 0 1 auto;
}
/* 设备块（输入 / 输出共用）：图标 + 下拉同处一行，与撤销、读数落在同一水平线上。
   固定宽度 + 单行省略号，绝不撑大翼宽；空间不够时先缩到 96px，再窄就整块隐藏。 */
.device-block {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 158px;
  min-width: 96px;
  flex: 0 1 auto;
}
.device-block :deep(.md3-select) {
  flex: 1 1 0;
  min-width: 0;
}
.pick-icon {
  flex: 0 0 auto;
  width: 14px;
  height: 14px;
  opacity: 0.6;
  transition: opacity 0.18s ease;
}
/* 没有输入设备时把麦克风压暗（输出不压：图标变暗会被误读成「设备不可用」）。
   电平本身没有信号时就是空的，不再额外保留一条底轨。 */
.device-block.idle .pick-icon {
  opacity: 0.32;
}
/* MIDI 模式：有硬件音符在响时把键盘图标点亮（没有电平表，这是唯一的「收到消息」指示）。 */
.device-block.midi.live .pick-icon {
  opacity: 1;
  color: rgb(var(--md-primary));
}
/* 电平：铺在下拉框**内部**的底色，从左侧随峰值长出来。
   底色刻意压得很淡（0.14 → 0.26），设备名始终压在上面；右缘叠一条 1px 的亮边，
   低电平时也能看清当前刻度落在哪里。亮边用 background 而不是 inset 阴影——
   宽度为 0 时背景不会绘制，电平归零后不会在框内留下一个假的刻度点。 */
.level-fill {
  position: absolute;
  inset: 0 auto 0 0;
  z-index: 0;
  width: 0;
  pointer-events: none;
  background-image:
    linear-gradient(rgb(var(--md-primary) / 0.45), rgb(var(--md-primary) / 0.45)),
    linear-gradient(90deg, rgb(var(--md-primary) / 0.14), rgb(var(--md-primary) / 0.26));
  background-size: 1px 100%, 100% 100%;
  background-position: right center, left center;
  background-repeat: no-repeat;
  transition: width 120ms linear;
}
/*
 * 播放头时间读数（时 / 分 / 秒）。
 *
 * 每段是「数字在上、单位在下」的两行小格：底栏只有 60px 高，竖排比一行
 * `0时:01分:23秒` 更省横向宽度；数字等宽，跳动时不会把旁边的拍号 / BPM 挤得左右晃。
 * 底是 `surface-container-high` 的药丸 + 一圈极淡的 outline，和设备块同一档层级。
 */
.timecode {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 9px 2px;
  flex: 0 0 auto;
  border-radius: 10px;
  background-color: rgb(var(--md-surface-container-high));
  box-shadow: inset 0 0 0 1px rgb(var(--md-outline-variant) / 0.35);
  transition: background-color var(--md-ease-emphasized);
}
.timecode-cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  min-width: 16px;
  line-height: 1;
}
.timecode-num {
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.2px;
  color: rgb(var(--md-on-surface));
}
.timecode-unit {
  font-size: 9px;
  line-height: 1;
  color: rgb(var(--md-on-surface-variant));
  opacity: 0.8;
}
/* 分隔号只跟「数字」那一行对齐：单位行是注脚，不参与。 */
.timecode-sep {
  align-self: flex-start;
  font-size: 12px;
  font-weight: 600;
  line-height: 13px;
  color: rgb(var(--md-outline));
}
/* 播放中：只把秒染色（活指示），整块不闪、不占宽度。 */
.timecode.on {
  background-color: rgb(var(--md-primary) / 0.1);
}
.timecode.on .timecode-cell.sec .timecode-num {
  color: rgb(var(--md-primary));
}
.readout {
  display: inline-flex;
  align-items: baseline;
  gap: 6px;
  white-space: nowrap;
  flex: 0 0 auto;
}
.readout-label {
  opacity: 0.72;
}
.tabular {
  font-variant-numeric: tabular-nums;
}
.strong {
  color: rgb(var(--md-on-surface));
  font-weight: 600;
  min-width: 20px;
  text-align: right;
}
/* AI 忙碌胶囊：只在代理写东西时出现，本身就是「活着」的信号，不再有空闲态。 */
.ai-pill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 24px;
  padding: 0 10px;
  border-radius: 999px;
  white-space: nowrap;
  flex: 0 0 auto;
  background-color: rgb(var(--md-primary-container));
  color: rgb(var(--md-on-primary-container));
}
.ai-pill .dot {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  flex: 0 0 auto;
  background-color: rgb(var(--md-primary));
  box-shadow: 0 0 0 3px rgb(var(--md-primary) / 0.25);
}
/* 离线指示本身就是重试按钮：小药丸，配色沿用 tertiary（与原来的离线色一致）。 */
.audio-retry {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 22px;
  padding: 0 9px;
  flex: 0 0 auto;
  border: 0;
  border-radius: 999px;
  cursor: pointer;
  font: inherit;
  letter-spacing: inherit;
  color: rgb(var(--md-tertiary));
  background-color: rgb(var(--md-tertiary) / 0.12);
  transition:
    background-color var(--md-ease-emphasized),
    transform var(--md-ease-emphasized);
}
.audio-retry:hover {
  background-color: rgb(var(--md-tertiary) / 0.22);
}
.audio-retry:active {
  transform: scale(0.96);
}
.audio-retry:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}

/* 状态区：贴住底栏右缘。平时不渲染，所以这里只负责把它推到最右。 */
.status {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
  padding-left: 8px;
  flex: 0 0 auto;
}

/* 窄屏：由远及近地丢信息，两翼始终等宽 → 播放键不会被挤偏。 */
@media (max-width: 1140px) {
  /* 设备下拉先走（还要留出撤销/重做与传输）。录音模式的开关留着——
     它是小控件，且没有它就没法切 MIDI 录音。输出设备前面那根竖线一起收掉，
     不然会剩下一根悬空的隔断。 */
  .device-tip,
  .out-sep {
    display: none;
  }
}
@media (max-width: 980px) {
  .readout {
    display: none;
  }
}
@media (max-width: 860px) {
  .timecode {
    display: none;
  }
}
@media (max-width: 760px) {
  .ai-text {
    display: none;
  }
}
@media (prefers-reduced-motion: reduce) {
  .play-btn,
  .t-btn,
  .audio-retry,
  .ai-pill .dot,
  .timecode {
    transition: background-color 0.15s ease;
  }
  .play-btn:hover,
  .play-btn:active,
  .t-btn:not(:disabled):active,
  .audio-retry:active {
    transform: none;
  }
  /* 电平本来就是 120ms 一次的离散读数，去掉补间仍然读得懂。 */
  .level-fill {
    transition: none;
  }
}
</style>
