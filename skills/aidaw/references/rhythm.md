# 域：节奏与鼓（rhythm）

鼓和律动决定「像不像这个曲风」。这一篇给三样东西：**拍与网格的换算**、**鼓的 MIDI 写法**、
**让节奏不机械的参数**。和弦/旋律的写法见 `theory.md`，工具语义见 `composition.md`。

## 1. 拍与网格

AIDAW 的所有时间（片段的 `start`/`length`、音符的 `start`/`duration`、`range`）都用**拍**。
4/4 拍号下，1 小节 = 4 拍；1 拍 = 四分音符。

| 记谱 | 拍 | 16 格中的步长 | 备注 |
| --- | --- | --- | --- |
| 全音符 | 4 | 16 | 一小节 |
| 二分音符 | 2 | 8 | 半小节 |
| 四分音符 | 1 | 4 | 一拍 |
| 八分音符 | 0.5 | 2 | 半拍 |
| 十六分音符 | 0.25 | 1 | 最常见的鼓格 |
| 三十二分音符 | 0.125 | 0.5 |  Trap 的 hat roll |
| 三连音（8 分） | 1/3 ≈ 0.3333 | — | 每拍 3 个 |
| 三连音（16 分） | 1/6 ≈ 0.1667 | — | 每拍 6 个 |

`snap` 可选：`off` / `bar` / `1/4` / `1/8` / `1/16` / `1/12`（三连音）/ `1/24` / `1/32`。
**只吸起点**；要写精确三连音就别依赖 snap，直接把 `start` 算成 `1/3` 的倍数。

> 记住：`start` 是**片段内相对拍**。片段从第 9 小节开始时，片段内 0 拍 = 工程第 33 拍。
> `range` 也是片段内拍；只有 `split_clip` 的 `beat` 是工程绝对拍。

## 2. 16 步鼓谱记号

下面的鼓型都用 1 小节 16 格表示，从左到右是 16 个十六分音符：

```
step:  0 1 2 3  4 5 6 7  8 9 10 11  12 13 14 15
beat:  1 . . .  2 . . .  3 .  .  .   4  .  .  .
```

记号：`x` = 正常击打，`X` = 重音，`g` = ghost（很轻），`o` = 开镲，`-` = 休止。

**换算**：第 `s` 格的 `start = s * 0.25` 拍；力度用 `velocity` 表示（见第 6 节）。

## 3. 鼓的三条路线（先选路线，再写音符）

### 路线 A：外部 GM 鼓插件（一套鼓一个轨道）

`list_plugins(kind="instrument", query="drum")` 找本机的鼓机/VST3。若有 GM 兼容鼓组，
**所有鼓音符写在同一条轨道、同一个片段**里，用 GM 音高区分鼓件：

| 音高 | 鼓件 | 音高 | 鼓件 | 音高 | 鼓件 |
| --- | --- | --- | --- | --- | --- |
| 35 | Acoustic Bass Drum | 42 | Closed Hi-Hat | 49 | Crash Cymbal 1 |
| 36 | Bass Drum 1（kick） | 43 | High Floor Tom | 50 | High Tom |
| 37 | Side Stick | 44 | Pedal Hi-Hat | 51 | Ride Cymbal 1 |
| 38 | Acoustic Snare | 45 | Low Tom | 52 | Chinese Cymbal |
| 39 | Hand Clap | 46 | Open Hi-Hat | 53 | Ride Bell |
| 40 | Electric Snare | 47 | Low-Mid Tom | 54 | Tambourine |
| 41 | Low Floor Tom | 48 | Hi-Mid Tom | 56 | Cowbell |

最常用的五个：**kick 36、snare 38、closed hat 42、open hat 46、crash 49**。
军鼓边击用 37，拍手用 39，ride 用 51。

> GM 鼓组在 MIDI 通道 10 上才按这个映射发声；AIDAW 的轨道没有「通道」概念，
> 所以前提是你的鼓插件本身在接收这些音高时映射到对应鼓件。挂上后先写 4 个音试听，
> 确认 kick/snare/hat 对得上，再批量写。

### 路线 B：AIDAW 内置 `builtin.drums`（**默认推荐**，一套鼓一个轨道）

`builtin.drums` 是随包内嵌的**完整原声鼓组**（MuldjordKit lite，3 层力度、22.05k、mono），
直接按上面的 GM 音高写在**同一条轨道、同一个片段**里：

```json
{"track": 4, "clip": 12, "notes": [
  {"pitch": 36, "start": 0.0, "duration": 0.1, "velocity": 110},
  {"pitch": 42, "start": 0.0, "duration": 0.1, "velocity": 70},
  {"pitch": 38, "start": 1.0, "duration": 0.1, "velocity": 110},
  {"pitch": 42, "start": 1.0, "duration": 0.1, "velocity": 55}
], "snap": "1/16"}
```

