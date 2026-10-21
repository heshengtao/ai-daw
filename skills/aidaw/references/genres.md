# 域：曲风配方（genres）

这一篇是「曲风知识库」：接到需求后先在这里找最接近的一张卡，拿到 **BPM、拍号、音阶/和声、
鼓型、贝斯、配器、结构、混音**，再去写 MIDI。具体怎么把卡片变成 AIDAW 调用，见
`workflow.md`；音符/和弦的理论依据见 `theory.md`；鼓谱记号见 `rhythm.md`。

> 曲风是**倾向**不是法律。下面给的是「80% 的作品长这样」的默认值；用户给了参考曲/明确要求时，
> 以用户为准，只在卡片的骨架上做偏离。

## 0. 先做一张「曲风卡」

分析任何需求，先填这 9 个格：

| 字段 | 要回答的问题 | 例子 |
| --- | --- | --- |
| 曲风/子风格 | 大类 + 细分 | Lo-fi hip-hop |
| 情绪 | 3 个形容词 | 温暖、慵懒、怀旧 |
| BPM | 具体数字，不写范围 | 82 |
| 拍号/律动 | 4/4？swing？half-time？ | 4/4，16 分 swing 0.58 |
| 调/音阶 | 调 + 音阶/调式 | C 大调 / A 小调，Ionian + Dorian |
| 和声 | 3–8 个和弦的循环 | Cmaj7–Am7–Dm7–G7 |
| 鼓 | 用哪套鼓型 | Boom bap 变体 |
| 配器 | 4–6 件，含角色 | 电钢、finger bass、drums、长笛、pad |
| 结构 | 段落与小节数 | Intro 4 + A 8 + B 8 + A' 8 + Outro 4 |

填完再动手。**先定 BPM 和调，再定和声，最后写鼓和旋律**——顺序反了会反复改。

## 1. 快速索引

| 用户可能说 | 看哪张卡 | 默认 BPM | 默认调式 |
| --- | --- | --- | --- |
| 流行、华语、抒情、K-pop、J-pop | 第 2 节 Pop | 100–128 | 大调 / 自然小调 |
| 摇滚、乐队、燃、吉他 | 第 3 节 Rock | 110–140 | Mixolydian / Aeolian |
| 说唱、trap、808、暗黑、drill | 第 4 节 Trap / Hip-hop | 130–150（half-time 65–75） | 自然/和声小调、Phrygian |
| 老学校、boom bap、采样、90s | 第 5 节 Boom bap | 85–95 | 小调/Dorian，七九和弦 |
| 学习、放松、咖啡、chill、lofi | 第 6 节 Lo-fi | 70–90 | 大调/小调，maj7/m9 |
| 电音、蹦迪、house、techno | 第 7 节 House / Techno | 120–150 | 小调，m7 |
| EDM、festival、drop、build | 第 8 节 EDM | 126–132 | 小调，vi–IV–I–V |
| R&B、neo-soul、slow jam | 第 9 节 R&B / Neo-soul | 60–100 | 小调/Dorian，延伸和弦 |
| 爵士、swing、酒馆、jazz | 第 10 节 Jazz | 100–160 | 大调/小调，ii–V–I |
| 蓝调、blues、12 小节 | 第 11 节 Blues | 70–120 | 小调五声 + 蓝音 |
| bossa、拉丁、bossa nova | 第 12 节 Bossa / Latin | 120–140（2/4 感） | 大调/小调，maj7/m7 |
| 电影、史诗、游戏、预告片 | 第 13 节 Cinematic | 60–120 | 小调/和声小调/调式 |
| 国风、古风、仙侠、五声 | 第 14 节 国风 | 70–110 | 五声羽/宫调式 |
| 氛围、太空、冥想、ambient | 第 15 节 Ambient | 60–90 | 调式/持续音 |

## 2. 流行 Pop

| 项目 | 默认配方 |
| --- | --- |
| BPM | 95–130（Spotify 中位约 120；抒情可 70–90） |
| 拍号/律动 | 4/4，直拍；副歌可以加 16 分 hat |
| 调/音阶 | 大调或自然小调；旋律用大调五声最稳 |
| 和声 | `I–V–vi–IV`、`vi–IV–I–V`、`I–vi–IV–V`；副歌可换 `IV–I–V–vi` |
| 鼓 | `rhythm.md` 4.1；kick 1/3，snare 2/4，hat 8 分 |
| 贝斯 | 根音为主，1 拍 + and of 2；八分音符 |
| 配器 | 电钢/钢琴（chords）、finger bass、drums、pad、lead（人声/长笛/合成 lead） |
| 结构 | Intro 4–8 / Verse 8–16 / Pre 4–8 / Chorus 8–16 / Verse / Chorus / Bridge 4–8 / Chorus / Outro |
| 混音 | 人声/lead 居中；keys 稍宽；副歌加 reverb/delay；低频让给 bass 与 kick |

