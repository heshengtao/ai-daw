/**
 * 新手引导的状态与记忆。
 *
 * 欢迎页现在**每次启动都会弹**（除非在「设置 → 常规 → 启动」里关掉），
 * 所以这里不再记「看过没有」，只留「编排视图那张起步浮卡被手动关掉」这种纯偏好。
 * 步骤、文案、锚点都属于界面，跟着 `GuidedTour.vue` 走。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

const KEY = "ai-daw-onboarding";

interface Saved {
  /** 编排视图那张「从这里开始」的浮卡被手动关掉了。 */
  coachDismissed: boolean;
}

/** 老版本还存过 `version` / `seen`：直接忽略，只读回仍然有意义的那一项。 */
function load(): Saved {
  const empty: Saved = { coachDismissed: false };
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return empty;
    const parsed = JSON.parse(raw) as Partial<Saved>;
    return { coachDismissed: parsed.coachDismissed === true };
  } catch {
    return empty;
  }
}

export const useOnboardingStore = defineStore("onboarding", () => {
  const saved = load();

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
        JSON.stringify({ coachDismissed: coachDismissed.value } satisfies Saved),
      );
    } catch {
      // 无痕模式 / 存储被禁用：记不住就记不住，最多下次再弹一遍，不影响功能。
    }
  }

  /** 启动时调用：由 `App.vue` 先看设置里的开关键，再决定要不要打开。 */
  function openWelcome(): void {
    welcomeOpen.value = true;
  }

  /** 关掉欢迎页（选入口 / 遮罩 / Esc / 「先随便看看」都走这里）。 */
  function closeWelcome(): void {
    welcomeOpen.value = false;
  }

  /** 从欢迎页（或顶栏「帮助」）进入引导。 */
  function startTour(): void {
    welcomeOpen.value = false;
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

  /** 「再看一次引导」：从顶栏帮助菜单进来时用。 */
  function replayTour(): void {
    step.value = 0;
    tourOpen.value = true;
  }

  function dismissCoach(): void {
    coachDismissed.value = true;
    persist();
  }

  return {
    coachDismissed,
    welcomeOpen,
    tourOpen,
    step,
    openWelcome,
    closeWelcome,
    startTour,
    endTour,
    replayTour,
    goStep,
    dismissCoach,
  };
});
