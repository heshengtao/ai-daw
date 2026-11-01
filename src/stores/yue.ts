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
  yueSetCudaPerf,
} from "@/api/tauri";
import type { YueComposeArgs, YueEngineStatus, YueJobStatus } from "@/api/types";

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
    seconds: null,
  });
  const error = ref<string | null>(null);

  const ready = computed(() => engine.value?.state === "ready");
  const starting = computed(() => engine.value?.state === "starting");
  const busy = computed(() => job.value.state === "running");

  async function load(): Promise<void> {
    try {
      engine.value = await yueEngineStatus();
      cudaPerf.value = await yueCudaPerf();
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
    job.value = { state: "running", elapsed: 0, message: null, audio: null, score: null, seconds: null };
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
    job.value = { state: "idle", elapsed: 0, message: null, audio: null, score: null, seconds: null };
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
    ready,
    starting,
    busy,
    load,
    setCudaPerf,
    start,
    stop,
    compose,
    cancel,
    resetJob,
    watch,
  };
});
