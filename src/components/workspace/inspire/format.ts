/**
 * 灵感页里用到的本地小工具：时长格式化。
 *
 * 时长在列表里要**对齐**（等宽数字），且要能一路长到小时（一次生成可以到 10 分钟，
 * 加上工程里导入的长素材，`m:ss` 会不够用）。
 */
import { bcp47 } from "@/i18n";

/** `73` → `1:13`；超过一小时 → `1:02:03`。非法值按 0 处理。 */
export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** 一天的分组键：按**本地时区**的日期（`2026-11-18`）。 */
export function dayKey(at: number): string {
  const date = new Date(at);
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/**
 * 分组标题：今天 / 昨天 / 具体日期。
 *
 * 「具体日期」交给 `Intl`：中文是「11月18日」，英文是「Nov 18」。**不拼星期**——
 * 生成记录通常就是最近几天，星期对找歌没有帮助，反而每行更长。
 */
export function dayLabel(at: number, locale: string, t: (key: string) => string): string {
  const now = Date.now();
  const key = dayKey(at);
  if (key === dayKey(now)) return t("inspire.today");
  if (key === dayKey(now - 24 * 60 * 60 * 1000)) return t("inspire.yesterday");
  try {
    return new Intl.DateTimeFormat(bcp47(locale), {
      month: "short",
      day: "numeric",
    }).format(new Date(at));
  } catch {
    return key;
  }
}
