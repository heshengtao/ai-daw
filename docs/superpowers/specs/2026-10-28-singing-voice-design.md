# 歌声合成（SoulX-Singer ONNX）设计

- 日期：2026-10-28
- 状态：已完成（中文普通话 SVS + 参考音色克隆；英文/日文 G2P 与真·SVC 列为后续）
- 参考实现：SXSEditor（MIT）`src/inference/pipeline/*` 的 9 模型 SVS 管线
- 上游模型：SoulX-Singer（Apache-2.0，Soul AI Lab）

## 1. 问题与目标

AI DAW 现在能写 MIDI、能混音，但**没有人声**。用户要的是 DAW 里最自然的一件事：

> 选一段 MIDI（旋律）+ 填歌词 + 选一条参考声线 → 得到一条能继续编辑的歌声轨。

目标：

1. **本地、离线、ONNX**：不依赖云、不要 API Key，权重按需下载，跟现有 `local-ai` 的
   Basic Pitch / HTDemucs 一个路子。
2. **参考声线**：从一段 3–30 秒的干声/演唱里取音色（zero-shot timbre cloning），
   不需要训练、不需要 `.sxsinger` 之类的私有格式。
3. **进同一条命令管线**：产物是普通音频轨 + 普通音频片段，可撤销、可混音、可导出。
4. **国内外双源**：魔搭（国内）+ Hugging Face（国外），sha256 硬门禁，失败自动换源。
5. **换音色**：同一段 MIDI + 歌词换一条参考声线重渲染 = 换音色（见 §7 的取舍）。

非目标（本阶段明确不做）：

- 实时演唱（渲染是离线的，一段 10 秒大约 30–90 秒 CPU 时间）。
- 英文 / 日文 / 粤语的 G2P（模型支持，本阶段只接中文普通话；接口按语言可扩）。
- 音频→音频的真·SVC（SoulX-Singer 的 `model-svc.pt` 只有 PyTorch 权重，没有 ONNX 导出）。
- 歌词逐音符编辑（本阶段歌词是一整串，按顺序贴到音符上；见 §5）。

## 2. 为什么是这 9 个模型

SXSEditor 的导出把 SoulX-Singer 拆成 9 个 ONNX 图，契约稳定、有社区验证：

| 文件 | 作用 | fp16 大小 |
| --- | --- | --- |
| `note_text_encoder.onnx` | 音素 id → 512 维嵌入 | 3.07 MB |
| `note_pitch_encoder.onnx` | MIDI 音高 id → 512 维嵌入 | 0.26 MB |
| `note_type_encoder.onnx` | 音符类型（休止/唱/连音）→ 512 维嵌入 | 0.26 MB |
| `f0_encoder.onnx` | 量化 F0 id（361 档）→ 512 维嵌入 | 0.37 MB |
| `preflow.onnx` | ConvNeXtV2 预流（对三者之和） | 8.6 MB |
| `cond_emb.onnx` | 帧级条件 512 → 1024 | 1.05 MB |
| `diff_step_dml.onnx` | flow-matching 单步（22 层 transformer） | 887 MB |
| `vocoder_dml.onnx` | Vocos：128 维 mel → 24 kHz 波形 | 519 MB |
| `mel_transform.onnx` | 参考音频 → 标准化 mel | 0.26 MB |

模型总大小约 **1.44 GB**，比 fp32（2.9 GB）小一半；fp16 输入/输出在 macOS arm64 与
DirectML 上都可用（实测 Apple Silicon CPU 直接跑得动）。**不随安装包分发**，走
「管理器 → 模型」按需下载，与 HTDemucs 同一套下载器。

## 3. 推理流程（与参考实现对齐）

```
MIDI 音符 + 每音歌词 + BPM
  → 补空隙（gap → <SP> 休止）
  → 每个音：BOW + 音素 + EOW 三个 token（音高/类型各自一条序列）
  → 帧数 = Σ 音符秒数 × 50（24 kHz / hop 480）
  → mel2token：把 token 按帧时长展开（中文 1 音素/音，线性分配）
  → F0：逐音 MIDI → Hz，休止/换气 = 0；量化成 20 cent 一档的 bin
  → 四个编码器求和 → preflow → 按 mel2token 展开 + f0 嵌入 → cond_emb
  → 参考音频 → mel_transform → 标准化 mel（prompt，拼在目标前）
  → flow-matching Euler 采样：32 步，t=(step+0.5)/32，dt=1/32
      cond 分支：xt = [prompt | xt_target]，cond = cond，mask = 全 1
      uncond 分支：xt = [xt_target | 0]，cond = 0，mask = 目标段 1
      v = cond + 3.0 × (cond − uncond)，再按 cond 的标准差 rescale（0.7 权重）
      xt_target += v × dt
  → Vocos 声码器（1024 帧/块，32 帧重叠 + 交叉淡化）
  → 24 kHz 单声道 → 峰值归一 → 落 WAV
```

