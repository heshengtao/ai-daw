/**
 * 系统文件对话框（打开 / 保存）。
 *
 * 浏览器模式下**没有**原生对话框：能走"上传"的（工程 / 音频 / MIDI / 乐谱）走上传；
 * 不能上传的（LoRA 适配器、插件包、扫描目录）一律抛明确错误——以前它们返回
 * `mock://…` 假路径，用户会看到一个假的成功，然后后端报"不是绝对路径 / 未知插件"。
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
  if (!isTauri()) return pickAndUpload("project");
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: projectFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择工程保存路径；取消返回 null。 */
export async function pickProjectToSave(defaultPath: string): Promise<string | null> {
  // 浏览器里没有保存对话框：让服务端把导出文件写进数据目录的 exports/，
  // 拿回真实路径后前端触发下载（见 utils/download.ts）。
  if (!isTauri()) return "";
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
  // 浏览器里没有保存对话框：让服务端把导出文件写进数据目录的 exports/，
  // 拿回真实路径后前端触发下载（见 utils/download.ts）。
  if (!isTauri()) return "";
  const selected = await saveDialog({ defaultPath, filters: exportFilter() });
  return selected ?? null;
}

/** 选择乐谱（MusicXML）保存路径；取消返回 null。 */
export async function pickScoreToSave(defaultPath: string): Promise<string | null> {
  // 浏览器里没有保存对话框：让服务端把导出文件写进数据目录的 exports/，
  // 拿回真实路径后前端触发下载（见 utils/download.ts）。
  if (!isTauri()) return "";
  const selected = await saveDialog({ defaultPath, filters: scoreFilter() });
  return selected ?? null;
}

/**
 * 选择 MIDI 保存路径；取消返回 null。
 *
 * 只给 `.mid` / `.midi`：SMF 的容器扩展名就这两种（`.smf` 只作为导入的兼容写法）。
 */
export async function pickMidiToSave(defaultPath: string): Promise<string | null> {
  // 浏览器里没有保存对话框：让服务端把导出文件写进数据目录的 exports/，
  // 拿回真实路径后前端触发下载（见 utils/download.ts）。
  if (!isTauri()) return "";
  const selected = await saveDialog({
    defaultPath,
    filters: [{ name: useI18n().t("dialog.midiFilter"), extensions: ["mid", "midi"] }],
  });
  return selected ?? null;
}

/**
 * 把一段浏览器内存里的数据上传到 daemon，拿回服务端路径（`<data>/imports/`）。
 *
 * 录音走这条：浏览器只拿得到 `Blob`，没有真实路径；服务端落盘之后
 * `import_audio` / `add_audio_clip` 才有东西可读。
 */
export async function uploadBlob(name: string, blob: Blob): Promise<string> {
  const response = await fetch(`/upload?name=${encodeURIComponent(name)}`, {
    method: "POST",
    body: blob,
  });
  if (!response.ok) throw new Error(await response.text());
  const body = (await response.json()) as { path?: string };
  if (!body.path) throw new Error("上传后没有拿到路径");
  return body.path;
}

/**
 * 浏览器端选文件：把用户选中的文件上传到 daemon，拿回服务端路径。
 *
 * 浏览器的文件输入只给 `File` 对象，拿不到真实路径；所以先 POST 到 `/upload`，
 * 由服务端落到 `<数据目录>/imports/`，再把路径交给 `import_audio`。
 */
export function pickAndUpload(kind: string): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept =
      kind === "audio"
        ? "audio/*"
        : kind === "midi"
          ? ".mid,.midi,.smf"
          : kind === "score"
            ? ".musicxml,.xml,.mxl"
            : kind === "project"
              ? ".json,application/json"
              : "*";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) {
        resolve(null);
        return;
      }
      try {
        const response = await fetch(`/upload?name=${encodeURIComponent(file.name)}`, {
          method: "POST",
          body: file,
        });
        if (!response.ok) {
          resolve(null);
          return;
        }
        const body = (await response.json()) as { path?: string };
        resolve(body.path ?? null);
      } catch {
        resolve(null);
      }
    };
    // 用户在对话框里点取消时不会有 change 事件；靠 focus 回来兜一次，避免 Promise 永挂。
    input.oncancel = () => resolve(null);
    input.click();
  });
}

/** 选择要导入的音频文件；取消返回 null。浏览器里走上传。 */
export async function pickAudioToOpen(): Promise<string | null> {
  if (!isTauri()) return pickAndUpload("audio");
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: audioFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的 MIDI 文件；取消返回 null。浏览器里走上传。 */
export async function pickMidiToOpen(): Promise<string | null> {
  if (!isTauri()) return pickAndUpload("midi");
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: midiFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的谱面（MusicXML）；取消返回 null。 */
export async function pickScoreToOpen(): Promise<string | null> {
  if (!isTauri()) return pickAndUpload("score");
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: scoreOpenFilter(),
  });
  return typeof selected === "string" ? selected : null;
}

/** 浏览器模式下的"选不了东西"错误：统一一句话，由调用方 toast。 */
function desktopOnly(what: string): Error {
  return new Error(useI18n().t("dialog.desktopOnly", { what }));
}

/** 选择 YuE2 的 LoRA 适配器（`.safetensors`）；取消返回 null。 */
export async function pickLoraFile(): Promise<string | null> {
  if (!isTauri()) throw desktopOnly(useI18n().t("dialog.lora"));
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "LoRA", extensions: ["safetensors"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的插件（`.vst3` bundle / 文件、`.clap` 文件，或 macOS 的 `.component`）；取消返回 null。 */
export async function pickPluginToImport(): Promise<string | null> {
  if (!isTauri()) throw desktopOnly(useI18n().t("dialog.pluginPackage"));
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: useI18n().t("dialog.pluginFilter"), extensions: ["vst3", "clap", "component"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/**
 * 选择要导入的 SoundFont 音色库（`.sf2` / `.sf3`，可多选）；取消返回空数组。
 *
 * 浏览器模式直接抛错：上传一个几百 MB 的 `.sf2` 没有意义（后端要的是本地路径），
 * 与其让用户等完上传再失败，不如说清楚。
 */
export async function pickSoundfontsToImport(): Promise<string[]> {
  if (!isTauri()) throw desktopOnly(useI18n().t("dialog.soundfont"));
  const selected = await openDialog({
    multiple: true,
    directory: false,
    filters: [{ name: useI18n().t("dialog.soundfontFilter"), extensions: ["sf2", "sf3"] }],
  });
  if (Array.isArray(selected)) return selected;
  return typeof selected === "string" ? [selected] : [];
}

/** 选择一个目录（引擎归档离线安装、插件扫描目录等）；取消返回 null。 */
export async function pickDirectory(): Promise<string | null> {
  if (!isTauri()) throw desktopOnly(useI18n().t("dialog.directory"));
  const selected = await openDialog({ multiple: false, directory: true });
  return typeof selected === "string" ? selected : null;
}
