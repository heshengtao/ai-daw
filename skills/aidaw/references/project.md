# 域：工程（project）

读工程与全局设置、撤销/重做、工程级元数据。**每次编辑前先 `get_project` 看清现状**。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `get_project` | — | 返回工程 JSON（`name` / `transport` / `tracks` / `master`，可能含 `harmony` / `sections` / `note_tags`）——**音符是全量的**。 |
| `get_clip_notes` | `track`,`clip`,`offset?`,`limit?`,`range?` | 按页读一个片段的音符；缺省前 200 个、上限 1000，返回 `notes` / `tags` / `truncated` / `next_offset`。大工程核对音符用它。 |
| `get_transport` | — | 实时传输快照（是否播放、位置帧、设备采样率、循环）。 |
| `undo` | — | 撤销最后一次编辑（返回新快照 + 历史深度）。 |
| `redo` | — | 重做。 |
| `set_tempo` | `bpm` | 改速度（20..300）。 |
| `set_time_signature` | `num`,`den` | 改拍号。 |
| `set_project_harmony` | `bars[]` | 写/清小节和弦表（`{start,length?,symbol}`）。 |
| `set_project_sections` | `sections[]` | 写/清段落表（`{name,start,end,role?}`）。 |
| `set_track_meta` | `track`,`role?`,`min_pitch?`,`max_pitch?` | 写轨道分工与目标音域。 |

**元数据是给「编曲意图」用的，不是必填项**
- `harmony`：和弦符号表，`analyze_harmony` 的主要依据。**整体替换**语义——传空数组 `[]` 即清除。
  `length` 省略时按一个小节（`time_sig_num * 4 / time_sig_den` 拍）铺开；符号写错会在写入时直接报错，
  并指出是根音 / 低音 / 类型 / 修饰哪一段不认识（见 `composition.md` 的支持列表）。
- `sections`：段落标记，`start` 含、`end` 不含。用来告诉人类（和你自己）「这 16 小节是副歌」，
  也是 `analyze_harmony` 之外唯一的段落级信息源。
- `track_meta`：`role`（`lead` / `chords` / `bass` / `drums` / `pad` / `fx`…）与 `pitch_range`。
  `role` 用来判断声部关系（两条 `bass` 重叠通常是错的，两条 `pad` 叠着未必）；
  `pitch_range` 是 `fit_notes_to_range` 的参考音域。字段留空 = 清除该字段；两者都空 = 整块 `meta` 移除。
- `note_tags`：音符级标签，键是 `"track:clip:id"`，值是 `{ornament?, articulation?}`。
  用 `set_note_tag` 写；`analyze_harmony` 会自动把带 `ornament` 的音符标成 `ornament`。
- **空就不序列化**：这些字段都是可选的，没写过的工程 JSON 里不会出现这些键，老工程原样打开。

**要点**
- `get_project` 是**全量快照**（所有音符都在）；`add_notes` / `update_notes` / `replace_notes` 等批量
  工具的返回摘要才只给**前 32 个**音符。要逐个核对时：小工程用 `get_project`，大工程用
  `get_clip_notes` 分页读，别把批量摘要当成完整列表。
- 工程是唯一真相源：AI 与人类改同一份，所有写操作都可撤销。
- 撤销基于「重放」：一次工具调用若产生多条命令，只占**一个**撤销步骤。
- 元数据也走命令管线，同样可撤销；它们不触发音频重建（只改工程结构，不碰渲染）。