写音符顺序：先写 8 小节和弦循环（每小节 1 个和弦）→ 贝斯根音 → 鼓 → 主旋律。
副歌把旋律音区提高 3–7 个半音，主歌留白。`strum_notes(step_ms=15~25)` 让柱式和弦不那么硬。

## 3. 摇滚 Rock

| 项目 | 默认配方 |
| --- | --- |
| BPM | 110–140（统计中位约 125；朋克 140–180） |
| 拍号/律动 | 4/4，backbeat；可以是 straight 或 shuffle |
| 调/音阶 | Mixolydian、Aeolian、小调五声；强力和弦 |
| 和声 | `I–bVII–IV–I`、`i–bVI–bIII–bVII`、`I–IV–V`；少用复杂延伸音 |
| 鼓 | `rhythm.md` 4.2；snare 2/4，kick 跟 riff，crash 段落头 |
| 贝斯 | 根音 + 五度 + 八度，连续八分，跟 kick |
| 配器 | 失真/过载吉他（chords/lead）、bass、drums；主音可用铜管/合成 lead 替代 |
| 结构 | Intro / Verse / Chorus / Verse / Chorus / Solo 8 / Chorus / Outro |
| 混音 | 吉他双轨左右；bass 与 kick 分频；主音 solo 加 delay |

写音符顺序：先写 riff（强力和弦 0+7，必要时加 12）→ 鼓 → 贝斯跟根音 → solo 用五声。
`add_effect(track, "builtin.drive")` 给吉他/贝斯加毛边；主音 solo 用
`set_note_velocity` 做句首句尾的强弱。

## 4. Trap / Hip-hop

| 项目 | 默认配方 |
| --- | --- |
| BPM | 130–150（按 half-time 记 65–75；drill 135–145） |
| 拍号/律动 | 4/4，half-time 感觉；hat 细分密集 |
| 调/音阶 | 自然小调、和声小调、Phrygian；常用 `i–bVI–bIII–bVII`、`i–bVII–bVI–bVII` |
| 和声 | 小调三和弦/七和弦；可以整段只用一个 drone |
| 鼓 | `rhythm.md` 4.5；808 长音、snare/clap 2/4 或第 3 拍、hat 16 分 + 32 分 roll |
| 贝斯 | 808 跟根音，长音；用 `legato` 连相同音；滑音用相邻半音/全音级进近似 |
| 配器 | 808/sub bass、drums、dark pad/strings、bell/pluck lead |
| 结构 | Intro 4–8 / Verse 16 / Hook 8 / Verse 16 / Hook 8 / Outro |
| 混音 | 808 与 kick 分频/错开；hat 不要刺；人声/lead 居中 |

写音符顺序：先写 808 根音（每小节 1–2 个长音）→ kick/snare → hat（先 16 分，再加 roll）
→ 暗色旋律（小调五声/Phrygian）。808 不要写得太密，留空间给 vocal。
`apply_swing` 通常**不要**用在 trap 的 hat 上；要三连音就直接写 `1/3` 拍。

## 5. Boom bap

| 项目 | 默认配方 |
| --- | --- |
| BPM | 85–95 |
| 拍号/律动 | 4/4，16 分 swing 0.55–0.62；鼓偏松 |
| 调/音阶 | 小调/Dorian；爵士和弦 |
| 和声 | `ii7–V7–Imaj7`、`Imaj7–vi7–ii7–V7`、`i7–iv7–bVII–bIII` |
| 鼓 | `rhythm.md` 4.4；kick 切分、snare 2/4、ghost snare |
| 贝斯 | 根音 + 五度，切分；留空；可以跟 kick |
| 配器 | 采样钢琴/电钢、upright/finger bass、drums、vinyl noise、horn stab |
| 结构 | Intro 4 / Verse 16 / Hook 8 / Verse 16 / Hook 8 / Outro |
| 混音 | 鼓有颗粒；钢琴稍暗；整体不要过度压缩 |

