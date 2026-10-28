# AI DAW 总体计划（修正版）

- 更新：2026-10-10
- 定位：本文件是**项目进度与后续计划的唯一索引**。设计意图见 `specs/2026-10-07-ai-daw-design.md`；逐步实现步骤见 `plans/` 下的分计划。
- 背景：本文档修正了设计文档与分计划中已经与现实不符的若干陈述（见 §2），并给出剩余工作的统一路线（见 §5）。
- **影片配乐接入（2026-10-22）**：新增 `film.json` 消费链路 —— `import_cue_sheet`（一次事务建
  BPM/拍号/段落/和弦/轨道/片段）、`find_instrument`（语义音色映射，内置缺失时诚实给近似）、
  `export_stems`（逐轨 WAV + `stems.json` 的 startFrame/LUFS/真峰值）、`what_changed`（增量读操作记录）。
  引擎新增 `Command::ApplyCueSheet`，整份清单只占一个撤销步。MCP 工具数 99 → **103**；
  新增能力域 `references/film.md`。AI CUT 侧对应 `docs/film-manifest.md`。
  同批修正 smoke 报告里的三个低层摩擦：`new_project(exact_tracks:true)` 只建给定轨道、
  `analyze_track_overlap` 省略 `tracks` 默认全轨、`find_instrument` **已安装插件优先**并支持
  中英别名（guqin ↔ 古琴、pipa ↔ 琵琶）。插件目录现在还会读**插件自述**（VST3
  `moduleinfo.json` 的 `Description` / CLAP descriptor 的 `description`），名字是代号也能按
  描述里的 guqin/古琴 搜到；旧目录缓存需要 `rescan_plugins` 一次才会补上该字段。
- **本次修订（2026-10-10）**：① 把已完成但仍标「待评审 / 未开始」的文档翻正（设计文档 §3/§5 状态、多份 spec 横幅、两份老 plan 横幅）；② 修正 §5 D2 的 MCP 工具数（27 → 30）；③ 记录一批**近期已提交、尚无设计文档的改动**（§1 末尾）；④ 新增 **Stage F（skills + `aidaw` CLI）** 与 **Stage G（本地 AI 能力）** 两个方向。
- **第二次修订（2026-10-10，同日）**：完成 **C2 录音**、**C3 发送/总线/自动化（引擎 + API + 编辑器 UI）**、
  **F3/D3 skills 文档包 + `aidaw_help` 网关**、**G3 AI 面板**、**G0–G2 真模型（Basic Pitch / LocalVQE ONNX）**；
  MCP 工具数 30 → **33**。
  剩余：**F1/F2（`aidaw` CLI：MCP 薄客户端 + 发现/启动）**、**E1/E2**、**G4 其余四项**、
  **模型打包进 `bundle.resources`**（现在靠 `tools/fetch-models.sh`）。
  **（C3 的编辑器 UI 随后已补齐：检查器发送区 + 编排视图的自动化曲线编辑器。）**
  新增：`crates/engine/src/recorder.rs`、`skills/`、`specs+plans/2026-10-10-{recording,mixing-sends-buses-automation}`。
- **第三次修订（2026-10-10）**：**改向 Stage F** —— `aidaw` CLI 由「镜像全部命令」改为
  **MCP 薄客户端**（软件开着就转发 `tools/call`、没开就启动）；原 F1「抽 `crates/runtime` 无 Tauri 运行时」
  降级为 **F5（搁置）**；F2 的传输只用 HTTP（不用 stdio）。理由见 Stage F。
- **第四次修订（2026-10-10）**：**完成 Stage F 的 F1 / F2** —— 新增根 workspace 的 `aidaw` CLI
  （`crates/aidaw`，MCP 薄客户端：`tools` / `call` / `status` / `start`，HTTP 客户端用 `ureq`）；
  `src-tauri/src/mcp.rs` 启停时写 / 删**运行时发现文件** `mcp-url`，让 CLI 拿到实际端口。
  F3 早已完成；**F4（打包）/ F5（无 Tauri 运行时）仍暂缓**。
- **第七次修订（2026-10-10）**：**CLI 挂载改成显式开关** —— 不再「首次启动自动挂载」（那会平白弹一次
  密码框，体验很差）。改成设置页「命令行」里的一个开关；不挂也能用包内二进制的全路径调用，
  skill 里已写明这条回退。顺带删掉 `cli.json`（没有自动行为就不需要状态记忆）。
  **再追加**：新增发现文件 **`cli-path`**（启动时写包内 `aidaw` 的绝对路径，与 `mcp-url` 并列），
  让外部 AI harness **按 bundle id 推导**就能找到 CLI，不必搜索；并把 `build:aidaw` 串进
  `beforeDevCommand`（否则 `tauri dev` 在缺 sidecar 时直接报错）。
- **第八次修订（2026-10-10）**：**完成 Stage E 的 E1（插件管理器）** —— 原「插件商店」扩成
  **安装 / 导入 / 更新 / 删除**四合一，放进右侧栏新面板 `manager`（设置页「插件」一节也有入口）。
  装进**托管库**（应用数据目录 `plugins/{vst3,clap}`，宿主扫描额外带上）；注册表是静态 JSON，
  前端直接读。**明确不引入 `@open-audio-stack/core`**（Node 库，webview 跑不了）。
  新增 Rust 依赖 `ureq`(native-tls)/`sha2`/`zip`/`tar`/`flate2`（均宽松许可）。
  **E2 崩溃隔离仍搁置**。
- **第六次修订（2026-10-10）**：**`skills/` 改成标准 Agent Skill 目录** ——
  `skills/aidaw/SKILL.md`（带 `name` / `description` / `license` frontmatter，入口 + 索引）
  + `skills/aidaw/references/`（原 9 个能力域，后续扩到 14 个；旧路径 `skills/*.md` 与 `skills/README.md` 已随之下移）。
  MCP 的 `aidaw_help` 内嵌路径同步更新。
- **第五次修订（2026-10-10）**：**完成 F4** —— `aidaw` 以 Tauri sidecar 随包分发，并在首次启动时
  被接进 PATH（macOS 软链 `/usr/local/bin/aidaw`，一次管理员授权；Windows 用户 PATH；Linux deb/rpm 天生在
  `/usr/bin`）。设置页新增「命令行」一节。**F5 仍搁置**。
- **第九次修订（2026-10-11）**：**Stage G 大幅推进** —— ① **G4 第 ② 项音源分离（HTDemucs v4）落地**：
  手写移植 `stem-splitter-core` 的 Demucs STFT/iSTFT + 窗口循环进 `crates/local-ai/src/models/demucs.rs`
  （用本仓库自己的 `ort` rc.13 + `rustfft`，**不引第二套运行时**；`stem-splitter-core` 精确锁 `ort =2.0.0-rc.11`
  与本仓库冲突，故不依赖它）。宿主 `KIND_SEPARATE` 一次事务建**四条分轨**（鼓/贝斯/其他/人声），
  MCP 工具 `separate_stems`（**33 → 34**），AI 面板新增「音源分离」卡片。② **模型分发基建落地**：
  `crates/local-ai/src/models/catalog.rs`（编译期内嵌清单）+ `src-tauri/src/models.rs`（下载器：
  多源回退 + `Range` 断点续传 + sha256 + `.part` 原子 rename + 取消），事件 `engine://model`，
  权重落**应用配置目录 `models/`**（宿主启动时经 `models::set_extra_dirs` 登记进搜索路径）。
  **大模型不随包**（htdemucs 209 MB 按需下载）；**两个小模型（Basic Pitch / LocalVQE）改随包**
  （`src-tauri/resources/onnx/` + `bundle.resources`）。③ **侧边栏「管理器」新增「模型」标签**，
  统一查看 / 下载 / 删除模型（与插件共用同一份「网络」配置）。④ 设置页「网络」新增「模型镜像」；
  插件页提示语由「已迁移」改为可操作的「下载失败/代理有问题请到网络页配置」。魔搭（ModelScope）
  源待用户自建仓库后再补。设计/计划：`specs+plans/2026-10-10-local-ai-models*`（本轮更新）。
- **第十次修订（2026-10-15）**：**节拍检测（G5）以新方式回归** —— 改用社区 Rust 移植 crate
  **`beat-this`**（纯 Rust `rten` 后端）重做，**只写一个全局 BPM + 拍号**（不写逐小节速度图，
  那正是上一版撤掉的原因），并把片段首重拍对齐到小节线。权重 `final0`（83 MB）**不随包**、
  进「管理器 → 模型」下载列表；mel 前端（264 KB）随包。宿主 `KIND_TEMPO`、MCP 工具 `detect_tempo`
  （**34 → 35**）、AI 面板「节拍检测」卡片。设计见 `specs/2026-10-15-beat-this-tempo-design.md`；
  验收曲子由 `tools/make-test-song.py` 生成。
- **第十一次修订（2026-10-15）**：**修音频播放的采样率换算** —— 片段读指针一直漏了「素材采样率 /
  工程采样率」这一项（直接 `cursor += rate`），于是 **44.1 kHz 素材在 48 kHz 工程里会快
  48000/44100 ≈ 8.8% 并升调**，尾端还会被方框截掉（听感正是「最后一小节没声音」）。
  `ScheduledAudioClip` 新增 `source_ratio`，播放器直读与 WSOLA 读指针都按它推进——
  `r` 进**读步进**（重采样），**不进伸缩**（WSOLA 保的是帧周期，折进伸缩会照样升调）。
  回归测试：`engine/tests/audio_clip.rs::material_at_another_sample_rate_keeps_its_duration_and_pitch`、
  `engine/src/stretch.rs::sample_rate_conversion_keeps_duration_and_pitch`。
  顺带修底部「工程小节数」**差 1**（内容正好收在小节线上时多算了一小节），以及验收曲子末尾
  不再留 0.4 s 静音尾巴（现在正好 12 小节）。
- **第十二次修订（2026-10-15）**：**修「播放速度 / 升降调」不可用** —— 片段的 `rate` 漏在四处
  「拍 / 时间轴秒 ↔ 素材帧」的换算里（`trim_clip` / `clip_bounds` / `split_clip` / 前端 `ClipWaveform`），
  于是**设过播放速度之后再拖动片段就会把素材窗口设成 `1/rate`**（`rate = 2` 时后半段静音），
  右边缘还能拖到素材早已放完之后，波形也与听到的对不上。现在换算统一走
  `commands::{span_to_source_frames, source_frames_to_span}`（`trimmed_window` / `clip_bounds_for`），
  并补了 3 条回归测试；波形新增 `rate` 参数参与时间轴→素材的映射。
  （引擎侧本身是对的：`rate` 只改时长、`pitch` 只改音高，已用正弦 + 脉冲素材逐档量化核对过。）
