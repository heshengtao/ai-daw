# 灵感视图 — 生成结果可选导入 + 提示词引导 — 设计

> **状态（2026-11-20）**：设计已确认，待实施。前置设计见
> `2026-11-18-inspire-view-design.md`（灵感页本体）。

## 1 背景

两件事促成了这份设计。

**其一，生成结果没有「不要」这个选项。** `ai_yue_compose` 跑完会在后端**无条件**执行一次
`AddAudioTrack`（续写时 `AddAudioClip`），把 wav 摆成工程里的一条轨（`crates/daemon/src/yue_engine.rs`
约 1780–1821 行）。同一段代码上方紧挨着一份相反的先例：ABC 谱面**故意不自动导入**，注释里写明
「铺进工程只会多出一堆没用的轨」，需要的人点按钮显式导入。音频缺的正是这个显式动作——用户只想
先把歌听一遍，不想每次都在编排页里多一条轨。

**其二，风格提示词写错了语言。** 中文界面的占位符教用户用中文描述
（`例如：1930 年代摇摆爵士，大乐队与拉格泰姆钢琴…`，`src/i18n/zh.ts:2564`），但 YuE2 的 `style`
是英文 tag 分布：上游文档的每一个示例都以语言标签开头、以 BPM 收尾。实测「一首爵士乐」生成出来的
是 Bb 调的标准流行歌（intro/verse/chorus、I–vi–IV–V、直八分），风格完全没跟上。占位符在引导用户
走一条注定失败的路。

## 2 目标与非目标

**目标**

- 灵感界面生成的歌**只落在灵感页**，不再自动进编排；卡片上给一个「导入到工程」按钮，导入变成可选的。
- 卡片能看出「已导入」，且这个状态在重启、删轨、老记录上都正确。
- 占位符改成英文示范，把「语言标签 + 乐器 + BPM」这个正确写法直接摆在用户面前。
- 输入框上方给一排预设风格芯片，点一下插完整的英文 tag 串。
- 「纯音乐 / 有人声」变成显式开关，不再靠「歌词是否为空」隐式推导。

**非目标**

- 不动 AI 面板（`AiYuePage`）与 MCP / agent 的自动导入——它们的语义就是「下命令让工程多一轨」。
- 不加人声语言标签行（已讨论后放弃）。
- 不改 record JSON 结构，不做数据迁移。
- 不做「导入到指定轨道」；导入一律新建一条独立音频轨。

## 3 变更一：生成结果可选导入

### 3.1 后端开关

`ComposeArgs`（`crates/daemon/src/yue_engine.rs`）新增字段：

```rust
/// 生成完是否把音频摆进工程（默认 true）。
/// 灵感界面传 false：先出歌、再决定要不要进编排。
pub place_in_project: Option<bool>,
```

结构体已有 `#[derive(Default, serde::Deserialize)]` + `#[serde(default)]`，新字段对老调用方为零影响。
取值处用 `.unwrap_or(true)`，即**缺省仍然自动导入**，AI 面板与 MCP 行为不变。

摆轨那一段（`run_agent_all(app, commands)` 之前）按开关包一层：`false` 时既不 `AddAudioTrack`
也不 `AddAudioClip`，但 **wav 落盘与 record 写入照旧**——音频文件必须先存在，否则后面的手动导入
没有东西可导。`audio_path` / `seconds` / `appended` 这些返回值同样照填，界面才能拿到时长。

### 3.2 手动导入命令

新增与 `ai_yue_import_abc`（同文件 179 行）同形状的命令：

```rust
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn ai_yue_import_audio(
    app: crate::tauri_api::AppHandle,
    path: String,
    name: Option<String>,
) -> Result<String, String>
```

实现步骤：

1. `state.assets.load(&path)` 解码，拿 `sample_rate` 与 `frames`；解不开就回错误。
2. 读工程当前 tempo，`beats = seconds * tempo / 60.0`（与 `ai_yue_compose` 里 1743–1747 行同一套换算）。
3. 轨道名：用 `name`（前端传记录的标题），没有就退回文件名去扩展名——录出来的文件名是
   `yue2-1795187559510.wav` 这种，直接当轨道名太难认。
4. 发一条 `EngineCommand::AddAudioTrack { start: 0.0, length: beats.max(1.0), at: None }`。
5. 返回**轨道名**（`String`），不是整句中文——提示语由前端 i18n 拼，英文界面不该弹中文。

不做幂等：重复调用就是再建一条轨，符合「同一首歌可以放两处」的预期。

### 3.3 前端

