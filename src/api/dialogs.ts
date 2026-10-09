/**
 * 系统文件对话框（打开 / 保存）。
 * 浏览器预览（非 Tauri）没有原生对话框，返回一个假路径让流程可以走通。
 */
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { isTauri } from "@/utils/platform";
import { useI18n } from "@/i18n";

/** 过滤器名字要按当前语言取，所以每次现算（对话框是低频操作）。 */
function projectFilter() {
  return [{ name: useI18n().t("dialog.projectFilter"), extensions: ["json"] }];
}

function wavFilter() {
  return [{ name: useI18n().t("dialog.wavFilter"), extensions: ["wav"] }];
}

/** 可导入的音频格式（与 `crates/asset` 的解码能力一致）。 */
function audioFilter() {
  return [
    {
      name: useI18n().t("dialog.audioFilter"),
      extensions: ["wav", "flac", "mp3", "ogg", "m4a", "aac"],
    },
  ];
}

/** 选择要导入的工程文件；取消返回 null。 */
export async function pickProjectToOpen(): Promise<string | null> {
  if (!isTauri()) return "mock://demo.json";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: projectFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择工程保存路径；取消返回 null。 */
export async function pickProjectToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({ defaultPath, filters: projectFilter() });
  return selected ?? null;
}

/** 选择 WAV 导出路径；取消返回 null。 */
export async function pickWavToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({
    defaultPath,
    filters: wavFilter(),
  });
  return selected ?? null;
}

/** 选择要导入的音频文件；取消返回 null。 */
export async function pickAudioToOpen(): Promise<string | null> {
  if (!isTauri()) return "mock://kick.wav";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: audioFilter(),
  });
  return typeof selected === "string" ? selected : null;
}