写音符顺序：先写 2 小节和弦（maj7/m7/m9）→ 鼓（swing）→ 贝斯 → 采样式旋律/号角。
`apply_swing(subdivision=0.25, ratio=0.58)`；`humanize_notes(timing=0.02, velocity=10)`。

## 6. Lo-fi / Chillhop

| 项目 | 默认配方 |
| --- | --- |
| BPM | 70–90（统计中位约 109，但 lo-fi 实际常更慢） |
| 拍号/律动 | 4/4，微 swing，整体偏「拖」 |
| 调/音阶 | 大调/小调都行；Dorian、Aeolian；maj7/m9/11 |
| 和声 | `Imaj7–vi7–ii7–V7`、`iiø–V–i`、`Imaj7–iv7–bVII–III` |
| 鼓 | `rhythm.md` 4.6；轻 kick、snare 2/4、hat 低力度、ghost |
| 贝斯 | 根音为主，少变化；音色圆 |
| 配器 | Rhodes/电钢、finger bass、drums、长笛/吉他旋律、vinyl noise/pad |
| 结构 | Intro 4 / A 8 / B 8 / A' 8 / Outro 4（32 小节 ≈ 82 BPM 的 90 秒） |
| 混音 | 低通一点（去掉高频刺）；chorus/reverb 少量；黑胶噪声很轻 |

写音符顺序：先写 8 小节 `Cmaj7–Am7–Dm7–G7` → 鼓（swing + humanize）→ 贝斯 →
电钢用 `strum_notes(step_ms=25)` → 旋律用五声/调式，句尾留白。
**不要**把鼓量化到 100%，也不要所有和弦都弹满 4 拍；留空气。

## 7. House / Techno

| 项目 | 默认配方 |
| --- | --- |
| BPM | House 120–130；Techno 125–150 |
| 拍号/律动 | 4/4，four-on-the-floor；House 有反拍开镲，Techno 更机械 |
| 调/音阶 | 小调、Dorian；短小的和弦/音型 |
| 和声 | `i7–iv7–bVII–bIII`、`i–bVI–bIII–bVII`；也可以只有一个和弦持续 |
| 鼓 | `rhythm.md` 4.3；kick 每拍，clap 2/4，open hat 反拍 |
| 贝斯 | 反拍八分（off-beat），根音/八度，短音 |
| 配器 | 合成 bass、drums、pad/和弦 stab、pluck/arp、fx |
| 结构 | Intro 8–16 / Build 8 / Drop 16 / Break 8 / Build 8 / Drop 16 / Outro |
| 混音 | kick 与 bass 不要抢同一低频；用滤波自动化做 build；侧链感可用 gain 自动化近似 |

写音符顺序：先写 8 小节鼓循环 → bass 反拍 → 和弦/arp → 8 小节一加层。
House 的「反拍」是 8 分音符的反拍（第 2/6/10/14 格），不是正拍。
Techno 尽量少换和弦，靠音色/滤波/节奏推进。

## 8. EDM / Big Room

| 项目 | 默认配方 |
| --- | --- |
| BPM | 126–132（big room 128） |
| 拍号/律动 | 4/4，four-on-the-floor；build 里军鼓滚奏 |
| 调/音阶 | 小调；大调五声/小调五声都能写 hook |
| 和声 | `vi–IV–I–V`、`i–bVI–bIII–bVII`；drop 常只保留根音/五度 |
| 鼓 | 4.3；build 用 8/16 分军鼓加速，drop 回到四拍 |
| 贝斯 | drop 里根音八分/十六分；与 kick 侧链 |
| 配器 | supersaw 和弦、lead、sub bass、drums、riser/impact |
| 结构 | Intro 8 / Build 8 / Drop 16 / Break 8 / Build 8 / Drop 16 / Outro |
| 混音 | 和弦宽、lead 居中、低频单声道；riser 接 impact |

写音符顺序：先写 drop 的 4 小节和弦/lead hook → 鼓 → bass → build（军鼓滚奏 + riser）
→ intro/break（去掉鼓和低频）。用 `apply_preset` 把 `fx.riser` / `fx.boom` 的出厂参数写到
`builtin.synth` 轨道，可以当过渡音效（它们是预设参数，不是独立插件）。

## 9. R&B / Neo-soul