- **第十三次修订（2026-10-15）**：**修「变速 / 升降调一开就爆音、卡住」** —— 上一轮的量化核对是在
  离线渲染里做的，**没量实时预算**。WSOLA 的对齐搜索对 `±SEARCH` 每个候选都重算 512 抽头互相关
  = 105 万次乘加**每块**，而一块只出 `HOP` 个输出帧 → 每输出帧 ~2000 次乘加；Debug 构建实测
  **42–170 µs/输出帧**（实时预算 20.8 µs，即 200%–800%）→ 回调必然爆音、卡死。这正是
  「`rate ≠ 1` / `pitch ≠ 0` 才有问题、原速一切正常」的原因——**只有这条路径走 WSOLA**。
  改成**先粗后精**搜索（4 帧步长 + 4 抽点粗搜 → 最优点 ±4 内全抽点精搜），快 ~15×，Debug 下
  实测降到实时预算的 3%–20%。顺带修 `seek` 之后第一块「零模板互相关」把取段拽偏 −SEARCH
  （≈21 ms 错位）：模板能量为 0 时直接跳过搜索。见 `specs/2026-10-09-audio-clip-design.md` §3。
  回归测试：`stretch.rs::seek_lands_on_the_nominal_position_not_a_search_radius_offset`。
  （教训：成本没有进 CI 门禁——离线的音高 / 时长断言全绿，实时预算却爆了 2–8 倍。
  **DSP 断言要连「喂得动实时」一起量**。）

- **第十四次修订（2026-10-15）**：**Stage G ⑤ 生成音乐（ACE-Step 1.5）阶段一落地** —— 先补
  记在 §5 Stage G 里的**前置工程**：模型系统由「**单文件 + 单 sha256**」升级为
  「**多文件组 + 各自 sha256**」（`ModelFile` / `ModelEntry.files` / `ModelEntry.dir`），
  下载器改为**串行逐文件下载 + 逐个原子落地**、进度按**跨文件累计字节**上报、跳过已下文件；
  「可用」判定要求**同一个搜索目录**里齐备整组（`ort` 靠相对路径找 `.onnx.data`，混目录会加载失败）。
  据此新增清单条目 `ace-step-v15`（上游 MIT / 导出 Apache-2.0，**12 个文件 / 约 3.6 GiB**：
  DiT + 文本编码器 + 融合条件编码器 + 词嵌入 + VAE + 各自 `.onnx.data` + Qwen3 分词器 + 静音潜变量，
  INT4/fp16 混合，**不含 LM**）。
  界面按 pane 体系扩一页：**AI 面板更名「AI 功能首页」**并加一张「音乐生成」**入口卡片**，
  **新增独立标签页「AI 音乐生成」**（`PaneId = "aimusic"`，可从「+」标签列表直接打开），
  负责权重状态（整组 `n/N` 文件）、一键下载、进度与取消。
  **阶段二（推理管线：tokenize → 条件编码 → 8 步 DiT 去噪 → VAE 解码）已实现并端到端验收通过**：
  `crates/local-ai/src/models/ace_step.rs`（含调度器 / 提示词 / 静音潜变量 / 确定性噪声 + 12 条单测）、
  探针 `ace_step_probe`（打印图契约，`--generate` 真出 WAV）。
  **风险闸门已通过**：整组 6 张图在**原生 `ort` CPU EP** 上全部建会话成功，**包括 INT4 的 `MatMulNBits`**。
  **一处关键修正**：条件编码必须用**融合的 `condition_encoder`**——条件序列里还有一个 timbre
  编码器产生的 token，`pack_sequences` 的排序语义也在图里，用拆分图自己拼是拼不对的；
  为此清单去掉 `text_projector` / `lyric_encoder`，加入 `condition_encoder_q4v2` 与
  `silence_latent.pt`（文生音乐的 context，学习得到的常量、非 0）。整组 12 个文件 / 约 3.6 GiB。
  **实测**：10 秒音频 18.2 s、30 秒 46.8 s（≈1.5 s CPU / 音频秒）；峰值正好 −1 dBFS；
  同 seed 两次输出**字节相同**；频谱平坦度 0.056（有音调结构，非噪声）；尾段/头段能量比 ≈1.00
  （无时长塌陷）。**宿主接线（`ai_generate` + 落成音轨 + MCP 工具 + 生成表单）待做。**
  见 `specs/2026-10-15-ace-step-music-generation-design.md`。

- **第十五次修订（2026-10-16）**：**移除 ACE-Step 1.5 与「AI 音乐生成」整条线**。理由是
  **实测不成立**：① CoreML 在这套图上**全面更慢**（Basic Pitch 6.5×、LocalVQE 142×、
  6 分轨 12×；详见 `specs/2026-10-16-backend-runtime-download.md` §10.3）；② 即便纯 CPU，
  10 秒音乐也要 **93 s**，其中 LM 规划占 ~55 s（`lm_0.6b_q4.onnx` 没导出 KV cache，
  每 token 全序列前向 O(n²)），而重导 ONNX 需要上游权重（HF 在当前离线环境不可达）。
  对一个轻量级本地应用来说，这不是可接受的成本结构。
  **删掉的**：`models/{ace_step,ace_step_lm}.rs`、`examples/ace_step_probe.rs`、
  清单三条目（`ace-step-v15` / `-q8` / `ace-step-lm`）、`GenerateArgs` / `GenerateLimits` /
  `RepaintWindow` / LM 文本事件与流式增量、`KIND_{GENERATE,REPAINT,COVER,COMPLETE_SPEC}`、
  MCP 的 `generate_music` 工具、前端 `AiMusicPanel.vue` + `stores/musicForm.ts` +
  `PaneId = "aimusic"` + 全部 `aiMusic.*` 文案、`tools/{export-vae-encoder.py}` 与
  `fetch-models.sh` 的 ACE 段、`tokenizers` 依赖。
  **保留的**：多文件模型组机制（`ModelFile` / `ModelEntry.dir` / 分组校验）——它是通用能力，
  与 ACE 无关；`AudioModels` 仅剩 Basic Pitch / LocalVQE / HTDemucs / Beat This!。

- **第十六次修订（2026-10-16）**：**执行后端收成「三选一」，去掉所有下载**。
  既然 CoreML 在这些小模型上实测全面更慢（§第十五次修订的理由①），原先那套
  「CUDA 运行时下载 + `load-dynamic` + `BackendFile` 清单 + 下载进度事件」就没有成立的前提：
  CUDA 本机无法验证，且对几个 MB~百 MB 的模型来说收益不成立。
  **删掉的**：`BackendKind::Cuda`、`cuda-*` 清单条目、`BackendFile` / `BackendSource` /
  `min_driver`、`resolve.rs`（推荐 / 驱动门槛 / 自动选择）、`runtime-download` feature
  （含 `load-dynamic` 与 `prepare_runtime`）、`src-tauri/src/backends.rs` 里的下载器
  （`fs4` / `lzma-rust2` 依赖随之移除）、`EVENT_BACKEND` 与 `stores/backends.ts`、
  下载管理器里的「后端」页签。
  **现在的形态**：`BackendKind { Cpu, CoreMl, DirectMl }`，全部随包零下载；`BackendPreference`
  默认 **CPU**；设置页新增「AI」分区，用分段控件在**本机平台支持**的项里选
  （Intel Mac 无 CoreML、非 Windows 无 DirectML）。改设置立即生效（会话每次现建，无需重启）。

- **第十七次修订（2026-10-28）**：**歌声合成（Stage G6）落地** —— MIDI 旋律 + 一行歌词 +
  一条参考声线 → 24 kHz 歌声轨。实现 `crates/local-ai/src/singing/`（phones / seq / soulx 三层：
  音素表与中文 G2P、token / F0 / mel2token 的纯计算、SoulX-Singer 的 9 张 ONNX 图），
  宿主 `src-tauri/src/singing.rs`（后台线程 + `engine://singing` 进度 + 可取消 + 一次事务建轨），
  MCP 工具 `sing_song`（**103 → 104**），AI 面板新增「歌声合成」卡片。
  模型 `soulx-singer-fp16`（16 个文件、1.44 GB）进模型清单：多文件组、sha256 硬门禁、不随包；
  **国内外双源**：国内=魔搭，国际=`ailm32442/SoulX-Singer-onnx-fp16`（HF，逐文件 sha256 一致）。
  设计见 `specs/2026-10-28-singing-voice-design.md`。换音色 = 换参考声线重渲染（zero-shot）；
  真·SVC（`model-svc.pt`）没有 ONNX 导出，列入后续。

- **第十八次修订（2026-10-28）**：**逐音歌词 + 人声音色进音色库**。① `Note` 新增
  `lyric: Option<String>`（旧工程兼容，缺省不写 JSON），音符不再是 `Copy`；歌声合成优先用
  逐音歌词、没填的音才由整串歌词按顺序补。② **人声复用音色记忆库**：`SoundMemory` 增加
  `voice` 素材（单声道 24 kHz，落 `sound-memory-audio/`），自动打 `vocal` 标签；
  侧边栏「音色库」统一管理（改名 / 删除 / 标签筛选 / 「导入人声…」），人声条目隐藏「应用」。
  命令 `voice_list/save/update/delete` 是音色记忆的投影，MCP 四个对应工具（**104 → 108**），
  `sing_song` 新增 `reference_voice`；`apply_sound_memory` 对人声条目明确报错。
  ③ 顺手修掉 `host-vst3` 的 `au_smoke` 示例，`cargo test --workspace` 恢复全绿。
- **第十九次修订（2026-10-28）**：按试用反馈重做歌声合成的交互。① 钢琴卷帘新增**歌词鼠标模式**
  （快捷键 4）：点音符就地输入，Enter 提交并跳到下一个音，Esc 取消——不再要求进对话框。
  ② AI 面板删掉重复的「整串歌词」输入框，只留「去填词」入口（选中 MIDI 片段并切到歌词模式）。
  ③ 参考声线拆成「参考来源（工程内音频轨 / 人声音色库）+ 具体声线」两步，不再把所有来源
  混在一个分组下拉里；「管理音色库…」改为打开侧边栏音色库。④ 清理测试遗留的失效素材引用。
  设计见 `specs/2026-10-28-singing-voice-design.md` §5 / §5.1。

- **第二十次修订（2026-10-28）**：歌词与音色库的第二轮交互修正。① 歌词鼠标模式支持**溢出
  自动续写**：一个音只留一个记号，多写的字按起点顺序填到后面的音符（`utils/lyrics.ts` 与后端
  同一套切词规则）；逐音输入框的提示写明这个行为。② 侧边栏「音色库」标题右侧加**乐器 / 人声**
  二选一切换（`soundMemories.kind`），标签列表与空态跟着切；从唱歌卡片进来默认落在人声。
  ③ 「导入人声…」从侧边栏移到唱歌卡片、放在「管理音色库…」左边，与「存入音色库」互补。
  ④ 修掉 `local-ai` 里两条会互相踩 `UNSUPPORTED` 静态位的测试（加锁 + 用例开头清零），
  `cargo test --workspace` 稳定全绿。
- **第二十一次修订（2026-10-28）**：音色库的两处观感 / 交互问题。① 人声卡片右上角的 raw
  `role`（vocal）原来是纯文本、点了没反应——去掉；标签 chip 改成**可点击筛选**（点「女声」
  直接按该标签过滤）。② 人声卡片按 MD3 列表卡重做：圆形 tonal 头像、标题 / 支持文本层级、
  主按钮「用作参考」（把 id 交给 AI 面板并切过去，只消费一次）+ 改名 / 删除图标。
  ③ 「乐器 / 人声」切换、标签筛选、空态文案保持；误杀 `ai-cut` 的 vite 后已按 1430 重启。
