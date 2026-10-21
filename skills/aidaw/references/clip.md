# 域：片段（clip）

片段（clip）是时间轴上的内容容器：MIDI 片段（音符）或音频片段（素材）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_clip` | `track`,`start`,`length` | 新增 MIDI 片段（单位：拍）。 |
| `remove_clip` | `track`,`clip` | 删除片段。 |
| `move_clip` | `track`,`clip`,`start` | 移到新的起始拍。 |
| `trim_clip` | `track`,`clip`,`start`,`length` | 同时改起点与长度。音频片段按源帧调整素材窗口（**不变速、不变调**）；MIDI 片段只改方框，音符停在原时间线位置。 |
| `set_clip_rate` | `track`,`clip`,`rate`,`pitch_semitones?` | **音频片段**的播放速度（1.0 = 原速）与升降调（半音）；方框拍长跟着重算。 |
| `resize_clip` | `track`,`clip`,`length` | 改长度（音频片段的素材窗口按比例变）。 |
| `split_clip` | `track`,`clip`,`beat` | 切分：左半段沿用原 id，右半段是新 id。 |
| `duplicate_clip` | `track`,`clip`,`to_track?`,`start?` | 复制（缺省贴在原片段之后）。 |
| `merge_clips` | `track`,`clips[]` | 合并多个 **MIDI** 片段。 |

**要点**
- 音频片段用 `import_audio` / `record_start` + `record_stop`（见 `audio.md`）生成，不用 `add_clip`。
- **变速与裁剪是两件事**：`trim_clip` 只移动素材窗口（原速播放）；`set_clip_rate` 才改变播放速度
  与音高。要「把这段音频塞进 8 拍」用 `set_clip_rate`，要「只留素材的后半段」用 `trim_clip`。
- `start` 与 `length` 都是**拍**；改工程速度会让片段的时间跨度（秒）随之变化。
- `split_clip` 的 `beat` 是**绝对拍**，必须严格落在片段内部；音频片段还需要素材窗口长度
  （导入时写进去的 `frames`），否则拒绝。
- 移动 / 缩放不会改动音符的相对坐标；切分时右半段的音符重新从 0 起算。
- `merge_clips` 里有音频片段会**整体拒绝**（不会改一半），保留第一个片段的 id。