- `src/api/types.ts` 的 `YueComposeArgs` 加 `place_in_project?: boolean`。
- `src/stores/inspire.ts` 的 `craftLocal` 显式传 `place_in_project: false`。
  云端路径不受影响（云端本来就不碰工程）。
- `src/api/tauri.ts` 加 `yueImportAudio(path: string, name?: string): Promise<string>`；`src/api/mock.ts`
  加桩，预览模式下回一个假轨道名。

### 3.4 「已导入」状态从工程派生

**不新增持久化字段**。判断依据是工程里是否已有 clip 引用该 `item.path`：

```ts
function placedOf(item: InspireItem): boolean {
  return (project.project?.tracks ?? []).some((track) =>
    track.clips.some((clip) => clip.audio?.path === item.path),
  );
}
```

这个查询在 `hueOf()`（`src/stores/inspire.ts:251`）里已经写过一遍，抽出来两边共用。好处：

- 老记录（本改动之前生成、已被自动导入过的）状态自动显示为「已导入」，无需迁移。
- 重启后从 localStorage 恢复的记录照样正确，因为真相在工程里而不在记录里。
- 用户把轨道删掉，按钮自动变回「导入到工程」。

### 3.5 卡片交互

- `InspireRow.vue`：在标题右侧徽章旁加一个导入按钮（`ArrowDownTrayIcon`），`emit("import")`。
  按钮 `aria-label` 与 `Tooltip` 文案随状态切换。
- `InspireLibrary.vue`：透传 `@import` 到上层。
- `InspireView.vue` / store：执行 `importToProject(item)` → `await yueImportAudio(item.path)` →
  `layout.notify` 提示。命令抛错时提示错误、状态不变（仍显示未导入），不误标。

文案：

| key | 中文 | 英文 |
|---|---|---|
| `inspire.import` | 导入到工程 | Add to project |
| `inspire.imported` | 已导入 · 再导入一次 | Added · add again |
| `inspire.importedToast` | 已导入为轨道「{name}」 | Added as track "{name}" |

## 4 变更二：提示词引导与预设标签

### 4.1 占位符

中英界面都用英文占位符（这是刻意的：占位符本身就是一份可抄的范式）：

```
English, warm female jazz vocal, upright bass, brushed drums, tenor saxophone, 96 BPM
```

`inspire.style` 标签文字保持各语言（中文仍是「风格描述」）。中文界面在输入框下方加一行提示
`inspire.styleHint`：「用英文 tag 描述最稳，逗号分隔；开头写语言、结尾写 BPM」。英文界面同一句话。

### 4.2 预设风格芯片

新文件 `src/components/workspace/inspire/styles.ts`：

```ts
export interface StylePreset { id: string; label: string; tags: string }
// label 是 i18n key（inspire.stylePreset.<id>），tags 是英文常量
export const STYLE_PRESETS: StylePreset[]
```

默认十个（tag 串含乐器与 BPM，点一下就是一份可用的 style）：

| id | 中文 | tags |
|---|---|---|
| `jazz` | 爵士 | `swing jazz, walking upright bass, brushed drums, comping piano, tenor saxophone, 96 BPM` |
| `pop` | 流行 | `contemporary pop, bright acoustic guitar, punchy drums, warm bass, catchy hook, 104 BPM` |
| `rock` | 摇滚 | `indie rock, overdriven electric guitar, live drums, driving bass, 128 BPM` |
| `lofi` | Lo-fi | `lo-fi hip hop, dusty vinyl crackle, mellow Rhodes piano, soft boom-bap drums, 78 BPM` |
| `citypop` | City Pop | `city pop, slap bass, chorus electric guitar, gated reverb drums, dreamy pads, 112 BPM` |
| `edm` | 电子 | `melodic house, four-on-the-floor kick, sidechained pads, plucky synth arp, 124 BPM` |
| `hiphop` | 说唱 | `boom bap hip hop, sampled soul horns, dusty drums, deep sub bass, 88 BPM` |
| `cnfolk` | 国风 | `Chinese folk, guzheng, dizi flute, erhu, modern arrangement, 90 BPM` |
| `cinematic` | 配乐 | `cinematic orchestral, string swells, epic brass, timpani, choir pads, 72 BPM` |
| `ambient` | 氛围 | `ambient, warm analog pads, slow evolving textures, no drums, 60 BPM` |

**交互**：芯片是可切换的。点击未选中的 → 把 `tags` 追加到输入框（空则直接填，否则 `", " + tags`）；
点击已选中的 → 从输入框里删掉这段子串，并规整残留的连续逗号 / 首尾逗号。选中态用
`prompt.includes(tags)` 判定，不做额外状态，用户手改提示词也不会和芯片状态打架。

