# 域：MIDI 表达（expression）

插件侧会什么，取决于你送进去什么。这个域负责把 **触后、复音压力、逐音通道、奏法切换、一次性 MIDI 事件** 送到插件。

## 界面里怎么用（普通人路径）

1. 乐器区「演奏技巧 → 配置技巧…」：按界面里的三步走——**① 在「映射库」搜索厂商 / 产品 / patch 并导入**（随包 350+ 份，数据来自 ArtiCluster，MIT）；**② 检查导入的技巧**（通常不用改）；**③ 在钢琴卷帘选中音符，用检查器的「演奏技巧」下拉选择**。
2. 映射库导入的行是**只读**的：界面只显示「映射库切换方式」，不需要理解 MIDI 消息。要改触发方式点「改为手动配置」；想换一份映射点「重新导入」或「清除映射」。
3. 多选音符后切换奏法只占一个撤销步；右键仍然是删除。
4. 音符区的「音高表达」只显示摘要；点「编辑…」打开统一的音符设置弹窗，用预设 + 曲线预览调整推弦 / 揉弦。
5. 和弦里要让每个音独立推弦 / 揉弦：在技巧弹窗的「高级 MIDI」里打开 MPE（插件也要开 MPE）。
6. 扫弦、逐音通道、压力、原始事件表都在同一个音符设置弹窗 / 高级 MIDI 里，不再散落在检查器首屏。

## 先查能力，再写音符

`get_midi_capabilities(track)` 返回：

- `transport`：插件声明了哪些（`channel_pressure` / `poly_pressure` / `program_change` / `per_note_channel` / `note_expression.tuning|volume|...`）。
- `used`：工程里用了多少（`pressure_notes` / `expr_notes` / `articulation_notes` / `midi_events` / `articulations`），以及 `articulation_source`。
- `degraded`：哪些表达会被降级；会点名“有标签但没有映射”的奏法。**写之前先看它**，避免写了却没生效。

能探测的（VST3 Note Expression）是确定值；AU / CLAP 没有等价探测，只表示「宿主送得到」，是否响应看插件。

## 音符上的表达

| 字段 | 范围 | 说明 |
| --- | --- | --- |
| `channel` | 0..=15 | 逐音 MIDI 通道。缺省 = 跟随轨道（MPE 开启时由调度器分配）。 |
| `pressure` | 0..=127 | 起音时的复音压力（poly aftertouch）。 |
| `expr` | 见 `note.md` | 推弦 / 揉弦 / 滑音（音分）。 |
| `volume` | 0..=127 | 单音音量；插件支持 Volume note expression 时独立送达。 |

`add_notes` / `update_notes` 都接受这些字段。例：

```json
{"pitch": 64, "start": 4.0, "duration": 1.5, "velocity": 100,
 "pressure": 80, "channel": 2,
 "expr": {"bend": 0, "bend_to": 200, "bend_time": 0.25}}
```

### 推弦 / 揉弦会不会被浪费

- VST3 插件声明 `Tuning`：逐音表达，和弦里可以各自弯。
- 否则折叠成通道弯音：**同通道只有最晚起音的音符生效**。
- 想让和弦里的音各自弯：`set_track_meta(track, mpe_zone="lower", mpe_channels=15)` 开启 MPE，
  再给音符分配/让调度器分配成员通道。插件自己要开 MPE 才会按 MPE 解释。
- 插件完全不支持逐音音高时，开 MPE 也只能分通道，弯音仍是通道级。

## 连续触后（揉弦 / 渐强）

用自动化车道 `at:<通道>`（0..15）：

```json
{"track": 1, "param": "at:2", "points": [{"beat": 4.0, "value": 0}, {"beat": 4.5, "value": 90}]}
```

- 不开 MPE：通道触后影响该通道所有音。
- 开 MPE：每个成员通道对应一颗音，`at:<成员通道>` 就是逐音触后。
- 插件不支持通道触后时会直接丢弃（`degraded` 会说明）。

## 奏法映射（keyswitch / CC / program）

吉他泛音、闷音、击勾弦这类「奏法」通常由插件的 keyswitch 或 CC 切换。宿主不内置厂商键位——但**优先查随包映射库**，不要凭手册猜。

1. 查库并导入（推荐）：