- **第二十二次修订（2026-10-28）**：**参考声线自动裁剪**。整首歌存进音色库时不再原样保留：
  `singing::reference::trim_to_best` 用「能量 × 周期性 × 音调性」挑 10 秒人声段（FFT 自相关，
  4 分钟约数十毫秒），入库与直接给文件都生效；旧的长条目在启动时 `trim_long_voices` 就地迁移。
  `VoiceAsset` 记 `offset_seconds`，卡片显示「取自 3:36」。用用户存的孙燕姿（4:49）实测：
  自动选中 216.06 秒（89% 有声帧），8 个音全部音高正确（HPS 检测），输出与参考的 MFCC
  余弦 0.95（对比另一条参考 0.48），6 秒 WAV 在 `/tmp/svs-sunyanzi.wav`。
  新增开发工具 `cargo run -p ai-daw-local-ai --example svs_demo -- --ref …`。
- **第二十三次修订（2026-10-28）**：人声卡片**统一到乐器卡片的骨架**——去掉头像、底色与
  更大的圆角（那套让它和乐器卡片风格分裂、内容也不占满整行），改成同样的边框 / 圆角 /
  内边距 + `width: 100%`；标题、元信息、标签、页脚、操作行的排布与乐器卡片一致，主按钮
  仍是「用作参考」。标签 chip 可点筛选、类型切换、`取自 M:SS` 都保留。
## 1. 现状总览

| 编号 | 范围 | 状态 | 证据 / 说明 |
| --- | --- | --- | --- |
| Plan 1 | 无头引擎：工程模型 + 命令/事件 + 离线渲染 + WAV | ✅ 完成 | `crates/{plugin-api,synth,engine}`；`engine/tests/render_to_wav.rs` |
| Plan 2 | cpal 实时播放 + 传输控制 + 离线==实时 | ✅ 完成 | `engine/src/{schedule,player,transport,command_queue,realtime}.rs`；`examples/play_demo.rs` |
| Plan 4a | 最小桌面外壳：窗口 + 工程操作 + 试听 + 导出 | ✅ 完成（栈有变） | `src-tauri/src/commands.rs`；前端**已改用 Vue 3**（见 §2） |
| Plan 4b | 把原生实时传输接进 UI | ✅ 完成 | `src-tauri/src/audio.rs`（专用音频线程）+ `engine://transport` 事件 + `stores/transport.ts`；界面全部改为真实操作（见下） |
| Plan 3 | VST3 / CLAP 宿主 | ✅ 完成（栈有变） | `crates/host-vst3`（VST3 用 `vst3-host`、CLAP 用 `clack-host`，**未用 truce-rack**）；`src-tauri/src/plugins.rs` 装配注册表 + 原生窗口 |
| Plan 5 | 录音 | ✅ 完成 | `crates/engine/src/recorder.rs`（cpal 输入流 + 无锁环 + 消费线程）→ WAV → `AddAudioClip`；底部条录音键 + 输入设备选择 + 电平表 |
| Plan 6 | 音频片段 / 采样器 | ✅ 完成 | `crates/asset`（symphonia）+ `Clip.audio` + 播放路径 + 宿主素材缓存 + `import_audio`/`audio_waveform` + 波形 UI；真机端到端验证 |
| Plan 7 | AI 接口（MCP + skills + 可视化） | 🟡 进行中 | MCP ✅（`src-tauri/src/mcp.rs`，**33 工具**）+ 前端 AI 可视化 ✅（D4）+ **skills 文档包 + `aidaw_help` 网关 ✅（F3）**；本地 HTTP/WS 未做 |
| 新增 | `crates/ipc`（Command/Event schema） | ✅ 完成 | `crates/ipc`（`ai-daw-ipc`）：工程/命令/事件 schema 的**唯一来源**；engine 用 `pub use` 薄壳保持既有路径不变（零行为变化） |
| 新增 | `crates/analysis`（响度/频谱） | ✅ 完成 | `crates/analysis`（ebur128 + rustfft）：LUFS/LRA/真峰值 + RMS + 平均幅度谱；`analyze_project` 命令 |
| 新增 | 插件管理器（原「插件商店」） | ✅ E1 完成 | `src-tauri/src/manager.rs`：注册表（开放注册表静态 JSON）浏览 + 下载 + **sha256 校验** + 解压归档 + 安装 / 导入 / 删除 / 更新；托管库 `app_data/plugins/{vst3,clap}`（宿主额外扫描）。面板 `ManagerPane.vue`。**下载健壮性**：代理/原址双候选 + 重试 + `Range` 断点续传 + 卡片下边缘进度条。**网络设置独立成一节**（见 §4 技术栈）。**E2 崩溃隔离仍搁置**（见 Stage E） |
| 新增 | 混音：发送 / 总线 / 自动化 | ✅ 完成 | `Track.sends` / `Track.automation`；拓扑路由 + 逐块自动化求值；`set_sends` / `set_automation` 命令 + MCP 工具；**编辑器 UI** = 检查器发送区 + 编排视图的自动化曲线编辑器（`AutomationLane.vue`，见 C3） |
| 新增 | 内置效果器（延迟 / 混响 / 合唱 / 过载 / 两段 EQ） | ✅ 完成 | `crates/effects`（`ai-daw-effects`）：施罗德混响 + 插值延迟 + 合唱 + 软削波过载 + 两段 EQ；注册为 `builtin.delay` / `builtin.reverb` / `builtin.chorus` / `builtin.drive` / `builtin.eq`；缓冲只在 `prepare` 分配，零分配门禁已把它们纳入覆盖 |
| 新增 | 合成器音源扩展 + 8 类 32 条预设 | ✅ 完成 | 参数 18→26（噪声/压碎/失谐/FM/音高包络/滤波类型）；预设重写为 8bit/吉他/贝斯/鼓/弦乐/管乐/铺底/音效 各 4 条，可自带效果链。设计见 `specs/2026-10-09-synth-overhaul-design.md` |
| 新增 | 母带处理（纯 DSP） | ✅ 完成 | `crates/mastering`：响度归一 + **真峰值限幅（4× 过采样 + 前瞻）** + 立体声宽度；`ai.rs` 的 `kind = "master"` 任务（结果落到一条新轨道）+ MCP 工具 `master_project`；AI 面板一张卡片。**不依赖任何模型**。设计见 `specs/2026-10-10-local-ai-models-design.md` §6 |
| 品牌 | 应用图标 / favicon | ✅ 完成 | `src-tauri/app-icon.svg`（薰衣草紫，与 `common/Logo.vue` 同图形）→ `tauri icon` 生成 `src-tauri/icons/*`；`public/logo.svg` 作网页 favicon |
| 新方向 | skills 文档包 + `aidaw` CLI（AI harness 可挂载） | ✅ F1–F4 | **skills 包 + `aidaw_help` 网关（F3）**、**`aidaw` CLI（`crates/aidaw`，MCP 薄客户端 `tools`/`call`/`status`/`start`）+ 运行时发现文件 `mcp-url`（F1/F2）**、**随包分发并接进 PATH（F4，Ollama 式）**；F5 无 Tauri 运行时暂缓 |
| 新方向 | 本地 AI 能力（转 MIDI / 降噪 / 音源分离 / 歌声合成） | 🟢 四项已接入 | **Basic Pitch（转 MIDI）+ LocalVQE（降噪）+ HTDemucs v4（音源分离）**（`crates/local-ai/src/models/`，`ort` rc.13 进程内、后台线程、可取消）；转 MIDI/降噪**权重缺失时退回内置 DSP**，音源分离无兜底。**模型分发基建 ✅**：清单 `models/catalog.rs` + 下载器 `src-tauri/src/models.rs`（多源 + sha256 + 断点续传 + 取消）+ 「管理器 → 模型」标签。**小模型随包**，大模型按需下载。**SoulX-Singer 歌声合成（G6）✅**：MIDI + 歌词 + 参考声线 → 新音频轨（9 张 ONNX 图，1.44 GB 按需下载）。AI 面板 ✅（G3） |

### 近期已提交、尚无设计文档的改动（commit `38ef87b`，2026-10-10）

方向是「工程保存 / 会话与外围 UX」，**已提交但尚未写 spec/plan**（后续补文档）：

- **工程保存 / 会话缓存**（`src-tauri/src/session.rs`）：把「当前工程 + 保存基线 + 文件路径」写入应用配置目录的
  `session.json`，下次打开还原（与「保存工程」区分——即使从未保存也照常有）；原子写 + 单测。
- **工程名对话框 / 快捷键速查**（`components/common/ProjectDialog.vue`、`ShortcutsDialog.vue`）。
- **应用偏好**（`stores/settings.ts`）：与工程无关的偏好，目前只有「新工程默认速度」。
- **二级窗口同步**（`composables/useProjectSync.ts`）：内置合成器 / 效果器窗口是独立 Vue 实例，
  订阅 `engine://event` 节流重拉工程快照，避免改动后显示旧状态。
- **面板窗口能力**（`src-tauri/capabilities/panels.json`）：`synth-*` / `effect-*` / `plugin-ui-*` / `blank`
  窗口的权限集合。
- **3 个新内置效果器**（`crates/effects/src/{chorus,drive,eq}.rs`）：合唱 / 过载 / 两段 EQ（在既有延迟 / 混响之外）。
- **三把工具 + 新图标**（`components/common/icons/*`）：选择 / 画笔 / 分割，图标 FloppyDisk / Keyboard / Marquee / Piano / Split。

### Plan 4b 的实际交付（2026-10-08）

**实时传输链路**（`cpal::Stream` 是 `!Send`，因此宿主必须独占一条线程）：

- `crates/engine/src/realtime.rs`：新增 `SharedTransport`（跨线程共享位置/播放态）、
  `AudioHost::start_with`（复用共享状态重建宿主）、`AudioHost::send`、
  `default_output_sample_rate()`（不打开流即可探测设备采样率）。
- `src-tauri/src/audio.rs`：**专用音频线程**持有 `AudioHost`，经 `mpsc` 接收
  `Load`（装载/重载工程）与 `Command`（传输与混音）；控制侧通过原子量读取状态。
- 传输命令：`transport_play` / `transport_pause` / `transport_stop` / `transport_seek` /
  `transport_status`；位置经 `engine://transport` 以约 30 Hz 推送（**仅状态变化时**推送，
  暂停后完全静默）。
- 编辑 → 声音的两条路径（由事件类型决定，见 `Event::is_mix_only`）：
  - **混音类事件**（增益/声像/静音/独奏/总输出）→ 只推参数给音频线程，**不重建播放器**，
    因此拖动推子不会造成音频中断；
  - **其余事件** → 重建定帧事件表，并保留播放位置与播放状态。
- 前端 `stores/transport.ts`：统一 `realtime` / `preview` 两种模式；设备不可用时自动降级为
  「离线渲染 → `<audio>`」，位置由音频元素推进，界面无需分支。

**各视图的真实操作**（引擎命令面同步扩展，全部可撤销、可被 AI 复用）：

