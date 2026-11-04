import { diskFreeBytes } from "@/api/tauri";

/**
 * 下载前的磁盘预检。
 *
 * 读不到（无权限 / 命令被策略禁用 / 非本地盘）就当作「不知道」放行——这是尽力而为的拦截，
 * 不能因为拿不到数字就挡住用户。1 秒缓存：连点几个「下载」不该反复跑 PowerShell。
 */
let cached: { at: number; free: number | null } | null = null;

export async function freeDiskBytes(force = false): Promise<number | null> {
  const now = Date.now();
  if (!force && cached && now - cached.at < 1000) return cached.free;
  let free: number | null = null;
  try {
    free = await diskFreeBytes();
  } catch {
    free = null;
  }
  cached = { at: now, free };
  return free;
}

/**
 * 这次下载放不放得下（按 1.2 倍留解压 / 峰值余量）。
 *
 * 装到一半满盘会留下半个目录，比一开始就说「还差 4 GB」难受得多。
 */
export async function hasDiskRoom(bytes: number): Promise<boolean> {
  if (bytes <= 0) return true;
  const free = await freeDiskBytes();
  if (free === null) return true;
  return free >= Math.round(bytes * 1.2);
}
