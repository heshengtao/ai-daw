# 域：编曲工作流（workflow）

这是把「用户一句话」变成「DAW 里能播放的乐谱」的标准流程：

```
需求 → 曲风分析（风格卡）→ 查知识（skills / 可选联网）→ MIDI 设计 → AIDAW 落地 → 检查 → 导出
```

目标不是「生成一堆音符」，而是**先证明你理解这首曲子要什么，再让每一步都可解释、可撤销**。
工具细节见 `composition.md`、`note.md`、`track.md`、`mix.md`；曲风配方见 `genres.md`；
乐理见 `theory.md`；鼓谱见 `rhythm.md`。

## 0. 先判断需求属于哪一类

| 用户说法 | 你要先定的事 | 不要做的事 |
| --- | --- | --- |
| 「做一首 90 秒的 lo-fi」 | BPM、调、段落、循环和弦 | 直接开始随机写音符 |
| 「像某某歌/某某人」 | 提取**风格特征**（BPM/律动/和声/音色），不抄旋律 | 复制原曲旋律/和弦序列 |
| 「要燃一点/伤感一点」 | 情绪 → 曲风卡（见下表） | 只改音量，不改节奏/和声 |
| 「给我一段和弦」 | 调、级数、voicing、每和弦几拍 | 只给和弦名不给节奏 |
| 「帮我改这段」 | 先 `get_project` 看现状，再 `analyze_harmony` | 不看现状就覆盖 |
| 「分析这首参考音频」 | `import_audio` / `detect_tempo`（需 Beat This! 模型）/ `audio_waveform`；曲风靠特征判断 | 声称能「听」出精确曲风 |

**用户没给曲风时**，从关键词映射到 `genres.md` 的卡片：

| 关键词 | 卡片 | 默认 BPM | 默认调式 |
| --- | --- | --- | --- |
| 流行、华语、抒情、K-pop、J-pop | Pop | 100–128 | 大调/自然小调 |
| 摇滚、乐队、燃、吉他 | Rock | 110–140 | Mixolydian/Aeolian |
| 说唱、trap、808、drill、暗黑 | Trap / Hip-hop | 130–150（half-time 65–75） | 小调/Phrygian |
| 老学校、boom bap、90s、采样 | Boom bap | 85–95 | 小调/Dorian |
| 学习、放松、咖啡、chill、lofi | Lo-fi | 70–90 | maj7/m9 |
| 电音、蹦迪、house、techno | House / Techno | 120–150 | 小调 |
| EDM、festival、drop、build | EDM | 126–132 | 小调 |
| R&B、neo-soul、slow jam | R&B / Neo-soul | 60–100 | 延伸和弦 |
| 爵士、swing、酒馆 | Jazz | 100–160 | ii–V–I |
| 蓝调、blues、12 小节 | Blues | 70–120 | 小调五声 + 蓝音 |
| bossa、拉丁 | Bossa / Latin | 120–140（2/4 感） | maj7/m7 |
| 电影、史诗、游戏、预告片 | Cinematic | 60–120 | 小调/调式 |
| 国风、古风、仙侠 | 国风 | 70–110 | 五声羽/宫 |
| 氛围、太空、冥想 | Ambient | 60–90 | 调式/持续音 |

如果两个卡片都像，选**用户情绪更接近**的那个；在风格卡里写「主卡 + 一个混合维度」，
例如「Lo-fi 为主，鼓借用 Boom bap 的 swing」。

## 1. 曲风分析：写一张「风格卡」

不要跳过这一步。把下面的表填完，后面所有 MIDI 决策都从它来：

```md
## 风格卡
- 曲风/子风格：
- 情绪（3 个词）：
- BPM：            （给具体数字）
- 拍号/律动：      （4/4？swing？half-time？）
- 调/音阶：        （调 + 音阶/调式）
- 和声循环：       （3–8 个和弦 + 每和弦拍数）
- 鼓型：           （指向 rhythm.md 的哪一节）
- 配器（4–6 件）： （角色 + 音区）
- 结构：           （段落 + 小节数）
- 参考/来源：      （用户给的参考；可选联网来源 + 日期）
```

**分析参考曲时只提取这些维度**：BPM、拍号、鼓的密度、bass 的节奏、和弦的「复杂度」、
主奏音色、段落能量。不要复制具体旋律、riff 或整段和弦进行；生成**同风格的新材料**。

