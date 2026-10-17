# 插件语义目录：让模型知道「JC303 是贝斯」 — 设计

- 日期：2026-10-17
- 状态：✅ 已实现
- 相关：`src-tauri/src/plugin_info.rs`（新增）、`src-tauri/src/mcp.rs`（`list_plugins` / `describe_plugin` /
  `set_track_instrument` / `add_effect` / `remove_effect`）、`src-tauri/src/commands.rs`
  （`plugin_params_with`）、`skills/aidaw/references/plugin.md`、`skills/aidaw/SKILL.md`、
  `skills/aidaw/references/track.md`

## 1. 背景：一次真实的误用

写歌智能体拿 `JC303` 当普通合成器用（写和弦、铺底），而不是贝斯。查一下当时 `list_plugins` 的返回：

```json
{ "id": "vst3:…", "name": "JC303", "vendor": "midilab", "kind": "instrument",
  "format": "vst3", "accepts_midi": true, "has_editor": true, "path": "…/JC303.vst3" }
```

`kind: instrument` 只说明「它是音源」，不说明「它是**单音酸性贝斯**」。模型没有任何可用的语义，
只能从名字猜——而 `JC303` 这个名字对不熟悉 TB-303 的模型等于随机字符串。

顺着这条线还发现两个同等严重的缺口：

1. **内置插件根本不在目录里**。`list_plugins` 返回的是 `state.catalog_snapshot()`，也就是扫描结果；
   `builtin.synth` 与 5 个内置效果器只活在前端（`src/data/plugins.ts` 合并）。模型看不到这台通用合成器。
2. **名字与 id 之间没有桥**。模型习惯把名字当 id 传（`set_track_instrument(plugin_id="JC303")`），
   引擎不校验 id，于是挂上一个「缺失插件」——轨道静音，且没有任何提示。

## 2. 目标与非目标

**目标**：模型在**听不到声音**的前提下，能判断一件插件「是什么、适合干什么、怎么挂上去」；
判断不了时**诚实说明**，而不是编一个音色。

**非目标**：不做试听/音频指纹识别；不改插件扫描器（不往 `CatalogPlugin` 里加字段、不动磁盘缓存格式）；
不改界面侧插件选择器（前端仍用 `list_plugins` 命令拿扫描结果 + 内置表合并）。

## 3. 决策

| 决策 | 选择 | 理由 |
| --- | --- | --- |
| 语义从哪来 | 三级：**内置表 → 收录型号 → 关键词 + `moduleinfo.json` 兜底** | 越靠前越可信；兜底宁可说「未收录」也不编 |
| 语义存哪 | 运行时计算（`plugin_info::brief`），**不进** `CatalogPlugin` | 缓存文件与前端 `CatalogPlugin` 类型都不用动；描述属于展示层，不该污染持久化契约 |
| 收录表收录什么 | 只写**有把握**的型号（血统明确、性格公认） | 一条编造的描述比「未收录」危害大得多——模型会当真 |
| 怎么读 `moduleinfo.json` | 手写扫描 `"Sub Categories"` 数组，不用 `serde_json` | 这些文件由插件构建工具生成，带**尾随逗号**（非法 JSON），严格解析必然失败 |
| 名字解析 | id → 精确名 → 归一化（去空格/连字符/大小写）→ 唯一子串 → 中文子串；**多重匹配报错** | 模型把名字当 id 是常态；但 KR-106 同时有 VST3/CLAP 两份，替它选会掩盖问题 |
| 写操作要不要带语义 | **带**：`set_track_instrument` 的回执直接说明「这是什么 + 建议分工/音域」 | 即使模型没调 `list_plugins`，挂乐器这一步也能让它学到；比额外一次工具调用便宜 |
| 内置插件进目录 | 是（合成器 + 5 条老预设 id + 5 个效果器） | 模型需要知道「有一台通用合成器可用」，否则只会去挑外部插件 |
| 顺手补效果链工具 | 是：`add_effect` / `remove_effect` | 能看到效果器却没有**非破坏性**的挂载方式（`apply_preset` 的 `effects` 是整体替换），等于看得到用不了 |

