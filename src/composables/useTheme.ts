/*!
 * Theme (light/dark/system + accent) composable.
 * Adapted from ExamBot (https://github.com/heshengtao/exameow), Apache-2.0.
 */
import { ref, watch } from "vue";

/** 主题模式。 */
export type Theme = "system" | "light" | "dark";
/** 强调色；`lavender` 为 MD3 基线默认色。 */
export type AccentColor = "lavender" | "blue" | "green" | "coral" | "amber";

/** 可选强调色（顺序即设置页展示顺序；swatch 仅用于色板预览）。 */
export const ACCENTS: { id: AccentColor; labelKey: string; swatch: string }[] = [
  { id: "lavender", labelKey: "accent.lavender", swatch: "#6750A4" },
  { id: "blue", labelKey: "accent.blue", swatch: "#1A6CFF" },
  { id: "green", labelKey: "accent.green", swatch: "#2E7D4F" },
  { id: "coral", labelKey: "accent.coral", swatch: "#E05642" },
  { id: "amber", labelKey: "accent.amber", swatch: "#BD8900" },
];

/** 主题模式的可选项（设置页展示用）。 */
export const THEMES: { id: Theme; labelKey: string }[] = [
  { id: "system", labelKey: "theme.system" },
  { id: "light", labelKey: "theme.light" },
  { id: "dark", labelKey: "theme.dark" },
];

const THEME_KEY = "ai-daw-theme";
const ACCENT_KEY = "ai-daw-accent";

function loadTheme(): Theme {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved === "system" || saved === "light" || saved === "dark") return saved;
  // 音频工作站默认深色。
  return "dark";
}

function loadAccent(): AccentColor {
  const saved = localStorage.getItem(ACCENT_KEY);
  if (ACCENTS.some((a) => a.id === saved)) return saved as AccentColor;
  return "lavender";
}

const theme = ref<Theme>(loadTheme());
const accent = ref<AccentColor>(loadAccent());
const media = window.matchMedia("(prefers-color-scheme: dark)");

/**
 * 当前**实际**是不是深色（`system` 跟随系统）。
 *
 * 谱面渲染要按它换配色：OSMD 的 `darkMode` 只能在构造时给，主题一变就得重建实例，
 * 所以需要一个能 watch 的值，而不是去读 `<html>` 的 class。
 */
const isDark = ref(false);

function applyTheme(): void {
  const dark = theme.value === "dark" || (theme.value === "system" && media.matches);
  isDark.value = dark;
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

  return { theme, accent, isDark, setTheme, setAccent, cycleTheme };
}
