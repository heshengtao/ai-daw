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
import type { InputDeviceInfo, Note, OutputDeviceInfo, TransportSnapshot } from "@/api/types";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { useTimelineStore } from "@/stores/timeline";
import { useI18n } from "@/i18n";
import { beatsToSeconds, secondsToBeats } from "@/utils/tempo";

/** 离线试听的渲染采样率。 */
const PREVIEW_SAMPLE_RATE = 44_100;
/** 实时状态未知时的兜底采样率（避免除以 0）。 */
const FALLBACK_SAMPLE_RATE = 48_000;

/** 录音模式：音频（麦克风 / 线路输入）或 MIDI（虚拟键盘 / 硬件键盘）。 */
export type RecordMode = "audio" | "midi";

/** 录音模式选择的持久化键（跨会话记住上次的选择）。 */
const RECORD_MODE_KEY = "ai-daw-record-mode";

function initialRecordMode(): RecordMode {
  try {
    return localStorage.getItem(RECORD_MODE_KEY) === "midi" ? "midi" : "audio";
  } catch {
    return "audio";
  }
}

export const useTransportStore = defineStore("transport", () => {
  const layout = useLayoutStore();
  const i18n = useI18n();
  const project = useProjectStore();
  const selection = useSelectionStore();
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
  /** 启动宽限期：音频宿主是首帧之后才开始建的，这几秒里别急着报「离线」。 */
  const bootGrace = ref(true);
  let graceTimer: number | null = null;
  /** 正在等实时宿主启动（按播放后、命令真正被接受前）。 */
  const starting = ref(false);
  let playToken = 0;
  /** 进程（窗口）启动时刻：用来区分「宿主还在启动」与「设备真的不可用」。 */
  const startedAt = Date.now();
  /** 本会话是否已由界面改过播放范围；改过之后界面即权威，不再回吞引擎快照。 */
  let rangeTouched = false;

  /** 离线试听的播放器（仅预览模式使用）。 */
  let audio: HTMLAudioElement | null = null;
  let url: string | null = null;
  /**
   * 已经渲染好的离线试听（预览模式）。
   *
   * 整首离线渲染是**分钟级**的，而暂停 / 再播不该再等一遍。工程一改就作废（见下面的
   * `watch`）；`stop()` 只收播放器、保留缓存，所以「停 → 播」是即时的。
   */
  let previewBlob: Blob | null = null;
  /** 正在渲染离线试听：此时按暂停 = 取消这一次渲染。 */
  const previewPreparing = ref(false);
  let token = 0;
  let raf: number | null = null;
  /** `positionFrames` 是按哪个采样率记的（实时 = 设备率，预览 = 44.1 kHz）。 */
  let positionRate = FALLBACK_SAMPLE_RATE;
  /** 预览模式下的自动恢复轮询：事件丢一拍时也能回到实时。 */
  let recoveryTimer: number | null = null;
  let retryTimer: number | null = null;

  const mode = computed<"realtime" | "preview">(() =>
    available.value || bootGrace.value ? "realtime" : "preview",
  );

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
    const wasAvailable = available.value;
    available.value = snapshot.available;
    error.value = snapshot.error;
    sampleRate.value = snapshot.sample_rate;
    // 实时不可用时快照里的 `playing` / 位置恒为 0：那是后端的兜底值，不是离线试听的
    // 真实状态。预览模式下这两个值归 `<audio>` 的 rAF 时钟管；照抄会把「正在试听」
    // 每秒刷成暂停（新的恢复轮询每 4 秒来一次，症状会非常明显）。
    if (snapshot.available) {
      playing.value = snapshot.playing;
      positionFrames.value = snapshot.position_frames;
      positionRate = snapshot.sample_rate || FALLBACK_SAMPLE_RATE;
      loop.value = snapshot.loop;
    } else if (wasAvailable) {
      // 实时流刚掉：位置留在掉线前，界面先停住，等用户重试或自动恢复。
      playing.value = false;
    }
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
    positionRate = PREVIEW_SAMPLE_RATE;
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

  /** 工程变了：缓存的离线试听不再代表当前声音，连同播放器一起扔掉。 */
  function invalidatePreview(): void {
    teardownPreview();
    previewBlob = null;
  }

  async function startPreview(): Promise<void> {
    if (previewPreparing.value) return;
    // 一旦走离线试听，就不再算「启动宽限期」：后面的暂停 / 定位必须按预览模式处理。
    bootGrace.value = false;
    const my = ++token;
    // 实时位置的帧数是按**设备采样率**记的；预览固定 44.1 kHz，先按秒换算再落到预览帧。
    const resumeAt = Math.round((positionFrames.value / positionRate) * PREVIEW_SAMPLE_RATE);
    previewPreparing.value = true;
    try {
      let blob = previewBlob;
      if (!blob) {
        blob = await api.renderPreview(PREVIEW_SAMPLE_RATE);
        if (my !== token) return; // 期间被停止/取消
        previewBlob = blob;
      }
      if (my !== token) return;
      if (!url) url = URL.createObjectURL(blob);
      const el = new Audio(url);
      audio = el;
      el.loop = loop.value;
      el.onended = () => {
        playing.value = false;
        positionFrames.value = 0;
        teardownPreview();
      };
      // 先摆到目标位置再播：否则从 0 开始会先响一下再跳。
      if (resumeAt > 0) el.currentTime = resumeAt / PREVIEW_SAMPLE_RATE;
      await el.play();
      if (my !== token) {
        // 渲染期间用户按了暂停 / 停止：别把声音放出来。
        stopPreviewClock();
        el.pause();
        teardownPreview();
        return;
      }
      playing.value = true;
      startPreviewClock();
    } catch (err) {
      if (my !== token) return;
      teardownPreview();
      playing.value = false;
      layout.notify(i18n.t("error.audition", { error: String(err) }));
    } finally {
      if (my === token) previewPreparing.value = false;
    }
  }

  // 设备插回后实时会自动恢复（音频线程每秒轮询重建）。此时必须把离线试听收干净，
  // 否则 <audio> 与实时流会同时出声、并且 rAF 时钟会把播放位置覆盖掉。
  // `mode` 目前只由 `available` 决定（只有 realtime / preview），所以该条件等价于
  // 「回到 realtime」；写成 `next !== "preview"` 是为了将来再引入别的模式时依旧能收干净。
  // `teardownPreview()` 幂等（无试听时只是自增 token 并空转），没有试听时触发也安全。
  watch(mode, (next, prev) => {
    if (prev === "preview" && next !== "preview") {
      previewPreparing.value = false;
      teardownPreview();
      // 离线期间设的范围 / 循环开关没送到引擎（命令被拒）→ 回到实时后补推一次。
      void pushRange();
    }
  });

  // 编辑过工程之后，缓存里的试听已经不是「现在的声音」了：对象被整体换掉（打开工程 /
  // 应用 AI 结果）或第一次变脏时都作废。
  watch(
    () => project.project,
    () => invalidatePreview(),
  );
  watch(
    () => project.dirty,
    (dirty) => {
      if (dirty) invalidatePreview();
    },
  );

  // ---- 对外动作 ----

  /** 订阅传输状态（幂等）。 */
  async function init(): Promise<void> {
    if (unlisten) return;
    if (graceTimer === null) {
      graceTimer = window.setTimeout(() => {
        graceTimer = null;
        bootGrace.value = false;
      }, 8000);
    }
    // **先订阅、再取状态**：后端只在「状态变化」时广播，先取后订会漏掉取与订之间
    // 的那一次变化——最常见的就是启动时音频宿主还没建好（available=false），宿主
    // 建好后的那一条广播正好落进缝里，于是界面永远停在「离线试听」，播放走整首
    // 离线渲染（非常卡），实际引擎却是好的。
    unlisten = await api.onTransportEvent(applySnapshot);
    try {
      // 订阅后再取一次当前值：把上面那个窗口里漏掉的变化补回来。
      applySnapshot(await api.transportStatus());
    } catch (err) {
      available.value = false;
      error.value = String(err);
    }
    // 兜底：预览模式下每 4 秒问一次（事件万一再丢，也能自己回到实时）。
    if (recoveryTimer === null) {
      recoveryTimer = window.setInterval(() => {
        if (mode.value === "preview") void refresh();
      }, 4000);
    }
    ready.value = true;
  }

  /** 开始播放。 */
  async function play(): Promise<void> {
    if (playing.value || previewPreparing.value || starting.value) return;
    const my = ++playToken;
    if (mode.value === "realtime") {
      starting.value = true;
      try {
        // 启动后最初十几秒音频宿主可能还在建流（大工程要解码素材）。这时按播放不该
        // 立刻掉进「离线渲染整首」（分钟级、看起来就是卡死），先等它几拍再说。
        for (let attempt = 0; attempt < 8; attempt++) {
          if (my !== playToken) return;
          try {
            await api.transportPlay();
            if (my !== playToken) return;
            playing.value = true; // 乐观：随后的快照会纠正
            return;
          } catch (err) {
            error.value = String(err);
            const booting = Date.now() - startedAt < 15_000;
            if (!booting || attempt === 7) {
              // 真的不可用 → 降级到离线试听，并稍后自动再探测。
              available.value = false;
              layout.notify(i18n.t("bottom.audioFallback", { error: String(err) }));
              if (retryTimer !== null) window.clearTimeout(retryTimer);
              retryTimer = window.setTimeout(() => {
                retryTimer = null;
                void refresh();
              }, 1500);
              break;
            }
            await new Promise((resolve) => window.setTimeout(resolve, 700));
            if (my !== playToken) return;
            await refresh();
          }
        }
      } finally {
        if (my === playToken) starting.value = false;
      }
    }
    if (my !== playToken) return;
    await startPreview();
  }

  /** 暂停（保留当前位置）；渲染/等宿主期间按暂停 = 取消这一次。 */
  async function pause(): Promise<void> {
    // 录制中按暂停 = 结束这次录音：MIDI take 的时间轴不能停在原地又继续走墙钟，
    // 音频采集也不该在暂停后继续把静音写进同一个 WAV。
    if (recording.value) {
      await stopRecording();
      return;
    }
    if (starting.value) {
      playToken++;
      starting.value = false;
      // 上一条 `Play` 可能刚被音频线程收下（命令已发出、还没走到 `playing = true`），
      // 补一条暂停把它收回来；失败无所谓，引擎本来就没起来。
      if (mode.value === "realtime") void api.transportPause().catch(() => {});
      return;
    }
    if (previewPreparing.value) {
      token++;
      previewPreparing.value = false;
      playing.value = false;
      return;
    }
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
    // 停止键也负责收尾：录制中直接回起点会把还没落盘的 take 丢掉。
    if (recording.value) await stopRecording();
    if (starting.value) {
      playToken++;
      starting.value = false;
      if (mode.value === "realtime") void api.transportStop().catch(() => {});
    }
    if (mode.value === "realtime") {
      playing.value = false;
      positionFrames.value = 0;
      await api.transportStop();
      return;
    }
    previewPreparing.value = false;
    teardownPreview();
    playing.value = false;
    positionFrames.value = 0;
  }

  /** 播放 / 暂停切换。（离线试听渲染 / 等宿主启动时按一下 = 取消，而不是再起一次。） */
  async function toggle(): Promise<void> {
    // 录制中空格 = 结束录音；否则会变成「再起一次离线渲染」。
    if (recording.value) {
      await stopRecording();
      return;
    }
    if (playing.value || previewPreparing.value || starting.value) await pause();
    else await play();
  }

  /** 定位到指定帧。 */
  async function seekFrames(frame: number): Promise<void> {
    // 录制中定位 = 结束这次录音，再按新位置走：否则 take 里会出现一段跨越两个位置的时间轴。
    if (recording.value) await stopRecording();
    const target = Math.max(0, Math.round(frame));
    positionRate =
      mode.value === "preview" ? PREVIEW_SAMPLE_RATE : sampleRate.value || FALLBACK_SAMPLE_RATE;
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
    if (recoveryTimer !== null) {
      window.clearInterval(recoveryTimer);
      recoveryTimer = null;
    }
    if (retryTimer !== null) {
      window.clearTimeout(retryTimer);
      retryTimer = null;
    }
    if (graceTimer !== null) {
      window.clearTimeout(graceTimer);
      graceTimer = null;
    }
    invalidatePreview();
    previewPreparing.value = false;
    playToken++;
    starting.value = false;
    recording.value = false;
    recordingMode.value = null;
    midiTake = null;
  }

  // ---- 录音（C2） ----

  /** 是否正在录音。 */
  const recording = ref(false);
  /** 当前选择的录音模式（界面上的「音频 / MIDI」小开关）。 */
  const recordMode = ref<RecordMode>(initialRecordMode());
  /**
   * **正在进行的这次**录音的模式（开始时快照）。
   *
   * 与 `recordMode` 分开：录制中允许用户预先切下一次的模式，但这一次仍按开始时的模式收尾。
   * `null` = 没在录音。
   */
  const recordingMode = ref<RecordMode | null>(null);
  /** 可选的输入设备列表。 */
  const inputDevices = ref<InputDeviceInfo[]>([]);
  /** 当前选中的输入设备名（null = 未选）。 */
  const inputDevice = ref<string | null>(null);
  /** 输入电平表最近一帧的峰值（0..1）。 */
  const inputLevelPeak = ref(0);

  // ---- MIDI 录音（前端采集，落盘走后端 `add_midi_clip`） ----

  /** 一次正在进行的 MIDI take。 */
  interface MidiTake {
    /** 目标轨道（开始时选中，录制中不再改变）。 */
    track: number;
    /** 录音起点（拍，已吸附到网格）。 */
    start: number;
    /** 开始时的墙钟（毫秒）：播放器没起来时用它当时间轴。 */
    startedAtMs: number;
    /** 起点对应的工程秒数（用于墙钟 → 拍的换算）。 */
    startSeconds: number;
    /** 已经写进 take 的音符（相对起点）。 */
    notes: { pitch: number; start: number; duration: number; velocity: number; channel: number }[];
    /** 还按着没松开的音高：`pitch → 起点 / 力度 / 通道`。 */
    held: Map<number, { start: number; velocity: number; channel: number }>;
    /** 已写下最后一个音的结束位置（相对起点，拍）。 */
    end: number;
    /** 单调游标：循环回绕或快照滞后时不把音符写回过去。 */
    cursor: number;
  }

  let midiTake: MidiTake | null = null;

  /**
   * 当前 take 的播放位置（相对起点，拍）。
   *
   * 实时播放时用走带位置（与听到的完全一致）；离线 / 播放器还没起来时退回墙钟，
   * 免得所有音符都挤在 0 拍上。两种情况下都保持单调递增。
   */
  function midiTakePosition(): number {
    if (!midiTake) return 0;
    let pos: number;
    if (playing.value) {
      pos = Math.max(0, positionBeats.value - midiTake.start);
    } else {
      const elapsed = (performance.now() - midiTake.startedAtMs) / 1000;
      pos = Math.max(
        0,
        secondsToBeats(midiTake.startSeconds + elapsed, transportSettings.value) - midiTake.start,
      );
    }
    pos = Math.max(pos, midiTake.cursor);
    midiTake.cursor = pos;
    return pos;
  }

  /** 记下一个 MIDI 音符按下（来自虚拟键盘 / 硬件输入）。 */
  function captureMidiNoteOn(note: number, velocity: number, channel = 0): void {
    if (!midiTake || !recording.value || recordingMode.value !== "midi") return;
    const at = midiTakePosition();
    // 同一音高重复按下：先把上一颗没松开的收尾，避免这个音彻底丢掉。
    closeMidiNote(note, at);
    midiTake.held.set(note, { start: at, velocity: Math.min(127, Math.max(1, velocity)), channel });
  }

  /** 记下一个 MIDI 音符松开。 */
  function captureMidiNoteOff(note: number): void {
    if (!midiTake) return;
    closeMidiNote(note, midiTakePosition());
  }

  /** 把某个按住的音写成一颗完整音符。 */
  function closeMidiNote(note: number, at: number): void {
    const take = midiTake;
    const head = take?.held.get(note);
    if (!take || !head) return;
    take.held.delete(note);
    // 极短的点击也留一个可编辑的最小时值（1/32 拍），否则会变成 0 时值音符。
    const duration = Math.max(1 / 32, at - head.start);
    take.notes.push({
      pitch: note,
      start: head.start,
      duration,
      velocity: head.velocity,
      channel: head.channel,
    });
    take.end = Math.max(take.end, head.start + duration);
  }

  /** 切换录音模式（录制中只改「下一次」，不影响正在进行的这次）。 */
  function setRecordMode(mode: RecordMode): void {
    if (recording.value) return;
    recordMode.value = mode;
    try {
      localStorage.setItem(RECORD_MODE_KEY, mode);
    } catch {
      // 存不住也只影响下次启动的默认值。
    }
  }

  // 播放被外部停掉（MCP / 音频设备掉线 / 别的窗口的命令）时，录音也必须收尾，
  // 不能让 take 继续按墙钟往前走。
  watch(playing, (isPlaying) => {
    if (!isPlaying && recording.value) void stopRecording();
  });

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

  /**
   * 开始录音。
   *
   * - **音频**：后端打开输入流、采集 PCM，并从播放头走带；
   * - **MIDI**：先把播放头吸到 `startBeat`，再走带；音符由 MIDI 输入侧实时记入本 store
   *   （见 `noteInput` / `captureMidiNoteOn`），停止时一次性落成「片段 + 音符」。
   */
  async function startRecording(track: number, startBeat: number): Promise<void> {
    if (recordMode.value === "midi") {
      await startMidiRecording(track, startBeat);
      return;
    }
    try {
      await api.recordStart(track, startBeat, inputDevice.value);
      recording.value = true;
      recordingMode.value = "audio";
      playing.value = true;
    } catch (err) {
      layout.notify(String(err));
    }
  }

  /** 开始一次 MIDI take。 */
  async function startMidiRecording(track: number, startBeat: number): Promise<void> {
    if (recording.value) return;
    midiTake = {
      track,
      start: startBeat,
      startedAtMs: performance.now(),
      startSeconds: beatsToSeconds(startBeat, transportSettings.value),
      notes: [],
      held: new Map(),
      end: 0,
      cursor: 0,
    };
    // 先把播放头摆到录音起点：这样听到的伴奏与写进片段的音符是同一个时间原点。
    try {
      await seekBeats(startBeat);
    } catch (err) {
      midiTake = null;
      layout.notify(i18n.t("error.record", { error: String(err) }));
      return;
    }
    midiTake.startedAtMs = performance.now();
    recording.value = true;
    recordingMode.value = "midi";
    // 实时不可用时**不**起离线试听：整首渲染要几分钟，期间根本没有可听的时间轴；
    // 此时按墙钟记谱更实用（见 `midiTakePosition`）。
    if (mode.value === "realtime") await play();
  }

  /**
   * 结束当前录音。
   *
   * 音频走原有的「停带 → 写 WAV → 套用工程」；MIDI 则把 take 里的音符封成一条 MIDI 片段，
   * 片段与全部音符由后端的一次原子编辑写入（撤销一次就整段消失）。
   */
  async function stopRecording(): Promise<void> {
    if (!recording.value) return;
    if (recordingMode.value === "midi") {
      await stopMidiRecording();
      return;
    }
    recording.value = false;
    recordingMode.value = null;
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

  /** 收尾一次 MIDI take，并在有音符时落成片段。 */
  async function stopMidiRecording(): Promise<void> {
    const take = midiTake;
    const wasPlaying = playing.value;
    recording.value = false;
    recordingMode.value = null;
    midiTake = null;
    if (!take) return;

    // 停带后再结算：松手位置 / 最后按住的音都以停止瞬间为准。离线渲染 / 起流中的等待
    // 也要一并取消，否则「停止录音」之后伴奏还会自己响起来。
    if (wasPlaying || previewPreparing.value || starting.value) await pause();
    const stopAt = Math.max(
      0,
      wasPlaying
        ? positionBeats.value - take.start
        : secondsToBeats(
            take.startSeconds + (performance.now() - take.startedAtMs) / 1000,
            transportSettings.value,
          ) - take.start,
    );
    take.cursor = Math.max(take.cursor, stopAt);
    for (const note of [...take.held.keys()]) closeMidiNoteIn(take, note, take.cursor);

    if (take.notes.length === 0) {
      layout.notify(i18n.t("bottom.midiRecordEmpty"));
      return;
    }

    const end = Math.max(take.end, take.cursor);
    const length = Math.max(1, Math.ceil(end - 1e-9));
    const notes: Note[] = take.notes
      .slice()
      .sort((a, b) => a.start - b.start || a.pitch - b.pitch)
      .map((n) => ({
        pitch: n.pitch,
        start: n.start,
        duration: n.duration,
        velocity: n.velocity,
        volume: 127,
        ...(n.channel ? { channel: n.channel } : {}),
      }));

    try {
      const next = await api.addMidiClip(take.track, take.start, length, notes);
      await project.applyRecorded(next);
      // 选中刚录下的片段：用户按完录音通常就想看 / 改这一条（在编排视图里也会被框选中）。
      const recordedTrack = next.tracks.find((t) => t.id === take.track);
      const created = (recordedTrack?.clips ?? [])
        .filter((c) => Math.abs(c.start - take.start) < 1e-9)
        .sort((a, b) => b.id - a.id)[0];
      if (created) selection.selectClip(take.track, created.id);
      layout.notify(i18n.t("bottom.midiRecordDone", { count: notes.length }));
    } catch (err) {
      layout.notify(i18n.t("error.record", { error: String(err) }));
    }
  }

  /** `stopMidiRecording` 用的纯函数版本：take 已经脱离 store，避免再走 `midiTake`。 */
  function closeMidiNoteIn(take: MidiTake, note: number, at: number): void {
    const head = take.held.get(note);
    if (!head) return;
    take.held.delete(note);
    const duration = Math.max(1 / 32, at - head.start);
    take.notes.push({
      pitch: note,
      start: head.start,
      duration,
      velocity: head.velocity,
      channel: head.channel,
    });
    take.end = Math.max(take.end, head.start + duration);
  }

  /**
   * MIDI 输入侧的音符事件入口（虚拟键盘 / 硬件键盘都走这里）。
   *
   * 录音时写入 take；不录音时什么也不做——试听由 MIDI store 自己调 `audition_note`，
   * 传输层不负责发声。
   */
  function noteInput(note: number, velocity: number, on: boolean, channel = 0): void {
    if (!recording.value || recordingMode.value !== "midi") return;
    if (on) captureMidiNoteOn(note, velocity, channel);
    else captureMidiNoteOff(note);
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
    previewPreparing,
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
    recordMode,
    recordingMode,
    setRecordMode,
    noteInput,
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
