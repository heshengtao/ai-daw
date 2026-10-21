# 域：乐器与效果器（plugin）

**两步走，别一次倒光**：

1. `list_plugins` —— **只看描述**：分类、一句话音色、推荐分工与建议音域。**不带参数、不带预设**。
2. `describe_plugin` —— **只看一件**：这件插件的**全部音色参数**（id / 名称 / 范围 / 默认值）+
   它的**用户预设**（名字 + 参数快照）。

插件名通常只是型号（`JC303`、`KR-106`），名字里没有「贝斯」「铺底」这些信息。所以**挑音色前先调
`list_plugins`，确定要用哪一件之后再 `describe_plugin`**——不要反过来，更不要凭名字猜。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `list_plugins` | `kind?`,`query?` | 全部可用乐器/效果器（内置 + 本机 VST3/CLAP），**只给描述**。`kind` = `instrument` / `effect` / `all`；`query` 按名字/厂商/分类/描述/标签过滤。 |
| `rescan_plugins` | — | 重新扫描本机插件目录（装完新插件后调用）。较慢。 |
| `describe_plugin` | `plugin_id`,`query?`,`max_params?` | 单件的全部音色参数 + 用户预设 + 用法提示。`query` 只在**这一件**内部缩小范围；MIDI CC 映射这类管道参数不返回（用 `param_hidden` 报个数）。 |
| `list_synth_presets` | `category?`,`query?` | **内置合成器的 36 条出厂音色**：id / 中文名 / 分类 / 描述 / 完整参数补丁。捏音色从它开始，不用猜。 |
| `set_track_instrument` | `track`,`plugin_id` | 挂/换乐器；`plugin_id` 可以是 id 或名字（`JC303`）。回执里会说明这件乐器是什么、建议怎么用。 |
| `set_param` | `track`,`param`,`value` | 调**一个**乐器参数（`param` 用 `describe_plugin` 查）。 |
| `set_params` | `track`,`params`,`reset?` | **一次写入整组**乐器参数（一个撤销步）；`reset: true` 先复位到默认再写。捏音色/切音色用它，不要循环 `set_param`。 |
| `preview_synth` | `params`,`reset?`,`track?`,`note?`,`duration_ms?` | 离线试听**内置乐器**：渲染一个音，返回响度 + 音色指纹（频谱重心 / 平坦度 / 波峰因数 / 分段能量）与生效后的完整参数。不写工程。 |
| `add_effect` | `track`,`plugin_id` | 效果器追加到链尾（id 或名字）。 |
| `remove_effect` | `track`,`index` | 移除效果链上第 `index` 个（0 起，链头是 0）。 |
| `move_effect` | `track`,`from`,`to` | 调整效果器在链中的顺序（0 起）。 |
| `set_effect_param` | `track`,`index`,`param`,`value` | 调**一个**效果器参数（`param` 用 `describe_plugin` 查）。 |
| `set_effect_params` | `track`,`index`,`params` | **一次写入**一个效果器的整组参数（一个撤销步）——调混响/延迟/压缩的主入口。 |
| `apply_preset` | `track`,`params`,`effects` | 一次写整组乐器参数 + 整条效果链（`effects` 是**替换**，不是追加）。 |
| `list_presets` | — | 全部**用户预设**（跨工程）：id / 名字 / 归属插件 / 参数快照。 |
| `save_preset` | `name`,`plugin_id`,`params` | 把一组参数快照存成新的用户预设。 |
| `update_preset` | `id`,`name`,`params` | 覆盖一条用户预设（参数快照整体替换）。 |
| `delete_preset` | `id` | 删除一条用户预设。 |

**预设怎么用**：`describe_plugin` 返回的 `presets.items[].params` 就是「这条音色怎么调」，
可以原样喂给 `set_params(track, params, reset=true)` 复用；`id` 只用于引用，不能直接当 `preset_id` 传。
只有确实要**连效果链一起换**时才用 `apply_preset`——它的 `effects` 是整体替换，不传会清空现有链。

**目录里每条的含义**

- `category`：用途分类（`bass` / `drums` / `reverb` / `dynamics` …），`description` 是这件东西的性格与用法。
- `roles`：适合的编曲分工（与 `set_track_meta` 的 `role` 同一套词：`bass` / `chords` / `lead` / `pad` / `drums` / `fx`）。
- `pitch_range`：建议写作音域（MIDI 音高）。**乐器挂上后建议用 `set_track_meta` 写进轨道**，之后
  `fit_notes_to_range` 不给参数时会照它折返。
- `mono: true`：单音乐器——**同一时刻只发一个音，不要叠和弦**（叠了会被抢占或吞掉）。

**几个常见误区**

- 名字里没有分类：`JC303` 是 TB-303 克隆（**单音酸性贝斯**），不是「一台普通合成器」；`KR-106` 是
  Juno-106 克隆（复音，pad / 贝斯 / 主音）；`RdPiano` 是 Rhodes 电钢。这类判断交给 `list_plugins`。
- 「未收录」不是错误：外部插件没被收录时，描述会明说「未收录具体型号」并给出类别兜底。这时用
  `describe_plugin` 看参数表，或挂上后 `analyze_project` 确认听感——**不要自己补一个音色描述**。
