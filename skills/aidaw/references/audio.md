# 域：音频（audio）

导入/录制音频、看波形、本地 AI（转 MIDI / 降噪 / 分离）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `import_audio` | `path`,`track`,`start` | 把本机音频文件（wav/flac/mp3/ogg/m4a）导入成音频片段，落在 `start` 拍。 |
| `audio_waveform` | `path`,`buckets` | 波形包络（每桶 min/max 两个值），用来确认素材内容。 |
| `list_input_devices` | — | 本机可用的音频输入设备。 |
| `set_input_device` | `device?` | 选择输入设备；不传 = 关闭输入流、释放麦克风。 |
| `record_start` | `track`,`start`,`device?` | 开始录音：打开输入设备并从 `start` 拍走带采集。 |
| `record_stop` | — | 停带、写 WAV，并把录音作为一条可撤销的音频片段挂到轨道上。**必须配对调用**。 |
| `transcribe_audio` | `track` | 音频转 MIDI：结果写成**源轨道下面的新 MIDI 轨**（一次撤销）。有 Basic Pitch 模型就用，否则内置 DSP。 |
| `enhance_audio` | `track`,`mode?` | 降噪：结果写成**源轨道下面的新音频轨**（原轨不动）。`mode=voice`（缺省，人声/去混响，优先 LocalVQE）或 `music`（乐器底噪，内置 DSP）。 |
| `sing_song` | `track`,`lyrics`,`clip?`,`reference?`,`reference_track?`,`reference_voice?`,`model?`,`steps?`,`seed?`,`name?` | **歌声合成**（SoulX-Singer ONNX）：把一条 MIDI 轨的旋律 + 一行歌词 + 一条参考声线渲染成**新音频轨**（插在源轨下面，一次撤销）。歌词一字一音（中文字），`-` 连音，标点跳过；歌词不够时多出的音沿用前一字。参考音色三选一：`reference_voice`（人声音色库 id，优先级最高）、`reference_track`（某轨第一段音频）、`reference`（文件路径）。换参考重渲染 = 换音色。歌词可逐音写在音符 `lyric` 上（优先），也可以用 `lyrics` 整串给。需要 SoulX-Singer 模型：`model` 选 `soulx-singer-fp16`（1.44 GB）/ `soulx-singer-fp32`（2.85 GB，保真更高），省略 = 已安装的默认档（fp16 优先）；离线渲染，10 秒歌声在 CPU 上约 2–4 分钟（16 步约一半）。 |
| `swap_voice` | `track`,`clip?`,`reference?`,`reference_track?`,`reference_voice?`,`model?`,`steps?`,`seed?`,`pitch_shift?`,`name?` | **音色替换**（SoulX-Singer-SVC ONNX）：把一条**已有的人声轨**换成参考声线的音色，旋律 / 歌词 / 节奏不动，结果写成**源轨下面的新音频轨**（一次撤销）。参考三选一，语义同 `sing_song`。目标与参考都应是**单声部人声**——混音先 `separate_stems` 取人声，否则 F0 会被伴奏带偏。`pitch_shift` 是整体移调半音（−24..24，男女声对唱用）。模型：`soulx-svc-fp16hq`（推荐，约 2.4 GB）/ `soulx-svc-fp32`（约 3.0 GB）；离线渲染，10 秒约 1–2 分钟 CPU。 |
| `separate_stems` | `track`,`model?` | 音源分离：把该轨第一段音频拆成**鼓 / 贝斯 / 其他 / 人声**四条分轨，插到源轨道下面（一次事务，可整体撤销）。需要已安装 HTDemucs 模型（约 209 MB，在「管理器 → 模型」下载）；没有权重会报错，**没有内置兜底**。 |
| `list_voices` | — | 人声音色库：列出 id / 名字 / 描述 / 标签 / 时长。唱歌前先看这里有没有现成声线。 |
| `save_voice` | `name?`,`track?`,`path?`,`description?`,`tags?` | 把参考声线存进**音色记忆库**（单声道 24 kHz、自动 `vocal` 标签，跨工程可用）。`track` 取工程内音频轨第一段，或 `path` 给文件。**整首歌也行**：会自动挑 10 秒最像人声的片段存下来（起点记在 `offset_seconds`）。 |
| `update_voice` | `id`,`name?`,`description?`,`tags?` | 改音色元数据（不动音频）。 |
| `delete_voice` | `id` | 删除音色（连音频）。 |
| `list_models` | — | 列出本地 AI 模型：id / 用途 / 体积 / 是否随包 / 是否已安装 / 可下载源。 |
| `download_model` | `id`,`confirm` | **高风险**：下载模型权重（可能数百 MB）。必须用户明确同意且 `confirm=true`；否则会被拒绝。 |

**模型下载（高风险，必须先问用户）**

**先看 `get_capabilities`**：它会告诉你每个功能缺哪个模型/运行时、多大、该调哪个工具，
并直接给出 `recommended_workflow`（`route: "ai_draft"` = 写歌初稿 + 分离；`route: "manual_midi"`
= 自写 MIDI + 配器）。生成类工具（写歌 / 歌声 / 换声 / 分离 / 节拍）在开跑前也会做同样的检查——
缺依赖时**不会开跑**，而是返回
`{"ok": false, "status": "unavailable", "missing": [...], "fix": [...], "fallback": "..."}`
的**正常结果**（不是让你重试的错误）。看到 `ok:false` 就停下来：要么按 `fix` 征得用户同意后安装，
要么按 `fallback` 换一条路，**不要重复调用同一个不可用的功能**。