关键数值（与参考实现逐项一致）：`SAMPLE_RATE=24000`、`HOP=480`、`MEL=128`、
`EMBED=512`、`COND=1024`、`F0_BIN=361`、`F0_MIN=32.7031956625`、30 dB 每档 → 20 cent、
`CFG=3.0`、`CFG_RESCALE=0.7`、默认 32 步。

**prompt 的语义**：参考 mel 放在 xt 前缀、cond 对应位置填 0、mask 全 1；目标段在
`ptFrameCount..`。prompt 上限 500 帧（10 秒），更长的参考音色收益很小、注意力开销线性涨。

**参考声线是必填**（界面与 MCP 都会挡）：这是 zero-shot 音色的唯一来源。底层留了
「32 帧零 mel」的兜底路径（不会崩、也不会静音），但实测音色明显偏噪——高频能量占比
从带参考的 ~3% 升到 ~50%，所以不把它当功能暴露给用户。

**分段**：一个片段可能有几十秒。按休止（≥ 0.25 拍）切成 region，每个 region 单独推理、
按绝对拍位放回；单 region 超过 1000 帧（20 秒）时在最近的音符边界切开，后一段向前多带
2 秒、音频交叉淡化 100 ms。这与参考实现的「8–20 秒、2 秒重叠」同构但更简单。

## 4. 模型分发（双源）

新增清单条目 `soulx-singer-fp16`（`crates/local-ai/src/models/catalog.rs`）：

- `dir: Some("soulx-singer-fp16")`，14 个文件（9 图 + 5 个 `.onnx.data`），整组齐备才算装好。
- `bundled: false`（1.44 GB 不进安装包）。
- 每个文件两个源：`modelscope`（`syxppp/SoulX-Singer-onnx-directml-fp16`，国内）
  + `hf`（`ailm32442/SoulX-Singer-onnx-fp16`，国外镜像，byte-identical）。
- sha256 取自魔搭仓库的 LFS 元数据，逐文件硬校验；下载器已有的「多源回退 + 断点续传 +
  `.part` 原子落地」直接复用。

> 镜像状态：HF 镜像仓库需要上传（`tools/mirror-soulx-hf.sh` 是现成的上传/校验脚本）。
> 未上传前，国外网络的用户会先试 HF（失败）再回落魔搭——仍然可用，只是慢。

## 5. 歌词：逐音字段 + 整串兜底

引擎的 `Note` 现在有 **`lyric` 字段**（`Option<String>`，缺省不写进 JSON，旧工程照常读）：

- `Note.lyric` —— 这个音唱什么：一个汉字（可带 `1..5` 强制声调）、`-`（连音）、`<SP>`（休止）。
- **它优先于整串歌词**：改过的音不会被「一键套词」覆盖，没填的音才由整串按顺序补。
- `Note` 因此不再是 `Copy`（多了 `String`）。所有构造点显式写 `lyric`，转调 / 量化 / 拆分
  等变换用 `..note` 展开时自动带上歌词；`NotePatch` 用 `lyric: ""` 表示清除。

整串歌词（命令参数 `lyrics`）仍然是快速入口：

- 中文字符：一字一音节，按音符顺序贴给**还没有逐音歌词**的音符；`字3` 强制声调。
- `-`：连音（type 3，延续前一个音节的发音）。
- 空白 / 标点：分隔符，不占音符。
- 不够：多出来的音沿用**前一个音节**；一个都没有就用 `la`。

界面：

- 钢琴卷帘里音符上直接显示歌词（9px，放不下截断，完整内容在 tooltip）。
- **歌词鼠标模式**（顶栏 / 快捷键 4）：点音符就地输入；一个音只唱一个记号（汉字 + 可选
  声调，英文一个词），**多写的字自动按起点顺序填到后面的音符**；Enter 提交并跳到最后一个
  被填音符的下一个，Esc 取消。提示写在输入框的 tooltip 里。
