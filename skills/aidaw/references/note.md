# 域：音符（note）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_note` | `track`,`clip`,`pitch`,`start`,`duration`,`velocity`,`volume?`,`offset?`,`legato?`,`snap?` | 在片段内加**一个**音符（兼容接口）。 |
| `add_notes` | `track`,`clip`,`notes[]`,`snap?` | 一次加一组（编曲请用它，见 `composition.md`）。 |
| `update_notes` | `track`,`clip`,`updates[]` | 按**稳定 id** 改写。 |
| `remove_notes` | `track`,`clip`,`ids[]` | 按**稳定 id** 删除。 |
| `replace_notes` | `track`,`clip`,`notes[]` | 原子替换整段音符。 |
| `strum_notes` | `track`,`clip`,`step_ms`,`direction?`,`range?` | 扫弦：同一起点的一组音按音高依次错开。 |

字段：`id`（片段内稳定标识，由引擎发放）、`pitch`（0..127）、`start`/`duration`（拍，片段内相对）、
`velocity`（0..127，演奏强度）、`volume`（0..127，127 = 不衰减）、
`offset`（**起始偏移**，拍，正 = 延后发声；缺省 0 = 与 `start` 同时发声）、
`legato`（**连奏**：NoteOff 延到**同音高**的下一个音；缺省 false）。

**放置吸附（`snap`）**：`add_note` / `add_notes` 可带 `snap` 把**起点**吸到网格上——
`off`（缺省，原样写入）/ `bar` / `1/4` / `1/8` / `1/16` / `1/12`（三连音）/ `1/24` / `1/32`。
**只吸起点**：时值按你给的算，`offset` 也不参与吸附（那是演奏姿态，不是记谱位置）。

**扫弦**：`strum_notes` 把**起点相同**的一组音按音高依次错开（`direction: "down"` = 低音先响，
缺省；`"up"` 反之）。写的是每个音符的 `offset`，**记谱位置一个都不动**——量化、对齐、复制粘贴
都不会把扫弦搞乱。`step_ms` 是相邻两音的毫秒差（30 是常见的吉他扫弦；`0` = 清掉扫弦、回到同时
发声）。毫秒按该处生效的速度换算成拍，所以**改工程速度不会毁掉扫弦的「几根弦差这么多」**。

**要点**
- 音符是**片段内**坐标：`start` 相对片段起点；被片段左边缘裁掉的音符起点为**负**（仍保留）。
- **id 是片段内唯一的**，所以每个工具都要带 `track` + `clip`；删掉几个音符之后，剩下的 id 不会漂。
- 力度影响音色/振幅（各插件自行解释）；`volume` 是纯增益微调，不改变音色。
- `offset` / `legato` 为缺省值（0 / false）时**不写进工程 JSON**，旧工程形状逐字不变。
- 写整段旋律 / 伴奏请用 `add_notes`（一次事务 = 一个撤销步骤），别循环 `add_note`。
