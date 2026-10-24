# 域：影片配乐与分轨交付（film）

这个域解决「给一条已经剪好的片子配乐，并按时码交 stems」。

- 输入：`film.json`（AI CUT 与 AI DAW 共享的影片清单）
- 输出：段落 / 和弦 / 轨道 / 片段骨架 → 写音符 → 分轨 WAV + `stems.json`
- 时间：清单里是**秒**；进入工程后是**拍**。

## 1. film.json 最小形状

```json
{
  "version": 1,
  "name": "137s 草稿",
  "fps": 60,
  "scenes": [
    {"id": "s05-song", "t0": 29, "t1": 35, "title": "宋代山水", "markers": [31, 32]},
    {"id": "s06-cross", "t0": 35, "t1": 41, "title": "第一次穿越"}
  ],
  "music": {
    "bpm": 120,
    "key": "A minor",
    "offset": 0,
    "timeSignature": "4/4",
    "cues": [
      {"scene": "s05-song", "t0": 29, "t1": 35, "instrument": "guqin-like", "gainDb": -3},
      {"scene": "s06-cross", "t0": 35, "t1": 41, "instrument": "gamelan-like"}
    ],
    "harmony": [
      {"t0": 29, "t1": 32, "symbol": "Am"},
      {"t0": 32, "t1": 35, "symbol": "F"}
    ]
  }
}
```

- `scenes` → 段落表；`title` 作段落名。
- `music.cues` → 轨道（按 `instrument` 分组）+ 片段（按 `t0/t1`）。
- `music.harmony`（或 `music.chords`）→ 小节和弦表；符号会先用 `parse_chord` 校验。
- `music.offset`：拍 0 对应的影片时间（秒）。默认 0。
- `music.timeSignature` 支持 `"4/4"` 或 `{ "num": 4, "den": 4 }`。
- `cue.notes` 可选：`[{pitch, start, duration, velocity}]`，`start` 相对片段起点（拍）。
- 相对路径/额外字段由 AI CUT 负责；AI DAW 只读 `music` 与 `scenes`。

秒 → 拍：

```
beat = (t - offset) × bpm / 60
```

## 2. `import_cue_sheet`

```json
{"path": "/abs/film.json", "clear": true, "create_tracks": true, "apply_instruments": true}
```

一次事务完成：

1. 写 BPM / 拍号；
2. 用 `scenes` 整体替换段落表；
3. 用 `music.harmony` 整体替换和弦表；
4. 按 `instrument` / `track` 分组，每组建一条轨道；
5. 每条轨道按 cue 建 MIDI 片段（`t0/t1` 换成拍）；
6. 用 `find_instrument` 同一套规则挂内置音色 + 参数补丁 + 建议效果。

**只占一个撤销步。** 返回 `trackIds` / `clipIds` / 每轨摘要 / warnings。

注意：

- `clear: true` 会先清空当前工程（含默认分轨模板），适合「从 film.json 起一个新配乐工程」；
  默认 `false` 是追加，不会删旧轨。
- 重复导入会**再建一套轨道**；不会复用旧轨。导入前先 `get_project_overview`。
- 段落与和弦表是**整体替换**（清单里没有就清空）。
- 不会替你写音符（除非 `cue.notes` 显式给了）。导入后用 `add_notes` 一个乐句一个乐句写。
- `instrument` 解析不到时轨道仍会建，但不挂乐器，warning 里会写是哪一个。

## 3. `find_instrument`：语义音色

```json
{"query": "guqin-like", "limit": 5}
```

返回：

```json
{
  "matches": [
    {
      "pluginId": "builtin.nylon_guitar",
      "confidence": "high",
      "why": "没有古琴音源；尼龙弦木吉他的拨弦 + 混响是最接近的内置近似",
      "presetId": null,
      "params": {},
      "effects": ["builtin.reverb"],
      "roles": ["chords", "lead"],
      "pitchRange": [40, 88],
      "mono": false
    }
  ]
}
```

内置映射（均为**近似**，不是原声采样）：

| 语义名 | 推荐 | 说明 |
| --- | --- | --- |
| `guqin` / 古琴 | `builtin.nylon_guitar` + reverb | 拨弦 + 长衰减 |
| `guzheng` / 古筝 / `pipa` / 琵琶 / koto | `builtin.pizzicato` + reverb | 刮奏用快速五声音阶 |
| `gamelan` / 甘美兰 | `builtin.synth` + `builtin.synth.ep.tine` | 金属泛音 + 快衰减 |
| `shamisen` / 三味线 / sitar | `builtin.nylon_guitar` | 短音 + 快速交替拨弦 |
| `erhu` / 二胡 | `builtin.violin` + reverb | 单音连奏；滑音用 `expr` |
| `dizi` / 笛子 / flute / ocarina | `builtin.synth` + `builtin.synth.winds.flute` | 气息感靠混响 |
| piano / strings / bass / drums / pad / brass | 对应内置采样乐器 | 直接可用 |

