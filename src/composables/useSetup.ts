/**
 * **「缺什么就弹框，确认后跳转并真的开始下载」** —— 所有 AI 功能共用一套。
 *
 * 以前每个入口各写一遍「确认 → `openTab("manager")`」，结果是跳过去之后**什么都不下**，
 * 用户还要自己在管理器里找卡片再点一次；更糟的是下载管理器默认停在「已安装」页签，
 * 未装的条目根本不显示，看起来就是「点了没反应」。
 *
 * 这里的职责：
 * 1. 算出「还差什么」（`core/aiDeps`）；
 * 2. 弹 `SetupDialog` 让用户看一眼体积和推荐后端；
 * 3. 确认后跳管理器 → 定位到第一项 → **串行下载**（`stores/downloads`）；
 * 4. 装完自动切后端；需要重启时挂出管理器里那条常驻横幅；
 * 5. 完成后回到触发它的那个面板，并提示一句。
 *
 * 「在后台下载」走同一条链路，只是**不跳转**：进度交给页内的 `DownloadStrip`。
 */
import { ref } from "vue";
import { useDownloadsStore, type DownloadTask } from "@/stores/downloads";
import { useLayoutStore, type PaneId } from "@/stores/layout";
import { useRuntimesStore } from "@/stores/runtimes";
import { useI18n } from "@/i18n";
import { forgetSeen, hasSeen, markSeen } from "@/core/setupMemory";
import { hasDiskRoom } from "@/utils/disk";
import { humanSize } from "@/utils/models";
import { pendingDeps, type SetupDepKind, type SetupPlan } from "@/types/setup";

interface SetupOptions {
  /** 记忆键（`yue` / `singing` / `svc` / `runtime`）。 */
  feature: string;
  /** 组计划；返回 `null` 表示「没有可准备的东西」。 */
  build: () => SetupPlan | null;
  /** 下载完成之后把后端切过去（返回是否需要重启）。 */
  applyBackend?: (plan: SetupPlan) => Promise<boolean>;
  /** 全部成功后的收尾（回到原页面 / 打开子页）。 */
  done?: () => void;
}

function sectionOf(kind: SetupDepKind): DownloadTask["section"] {
  return kind === "model" ? "model" : "engine";
}

