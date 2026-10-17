# 域：音符（note）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_note` | `track`,`clip`,`pitch`,`start`,`duration`,`velocity` | 在片段内加**一个**音符（兼容接口）。 |
| `add_notes` | `track`,`clip`,`notes[]` | 一次加一组（编曲请用它，见 `composition.md`）。 |
| `update_notes` | `track`,`clip`,`updates[]` | 按**稳定 id** 改写。 |
| `remove_notes` | `track`,`clip`,`ids[]` | 按**稳定 id** 删除。 |
| `replace_notes` | `track`,`clip`,`notes[]` | 原子替换整段音符。 |

字段：`id`（片段内稳定标识，由引擎发放）、`pitch`（0..127）、`start`/`duration`（拍，片段内相对）、
`velocity`（0..127，演奏强度）、`volume`（0..127，127 = 不衰减）。

**要点**
- 音符是**片段内**坐标：`start` 相对片段起点；被片段左边缘裁掉的音符起点为**负**（仍保留）。
- **id 是片段内唯一的**，所以每个工具都要带 `track` + `clip`；删掉几个音符之后，剩下的 id 不会漂。
- 力度影响音色/振幅（各插件自行解释）；`volume` 是纯增益微调，不改变音色。
- 写整段旋律 / 伴奏请用 `add_notes`（一次事务 = 一个撤销步骤），别循环 `add_note`。
