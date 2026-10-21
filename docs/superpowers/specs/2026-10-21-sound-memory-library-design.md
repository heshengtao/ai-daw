# 音色记忆库（整轨音色快照）— 设计

- 日期：2026-10-21
- 状态：📝 设计已确认，待实现
- 相关：`src-tauri/src/sound_memories.rs`（新增）、`src-tauri/src/state.rs`、`src-tauri/src/lib.rs`、
  `src-tauri/src/commands.rs`、`src-tauri/src/mcp.rs`、`src-tauri/src/agent/mod.rs`、
  `src/api/types.ts`、`src/api/tauri.ts`、`src/api/mock.ts`、`src/stores/soundMemories.ts`（新增）、
  `src/stores/layout.ts`、`src/components/workspace/RightPanel.vue`、
  `src/components/workspace/panes/SoundLibraryPane.vue`（新增）、`src/i18n/{zh,en}.ts`、
  `skills/aidaw/SKILL.md`、`skills/aidaw/references/sound-memory.md`（新增）、
  `skills/aidaw/references/sound-design.md`

## 1 背景

内置合成器已经把「捏音色」做成了可用闭环：`list_synth_presets` 先看 36 条出厂音色，
`preview_synth` 离线预听，`set_params` 一次事务落地。但用户自己反复试出来、又加了很多效果器的
「好音色」没有地方存：

- 现有用户预设（`src-tauri/src/presets.rs`、`PresetBar.vue`）只保存**单个插件**的 `params`，
  不保存效果链、不保存插件二进制状态、不保存增益/声像，也没有描述/标签/检索。
- 工程文件里虽然有一整条轨道的乐器 + 效果链，但换一首歌就找不回来；靠记忆重新捏会漂移。
- AI 即使听到/分析过这个音色，也没有跨工程复用的通道。

因此需要一个**独立于工程、独立于现有预设**的音色记忆库：一条记忆就是一条轨道当前音色链的完整
快照，AI 可通过 MCP 存取，用户可在前端查看、编辑、删除。

## 2 目标与非目标

**目标**

- 一条记忆保存：乐器插件（id / 参数 / 可用的二进制状态）+ 有序效果链（同上）+ 轨道增益/声像。
- AI 通过 MCP 保存、检索、应用；保存时由**后端从轨道抓取**，不要求模型复述上百个参数。
- 用户在前端「音色库」面板查看、搜索、编辑元数据、删除；AI 保存后界面自动刷新。
- 默认**不自动保存**：只有用户明确要求，或 AI 询问后用户确认，才写入。
- 应用一条记忆走一次命令事务，只占一个撤销步。

**非目标（v1）**

- 不保存片段、音符、自动化、发送（aux send）、静音/独奏、轨道名。
- 不做云端同步、导入/导出文件格式。
- 不替换现有 `PresetBar` 的单插件预设；两套并存。
- 不做自动打标签、自动清理、相似音色去重。
- 不解决「外部插件二进制状态进入撤销/重做重放」这一引擎既有问题（见 §10）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 快照范围 | 乐器 + 整条效果链 + `gain_db` / `pan` | 用户明确要「乐器参数 + 很多效果器参数」，并选择整轨模板 |
| 谁抓快照 | MCP 工具只收 `track`，后端读工程 | 参数动辄上百个；让模型逐字复述必漏、必截断，还浪费上下文 |
| 与现有预设的关系 | 完全独立的新库（`sound-memories.json`） | 现有预设语义是「单插件快速切参数」；混在一起会破坏老数据与既有约定 |
| 发送怎么处理 | **不保存** | send 的目标是当前工程的轨道 id，跨工程没有意义；只存增益/声像 |
| 插件二进制状态 | 存，base64 字符串；应用时 best-effort 补写 | 外部插件的隐藏状态（采样器样本等）只活在 `state` 里，不存会丢音色 |
| 应用事务 | 一次 `execute_all`：换乐器 → 写参数 → 重建效果链 → 写增益/声像 | 与 `apply_preset_inner` 同一套模式；用户一个撤销步就能退回 |
| 前端入口 | 右侧新增顶级「音色库」标签页（图标 `BookmarkSquareIcon`） | 与「插件 / 下载管理器」同级，不挤压现有面板 |
| 保存触发 | 默认绝不自动保存；用户明确要求或确认后才存 | 用户要求「只有我主动要求记住的时候，AI 才去记住」 |
| 主动提醒 | 只在对话里问一次，不新增前端确认控件 | 用户选择；实现面最小 |
| 变更通知 | 后端广播 `engine://sound-memories` | AI 在后台保存时，已打开的音色库面板要能立即看到 |

