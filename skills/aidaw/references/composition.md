# 域：编曲（composition）

编曲的三件事在这里：**一次写一批音符**、**批量变换律动/音域**、**检查是否与和弦和其他轨道相容**。
工具都走同一条命令管线：一次调用 = 一个撤销步骤，人在界面上看得见、随时能撤。

## 批量音符（编曲主力）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_notes` | `track`,`clip`,`notes[]` | 一次写入一组音符（最多 5000）。返回 `added_ids` 与片段摘要。 |
| `replace_notes` | `track`,`clip`,`notes[]` | **原子替换**片段内全部音符；空数组 = 清空片段。 |
| `update_notes` | `track`,`clip`,`updates[]` | 按**稳定 id** 改写：`{id, pitch?, start?, duration?, velocity?, volume?}`。 |
| `remove_notes` | `track`,`clip`,`ids[]` | 按稳定 id 删除。 |

`notes[]` 的字段：`pitch`（0..127）、`start`/`duration`（拍，**片段内相对**）、
`velocity`（0..127，缺省 100）、`volume`（0..127，缺省 127）。`id` 由引擎发放，不用自己填。

**为什么用 id 而不是下标**：删掉一个音符后，后面所有音符的数组下标都会前移。`get_project`
返回的每个音符都带 `id`，`update_notes` / `remove_notes` 用它定位，改完还是指得准。

返回形状（批量工具统一）：

```json
{ "track": 3, "clip": 7, "start": 0.0, "length": 16.0, "note_count": 128,
  "notes": [ /* 前 32 个 */ ], "notes_truncated": true, "changed": 128 }
```

## 律动与音域

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `transpose_notes` | `track`,`clip`,`semitones`,`range?` | 移调（半音）。 |
| `set_note_velocity` | `track`,`clip`,`value` 或 `from`+`to` 或 `scale`,`range?` | 固定 / 渐变 / 缩放（三选一）。 |
| `quantize_notes` | `track`,`clip`,`grid`,`strength?`,`range?` | 起点吸到 `grid` 的整数倍。 |
| `apply_swing` | `track`,`clip`,`subdivision?`,`ratio?`,`strength?`,`range?` | 每对子拍的第二个往后推。 |
| `humanize_notes` | `track`,`clip`,`timing?`,`velocity?`,`range?`,`seed?` | 确定性抖动（同 seed 同结果）。 |
| `fit_notes_to_range` | `track`,`clip`,`min_pitch?`,`max_pitch?`,`mode?`,`range?` | 八度折返 / 只报告。 |

要点：

- `range` 是 `{start, end}`（**片段内**拍，左闭右开），只作用于起点落在其中的音符；不给 = 整段。
- `quantize_notes` 的 `grid`：16 分 = 0.25、8 分 = 0.5、4 分 = 1.0；`strength` 0..1（0 = 不动，1 = 完全对齐，0.5 = 吸一半）。
- `apply_swing` 的 `subdivision` 是**子拍长度**：8 分 swing = `0.5`，16 分 = `0.25`；
  `ratio` 是第二个子拍在一对子拍里的位置（0.5 平直 / 0.66 ≈ 三连音 / 0.75 硬 shuffle）。
  只动第二个子拍，小节长度不变。
- `humanize_notes` 缺省 `timing: 0.02` 拍、`velocity: 8`。**同一个 `seed` 结果逐字相同**，
  所以撤销再重做不会漂——想换一种抖动就换 seed。
- `fit_notes_to_range` 的 `mode`：`fold`（默认，整八度上下挪，**音级不变**）/ `report`（只统计）。
  不给音域时用轨道的 `pitch_range` 元数据（`set_track_meta` 写入）。

变换的硬约束：起点不早于片段起点、时值至少 1/32 拍、变换后不越出片段范围；被夹住的音符数
会在返回里体现（`clamped`）。起点为负的音符（被左边缘收起来的）不参与变换。

## 片段编辑

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `move_clip` | `track`,`clip`,`start` | 移到新的起始拍。 |
| `resize_clip` | `track`,`clip`,`length` | 改长度；音频片段的素材窗口按比例变。 |
| `split_clip` | `track`,`clip`,`beat` | 切分：左半段沿用原 id，右半段是新 id。 |
| `duplicate_clip` | `track`,`clip`,`to_track?`,`start?` | 复制；缺省贴在原片段之后。 |
| `merge_clips` | `track`,`clips[]` | 合并多个 **MIDI** 片段（保留第一个的 id，跨度取并集）。 |