- 检查器「歌词」一行显示选中音的音节，点「编辑」进 `NoteEditorDialog` 的**歌词标签页**：
  - 「按顺序填入」把一行文字（与后端同规则切词）铺到选中音符上；
  - 下面是逐音输入框（按起点排序），可以一个一个改；
  - 应用只提交**变过的**音符（一次撤销）。
- AI 面板的「歌词」框仍可整串给；两者混用。

## 5.1 人声音色 = 音色记忆库里带 `voice` 的条目（跨工程）

**不新建第二套库**：人声也是音色，和乐器 / 效果链一起进**音色记忆库**（`sound_memories`）：

- `SoundMemory` 增加 `voice: Option<VoiceAsset>`（`path / seconds / sample_rate / frames`）。
  人声条目没有 `instrument` / `effects`、`mixer` 归零，`tags` 自动补一个 `vocal`，`role = "vocal"`。
- 音频落在 `<配置目录>/sound-memory-audio/<id>.wav`：单声道 24 kHz、峰值归一 −1 dBFS
  （模型要的就是这个格式，10 秒 ≈ 480 KB）。删除条目会连 WAV 一起删。
- **入库自动裁剪成 10 秒**（`singing::reference::trim_to_best`）：按 50 fps 分帧，用
  能量 × 自相关周期性 × 频谱音调性打分，挑平均分最高、有声帧占比最高的一段 10 秒，
  两端 30 ms 淡入淡出。整首歌、带前奏的干声都能直接存，不会把开头静音当 prompt。
  裁剪起点记在 `voice.offset_seconds`，卡片显示「人声参考 · 10 秒 · 取自 3:36」。
  旧版本存下的整首歌条目（例如 4 分 49 秒）在**启动时**由 `trim_long_voices` 就地迁移
  （解码 → 选段 → 覆写同一个 WAV → 更新时长 / 起点，幂等）。
- 直接给 `reference` 文件路径（不走音色库）时，`synthesize` 也会做同一次选段，
  所以「随手丢一首整歌」同样可用。
- **侧边栏「音色库」就是管理界面**：标题右侧一个「乐器 / 人声」二选一切换（同一个库、
  两套内容），统一浏览 / 改名 / 删除 / 按标签筛选。自动标签 `vocal` 显示为「人声」。
- 人声卡片与乐器卡片**同一副骨架**：一样的边框 / 圆角 / 内边距、`width: 100%` 占满整行；
  标题行 + 元信息（「人声参考 · N 秒 · 取自 M:SS」）+ 可点击的标签 chip + 页脚 + 操作行。
  主按钮是「用作参考」（把 id 交给 AI 面板、消费一次并切过去），改名 / 删除仍是图标按钮。
  不显示 raw `role`（vocal）徽标，也没有「应用」（人声套不到轨道上）。
  入库入口在唱歌卡片：「存入音色库」（从工程轨道）与「导入人声…」（从文件）并排放在
  「管理音色库…」左边——两个入口互补，管理仍在侧边栏。
- 唱歌卡片：`参考来源` = 工程内音频轨 / 人声音色库；`管理音色库…` 直接打开侧边栏音色库，
  不再弹独立模态框。
- 后端 `voice_list / voice_save / voice_update / voice_delete` 只是音色记忆的**投影**
  （过滤 `voice.is_some()`），`singing_render` 的 `reference_voice` 指的就是记忆 id。
  `apply_sound_memory` 遇到人声条目会明确报错，而不是把轨道乐器清空。
- MCP 的 `list_voices / save_voice / update_voice / delete_voice` 同义；`apply_sound_memory`
  也受同一条保护。

## 6. 宿主 / API 面

- **Tauri 命令**（`src-tauri/src/singing.rs`）
  - `singing_status()` → 模型是否就位、正在跑什么、进度。
  - `singing_render(track, lyrics, reference | reference_track, steps?, seed?, name?)` → 后台线程渲染，
    事件 `engine://singing` 推进度；成功后一次事务建**新音频轨**（插在 MIDI 轨下面）。
  - `singing_cancel()`。
- **MCP 工具** `sing_song`：同样的参数 + 返回一段摘要（音符数 / 段数 / 时长 / 用了哪条参考）。
  代理可以「写旋律 → 填词 → 出声 → 再调混音」。
- **界面**：AI 面板新增「歌声合成」卡片（选轨道、歌词框、参考声线、步数、生成按钮）。
  渲染效果见 `docs/images/singing-panel.png`（浏览器预览的 mock 数据，同一条卡片的真实渲染）。
