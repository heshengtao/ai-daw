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

/**
 * 选择 MIDI 保存路径；取消返回 null。
 *
 * 只给 `.mid` / `.midi`：SMF 的容器扩展名就这两种（`.smf` 只作为导入的兼容写法）。
 */
export async function pickMidiToSave(defaultPath: string): Promise<string | null> {
  if (!isTauri()) return `mock://${defaultPath}`;
  const selected = await saveDialog({
    defaultPath,
    filters: [{ name: useI18n().t("dialog.midiFilter"), extensions: ["mid", "midi"] }],
  });
  return selected ?? null;
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

/** 选择 YuE2 的 LoRA 适配器（`.safetensors`）；取消返回 null。 */
export async function pickLoraFile(): Promise<string | null> {
  if (!isTauri()) return "mock://style-lora.safetensors";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: "LoRA", extensions: ["safetensors"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择要导入的插件（`.vst3` bundle / 文件、`.clap` 文件，或 macOS 的 `.component`）；取消返回 null。 */export async function pickPluginToImport(): Promise<string | null> {
  if (!isTauri()) return "mock://CoolSynth.vst3";
  const selected = await openDialog({
    multiple: false,
    directory: false,
    filters: [{ name: useI18n().t("dialog.pluginFilter"), extensions: ["vst3", "clap", "component"] }],
  });
  return typeof selected === "string" ? selected : null;
}

/** 选择一个目录（引擎归档离线安装、插件扫描目录等）；取消返回 null。 */
export async function pickDirectory(): Promise<string | null> {
  if (!isTauri()) return "mock://engine-archives";
  const selected = await openDialog({ multiple: false, directory: true });
  return typeof selected === "string" ? selected : null;
}