`split_clip` 的 `beat` 是**绝对拍**，必须严格落在片段内部；音频片段还需要素材窗口长度
（导入时写进去的 `frames`），否则拒绝。`merge_clips` 里有音频片段会整体拒绝（不会改一半）。

## 分析与检查（只读，不改工程）

| 工具 | 参数 | 返回 |
| --- | --- | --- |
| `get_track_note_stats` | `track`,`clip?` | 音域、平均音高/力度、每小节密度、同音重叠、片段外音符数。 |
| `analyze_track_overlap` | `tracks[]`,`clip?` | 每两条轨道的同发时长、同音重叠、音区差、分工冲突与建议。 |
| `analyze_harmony` | `bars?`,`tracks?`,`allow_extensions?`,`allow_approach_notes?`,`allow_non_chord_tone_duration?` | 和弦外音、不推荐的延伸音、未解决导音、悬挂音、跨轨重叠、每小节冲突等级。 |

`analyze_harmony` 的输入：

```json
{ "bars": [{"start": 0, "length": 4, "symbol": "Dm9"},
           {"start": 4, "length": 4, "symbol": "G13"}],
  "tracks": [74, 75, 76],
  "allow_extensions": true,
  "allow_approach_notes": true,
  "allow_non_chord_tone_duration": 0.5 }
```

- `bars` 不给就用工程里存的和弦表（`set_project_harmony` 写过的那份）；都没有会报错。
- 输出的每条问题带 `kind`（`non_chord_tone` / `extension` / `unresolved_leading_tone` /
  `suspension` / `track_overlap`）、`severity`（`info` / `warning` / `error`）、`bar`、`beat`、
  `track`、`pitch`、`pitch_name`、`duration`、`message`。
- 短经过音（时值 ≤ `allow_non_chord_tone_duration`）只给 `info`，不算冲突。

**支持的和弦符号**：大三 / 小三 / `dim` / `aug` / `sus2` / `sus4` / `5` / `6` / `69` /
`7` / `maj7` / `m7` / `m7b5`（`ø`）/ `dim7` / `9` / `11` / `13` / `7b9` / `7#9` / `7#11` /
`7b13` / `7sus4` / `alt`，以及 `add9` `add11` `no3` `no5` `b5` `#5` 这类修饰与 `C/E` 斜杠低音。
**无法识别的符号直接报错**（带小节号），不会静默放宽规则。

## 编曲元数据（跟着工程走）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `set_project_harmony` | `bars[]` | 整体替换工程和弦表；符号先校验，写错直接报错。 |
| `set_project_sections` | `sections[]` | 段落表：`{name, start, end, role?}`。 |
| `set_track_meta` | `track`,`role?`,`min_pitch?`,`max_pitch?` | 轨道分工与目标音域。 |

这些字段**为空时不写进工程 JSON**，所以旧工程形状不变；工具在没有元数据时也照样接受显式参数。

## 一个可用的编曲闭环

1. `get_project` 看清轨道 / 片段 id 与速度、拍号。
2. `set_project_harmony` 写一份和弦谱（也可以只在分析时传 `bars`）。
3. `set_track_meta` 给每条轨标分工与目标音域（`lead` 60..84、`bass` 28..48…）。
4. `add_notes` 一次写出整段旋律 / 伴奏（别循环 `add_note`）。
5. `quantize_notes` + `apply_swing` 定节奏感；`humanize_notes` 去掉机械感。
6. `fit_notes_to_range` 把跑出音域的音折回来（`mode: "report"` 先看一眼）。
7. `analyze_harmony` 查和声，`analyze_track_overlap` 查轨道打架，按返回的 `message` 修。
8. `analyze_project` 听感确认 → `export_wav` 导出。

## 坑

- **id 是片段内的**：同一个 id 在两条片段里可能都存在，所以每个工具都要带 `track` + `clip`。
- **一次事务 = 一个撤销步骤**：写 1000 个音符只占一步；但连着调 5 次变换就是 5 步，
  别在一个循环里反复 `transpose_notes`——一次算好总移调量再调。
- **`range` 是片段内拍**，不是工程绝对拍；`split_clip` 的 `beat` 反而是绝对拍。
- **变换只作用于可见音符**（起点 >= 0）；被片段左边缘裁掉的音符留在原地。
- **`analyze_harmony` 不自动改音**：它只指出问题。要不要改、怎么改，由你（或人）决定。