| 项目 | 默认配方 |
| --- | --- |
| BPM | 60–100（neo-soul 70–90） |
| 拍号/律动 | 4/4，laid-back；16 分可带轻微 swing/ghost，但**核心是反拍重音** |
| 调/音阶 | 小调/Dorian；大七、小九、十一、十三和弦 |
| 和声 | `ii7–V7–Imaj7`、`Imaj7–vi7–ii7–V7`、`i9–iv9–bVIImaj7–bIIImaj7` |
| 鼓 | 轻 kick、snare 2/4、ghost 多；hat 低力度；**重音落在 `and` 上** |
| 贝斯 | 根音 + 经过音，切分；可以跟 kick 但更圆 |
| 配器 | Rhodes/电钢、finger bass、drums、pad/strings、lead（人声/长笛） |
| 结构 | Intro 4 / Verse 8 / Pre 4 / Chorus 8 / Verse 8 / Chorus 8 / Bridge 4 / Outro |
| 混音 | 电钢 chorus；人声/lead 居中；低频温暖但不糊 |

写音符顺序：先写 4–8 小节延伸和弦（maj7/m9/11）→ 鼓（ghost；swing 只给需要摇摆的声部）→
贝斯 → 电钢 comping（用 `offset` 做「提前/延后」）→ 旋律。

R&B 的「反拍重音」不要用 `apply_swing` 做：把音写在 `and`（0.5 / 1.5 / 2.5 / 3.5）上，
再用 `set_note_velocity(grid=1.0, beat=0.5, value=92)` 把反拍抬起来，正拍降到 76–84。

`analyze_harmony` 开 `allow_extensions: true`：普通 9/11/13 会记成 `extension` / `info`，
但大三和弦上的 11、b9 这类仍会给 warning（那是风格判断，不是 bug）。

## 10. Jazz

| 项目 | 默认配方 |
| --- | --- |
| BPM | 100–160（ballad 60–80；bebop 160–220） |
| 拍号/律动 | 4/4，swing；ride 的「ding-ding-a-ding」 |
| 调/音阶 | 大调/小调、Dorian、Mixolydian、bebop 音阶 |
| 和声 | `ii7–V7–Imaj7`、`I–vi–ii–V`、rhythm changes、12 小节 blues |
| 鼓 | `rhythm.md` 4.7；ride + hi-hat 2/4 + 军鼓 comping |
| 贝斯 | walking：四分音符，根音→经过音→目标和弦音 |
| 配器 | 钢琴/电钢、upright bass、drums、萨克斯/小号/长笛 |
| 结构 | Head 8–16 / Solos（每人 1–2 段）/ Head / Outro |
| 混音 | 保留动态；不要过度压缩；ride 稍亮，bass 圆 |

写音符顺序：先写和弦谱（每小节 1–2 个）→ walking bass → 鼓（swing）→ comping →
旋律（用和弦音 + 经过音）。`apply_swing(subdivision=0.5, ratio=0.66)`；
`analyze_harmony` 里延伸音很多，开 `allow_extensions`。

## 11. Blues

| 项目 | 默认配方 |
| --- | --- |
| BPM | 70–120（slow blues 60–70；shuffle 90–120） |
| 拍号/律动 | 4/4，shuffle/swing；三连音感 |
| 调/音阶 | 小调五声 + b5（蓝音）；大调 blues 也可 |
| 和声 | 12 小节 `I7–IV7–V7`；quick change 第 2 小节到 IV |
| 鼓 | shuffle：kick 1/3、snare 2/4、hat 三连音感 |
| 贝斯 | 根音 + 五度 + 六度/七度；walking 或 shuffle |
| 配器 | 电吉他、口琴/萨克斯、piano、bass、drums |
| 结构 | Intro 4 / 12-bar chorus × 3–4 / Solo / Outro |
| 混音 | 中频饱满；吉他稍过载；人声/口琴居中 |

写音符顺序：先写 12 小节和弦（全部属七）→ shuffle 鼓 → bass → 吉他 riff/旋律。
蓝音 b5 只作经过音，时值短；`apply_swing(subdivision=0.5, ratio=0.66)`。

**推弦 / 滑音**：现在用音符上的 `expr` 表达，不要再拆成两个相邻半音。
蓝调里「♭3 推弦到 3」写成
`{"pitch": 63, "duration": 1.0, "expr": {"bend": 0, "bend_to": 100, "bend_time": 0.25}}`；
长音再叠 `{"vibrato": 30, "vibrato_rate": 5.5, "vibrato_delay": 0.4, "vibrato_fade": 0.4}`。
逐音/通道的播放差异见 `note.md`；短倚音仍然用 `duration 0.04–0.08` + 负 `offset`。

## 12. Bossa / Latin

