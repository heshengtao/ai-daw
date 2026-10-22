# 域：工程（project）

读工程与全局设置、工程文件（保存 / 打开 / 新建 / 历史）、撤销/重做、工程级元数据。
**每次编辑前先 `get_project_overview` 看清现状**（轨道 / 片段 id、乐器、速度、拍号）；要改音符时再按片段用 `get_clip_notes`。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `get_project_overview` | — | **轻量概览**：轨道 id / 名字 / 分工 / 乐器（id 与名称）/ 效果链 / 混音、每个片段的 id / 位置 / 音符数、速度拍号与总长、段落表。不含音符。了解现状先调它。 |
| `get_project` | — | 返回工程 JSON（`name` / `transport` / `tracks` / `master`，可能含 `harmony` / `sections` / `note_tags`）——**音符是全量的**；插件二进制状态只给 `state_bytes`。 |
| `get_project_path` | — | 当前工程的 `path` 与 `dirty`（是否有未保存改动）。保存前先看它。 |
| `get_clip_notes` | `track`,`clip`,`offset?`,`limit?`,`range?` | 按页读一个片段的音符；缺省前 200 个、上限 1000，返回 `notes` / `tags` / `truncated` / `next_offset`。大工程核对音符用它。 |
| `get_transport` | — | 实时传输快照（是否播放、位置帧、设备采样率、循环）。 |
| `save_project` | `path?` | 保存为 JSON；不传 `path` 就存回当前工程文件（从未保存过会报错）。旧版本自动进历史。 |
| `open_project` | `path` | 从文件打开工程，**替换当前工程**（原工程先进历史）。 |
| `new_project` | `track_names?`,`tempo?` | 新建空工程（默认多轨模板），**替换当前工程**（原工程先进历史）。 |
| `rename_project` | `name` | 重命名当前工程。 |
| `list_project_history` | — | 历史快照列表（`id` / 名字 / 时间 / 轨道与音符数）。 |
| `restore_project_history` | `id` | 恢复历史快照（先备份当前工程）。 |
| `undo` | — | 撤销最后一次编辑（返回新快照 + 历史深度）。 |
| `redo` | — | 重做。 |
| `set_tempo` | `bpm` | 改速度（20..300）。 |
| `set_time_signature` | `num`,`den` | 改拍号。 |
| `set_keyframe` | `beat`,`tempo`,`num`,`den` | 在第 `beat` 拍写入 / 更新速度关键点（渐快 / 渐慢）。 |
| `remove_keyframe` | `beat` | 删除该拍的速度关键点。 |
| `set_project_harmony` | `bars[]` | 写/清小节和弦表（`{start,length?,symbol}`）。 |
| `set_project_sections` | `sections[]` | 写/清段落表（`{name,start,end,role?}`）。 |
| `set_track_meta` | `track`,`role?`,`min_pitch?`,`max_pitch?` | 写轨道分工与目标音域。 |

**工程文件与历史**
- `open_project` / `new_project` 会**替换整个工程**（撤销栈重置）；替换前当前工程会自动进历史，
  用 `restore_project_history` 可以回到它。不确定要动之前先 `list_project_history` 看一眼。
- `save_project` 的路径是任意本机路径；覆盖已有文件前，旧版本会进历史。`path` 省略时用当前工程文件。
- 历史最多保留 50 份，恢复出来的那份会从列表取出（它已经是当前工程了）。

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
- 三层读取，按代价从低到高：`get_project_overview`（结构 + id）→ `get_clip_notes`（一个片段的音符，分页）
  → `get_project`（全量快照）。**大工程不要一上来就 `get_project`**——插件状态和所有音符会把上下文挤爆。
- `get_project` 是**全量快照**（所有音符都在）；`add_notes` / `update_notes` / `replace_notes` 等批量
  工具的返回摘要才只给**前 32 个**音符。要逐个核对时：小工程用 `get_project`，大工程用
  `get_clip_notes` 分页读，别把批量摘要当成完整列表。
- 找不到轨道 / 片段时，报错会列出当前工程里**现有的 id 与名字**，直接照抄别猜。
- 工程是唯一真相源：AI 与人类改同一份，所有写操作都可撤销。
- 撤销基于「重放」：一次工具调用若产生多条命令，只占**一个**撤销步骤。
- 元数据也走命令管线，同样可撤销；它们不触发音频重建（只改工程结构，不碰渲染）。
