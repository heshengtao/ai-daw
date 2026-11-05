/**
 * 右键菜单里「一点就跑」的 AI 任务：音频转 MIDI / 降噪 / 音源分离 / 节拍检测 / 母带。
 *
 * 只做三件事：拦并发、缺权重时把人带到下载管理器、跑完弹一次摘要。真正需要填参数的功能
 * （写歌 / 翻唱 / 歌声合成 / 音色替换）不在这里——那些由 `aiIntent` 打开对应子页。
 *
 * 组件卸载后就不再弹通知（订阅在 `onUnmounted` 里断掉）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { aiStart, aiStatus, onAiEvent } from "@/api/tauri";
import type { AiStatus } from "@/api/types";
import { useDownloadsStore } from "@/stores/downloads";
import { useLayoutStore } from "@/stores/layout";
import { useModelsStore } from "@/stores/models";
import { useI18n } from "@/i18n";
import { modelName } from "@/utils/models";

/** `ai_start` 认的任务类型。 */
export type QuickAiKind =
  | "to_midi"
  | "enhance"
  | "enhance_dsp"
  | "separate"
  | "detect_tempo"
  | "master";

/** 需要本地权重的两类任务：`feature` 与菜单文案的对应关系。 */
const MODEL_FEATURE: Partial<Record<QuickAiKind, string>> = {
  separate: "separate",
  detect_tempo: "detect_tempo",
};

export function useAiQuickActions() {
  const layout = useLayoutStore();
  const models = useModelsStore();
  const downloads = useDownloadsStore();
  const i18n = useI18n();

  const status = ref<AiStatus>({ running: false, kind: null, progress: 0, message: null });
  let stop: (() => void) | null = null;
  let disposed = false;

  onMounted(() => {
    void aiStatus()
      .then((s) => {
        if (!disposed) status.value = s;
      })
      .catch(() => {});
    void onAiEvent((s) => {
      status.value = s;
    }).then((off) => {
      if (disposed) off();
      else stop = off;
    });
  });

  onUnmounted(() => {
    disposed = true;
    stop?.();
    stop = null;
  });

  /** 任务结束（含失败 / 取消）时弹一次摘要，和「调音台」面板的行为一致。 */
  watch(
    () => status.value.running,
    (running, wasRunning) => {
      if (wasRunning && !running && status.value.message) layout.notify(status.value.message);
    },
  );

  /**
   * 确保某个 feature 的权重就位；不在位就打开下载管理器并返回 `null`。
   *
   * 不在这里擅自开始下载：这些模型动辄几百 MB 到几 GB，确认框留给管理器。
   */
  async function ensureModel(feature: string): Promise<string | null> {
    if (models.catalog.length === 0) await models.load();
    const candidates = models.catalog.filter((m) => m.feature === feature);
    const installed = candidates.find((m) => m.installed);
    if (installed) return installed.id;
    const wanted = candidates[0];
    if (!wanted) {
      layout.notify(i18n.t("menu.modelUnavailable", { feature }));
      return null;
    }
    layout.notify(i18n.t("menu.needModel", { name: modelName(wanted) }));
    downloads.jumpTo("model", wanted.id);
    return null;
  }

  /**
   * 跑一个任务。`trackId` 必须存在；`clipId` 只在音频类任务上有效——后端按它选中**这一块**
   * 音频，而不是轨道上的第一块。
   */
  async function run(
    kind: QuickAiKind,
    trackId: number,
    clipId?: number | null,
  ): Promise<void> {
    if (status.value.running) {
      layout.notify(i18n.t("menu.aiBusy"));
      return;
    }
    let modelId: string | null = null;
    const feature = MODEL_FEATURE[kind];
    if (feature) {
      modelId = await ensureModel(feature);
      if (modelId === null) return;
    }
    try {
      await aiStart(kind, trackId, undefined, modelId ?? undefined, clipId ?? undefined);
      layout.notify(i18n.t("menu.aiStarted"));
    } catch (err) {
      layout.notify(String(err));
    }
  }

  return {
    status,
    busy: computed(() => status.value.running),
    run,
  };
}