- 内置合成器只有一个：`builtin.synth`。界面上的 36 条音色是它的参数补丁，用 `set_params(track, params, reset=true)` 调出；
  MCP 侧先 `list_synth_presets` 取完整参数，或直接读界面选中后的 `track.instrument.params`；
  `builtin.synth.lead/.bass/.drums/.strings/.pad` 是老 id，仍可直接挂。
- **捏音色的完整知识在 `sound-design.md`**：参数的实际映射（cutoff 是归一值不是 Hz）、预听指标怎么读、
  各风格配方、以及这台合成器做不到的事（没有调制矩阵 / 波表 / 真正的每副本立体声 unison）。
  调音色前先 `aidaw_help("sound-design")`，不要靠猜。
- **内置采样乐器**（随包内嵌，立即可用，不需要扫描/安装）：

  | id | 名称 | 特点 | 建议音域 |
  | --- | --- | --- | --- |
  | `builtin.piano` | 原声钢琴 | FreePats Upright Piano KW · SF3 · mono | 21–108 |
  | `builtin.nylon_guitar` | 尼龙弦木吉他 | Spanish classical guitar · SF3 · 48 半音采样 | 40–88 |
  | `builtin.electric_guitar` | 电吉他（清音） | FSBS clean · SF3 · 13 键位采样 | 35–86 |
  | `builtin.bass` | 指弹贝斯 | Finger Bass YR · SF3 · 12 半音采样 | 28–48 |
  | `builtin.drums` | 原声鼓组 | MuldjordKit lite · SF2 · **GM 35–59** · 3 层力度 | 35–59 |
  | `builtin.strings` | 弦乐群奏 | Florestan Strings · SF3 · 真实录音合奏 | 28–100 |
  | `builtin.slow_strings` | 慢弦乐 | Florestan Strings 的 Slow 预设 · SF3 | 28–100 |
  | `builtin.violin` | 小提琴 | Florestan String Quartet · SF3 · program 40 | 55–100 |
  | `builtin.viola` | 中提琴 | Florestan String Quartet · SF3 · program 41 | 48–88 |
  | `builtin.cello` | 大提琴 | Florestan String Quartet · SF3 · program 42 | 36–81 |
  | `builtin.contrabass` | 低音提琴 | Florestan String Quartet · SF3 · program 43 | 28–67 |
  | `builtin.pizzicato` | 弦乐拨奏 | Florestan Pizzicato · SF3 | 28–100 |
  | `builtin.trumpet` | 小号 | Florestan Trumpet Metallic · SF3 | 55–82 |
  | `builtin.french_horns` | 圆号齐奏 | Florestan French Horns · SF3 | 41–77 |
  | `builtin.synth_strings_1` | 合成弦乐 1 | FreePats Synth Strings 1 · SF3 | 36–96 |
  | `builtin.synth_strings_2` | 合成弦乐 2 | FreePats Synth Strings 2 · SF3 · 更厚 | 36–96 |
  | `builtin.synth_brass_1` | 合成铜管 1 | FreePats Synth Brass 1 · SF3 | 36–84 |
  | `builtin.synth_brass_2` | 合成铜管 2（DX7） | FreePats Synth Brass 2 · SF3 | 36–84 |

  采样乐器的参数只有两个：`gain`（0..4，默认按乐器校准）与 `tone`（0..1 一阶低通，1 = 旁通）。
  它们输出**单声道**（仍可复音演奏）；要宽度用 `builtin.chorus` / `builtin.reverb` / 声像。
  `builtin.drums` 是完整的原声鼓组，**直接在一轨里按 GM 音高写 kick/snare/hat/tom/cymbal**；
  `builtin.synth.drums` 只是旧的底鼓预设，不要拿它当鼓组。
  独奏弦乐（violin / viola / cello / contrabass）与小号按**单声部**写；圆号齐奏、弦乐群奏可以叠和弦。
  `builtin.strings` 的长音采样偏短，需要持续长音时用 `builtin.slow_strings` 或 `builtin.synth_strings_2`。
- 内置效果器：`builtin.delay` / `builtin.reverb` / `builtin.chorus` / `builtin.drive` / `builtin.eq`。
- 效果链是**有序**的：`add_effect` 追加到末尾；`apply_preset` 的 `effects` 整体替换（想保留的现有效果器
  要一并写回，否则会被清掉）。
- 插件原生界面只在桌面端点得开；MCP 侧只负责参数与状态。

## 音高表达（推弦 / 揉弦）在不同插件上的路径

- **VST3**：`set_track_instrument` 后，插件若声明 Tuning note expression，引擎走逐音表达；
  否则自动退化成通道弯音。用 `describe_plugin` 看参数表即可，不需要手动选路径。
- **CLAP / AU / 内置采样乐器**：走通道弯音（MIDI 1.0）。同一轨同时有多个表达音符时，
  以「最晚起音」的音符为主；单音旋律（吉他 solo、二胡）没有这个问题。
- **内置合成器**：逐音表达，复音也能各自弯曲。
- 弯音范围默认 ±2 半音；音源不是这个范围时，用 `set_track_meta(track, pitch_bend_range=<半音>)`
  改（0.01–24）。改完不需要重新挂乐器。
- 这些技法写在音符的 `expr` 上，不是插件参数：见 `note.md`。