| 项目 | 默认配方 |
| --- | --- |
| BPM | 120–140（按 2/4 感记会更慢） |
| 拍号/律动 | 2/4 或 4/4；切分、轻打击 |
| 调/音阶 | 大调/小调；maj7、m7、m9 |
| 和声 | `ii7–V7–Imaj7`、`Imaj7–vi7–ii7–V7`、`i9–iv9–bVII–bIII` |
| 鼓 | `rhythm.md` 4.8；surdo/rim/shaker，轻 |
| 贝斯 | 根音 + 五度，2/4 切分；不要盖过吉他 |
| 配器 | 尼龙吉他、piano、flute/sax、bass、轻打击 |
| 结构 | Intro 4 / A 8 / B 8 / A 8 / Outro |
| 混音 | 高频柔、空间感；打击乐稍宽 |

写音符顺序：先写 8 小节和弦 → 吉他 comping（切分）→ bass → 轻鼓 → 旋律。
吉他不要每拍都扫，留切分空隙；`strum_notes(step_ms=15)` 让和弦有指弹感。

## 13. Cinematic / Orchestral

| 项目 | 默认配方 |
| --- | --- |
| BPM | 60–120（预告片可 90–140 的 half-time） |
| 拍号/律动 | 4/4、3/4、6/8；自由速度 |
| 调/音阶 | 小调、和声小调、Dorian、Phrygian；持续低音 |
| 和声 | `i–bVI–bIII–bVII`、`i–iv–bVII–bIII`、`i–bII–i`；少用流行终止式 |
| 鼓 | `rhythm.md` 4.9；taiko/定音鼓/大鼓，重音 1/3 |
| 贝斯 | 低音持续音/八度；弦乐低音 |
| 配器 | strings、brass、choir/pad、timpani/taiko、piano、harp/glass |
| 结构 | Intro 8 / Theme 16 / Development 16 / Climax 16 / Resolution 8 |
| 混音 | 动态范围大；低频厚；reverb 大；高潮加 brass/choir |

写音符顺序：先写持续低音/ostinato → 弦乐和弦长音 → 主题旋律 → 铜管/打击 → 高潮加层。
用 `set_note_velocity(from=40, to=110)` 做长句渐强；弦乐/铜管不要写太快。

## 14. 国风 / 五声

| 项目 | 默认配方 |
| --- | --- |
| BPM | 70–110（非统计值，按歌曲气质定） |
| 拍号/律动 | 4/4 或 2/4；可自由；少用强 backbeat |
| 调/音阶 | 五声：宫/商/角/徵/羽；羽调式最常用；偏音 4/7 只作经过 |
| 和声 | 省略三度、空五度、sus2/sus4/add9；`I–vi–IV–V` 也可但加五声化处理 |
| 鼓 | 轻的中国大鼓/排鼓/木鱼；4/4 重音 1/3；不要重摇滚 backbeat |
| 贝斯 | 根音/五度长音；少用密集变化 |
| 配器 | 古筝（pizz/pluck）、笛子（flute）、二胡（solo violin/cello）、strings、pad、中国打击 |
| 结构 | Intro 4–8 / A 8 / B 8 / A' 8 / Outro 4–8 |
| 混音 | 中高频清透；笛子/古筝稍靠前；大混响营造空间 |

写音符顺序：先定调式（推荐羽调式 A C D E G）→ 写 4+4 句的旋律（五声骨架，4/7 只经过）
→ 和弦用空五度/sus/add9 → 低音长音 → 轻打击。`analyze_harmony` 传
`allow_approach_notes: true`、`allow_non_chord_tone_duration: 0.5`。
古筝的刮奏可以用快速五声音阶上行/下行音型近似；二胡的滑音用音符 `expr` 写
（`bend` → `bend_to` + `bend_time`，长音再加 `vibrato`），比 `offset`/`legato` 更接近真实滑音。

## 15. Ambient

| 项目 | 默认配方 |
| --- | --- |
| BPM | 60–90（或不定速） |
| 拍号/律动 | 4/4 自由；常无鼓 |
| 调/音阶 | 调式/持续音；Lydian、Dorian、Aeolian |
| 和声 | 长时间持续和弦、add9/sus2、避免强终止 |
| 鼓 | 无，或极轻的脉冲/心跳 |
| 贝斯 | 低音持续音/八度 |
| 配器 | pad、strings、玻璃/钟、field recording、reverb/delay |
| 结构 | 不强调段落；用音色进出制造变化 |
| 混音 | 大空间、长 release、低频稳；不要挤满 |

