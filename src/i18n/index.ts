/**
 * 轻量国际化：支持 10 种界面语言。
 *
 * 语言按「用户选过的 → 系统语言 → 兜底」的顺序决定，选择存在 localStorage 里，设置页可以切。
 * 加一门语言只需在 `messages` 里补一份词典 + 在 `LOCALES` 里登记。
 *
 * **回退链是「当前语言 → English → 简体中文 → key 本身」**：新语言先翻常用条目也能用，
 * 缺词时看到英文而不是中文（中文只作为中文系语言的最后兜底）。`{name}` 形式的占位由 `vars` 替换。
 */
import { computed, ref } from "vue";

import en from "./en";
import zh from "./zh";
import zhTW from "./zh-TW";
import ja from "./ja";
import ko from "./ko";
import es from "./es";
import fr from "./fr";
import de from "./de";
import ru from "./ru";
import ar from "./ar";

export type Locale = "zh" | "zh-TW" | "en" | "ja" | "ko" | "es" | "fr" | "de" | "ru" | "ar";

/** 一门语言的展示信息。 */
export interface LocaleMeta {
  id: Locale;
  /** 母语写法（语言选择器里显示）。 */
  label: string;
  /** 英文名，给「母语写法 + 英文」这种两行显示用。 */
  english: string;
  /** 交给 `Intl` / `<html lang>` 的 BCP 47 标签。 */
  bcp47: string;
  /** 书写方向。 */
  dir: "ltr" | "rtl";
}

/**
 * 支持的语言。顺序即语言选择器里的顺序：中文系在最前，之后按用户量排。
 * （语言名一律用**母语写法**：切到看不懂的语言时，用户还能找到自己的那一项。）
 */
export const LOCALES: LocaleMeta[] = [
  { id: "zh", label: "简体中文", english: "Chinese (Simplified)", bcp47: "zh-CN", dir: "ltr" },
  { id: "zh-TW", label: "繁體中文", english: "Chinese (Traditional)", bcp47: "zh-TW", dir: "ltr" },
  { id: "en", label: "English", english: "English", bcp47: "en", dir: "ltr" },
  { id: "ja", label: "日本語", english: "Japanese", bcp47: "ja-JP", dir: "ltr" },
  { id: "ko", label: "한국어", english: "Korean", bcp47: "ko-KR", dir: "ltr" },
  { id: "es", label: "Español", english: "Spanish", bcp47: "es-ES", dir: "ltr" },
  { id: "fr", label: "Français", english: "French", bcp47: "fr-FR", dir: "ltr" },
  { id: "de", label: "Deutsch", english: "German", bcp47: "de-DE", dir: "ltr" },
  { id: "ru", label: "Русский", english: "Russian", bcp47: "ru-RU", dir: "ltr" },
  { id: "ar", label: "العربية", english: "Arabic", bcp47: "ar", dir: "rtl" },
];

export const LOCALE_MAP: Record<Locale, LocaleMeta> = Object.fromEntries(
  LOCALES.map((meta) => [meta.id, meta]),
) as Record<Locale, LocaleMeta>;

/** 找不到词时的主兜底语言（英文词典最完整）。 */
const FALLBACK: Locale = "en";
/** 英文也缺时的次兜底：中文系语言回中文，其余仍回 key。 */
const SECONDARY: Locale = "zh";

const MESSAGES: Record<Locale, Record<string, string>> = {
  zh,
  "zh-TW": zhTW,
  en,
  ja,
  ko,
  es,
  fr,
  de,
  ru,
  ar,
};

/** 语言选择的持久化键。 */
const KEY = "ai-daw-locale";

/** 文案里 `{name}` 形式的占位由 `vars` 替换。 */
export type Vars = Record<string, string | number>;

/** 取一门语言的 BCP 47 标签（`Intl` 用）。 */
export function bcp47(locale: string): string {
  return LOCALE_MAP[locale as Locale]?.bcp47 ?? "en";
}

/** 这门语言是不是从右往左写。 */
export function isRtlLocale(locale: string): boolean {
  return LOCALE_MAP[locale as Locale]?.dir === "rtl";
}

/**
 * 按系统语言猜一门：浏览器可能给一列首选项，逐个匹配。
 *
 * 中文必须**先判断繁体区**（`zh-Hant` / `zh-TW` / `zh-HK` / `zh-MO`）：这些地方用繁体字，
 * 只按 `zh` 前缀猜会把他们送进简体。
 */
function detect(): Locale {
  const raw =
    typeof navigator === "undefined"
      ? []
      : navigator.languages?.length
        ? [...navigator.languages]
        : [navigator.language];
  for (const tag of raw) {
    const lower = (tag ?? "").toLowerCase();
    if (/^zh-(hant|tw|hk|mo)/.test(lower)) return "zh-TW";
    if (/^zh-(hans|cn|sg)/.test(lower) || lower === "zh") return "zh";
    if (lower.startsWith("ja")) return "ja";
    if (lower.startsWith("ko")) return "ko";
    if (lower.startsWith("es")) return "es";
    if (lower.startsWith("fr")) return "fr";
    if (lower.startsWith("de")) return "de";
    if (lower.startsWith("ru")) return "ru";
    if (lower.startsWith("ar")) return "ar";
    if (lower.startsWith("en")) return "en";
  }
  return FALLBACK;
}

/** 用户选过的语言优先，其次才是系统语言。 */
function initial(): Locale {
  try {
    const saved = localStorage.getItem(KEY);
    const hit = LOCALES.find((l) => l.id === saved);
    if (hit) return hit.id;
  } catch {
    // 读不到就用系统语言。
  }
  return detect();
}

const locale = ref<Locale>(initial());
applyDocumentLang(locale.value);

/**
 * 把语言与书写方向写到 `<html>`：CSS、屏幕阅读器与镜像布局都靠它。
 *
 * 阿拉伯语是 RTL，整页镜像；**时间轴类组件（编排 / 编辑 / 乐谱 / 调音台 / 走带）
 * 自己在模板上标了 `dir="ltr"`**——小节要从左往右读、自动化曲线与钢琴卷帘的横轴
 * 也一样，把它们镜像过来语义就反了。所以这里只管外层，时间轴保持 LTR 是组件级的决定。
 */
function applyDocumentLang(value: Locale): void {
  if (typeof document === "undefined") return;
  const meta = LOCALE_MAP[value] ?? LOCALE_MAP[FALLBACK];
  document.documentElement.lang = meta.bcp47;
  document.documentElement.dir = meta.dir;
}

/** 切换语言（设置页调用）。 */
function setLocale(next: Locale): void {
  if (!LOCALE_MAP[next]) return;
  locale.value = next;
  applyDocumentLang(next);
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // 存不住也不影响本次会话。
  }
}

/** 取一条文案：当前语言 → 英文 → 中文 → key。 */
function t(key: string, vars?: Vars): string {
  const dict = MESSAGES[locale.value] ?? MESSAGES[FALLBACK];
  let text = dict[key] ?? MESSAGES[FALLBACK][key] ?? MESSAGES[SECONDARY][key] ?? key;
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      text = text.replaceAll(`{${name}}`, String(value));
    }
  }
  return text;
}

export function useI18n() {
  return {
    /** 当前语言。 */
    locale: computed(() => locale.value),
    /** 当前语言是不是 RTL（阿拉伯语）。 */
    isRtl: computed(() => isRtlLocale(locale.value)),
    locales: LOCALES,
    t,
    setLocale,
  };
}
