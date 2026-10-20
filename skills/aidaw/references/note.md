# 域：音符（note）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_note` | `track`,`clip`,`pitch`,`start`,`duration`,`velocity`,`volume?`,`offset?`,`legato?`,`snap?` | 在片段内加**一个**音符（兼容接口）。 |
| `add_notes` | `track`,`clip`,`notes[]`,`snap?` | 一次加一组（编曲请用它，见 `composition.md`）。 |
| `update_notes` | `track`,`clip`,`updates[]` | 按**稳定 id** 改写。 |
| `remove_notes` | `track`,`clip`,`ids[]` | 按**稳定 id** 删除。 |
| `replace_notes` | `track`,`clip`,`notes[]` | 原子替换整段音符。 |
| `set_note_tag` | `track`,`clip`,`id`,`ornament?`,`articulation?` | 给单个音符写/清除装饰音与奏法标签（工程级 `note_tags`，不改播放）。 |
| `strum_notes` | `track`,`clip`,`step_ms`,`direction?`,`range?` | 扫弦：同一起点的一组音按音高依次错开。 |

字段：`id`（片段内稳定标识，由引擎发放）、`pitch`（0..127）、`start`/`duration`（拍，片段内相对）、
`velocity`（0..127，演奏强度）、`volume`（0..127，127 = 不衰减）、
`offset`（**起始偏移**，拍，**负 = 提前、正 = 延后**，范围 ±4 拍；缺省 0 = 与 `start` 同时发声）、
`legato`（**连奏**：NoteOff 延到**同音高**的下一个音；缺省 false）。

**最短时值**是 **0.03125 拍（1/32 拍）**，4/4 下相当于 **1/128 音符**；76 BPM 时约 **25 ms**。
别把它和「32 分音符」混淆：32 分音符是 **0.125 拍**（76 BPM 时约 99 ms）。

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

## 倚音 / 装饰音（没有独立字段，但可以正确表达）

工具没有 `grace` / `ornament` 字段，用 **短 `duration` + 负 `offset`** 做出真正的倚音：

```json
{"pitch": 71, "start": 8.0, "duration": 0.05, "offset": -0.05, "velocity": 70},
{"pitch": 72, "start": 8.0, "duration": 1.0,  "velocity": 92}
```

- **主音留在网格上**，不要为了给倚音让位而把主音往后推。
- 倚音与主音用**同一个 `start`**，倚音 `duration` 取 **0.04–0.08 拍**（76 BPM ≈ 32–63 ms），
  `offset = -duration`，这样倚音的 NoteOff 正好落在主音起点。
- 先 `quantize_notes` 把主音锁回网格，再 `add_notes` 加倚音；**加完不要再量化 / 加 `snap`**。
- 不要对含倚音的旋律跑 `humanize_notes`。必须跑时：
  - 倚音与主音用**同一个 `start`**（上面的推荐写法）：传 `group_by_start: true`，让它们共用
    同一个位置抖动，`offset` 差值保持不变；再配 `min_duration` 时，同组只要有一个短音，
    整组都不动（不会把倚音和主音拆开）。
  - 倚音用了**不同 `start`**：用 `range` 排除，或 `min_duration: 0.1` 跳过短音——但主音仍会动，
    两者的间距会变，能不用就不用。
  - 最稳的顺序：先 `humanize`，再按主音的最终 `start` 加倚音。
- `strum_notes` 会重写**同起点**音符的 `offset`；先扫弦、后加倚音，别反过来。

**看清全部音符**：`get_project` 是全量；批量工具摘要只有前 32 个。大工程用
`get_clip_notes(track, clip, offset?, limit?, range?)` 分页核对每个音符。

**让分析知道这是装饰音**：
- 持久写法：`set_note_tag(track, clip, id, ornament="grace")`。标签存在工程的 `note_tags`
  里（键 `track:clip:id`），会随工程保存；`analyze_harmony` 自动读取，命中时无论时值长短
  都记成 `ornament` / `info`。`articulation` 也可以写（`staccato` / `accent` / `legato`…），
  当前只作为元数据，不改变播放。
- 一次性写法：`analyze_harmony(ornaments=[{track, clip, id}])`，不写回工程。
- 都不传时，分析用「≤ 1/16 拍 + 后接相邻和弦音」的启发式。
