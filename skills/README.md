# AI DAW skills（渐进式披露的索引）

这是给 **AI harness** 用的能力索引。**只加载这一条**，然后按需加载某个能力域的 skill——
每个域只描述它那一小组命令 / MCP 工具与组合用法，避免一次把几十上百个工具灌进上下文。

## 两种入口

1. **逐个 MCP 工具**：`src-tauri` 内嵌的 MCP 服务器暴露 30+ 个工具（`get_project` / `add_track` / …）。
   想直接调就用它们，与下面的域文档一一对应。
2. **网关工具**（推荐给大模型）：`aidaw_help(topic)` 按域取回这份文档；`aidaw_call(op, args)` 用
   「操作名 + JSON 参数」执行——固定两条工具，能力按需拉取。

> 目录：本目录即 skills 包。索引 = `README.md`；每个域一个文件（`project.md` / `track.md` / …）。

## 能力域

| 域 | 文件 | 覆盖 |
| --- | --- | --- |
| 工程 | `project.md` | 读工程、新建/重命名、保存/打开、会话缓存 |
| 轨道 | `track.md` | 增删改轨道、挂/换乐器、效果链 |
| 片段 | `clip.md` | MIDI/音频片段的增删改、移动、缩放、切分 |
| 音符 | `note.md` | 钢琴卷帘里的音符与力度/音量 |
| 混音 | `mix.md` | 增益/声像/静音/独奏/总输出、**发送 → 总线**、**自动化** |
| 音频 | `audio.md` | 导入音频、波形、**录音** |
| 传输 | `transport.md` | 播放/暂停/停止/定位/循环 |
| 分析导出 | `analysis.md` | 渲染预览、响度/频谱分析、导出 WAV |
| 插件 | `plugin.md` | 扫描/列出插件、参数、用户预设 |

## 典型闭环（建轨 → 编曲 → 导出 → 分析）

1. `project`：`get_project` 看清现状。
2. `track`：`add_track` 建轨 → `set_track_instrument` 选音色。
3. `clip` + `note`：`add_clip` 画片段 → `add_note` 写音符。
4. `mix`：`set_gain` / `set_pan` 调平衡；要总线就再建一条轨 + `set_sends`。
5. `analysis`：`analyze_project` 看响度/频谱 → 回头微调。
6. `analysis`：`export_wav` 导出。

**约定**：所有写操作都走与界面同一条命令管线（可撤销、会广播 `engine://event`）；AI 署名 `Agent("mcp")`。
