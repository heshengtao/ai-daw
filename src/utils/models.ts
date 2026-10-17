import type { ModelStatus } from "@/api/types";

/**
 * 模型条目**还差多少字节**没下载。
 *
 * 组条目（多个文件缺一不可）经常是「缺一两个组件」的状态——按整组体积报数会让人以为
 * 要重下几个 GB。下载器本来就会跳过已就位的文件，所以界面也该按**缺的那些**报数。
 */
export function missingBytes(model: ModelStatus): number {
  if (model.installed) return 0;
  const total = model.files.reduce((sum, f) => sum + f.approx_bytes, 0);
  const present = model.files.reduce(
    (sum, f) => (f.installed ? sum + f.approx_bytes : sum),
    0,
  );
  // 单文件条目（或清单尚未加载出来）时退回整组体积，别显示成 0。
  return total > 0 ? total - present : model.approx_bytes;
}

/** **还差几个文件**（组条目的「还差 n 个」用；单文件条目只可能是 0 或 1）。 */
export function missingFiles(model: ModelStatus): number {
  if (model.installed) return 0;
  return model.files.filter((f) => !f.installed).length;
}

/** 人类可读体积（`1.5 GB` / `352.0 MB`）。 */
export function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}