挂载：

```
set_track_instrument(track, "builtin.drums")
set_track_meta(track, role="drums", min_pitch=35, max_pitch=59)
```

要点：

- **kick 36、snare 38、closed hat 42、open hat 46、crash 49、ride 51**；音高就是鼓件。
- 参数只有 `gain`（0..4）和 `tone`（0..1，一阶低通）；`tone` 调低会让镲片变暗。
- 一套鼓只占**一条轨道**；不要拆成「kick 一轨 / snare 一轨」。
- 旧 `builtin.synth.drums` **只是底鼓预设**，不要拿它当鼓组。

### 路线 C：旧合成器单件鼓（legacy，仅电子鼓/音效）

只有在没有 `builtin.drums`、又想要合成鼓音色时才用：**一件鼓一个轨道**，每轨挂
`builtin.synth`，再用 `apply_preset` 写对应参数。新轨默认没有效果链，`effects` 不传 = 空链；
已有链要一并写回。

| 鼓件 | 轨道音色参数（`apply_preset` 的 `params`） | 建议固定音高 |
| --- | --- | --- |
| Kick | `gain .85, wave 2, attack 0, decay .35, sustain 0, release .05, cutoff 1, resonance 0, filterEnv 0, keyFollow 0, pitchEnv .35, pitchDecay .3` | 36 |
| Snare | `gain .8, wave 3, noise .6, attack 0, decay .16, sustain 0, release .06, cutoff .75, resonance .15, filterEnv .2, keyFollow 0, hpf .2, pitchEnv .2, pitchDecay .12` | 60 |
| Closed hat | `gain .45, wave 0, noise 1, attack 0, decay .045, sustain 0, release .02, cutoff .9, resonance 0, filterEnv 0, keyFollow 0, hpf 1, filterType 2` | 60 |
| Tom | `gain .85, wave 2, attack 0, decay .4, sustain 0, release .08, cutoff 1, resonance 0, filterEnv 0, keyFollow 0, pitchEnv .25, pitchDecay .25` | 45 |

用法（每条鼓轨各来一次）：

```json
{"track": 4, "params": {"gain": 0.8, "wave": 3, "noise": 0.6, "attack": 0,
  "decay": 0.16, "sustain": 0, "release": 0.06, "cutoff": 0.75,
  "resonance": 0.15, "filterEnv": 0.2, "keyFollow": 0, "hpf": 0.2,
  "pitchEnv": 0.2, "pitchDecay": 0.12}, "effects": []}
```

要点：

- **每个鼓件固定一个音高**。合成鼓的音高会改变鼓的调音；整轨都写同一个音高，
  不要像 GM 鼓那样在同一轨写 36/38/42。
- 底鼓想更低就把固定音高往下移（36 → 33），想更紧就往上移；军鼓同理。
- 同一条鼓轨不要同时写 kick 和 snare——它们是两个不同的合成器音色。
- `set_track_meta` 只给**主鼓轨**标 `role: "drums"`；其余辅助打击乐标 `fx` 或不标，
  否则 `analyze_track_overlap` 会把它们当成「两条鼓轨打架」。

## 4. 常用鼓型

### 4.1 流行 / 民谣（Pop）

```
Kick   x . . . . . . . x . . . . . . .
Snare  . . . . x . . . . . . . x . . .
Hat    x . x . x . x . x . x . x . x .
```

变体：副歌把 kick 加到「and of 3」（第 10 格），hat 改 16 分；主歌只留 kick + snare。

### 4.2 摇滚（Rock）

```
Kick   x . . . . . . . x . . x . . . .
Snare  . . . . x . . . . . . . x . . .
Hat    x . x . x . x . x . x . x . x .
Crash  x . . . . . . . . . . . . . . .
```

要点：军鼓 2/4 拍是骨架；crash 落在段落第一拍；副歌 hat 可以全 16 分。

### 4.3 House / Techno（四拍底鼓）

```
Kick   x . . . x . . . x . . . x . . .
Clap   . . . . x . . . . . . . x . . .
OpenHat. . x . . . x . . . x . . . x .
Hat    x . x . x . x . x . x . x . x .
```

要点：kick 每一拍；open hat 在反拍（第 2/6/10/14 格）；clap/snare 在 2/4。
Techno 可以把 hat 写成连续 16 分，底鼓不变。

### 4.4 Boom Bap（90 年代嘻哈）

```
Kick   x . . . . . . x . . x . . . . .
Snare  . . . . x . . . . . . . x . . .
Hat    x . x . x . x . x . x . x . x .
Ghost  . . . . . . . . . . . . . . g .
```