## 2. 查知识：skills 优先，联网只补「时效性」

### 2.1 先查 skills（本地、与工具语义一致）

```
aidaw_help("genres")     # 曲风卡：BPM/和声/鼓/配器/结构
aidaw_help("theory")     # 音阶/和弦/进行/声部连接/音域
aidaw_help("rhythm")     # 鼓映射/鼓型/力度/量化/人性化
aidaw_help("composition")# 批量写音符、量化、swing、和弦分析的工具参数
```

顺序：**genres → theory → rhythm → composition**。只加载你要用的那几篇，不要全灌。

如果走 CLI：

```bash
aidaw call aidaw_help --json '{"topic":"genres"}'
```

### 2.2 再决定要不要联网

需要联网的情况：

- 用户提到**很新的子风格/艺人**，skills 里没有；
- 需要**当前流行的 BPM/编曲趋势**；
- 用户明确说「查一下最新的」。

不需要联网的情况：

- 只是要工具怎么用（skills 里已有，且不会过时）；
- 只是要基础乐理（`theory.md` 已覆盖）；
- 联网结果与 AIDAW 能力冲突（例如让你用侧链压缩、pitch bend，AIDAW 没有对应工具）。

联网时的做法：

1. 只查 1–2 个问题（例如「genre BPM range 2026」「genre chord progressions」）。
2. 优先开放教材/统计数据（Open Music Theory、Chosic/Spotify BPM 统计、Wikipedia）。
3. 把来源和日期写进风格卡的「参考/来源」。
4. **skills 是工具语义的真相，联网只补风格数据**；冲突时以 skills 为准。

### 2.3 把知识变成「可执行的数字」

不要停在「用爵士和弦」。要落到：

- 调：`C major` / `A minor`；
- 和弦：`Cmaj7 | Am7 | Dm7 | G7`，每和弦 4 拍；
- 音区：keys 48–72、bass 28–48、lead 60–84；
- 鼓：16 格里的具体位置 + 力度；
- 结构：32 小节 = 128 拍（4/4）。

## 3. MIDI 设计：先纸面，后 DAW

在调 AIDAW 之前，先在纸上（或回复里）写清楚这四张表。它们直接对应后面的 JSON。

### 3.1 段落表（单位：拍）

4/4 下，1 小节 = 4 拍。例：32 小节 = 128 拍。

| 段落 | 小节 | start（拍） | end（拍） | 能量 |
| --- | --- | --- | --- | --- |
| Intro | 1–4 | 0 | 16 | 低 |
| A | 5–12 | 16 | 48 | 中 |
| B | 13–20 | 48 | 80 | 中高 |
| A' | 21–28 | 80 | 112 | 高 |
| Outro | 29–32 | 112 | 128 | 低 |

`set_project_sections` 的 `start`/`end` 就是这张表里的**拍**（不是小节号）。

### 3.2 和弦表

每个和弦写 `start`（拍）和 `length`（拍），不写 length 时默认 1 小节（4 拍）：

```json
[
  {"start": 0,  "length": 4, "symbol": "Cmaj7"},
  {"start": 4,  "length": 4, "symbol": "Am7"},
  {"start": 8,  "length": 4, "symbol": "Dm7"},
  {"start": 12, "length": 4, "symbol": "G7"}
]
```

写进 `set_project_harmony` 前先确认符号能被识别（见 `composition.md` 的支持列表）。
**无法识别的符号会直接报错**，不会静默跳过。

### 3.3 轨道表

| 轨道 | role | 音区 | 乐器 | 音符密度 |
| --- | --- | --- | --- | --- |
| Keys | chords | 48–72 | Rhodes/电钢 | 每 1–2 拍一个和弦 |
| Bass | bass | 28–48 | finger/sub | 每拍/切分 |
| Drums | drums | 35–59（GM） | 鼓机/内置鼓 | 16 分网格 |
| Lead | lead | 60–84 | 长笛/合成 lead | 稀疏动机 |
| Pad/FX | pad/fx | 48–76 | 暖垫/黑胶噪声 | 长音 |

`set_track_meta` 的 role 用 `lead` / `chords` / `bass` / `drums` / `pad` / `fx`。
音区写进去后，`fit_notes_to_range` 不给参数时会照它折返。