## 4. 工具面变化

| 工具 | 变化 |
| --- | --- |
| `list_plugins` | 从「扫描结果原样回传」→「内置 + 外部，带 `category` / `description` / `roles` / `pitch_range` / `mono` / `tags`」，支持 `kind` 过滤与 `query` 搜索 |
| `describe_plugin` | **新增**：单插件详情 + **实时参数表** + 用法提示（参数表要 dlopen，走 `spawn_blocking`） |
| `set_track_instrument` | 接受 id **或名字**；挂错类别（把效果器当乐器）直接报错；回执带语义 |
| `add_effect` / `remove_effect` | **新增**：效果链末尾追加 / 按下标移除，回执给出可读的当前链 |
| `set_param` | 描述改为指向 `describe_plugin` 查参数表 |

## 5. 数据形状

```json
{
  "counts": { "instruments": 6, "effects": 6 },
  "instruments": [
    {
      "id": "vst3:ABCDEF…",
      "name": "JC303",
      "vendor": "midilab",
      "kind": "instrument",
      "format": "vst3",
      "builtin": false,
      "category": "bass",
      "description": "TB-303 克隆：单音酸性贝斯合成器（锯齿/方波 → 24 dB 低通 + 包络量 + 共振，accent / slide 做重音与滑音，另带过载）。写单音 bass line，不要叠和弦；典型音域 E1–E4。",
      "roles": ["bass"],
      "pitch_range": [28, 64],
      "mono": true,
      "tags": ["acid", "303", "tb-303", "bass", "贝斯", "单音", "mono", "house", "techno"]
    }
  ],
  "effects": [],
  "hint": "category / description / roles 说明「这件东西是干什么的」；pitch_range 是建议写作音域（MIDI 音高）；mono=true 是单音乐器，不要叠和弦。要某个插件的参数表用 describe_plugin。别凭名字猜音色——认不出的条目会自己说明「未收录」。"
}
```

未收录的外部插件**不编音色**，描述形如：

> 外部效果器（VST3，厂商 FX-Mechanics），未收录具体型号。按名称与插件自报信息判断属于「空间/声场」。
> 插件自报分类：Fx / Spatial。要判断实际音色/效果，用 `describe_plugin` 看参数表，或挂上后
> `analyze_project` 确认听感——不要凭名字猜。

## 6. 测试

- `plugin_info` 12 条单测：JC303 是单音贝斯、名字匹配忽略大小写/空格/连字符、KR-106 是复音合成器、
  RdPiano 是电钢、jdrummer 是鼓、未收录必须说「未收录」、效果器拿到效果分类且没有分工、内置目录完整
  （11 条）、名字/id/片段解析、歧义报错、失败时列出可用清单、归一化规则。
- MCP 守卫测试同步更新（工具数与 `definitions()` 一一对应，共 59 个工具）。
- 手动诊断：`dump_installed_plugins`（`#[ignore]`）打印本机插件卡片 + 参数表，用来核对收录表。
- 验证：`cargo test --manifest-path src-tauri/Cargo.toml --lib` → 73 passed；`cargo check` 干净。

## 7. 已知边界

- 收录表要人工维护。新装一个没收录的型号，只能靠关键词/子分类兜底——这是**有意的**：宁可信息少，
  不要信息错。
- 内置合成器的 32 条出厂音色表只在**前端**（`src/data/plugins.ts`，名字还是 i18n key）。Rust 侧只有
  5 条引擎级预设 id，所以模型能调参数、能用老 id，但**不能按名字**选「电钢琴 / 闷音吉他」这类音色。
  后续可把预设表下沉到引擎（`BUILTIN_PRESETS` 扩容），让 `list_plugins` 直接列出音色。
- 描述是中文。MCP 输出面向模型，可接受；若将来要在界面上展示，需要 i18n。