要点：16 分 swing（`apply_swing(subdivision=0.25, ratio=0.58)` 左右）；军鼓 2/4；
kick 错开；ghost snare 力度 25–45。整体 BPM 85–95。

### 4.5 Trap

```
Kick   x . . . . . . . x . . . . . . .
Snare  . . . . . . . . x . . . . . . .
Hat    x x x x x x x x x x x x x x x x
OpenHat. . . . . . . . . . . . . . . o
```

要点：BPM 130–150（或按 half-time 记 65–75）；808 长音跟根音；hat 连续 16 分，
在第 12–15 格做 32 分/三连音 roll（每个 16 分格塞 2–3 个音）；军鼓/拍手常落在第 3 拍
（第 8 格，half-time 写法）；要 2/4 backbeat 就同时写第 2、4 拍（第 4、12 格）。
底鼓与 808 不要在同一时刻抢低频，808 让 kick 半拍。

### 4.6 Lo-fi / Chillhop

```
Kick   x . . . . . . . x . x . . . . .
Snare  . . . . x . . . . . . . x . . .
Hat    x . x . x . x . x . x . x . x .
Rim    . . . . . . . . . . . . . . g .
```

要点：BPM 70–90；swing 0.55–0.62；力度整体偏低（hat 45–75，snare 80–105）；
留一点 timing 抖动（`humanize_notes(timing=0.015, velocity=6)`）。

### 4.7 Jazz Swing

爵士的 8 分音符不是 1:1，而是三连音感觉。用 **12 格/小节**（每拍 3 格）写 ride：

```
step:  0 1 2  3 4 5  6 7 8  9 10 11
Ride   x . .  x . x  x . .  x .  x
HiHat  . . .  . . .  . . .  . .  .   （踏板在 2/4 踩）
```

要点：ride 的「ding-ding-a-ding」；军鼓用 cross-stick/ghost 做 comping；bass 走 walking
（四分音符，根音→经过音→目标和弦音）；`apply_swing(subdivision=0.5, ratio=0.66)` 也可以
近似直录的 swing 8 分。

### 4.8 Bossa / Latin

```
Surdo  x . . x . . x . x . . x . . x .
Rim    . . x . . x . . . x . . . x . .
Shaker x . x . x . x . x . x . x . x .
```

要点：2/4 感觉；低鼓（surdo）在 1 和 2 的反拍；rim/clave 3-2；吉他 comping 切分；
整体轻、不要重踩。BPM 120–140（按 2/4 记会更慢）。

### 4.9 电影 / 史诗

```
Taiko  X . . . X . . . X . . . X . . .
Low    x . . . . . . . x . . . . . . .
Cym    X . . . . . . . . . . . . . . .
```

要点：重音在 1/3 或 1/2；用 `X`（110–127）和 `x`（80–100）做层次；
中段可以加 16 分/三连音 build；最后一段全奏 + crash。

## 5. 贝斯与和弦节奏

| 风格 | 贝斯写法 |
| --- | --- |
| 流行 | 根音为主，1 拍与「and of 2」；八分音符 |
| 摇滚 | 根音 + 五度 + 八度，连续八分；跟 kick |
| House | 反拍八分（off-beat），根音/八度，短音 |
| Trap/808 | 长音根音，跟随和弦根音；用 `legato` 让同音相连；滑音用相邻音级进近似 |
| Boom bap | 根音 + 五度，切分；留空 |
| Jazz | walking：四分音符，根音→经过音→目标和弦音 |
| Bossa | 根音 + 五度，2/4 切分；不要盖过吉他 |
| 国风 | 根音/五度长音，少用密集变化；可用空五度 |

**和弦轨的节奏**：柱式和弦（每小节一次）最稳；流行可以用 8 分切分；Lo-fi/Neo-soul 用
「早一点/晚一点」的 comping（用 `offset`，不要改 `start`）；摇滚用闷音吉他扫 8 分/16 分。
写完柱式和弦后，`strum_notes(step_ms=20~40)` 会让它立刻有「扫过去」的感觉。

## 6. 力度、量化与人性化

### 6.1 力度参考

| 鼓件 | 正常 | 重音 | Ghost |
| --- | --- | --- | --- |
| Kick | 100–115 | 120–127 | 60–75 |
| Snare | 100–120 | 125–127 | 20–55 |
| Closed hat | 50–80 | 90–105 | 30–45 |
| Open hat | 70–95 | — | — |
| Ride | 70–95 | 100–110 | — |
| Crash | 105–125 | — | — |

旋律/和弦：40–110；句首和句尾轻一点，最高点重一点。**不要所有音都 100**。

### 6.2 工具怎么用