### 3.4 音符表（写到「格」）

对每个声部，先写出「第几拍、什么音、多长、多响」，再转 JSON：

| 声部 | 写法 |
| --- | --- |
| 和弦 | 每小节 1 次，voicing 用 `theory.md` 的 3/4 声部规则 |
| Bass | 根音为主，1 拍 + and of 2；Trap 用长音 |
| Drums | 16 格 grid，按 `rhythm.md` 的鼓型 |
| Lead | 2–4 小节动机，重复 + 变奏；句尾留白 |
| Pad | 2–4 小节换一次和弦，长音 |

## 4. AIDAW 落地：按顺序调用

下面的调用顺序是**推荐的**，每一步都可在界面上看到、可撤销。先读后写：

```
1. aidaw status                         # 软件在不在
2. get_project                          # 拿轨道/片段 id、速度、拍号、现有和弦（音符是全量）
3. set_tempo / set_time_signature       # 先定全局
4. set_project_harmony                  # 和弦表（整体替换）
5. set_project_sections                 # 段落表（整体替换）
6. add_track × N                        # 建轨（或复用现有轨）
7. list_plugins → set_track_instrument  # 选音色（别凭名字猜）
8. set_track_meta                       # role + pitch_range
9. add_clip                             # 每条轨一个 MIDI 片段
10. add_notes                           # 批量写（一次事务）；带 expr 的音符同时写推弦/揉弦
11. set_track_meta(pitch_bend_range)   # 音源弯音范围不是 ±2 半音时才需要
12. quantize_notes / apply_swing        # 定节奏感（不改 expr）
13. strum_notes / humanize_notes        # 演奏姿态（humanize 要避开倚音）
14. set_note_velocity                   # 力度曲线；可用 grid/beat 只改反拍
15. add 倚音（短 duration + 负 offset） # 最后做，之后不要再量化/humanize
16. add_effect / set_gain / set_pan     # 混音
17. analyze_harmony / analyze_track_overlap / get_track_note_stats
18. analyze_project                     # 响度/频谱
19. export_wav                          # 导出
```

### 4.1 关键约束（不遵守会白写）

- **所有 id 从 `get_project` / `add_track` / `add_clip` 的返回里拿**，不要自己猜。
- `add_clip` 的 `start`/`length` 是**拍**；`add_notes` 的 `start`/`duration` 是**片段内拍**。
- 一次 `add_notes` 写一个段落（最多 5000 个音）；**不要循环 `add_note`**。
- `set_project_harmony` / `set_project_sections` / `set_sends` / `set_automation` /
  `replace_notes` 是**整体替换**；空数组 = 清空。
- 单音乐器（`list_plugins` 里 `mono: true`，例如 `JC303`）**不要叠和弦**。
- 改动越界会被夹住（起点 ≥ 0、时长 ≥ **0.03125 拍**、不超出片段）；返回里的 `clamped` 要看。
  注意 0.03125 拍是 **1/32 拍 = 1/128 音符**，不是 32 分音符（32 分音符 = 0.125 拍）。
- **一次事务 = 一个撤销步骤**：写 1000 个音是一步；连着调 5 次变换是 5 步。先算好再一次调。

### 4.2 写音符的两种模式

**模式 A：先写和弦，再派生其他声部**（推荐给流行/爵士/R&B）

1. `add_notes` 写 keys 的柱式和弦；
2. bass 写根音/五度；
3. drums 按 grid 写；
4. lead 写动机；
5. `analyze_harmony` 查错音。

**模式 B：先写鼓和 bass 的 groove，再补和声**（推荐给 House/Trap/Hip-hop）

1. drums 先写 1–2 小节循环；
2. bass 跟根音；
3. keys/pad 用短和弦或长音；
4. lead 写 hook；
5. `quantize_notes` 把电子声部硬对齐，鼓按需 swing。

### 4.3 一个 90 秒 Lo-fi 的完整参数示例

需求：「做一首 90 秒的 lo-fi，学习/放松，温暖一点。」

**风格卡**：Lo-fi / 82 BPM / 4/4 / C 大调（A 小调色彩）/ `Cmaj7–Am7–Dm7–G7` /
Boom bap 变体 / Rhodes + finger bass + drums + 长笛 + 黑胶噪声 / 32 小节。

