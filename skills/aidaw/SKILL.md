---
name: aidaw
description: 通过内嵌 MCP 工具或 aidaw 命令行操作正在运行的 AI DAW——读写工程、轨道、片段、音符、批量编曲（转调/量化/摇摆/人性化/和弦分析）、乐理与曲风配方（音阶/调式/和弦进行/鼓谱/配器）、内置采样乐器（钢琴/吉他/贝斯/鼓组/弦乐/铜管）、混音（增益/声像/发送/自动化）、传输播放、音频导入与录音、响度与频谱分析、导出 WAV、扫描插件。凡用户要求编曲、分析曲风、建轨、写音符、改和弦、量化节奏、调混音、录音、分析或导出音频时使用。
license: Apache-2.0
---

# AI DAW：用 MCP / `aidaw` 驱动正在运行的 DAW

这是**能力包入口**。本文件只给公共约定 + 索引；每个能力域的细节在 `references/` 下单独一篇。
**按需加载**：只读你要动手的那一两个域，不要把整包灌进上下文。

## 前提

- 软件（AI DAW）需要在运行。走 CLI 时先 `aidaw status` 确认，没开就 `aidaw start`。
- **所有写操作都走与界面同一条命令管线**：进撤销历史、广播 `engine://event`、在界面上留痕，
  署名 `Agent("mcp")`。你做的每一步人都看得见，也随时可以撤销——所以放心动手，不必畏手畏脚，
  但也不要一次改一大片（一次事务 = 一个撤销步骤，人撤销起来才不痛苦）。

## 三条入口

1. **逐个 MCP 工具**（`get_project` / `add_track` / …）：与下面的域一一对应，工具名即能力。
2. **网关**（推荐给大模型）：`aidaw_help(topic)` 按域取回文档（就是这里）；`aidaw_call(op, args)`
   用「操作名 + JSON 参数」执行——固定两条工具，能力按需拉取。
3. **命令行**：`aidaw` 是这些工具的**薄客户端**，命令面 = 工具面。

### 定位 App 与 CLI（**不用搜索**）

App 的 bundle id 是 **`com.aidaw.app`**，它的配置目录由这个 id 推导得出——**路径是固定的**：