- **固定/渐变/缩放力度**：
  `set_note_velocity(track, clip, value=90)`；
  `set_note_velocity(track, clip, from=70, to=110, range={"start":0,"end":16})`；
  `set_note_velocity(track, clip, scale=1.15, range=…)`。
- **量化**：`quantize_notes(grid=0.25, strength=0.8)`。电子鼓用 1.0；真人演奏 0.7–0.9。
- **Swing**：`apply_swing(subdivision=0.5, ratio=0.66, strength=0.7)`（8 分 swing）；
  Boom bap/Lo-fi 用 `subdivision=0.25, ratio=0.58`。
- **人性化**：`humanize_notes(timing=0.02, velocity=8, seed=7)`；同一个 seed 结果一致。
  Trap/House 的 kick 不要 humanize，军鼓/hat 可以。
- **扫弦**：`strum_notes(step_ms=30, direction="down")`；写的是 `offset`，不破坏 `start`。

### 6.3 什么该对齐、什么该留活

| 元素 | 建议 |
| --- | --- |
| EDM/House kick、bass | 硬对齐（strength 1.0），要机械感 |
| Trap 808 | 起点对齐，长音自然衰减 |
| Hip-hop/Lo-fi 鼓 | 70–85% 量化 + swing |
| Jazz | 不量化或 50–70%，保留 push/pull |
| 弦乐/管乐 | 起点可量化，长音用力度曲线 |
| 旋律主唱 | 70–80% 量化，句尾可稍晚 |

## 7. 从鼓谱到 `add_notes`

一条鼓轨 = 一个片段。假设片段从工程第 1 拍开始、1 小节长（4 拍），把 16 格写成音符：

```json
{
  "track": 4, "clip": 12,
  "notes": [
    {"pitch": 42, "start": 0.0,  "duration": 0.1, "velocity": 70},
    {"pitch": 42, "start": 0.5,  "duration": 0.1, "velocity": 55},
    {"pitch": 42, "start": 1.0,  "duration": 0.1, "velocity": 70},
    {"pitch": 42, "start": 1.5,  "duration": 0.1, "velocity": 55},
    {"pitch": 42, "start": 2.0,  "duration": 0.1, "velocity": 70},
    {"pitch": 42, "start": 2.5,  "duration": 0.1, "velocity": 55},
    {"pitch": 42, "start": 3.0,  "duration": 0.1, "velocity": 70},
    {"pitch": 42, "start": 3.5,  "duration": 0.1, "velocity": 55},
    {"pitch": 36, "start": 0.0,  "duration": 0.1, "velocity": 110},
    {"pitch": 36, "start": 2.0,  "duration": 0.1, "velocity": 110},
    {"pitch": 38, "start": 1.0,  "duration": 0.1, "velocity": 110},
    {"pitch": 38, "start": 3.0,  "duration": 0.1, "velocity": 110}
  ]
}
```

如果是内置合成器的**单件鼓轨**，把上面的 `pitch` 换成该鼓件的固定音高（kick 36、
snare 60、hat 60、tom 45），并且**一条轨只写一种鼓**。

多小节鼓型：把上面的 `start` 加上 `(小节序号 - 1) * 4`，一次 `add_notes` 写完整个段落；
不要用循环一条条 `add_note`（一次事务 = 一个撤销步）。

## 8. 常见错误

- **同一轨写 GM 鼓映射，却挂的是 `builtin.synth` / `builtin.synth.drums`**：会听到一个合成器在「唱」鼓点，不是鼓组；完整鼓组用 `builtin.drums`。
- **旧合成器单件鼓轨写多个不同音高**：会变成旋律化的鼓调音，不是 kick + snare；`builtin.drums` 才是按音高分鼓件的。
- **所有力度都是 100**：没有律动，像打字机。至少让 hat 强弱交替、snare 有 ghost。
- **Trap 的 808 和 kick 同时响**：低频打架；让 808 稍晚或让 kick 短。
- **把 swing 应用到已经硬对齐的电子鼓**：四拍底鼓会变瘸；swing 只给需要摇摆的声部。
- **量化 100% 还想要「真人感」**：两者矛盾；先量化到 70–85%，再 humanize。

## 参考

- General MIDI Drum Map（35–81 号音高表）：<https://qsrdrums.com/webhelp-responsive/References/r_general_midi_drum_kit.html>
- MusicProductionWiki, How to Use MIDI in Your DAW（velocity/量化/humanize 的实践）：
  <https://musicproductionwiki.com/articles/how-to-use-midi-in-your-daw>
- Wikipedia, House music（four-on-the-floor 与 120–130 BPM）：<https://en.wikipedia.org/wiki/House_music>
- Wikipedia, Trap music（808、hi-hat 细分、half-time）：<https://en.wikipedia.org/wiki/Trap_music>
