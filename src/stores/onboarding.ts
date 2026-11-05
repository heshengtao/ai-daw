/**
 * 新手引导的状态与记忆。
 *
 * 只把**「看过没有」**这种事实写进 localStorage；步骤、文案、锚点都属于界面，
 * 跟着 `GuidedTour.vue` 走——将来加一步不会让老用户的记录失效。
 *
 * 版本号的意义：引导内容大改（例如界面结构重排）时把它 +1，
 * 所有用户会再看一次欢迎页；只补一句文案就不必动它。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

const KEY = "ai-daw-onboarding";

/** 当前引导内容的版本（与产品结构绑定）。 */
const VERSION = 1;

interface Saved {
  version: number;
  seen: boolean;
  /** 编排视图那张「从这里开始」的浮卡被手动关掉了。 */
  coachDismissed: boolean;
}

function load(): Saved {
  const empty: Saved = { version: VERSION, seen: false, coachDismissed: false };
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return empty;
    const parsed = JSON.parse(raw) as Partial<Saved>;
    // 版本对不上 = 界面结构变了：当作没看过（但「关掉浮卡」是纯偏好，保留）。
    const fresh = parsed.version !== VERSION;
    return {
      version: VERSION,
      seen: fresh ? false : parsed.seen === true,
      coachDismissed: parsed.coachDismissed === true,
    };
  } catch {
    return empty;
  }
}

export const useOnboardingStore = defineStore("onboarding", () => {
  const saved = load();

  /** 是否已经完整看过欢迎页 / 引导（决定启动时要不要弹）。 */
  const seen = ref(saved.seen);
  /** 编排视图的起步浮卡是否被手动关掉。 */
  const coachDismissed = ref(saved.coachDismissed);

  /** 欢迎面板是否打开。 */
  const welcomeOpen = ref(false);
  /** 聚光灯引导是否进行中。 */
  const tourOpen = ref(false);
  /** 当前步骤下标（换步骤只改这个值，由引导组件自己滚动 / 量位置）。 */
  const step = ref(0);

  function persist(): void {
    try {
      localStorage.setItem(
        KEY,
        JSON.stringify({ version: VERSION, seen: seen.value, coachDismissed: coachDismissed.value } satisfies Saved),
      );
    } catch {
      // 无痕模式 / 存储被禁用：记不住就记不住，最多下次再弹一遍，不影响功能。
    }
  }

  /** 首次启动时调用：没看过才弹欢迎页。 */
  function maybeWelcome(): void {
    if (!seen.value) welcomeOpen.value = true;
  }

  function openWelcome(): void {
    welcomeOpen.value = true;
  }

  /** 关掉欢迎页：正文点了任何入口都算「看过了」，不再自动弹。 */
  function closeWelcome(): void {
    welcomeOpen.value = false;
    seen.value = true;
    persist();
  }

  /** 从欢迎页（或顶栏「帮助」）进入引导。 */
  function startTour(): void {
    welcomeOpen.value = false;
    seen.value = true;
    persist();
    step.value = 0;
    tourOpen.value = true;
  }

  function endTour(): void {
    tourOpen.value = false;
    step.value = 0;
  }

  function goStep(next: number): void {
    step.value = Math.max(0, next);
  }

  /** 「再看一次引导」：从顶栏帮助菜单进来时用，不动 `seen`。 */
  function replayTour(): void {
    step.value = 0;
    tourOpen.value = true;
  }

  function dismissCoach(): void {
    coachDismissed.value = true;
    persist();
  }

  return {
    seen,
    coachDismissed,
    welcomeOpen,
    tourOpen,
    step,
    maybeWelcome,
    openWelcome,
    closeWelcome,
    startTour,
    endTour,
    replayTour,
    goStep,
    dismissCoach,
  };
});
