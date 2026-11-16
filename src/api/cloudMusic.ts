/**
 * 云端音乐生成的命令封装（与 `crates/daemon/src/cloud_music.rs` 一一对应）。
 *
 * 生成是长任务：命令本身会等到成品落盘并加入工程才返回；过程中的状态走
 * `cloud-music-progress` 事件（桌面 Tauri event / 浏览器 SSE 同一套语义）。
 */
import { invoke, listen, type UnlistenFn } from "./ipc";
import { mockEnabled } from "./mock";
import type {
  CloudMusicConfig,
  CloudMusicPreset,
  CloudMusicProgress,
  CloudMusicProvider,
  CloudMusicRequest,
  CloudMusicResult,
} from "./types";

export const EVENT_CLOUD_MUSIC = "cloud-music-progress";

const MOCK_PROVIDER: CloudMusicProvider = {
  id: "mock-lyria",
  name: "示例 · Google Lyria 3.5",
  kind: "lyria",
  enabled: true,
  base_url: "https://generativelanguage.googleapis.com",
  api_key: "sk-demo",
  api_secret: "",
  model: "lyria-3.5",
};

const MOCK_PRESETS: CloudMusicPreset[] = [
  {
    kind: "lyria",
    name: "Google Lyria 3.5",
    group: "official",
    base_url: "https://generativelanguage.googleapis.com",
    model: "lyria-3.5",
    max_seconds: 180,
    docs_url: "https://ai.google.dev/gemini-api/docs/music-generation",
    note: "Gemini API；44.1 kHz 立体声，支持人声与歌词。",
  },
  {
    kind: "elevenlabs",
    name: "ElevenLabs Music",
    group: "official",
    base_url: "https://api.elevenlabs.io",
    model: "music_v2_5",
    max_seconds: 600,
    docs_url: "https://elevenlabs.io/docs/api-reference/music/compose",
    note: "官方 Music API；支持 composition plan 逐段歌词。",
  },
  {
    kind: "mureka",
    name: "Mureka",
    group: "official",
    base_url: "https://api.mureka.cn",
    model: "auto",
    max_seconds: 300,
    docs_url: "https://platform.mureka.ai/docs/",
    note: "昆仑万维；支持歌词 / 提示词生歌与纯音乐。",
  },
  {
    kind: "treblo",
    name: "Treblo (Sonauto)",
    group: "official",
    base_url: "https://api.treblo.com",
    model: "",
    max_seconds: 285,
    docs_url: "https://treblo.com/developers/docs",
    note: "原 Sonauto；支持歌词、纯音乐、局部替换。",
  },
  {
    kind: "alibaba",
    name: "阿里云百炼 Fun-Music",
    group: "official",
    base_url: "https://YOUR_WORKSPACE.cn-beijing.maas.aliyuncs.com",
    model: "fun-music-v1",
    max_seconds: 300,
    docs_url: "https://help.aliyun.com/zh/model-studio/fun-music-api",
    note: "百炼 Fun-Music；中英文人声、歌词、纯音乐。",
  },
  {
    kind: "volcengine",
    name: "火山引擎 豆包音乐",
    group: "official",
    base_url: "https://open.volcengineapi.com",
    model: "v5.0",
    max_seconds: 240,
    docs_url: "https://www.volcengine.com/docs/84992/1967910",
    note: "豆包音乐；人声歌曲 / 纯音乐；AK/SK 签名。",
  },
  {
    kind: "tencent",
    name: "腾讯云 MPS（MiniMax）",
    group: "official",
    base_url: "https://mps.tencentcloudapi.com",
    model: "MinimaxMusic",
    max_seconds: 300,
    docs_url: "https://cloud.tencent.com/document/product/862/133520",
    note: "MPS 聚合 MiniMax Music；SecretId + SecretKey 的 TC3 签名。",
  },
];

export function cloudMusicConfig(): Promise<CloudMusicConfig> {
  if (mockEnabled()) return Promise.resolve({ providers: [MOCK_PROVIDER] });
  return invoke<CloudMusicConfig>("cloud_music_config");
}

export function cloudMusicPresets(): Promise<CloudMusicPreset[]> {
  if (mockEnabled()) return Promise.resolve(MOCK_PRESETS);
  return invoke<CloudMusicPreset[]>("cloud_music_presets");
}

export function cloudMusicSetConfig(
  providers: CloudMusicProvider[],
): Promise<CloudMusicConfig> {
  if (mockEnabled()) return Promise.resolve({ providers });
  return invoke<CloudMusicConfig>("cloud_music_set_config", { providers });
}

export function cloudMusicGenerate(
  request: CloudMusicRequest,
): Promise<CloudMusicResult> {
  if (mockEnabled()) {
    return Promise.reject(
      new Error(
        `浏览器预览不生成音频（已选 ${request.provider_id}）。请在桌面端或启动 aidawd 后使用。`,
      ),
    );
  }
  return invoke<CloudMusicResult>("cloud_music_generate", { request });
}

export function cloudMusicTest(provider: CloudMusicProvider): Promise<string> {
  if (mockEnabled()) {
    return Promise.resolve(
      `示例模式：${provider.name} 的地址与 Key 看起来没问题（不会真的发请求）`,
    );
  }
  return invoke<string>("cloud_music_test", { provider });
}

export function cloudMusicBusy(): Promise<boolean> {
  if (mockEnabled()) return Promise.resolve(false);
  return invoke<boolean>("cloud_music_busy");
}

export function cloudMusicCancel(): Promise<void> {
  if (mockEnabled()) return Promise.resolve();
  return invoke<void>("cloud_music_cancel");
}

export function onCloudMusicProgress(
  handler: (progress: CloudMusicProgress) => void,
): Promise<UnlistenFn> {
  if (mockEnabled()) return Promise.resolve(() => {});
  return listen<CloudMusicProgress>(EVENT_CLOUD_MUSIC, (event) =>
    handler(event.payload),
  );
}
