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

/** 导出可选的音频格式（两个过滤器都给：用户在这次对话框里改扩展名就等于改格式）。 */
function exportFilter() {
  return [
    { name: useI18n().t("dialog.wavFilter"), extensions: ["wav"] },
    { name: useI18n().t("dialog.mp3Filter"), extensions: ["mp3"] },
  ];
}

/**
 * 乐谱**导出**过滤器。
 *
 * 只给 `.musicxml` 一个扩展名：MuseScore / Dorico / Sibelius 都能直接认，而 `.xml` 太泛，
 * 系统对话框里会混进一堆无关文件。
 */
function scoreFilter() {
  return [{ name: useI18n().t("dialog.scoreFilter"), extensions: ["musicxml"] }];
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

/** 可导入的 MIDI 文件（`.smf` 偶见于老工具导出的裸 SMF）。 */
function midiFilter() {
  return [{ name: useI18n().t("dialog.midiFilter"), extensions: ["mid", "midi", "smf"] }];
}

/** 可导入的谱面：`.mxl` 是压缩的 MusicXML（zip），现在还不解压，先让用户能选中。 */
function scoreOpenFilter() {
  return [{ name: useI18n().t("dialog.scoreOpenFilter"), extensions: ["musicxml", "xml", "mxl"] }];
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

/**
 * 选择音频导出路径；取消返回 null。
 *
 * WAV 与 MP3 两个过滤器都给出去，`defaultPath` 的扩展名决定对话框默认选中哪个。**返回值里的
 * 扩展名才是最终格式**（用户可能在对话框里换了过滤器、也可能手打扩展名），调用方按它决定用
 * 哪个编码器——所以「文件名写 .mp3」永远会得到 MP3。
 */
export async function pickAudioToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({ defaultPath, filters: exportFilter() });
  return selected ?? null;
}

/** 选择乐谱（MusicXML）保存路径；取消返回 null。 */
export async function pickScoreToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({ defaultPath, filters: scoreFilter() });
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

/** 选择要导入的 MIDI 文件；取消返回 null。 */
export async function pickMidiToOpen(): Promise<string | null> {
  if (!isTauri()) return "mock://song.mid";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: midiFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的谱面（MusicXML）；取消返回 null。 */
export async function pickScoreToOpen(): Promise<string | null> {
  if (!isTauri()) return "mock://score.musicxml";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: scoreOpenFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的插件（`.vst3` bundle / 文件、`.clap` 文件，或 macOS 的 `.component`）；取消返回 null。 */
export async function pickPluginToImport(): Promise<string | null> {
  if (!isTauri()) return "mock://CoolSynth.vst3";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: useI18n().t("dialog.pluginFilter"), extensions: ["vst3", "clap", "component"] }],
  });
  return typeof selected === "string" ? selected : null;
}