| 视图 | 可执行的操作 | 对应命令 |
| --- | --- | --- |
| 底部传输条 | 播放/暂停、停止、回到起点、位置读数、点击标尺定位；**播到结尾后再按播放会从头重播** | `transport_*`（重播策略在 `Player::apply_command`） |
| **共享时间轴** | 编排与编辑**同一条时间轴**：同一缩放、同一横向滚动位置、同一套由**拍号**推导的网格；标尺是同一个组件（`TimelineRuler.vue`）。点击/拖动标尺定位播放头（正在播放时从新位置继续）。`Ctrl/Cmd + 滚轮`缩放（锚点在指针下，两视图同步） | — |
| 编排 | **在空白处拖动画出等长片段**（框选模式下拖动 = 框选）、拖动移动（多选整组移动、**单个片段可跨轨道拖动**）、拖右边缘改长度、左键点选 / Ctrl 加选、**单击片段顺手打开右侧检查器**（方便立刻换音色）、**右键点击删除**、**右键拖动框选批量删除**、**双击进入钢琴卷帘** | `add_clip` / `move_clip` / `move_clips`（带 `to_track`）/ `resize_clip` / `remove_clip` / `remove_clips` |
| 编辑（钢琴卷帘） | **顶部有与编排同一条的标尺**；**在空白处拖动画出等长音符**、拖动改音高与起点（多选整组）、拖右边缘改时长、左键点选 / Ctrl 加选、右键点击删除、右键拖动框选批量删除；**左侧琴键列画成真正的键盘（白键 / 黑键 + C 音名）**，与可放音符的网格明显区分；进入片段时纵向自动对准音符；轨道没有片段时给出「新建 1 小节片段」入口 | `add_note` / `set_note` / `set_notes` / `remove_note` / `remove_notes` / `add_clip` |
| **控制器泳道**（卷帘下方） | 每个音符一根直角条：**横位置与宽度与上方音符一一对应**（绝对时间轴，片段有偏移也对齐），高度表示所选属性；属性是**音量 / 力度**（所有音源通用；不同插件将来会带来各自属性），上下拖动改值（多选整组、松手提交一次）、框选、右键删除；与卷帘**横向滚动同步**，且与卷帘之间的**分隔线可拖拽**（高度持久化） | `set_notes` / `remove_notes` |
| 混音台 | 推子改增益（相对拖动）、声像、静音、独奏、主输出增益 | `set_gain` / `set_pan` / `set_mute` / `set_solo` / `set_master_gain` |
| 检查器 | 轨道重命名、**音色用分类下拉菜单切换**（附「浏览全部音色…」模态框：搜索 / 分类筛选 / 排序）、乐器参数、混音参数 | `rename_track` / `set_track_instrument` / `set_param` |
| 音轨栏 | 选中、静音、独奏、删除；**在钢琴卷帘界面点它会跳到该轨道「同一时刻」的片段，没有片段就回编排** | 同上 |
| 插件面板 | 按分类列出插件，把插件加载到当前选中轨道 | `set_track_instrument` |

**鼠标模式（顶栏切换 / 快捷键 `1` 绘制、`2` 框选）**：解决「左键拖动空白 = 创建」与
「左键拖动 = 框选」的手势冲突。模式决定左键的默认含义，`Shift` / `Alt` 随时可临时切到
框选；右键拖动始终是框选删除。

**网格由拍号推导**（`stores/timeline.ts`）：`meterBeat = 4 / 分母`（拍号里那一「拍」有多长），
`barBeats = 分子 × meterBeat`（一小节多长：4/4 → 4、3/4 → 3、6/8 → 3、2/2 → 4），
`snap = meterBeat / 4`（吸附网格：4/4 → 十六分、6/8 → 三十二分）。**切换拍号会同时改变
编排与编辑两个视图的网格格子与吸附精度**。细分网格密于 10px 时自动不画，避免糊成一片密纹。

**新工程的默认分轨**（`STARTER_TRACKS`，引擎常量，数组顺序即轨道顺序）：

| # | 轨道 | 角色 | 默认音色（内置合成器预设） |
| --- | --- | --- | --- |
| 1 | 鼓组 | 节奏骨架 | 拨弦 Pluck（最短促的包络） |
| 2 | 低音 | 根音与低频 | 低音 Bass |
| 3 | 和弦 | 和声铺底 | 铺底 Pad |
| 4 | 主旋律 | 主线条 | 主音 Lead |
| 5 | 铺底 | 织体 / 氛围 | 铺底 Pad |

选的是流行与电子编曲里最通用的五个**功能位**（节奏 → 低频 → 和声 → 主旋律 → 色彩），
避免「节奏」与「鼓组」这类职责重叠的命名。鼓组专用采样器已接入为 `builtin.drums`（MuldjordKit lite），默认模板直接改 `plugin_id`。
`Engine::new_project`（应用启动时的初始状态）不写历史，用户主动点「新建工程」走命令、可撤销。

**内置音色**：`builtin.synth` 是合成器本体，另有 5 个**预设音色**（`builtin.synth.lead` /
`.bass` / `.drums` / `.strings` / `.pad`）——同一套 DSP 配不同默认包络，各自是独立插件 id
（`crates/engine/src/registry.rs` 的 `BUILTIN_PRESETS`），所以音色选择器里就是一条条真音色。
另有 18 件采样乐器：钢琴 / 尼龙吉他 / 清音电吉他 / 指弹贝斯 / 原声鼓组，以及
弦乐群奏、慢弦乐、小提琴、中提琴、大提琴、低音提琴、弦乐拨奏、小号、圆号齐奏、
合成弦乐 1/2、合成铜管 1/2（`builtin.*`），资产在 `crates/instruments/assets` 内嵌（SF3/SF2，约 13.5 MB）。

**音符的两个「响度类」属性**：`velocity` **力度**（演奏强度，插件各自解释；内置合成器折算成振幅）
与 `volume` **音量**（单音电平微调，不改音色，默认 127 = 不衰减）。插件 ABI 的 `NoteOn` 只有
一个力度字段，所以两者在**排程时**合成（`Note::midi_velocity`）；旧工程缺 `volume` 字段时按
127 读入，听感完全不变。

**批量操作与撤销**：框选删除、整组拖动、跨轨道拖动、泳道批量改值都各自只产生**一条**
撤销记录（引擎侧 `RemoveNotes` / `SetNotes` / `RemoveClips` / `MoveClips` 四条批量命令；
`RemoveNotes` 内部倒序删除并去重、越界保护；`ClipsMoved` 先校验目标轨道存在，非法目标整条跳过）。

**已修复的隐患**：`Md3Slider` 的 `commit` 事件此前报的是 `props.modelValue`——在父组件
用「草稿 + 只监听 commit」的模式下那是**尚未更新的旧值**，会让拖完松手等于没改。现在
`commit` 报的是本次交互实际选中的值（混音台声像、检查器增益/声像、音符力度都受益）。

**主题在挂载前应用**（`main.ts` 调一次 `useTheme()`）：此前主题只在设置页挂载时才写入
`<html>`，选了深色后重新打开应用会退回浅色。

**实时安全**：`Player` 新增实时混音（`TransportCommand::SetTrackMix` / `SetMasterGain`），
`refresh_mix` 只做算术与字段写入；`tests/realtime_safety.rs` 已把混音命令纳入零分配断言。
总输出增益现在真正生效（`ScheduledEvents` 携带 `master`，离线渲染与实时播放一致）。

**已知取舍**：结构性编辑会重建播放器（重新打开设备流），播放中做此类编辑会有极短的
音频间断；混音参数不受影响。参数改动同样走重建，因此拖动参数滑块只在松手时生效一次。


**已具备、可直接复用的能力**：确定性离线渲染（同一工程逐位一致）、cpal 实时宿主（RT-safe，回调内不分配内存）、**专用音频线程 + 共享传输状态**、**实时混音（推子/声像/静音/独奏/总输出不重建事件表）**、无锁 SPSC 传输命令队列、事件溯源 + 撤销、统一插件 ABI（`Plugin` trait）、内置合成器、工程 JSON 往返、Tauri 命令层 + 引擎事件广播、Vue 外壳（顶栏 / 传输条 / 轨道列表 / 标签页面板 / 主题 / 选中态）。

## 2. 修正记录（设计文档 / 分计划 与现实及事实的偏差）

| 项 | 原述 | 实际 / 修正 |
| --- | --- | --- |
| 前端 | React 18 + `@m3e/react` | **Vue 3 + Pinia + Vue Router + Tailwind**（Material You token 适配自 Exameow）；自提交 `94d2797` 起重建 |
| 内置合成器 DSP | `fundsp` | **手写正弦振荡器 + 线性 ADSR**，只依赖 `plugin-api` |
| 开源许可 | 「Apache + GPL 双许可」（外部建议） | **纯 Apache-2.0**（`Cargo.toml` `license = "Apache-2.0"`、`NOTICE`），无 GPL |
| VST3 宿主选型 | 评估 `rack`、`symdaw/audio-plugin-host` | 首选 **`truce-rack`**（MIT OR Apache-2.0、纯 Rust binding，CLAP+VST3 默认、AU/LV2 可选）；`rack` 是其前身（含 C++ glue） |
| VST3 宿主实装 | （承上）预期用 truce-rack | **实际改用 `vst3-host` + `clack-host`**（见 `crates/host-vst3/Cargo.toml` 注释）：truce-rack 的 VST3 后端托管不了「无音频输入总线的音源」，其 CLAP 后端加载 KR-106 会段错误。两者同为宽松许可，不破坏 Apache-2.0 分发。 |
| 低延迟（Windows） | （外部建议）启用 cpal 的 ASIO 后端 | **不启用 ASIO**：ASIO SDK 为 GPLv3 / 专有，会破坏纯 Apache-2.0 分发。走 WASAPI |
| AI 听感引擎 | （外部建议）Dhwani | **不用**：Dhwani 是 **GPL-3.0-only**，链接即传染。改用 `ebur128` / `rustfft` |
| 音频解码 | `symphonia` / `rubato`（拟定） | **symphonia 已采用**（wav/flac/mp3/ogg/aac/m4a）。它是 **MPL-2.0**，原先不在允许清单里 → 2026-10-09 把 MPL-2.0 加入 §3（**仅限未修改依赖**，见 `NOTICE`/`deny.toml`）。`rubato`（保音高变速）暂未引入。 |
| 音频片段时间语义 | 原为「片段拍长是唯一真相，素材铺满跨度 → 会变调」 | **2026-10-13 解耦**：片段拍长只决定**方框**；素材按 `rate` **原速**播放——**改 BPM 不改音频**、改速度时方框自动重算跟住素材；**变速 / 升降调**是片段自己的参数（`AudioSource.rate`/`pitch_semitones`，引擎用 **WSOLA 时值伸缩** 保音高）；拖左右边缘 = **裁剪**（改读写窗口，不改速度/音高）。见 `specs/2026-10-09-audio-clip-design.md` §3。 |
| 声像律 | （未指定，原实现为等功率） | **改为线性 balance**（居中 = 单位增益，向一侧偏则衰减另一侧）。原因：引擎内部恒为立体声，等功率律让居中信号凭空 −3 dB，轨道与 master 各套一次 = 整链 **−6 dB**（导入满幅素材会明显变轻）。代价：硬偏不再有 +3 dB 补偿。 |
| 暂停语义 | （原实现：暂停后插件继续跑） | **暂停 = 静音**：`Pause`/`Stop` 重置乐器与效果链（all-notes-off）。原实现只置 `playing=false`，长音的 NoteOff 再也送不到 → 插件一直挂着该音（「暂停了还在响」）。插件实例不销毁，界面键盘与插件自带界面预览仍可用。 |
| 合成器扩展路线 | （用户提出多种合成法） | **一个合成器 + 更多原子，而不是每类一个引擎**：补齐噪声 / 位深压碎 / 失谐 / FM / 音高包络 / 滤波类型六种原子，8 类音色 = 8 组参数配方。波表/加法/物理建模/颗粒在 v1 是**近似**（见 `specs/2026-10-09-synth-overhaul-design.md` §2），真实现留后续。 |
| 预设与效果链 | （未指定） | **预设 = 整套音色**：`FactoryPreset.effects` 可带效果链，切预设**替换**该轨整条效果链（与「切预设会丢掉旋钮微调」同一个契约）。实现上复用引擎既有的 `execute_all` 批量事务，因此**没有新增 IPC 契约**，一次事务 = 一个撤销步骤。 |
| 默认模板的预设 | （未指定） | **刻意干声**：`STARTER_TRACKS` 的 plugin_id 必须是引擎注册过的 id（否则新建工程就是「缺失插件」占位），而注册表补丁带不了效果链。所以 8 类里 5 条第 1 位是干声版本，湿版本放第 2..4 条。 |
| 插件宿主（外部建议首选） | UAPMD | 仅作 **MCP / 能力参考**：UAPMD 是 **C++/CMake** 工程，接入需 FFI，与引擎 `#![forbid(unsafe_code)]` 相悖 |

