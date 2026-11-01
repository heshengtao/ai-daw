# 域：分析与导出（analysis）

这是 AI 的「听感」闭环：渲染当前的工程 → 得到量化指标 → 据此调整。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `analyze_project` | `sample_rate`,`start_beat?`,`end_beat?`,`tracks?` | 离线渲染并返回响度（LUFS/LRA/真峰值）+ RMS + 平均幅度谱；可只分析一段/几条轨道。 |
| `export_wav` | `path`,`sample_rate`,`start_beat?`,`end_beat?`,`tracks?` | 渲染并导出 WAV（含 0.5 s 尾音）；可只导出一段/几条轨道。 |
| `export_midi` | `path`,`tracks?` | 导出**标准 MIDI 文件**（`.mid`，SMF type 1）：指挥轨（速度图 / 拍号图 / 段落标记）+ 每条带音符的轨道一条 MIDI 轨（轨道名 / 音符 / 逐音歌词 / CC 自动化）。音频轨不进 MIDI，不渲染音频、很快。 |
| `master_project` | `path?`,`target_lufs`,`ceiling_dbtp`,`stereo_width`,`as_track?` | **母带处理**（纯 DSP）：响度归一 + 真峰值限幅 + 立体声宽度。`as_track=true` 与界面「母带处理」一致——落到一条**新音频轨**（可 A/B、可撤销）；否则用 `path` 写成 WAV 并返回处理前后的响度与真峰值。 |
| `detect_tempo` | `track` | **节拍检测**（Beat This!）：对该轨第一段音频推出**单一全局 BPM + 拍号**写进工程，并把片段首重拍对齐到小节线（速度 / 拍号 / 片段位置**一次事务**，可整体撤销）。需要已安装 Beat This! 模型（`final0`，约 83 MB，在「管理器 → 模型」下载）；没有权重会报错。 |

**要点**
- `analyze_project` 与导出**逐位一致**（同一张图、同一套确定性渲染）。
- **局部渲染**：给 `start_beat` / `end_beat`（工程绝对拍）只分析/导出这一段；给 `tracks`
  只保留这些轨道（其他轨道静音，`solo` 会被清掉）。不传 = 整曲/全部轨道。局部导出会在段尾
  保留 0.5 s 尾音。
- 局部渲染仍然是**先整曲渲染再按拍裁切**：结果与整曲逐位一致，但不是「只渲染这 4 小节」的
  性能优化；超长工程 + 重插件时仍然要等整曲渲染完。
- 响度类字段在静音时是 `null`（-∞ 经 JSON）。
- 闭环示例：`analyze_project` 看到响度偏低 → `set_master_gain` → 再分析一次确认。
- 验证某个倚音/和弦色彩：`analyze_project(start_beat=8, end_beat=12, tracks=[74])`，
  或 `export_wav(path=..., start_beat=8, end_beat=12, tracks=[74])` 单独导出这 4 小节。
- `master_project` 的默认值：`target_lufs` = -14、`ceiling_dbtp` = -1、`stereo_width` = 1（原样）。
  想「响度到 -14 且不削顶」就一次调用搞定，不用手算增益；返回的 `report` 里有 `gain_db`（施加的
  归一增益）与 `reduced_db`（限幅器压了多少），据此判断是否需要回头调整混音。
- **母带落轨 vs 落文件**：`master_project(as_track=true)` 是「界面上的母带按钮」——结果进工程、
  进撤销历史，可以原轨/母带轨 A/B；`master_project(path=...)` 只是导出一个母带文件，不动工程。
  交付文件后要用 `export_wav` 再走一遍时，注意别把母带轨重复叠加。
- **MIDI 导出**：`export_midi` 不渲染、不需要插件，适合把编曲交给 MuseScore / Cubase 继续加工。
  逐音歌词会写成 `FF 05` 歌词元事件；`cc:<n>` 自动化写成控制器事件；混音（增益 / 声像 / 发送）
  不会写进 MIDI。只有音频的轨道会被跳过；一条可导出的 MIDI 轨都没有时直接报错。
- `detect_tempo` **只写一个全局速度**（不写逐小节速度图）——时间轴按「拍」线性绘制，只有恒定
  速度下网格 / 波形才稳定对齐。拍号是「相邻重拍之间的拍数」的众数（分母固定 4）；重拍不足时退回 4/4。
  它同时把片段起点吸附到**下一条小节线**（片段起点不能为负）。

**MIDI 层检查的边界（别把 `analyze_harmony` 当风格裁判）**
- `analyze_harmony` 只拿**和弦符号**解析出的音集逐音比对，不看实际 voicing。传 `key` + `scale`
  后会多一层调式上下文：调内但不在和弦里的音记成 `scale_tone`（短音 `info`、长音 `warning`），
  不再算普通外音。`allow_extensions: true` 会把普通 9/11/13 降成 `info`，但风险延伸音仍是
  `warning`。极短且后接相邻和弦音的音会被标成 `ornament`（启发式）；也可以用
  `ornaments: [{track, clip, id}]` 显式指定，覆盖时长启发式。
- `time_tolerance`（缺省 1/16 拍）只用于小节边界：humanize/swing 把音推过小节线几毫秒时，
  不再报假 `suspension`。它不按 `offset` 的发声位置重新分小节。
- 它**不检测小二度/小九度冲突**。那类问题看 `analyze_track_overlap` 的 `semitone_clashes` /
  `semitone_clash_beats`，以及 `get_track_note_stats` 的同轨版本；`unison_notes` 只表示完全同音。
- 它按**记谱 `start`** 分小节，不按 `offset` 的发声位置；humanize/swing 后贴着小节线的音可能
  被分到上一小节，产生假 suspension。遇到这种情况先人工确认是不是 anticipation。
- `severity` 的 `info / warning / error` 混了不同性质的问题；**按 `kind` 过滤**比数个数有用。
