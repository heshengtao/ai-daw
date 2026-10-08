/**
 * 系统文件对话框（打开 / 保存）。
 * 浏览器预览（非 Tauri）没有原生对话框，返回一个假路径让流程可以走通。
 */
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { isTauri } from "@/utils/platform";

const PROJECT_FILTER = [{ name: "daw-party 工程", extensions: ["json"] }];

/** 选择要导入的工程文件；取消返回 null。 */
export async function pickProjectToOpen(): Promise<string | null> {
  if (!isTauri()) return "mock://demo.json";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: PROJECT_FILTER,
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择工程保存路径；取消返回 null。 */
export async function pickProjectToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({ defaultPath, filters: PROJECT_FILTER });
  return selected ?? null;
}

/** 选择 WAV 导出路径；取消返回 null。 */
export async function pickWavToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({
    defaultPath,
    filters: [{ name: "WAV 音频", extensions: ["wav"] }],
  });
  return selected ?? null;
}