`transcribe_audio` / `enhance_audio` / `separate_stems` / `detect_tempo` / `sing_song` / `swap_voice` / `compose_song`
依赖本地模型。Basic Pitch / LocalVQE 随包；HTDemucs、Beat This 等大模型要按需下载：

1. 先 `get_capabilities`（或 `list_models`）看该功能缺什么、`installed` / `downloadable` / `approx_bytes`，
   以及它的 `fallback`。
2. 没装时**先把「需要哪个模型、多大、下载后能做什么、不下载能用什么内置兜底」告诉用户**。
3. 只有用户明确同意后，才调用 `download_model(id, confirm=true)`。下载在后台进行，
   之后用 `list_models` 看 `installed`；不要假报「已下载完成」。
4. 用户不想下载就退回内置方案：转 MIDI / 降噪都有 DSP 兜底；音源分离、歌声合成与音色替换
   **没有兜底**，只能等模型或换做法（如手工分轨 / 先用内置合成器弹旋律）。

**AI 运行环境（写歌 / 换后端前先看）**

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `get_capabilities` | — | **只读：能力预检**。逐项给出写歌 / 歌声 / 换声 / 分离 / 节拍 / 转 MIDI / 降噪 / 母带 / MIDI 导出的 `available`、`degraded`、缺什么（模型 / 运行时 + 体积）、修复工具与替代方案。**碰任何生成类功能前先调它。** |
| `get_ai_status` | — | 只读：聚合任务（普通 AI / 歌声 / 音色替换）、下载（模型 / 运行时 / 引擎）、YuE2 引擎状态与执行后端。 |
| `list_runtimes` | — | 只读：ONNX Runtime（`cpu` / `cuda` / `cuda13` / …）与 audio.cpp 引擎（`audiocpp-*`）的安装状态。 |
| `install_runtime` | `id`,`confirm` | **高风险**：下载并安装运行时（联网、可执行文件）。必须用户同意 + `confirm=true`。 |
| `remove_runtime` | `id`,`confirm` | **高风险**：删除已下载的运行时。只在用户明确要求时调用。 |
| `remove_model` | `id`,`confirm` | **高风险**：删除已下载的模型权重。只在用户明确要求时调用。 |
| `cancel_ai_task` | — | 取消所有正在跑的 AI 任务与下载；写歌会顺带停掉引擎。 |
| `get_ai_backend` / `set_ai_backend` | `backend?` | 执行后端（`cpu` / `coreml` / `directml` / `cuda12` / `cuda13`）。换后端可能要重启。 |
| `start_yue_engine` / `stop_yue_engine` | `model?` | 手动起停 audio.cpp 引擎。写歌工具会自动拉起，一般不用手动。 |
| `get_yue_engine_log` | `lines?` | 只读：引擎日志尾部，起不来时先看它。 |
| `set_yue_cuda_perf` | `enabled` | 引擎的 CUDA 性能预设（默认开）。 |

**写歌的前置条件**：`compose_song` 需要两样东西——audio.cpp 引擎（运行时里的
`audiocpp-*`，用 `install_runtime`）与**一份写歌权重**（模型里的 `yue2-q8-0` / `yue2-q4-0` /
`yue2-bf16`，或 `ace-step-turbo-q8` / `ace-step-turbo-bf16`，用 `download_model`）。
两者都装好之前，写歌会报「引擎启动失败 / 没装」。两家的能力差别见 `composition.md`
的「用哪份写歌权重」。

**改逐音歌词**：`update_notes` 支持 `lyric`（空串清除）——歌声合成发现某个字唱错时改这一个音
即可，不用删了重建（删了会换 id，后续 `set_note_tag` 的引用会失效）。

**两条整曲路线**：写歌可用时「`compose_song` 初稿 → `separate_stems` → 按需 `swap_voice` /
`transcribe_audio`」；不可用时「自己写 MIDI + 自主配器 / 捏音色」。选择依据与完整步骤见
`workflow.md` 的「开工前：能力预检与路线选择」。

**要点**
- 片段的拍长按素材**自然时长**换算（导入时不变调）；之后拖宽窄或改速度会按比例重采样（**变调**）。
- 素材缺失/损坏时该片段静音，不影响整曲。
- 录完不调用 `record_stop` 不会产生片段；录音期间输入设备被占用，结束时（或不录了）可 `set_input_device` 不传参数释放。
- `transcribe_audio` / `enhance_audio` 处理的是该轨道**第一段音频**，结果都插在源轨下面，原轨不动，便于 A/B。
- `separate_stems` 的结果是四条新的**音频轨**（源轨不动，便于 A/B 对比）；四条轨一次撤销即可整体回退。
- `sing_song` 的歌词优先用**逐音 `lyric`**，没填的音才由 `lyrics` 按起点顺序补；想「一字多音」
  就把那个音的 `lyric` 写成 `-`（或整串里写 `-`）。唱英文目前会退化成 `la`（中文 G2P 之外的语音暂未接入）。
- `swap_voice` 只换音色，**不换唱法**：F0 / 节奏 / 咬字都来自目标音频。目标或参考是混音时
  先 `separate_stems` 取人声；结算后输出会按目标 RMS 对齐响度，方便 A/B。