## 3. 许可证护栏（强制）

- 本项目许可：**Apache-2.0-only**。
- 允许的依赖许可：MIT / Apache-2.0 / BSD-2-Clause / BSD-3-Clause / ISC / CC0 / Zlib / Unlicense
  / **MPL-2.0（仅限「未修改的依赖」，2026-10-09 加入）**。
- **禁止**：GPL / AGPL / ASIO 专有件 / SSPL；LGPL 需单独评估（动态链接）。
- MPL-2.0 是**文件级**弱著佐权：把**未修改**的 MPL 库当依赖链接进 Apache-2.0 程序不会传染本仓库
  代码，义务只有「保留其许可声明」+「若修改其源码则须公开该文件的修改」。首个（也是当前唯一）
  用例是 `symphonia`（音频解码，见 `crates/asset` 与 `NOTICE`）。
- 落地手段：仓库根已落 `deny.toml`（`cargo deny check licenses`，但本机/CI 尚未执行）；CI 接入待办。
- `unsafe` 例外：仅 `crates/host-vst3` 允许（Plan 1 约束已声明）；其余 crate 保持 `#![forbid(unsafe_code)]`。

## 4. 技术栈（修正后）

| 层 | 选择 | 状态 |
| --- | --- | --- |
| 应用外壳 | Tauri v2 | ✅ |
| 前端 | Vue 3 + Pinia + Vue Router + Tailwind（Material You） | ✅ |
| 音频 IO | `cpal`（Windows 走 WASAPI，**不用 ASIO**） | ✅ |
| 实时/离线渲染 | 自研 `Player` + `ScheduledEvents`（离线与实时逐位一致） | ✅ |
| 内置合成器 DSP | 手写正弦 + 线性 ADSR | ✅ |
| WAV 读写 | `hound` | ✅ |
| 插件 ABI | 自研 `Plugin` trait（就地平面缓冲） | ✅ |
| VST3 / CLAP 宿主 | **`vst3-host`** + **`clack-host`**（见 §2） | ✅ |
| MIDI 输入 | `midir` | ⬜ 待建 |
| 音频解码/重采样 | `symphonia` / `rubato` | ⬜ 待建 |
| 响度/频谱分析 | `ebur128` / `rustfft` | ✅ |
| 插件管理器 | 开放注册表静态 JSON（前端直接读）+ Rust 侧下载/校验/解压；**不用** `@open-audio-stack/core`（Node 库，webview 跑不了） | ✅ E1 完成 |
| 母带处理 | **纯 DSP**（`crates/mastering`：`ebur128` 复用分析层 + 自研真峰值限幅）；**不引模型** | ✅ 完成 |
| 网络 / 代理 | `ureq`（`native-tls` + `gzip` + `socks-proxy`）；三种代理模式（直连 / **跟随系统** / 手动），系统代理走 `scutil --proxy`（macOS）与注册表（Windows） | ✅ 完成 |

## 5. 剩余路线

### Stage A — v1 收尾（P0）

- **A1 · `crates/ipc`**：✅ 完成（2026-10-09）。`crates/ipc`（`ai-daw-ipc`）只依赖 `serde`，
  持有 `model/*` / `command.rs` / `event.rs`（含 `Actor`、`Event::actor()`、`Event::is_mix_only()` 与全部
  `normalize_*`）；`ai-daw-engine` 用 `pub use` 薄壳再导出，**既有路径与行为零变化**
  （测试通过数守恒：engine 106→90、新增 ipc 17、`--features realtime` 123→107）。
  设计/计划：`specs/2026-10-09-ipc-contract-design.md`、`plans/2026-10-09-ipc-contract.md`。
  顺带落 `deny.toml` 并把 MPL-2.0（仅限未修改依赖）加入 §3 允许清单。
- **A2 · Plan 4b：原生实时传输接进 UI** ✅ 完成（2026-10-08）
  - 后端：`transport_play` / `transport_pause` / `transport_stop` / `transport_seek` / `transport_status` + `engine://transport` 位置事件（位置从音频线程的 `SharedTransport` 读取）。
  - 线程：`AudioHost` 移入专用音频线程（`src-tauri/src/audio.rs`），用 `mpsc` 与命令层通信。
  - 前端：`stores/transport.ts` 走实时命令；离线渲染仅保留给导出与「设备不可用时的回退试听」。
  - 验收：播放/暂停/停止/定位即时响应，无渲染等待 ✅（浏览器预览为 mock 模拟，真实设备行为需在桌面端验证）。

### Stage B — 插件托管（P0，核心差异化）— ✅ 基本完成（2026-10-09 核对）

- **B1 · `crates/host-vst3`**：✅ 已实现。VST3 后端用 `vst3-host`（能托管无输入总线的音源）、CLAP 后端用 `clack-host`（truce-rack 的 VST3/CLAP 均不可用，见 §2 修正记录）；两者都适配到既有 `Plugin` trait（未替换 ABI）。含扫描（磁盘缓存）、加载、参数、状态、原生窗口（`editor.rs`）、主线程派发（`dispatch.rs`）。
- **B2 · 注册表扩展**：✅ 已实现（`src-tauri/src/plugins.rs`）。`build_registry` 装配外部插件工厂；缺失插件给 `missing_plugin` 静音占位；工程引用但本机没有的插件也会 `register_missing`。
- **B3 · 前端**：✅ 已实现。插件清单 / 通用参数面板 / 效果链 / 原生界面窗口（`InspectorPanel`、`PluginsPane`、`data/plugins.ts`、`usePluginInterface`）。
- **B4 · 测试**：🟡 部分。`crates/host-vst3/tests/kr106.rs` + `src-tauri/src/plugins.rs::renders_a_track_hosting_the_clap_kr106`（真 KR-106，未安装则跳过）。**尚未接入 VST3 官方 Validator**，也未覆盖更多免费插件。
- 验收：能扫描本机 VST3/CLAP、挂到轨道、存进工程 JSON、重开还原 ✅。

### Stage C — 音频能力（P1）

- **C1 · Plan 6**：`AudioClip` 模型 + 采样器 + 波形显示。✅ **完成（2026-10-09）**：
  `crates/asset`（symphonia 解码 wav/flac/mp3/ogg/aac/m4a + `peaks`）、`Clip.audio`/`AudioSource`、
  `AddAudioClip`/`SetClipAudio` 与对应事件、`SampleProvider` + `ScheduledAudioClip` + 播放/定位/暂停语义、
  宿主 `AssetCache`（素材在命令线程解码，绝不进音频回调）、`import_audio`/`audio_waveform` 命令 + MCP 工具、
  前端波形（`ClipWaveform.vue`）与「导入音频…」入口。
  测试：`audio_clip.rs` 7 条量化断言 + 播放器/解码单测，`--features realtime` 的零分配门禁仍通过；
  **真机端到端**（MCP 导入 1 kHz WAV → 分析渲染结果）音高与单位增益均与理论值一致。
  设计与计划：`specs/2026-10-09-audio-clip-design.md`、`plans/2026-10-09-audio-clip.md`。
  语义决策：**片段拍长只决定方框，素材原速播放（与工程 BPM 解耦）**；变速 / 升降调是片段参数，
  拖左右边缘 = 裁剪（2026-10-13 从「铺满跨度会变调」改为解耦模型）。
- **C2 · Plan 5**：录音（cpal 输入流 → 写入 `AudioClip`）。✅ **完成（2026-10-10）**：
  引擎新增 `crates/engine/src/recorder.rs`（输入设备枚举 + cpal 输入流 + 无锁 `rtrb` 采集环 +
  消费线程 → `DecodedAudio`），`AudioHost` 增加输入流生命周期（arm/disarm/start/stop）；
  宿主命令 `list_input_devices` / `set_input_device` / `input_level` / `record_start` / `record_stop`
  （录音落 `app_data_dir()/recordings/`，写 WAV → `AssetCache::insert` → `AddAudioClip`，一个撤销步骤）；
  前端底部条录音键 + 输入设备下拉 + 输入电平表。**零 IPC 契约变更**（复用 C1 的 `AudioSource`/`AddAudioClip`）。
  测试：`recorder` 单测 9 条 + 输入回调**零分配门禁**（`tests/input_capture_safety.rs`）；
  真机验证：4 个输入设备，采 300 ms → 13312 帧 @ 44.1 kHz。
  设计与计划：`specs/2026-10-10-recording-design.md`、`plans/2026-10-10-recording.md`。
  语义决策：**保留输入设备原生采样率**（播放侧复用 C1 的「铺满」换算）；**不监听、不补偿延迟**。
- **C3 · 混音完善**：`AutomationLane`、send / bus。✅ **完成（2026-10-10）**：
  模型新增 `Track.sends`（推子后 aux send）与 `Track.automation`（`gain_db` / `pan` 折线）；
  调度表把发送**解析成轨道下标**并算出**拓扑处理顺序**（`routing_order`，有环不挂死）；
  `Player` 逐轨处理：总线输入并入 → 乐器/音频 → 效果链 → 推子 → master **且**按发送复制进目标总线；
  自动化按拍**逐块求值**并**覆盖推子值**（读取模式，`is_mix_only` 不重建播放器）。
  命令 `set_sends` / `set_automation` + 同名 MCP 工具。
  **编辑器 UI ✅**：检查器发送区（`InspectorPanel.vue`）+ 编排视图底部的自动化曲线编辑器
  （`components/workspace/AutomationLane.vue`：参数下拉 `gain_db` / `pan`、点击加点、拖动改拍与值、
  右键删点、清空；与编排网格共用同一条时间轴，画布两端补抽头以匹配引擎的「夹住」语义；
  顶栏开关在编排与钢琴卷帘两个视图都有、各自控制本视图的底部窗格且默认打开；可见性/高度持久化、
  高度可拖拽）。**自动化目标已扩展到 `inst:` / `fx<i>:`：乐器与效果链上任意参数都能画曲线**
  （键的命名空间见 `specs/2026-10-10-plugin-param-automation-design.md` §4），
  见 `specs+plans/2026-10-10-plugin-param-automation*`；卷帘下方的控制器泳道随之改成**参数车道**
  （`力度` 仍是逐音符直角条，其余目标是与编排视图**同一份** `Track.automation` 的曲线），
  下拉里删掉与力度重复的「音量」（`Note.volume` 字段保留，旧工程听感不变）。
  测试：`cargo test -p ai-daw-engine --features realtime` → 141 通过；`realtime_safety.rs` 门禁已覆盖发送 + 自动化
  （含 `SetAutomation` 的 `apply_command` 零分配断言）。
  设计与计划：`specs/2026-10-10-mixing-sends-buses-automation-design.md`、`plans/2026-10-10-mixing-sends-buses-automation.md`。