```
32 小节 × 4 拍 = 128 拍；82 BPM 下 128 拍 ≈ 93.7 秒。
段落：Intro 0–16、A 16–48、B 48–80、A' 80–112、Outro 112–128（拍）。
和弦：每 4 拍一个，循环 Cmaj7 | Am7 | Dm7 | G7；
      B 段换成 Fmaj7 | Em7 | Dm7 | G7。
```

**轨道与音色**：

| 轨道 | role | 音色/参数 | 音区 |
| --- | --- | --- | --- |
| Keys | chords | 电钢：`wave 2, fm .3, fmRatio 1, decay 1.6, sustain .3, cutoff .88` | 48–72 |
| Bass | bass | `builtin.bass`（SF3 指弹贝斯） | 28–48 |
| Drums | drums | `builtin.drums`（SF2 原声鼓组，GM 35–59） | 35–59 |
| Lead | lead | 长笛：`wave 3, noise .12, attack .05, sustain .85` | 60–84 |
| Pad/FX | pad | 暖垫 + 很轻的黑胶噪声 | 48–76 |

**鼓（1 小节，16 格）**：

```
Kick   x . . . . . . . x . x . . . . .
Snare  . . . . x . . . . . . . x . . .
Hat    x . x . x . x . x . x . x . x .
```

**落地顺序**（把 `<id>` 换成真实 id）：

```
set_tempo {"bpm": 82}
set_time_signature {"num": 4, "den": 4}
```

```json
{"sections": [
  {"name":"Intro","start":0,  "end":16, "role":"intro"},
  {"name":"A",    "start":16, "end":48, "role":"verse"},
  {"name":"B",    "start":48, "end":80, "role":"chorus"},
  {"name":"A2",   "start":80, "end":112,"role":"verse"},
  {"name":"Outro","start":112,"end":128,"role":"outro"}
]}
```

`set_project_harmony` 写 32 条（这里只展示前 4 条）：

```json
{"bars": [
  {"start":0, "length":4, "symbol":"Cmaj7"},
  {"start":4, "length":4, "symbol":"Am7"},
  {"start":8, "length":4, "symbol":"Dm7"},
  {"start":12,"length":4, "symbol":"G7"}
]}
```

然后：`add_track` × 5 → `list_plugins` 选音色 → `set_track_meta` 写 role/音区 →
`add_clip(track=<keys>, start=0, length=128)`（每条轨）→ `add_notes` 写和弦/贝斯/鼓/旋律 →
`apply_swing(subdivision=0.25, ratio=0.58, strength=0.7)` 给鼓 →
`humanize_notes(timing=0.015, velocity=6, seed=7, group_by_start=true)` 给 keys/lead →
`strum_notes(step_ms=25)` 给 keys → `set_gain` 做平衡 →
`analyze_harmony` 检查 → `analyze_project` → `export_wav(path="/绝对路径/lo-fi.wav")`。

**混音参考**：keys -6 dB、bass -3 dB、drums -4 dB、lead -8 dB、pad -12 dB；
keys 加 `builtin.chorus` + `builtin.reverb`，lead 加 `builtin.delay` + `reverb`，
bass 可轻加 `builtin.drive`。具体以 `analyze_project` 的 LUFS/频谱为准。

## 5. 检查：不要「写完就交」

### 5.1 音符层

```
get_clip_notes(track, clip)           # 逐音核对（分页；get_project 是全量，批量摘要是前 32 个）
get_track_note_stats(track)           # 音域、平均力度、每小节密度、同音重叠、同轨半音冲突
analyze_track_overlap(tracks=[...])   # 两条轨是否抢同一音区/同发/小二度·小九度冲突
analyze_harmony(bars=..., tracks=[...], allow_extensions=true,
                allow_approach_notes=true, allow_non_chord_tone_duration=0.5)
```

- `analyze_harmony` 的 `kind`：`non_chord_tone` / `ornament` / `scale_tone` / `extension` /
  `unresolved_leading_tone` / `suspension` / `track_overlap`；`severity` 是 `info` / `warning` /
  `error`。
- **短经过音**（≤ `allow_non_chord_tone_duration`）只给 `info`，不算错；倚音通常走这条，
  后接相邻和弦音的极短音会标成 `ornament`。
