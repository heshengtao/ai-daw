# 域：音符（note）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_note` | `track`,`clip`,`pitch`,`start`,`duration`,`velocity` | 在片段内加一个音符。 |

字段：`pitch`（0..127）、`start`/`duration`（拍，片段内相对）、`velocity`（0..127，演奏强度）。

**要点**
- 音符是**片段内**坐标：`start` 相对片段起点。
- 力度影响音色/振幅（各插件自行解释）；单音音量属界面能力，MCP 暂未暴露。
