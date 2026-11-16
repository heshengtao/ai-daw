/**
 * 云端音乐生成：供应商列表 + 当前选中的服务 + 一次生成任务的状态。
 *
 * 供应商配置存在后端（`cloud-music.json`）；这里只缓存一份给界面用。选中的服务 id
 * 存在 localStorage，跨会话记住——与「写歌」的模型选择同一个体验。
 */
import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  cloudMusicBusy,
  cloudMusicCancel,
  cloudMusicConfig,
  cloudMusicGenerate,
  cloudMusicPresets,
  onCloudMusicProgress,
} from "@/api/cloudMusic";
import type {
  CloudMusicPreset,
  CloudMusicProgress,
  CloudMusicProvider,
  CloudMusicRequest,
  CloudMusicResult,
} from "@/api/types";

const KEY = "ai-daw-cloud-music-provider";

function loadSelected(): string {
  try {
    return localStorage.getItem(KEY) ?? "";
  } catch {
    return "";
  }
}

export const useCloudMusicStore = defineStore("cloudMusic", () => {
  const providers = ref<CloudMusicProvider[]>([]);
  const presets = ref<CloudMusicPreset[]>([]);
  const loaded = ref(false);
  const loading = ref(false);
  const loadError = ref<string | null>(null);

  const selectedId = ref(loadSelected());
  const running = ref(false);
  const progress = ref<CloudMusicProgress | null>(null);
  const result = ref<CloudMusicResult | null>(null);
  const error = ref<string | null>(null);

  const enabledProviders = computed(() => providers.value.filter((p) => p.enabled));
  const selected = computed(
    () => providers.value.find((p) => p.id === selectedId.value) ?? null,
  );
  const progressPct = computed<number | null>(() => {
    const current = progress.value;
    if (!current || current.state === "preparing") return null;
    if (current.progress <= 0) return null;
    return Math.min(100, Math.max(0, Math.round(current.progress * 100)));
  });

  /** 选中一个服务；空 id = 清空。 */
  function select(id: string): void {
    selectedId.value = id;
    try {
      localStorage.setItem(KEY, id);
    } catch {
      // 记不住不影响本次会话。
    }
  }

  /** 拉配置；`force` 用于设置页保存后强刷。 */
  async function load(force = false): Promise<void> {
    if (loading.value) return;
    if (loaded.value && !force) return;
    loading.value = true;
    loadError.value = null;
    try {
      providers.value = (await cloudMusicConfig()).providers;
      loaded.value = true;
      if (!providers.value.some((p) => p.id === selectedId.value)) {
        select(enabledProviders.value[0]?.id ?? "");
      }
    } catch (err) {
      loadError.value = err instanceof Error ? err.message : String(err);
    } finally {
      loading.value = false;
    }
  }

  /** 预设只需要拉一次。 */
  async function loadPresets(): Promise<void> {
    if (presets.value.length) return;
    try {
      presets.value = await cloudMusicPresets();
    } catch {
      presets.value = [];
    }
  }

  let stopProgress: (() => void) | null = null;
  let subscribing: Promise<void> | null = null;

  /** 订阅进度事件；重复调用只挂一次。 */
  function watch(): void {
    if (stopProgress || subscribing) return;
    subscribing = onCloudMusicProgress((payload) => {
      progress.value = payload;
      if (payload.state === "error" && payload.message) error.value = payload.message;
    })
      .then((off) => {
        stopProgress = off;
        subscribing = null;
      })
      .catch(() => {
        subscribing = null;
      });
  }

  function unwatch(): void {
    stopProgress?.();
    stopProgress = null;
  }

  async function refreshBusy(): Promise<void> {
    try {
      running.value = await cloudMusicBusy();
    } catch {
      // 旧版桌面端 / 浏览器没有这条命令：保持当前状态。
    }
  }

  async function generate(request: CloudMusicRequest): Promise<CloudMusicResult | null> {
    if (running.value) return null;
    running.value = true;
    error.value = null;
    result.value = null;
    progress.value = { state: "preparing", message: null, progress: 0, elapsed: 0 };
    select(request.provider_id);
    try {
      const created = await cloudMusicGenerate(request);
      result.value = created;
      return created;
    } catch (err) {
      const text = err instanceof Error ? err.message : String(err);
      if (!text.includes("已取消")) error.value = text;
      return null;
    } finally {
      running.value = false;
    }
  }

  async function cancel(): Promise<void> {
    try {
      await cloudMusicCancel();
    } catch {
      // 状态事件会纠正。
    }
  }

  return {
    providers,
    presets,
    loaded,
    loading,
    loadError,
    selectedId,
    selected,
    enabledProviders,
    running,
    progress,
    progressPct,
    result,
    error,
    select,
    load,
    loadPresets,
    watch,
    unwatch,
    refreshBusy,
    generate,
    cancel,
  };
});
