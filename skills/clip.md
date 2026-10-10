# 域：片段（clip）

片段（clip）是时间轴上的内容容器：MIDI 片段（音符）或音频片段（素材）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_clip` | `track`,`start`,`length` | 新增 MIDI 片段（单位：拍）。 |
| `remove_clip` | `track`,`clip` | 删除片段。 |

**要点**
- 音频片段用 `import_audio`（见 `audio.md`）生成，不用 `add_clip`。
- 移动/缩放/切分等编辑在界面里做；MCP 目前只暴露增删（后续可补）。
- `start` 与 `length` 都是**拍**；改工程速度会让片段的时间跨度（秒）随之变化。