## 4 数据模型

新文件 `src-tauri/src/sound_memories.rs`；持久化到应用配置目录的 `sound-memories.json`
（拿不到配置目录时退化为内存模式，重启丢失，与 `PresetStore` 一致）。

```json
[
  {
    "id": "s1760000000000",
    "name": "温暖电影感 Pad",
    "description": "慢起音、宽立体声、带轻微磁带抖动的 Pad，适合副歌铺底。",
    "tags": ["pad", "warm", "wide", "cinematic"],
    "role": "pad",
    "instrument": {
      "plugin_id": "builtin.synth",
      "params": { "attack": 1.2, "release": 2.8, "cutoff": 1800.0 },
      "state": ""
    },
    "effects": [
      {
        "plugin_id": "builtin.chorus",
        "params": { "mix": 0.35, "rate": 0.2 },
        "state": ""
      },
      {
        "plugin_id": "builtin.reverb",
        "params": { "mix": 0.28, "size": 0.7 },
        "state": ""
      }
    ],
    "mixer": { "gain_db": -6.0, "pan": 0.0 },
    "source_project": "Night Drive",
    "created_at": 1760000000000,
    "updated_at": 1760000000000
  }
]
```

Rust 类型：

```rust
pub struct SoundMemory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub instrument: Option<PluginSnapshot>,
    #[serde(default)]
    pub effects: Vec<PluginSnapshot>,
    pub mixer: SoundMixer,
    #[serde(default)]
    pub source_project: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

pub struct PluginSnapshot {
    pub plugin_id: String,
    pub params: BTreeMap<String, f32>,
    /// base64；空字符串表示没有可存的二进制状态（内置合成器/效果器通常为空）。
    #[serde(default)]
    pub state: String,
}

pub struct SoundMixer {
    pub gain_db: f32,
    pub pan: f32,
}
```

- `params` 用 `BTreeMap`，保证序列化顺序稳定。
- `state` 用 base64（新增 `base64` 依赖），避免 `Vec<u8>` 在 JSON 里膨胀成数字数组。
- 列表接口返回 `SoundMemorySummary`（不含 `params` / `state`），只有 `get_sound_memory` 返回完整快照：

  ```rust
  pub struct SoundMemorySummary {
      pub id: String,
      pub name: String,
      pub description: String,
      pub tags: Vec<String>,
      pub role: Option<String>,
      pub instrument_plugin_id: Option<String>,
      pub effect_plugin_ids: Vec<String>,
      pub mixer: SoundMixer,
      pub source_project: Option<String>,
      pub created_at: u64,
      pub updated_at: u64,
  }
  ```

- 名称允许重复；`id` 唯一（`s<毫秒>`，冲突时加 `-<序号>`）。

## 5 后端

### 5.1 存储

`SoundMemoryStore`（`sound_memories.rs`）：

- `load(Option<PathBuf>)`：文件不存在或损坏都当作空库。
- `all()` / `summaries()`：按 `created_at` 升序。
- `get(id)`：完整快照。
- `save(track_snapshot, metadata, overwrite: Option<id>)`：新建或覆盖内容。
- `update_metadata(id, name?, description?, tags?, role?)`：只改元数据。
- `remove(id)`：删不到报错。
- `flush()`：整文件读改写（记忆量级是几十到几百条，不值得增量写）。

`AppState` 增加 `sound_memories: Mutex<SoundMemoryStore>`；`lib.rs` 增加
`sound_memory_file()`，在 `setup` 里与 `init_presets` 相邻调用
`state.init_sound_memories(...)`。

### 5.2 保存快照

Tauri 命令与 MCP 工具共用：

1. 先调用现有的 `capture_plugin_states(state)`（需要从私有改为 `pub(crate)`），
   把外部插件编辑器里的最新二进制状态写进工程。
2. 读取目标轨道：
   - `instrument` → `PluginSnapshot { plugin_id, params, state: base64(instance.state) }`
   - `effects` → 按顺序逐个转换
   - `mixer.gain_db` / `mixer.pan`
   - `meta.role` → 默认 `role`（用户没显式给时使用）
   - 当前工程名 → `source_project`
3. 写入 store，广播 `EVENT_SOUND_MEMORIES`。

### 5.3 应用快照

