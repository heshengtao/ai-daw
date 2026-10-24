# 域：音符（note）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_note` | `track`,`clip`,`pitch`,`start`,`duration`,`velocity`,`volume?`,`offset?`,`legato?`,`snap?` | 在片段内加**一个**音符（兼容接口）。 |
| `add_notes` | `track`,`clip`,`notes[]`,`snap?` | 一次加一组（编曲请用它，见 `composition.md`）。 |
| `update_notes` | `track`,`clip`,`updates[]` | 按**稳定 id** 改写。 |
| `remove_notes` | `track`,`clip`,`ids[]` | 按**稳定 id** 删除。 |
| `replace_notes` | `track`,`clip`,`notes[]` | 原子替换整段音符。 |
| `set_note_tag` | `track`,`clip`,`id`,`ornament?`,`articulation?` | 给单个音符写/清除装饰音与奏法标签。**`articulation` 现在会发声**：需要轨道先有 `set_articulations` 映射（见 `expression.md`）。 |
| `strum_notes` | `track`,`clip`,`step_ms`,`direction?`,`range?` | 扫弦：同一起点的一组音按音高依次错开。 |

字段：`id`（片段内稳定标识，由引擎发放）、`pitch`（0..127）、`start`/`duration`（拍，片段内相对）、
`velocity`（0..127，演奏强度）、`volume`（0..127，127 = 不衰减）、
`offset`（**起始偏移**，拍，**负 = 提前、正 = 延后**，范围 ±4 拍；缺省 0 = 与 `start` 同时发声）、
`legato`（**连奏**：NoteOff 延到**同音高**的下一个音；缺省 false）、
`channel`（逐音 MIDI 通道 0..15；缺省 = 跟随轨道 / MPE 自动分配）、
`pressure`（起音时的复音压力 0..127；缺省 = 不发）、
`expr`（**音高表达**：推弦 / 揉弦 / 滑音；缺省无，见下节）。

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

## 音高表达：推弦 / 揉弦 / 滑音（`expr`）

`expr` 是音符上的可选对象，**不改变记谱音高**，只改变发声时的音高曲线：

| 字段 | 单位 | 说明 |
| --- | --- | --- |
| `bend` | 音分 | 起音时的偏移；预推弦 = 200（起音就在目标音高） |
| `bend_to` | 音分 | 推弦 / 滑音的目标偏移；与 `bend` 相同 = 保持 |
| `bend_time` | 拍 | 从 `bend` 到 `bend_to` 的时间；0 = 立即到达 |
| `vibrato` | 音分 | 揉弦深度（0 = 不揉） |
| `vibrato_rate` | Hz | 揉弦速率；0 = 默认 5.5 |
| `vibrato_delay` | 拍 | 起音后延迟多久开始揉 |
| `vibrato_fade` | 拍 | 揉弦渐入时长；0 = 立即满深度 |

单位：**100 音分 = 1 个半音**。范围：音分 ±2400，时间 / 延迟 0–256 拍，速率 0–20 Hz。

### 常用写法

**推弦（0 → +200 音分，0.25 拍推到位，然后保持）**

```json
{"pitch": 64, "start": 4.0, "duration": 1.5, "velocity": 100,
 "expr": {"bend": 0, "bend_to": 200, "bend_time": 0.25}}
```

**预推弦（起音就在 +200 音分，之后再揉）**

```json
{"pitch": 64, "start": 4.0, "duration": 1.5, "velocity": 100,
 "expr": {"bend": 200, "bend_to": 200, "vibrato": 35, "vibrato_rate": 5.5,
          "vibrato_delay": 0.5, "vibrato_fade": 0.5}}
```

**延迟揉弦（起音 0.5 拍后开始，0.5 拍渐入）**

```json
{"pitch": 67, "start": 8.0, "duration": 2.0, "velocity": 95,
 "expr": {"vibrato": 40, "vibrato_rate": 5.5,
          "vibrato_delay": 0.5, "vibrato_fade": 0.5}}
```

**下滑（0 → -200 音分）**

```json
{"pitch": 67, "start": 12.0, "duration": 0.75, "velocity": 90,
 "expr": {"bend": 0, "bend_to": -200, "bend_time": 0.3}}
```

**推弦后揉弦**：把 `bend_to` 与 `vibrato` 写在同一个 `expr` 里即可，两个分量相加。

### 播放路径与限制

- 支持逐音表达的插件（VST3 声明了 Tuning note expression / 内置合成器）：每颗音符独立弯曲。
- 只有通道弯音的插件（CLAP / AU / 内置采样乐器 / 多数 VST3）：引擎把逐音表达折叠成通道弯音；
  **重叠音符以「最晚起音」为主**，和弦里同时揉多个音不保证独立。想让和弦里各自弯：
  `set_track_meta(track, mpe_zone="lower", mpe_channels=15)` 开启 MPE（插件也要开 MPE）。
- 弯音范围默认 ±2 个半音；音源不是这个范围时，用
  `set_track_meta(track, pitch_bend_range=<半音>)` 改。
- `quantize_notes` / `apply_swing` / `humanize_notes` / `strum_notes` **不改** `expr`；
  `transpose_notes` 只改 `pitch`，音分偏移保持不变。
- 与倚音一起用时：先写主音与 `expr`，再加倚音，之后不要再量化 / humanize。
- **先 `get_midi_capabilities(track)`**：它会告诉你插件的逐音表达会不会被降级。

## MIDI 通道 / 压力 / 奏法

- `channel` / `pressure` 是音符字段，`add_notes` / `update_notes` 都能写。
- 连续触后（揉弦 / 渐强）用自动化车道 `at:<通道>`（0..15）。
- 奏法：优先 `list_articulation_maps` + `import_articulation_map` 导入库里的映射；没有时再用
  `set_articulations` 定义「奏法名 → keyswitch / CC / program / events」，然后用
  `set_note_tag(track, clip, id, articulation="harmonics")` 给音符打标签。
  界面里：检查器最下方「演奏技巧 → 映射库」导入一次，之后选中音符在音符组的「演奏技巧」下拉里直接切换。
- 通用一次性事件（keyswitch、program change、逐音表达）用 `set_midi_events` / `add_midi_events`。
- 详细写法与限制见 `expression.md`（`aidaw_help("expression")`）。

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

**看清全部音符**：`get_project_overview` 只给每个片段的音符数；`get_project` 才是全量；批量工具摘要只有前 32 个。大工程用
`get_clip_notes(track, clip, offset?, limit?, range?)` 分页核对每个音符。

**让分析知道这是装饰音**：
- 持久写法：`set_note_tag(track, clip, id, ornament="grace")`。标签存在工程的 `note_tags`
  里（键 `track:clip:id`），会随工程保存；`analyze_harmony` 自动读取，命中时无论时值长短
  都记成 `ornament` / `info`。`articulation` 也可以写（`staccato` / `accent` / `legato`…）；
  轨道有对应奏法映射（导入或手写）时会改变播放。
- 一次性写法：`analyze_harmony(ornaments=[{track, clip, id}])`，不写回工程。
- 都不传时，分析用「≤ 1/16 拍 + 后接相邻和弦音」的启发式。