写音符顺序：先写 8–16 小节的长音和弦（每 2–4 小节换一次）→ 加一层高音动机 →
用 `set_automation` 给 pad 的 cutoff/gain 做缓慢起伏 → 少量延迟/混响。

## 16. 内置音色速查

**采样内置乐器**（`builtin.piano` / `builtin.nylon_guitar` / `builtin.electric_guitar` /
`builtin.bass` / `builtin.drums` / `builtin.strings` / `builtin.slow_strings` /
`builtin.violin` / `builtin.viola` / `builtin.cello` / `builtin.contrabass` /
`builtin.pizzicato` / `builtin.trumpet` / `builtin.french_horns` /
`builtin.synth_strings_1/2` / `builtin.synth_brass_1/2`）见 `plugin.md`：它们是独立插件 id，
挂上即用，参数只有 `gain` / `tone`，不需要 `apply_preset`。

**内置合成器** `builtin.synth` 有 36 条出厂音色（9 类各 4 条）。MCP 侧用
`list_synth_presets()` 取完整参数，再用 `set_params(track, params, reset=true)` 落地；
换音色一定带 `reset=true`，否则上一条音色独有的参数会残留。下面是常用风格的速查。

| 想要 | 关键参数（其余取默认） |
| --- | --- |
| Rhodes 电钢 | `wave 2, fm .3, fmRatio 1, attack .004, decay 1.6, sustain .3, release .35, cutoff .88` |
| FM 电钢（亮） | `wave 2, fm .24, fmRatio 3, attack .003, decay 1.4, sustain .25, release .3, cutoff .92` |
| 暖 Pad | `wave 0, detune .4, attack .35, decay 2, sustain .8, release .9, cutoff .6` |
| 弦乐合奏 | `wave 0, detune .35, attack .18, decay 1.5, sustain .85, release .5, cutoff .72` |
| 长笛 | `wave 3, noise .12, attack .05, decay .6, sustain .85, release .15, cutoff .7` |
| 铜管 | `wave 0, attack .08, decay .7, sustain .8, release .18, cutoff .6, resonance .12, filterEnv .35` |
| 木吉他 | `wave 0, attack .001, decay .6, sustain .12, release .3, cutoff .72, noise .15, pitchEnv .08` |
| 电吉他清音 | `wave 0, attack .002, decay .8, sustain .3, release .35, cutoff .65, resonance .28` |
| Sub bass | `wave 2, sub .7, attack .008, decay .6, sustain .7, release .15, cutoff .42` |
| Finger bass | `wave 0, sub .35, attack .005, decay .5, sustain .4, release .12, cutoff .45` |

鼓的参数见 `rhythm.md` 第 3 节。外部 VST3/CLAP 用 `list_plugins` 查；没有就回退内置。

## 17. 混合与变体

- **Pop + Trap**（trap-pop）：鼓用 trap 的 hat/808，和声用 pop 的 `vi–IV–I–V`，BPM 140–150。
- **Lo-fi + Jazz**：`ii–V–I` + Rhodes + swing，BPM 75–90。
- **Rock + Electronic**（industrial）：四拍底鼓 + 失真 riff，BPM 110–130。
- **国风 + Cinematic**：五声羽调式 + 弦乐/大鼓，BPM 70–100。
- **Bossa + Lo-fi**：bossa 的切分吉他 + lo-fi 的鼓/黑胶，BPM 80–100。

混合时只混**两个**维度（例如鼓型 + 和声），不要每个维度都取一半，否则会变成「四不像」。

## 参考

- Chosic, Typical BPM Range by Genre（按 Spotify 统计的 BPM 中位/10–90 分位）：
  <https://www.chosic.com/bpm-by-genre-list/>
- Open Music Theory, Harmony in pop/rock（常见进行与 schemata）：
  <https://openmusictheory.github.io/popRockHarmony.html>
- Wikipedia, House music（four-on-the-floor、120–130 BPM）：<https://en.wikipedia.org/wiki/House_music>
- Wikipedia, Trap music（808、hat 细分、half-time）：<https://en.wikipedia.org/wiki/Trap_music>
- Wikipedia, Lofi hip-hop（tempo、jazzy chords、vinyl 质感）：<https://en.wikipedia.org/wiki/Lo-fi_hip_hop>
- Wikipedia, Pentatonic scale（中国五声调式）：<https://en.wikipedia.org/wiki/Pentatonic_scale>
