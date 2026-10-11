---
name: aidaw
description: 通过内嵌 MCP 工具或 aidaw 命令行操作正在运行的 AI DAW——读写工程、轨道、片段、音符、混音（增益/声像/发送/自动化）、传输播放、音频导入与录音、响度与频谱分析、导出 WAV、扫描插件。凡用户要求编曲、建轨、写音符、调混音、录音、分析或导出音频时使用。
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
| 工程 | `references/project.md` | 读工程、新建/重命名、保存/打开、会话缓存 |
| 轨道 | `references/track.md` | 增删改轨道、挂/换乐器、效果链 |
| 片段 | `references/clip.md` | MIDI / 音频片段的增删改、移动、缩放、切分 |
| 音符 | `references/note.md` | 钢琴卷帘里的音符与力度 / 音量 |
| 混音 | `references/mix.md` | 增益/声像/静音/独奏/总输出、**发送 → 总线**、**自动化** |
| 音频 | `references/audio.md` | 导入音频、波形、**录音** |
| 传输 | `references/transport.md` | 播放 / 暂停 / 停止 / 定位 / 循环 |
| 分析导出 | `references/analysis.md` | 渲染预览、响度 / 频谱分析、导出 WAV |
| 插件 | `references/plugin.md` | 扫描 / 列出插件、参数、用户预设 |

## 典型闭环（建轨 → 编曲 → 导出 → 分析）

1. `references/project.md`：`get_project` 看清现状（轨道 id、片段 id、速度、拍号）。
2. `references/track.md`：`add_track` 建轨 → `set_track_instrument` 选音色。
3. `references/clip.md` + `references/note.md`：`add_clip` 画片段 → `add_note` 写音符。
4. `references/mix.md`：`set_gain` / `set_pan` 调平衡；要总线就再建一条轨 + `set_sends`。
5. `references/analysis.md`：`analyze_project` 看响度 / 频谱 → 回头微调。
6. `references/analysis.md`：`export_wav` 导出。

## 约定与坑

- **先读后写**：`get_project` 拿到的 id 才是真相；`add_track` / `add_clip` / `add_note` 会在返回文本里
  给出新建对象的 id，下一步要用它。
- **整体替换类操作**：`set_sends` / `set_automation` 是**整体替换**（空数组 = 清空 / 删除该车道）；
  `apply_preset` 的 `effects` 是**替换**整条效果链，不是叠加。
- **自动化的 `param` 命名空间**：`gain_db` / `pan`，或 `inst:<参数id>`（轨道乐器）、
  `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）。
- **音频片段的时间语义**：片段拍长是唯一真相，素材铺满跨度——**改速度 / 缩放会变调**（v1 不做保音高变速）。
- **能力面 = MCP 工具面**：只在命令层、没暴露成 MCP 工具的动作（如「打开插件窗口」）在这里不可用；
  要用的办法是「先给它加一个 MCP 工具」。