- 可直接用的开源零件与参考项目（许可已核）：见 `research/2026-10-10-open-source-survey.md`。

### Stage D — AI 层（P1，项目定位所在）

- **D1 · `crates/analysis`**：✅ 完成。渲染 → 响度（EBU R128：LUFS / LRA / 采样峰值 / 真峰值）+ RMS + 平均幅度谱（Hann 窗 + 50% 交叠 FFT）。对上层是 `analyze_project` IPC 命令；这是 AI 的「听感」闭环。
- **D2 · `src-tauri` 内嵌 MCP 服务器**：✅ 完成。`src-tauri/src/mcp.rs`（`rmcp` 3，Streamable HTTP，绑 `127.0.0.1` 临时端口，启动时把 `http://127.0.0.1:<port>/mcp` 打到 stderr）。**33 个工具**：`get_project` / `get_transport` / `list_plugins` / `transport_play` / `transport_pause` / `transport_stop` / `transport_seek` / `transport_set_loop` / `set_tempo` / `set_time_signature` / `add_track` / `remove_track` / `rename_track` / `set_track_instrument` / `set_param` / `add_clip` / `remove_clip` / `add_note` / `set_gain` / `set_pan` / `set_mute` / `set_solo` / `set_master_gain` / `set_sends` / `set_automation` / `aidaw_help` / `undo` / `redo` / `analyze_project` / `export_wav` / `import_audio` / `audio_waveform` / `apply_preset`。写操作全部走 `commands::run_agent`（`actor = Agent("mcp")`），与界面**同一条**管线：撤销历史、`engine://event` 事件流、音频同步、结构变更关插件窗口。
  - **配置**：设置页「AI / MCP」一节可开关、改端口（0 = 临时端口），即时启停；配置落在应用配置目录的 `mcp.json`。
  - 安全：只绑回环；rmcp 默认只放行回环 `Host`（防 DNS rebinding）。**当前不鉴权**——同机任意进程都能连，后续可加 token。
- **D3 · skills 文档包 + 本地 HTTP/WS**（设计文档 §9）：🟡 **skills 文档包已完成（= F3）**：
  仓库根 `skills/aidaw/`（`SKILL.md` + `references/` 14 个能力域，含乐理 / 节奏鼓谱 / 曲风 / 编曲流程），并内嵌进 MCP 的 `aidaw_help(topic)` 工具（渐进式披露）。
  **本地 HTTP/WS 未做**（MCP 已提供 HTTP 入口）。
- **D4 · 前端 AI 可视化**：✅ 完成。`stores/engineEvents.ts` 解析事件里的 `Actor`，把**代理**发起的变更单独记账（代理名、事件类型、涉及轨道）；底栏胶囊只在代理动手时点亮并显示代理名（tooltip 给出最近改动处数与轨道），音轨栏对被代理改动的行短暂高亮（900ms 胶囊窗 / 1600ms 轨道窗）。人类自己的编辑不再误点亮 AI 指示。浏览器预览的 mock 会轮播代理事件，便于无后端演示。

### Stage E — 生态与隔离（P2）

> **状态（2026-10-10）**：**E1 完成**（改名「插件管理器」，安装 / 导入 / 更新 / 删除四合一）；
> **E2 仍搁置**（「插件上量后再做」）。

- **E1 · 插件管理器**：✅ **完成（2026-10-10）**。浏览开放注册表 → 下载 → **校验 sha256** →
  解压归档（zip / tar.gz）→ 装进**托管库**（应用数据目录 `plugins/{vst3,clap}`，宿主额外扫描）；
  另有**导入本地**、**更新**（同 slug 先删旧再装）、**删除**（只删本管理器装的）。
  注册表是一份**静态 JSON**，前端直接读——**没有**用 `@open-audio-stack/core`（那是 Node 库，
  Tauri 的 webview 跑不起来）。**安装器类型（dmg/pkg/exe/msi/deb/rpm/AppImage）绝不自动执行**，
  只「打开下载页」；这是安全边界。
  实现：`src-tauri/src/manager.rs` + `crates/host-vst3/src/scan.rs`（`scan_catalog_with`）；
  面板 `ManagerPane.vue`（右侧栏 pane `manager` + 设置页入口）。
  **网络与镜像（国内可用性）**：设置页可配**注册表镜像**（多源回退，官方源兜底，全失败退回旧缓存）与
  **下载代理前缀**（只重写 GitHub 地址，sha256 兜底）；**不提供任何默认镜像/代理**，配置落 `store.json`。
  设计与计划：`specs/2026-10-10-plugin-manager-design.md`、`plans/2026-10-10-plugin-manager.md`。
- **E2 · 崩溃隔离**：⬜ **暂停**。第三方插件上量后，再把插件宿主拆成 helper 进程
  （**此时 sidecar 才有意义；音频引擎本身不必拆进程**）。注：`vst3-host` 0.9 已自带
  `with_process_isolation(true)` + `vst3-host-helper` 的进程隔离开关，将来做 E2 时值得先评估它。

### Stage F — Skills 层 + `aidaw` CLI（让任意 AI Harness 可挂载）

**定位**：把已存在的 **33 个 MCP 工具**做成 AI harness 可**渐进式**接入的统一接口：
一个 `aidaw` CLI（MCP 薄客户端）负责**执行**，`skills/` 文档包负责**按需披露**。

> **状态（2026-10-10）**：**F1 / F2 / F3 / F4 ✅ 完成**；**F5 暂缓**。**改向（2026-10-10）**：
> 原 F1（无 Tauri 运行时）+ F2（CLI 镜像全部命令）方案**改为薄客户端** —— 见下；原方案降级为 **F5（搁置）**。

- **设计基线（2026-10-10 确定）**：`aidaw` **是 MCP 客户端，不是引擎**。
  - 软件开着 → `aidaw` 连它的 MCP HTTP 端点（仅回环）执行；
  - 软件没开 → `aidaw` 先把它**启动**起来，再执行；
  - 因此 `aidaw` **不链接 Tauri、不内置引擎**，也**不需要**「无 Tauri 运行时」。
  - **加一个 MCP 工具 = 自动多一条可调用的 CLI 命令**（按名转发，无需逐工具写代码）——
    这是选「CLI 作 MCP 薄客户端」而非「CLI 镜像命令面」的核心理由。
- **F1 · `aidaw` CLI（MCP 薄客户端）** ✅ **完成（2026-10-10）**
  - 根 workspace 新增 `crates/aidaw`（`[[bin]] name = "aidaw"`）；`clap`（MIT/Apache，符合 `deny.toml`）。
  - 子命令：
    - `aidaw tools` —— 转发 MCP `tools/list`（列出当前所有工具，输出「工具名 — 一行描述」）。
    - `aidaw call <tool> [--json '{…}']` —— 转发 MCP `tools/call`；JSON 参数直通；工具报错 → 非零退出。
    - `aidaw status` —— 软件在不在、MCP 地址是否可达（在 → 0，不在 → 1）。
    - `aidaw start` —— 没开就启动（macOS `open -a "AI DAW"`，可用 `AIDAW_APP_PATH` 覆盖；
      其余平台用该变量指定可执行文件），轮询等到 MCP 就绪再返回。
  - 客户端实现：**选了 `ureq`**（`default-features = false`，只连回环、不需要 TLS；它已在 workspace 里）——
    不引入 rmcp 客户端那棵大树。协议细节（`initialize` 握手、`Mcp-Session-Id`、SSE 帧解析）在
    `client.rs` 自处理：SSE 是长连接，故**读到与请求 id 匹配的那条消息就停**，不读到底。
  - 能力面 = MCP 工具面：只在 Tauri 命令层、未暴露成 MCP 工具的动作（如「打开插件窗口」）
    CLI 暂不可用——解决办法就是「先加一个 MCP tool」，与「加工具顺便加命令」一致。
- **F2 · 发现与启动（F1 的前置）** ✅ **完成（2026-10-10）**
  - **为什么需要**：MCP 默认端口 `50562`，但被占用时 `mcp.rs` 会**退回临时端口**，那时 CLI 找不到。
  - `src-tauri/src/mcp.rs`：`start()` 拿到 `addr` 后把**实际**地址写入应用配置目录
    （`com.aidaw.app`）的运行时文件 `mcp-url`；`stop()` 删除；未启用时 `autostart()` 也清一次。
  - CLI 读取顺序：`AIDAW_MCP_URL`（覆盖）→ `mcp-url` → 默认端口 `50562` 兜底；`start` 轮询时每轮重读。
  - **传输只用 HTTP**（不用 stdio）：GUI 进程本就在跑 HTTP MCP；stdio 只在「CLI 自己当 MCP 服务器」时才需要，
    而薄客户端模式下 CLI 是**客户端**，不需要。
- **F3 · skills 文档包（渐进式披露）** ✅ **完成（2026-10-10）**
  - 仓库根 `skills/aidaw/`：**一个标准 Agent Skill 目录**——`SKILL.md`（入口：frontmatter
    `name: aidaw` / `description` / `license`，正文是索引 + 公共约定 + CLI 入口）+ `references/`
    下 14 个能力域（`project` / `track` / `clip` / `note` / `composition` / `mix` / `audio` /
    `transport` / `analysis` / `plugin` / `theory` / `rhythm` / `genres` / `workflow`），
    每个域只描述它那一小组工具与组合用法。
  - **gateway 工具**：MCP 的 `aidaw_help(topic)` —— 留空给索引，按域取细则；文档用 `include_str!` 编译期内嵌。
  - **CLI 入口 ✅（随 F1 补齐）**：`skills/aidaw/SKILL.md` 已有「CLI 入口」一节——`aidaw status` → 没开
    `aidaw start` → `aidaw call …`；`aidaw_call(op, args)` 由 `aidaw call` 落地。