- **结果**：`local-ai/sing-<stamp>.wav`，`AddAudioTrack` 插到源轨下一行；
  一次撤销去掉整条轨。

## 7. 「换音色」的取舍

SoulX-Singer 的 zero-shot 能力本来就是「参考音频决定音色」。所以：

- **换音色 = 换参考声线重渲染**。同一条 MIDI + 歌词，选另一条参考，得到另一个人的声音。
  这是模型原生能力，质量与首次合成一致。
- 真正的**音频→音频 SVC**（`model-svc.pt`，2.79 GB + Whisper 语义编码器）没有 ONNX 导出，
  要自己从 PyTorch 导出并验证。本阶段不做，路线记在 §9。

对 DAW 用户来说两者体验差别很小：旋律和歌词本来就在工程里，重渲染比「先转音频再转音色」
更可控（可以只改一个音再重出）。

## 8. 测试

- **纯函数**（无模型，CI 可跑）：歌词分配、补空隙/休止判定、F0 量化边界、
  mel2token 覆盖性（每个 token 都有帧、总帧数守恒）、分段规划。
- **模型存在才算**（`#[ignore]`）：端到端合成 4 个音，检查时长误差 < 1%、RMS 在合理区间、
  无 NaN、基频跟踪目标音高（±3%）。
- **手工听感**：`test-files/ace-step-10s-lyric-zh.wav` 当参考，合成 5 秒乐句。

## 8.5 实施记录（与初稿的偏差）

- **参考声线改成必填**（界面与 MCP 都挡）：初稿留了「无参考 = 中性音色」的入口，实测
  零 mel prompt 的输出高频能量占比 ~50%（带参考时 ~3%），听感是噪声感的气声。底层仍保留
  32 帧零 mel 的兜底（不会崩、不会静音），但不作为用户可见选项。
- **prompt 上限 500 帧**（10 秒）落地；超过的参考音频自动截断。
- **分段**：按休止（≥ 0.5 拍）切 + 单段上限 1000 帧（20 秒）；硬切处两端 8 ms 淡入淡出。
- **实测耗时**（M 系列 CPU、fp16、32 步）：2 秒歌声约 40 秒（含每任务 ~8 秒加载 1.4 GB），
  10 秒约 2–4 分钟；16 步约一半。模型不常驻——每次合成重新加载，换内存不换速度。
- **验证**：`crates/local-ai/tests/soulx_svs.rs`（`#[ignore]`）覆盖时长/峰值/基频；
  另外在运行中的 App 里经 MCP 跑了四种参考（中文流行、folk、低八度、无参考），
  确认「同旋律 + 换参考 → 谱质心 1053 → 842 Hz」的音色变化，以及撤销/建轨都走正常命令管线。
- **逐音歌词验证**（在运行中的 App 里经 MCP）：给 4 个音写 `春 / 江 / - / 花4`、整串歌词留空，
  渲染摘要回报「3 个音节、1 个连音」，输出基频 261 / 293 / 329 / 348 Hz——连音的第 3 个音
  承接的是前一个音节的发音，不是新字。
- **人声音色（音色记忆库）验证**：`create_voice` 单测覆盖「写 24 kHz WAV + 自动 vocal 标签 +
  重新加载 + 删除条目连音频一起删」；早期版本还在运行中的 App 里跑通过「从轨道存 10 秒参考 →
  `reference_voice` 唱歌 → 改名 → 删除」的完整链路（当时的存储路径后来并入了
  `sound-memory-audio/`，命令与交互不变）。
- **国内外双源**：16 个文件各自带 `hf/intl`（`ailm32442/SoulX-Singer-onnx-fp16`）与
  `modelscope/cn` 两条源，逐文件 sha256 与魔搭一致（上传脚本 `tools/mirror-soulx-hf.sh`，
  写完用 `cargo test -p ai-daw-local-ai models::catalog` 复核）；下载器按网络位置排序、
  一边失败自动换另一边。

## 9. 后续

1. 英文 / 日文 G2P（CMUdict 4.3 MB + JP kanji dict，参考实现有现成数据）。
2. `Note.lyric` 逐音符歌词（钢琴卷帘里直接编辑）。
3. 真·SVC：从 `model-svc.pt` + Whisper 导出 ONNX，走同一条「参考音色」入口。
4. 16 步的「快速预览」档、fp32 精度档（x86 CPU 没有 fp16 kernel 时）。