- `key` + `scale` 给了调式上下文：调内非和弦音记成 `scale_tone`（短音 info、长音 warning）。
- `ornaments: [{track, clip, id}]` 可以显式把某个音符标成装饰音（覆盖时长启发式）。
- `time_tolerance`（缺省 1/16 拍）容忍 humanize/swing 造成的小节线漂移，避免假 suspension。
- `allow_extensions: true` 会真正放行普通 9/11/13（`extension` / `info`）；不推荐的延伸仍给 warning。
- 半音/小九度冲突看 `analyze_track_overlap` 的 `semitone_clashes`，不要只看 `unison_notes`。
- 国风/爵士/蓝调会有大量「理论上的外音」——先判断是不是风格需要，不要机械地改。

### 5.2 声音层

```
analyze_project(sample_rate=44100)    # LUFS / LRA / true peak / RMS / 频谱（整曲）
analyze_project(start_beat=8, end_beat=12, tracks=[74])  # 只听这一段/这条轨
```

- 流媒体常见目标：约 -14 LUFS，true peak ≤ -1 dBTP；按发布平台调整。
- 低频糊：让 bass 与 kick 分频/错开；中频挤：减少同时发声的声部；高频刺：降 hat/lead 的
  高频或加 EQ。
- 改完再分析一次，确认变化方向对。

### 5.3 交付前清单

- [ ] 速度/拍号/调与风格卡一致；
- [ ] 每条轨都有 role/音区，没有越界；
- [ ] 鼓的 kick/snare/hat 路线一致（`builtin.drums` 的 GM 鼓组，或旧合成器单件，没混用）；
- [ ] 没有单音乐器叠和弦；
- [ ] 力度不是全 100；需要 swing 的声部真的 swing 了；反拍重音不是用 swing 冒充的；
- [ ] 含倚音的旋律没有被 humanize 拆散；倚音 duration 在 0.04–0.08 拍、主音仍在网格上；
- [ ] `analyze_harmony` 没有未处理的 `error`；
- [ ] `analyze_project` 的响度/峰值在目标范围；
- [ ] 导出 WAV 成功，路径可打开。

## 6. 回复用户时说什么

简短说清四件事，不要贴大段 JSON：

1. **风格卡**：曲风、BPM、调、和声循环、结构；
2. **做了什么**：几条轨、什么音色、鼓型、多少小节；
3. **怎么检查的**：和声/重叠/响度的结果；
4. **下一步**：想更亮/更暗/更满/更空时可以调哪里。

如果用户要「更像某首歌」，说明你提取的是**风格特征**，不是复制原曲；旋律与和弦进行是
新写的。若用户要求直接复刻受版权保护的具体段落，拒绝并给同风格的替代方案。

## 7. 失败与回退

| 情况 | 处理 |
| --- | --- |
| 不知道曲风 | 用关键词表选最接近的卡；在回复里说明假设 |
| 没有外部音源 | 用内置合成器预设（`genres.md` 第 16 节）；不要假装有 |
| 和弦符号报错 | 换等价写法（`Cmaj9` → `Cmaj7` + 旋律 D），或先查支持列表 |
| 单音乐器吃掉和弦 | 换复音乐器，或把和弦拆成琶音/分轨 |
| 音符被 `clamped` | 检查 `start` 是否超出片段、`duration` 是否 < 0.03125 拍（1/32 拍，不是 32 分音符） |
| 听起来太满 | 减轨/减音符/换音区；先删再补，不要一直加 |
| 听起来太机械 | 量化 70–85% + swing + humanize；鼓的 kick 保持对齐 |
| 导出失败 | 检查路径权限/磁盘；`analyze_project` 先确认能渲染 |

## 参考

- Open Music Theory（开放乐理教材）：<https://openmusictheory.github.io/>
- MusicProductionWiki, How to Use MIDI in Your DAW（量化/力度/humanize 实践）：
  <https://musicproductionwiki.com/articles/how-to-use-midi-in-your-daw>
- Chosic, Typical BPM Range by Genre（曲风 BPM 统计）：<https://www.chosic.com/bpm-by-genre-list/>
- General MIDI Drum Map：<https://qsrdrums.com/webhelp-responsive/References/r_general_midi_drum_kit.html>