- **F4 · 分发与打包** ✅ **完成（2026-10-10）**：`aidaw` 随应用分发，并被接进 PATH（Ollama 式）。
  - 打包：`bundle.externalBin: ["binaries/aidaw"]`（sidecar 落到主程序旁边）+ `tools/build-aidaw.sh`
    （编译 `crates/aidaw` → `src-tauri/binaries/aidaw-<triple>`）+ `beforeBuildCommand` 串上 `build:aidaw`。
  - 上 PATH：`src-tauri/src/cli.rs`。macOS = 软链 `/usr/local/bin/aidaw` → 包内二进制（一次管理员授权）；
    Windows = 安装目录追加进**用户** PATH；Linux deb/rpm = 本来就在 `/usr/bin`，无需动作。
  - **入口是设置页里的一个显式开关**（开 = 挂载 / 关 = 卸载）——**启动时不做任何动作**，
    不碰 PATH、不弹密码框；不挂也能用，用包内二进制的**全路径**调即可。
    `skills/aidaw/SKILL.md` 里已写明：AI 拿不到短命令就用全路径，可以顺带建议用户去开开关（可选）。
    没有配置文件（真相在文件系统 / 注册表），取消授权只回弹开关、不记「拒绝过」。
  - **发现文件 `cli-path`**：启动时把包内 `aidaw` 的**绝对路径**写进应用配置目录（与 `mcp-url` 并列），
    每次启动重写。外部 AI harness 按 bundle id（`com.aidaw.app`）推导出配置目录、读它就能拿到全路径
    ——**不用搜索**（不依赖 Spotlight / 注册表 / PATH），也不要求用户开挂载开关；顺带解决「没开挂载时
    AI 只能用猜的」。发现顺序已写进 `skills/aidaw/SKILL.md`。
  - 构建链：`bundle.externalBin` 会让 `tauri dev` 在缺 sidecar 时**直接报错**，所以
    `beforeDevCommand` 也串了 `build:aidaw`（全新 clone 才能一把跑起来）。
  - `skills/` 随应用分发（`bundle.resources`）**仍未做**——目前只在仓库里、由 MCP 的 `aidaw_help` 内嵌提供。
  - 设计 / 计划：`specs/2026-10-10-cli-path-install-design.md`、`plans/2026-10-10-cli-path-install.md`。
  - **开发时**不改 PATH：`cargo run -p aidaw -- status`，或 `cargo install --path crates/aidaw`（会装到
    `~/.cargo/bin`，是**快照**，改了源码要重装）。`pnpm tauri dev` 下自动安装是 no-op——按「路径含
    `.app/Contents/MacOS/` ⇒ 真 App；否则看最后一个 `target` 之后是不是 `debug` / `release`」判定
    （Tauri 也会把 sidecar 复制进 `target/debug`，只靠「旁边有没有 sidecar」会误判、白弹一次密码框；
    而 `.app` 必须先认，否则本机 `tauri build` 的包也会被误判成开发构建）。
- **F5 · 无 Tauri 运行时 / headless 引擎（原 F1）** 🅿️ **搁置**
  - 原方案：抽 `crates/runtime`（`AppState` + `AudioEngine` + `AssetCache` + `build_registry` + `run/run_agent`）、
    副作用抽成 `Effect` trait、33 个 `#[tool]` 移入 `crates/mcp` 并对 `Arc<AppState>` 泛型化、
    `src-tauri` 退化为薄适配层（依据：`state.rs` / `audio.rs` / `assets.rs` / `plugins.rs` / `presets.rs` / `session.rs`
    本就不 import tauri；`host-vst3` 无 dispatcher 时默认 inline 派发）。
  - **搁置理由**：它解决的是「**不开 GUI 也能跑引擎**」（CI / 服务器 / 纯 headless 渲染）。
    当前诉求（AI 遥控**正在运行的**软件 + 没开就启动）**不需要**它，而它是本仓库风险最高的一块重构。
  - **重启条件**：确有 headless / CI 渲染需求时再做；迁移手法沿用 `crates/ipc`（搬家 + `pub use` 薄壳、零行为变化）。

### Stage G — 本地 AI 能力（Local AI，内置模型）

> **状态（2026-10-10）**：**G0 / G1 / G2 已用真模型落地**（`crates/local-ai` 的 `models/` 层，
> `ort` 2.0-rc.13，权重见 `tools/fetch-models.sh`）：
> - **G1 音频转 MIDI = Basic Pitch**（官方 `nmp.onnx`，Apache-2.0，230 KB）：谐波 CQT 前端在
>   图里，喂原始波形；窗口化（43844 样本 / 步进 36164 / 每窗掐首尾 15 帧）+ 官方
>   `note_creation.py` 的解码（帧差补 onset、邻频能量抑制、melodia 后处理）逐行移植。
> - **G2 降噪 = LocalVQE v1.2**（Apache-2.0，1.29 M 参数）：上游只出 `.pt`/GGUF，ONNX 由
>   `tools/export-localvqe-onnx.py` 从官方权重导出（sha256 与上游 manifest 一致，自检与
>   PyTorch 参考逐样本 ≤5.3e-6）。固定 2 秒分块 + 50% Hann 交叠相加（官方整段前向的中间张量
>   随时长线性增长，几分钟素材要吃几百 MB）。
> - 采样率：模型要 22 050 / 16 000 Hz，工程素材多是 44.1/48 k，用带限 sinc 重采样
>   （`resample.rs`，降采样带抗镜像）。
> - **两个小模型随安装包分发**（Basic Pitch / LocalVQE，`src-tauri/resources/onnx/` + `bundle.resources`
>   → 包内 `<Resources>/models/`）；缺失时宿主自动**退回内置 DSP** 并在任务摘要里标明走的是
>   「模型」还是「内置 DSP」，AI 面板的条目上有对应标签。
> **G3（AI 面板）✅ 完成**（开始/停止按钮：处理中条目下边缘是进度条）。
> **G4 ② 音源分离 ✅ 完成（2026-10-11）**（HTDemucs v4 手写移植 + 四条分轨 + `separate_stems`）；
> **模型分发基建 ✅ 完成**（清单 + 应用内下载器 + 「管理器 → 模型」标签 + 网络页「模型镜像」）。
> G4 其余三项（SVC / SVS / 生成音乐）仍 ⬜。
> **识谱 / 分析并对齐 ❌ 已移除（2026-10-14）**：Beat This! 那一版集成（模型 + AI 面板卡片 +
> MCP 工具 `detect_beats`）整体撤掉——模型本身准，但配这套**按「拍」线性绘制**的时间轴无法稳定对齐。
> **保留**底部条的**节拍器**（`TransportCommand::SetMetronome`，与模型无关）。

**六个目标功能**：音频转 MIDI、音频增强/降噪/去混响、音频分离、生成音乐、人声音色替换、母带处理。
**先做前两个。**

- **G0 · 运行方式（进程内小模型 + 重型模型走 Rust sidecar；**不引 Python**）** ✅
  - **小模型进程内 `ort`**（Basic Pitch 230 KB、LocalVQE 6 MB）—— ✅ 已落地（`ort` 2.0-rc.13，
    `crates/local-ai/src/models/`），**放到后台工作线程**跑（复用 `ai.rs` 的
    「重活走线程 + 进度 + 取消」模式），**UI 主线程与音频线程全程不阻塞**。
  - **统一走 `ort` 进程内，不引入 Python / `candle`**：③④⑤ 全部「自建或现成 ONNX → `ort`」，
    主程序只依赖 `ort`（与 htdemucs 一条路）。**Python 只在开发期「`.pt` → ONNX」导出时用一次**，
    不进随包/运行时。**Rust sidecar 仅作兜底**（将来真遇到不可链接的 GPL 模型、或要崩溃隔离时才用；
    复用 `aidaw` 那套 `externalBin` 分发）。⬜（尚未做）
  - 模型文件解析：`models/` 目录（`$AIDAW_MODELS` → 宿主登记的额外目录（下载落点 + 打包资源）
    → 可执行文件同级 → 应用包 Resources → 仓库根 → cwd），已实现；**两个小模型已随包**
    （`bundle.resources`），**大模型（htdemucs）不随包**、由应用内下载器下到应用配置目录 `models/`。
- **G1 · 音频转 MIDI（Basic Pitch，Apache-2.0）** ✅ 权重用官方 ONNX 导出（无需 TF 转换），
  在 `crates/local-ai/src/models/basic_pitch.rs`；作用于所选轨道的音频片段；
  结果作为**新 MIDI 轨道插入到源轨道的下一行**。已用合成和弦验证（C 大三和弦 → 60/64/67）。
- **G2 · 音频增强 / 降噪 / 去混响** ✅ **拆成两条**（实测确认 LocalVQE 单独一条不够用）：
  - **人声降噪 / 去混响** = LocalVQE v1.2（Apache-2.0）。ONNX 由
    `tools/export-localvqe-onnx.py` 从官方 `.pt` 导出。
  - **乐器底噪降噪** = 内置 DSP（全带宽宽带扩展器）。
  - **为什么必须拆**：LocalVQE 是 16 kHz 的**语音**模型（DNS/AEC 训练），用于音乐时两处伤害叠加
    ——① 采样率降到 16 kHz 再升回来，**8 kHz 以上整段丢失**；② 掩码判的是语音/非语音，
    持续的音乐（pad / 弦乐 / 失真吉他）被判成噪声压掉。实测「乐器都被去掉」。
  - 两条都把结果放在**源轨道下面的一条新轨道**上（`Command::AddAudioTrack`，一次事务 = 一条
    撤销记录，原轨不动 → 直接 A/B 对比试听）。
  - **后续更好的乐器降噪**（按性价比排序）：① **噪声样本 + 谱减法**（Audacity 那套：采一段
    纯底噪 → 全曲减掉），对嘶声/交流声比语音模型强得多；② **先分离再处理**（G4 的音源分离：
    Demucs/MDX 拆轨，只对人声用 LocalVQE 再混回），从根上避免乐器经过语音模型。
- **G3 · AI 面板**：✅ **完成（2026-10-10）**：右侧栏 pane `ai`（`PanelTabs`/`RightPanel` 两处 `ICONS` + 渲染分支
  + i18n），`panes/AiPanel.vue` 与插件面板同一套扁平卡片风格；**MCP 开关 + 接入地址（同一容器内、可复制）**；
  两条本地能力**只要一个开始按钮**（处理中转成「停止」，条目**下边缘变进度条**，可**随时停止**），
  条目上带 `模型 / 内置 DSP` 标签（来自 `ai_models`）；
  结果走同一条命令管线（转 MIDI → `AddNoteTrack` 一次事务；降噪 → 写 WAV + `AddAudioClip`），可撤销。
- **G5 · 节拍检测（BPM + 拍号对齐）**：✅ **以新方式重新实现（2026-10-15）**。
  上一版（自实现 log-mel 前端 + 自导出 ONNX）于 2026-10-14 撤掉——**模型是准的**，但它把结果写成
  **逐小节速度图**，而本仓库的时间轴是**按「拍」线性绘制**的，两者天然对不上（详见撤掉时的记录）。
  这一版改用社区 Rust 移植 crate **`beat-this`**（Beat This!，ISMIR 2024，MIT；默认纯 Rust `rten`
  后端，不需要 `libonnxruntime`）：
  - **只写一个全局 BPM + 一个拍号**（`SetTempo` / `SetTimeSignature`），**不写 keyframes** ——
    恒定速度下网格 / 波形 / 节拍器才稳定对齐；
  - 拍号分子 = **相邻重拍之间的拍数**的众数（分母固定 4，重拍不足退回 4/4）；
    **对齐** = 把片段平移到**下一条**小节线，使**首重拍落在小节线上**（片段起点不能为负）；
  - 速度 / 拍号 / 片段起点 / 片段长度 / 速率复位走 `run_agent_all`，**一次事务 = 一笔撤销**；
  - 权重：`final0`（FP32，83 MB）**不随包**，用户在「管理器 → 模型」下载（id `beat-this`；
    源是上游 GitHub Releases 的 `model-large` 资产，sha256 硬门禁）；**mel 前端（264 KB）随包**
    （`src-tauri/resources/onnx/mel_spectrogram.onnx`）。
  - 宿主 `KIND_TEMPO`（AI 面板「节拍检测」卡片；缺权重时先弹确认框、再跳「管理器」下载）；
    MCP 工具 `detect_tempo`（**34 → 35**）；诊断工具 `crates/local-ai/examples/beat_probe.rs`。
  - 设计与非目标见 `specs/2026-10-15-beat-this-tempo-design.md`；人工验收曲子由
    `tools/make-test-song.py` 生成（120 BPM 4/4，期望面板显示「120.0 BPM · 4/4」）。
  - **适用素材**：只对**节拍稳定**的音乐有效（自由速度 / 氛围音 / 朗读 / <2 秒的素材测出的 BPM
    没有意义）——AI 面板卡片上**常驻一行适用范围说明**，MCP 工具描述里也写了。
    检测中的进度条用**不确定态**（推理是一整段不可打断的调用，release 下 100% 时间都在 beat 模型里）。
  - **保留**：底部条的**节拍器**（`TransportCommand::SetMetronome`）——按工程速度 / 拍号在整数拍上打点、
    小节头重音，只在播放时出声，与任何模型无关。