`commands::apply_sound_memory_inner(state, track, memory, actor) -> (Project, Vec<Event>)`：

1. 组装命令（顺序固定，保证事件流与撤销历史可复现）：
   - 乐器：`SetTrackInstrument { plugin_id }`（快照无乐器时传空 id，引擎会转成
     `TrackInstrumentCleared` 卸载乐器）；随后按参数名排序逐个 `SetParam`。
   - 效果链：先按现有长度循环 `RemoveEffect { index: 0 }`，再按快照顺序
     `AddEffect` + 排序后的 `SetEffectParam`。
   - 混音：`SetGain` / `SetPan`。
2. `engine.execute_all(commands, actor)`。
3. 事务后补写二进制状态：`engine.set_instrument_state(...)`、`engine.set_effect_state(...)`。
   （`set_*_state` 不广播事件、不进撤销历史，这是引擎现有语义。）
4. 返回 project + events；调用方负责 `app.emit(EVENT_ENGINE, ...)` 与
   `state.audio.sync_events(...)`。
5. 广播 `EVENT_SOUND_MEMORIES` 不必要——应用不改变记忆库本身。

**撤销语义**：参数、效果链、增益/声像都在一次 `execute_all` 里，是一个撤销步；
外部插件的 `state` 是事务后补写的，撤销/重做时不会被重放（见 §10）。

### 5.4 Tauri 命令

| 命令 | 返回 |
| --- | --- |
| `list_sound_memories()` | `Vec<SoundMemorySummary>` |
| `save_sound_memory(track, name, description, tags, id?)` | `SoundMemorySummary` |
| `update_sound_memory(id, name?, description?, tags?, role?)` | `SoundMemorySummary` |
| `delete_sound_memory(id)` | `()` |
| `apply_sound_memory(id, track)` | `Project` |

- 写命令（save/update/delete）完成后 `app.emit(EVENT_SOUND_MEMORIES, ())`。
- 错误统一 `Result<_, String>`：名字为空、id 不存在、轨道不存在、参数不是有限数等。
- 应用到不存在的轨道要明确报错，不静默成功。

## 6 MCP 工具面

新增 6 个工具；每个工具自动获得同名 CLI 命令，并可通过 `aidaw_call` 网关调用。

| 工具 | 参数 | 返回 |
| --- | --- | --- |
| `list_sound_memories` | `query?` / `tag?` / `plugin_id?` / `limit?` | 摘要列表 + `total`；`query` 匹配名称、描述、标签、role、plugin_id |
| `get_sound_memory` | `id` | 完整快照（含参数/状态） |
| `save_sound_memory` | `track` / `name` / `description?` / `tags?` / `id?` | 保存后的摘要；传 `id` 表示覆盖该条记忆的内容 |
| `update_sound_memory` | `id` / `name?` / `description?` / `tags?` / `role?` | 更新后的摘要（只改元数据） |
| `delete_sound_memory` | `id` | 删除结果 |
| `apply_sound_memory` | `id` / `track` | 应用后的乐器 / 效果链 / 增益声像摘要 |

- 工具描述里写明「保存前先确认用户意图」「删除只在用户明确要求时」。
- `apply_sound_memory` 的返回要包含可读的效果链顺序与 `undo` 提示，方便模型向用户复述。
- 技能文档新增 `skills/aidaw/references/sound-memory.md`，并在
  `src-tauri/src/mcp.rs::SKILL_DOCS` 注册 `("sound-memory", include_str!(...))`；
  同时更新 `skills/aidaw/SKILL.md` 的能力域表与「约定与坑」。
- `skills/aidaw/references/sound-design.md` 增加一句：捏完满意后，用户要求时可存入音色库；
  下次做同类音色先查音色库。

## 7 前端

### 7.1 数据层

- `src/api/types.ts`：`SoundMemory` / `SoundMemorySummary` / `PluginSnapshot` / `SoundMixer`。
- `src/api/tauri.ts`：5 个封装（列表 / 保存 / 元数据更新 / 删除 / 应用），命名与 Tauri 命令一致
  （camelCase 参数）。
- `src/api/mock.ts`：内存数组 + 与后端一致的增删改查语义，保证浏览器开发模式可用。
- `src/stores/soundMemories.ts`：
  - `refresh()`、`items`、`loading`、`query`、`activeTag`
  - `filtered`（按名称/描述/标签/乐器 id 过滤）
  - `save(track, name, description, tags, id?)`、`updateMetadata(...)`、`remove(id)`、`apply(id, track)`
  - 订阅 `engine://sound-memories` 自动 `refresh()`，面板卸载时取消订阅。