export function useSetupSession(options: SetupOptions) {
  const i18n = useI18n();
  const layout = useLayoutStore();
  const runtimes = useRuntimesStore();
  const downloads = useDownloadsStore();

  const open = ref(false);
  const plan = ref<SetupPlan | null>(null);
  const running = ref(false);
  /** 首次引导：不给「稍后」，只能去下载或离开这一页。 */
  const forced = ref(false);
  /** 上一次失败的任务（「重试剩余」用）。 */
  const failed = ref<DownloadTask[]>([]);
  /** 弹框是从哪个面板弹出来的（装完回哪儿）。 */
  let origin: PaneId = layout.activeTab;
  /** 本次「准备」成功后要执行的额外动作（例如直接开始写歌）。 */
  let onDone: (() => void) | null = null;

  /** 登记「装好之后做什么」；取消时自动丢弃。 */
  function setDone(fn: (() => void) | null): void {
    onDone = fn;
  }

  /** 缺依赖就弹框；返回是否**已经就绪**（调用方据此决定要不要继续跑）。 */
  function ensure(): boolean {
    const next = options.build();
    plan.value = next;
    if (!next) {
      open.value = false;
      return true;
    }
    origin = layout.activeTab;
    forced.value = false;
    open.value = true;
    return false;
  }

  /** 首次进入某个功能时自动弹一次（同一「功能 + 推荐后端」只弹一次）。 */
  function maybePrompt(): void {
    const next = options.build();
    if (!next) return;
    const key = next.backend?.kind ?? "model";
    if (hasSeen(options.feature, key)) return;
    plan.value = next;
    origin = layout.activeTab;
    // 首次引导不给「稍后」：功能一个字节都没装，留着也只是对着点不动的按钮。
    forced.value = true;
    open.value = true;
  }

  function cancel(): void {
    const next = plan.value;
    open.value = false;
    // 取消后别再记着「装好要做什么」：那是上一次的意图。
    onDone = null;
    // 用户看过并关了：之后不再自动糊上来（主动点按钮时仍会弹）。
    if (next) markSeen(options.feature, next.backend?.kind ?? "model");
  }

  /** 计划里的依赖 → 队列任务。 */
  function tasksOf(next: SetupPlan): DownloadTask[] {
    return pendingDeps(next).map((dep) => ({
      section: sectionOf(dep.kind),
      id: dep.id,
      label: dep.label,
      bytes: dep.bytes,
    }));
  }

  /** 跳管理器 + 串行下载 + 切后端 + 回原页面。 */
  async function run(tasks: DownloadTask[], jump = true, checkDisk = true): Promise<boolean> {
    if (tasks.length === 0) return true;

    if (checkDisk) {
      const needed = tasks.reduce((sum, task) => sum + (task.bytes ?? 0), 0);
      if (!(await hasDiskRoom(needed))) {
        layout.notify(i18n.t("setup.diskLow", { size: humanSize(needed) }));
        return false;
      }
    }

    origin = layout.activeTab;
    running.value = true;
    let ok = false;
    try {
      ok = await downloads.run(tasks, { jump });
    } finally {
      running.value = false;
    }

    if (!ok) {
      if (downloads.cancelled) return false;
      const failure = downloads.failure;
      failed.value = failure?.rest ?? [];
      layout.notify(
        i18n.t("setup.failed", {
          name: failure?.task.label ?? "",
          error: failure?.message ?? "",
        }),
      );
      return false;
    }

    failed.value = [];

    const current = plan.value;
    let restart = false;
    try {
      if (current && options.applyBackend) restart = await options.applyBackend(current);
    } catch (error) {
      layout.notify(error instanceof Error ? error.message : String(error));
    }

    // 装好了：这个功能的「已引导」记录作废（下次换机器才会重新提示）。
    forgetSeen(options.feature);

    // 还停在下载管理器才把用户带回去：中途自己切走了就别抢。
    if (jump && layout.activeTab === "manager") layout.openTab(origin);
    options.done?.();
    const after = onDone;
    onDone = null;
    after?.();

    if (restart) {
      // 用管理器那条常驻横幅（不弹框）：从 AI 页跳走 / 面板被卸载都不会丢。
      runtimes.requestRestartPrompt();
      layout.notify(i18n.t("setup.restartHint"));
    } else {
      layout.notify(i18n.t("setup.done"));
    }
    return true;
  }

  /** 用户点「下载并安装」：跳管理器并开始下载。 */
  async function confirm(jump = true): Promise<void> {
    const current = plan.value;
    if (!current) {
      open.value = false;
      return;
    }
    const tasks = tasksOf(current);
    // 空间不够就**别关弹窗**：用户还站在原地，看得到「还差多少」再决定要不要清理。
    const needed = tasks.reduce((sum, task) => sum + (task.bytes ?? 0), 0);
    if (!(await hasDiskRoom(needed))) {
      layout.notify(i18n.t("setup.diskLow", { size: humanSize(needed) }));
      return;
    }
    open.value = false;
    await run(tasks, jump, false);
  }

  /** 「在后台下载」：不跳管理器，进度留在本页。 */
  async function confirmBackground(): Promise<void> {
    await confirm(false);
  }

  /** 失败后「重试剩余」。 */
  async function retry(): Promise<void> {
    const tasks = failed.value;
    if (tasks.length === 0) return;
    failed.value = [];
    await run(tasks);
  }

  /** 取消当前下载。 */
  async function cancelDownload(): Promise<void> {
    await downloads.cancel();
  }

  /** 换后端：由调用方重算计划并塞回来。 */
  function setPlan(next: SetupPlan | null): void {
    plan.value = next;
    open.value = next !== null;
  }

  return {
    open,
    plan,
    running,
    forced,
    failed,
    /** 当前这一项的进度（页内 / 对话框里的进度条）。 */
    progress: downloads.progress,
    ensure,
    maybePrompt,
    cancel,
    confirm,
    confirmBackground,
    run,
    retry,
    cancelDownload,
    setDone,
    setPlan,
  };
}
