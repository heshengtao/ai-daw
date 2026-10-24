/** 在系统浏览器里打开外部链接（浏览器调试时退回新标签页）。 */
import { managerOpen } from "@/api/tauri";
import { isTauri } from "@/utils/platform";

export async function openExternal(url: string): Promise<void> {
  if (!isTauri()) {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  try {
    await managerOpen(url);
  } catch {
    window.open(url, "_blank", "noopener,noreferrer");
  }
}
