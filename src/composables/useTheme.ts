/*!
 * Theme (light/dark/system + accent) composable.
 * Adapted from ExamBot (https://github.com/heshengtao/exameow), Apache-2.0.
 */
import { ref, watch } from "vue";

/** 主题模式。 */
export type Theme = "system" | "light" | "dark";
/** 强调色。 */
export type AccentColor = "blue" | "green" | "coral" | "purple" | "amber";

const THEME_KEY = "daw-party-theme";
const ACCENT_KEY = "daw-party-accent";

function loadTheme(): Theme {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved === "system" || saved === "light" || saved === "dark") return saved;
  // 音频工作站默认深色。
  return "dark";
}

function loadAccent(): AccentColor {
  const saved = localStorage.getItem(ACCENT_KEY);
  if (saved === "blue" || saved === "green" || saved === "coral" || saved === "purple" || saved === "amber") {
    return saved;
  }
  return "blue";
}

const theme = ref<Theme>(loadTheme());
const accent = ref<AccentColor>(loadAccent());
const media = window.matchMedia("(prefers-color-scheme: dark)");

function applyTheme(): void {
  const dark = theme.value === "dark" || (theme.value === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.setAttribute("data-accent", accent.value);
  localStorage.setItem(THEME_KEY, theme.value);
  localStorage.setItem(ACCENT_KEY, accent.value);
}

let initialized = false;

/** 提供主题状态与操作；首次调用时开始同步到 <html>。 */
export function useTheme() {
  if (!initialized) {
    initialized = true;
    watch(theme, applyTheme, { immediate: true });
    watch(accent, applyTheme, { immediate: true });
    media.addEventListener("change", () => {
      if (theme.value === "system") applyTheme();
    });
  }

  function setTheme(t: Theme): void {
    theme.value = t;
  }
  function setAccent(a: AccentColor): void {
    accent.value = a;
  }
  function cycleTheme(): void {
    theme.value = theme.value === "system" ? "light" : theme.value === "light" ? "dark" : "system";
  }

  return { theme, accent, setTheme, setAccent, cycleTheme };
}