**排序与语言：**

- **本机安装的插件优先**：名字/标签/安装包文件名命中（`source:"installed"`）永远排在
  内置近似（`source:"approximation"`）之前。装了真的 Guqin/Pipa，`guqin-like` / `pipa-like`
  会先返回它。
- **中英别名**：查询会拆词，并带上语义规则里的中英别名。`guqin` 能搜到中文名「古琴」，
  `古琴` 也能搜到英文名 `Guqin`；`pipa` ↔ 琵琶同理。
- **匹配字段**：插件名 > 标签 > **插件自述** > 安装包文件名 > 厂商/id > 描述/分类，
  大小写、空格、连字符不敏感。返回里的 `score` / `confidence` / `why` 会说明是哪一项命中的。
- **插件自述也参与匹配**：VST3 读 `moduleinfo.json` 的 `Description`，CLAP 读 descriptor 的
  `description`。所以插件名是 `Zebra9000`、自述里写 “guqin-like plucked zither” 也能搜到；
  中文查询同样会命中（规则别名）。
- **搜不到的诚实边界**：如果插件名是完全无意义的代号、又没有标签/自述/可读的包名，
  `find_instrument` 无法从名字推断音色。这时用 `list_plugins` / `describe_plugin` 看参数表，
  不要硬猜。

**不要编造插件名。** 内置没有的专用音色，要么用近似，要么按 `clap-plugin-dev.md` 做一个 CLAP。

## 4. `export_stems`：按时码交分轨

```json
{"dir": "/abs/stems", "start_seconds": 29.0, "end_seconds": 35.0, "include_mix": true}
```

也可以给 `start_beat` / `end_beat`；两组都出现时 **秒优先**。

产物：

```
/stems/
  古琴-like.wav
  gamelan-like.wav
  mix.wav
  stems.json
```

`stems.json` 形状：

```json
{
  "project": "137s 配乐",
  "sampleRate": 44100,
  "bpm": 120,
  "timeSignature": [4, 4],
  "startBeat": 58,
  "endBeat": 70,
  "startFrame": 1278900,
  "tailSeconds": 0.5,
  "stems": [
    {
      "trackId": 3,
      "name": "guqin-like",
      "path": "/abs/stems/guqin-like.wav",
      "startFrame": 1278900,
      "frames": 529200,
      "gainDb": -3,
      "pan": 0,
      "lufs": -18.2,
      "loudnessRangeLu": 4.1,
      "truePeakDbtp": -1.8,
      "samplePeakDbfs": -1.9,
      "rmsDbfs": -21.4
    }
  ],
  "master": { "path": "/abs/stems/mix.wav", "lufs": -14.1, "truePeakDbtp": -1.0 },
  "warnings": []
}
```

- `startFrame` 由 `TempoMap` 的 `beats_to_seconds(startBeat) × sampleRate` 算出，**逐样本对齐**；
  不要用文件名或导出顺序猜位置。
- 每个 stem 末尾保留 **0.5s 尾巴**（总线混响/延迟），清单里是 `tailSeconds`；起始位置仍然精确，
  结束位置 = `frames / sampleRate`，不要假设刚好等于 `endBeat`。
- 省略 `start_beat` / `end_beat` = 整曲。
- 缺省跳过没有音符/音频的轨道；显式传 `tracks` 时按给定列表导。
- **串行渲染**：每条轨都做一次离线渲染，20 条轨道会比单轨慢约 20 倍。
- 分轨会保留**发送到总线**的效果（总线本身没有片段时保持可听），所以 stem 里带着混响/延迟；
  总线自己有片段时会按普通轨道静音。
- `mix.wav` 是未做母带的混音；要交付母带再跑 `master_project`。

## 5. `what_changed`

```json
{"since_op_id": 41, "limit": 20}
```

返回 `{currentOpId, ops:[{id, at, actor, kind, target, args, event_count, applied}]}`。
agent 改完一批后先看它，别重读全量工程。

## 6. 标准闭环

```
film.json
  → get_project_overview
  → import_cue_sheet {path, name?}
  → find_instrument（对每个 instrument 确认音色）
  → add_notes（按段落写，一个乐句一个事务）
  → quantize_notes / apply_swing / strum_notes / humanize_notes
  → analyze_harmony / analyze_track_overlap / get_track_note_stats
  → analyze_project
  → export_stems {dir, start_beat, end_beat}
  → what_changed（确认这一轮改了什么）
```

## 7. 边界（v1）

- 不生成音频素材；cue 只建 MIDI 骨架与音色。
- 不支持逐帧 hit point marker（`scenes[].markers` 目前不写入 DAW；需要时用段落边界近似）。
- 不自动做 varispeed / tape；蒸汽波类效果用 `builtin.chorus` + `builtin.reverb` 近似，
  真正的降速/抖动留给外部渲染或 Web Audio。
- 分轨是整轨渲染后切片；不做单次多轨并行渲染。
