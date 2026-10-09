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
import type { TransportSnapshot } from "@/api/types";
import { useLayoutStore } from "@/stores/layout";
import { useProjectStore } from "@/stores/project";
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

  // 乐观初值：初始化完成前先按「实时可用」处理，失败后再降级并提示。
  const available = ref(true);
  const error = ref<string | null>(null);
  const sampleRate = ref(0);
  const playing = ref(false);
  const positionFrames = ref(0);
  /** 循环播放开关（运行时状态，不进工程）。 */
  const loop = ref(false);
  const ready = ref(false);

  let unlisten: (() => void) | null = null;

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
      positionFrames.value = Math.round(audio.currentTime * PREVIEW_SAMPLE_RATE);
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
    if (prev === "preview" && next !== "preview") teardownPreview();
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

  return {
    available,
    error,
    sampleRate,
    playing,
    positionFrames,
    positionBeats,
    loop,
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
    dispose,
  };
});