| 平台 | 配置目录 |
| --- | --- |
| macOS | `~/Library/Application Support/com.aidaw.app/` |
| Linux | `~/.config/com.aidaw.app/` |
| Windows | `%APPDATA%\com.aidaw.app\` |

目录里有两个运行时文件（App 每次启动都会重写）：

- **`cli-path`** —— 包内 `aidaw` 的**绝对路径**。读到就直接用：不需要搜索、不依赖 PATH、
  也不需要用户开挂载开关。
- `mcp-url` —— MCP 的 HTTP 端点（App 正在运行时才有）。

**按这个顺序找，不要瞎猜：**

1. `command -v aidaw` —— 用户开了挂载开关就用短命令。
2. 否则读 `<配置目录>/cli-path` → 拿到全路径，用 `"<那个路径>" status` 这样调。
3. 都没有 → App 从没启动过：macOS 用 `open -b com.aidaw.app` 起它（LaunchServices 按 id 找得到），
   再回到第 2 步。
4. 还不行 → 退回常见位置（`/Applications/AI DAW.app/Contents/MacOS/aidaw`、`/usr/bin/aidaw`、
   `%LOCALAPPDATA%\AI DAW\aidaw.exe`），或 macOS 上
   `mdfind "kMDItemCFBundleIdentifier == 'com.aidaw.app'"`。

> 为什么不拿 `mdfind` 当主路径：它依赖 Spotlight 索引、可能搜到多份、沙箱里可能查不到。
> 第 2 步是**确定的**，所以优先。

### 用 CLI 执行

拿到可执行文件（短命令或全路径）之后，子命令完全一样：

1. `<aidaw> status` —— 软件在不在、MCP 地址是否可达（在 → 退出码 0，不在 → 1）。
2. `<aidaw> start` —— 没开就启动它，等到 MCP 就绪再返回。
3. `<aidaw> call <工具> --json '{…}'` —— 执行，例如 `<aidaw> call set_tempo --json '{"bpm":120}'`。
4. `<aidaw> tools` —— 需要工具清单时列出。

**可选**：想让用户以后能敲短命令，可以提一句 **设置 → 命令行** 里的「在终端使用 aidaw」开关
（会请求一次管理员授权）。**这是可选项**——别替用户决定，也别因为它没开就说做不到。

> **加一个 MCP 工具 = 自动多一条可调用的 CLI 命令**（按名转发，CLI 侧零改动）。

## 能力域（按需加载）

| 域 | 加载 | 覆盖 |
| --- | --- | --- |
| 工程 | `references/project.md` | 读工程（全量）、**分页读片段音符**、速度/拍号、撤销/重做、**工程元数据（和弦表 / 段落 / 轨道分工与音域）** |
| 轨道 | `references/track.md` | 增删改轨道、挂/换乐器、效果链 |
| 片段 | `references/clip.md` | MIDI / 音频片段的增删改、**移动、缩放、切分、复制、合并** |
| 音符 | `references/note.md` | 钢琴卷帘里的音符与力度 / 音量 |
| 编曲 | `references/composition.md` | **批量改音符（一个撤销步）、量化 / 摇摆 / 人性化、音域折叠、节奏统计、半音冲突、和弦分析** |
| 混音 | `references/mix.md` | 增益/声像/静音/独奏/总输出、**发送 → 总线**、**自动化** |
| 音频 | `references/audio.md` | 导入音频、波形、**录音** |
| 传输 | `references/transport.md` | 播放 / 暂停 / 停止 / 定位 / 循环 |
| 分析导出 | `references/analysis.md` | 渲染预览、响度 / 频谱分析、导出 WAV |
| 插件 | `references/plugin.md` | **按分类与音色描述挑乐器/效果器**、**内置采样乐器（钢琴/吉他/贝斯/鼓组/弦乐/铜管）**、参数表、效果链、用户预设 |
| 乐理 | `references/theory.md` | MIDI 音高、音阶/调式、和弦与进行、声部连接、音域、旋律写作 |
| 节奏 | `references/rhythm.md` | 拍/网格换算、GM 鼓映射、`builtin.drums` 原声鼓组、鼓型、力度/量化/swing/人性化 |
| 曲风 | `references/genres.md` | 14 类曲风的 BPM/拍号/和声/鼓型/贝斯/配器/结构/混音配方 |
| 编曲流程 | `references/workflow.md` | **需求 → 曲风卡 → 查知识（skills/联网）→ MIDI 设计 → AIDAW 落地 → 检查 → 导出** |

## 接到编曲需求：先分析曲风，再动手

**不要拿到「做一首 X」就直接写音符。** 标准流程在 `references/workflow.md`，核心是五步：

1. **填风格卡**：曲风/情绪/BPM/拍号/调式/和声/鼓型/配器/结构。用户没给曲风就按
   `genres.md` 的「快速索引」选最接近的一张；参考曲只提取**风格特征**，不抄旋律与整段和弦。
2. **查知识**：`aidaw_help("genres")` 找配方 → `aidaw_help("theory")` 定音阶/和弦/音域 →
   `aidaw_help("rhythm")` 定鼓型与网格。**skills 是工具语义的真相**；需要最新 BPM/风格趋势时，
   再联网补 1–2 个来源，把来源与日期写进风格卡。
3. **先纸面设计**：段落表（拍）、和弦表（`start`/`length`/`symbol`）、轨道表（role/音区/音色）、
   音符表（第几格、什么音、多长、多响）。
4. **再落地 AIDAW**：`get_project` → `set_tempo` / `set_time_signature` →
   `set_project_harmony` / `set_project_sections` → `add_track` → `list_plugins` /
   `set_track_instrument` → `set_track_meta` → `add_clip` → `add_notes`（批量）→
   `quantize_notes` / `apply_swing` / `strum_notes` / `humanize_notes` →
   `analyze_harmony` / `analyze_track_overlap` / `get_track_note_stats` → `analyze_project` → `export_wav`。
5. **检查后再交付**：和声 `error`、轨道打架、音域越界、响度/峰值、鼓的路线是否混用（GM / 内置单件）。

## 典型闭环（建轨 → 编曲 → 导出 → 分析）

1. `references/project.md`：`get_project` 看清现状（轨道 id、片段 id、速度、拍号）。
2. `references/genres.md` + `theory.md` + `rhythm.md`：定风格卡、和弦谱、鼓型与 MIDI 设计。
3. `references/track.md`：`add_track` 建轨 → 先 `list_plugins`（读分类与音色描述）再
   `set_track_instrument` 选音色 → `set_track_meta` 写 role/音区。
4. `references/clip.md`：`add_clip` 画片段（`move_clip` / `resize_clip` / `split_clip` / `duplicate_clip` 调结构）。
5. `references/composition.md` + `references/note.md`：`add_notes` **一次写完一个乐句**
   → `quantize_notes` / `apply_swing` 定节奏 → `set_project_harmony` 写和弦 → `analyze_harmony` 查错音。
6. `references/mix.md`：`set_gain` / `set_pan` 调平衡；要总线就再建一条轨 + `set_sends`。
7. `references/analysis.md`：`analyze_project` 看响度 / 频谱 → 回头微调。
8. `references/analysis.md`：`export_wav` 导出。

## 约定与坑

- **先读后写**：`get_project` 拿到的 id 才是真相；`add_track` / `add_clip` / `add_note` 会在返回文本里
  给出新建对象的 id，下一步要用它。
- **挑音色先看目录**：插件名只是型号（`JC303` 是 TB-303 克隆的**单音酸性贝斯**，不是普通合成器）。
  `list_plugins` 给分类 + 一句话描述 + 推荐分工 + 建议音域；参数表用 `describe_plugin`。
  `set_track_instrument` / `add_effect` 也接受名字，但**别凭名字猜音色**——认不出的条目会自己说明「未收录」。
- **批量优先，别一条条加**：写一个乐句用 `add_notes`（一次事务 = 一个撤销步），不要循环 `add_note`；
  改一批音符用 `update_notes` / `transpose_notes`，别删了重加（删了 id 就变了）。
- **批量返回的 `notes` 只有前 32 个**：那是摘要，不是全量。`get_project` 是全量；要逐音核对
  大工程用 `get_clip_notes(track, clip, offset?, limit?, range?)` 分页读。
- **倚音/装饰音**：没有独立字段，用「短 `duration`（0.04–0.08 拍）+ 负 `offset`」写；主音留在
  网格上，先量化/加倚音后不要再 humanize。细节见 `references/note.md`。
- **音符 id 是片段内的**：`clip` 换了，同一个 `id` 指的是另一颗音符；跨片段要先 `get_project` 确认。
- **改音符的工具会自动夹住越界**：起点 ≥ 片段起点、时长 > 0、不超出片段——被夹的条数会在返回里报出来。
- **整体替换类操作**：`set_sends` / `set_automation` / `replace_notes` / `set_project_harmony` /
  `set_project_sections` 是**整体替换**（空数组 = 清空 / 删除该车道）；`apply_preset` 的 `effects`
  是**替换**整条效果链，不是叠加。
- **自动化的 `param` 命名空间**：`gain_db` / `pan`，或 `inst:<参数id>`（轨道乐器）、
  `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）。
- **音频片段的时间语义**：片段拍长是唯一真相，素材铺满跨度——**改速度 / 缩放会变调**（v1 不做保音高变速）。
- **能力面 = MCP 工具面**：只在命令层、没暴露成 MCP 工具的动作（如「打开插件窗口」）在这里不可用；
  要用的办法是「先给它加一个 MCP 工具」。