- **G4 · 其余四项（后续）**：**① 母带 ✅、② 音源分离 ✅（2026-10-11）**、**⑤ 生成音乐 🟡 阶段一**
  （2026-10-15）；③④ 仍 ⬜。
  见 `specs/2026-10-10-local-ai-models-design.md`、`plans/2026-10-10-local-ai-models.md`：
  - **前置基建 ✅ 完成（2026-10-11）**：应用内**模型下载器**——多源（**HuggingFace / 魔搭**等官方源；
    第三方镜像前缀由用户自配）+ sha256 + 断点续传 + 进度/取消；权重落到应用配置目录 `models/`；
    清单见 `crates/local-ai/src/models/catalog.rs`，下载器 `src-tauri/src/models.rs`，事件
    `engine://model`。**魔搭（ModelScope）源已接入**（自建仓库 `ailm32442/htdemucs-ort`，
    sha256 与 HF 一致，排在候选源第一位 / 国内优先）。
    小模型一步到位（≤ 300 MB 直下），GB 级先确认体积（磁盘预检待补）。
  - **优先序**：① **母带处理** ✅（**纯 DSP、不下载模型**，见下）→ ② **音源分离** ✅
    （HTDemucs v4，**MIT**（源自 Meta 的 Demucs），209 MB，**手写移植进 `crates/local-ai/src/models/demucs.rs`**，
    用自己的 `ort` rc.13；`stem-splitter-core` 因精确锁 rc.11 被弃用）→ ③ 歌声替换
    `SoulX-Singer-SVC`（Apache-2.0，~2.8 GB，上游只有 `.pt` → **自建 ONNX 导出**）→
    ④ MIDI 转歌声 `SoulX-Singer`（共用推理管线）→ ⑤ 生成音乐 **ACE-Step v1.5**（Apache-2.0，
    **现成 ONNX**：HF `shreyask/ACE-Step-v1.5-ONNX` / 魔搭 `tak916/acestep-onnx-fp16` 等；
    **走 `ort` 进程内，不引 `candle` / sidecar**）。权重 7.5–25 GB，**按需下载**。
  - **统一运行线（2026-10-11 定，纯 Rust + `ort`、保轻量）**：③④⑤ 一律「**自建或现成 ONNX →
    `ort` 进程内**」（同 htdemucs 一条路）：
    - **Python 只在开发期**把 `.pt` 导成 ONNX（一次性，同 `tools/export-localvqe-onnx.py` 先例）；
      产出 ONNX 后 Python 退场，随包/运行时不带 Python。
    - **不引 `candle`**（原计划的 ACE-Step 运行时）——⑤ 改用现成 ONNX 后，`candle` 与 git 依赖都不再需要。
    - **不需要 sidecar**；sidecar 只作将来 GPL / 崩溃隔离的兜底（且是 **Rust sidecar**，非 Python）。
    - **前置工程 🟡 部分完成（2026-10-15）**：模型系统已从「单文件 + 单 sha256」升级为
      「**多文件组 + 各自 sha256**」（`catalog.rs` 的 `ModelFile` / `ModelEntry.files` /
      `ModelEntry.dir`，下载器逐文件串行下载与原子落地）；**仍待实测** fp16 / 量化算子在
      `ort`（CPU/CoreML）上的可加载性——这是 ⑤ 阶段二的第一道闸门。
  - **① 母带处理 ✅ 完成（2026-10-10，方案 A：纯 DSP）**：新 crate `crates/mastering` ——
    **响度归一（目标 LUFS）+ 真峰值限幅（4× 过采样 + 前瞻）+ 立体声宽度**；宿主侧作为
    `ai.rs` 的 `kind = "master"` 任务（复用进度/取消/事件/新轨机制），AI 面板一张卡片三个参数，
    MCP 工具 `master_project`（工具数 32 → 33）。14 条单测，含采样间峰值（+3 dB）那条。
    **不引入任何模型**，因此下载基建的首验顺延给 ②。设计见 spec §6 的拍板记录。
    未做：实时主总线上的效果链（`Project.master` 目前只有 gain/pan）。
  - **② 音源分离 ✅ 完成（2026-10-11，HTDemucs v4）**：`crates/local-ai/src/models/demucs.rs`
    **手写移植** `stem-splitter-core` 的 Demucs 复数谱（cac）STFT + iSTFT + 窗口循环，用本仓库的
    `ort` rc.13 + `rustfft`（**不引第二套运行时**）；模型 `gentij/htdemucs-ort` 的 `htdemucs.ort`
    （209 MB，HTTP ORT 格式，双输入 `input`/`x`、双输出 `add_67`/`output`）。宿主 `KIND_SEPARATE`
    取当前轨第一段音频 → 输出**鼓/贝斯/其他/人声四条新轨**（一次事务 = 一条撤销记录），MCP 工具
    `separate_stems`（33 → 34）；AI 面板「音源分离」卡片**缺权重时按钮变「下载模型 · 200.2 MB」**。
    **无内置 DSP 兜底**（分离没有近似实现）。单测：STFT↔iSTFT 往返（无权重）+ 权重存在的分离 smoke。
    顺带解锁 G2 的「先分离再处理」降噪路径（后续）。
  - **实查修正**：`ace-step-rs` **未发布到 crates.io**（只能 git 依赖）；SoulX 两个模型 HF 上
    **只有 `.pt`、无 ONNX**（需自建导出）；`denoize` 是**降噪/响度修复**而非母带处理，且用
    `tract-onnx`、要求 Rust 1.96 —— **不宜放在「母带处理」这一格**。
  - **运行时策略**：统一 `ort`；`tract` 仅作显式记录的例外；`candle` 不参与本轮。
  - 凡不可 permissive 的权重（如 Matchering 的 copyleft）一律 **sidecar-only**。
- **护栏**：每个模型的权重许可必须逐项核对；GPL / AGPL / CC-BY-NC 一律不链接、只走 sidecar；
  MPL 组件需未修改并在 `NOTICE` 记名（沿用 symphonia 先例）。

## 6. 里程碑

**M1（Stage A 完成：契约单一化 + 实时传输可用）**：✅ 达成。A2 已完成；A1 于 2026-10-09 完成 ——
`Command` / `Event` / `Project` 的 schema 现由 `crates/ipc` 单点持有，引擎 ↔ Tauri ↔ MCP 共用同一份契约。

**M2（Stage B：可托管 VST3/CLAP）**：✅ 基本达成（B1–B3 完成，B4 部分；见 Stage B）。

M3（Stage C：可录音、有音频片段）→ M4（Stage D：可被 AI 驱动）→ M5（Stage E：插件商店 + 崩溃隔离）。

## 7. 文档索引

- 设计意图：`specs/2026-10-07-ai-daw-design.md`
- 设计（specs）：
  - `specs/2026-10-09-ipc-contract-design.md`（A1，✅ 完成）
  - `specs/2026-10-09-audio-clip-design.md`（C1，✅ 完成）
  - `specs/2026-10-10-recording-design.md`（C2，✅ 完成）
  - `specs/2026-10-10-mixing-sends-buses-automation-design.md`（C3，✅ 完成）
  - `specs/2026-10-10-plugin-param-automation-design.md`（C3 扩展：插件参数自动化 + 卷帘参数车道，✅ 完成）
  - `specs/2026-10-15-beat-this-tempo-design.md`（G5：节拍检测 —— 全局 BPM + 拍号对齐，✅ 完成）
  - `specs/2026-10-15-ace-step-music-generation-design.md`（G4⑤：生成音乐 ACE-Step 1.5 ——
    多文件模型组 + 「AI 音乐生成」面板，🟡 阶段一完成、阶段二待做）
  - `specs/2026-10-10-cli-path-install-design.md`（F4：`aidaw` 随包分发并上 PATH，✅ 完成）
  - `specs/2026-10-10-plugin-manager-design.md`（E1：插件管理器 —— 安装 / 导入 / 更新 / 删除，✅ 完成）
  - `specs/2026-10-09-synth-overhaul-design.md`（✅ 完成）
  - `specs/2026-10-09-audio-device-resilience-design.md`（✅ 完成）
  - `specs/2026-10-08-builtin-synth-ui-redesign-design.md`（✅ 完成，预设部分被 synth-overhaul 取代）
  - `specs/2026-10-08-dynamic-tempo-meter-design.md`（✅ 完成）
  - `specs/2026-10-08-transport-loop-design.md`（✅ 完成）
  - `specs/2026-10-08-ux-clipboard-and-md3-controls-design.md`（✅ 完成）
- 分计划（plans）：
  - `plans/2026-10-07-engine-core.md`（Plan 1，✅ 完成）
  - `plans/2026-10-07-realtime-playback.md`（Plan 2，✅ 完成）
  - `plans/2026-10-07-minimal-ui-shell.md`（Plan 4a，✅ 完成，实现栈已改为 Vue）
  - `plans/2026-10-08-arrange-gutter-and-grid.md`（✅ 完成）
  - `plans/2026-10-08-builtin-synth-ui.md`（✅ 完成）
  - `plans/2026-10-08-dynamic-tempo-meter.md`（✅ 完成）
  - `plans/2026-10-08-transport-loop.md`（✅ 完成）
  - `plans/2026-10-08-ux-clipboard-and-md3-controls.md`（✅ 完成）
  - `plans/2026-10-09-audio-clip.md`（✅ 完成）
  - `plans/2026-10-10-recording.md`（C2，✅ 完成）
  - `plans/2026-10-10-mixing-sends-buses-automation.md`（C3，✅ 完成）
  - `plans/2026-10-10-plugin-param-automation.md`（C3 扩展：插件参数自动化 + 卷帘参数车道，✅ 完成）
  - `plans/2026-10-09-audio-device-resilience.md`（✅ 完成）
  - `plans/2026-10-09-ipc-contract.md`（✅ 完成）
  - `plans/2026-10-10-aidaw-cli-mcp-client.md`（Stage F 改向后的 `aidaw` 薄客户端实现计划，✅ 完成）
  - `plans/2026-10-10-cli-path-install.md`（F4：`aidaw` 上 PATH，✅ 完成）
  - `plans/2026-10-10-plugin-manager.md`（E1：插件管理器 —— 安装 / 导入 / 更新 / 删除，✅ 完成）
- 调研（research）：
  - `research/2026-10-10-open-source-survey.md`（录音 / 混音 / MIDI 输入 / 插件商店：可直接用的库与只能参考的项目，含许可结论）
- skills 文档包（渐进式披露，AI harness 用）：`skills/aidaw/`（`SKILL.md` + `references/` 能力域，含乐理 / 节奏鼓谱 / 曲风 / 编曲流程 / **影片配乐 film**；由 MCP 的 `aidaw_help` 提供）
- 进度与后续计划的唯一索引：**本文件**。
