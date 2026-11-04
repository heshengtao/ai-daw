import { computed, ref } from "vue";
import type { ModelStatus } from "@/api/types";
import { useI18n } from "@/i18n";
import { useDownloadsStore } from "@/stores/downloads";
import { useLayoutStore } from "@/stores/layout";
import { humanSize, missingBytes, modelName } from "@/utils/models";

/**
 * 「缺权重 → 应用内 MD3 确认框 → 真正开始下载」这一套，所有 AI 功能共用。
 *
 * 以前每个面板各自 `pendingModel` + `promptOpen` + `downloadMessage`，行为一致却抄了
 * 好几遍；翻唱的「转谱 / 抓歌词」也用同一套，所以收到这里——**一次确认就既跳转又下载**，
 * 不会出现「跳到管理器却什么都不下」的悬空状态。
 *
 * 两种确认方式：
 * - `confirm()`   —— 跳到「管理器 → 可下载 → 模型」并开始下载（默认）；
 * - `background()`—— **不跳转**，留在当前面板继续手头的事，进度看页顶的下载条。
 *
 * 用法：
 * ```ts
 * const { open, message, require, confirm, background } = useModelDownloadPrompt();
 * if (!require(model)) return;   // 没装 → 弹框并返回；装了 → 继续跑
 * await doTheWork();
 * ```
 * 模板里挂一个 `ConfirmDialog`：
 * ```html
 * <ConfirmDialog v-model:open="open" :title="…" :message="message"
 *   :confirm-label="…" :extra-label="i18n.t('setup.background')"
 *   @confirm="confirm" @extra="background" @cancel="cancel" />
 * ```
 */
export function useModelDownloadPrompt() {
  const downloads = useDownloadsStore();
  const layout = useLayoutStore();
  const i18n = useI18n();

  /** 待下载的那一条（确认框关闭后仍留着，用于文案）。 */
  const pending = ref<ModelStatus | null>(null);
  /** 确认框是否打开。 */
  const open = ref(false);

  /** 确认框正文：名称 + 还差多少体积 + 「可以留在本页下」。 */
  const message = computed(() => {
    const model = pending.value;
    return model
      ? i18n.t("dialog.downloadModelMessage", {
          name: modelName(model),
          size: humanSize(missingBytes(model)),
        })
      : "";
  });

  /**
   * 权重就位返回 `true`（调用方直接开跑）；没装则弹确认框并返回 `false`。
   * 清单里根本没有这一条时也返回 `false`（调用方自己决定要不要退回管理器）。
   */
  function require(model: ModelStatus | null): boolean {
    if (!model) return false;
    if (model.installed) return true;
    pending.value = model;
    open.value = true;
    return false;
  }

  /** 把模型收成队列任务。 */
  function taskOf(model: ModelStatus) {
    return {
      section: "model" as const,
      id: model.id,
      label: modelName(model),
      bytes: missingBytes(model),
    };
  }

  /** 另一个下载正在跑：别静默失败，说一句「已排队不了」的原因。 */
  function reportBusy(): void {
    if (downloads.running) layout.notify(i18n.t("setup.busyOther"));
  }

  /** 确认：跳到「管理器 → 可下载 → 模型」，并立刻开始下载。 */
  async function confirm(): Promise<void> {
    const model = pending.value;
    open.value = false;
    if (!model) return;
    const ok = await downloads.run([taskOf(model)], { jump: true });
    if (!ok) reportBusy();
  }

  /** 「在后台下载」：不跳转，进度留在本页顶部的下载条上。 */
  async function background(): Promise<void> {
    const model = pending.value;
    open.value = false;
    if (!model) return;
    const ok = await downloads.run([taskOf(model)], { jump: false });
    if (!ok) reportBusy();
  }

  /** 取消：什么都不下，留在当前面板（下次点按钮还会再问）。 */
  function cancel(): void {
    open.value = false;
    pending.value = null;
  }

  /** 失败后「重试剩余」（当前这类提示只有一项）。 */
  async function retry(): Promise<void> {
    await downloads.retry({ jump: true });
  }

  return {
    open,
    pending,
    message,
    running: computed(() => downloads.running),
    failure: computed(() => downloads.failure),
    require,
    confirm,
    background,
    cancel,
    retry,
  };
}
