# 域：片段（clip）

片段（clip）是时间轴上的内容容器：MIDI 片段（音符）或音频片段（素材）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_clip` | `track`,`start`,`length` | 新增 MIDI 片段（单位：拍）。 |
| `remove_clip` | `track`,`clip` | 删除片段。 |
| `move_clip` | `track`,`clip`,`start` | 移到新的起始拍。 |
| `resize_clip` | `track`,`clip`,`length` | 改长度（音频片段的素材窗口按比例变）。 |
| `split_clip` | `track`,`clip`,`beat` | 切分：左半段沿用原 id，右半段是新 id。 |
| `duplicate_clip` | `track`,`clip`,`to_track?`,`start?` | 复制（缺省贴在原片段之后）。 |
| `merge_clips` | `track`,`clips[]` | 合并多个 **MIDI** 片段。 |

**要点**
- 音频片段用 `import_audio`（见 `audio.md`）生成，不用 `add_clip`。
- `start` 与 `length` 都是**拍**；改工程速度会让片段的时间跨度（秒）随之变化。
- `split_clip` 的 `beat` 是**绝对拍**，必须严格落在片段内部；音频片段还需要素材窗口长度
  （导入时写进去的 `frames`），否则拒绝。
- 移动 / 缩放不会改动音符的相对坐标；切分时右半段的音符重新从 0 起算。
- `merge_clips` 里有音频片段会**整体拒绝**（不会改一半），保留第一个片段的 id。