```json
// list_articulation_maps(query="BBC Symphony")
// import_articulation_map(track=1, map_id="Spitfire/BBC Symphony Orchestra Core/Basoon",
//                        mode="replace")   // 或 "merge"
```

导入是一个撤销步，来源会写进 `meta.articulation_source`。映射库按厂商 / 产品 / patch 索引，覆盖 Spitfire、8DIO、Cinematic Studio、Impact Soundworks、Toontrack、Ample Sound、MusicLab 等 350+ 份 patch。

2. 库里没有时才手写映射：

```json
// set_articulations(track=1, articulations={
//   "harmonics": {"keyswitch": 24},
//   "palm_mute": {"cc": {"controller": 1, "value": 0}},
//   "slide":     {"keyswitch": 26, "momentary": true},
//   "sul_tasto": {"events": [
//     {"event": {"ControlChange": {"controller": 32, "value": 5}}},
//     {"event": {"NoteOn": {"note": 24, "velocity": 1}}}
//   ]}
// }, default="sustain")
```

`events` 是通用消息序列，优先于 `keyswitch` / `cc` / `program`：

- `offset_beats`（拍，非负）= 相对切换时刻的偏移，缺省 0。
- `channel` 缺省 = 跟随音符通道。
- 序列里的 `NoteOn` 如果没有同音高 `NoteOff`，调度器会自动补一个 1/64 拍后的 NoteOff。

3. 给音符打标签（手写映射和导入映射都一样）：

```json
// set_note_tag(track=1, clip=2, id=17, articulation="harmonics")
```

4. 播放时，调度器会在音符起音前 `lead_beats`（缺省 1/64 拍）发出 keyswitch/CC/program/events；
   `momentary: true` 会在音符结束后切回 `default`。

- `keyswitch` 是一个很短的 NoteOn/NoteOff（velocity 1）。
- 没有映射的标签不会发声（`get_midi_capabilities` 会警告）。
- 标签存在工程的 `note_tags` 里；切分 / 复制片段时会跟着音符走。

### 击勾弦 / 连奏

- 插件通常靠**音符重叠**识别 legato：把前一个音的 `duration` 延过后一个音的 `start`。
- `legato: true` 只延长**同音高**的 NoteOff（避免重复音断开），不会为不同音高造重叠。
- 没有 CC68 / CC65 的专门字段；需要时用 `cc:68` 自动化或 MIDI 事件表。

## 通用 MIDI 事件表

`set_midi_events` / `add_midi_events` 写绝对拍的事件，适合 keyswitch、一次性 CC、program change、
逐音表达等没有专门 UI 的东西。事件是外部标记 JSON：

```json
[
  {"beat": 0.0,  "channel": 0, "event": {"ControlChange": {"controller": 64, "value": 127}}},
  {"beat": 4.0,  "channel": 0, "event": {"ProgramChange": {"program": 3}}},
  {"beat": 8.0,  "channel": 2, "event": {"ChannelPressure": {"value": 90}}},
  {"beat": 8.0,  "channel": 2, "event": {"PolyPressure": {"note": 60, "value": 80}}},
  {"beat": 9.0,  "channel": 0, "event": {"NoteExpression": {"note": 60, "kind": "brightness", "value": 0.5}}}
]
```

事件与音符事件一起按帧排序；通道缺省 0。读回来用 `get_midi_events(track)`。

## 常见组合

- **吉他推弦 + 揉弦**：`expr.bend_to` + `expr.vibrato`；和弦里想独立弯就开 MPE。
- **吉他泛音 / 闷音**：`set_articulations` 建映射 + `set_note_tag` 打标签。
- **管弦乐 keyswitch**：同上；不同音色库键位不同，务必按手册。
- **触后揉弦**：`at:<通道>` 曲线（或 MPE + 逐音通道）。
- **音色切换**：`ProgramChange` 事件，或插件参数自动化（`inst:<param>`）。

## 不要做的事

- 不要凭名字猜 keyswitch 键位 / CC 号。
- 不要在插件不支持逐音音高时，给和弦里多个音同时写大幅推弦——只有最晚起音那个生效。
- 不要把 `pressure` 当成「每颗音持续变化的压力」：它是起音值；连续变化用 `at:` 曲线。
