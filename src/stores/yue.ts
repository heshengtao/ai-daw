import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  onYueEngineEvent,
  onYueJobEvent,
  yueCompose,
  yueComposeCancel,
  yueCudaPerf,
  yueEngineStart,
  yueEngineStatus,
  yueEngineStop,
  yueLora,
  yueSetCudaPerf,
  yueSetLora,
  yueLyrics,
  yueLyricsCancel,
  yueTranscribe,
  yueTranscribeCancel,
} from "@/api/tauri";
import type {
  LyricsArgs,
  LyricsOutcome,
  TranscribeArgs,
  TranscribeOutcome,
  YueComposeArgs,
  YueEngineStatus,
  YueJobStatus,
  YueLora,
} from "@/api/types";

/**
 * YuE2 本地写歌：引擎子进程（audio.cpp 的 `audiocpp_server`）+ 一次生成任务。
 *
 * 两段状态是分开的：
 * - **引擎**（`engine`）分钟级地加载几 GB 权重，只有 `ready` 之后才能提交任务；
 * - **任务**（`job`）提交后秒级返回，实际推理在后台跑，完成时后端直接把音频轨加进工程。
 *
 * 进度分别走 `engine://yue-engine` / `engine://yue-job`，与其它 AI 任务同一套事件风格。
 */
export const useYueStore = defineStore("yue", () => {
  const engine = ref<YueEngineStatus | null>(null);
  const cudaPerf = ref(true);
  const job = ref<YueJobStatus>({
    state: "idle",
    elapsed: 0,
    message: null,
    audio: null,
    score: null,
    semantic: null,
    record: null,
    seconds: null,
  });
  const error = ref<string | null>(null);
  /** LoRA（`ar` 作曲 / `nar` 音色）；改完要重启引擎才生效。 */
  const lora = ref<YueLora>({ ar_path: "", ar_scale: 1, nar_path: "", nar_scale: 1 });
  /** 转谱（SheetSage2）：分钟级，跑的时候写歌引擎会被停掉。 */
  const transcribing = ref(false);
  const transcribeError = ref<string | null>(null);
  /** 抓歌词（ASR）：分钟级，与转谱共用互斥锁。 */
  const lyricizing = ref(false);
  const lyricError = ref<string | null>(null);

  const ready = computed(() => engine.value?.state === "ready");
  const starting = computed(() => engine.value?.state === "starting");
  const busy = computed(() => job.value.state === "running");

  async function load(): Promise<void> {
    try {
      engine.value = await yueEngineStatus();
      cudaPerf.value = await yueCudaPerf();
      lora.value = await yueLora();
    } catch (err) {
      error.value = String(err);
    }
  }

  async function setCudaPerf(enabled: boolean): Promise<void> {
    cudaPerf.value = enabled;
    try {
      await yueSetCudaPerf(enabled);
    } catch (err) {
      error.value = String(err);
    }
  }

  async function start(model: string | null = null): Promise<void> {
    error.value = null;
    try {
      await yueEngineStart(model);
    } catch (err) {
      error.value = String(err);
    }
    await load();
  }

  async function stop(): Promise<void> {
    error.value = null;
    try {
      await yueEngineStop();
    } catch (err) {
      error.value = String(err);
    }
    await load();
  }

  async function compose(args: YueComposeArgs): Promise<void> {
    error.value = null;
    job.value = {
      state: "running",
      elapsed: 0,
      message: null,
      audio: null,
      score: null,
      semantic: null,
      record: null,
      seconds: null,
    };
    try {
      await yueCompose(args);
    } catch (err) {
      error.value = String(err);
      job.value = { ...job.value, state: "failed", message: String(err) };
    }
  }

  async function cancel(): Promise<void> {
    try {
      await yueComposeCancel();
    } catch {
      // 状态事件会纠正。
    }
  }

  function resetJob(): void {
    job.value = {
      state: "idle",
      elapsed: 0,
      message: null,
      audio: null,
      score: null,
      semantic: null,
      record: null,
      seconds: null,
    };
  }

  /** 写 LoRA 设置（下次启动引擎生效）。 */
  async function saveLora(next: YueLora): Promise<void> {
    lora.value = next;
    try {
      await yueSetLora(next);
    } catch (err) {
      error.value = String(err);
    }
  }

  /**
   * 录音转谱（SheetSage2）：把选中轨道的音频变成 ABC 谱。
   *
   * 会先停掉写歌引擎腾显存——转谱与写歌不并行，顺序永远是「先转谱，再按谱重唱」。
   */
  async function transcribe(args: TranscribeArgs): Promise<TranscribeOutcome | null> {
    transcribeError.value = null;
    transcribing.value = true;
    try {
      const outcome = await yueTranscribe(args);
      // 引擎已经被停掉了，状态要重新读一次（界面上的「就绪」别骗人）。
      await load();
      return outcome;
    } catch (err) {
      transcribeError.value = String(err);
      return null;
    } finally {
      transcribing.value = false;
    }
  }

  async function cancelTranscribe(): Promise<void> {
    try {
      await yueTranscribeCancel();
    } catch {
      // 结束后状态会自己回来。
    }
  }

  /**
   * 抓歌词（ASR）：参考音频 → 歌词文本。
   *
   * 和转谱共用同一把互斥锁（都会起一次性引擎进程），同样会先停掉写歌引擎。
   * 结果是「初稿」：调用方直接填进歌词框，用户随时能改。
   */
  async function lyrics(args: LyricsArgs): Promise<LyricsOutcome | null> {
    lyricError.value = null;
    lyricizing.value = true;
    try {
      const outcome = await yueLyrics(args);
      // 引擎已经被停掉了，状态要重新读一次。
      await load();
      return outcome;
    } catch (err) {
      lyricError.value = String(err);
      return null;
    } finally {
      lyricizing.value = false;
    }
  }

  async function cancelLyrics(): Promise<void> {
    try {
      await yueLyricsCancel();
    } catch {
      // 结束后状态会自己回来。
    }
  }

  /** 订阅两路事件（挂载时调用，返回取消订阅函数）。 */
  function watch(): () => void {
    let offEngine: (() => void) | null = null;
    let offJob: (() => void) | null = null;
    void onYueEngineEvent((s) => {
      engine.value = s;
    }).then((off) => {
      offEngine = off;
    });
    void onYueJobEvent((s) => {
      job.value = s;
    }).then((off) => {
      offJob = off;
    });
    return () => {
      offEngine?.();
      offJob?.();
    };
  }

  return {
    engine,
    cudaPerf,
    job,
    error,
    lora,
    transcribing,
    transcribeError,
    lyricizing,
    lyricError,
    ready,
    starting,
    busy,
    load,
    setCudaPerf,
    saveLora,
    transcribe,
    cancelTranscribe,
    lyrics,
    cancelLyrics,
    start,
    stop,
    compose,
    cancel,
    resetJob,
    watch,
  };
});