### 7.2 界面

- `src/stores/layout.ts`：`PaneId` 增加 `"sounds"`，`PICKABLE_PANES` 增加
  「音色库 / AI 与你自己保存的音色快照」。
- `RightPanel.vue`：注册 `BookmarkSquareIcon` 与 `SoundLibraryPane`。
- `SoundLibraryPane.vue`：
  - 顶部：搜索框 + 标签 chips + 数量。
  - 卡片：名称、描述、标签、乐器显示名、效果链顺序（`A → B → C`）、保存日期、来源工程。
  - 操作：**应用到选中轨道**（无选中时禁用并提示「先选中一条轨道」）、编辑元数据（内联表单：
    名称/描述/标签）、删除（二次确认）。
  - 空态：说明可以在对话里让 AI 记住当前音色。
  - 应用成功后 toast，提示「已应用，可撤销」。
- 插件/效果器显示名优先用 `usePluginsStore().name(pluginId)`，取不到就显示原 id。
- 补齐 `zh.ts` / `en.ts` 的所有 key；不硬编码中文。

## 8 AI 行为规则

同时写进 `src-tauri/src/agent/mod.rs::system_prompt()` 与技能文档：

- **默认绝不保存。** 只有用户明确说「记住这个音色 / 存起来 / 加到音色库」，或 AI 询问后用户
  明确确认，才调用 `save_sound_memory`。
- 用户表达满意（「这个音色真好」「就用这个」「这个设置很舒服」）时，可以问**一次**
  「要存进音色库吗？」；用户不确认就不存，也不在同一轮反复追问。
- **绝不自动删除或清理**；`delete_sound_memory` 只响应用户明确要求。
- 用户要求「来个 XX 音色」时，先 `list_sound_memories(query=...)` 查用户存过的，再考虑从零捏。
- 保存时自动补一句中文 `description` 和 2–5 个 `tags`；标签优先用角色/风格/听感
  （`pad` / `warm` / `wide` / `cinematic`），不要只写插件名。
- 应用前先用摘要确认是哪一条；应用后告诉用户改了哪条轨道的乐器、效果链和增益/声像。

## 9 测试

- `sound_memories.rs` 单测：
  - 文件往返（含 `state` base64、`params` 顺序、`mixer`）
  - 唯一 id（同毫秒多次保存）
  - 空名字拒绝、删除不存在报错
  - 覆盖保存保留 `created_at`、更新 `updated_at`
  - 元数据更新不动 `instrument` / `effects`
- `commands.rs` / MCP 单测：
  - 从一条带乐器 + 两个效果器 + 非零增益/声像的轨道保存，快照逐值正确
  - 应用到一个空轨道后，乐器、效果链顺序、参数、增益/声像逐值一致
  - 应用时覆盖已有轨道效果链（旧效果被清掉）
  - `list_sound_memories` 的 query/tag/plugin 过滤
- 守卫测试：`mcp.rs` 的工具数与 `definitions()` 一一对应；`SKILL_DOCS` 新域非空。
- 前端：`pnpm build` / 类型检查。
- 手动验收：
  1. AI 捏一个音色 + 效果链 → 说「记住」→ 音色库出现该条。
  2. 换一条轨道应用 → 音色/效果链/增益声像一致 → 撤销一次全部恢复。
  3. 删除后列表消失，重启应用不复活。
  4. AI 未获确认时不会自动保存。

## 10 已知边界与后续

- **外部插件二进制状态不在撤销历史里**。应用记忆后撤销，参数、效果链、增益/声像会恢复，
  但外部插件的 opaque `state` 不会重放（引擎现状：`set_*_state` 不产生事件）。
  内置合成器/效果器只用参数，不受影响。后续若要让 state 也参与撤销，需要给 `Command` /
  `Event` 增加状态变体，并评估事件体积。
- 快照里的插件 id 如果在新机器/新工程里不存在，应用后会得到一条「缺失插件」轨
  （与打开老工程的行为一致）；回执里应提示这一点。
- v1 不保存发送、自动化；如果以后要支持，需要先定义「如何把目标映射到新工程」。
- v1 不做相似音色检测；同一音色可能被重复保存多条。
- 标签是自由字符串，前端只做展示与包含匹配，不做同义词归一化。
