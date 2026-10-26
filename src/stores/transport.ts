/**
 * 传输状态：真实实时播放 + 离线试听回退。
 *
 * - **realtime**：后端在专用音频线程上持有 cpal 宿主，播放/暂停/停止/定位都是即时命令，
 *   位置由 `engine://transport` 事件推送（约 30 Hz）。
 * - **preview**：设备不可用（或后端未打开成功）时自动降级为「离线渲染 → `<audio>`」，
 *   位置由音频元素推进。两条路径对外暴露同一组状态，界面无需分支。
 */
import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import * as api from "@/api/tauri";
import type { InputDeviceInfo, OutputDeviceInfo, TransportSnapshot } from "@/api/types";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useTimelineStore } from "@/stores/timeline";
import { useI18n } from "@/i18n";
import { beatsToSeconds, secondsToBeats } from "@/utils/tempo";

/** 离线试听的渲染采样率。 */
const PREVIEW_SAMPLE_RATE = 44_100;
/** 实时状态未知时的兜底采样率（避免除以 0）。 */
const FALLBACK_SAMPLE_RATE = 48_000;

export const useTransportStore = defineStore("transport", () => {
  const layout = useLayoutStore();
  const i18n = useI18n();
  const project = useProjectStore();
  const timeline = useTimelineStore();

  // 乐观初值：初始化完成前先按「实时可用」处理，失败后再降级并提示。
  const available = ref(true);
  const error = ref<string | null>(null);
  const sampleRate = ref(0);
  const playing = ref(false);
  const positionFrames = ref(0);
  /** 循环播放开关（运行时状态，不进工程）。 */
  const loop = ref(false);
  /**
   * 播放范围（cycle range）的起止帧。`loopEndFrames > loopStartFrames` 才算设了范围；
   * 否则按「整首循环」处理。范围本身不启动播放、也不改 `loop`——由 `setLoopRange` 一并打开。
   */
  const loopStartFrames = ref(0);
  const loopEndFrames = ref(0);
  /** 节拍器开关（运行时状态，不进工程）。 */
  const metronome = ref(false);
  const ready = ref(false);

  let unlisten: (() => void) | null = null;
  /** 本会话是否已由界面改过播放范围；改过之后界面即权威，不再回吞引擎快照。 */
  let rangeTouched = false;

  /** 离线试听的播放器（仅预览模式使用）。 */
  let audio: HTMLAudioElement | null = null;
  let url: string | null = null;
  let token = 0;
  let raf: number | null = null;

  const mode = computed<"realtime" | "preview">(() => (available.value ? "realtime" : "preview"));

  /** 当前实际生效的采样率：实时用设备采样率，预览用渲染采样率。 */
  const effectiveSampleRate = computed(() =>
    mode.value === "preview" ? PREVIEW_SAMPLE_RATE : sampleRate.value || FALLBACK_SAMPLE_RATE,
  );

  /** 当前工程的传输设置（分段换算用）。 */
  const transportSettings = computed(
    () => project.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] },
  );

  /** 播放位置（拍）。 */
  const positionBeats = computed(() =>
    secondsToBeats(positionFrames.value / effectiveSampleRate.value, transportSettings.value),
  );

  /** 是否设了播放范围（`end > start`）。 */
  const hasLoopRange = computed(() => loopEndFrames.value > loopStartFrames.value);

  /** 播放范围起点（拍）；没设范围时是 0。 */
  const loopStartBeats = computed(() =>
    hasLoopRange.value
      ? secondsToBeats(loopStartFrames.value / effectiveSampleRate.value, transportSettings.value)
      : 0,
  );

  /** 播放范围终点（拍）；没设范围时是 0。 */
  const loopEndBeats = computed(() =>
    hasLoopRange.value
      ? secondsToBeats(loopEndFrames.value / effectiveSampleRate.value, transportSettings.value)
      : 0,
  );

  /**
   * 播放范围的小节文案（例如「第 3–6 小节」）；没设范围时是空串。
   *
   * 放在 store 里而不是各组件里：「第几小节」要问时间轴（拍号 / 速度分段），而范围本身归
   * 传输管——标尺与底栏都要显示，不该各算一遍。`t()` 读的是响应式的 `locale`，所以切语言会跟着变。
   */
  const loopRangeLabel = computed(() => {
    if (!hasLoopRange.value) return "";
    const { first, last } = timeline.barRange(loopStartBeats.value, loopEndBeats.value);
    return first === last
      ? i18n.t("transport.rangeSingle", { bar: first })
      : i18n.t("transport.rangeLabel", { start: first, end: last });
  });

  /** 拍 → 帧（与后端分段换算同一套）。 */
  function beatsToFrames(beats: number): number {
    if (!Number.isFinite(beats) || beats <= 0) return 0;
    return Math.round(beatsToSeconds(beats, transportSettings.value) * effectiveSampleRate.value);
  }

  function applySnapshot(snapshot: TransportSnapshot): void {
    available.value = snapshot.available;
    error.value = snapshot.error;
    sampleRate.value = snapshot.sample_rate;
    playing.value = snapshot.playing;
    positionFrames.value = snapshot.position_frames;
    if (snapshot.available) loop.value = snapshot.loop;
    // 播放范围**不**回吞快照：引擎自己从不改范围，而 30 Hz 的快照可能在我们刚发完命令、
    // 音频线程还没消费时到达——回吞会让拖动中的区间来回抖；离线（命令被 `AudioEngine::send`
    // 拒绝）时更会把用户刚划好的段直接抹成 0。所以只有「本会话还没改过范围」时才采纳
    // 引擎里的值（例如界面热重载后恢复）。
    if (!rangeTouched) {
      loopStartFrames.value = snapshot.loop_start_frames;
      loopEndFrames.value = snapshot.loop_end_frames;
    }
  }

  // ---- 离线试听（预览模式） ----

  function stopPreviewClock(): void {
    if (raf !== null) {
      cancelAnimationFrame(raf);
      raf = null;
    }
  }

  function startPreviewClock(): void {
    const step = (): void => {
      if (!audio) return;
      const pos = Math.round(audio.currentTime * PREVIEW_SAMPLE_RATE);
      // `<audio>` 只会整段循环，设了播放范围就由这个时钟把播放头拉回范围起点。
      if (loop.value && hasLoopRange.value && pos >= loopEndFrames.value) {
        audio.currentTime = loopStartFrames.value / PREVIEW_SAMPLE_RATE;
        positionFrames.value = loopStartFrames.value;
      } else {
        positionFrames.value = pos;
      }
      raf = requestAnimationFrame(step);
    };
    stopPreviewClock();
    raf = requestAnimationFrame(step);
  }

  function teardownPreview(): void {
    token++;
    stopPreviewClock();
    if (audio) {
      audio.onended = null;
      audio.pause();
      audio = null;
    }
    if (url) {
      URL.revokeObjectURL(url);
      url = null;
    }
  }

  async function startPreview(): Promise<void> {
    const my = ++token;
    const resumeAt = positionFrames.value;
    try {
      const blob = await api.renderPreview(PREVIEW_SAMPLE_RATE);
      if (my !== token) return; // 期间被停止/重播
      url = URL.createObjectURL(blob);
      const el = new Audio(url);
      audio = el;
      el.loop = loop.value;
      el.onended = () => {
        playing.value = false;
        positionFrames.value = 0;
        teardownPreview();
      };
      await el.play();
      if (my !== token) return;
      if (resumeAt > 0) el.currentTime = resumeAt / PREVIEW_SAMPLE_RATE;
      playing.value = true;
      startPreviewClock();
    } catch (err) {
      teardownPreview();
      playing.value = false;
      layout.notify(i18n.t("error.audition", { error: String(err) }));
    }
  }

  // 设备插回后实时会自动恢复（音频线程每秒轮询重建）。此时必须把离线试听收干净，
  // 否则 <audio> 与实时流会同时出声、并且 rAF 时钟会把播放位置覆盖掉。
  // `mode` 目前只由 `available` 决定（只有 realtime / preview），所以该条件等价于
  // 「回到 realtime」；写成 `next !== "preview"` 是为了将来再引入别的模式时依旧能收干净。
  // `teardownPreview()` 幂等（无试听时只是自增 token 并空转），没有试听时触发也安全。
  watch(mode, (next, prev) => {
    if (prev === "preview" && next !== "preview") {
      teardownPreview();
      // 离线期间设的范围 / 循环开关没送到引擎（命令被拒）→ 回到实时后补推一次。
      void pushRange();
    }
  });

  // ---- 对外动作 ----

  /** 订阅传输状态（幂等）。 */
  async function init(): Promise<void> {
    if (unlisten) return;
    try {
      applySnapshot(await api.transportStatus());
    } catch (err) {
      available.value = false;
      error.value = String(err);
    }
    unlisten = await api.onTransportEvent(applySnapshot);
    ready.value = true;
  }

  /** 开始播放。 */
  async function play(): Promise<void> {
    if (playing.value) return;
    if (mode.value === "realtime") {
      try {
        await api.transportPlay();
        playing.value = true; // 乐观：随后的快照会纠正
        return;
      } catch (err) {
        // 后端明确回报不可用 → 立刻降级，用户至少能听到离线试听。
        available.value = false;
        error.value = String(err);
        layout.notify(`实时播放不可用，已切换为离线试听（${String(err)}）`);
      }
    }
    await startPreview();
  }

  /** 暂停（保留当前位置）。 */
  async function pause(): Promise<void> {
    if (!playing.value) return;
    if (mode.value === "realtime") {
      playing.value = false;
      await api.transportPause();
      return;
    }
    audio?.pause();
    stopPreviewClock();
    playing.value = false;
  }

  /** 停止并回到起点。 */
  async function stop(): Promise<void> {
    if (mode.value === "realtime") {
      playing.value = false;
      positionFrames.value = 0;
      await api.transportStop();
      return;
    }
    teardownPreview();
    playing.value = false;
    positionFrames.value = 0;
  }

  /** 播放 / 暂停切换。 */
  async function toggle(): Promise<void> {
    if (playing.value) await pause();
    else await play();
  }

  /** 定位到指定帧。 */
  async function seekFrames(frame: number): Promise<void> {
    const target = Math.max(0, Math.round(frame));
    if (mode.value === "realtime") {
      positionFrames.value = target;
      await api.transportSeek(target);
      return;
    }
    positionFrames.value = target;
    if (audio) audio.currentTime = target / PREVIEW_SAMPLE_RATE;
  }

  /** 定位到指定拍。 */
  async function seekBeats(beats: number): Promise<void> {
    await seekFrames(beatsToFrames(beats));
  }

  /** 回到起点。 */
  async function rewind(): Promise<void> {
    await seekFrames(0);
  }

  /** 开关循环播放。 */
  async function setLoop(on: boolean): Promise<void> {
    loop.value = on;
    if (mode.value === "realtime") {
      try {
        await api.transportSetLoop(on);
      } catch {
        // 实时不可用时保持界面状态即可；下次快照会纠正。
      }
      return;
    }
    if (audio) audio.loop = on;
  }

  /**
   * 把当前的播放范围 + 循环开关推给引擎（仅实时模式）。
   *
   * 失败不打扰：设备没起来时命令会被拒，等回到实时（`watch(mode)`）再补推一次。
   */
  async function pushRange(): Promise<void> {
    if (mode.value !== "realtime") return;
    try {
      await api.transportSetLoopRange(loopStartFrames.value, loopEndFrames.value);
      await api.transportSetLoop(loop.value);
    } catch {
      /* 设备未就绪：下次回到实时时补推 */
    }
  }

  /**
   * 设定播放范围（拍）。会一并打开循环——「划了一段」的意图就是循环这一段。
   *
   * `endBeats <= startBeats` 视同取消范围（回到整首循环）。界面上拖动区间时会以
   * 每秒数十次的频率调用，所以这里**先改本地状态、再异步推引擎**，不做等待。
   */
  async function setLoopRange(startBeats: number, endBeats: number): Promise<void> {
    const start = beatsToFrames(startBeats);
    const end = beatsToFrames(endBeats);
    const valid = end > start;
    rangeTouched = true;
    loopStartFrames.value = valid ? start : 0;
    loopEndFrames.value = valid ? end : 0;
    if (valid) loop.value = true;
    if (mode.value === "realtime") {
      await pushRange();
      return;
    }
    if (audio) audio.loop = loop.value;
  }

  /** 取消播放范围（回到整首循环；循环开关本身不动）。 */
  async function clearLoopRange(): Promise<void> {
    await setLoopRange(0, 0);
  }

  /** 开关节拍器（只在实时播放时打点；离线模式只切界面状态）。 */
  async function setMetronome(on: boolean): Promise<void> {
    metronome.value = on;
    if (mode.value === "realtime") {
      try {
        await api.transportSetMetronome(on);
      } catch {
        // 实时不可用时保持界面状态即可；下次快照会纠正。
      }
    }
  }

  /** 重新读取一次状态（例如音频设备在运行期间才可用）。 */
  async function refresh(): Promise<void> {
    try {
      applySnapshot(await api.transportStatus());
    } catch {
      // 状态查询失败不打断用户；下一次事件推送会纠正。
    }
  }

  /** 退订并释放离线试听资源。 */
  function dispose(): void {
    unlisten?.();
    unlisten = null;
    teardownPreview();
  }

  // ---- 录音（C2） ----

  /** 是否正在录音。 */
  const recording = ref(false);
  /** 可选的输入设备列表。 */
  const inputDevices = ref<InputDeviceInfo[]>([]);
  /** 当前选中的输入设备名（null = 未选）。 */
  const inputDevice = ref<string | null>(null);
  /** 输入电平表最近一帧的峰值（0..1）。 */
  const inputLevelPeak = ref(0);

  /** 拉取输入设备列表。 */
  async function refreshInputDevices(): Promise<void> {
    try {
      inputDevices.value = await api.listInputDevices();
    } catch {
      inputDevices.value = [];
    }
  }

  /** 选择输入设备（null = 关闭输入流）。 */
  async function selectInput(device: string | null): Promise<void> {
    inputDevice.value = device;
    try {
      await api.setInputDevice(device);
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 输入流是否处于打开（占用麦克风）状态。 */
  const inputActive = computed(() => recording.value || inputDevice.value !== null);

  /** 读一次输入电平（供界面画条）。输入流没开时直接归零，不打扰后端。 */
  async function pollInputLevel(points = 64): Promise<void> {
    if (!inputActive.value) {
      inputLevelPeak.value = 0;
      return;
    }
    try {
      const samples = await api.inputLevel(points);
      inputLevelPeak.value = samples.reduce((peak, v) => Math.max(peak, Math.abs(v)), 0);
    } catch {
      inputLevelPeak.value = 0;
    }
  }

  /** 关闭输入流（释放麦克风）。失败不打扰：下次开录会重新 arm。 */
  async function releaseInput(): Promise<void> {
    try {
      await api.setInputDevice(null);
    } catch {
      /* 释放失败无需提示 */
    }
  }

  // ---- 输出设备 ----

  /** 可选的输出设备列表。 */
  const outputDevices = ref<OutputDeviceInfo[]>([]);
  /** 当前选中的输出设备名（null = 跟随系统默认设备）。 */
  const outputDevice = ref<string | null>(null);

  /** 拉取输出设备列表。 */
  async function refreshOutputDevices(): Promise<void> {
    try {
      outputDevices.value = await api.listOutputDevices();
    } catch {
      outputDevices.value = [];
    }
  }

  /** 选择输出设备（null = 跟随系统默认）。后端立刻重建流，可用性由传输事件推回。 */
  async function selectOutput(device: string | null): Promise<void> {
    outputDevice.value = device;
    try {
      await api.setOutputDevice(device);
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 输出电平表最近一帧的峰值（0..1）。 */
  const outputLevelPeak = ref(0);

  /** 读一次输出电平（主输出示波器缓冲）。没在播放时直接归零，不打扰后端。 */
  async function pollOutputLevel(points = 64): Promise<void> {
    if (!playing.value) {
      outputLevelPeak.value = 0;
      return;
    }
    try {
      const samples = await api.scopeData(points);
      outputLevelPeak.value = samples.reduce((peak, v) => Math.max(peak, Math.abs(v)), 0);
    } catch {
      outputLevelPeak.value = 0;
    }
  }

  /** 开始录音：记录起点并走带（后端会一并开始播放）。 */
  async function startRecording(track: number, startBeat: number): Promise<void> {
    try {
      await api.recordStart(track, startBeat, inputDevice.value);
      recording.value = true;
      playing.value = true;
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 结束录音：停带并套用返回的工程；没有内容时提示。 */
  async function stopRecording(): Promise<void> {
    if (!recording.value) return;
    recording.value = false;
    try {
      const next = await api.recordStop();
      playing.value = false;
      if (next) await project.applyRecorded(next);
      else layout.notify(i18n.t("bottom.recordEmpty"));
    } catch (err) {
      layout.notify(i18n.t("error.record", { error: String(err) }));
    } finally {
      // 录音前没显式选设备（用的是系统默认）→ 录完就释放输入流，不让麦克风一直开着。
      if (inputDevice.value === null) await releaseInput();
    }
  }

  return {
    available,
    error,
    sampleRate,
    playing,
    positionFrames,
    positionBeats,
    loop,
    loopStartFrames,
    loopEndFrames,
    loopStartBeats,
    loopEndBeats,
    loopRangeLabel,
    hasLoopRange,
    metronome,
    ready,
    mode,
    effectiveSampleRate,
    init,
    refresh,
    play,
    pause,
    stop,
    toggle,
    seekFrames,
    seekBeats,
    rewind,
    setLoop,
    setLoopRange,
    clearLoopRange,
    setMetronome,
    dispose,
    recording,
    inputDevices,
    inputDevice,
    inputLevelPeak,
    inputActive,
    outputDevices,
    outputDevice,
    outputLevelPeak,
    refreshOutputDevices,
    selectOutput,
    pollOutputLevel,
    refreshInputDevices,
    selectInput,
    pollInputLevel,
    startRecording,
    stopRecording,
  };
});