样式复用歌词区 `[Verse]` 那排芯片的 `.tags` / `.chip`（`m3-chip tag`），不新造控件。

### 4.3 纯音乐 / 有人声开关

用 `Md3Segmented`（与「简单/进阶」同一个控件）放在参数区：

- **有人声**（默认）：歌词照常提交；歌词留空时沿用现有语义——本地让模型自己写词，
  提示语仍是 `inspire.lyricsAuto`。
- **纯音乐**：歌词输入区禁用（内容保留不删，切回来还在），提交时 `lyrics: ""`，并在提示词末尾
  补 `instrumental`（去重：已经有了就不重复加）。

对应 `CraftRequest.instrumental`：本地映射为「是否传歌词」，云端映射为它的 `instrumental` 参数。
原来 `submitLocal` 里的 `instrumental: !lyrics.value.trim()` 推导删除，改为读开关。

### 4.4 与后端字段的关系

`ComposeArgs.instrumental` 不存在，纯音乐在后端就是 `lyrics` 为空；因此这一项**只改前端**，
不需要动 RPC 协议。`instrumental` 补进 style 是给模型的条件提示，不是给后端的开关。

## 5 文件清单

**后端**

- `crates/daemon/src/yue_engine.rs`：`ComposeArgs` 加字段；摆轨段加判断；新增 `ai_yue_import_audio`；
  单测补一条「`place_in_project` 缺省 = true」的反序列化断言。

**前端**

- `src/api/types.ts`：`YueComposeArgs.place_in_project`
- `src/api/tauri.ts`：`yueImportAudio`
- `src/api/mock.ts`：`yueImportAudio` 桩
- `src/stores/inspire.ts`：`craftLocal` 传 `false`；抽 `placedOf`；`importToProject`
- `src/components/workspace/inspire/styles.ts`（新）
- `src/components/workspace/inspire/InspireComposer.vue`：占位符、芯片行、纯音乐开关
- `src/components/workspace/inspire/InspireRow.vue`：导入按钮
- `src/components/workspace/inspire/InspireLibrary.vue`：透传事件
- `src/views/InspireView.vue`：落 `importToProject`
- `src/i18n/zh.ts`、`src/i18n/en.ts`：占位符改写 + 上述新 key

## 6 错误处理与边界

- **音频文件被删**：`ai_yue_import_audio` 解码失败 → 前端 toast 报错，卡片保持「未导入」。
- **工程还没加载**：`placedOf` 返回 false，按钮可用；导入命令内部自己读工程，不依赖前端快照。
- **云端生成的歌**：`path` 指向下载文件，导入走同一条命令，行为一致。
- **预览模式（mock）**：导入返回固定假轨道名，`placedOf` 恒为 false，按钮永远停在「导入到工程」。
- **时长换算**：工程 tempo 变了，已导入片段的拍长不会变（与现有 `add_audio_clip` 行为一致），不在本次处理。
- **续写（带 `semantic_prefix`）不经过手动导入**：续写结果要裁掉重渲染的前缀，裁剪信息只在
  `ai_yue_compose` 内部；而续写只从 AI 面板 / MCP 触发，这两条路径保持 `place_in_project = true`，
  所以手动导入不需要处理前缀偏移。若将来让灵感页支持续写，这里要重新设计。

## 7 验证

- `cargo test`（`crates/daemon`）：全绿，含新加的反序列化断言。
- `npm run build`：`vue-tsc --noEmit` 类型检查通过。
- 手动：生成一首 → 编排界面**不**出现新轨 → 卡片点「导入到工程」→ 新轨出现、按钮变「已导入 ·
  再导入一次」→ 再点一次多一条轨 → 删掉轨道，按钮回到「导入到工程」。
- 手动：点「爵士」芯片 → 提示词变成完整英文 tag 串 → 生成结果听得出来是爵士而不是流行歌。
- 手动：切「纯音乐」→ 歌词区禁用、提示词末尾出现 `instrumental`；切回「有人声」歌词还在。

## 8 为什么不选其它做法

- **在 `AddAudioTrack` 之前加确认弹窗**：每次生成都打断，比现在的自动导入更烦。
- **新增「已导入」持久化字段**：要迁移、会在删轨后说谎；从工程派生没有这两个问题。
- **搞一个风格预设下拉 / 弹层**：芯片一行就能看全，点一下即插，少一次交互。
- **预设插中文再翻译**：翻译层本身就是不可靠环节，直接给正确语言的字符串更实在。
