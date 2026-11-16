# 云端音乐生成：供应商适配说明

这份文档记录「音乐生成」里云端 API 的接入方式，供后续加供应商 / 排查问题用。

## 1. 架构（参考 ai-cut 的供应商适配）

ai-cut 把云端图像 / 视频 / TTS 拆成同一套四层：

1. **配置层**：一家供应商一个条目，落 `*.json`，字段含 `base_url` / `api_key` / `model`；
2. **预设层**：`kind` 决定默认地址、默认模型、能力与文档链接；
3. **调用层**：按 `kind` 分派到各家的 HTTP 实现，统一成「请求 → 音频字节」；
4. **结果层**：落盘、解码、作为一条新轨加入工程，进度走统一事件。

ai-daw 的 `crates/daemon/src/cloud_music.rs` 沿用同一套形态：

```text
<配置目录>/cloud-music.json     # 供应商配置（明文密钥）
<配置目录>/ai-music/            # 生成结果（wav / mp3）
事件 cloud-music-progress       # preparing / running / done / error / cancelled
```

前端：

- `src/components/settings/MusicApiSettings.vue`：设置 →「音乐 API」；
- `src/stores/cloudMusic.ts`：供应商列表 + 当前选中 + 生成状态；
- `src/components/workspace/panes/ai/AiCloudMusicPage.vue`：云端服务的描述 / 歌词 / 时长 / 纯音乐表单；
- `src/components/workspace/panes/AiPanel.vue`：「AI 写歌」卡片的下拉；第一项是「设置音乐 API…」，下面分「本地模型 / 云端 API」两组。

## 2. 已接入的七家（均以官方公开文档为准）

| kind | 服务 | 官方文档 | 关键请求 |
|---|---|---|---|
| `lyria` | Google Lyria 3.5 | <https://ai.google.dev/gemini-api/docs/music-generation> | `POST {base}/v1beta/models/{model}:generateContent`，`x-goog-api-key`，`responseModalities: ["AUDIO","TEXT"]` |
| `elevenlabs` | ElevenLabs Music | <https://elevenlabs.io/docs/api-reference/music/compose> | `POST {base}/v1/music?output_format=mp3_48000_192`，`xi-api-key`；有歌词时用 `composition_plan` |
| `mureka` | Mureka | <https://platform.mureka.ai/docs/> | `POST {base}/v1/song/generate`（歌词）/ `/v1/song/easy-generate`（提示词）/ `/v1/instrumental/generate`（纯音乐），轮询 `.../query/{id}` |
| `treblo` | Treblo（原 Sonauto） | <https://treblo.com/developers/docs> | `POST {base}/v1/generations/v3`，轮询 `/v1/generations/status/{id}`，取 `/v1/generations/{id}` 的 `song_paths` |
| `alibaba` | 阿里云百炼 Fun-Music | <https://help.aliyun.com/zh/model-studio/fun-music-api> | `POST {base}/api/v1/services/audio/music/generation`，Bearer，`model` + `input` |
| `volcengine` | 火山引擎 豆包音乐 | <https://www.volcengine.com/docs/84992/1967910> | `POST {base}/?Action=GenSongForTime&Version=2024-08-12`，AK/SK 签名；纯音乐用 `GenBGMForTime` |
| `tencent` | 腾讯云 MPS（MiniMax） | <https://cloud.tencent.com/document/product/862/133520> | `CreateAigcAudioTask` / `DescribeAigcAudioTask`，TC3-HMAC-SHA256 签名 |

默认值：

- Lyria：`https://generativelanguage.googleapis.com`，`lyria-3.5`，最长 180 秒；
- ElevenLabs：`https://api.elevenlabs.io`，`music_v2_5`，最长 600 秒（以套餐为准）；
- Mureka：`https://api.mureka.cn`（海外站改成 `https://api.mureka.ai`），`auto`，约 300 秒；
- Treblo：`https://api.treblo.com`，最长 285 秒，`length_range` 两端取 30 的倍数；
- 阿里云百炼：`https://YOUR_WORKSPACE.cn-beijing.maas.aliyuncs.com`（把 `YOUR_WORKSPACE` 换成业务空间 ID），`fun-music-v1`；
- 火山引擎：`https://open.volcengineapi.com`，`v5.0`，AK/SK；
- 腾讯云：`https://mps.tencentcloudapi.com`，`MinimaxMusic`，SecretId/SecretKey。

### 歌词怎么进

- **Lyria**：歌词拼进 prompt，并要求保留段落标签；返回的文本 part 里也会带生成的歌词。
- **ElevenLabs**：有歌词时走 `composition_plan.sections[].lines`；失败会退回「歌词写进 prompt」。
- **Mureka**：歌词走独立 `lyrics` 字段；留空则走 `easy-generate` 让模型自己写。
- **Treblo**：歌词走独立 `lyrics` 字段；`instrumental=true` 时不传歌词。
- **阿里云百炼**：`input.lyrics`；纯音乐用 `is_instrumental=true`。
- **火山引擎**：人声歌曲传 `Lyrics`，纯音乐走 `GenBGMForTime` 的 `Text`。
- **腾讯云**：歌词放进 `AdditionalParameters`（JSON 字符串）；不传则生成纯音乐。

### 为什么没有 Suno / Udio / Stability / Mubert / Soundraw / Beatoven

- **Suno**：只有邀请制 API Beta 与 MCP，没有公开自助 REST API；非官方中转违反其条款。
- **Udio**：没有公开 API，且已关闭下载。
- **Stability / Mubert / Soundraw / Beatoven**：官方 API 存在，但定位是纯音乐 / 配乐，不支持人声与歌词；
  本次只接「人声 / 歌词」这一组。
- **MiniMax Music 3**：直连 API 已对新用户下线，走腾讯云 MPS 或自托管开放权重。

## 3. 加一家新供应商的步骤

1. `CloudMusicKind` 加枚举值；
2. `CloudMusicKind::preset()` 补默认地址 / 模型 / 最长秒数 / 文档链接；
3. `cloud_music_presets()` 的数组里加一项；
4. `generate_inner()` 的 `match` 里加分支，实现 `generate_xxx() -> Generated`；
5. `cloud_music_test()` 里补一个轻量 GET（验证地址与 Key）；
6. 前端补 `settings.music.kind.<kind>` / `settings.music.note.<kind>` 两组文案；
7. 若命令签名新增参数类型，记得同步 `crates/daemon/src/server/cmds.rs` 的手写 RPC 分支。

## 4. 已知边界

- 只支持**同步返回或轮询取结果**的 API；不支持 WebSocket 实时流（Lyria RealTime 未接）。
- 一次只跑一个生成任务（全局 `running` 标志），与本地写歌的引擎任务互不抢占。
- 结果按当前工程速度换算成拍；生成结果本身不带 BPM / 调性元数据。
- 生成的文件在配置目录的 `ai-music/`，不会自动清理；需要的话后续加保留策略。
