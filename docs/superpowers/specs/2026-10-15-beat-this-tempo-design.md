# 节拍检测（Beat This!）：BPM + 拍号对齐 — 设计

- 日期：2026-10-15
- 状态：✅ 已实现
- 相关：`crates/local-ai/src/models/beat_this.rs`、`src-tauri/src/ai.rs`（`KIND_TEMPO`）、
  `src-tauri/src/mcp.rs`（`detect_tempo`）、`src/components/workspace/panes/AiPanel.vue`、
  `crates/local-ai/src/models/catalog.rs`、`tools/make-test-song.py`

## 1. 背景：上一版为什么被撤掉

2026-10-14 把曾经的一版 Beat This! 集成（ROADMAP G5）**整体撤掉**了。撤掉的理由不是模型不准，
而是**集成方式与时间轴模型不匹配**：

- 本仓库的时间轴是**按「拍」线性绘制**的（`px = 拍 × zoom`，见 `TimelineRuler.vue` / `stores/timeline.ts`）；
- 上一版把检测结果写成了**逐小节变化的速度图（keyframes）**；
- 全局速度一旦逐小节变化，「一拍 = 固定像素」的前提就破了 → 波形 / 网格 / 节拍器无法稳定对齐。

## 2. 目标与非目标

**目标**：给一段音频推出**一个全局 BPM 与一个拍号**，写进工程，并让**波形、网格、节拍器对齐**。

**非目标**：逐小节速度图、变速段落的跟随、DBN 后处理、把检测到的拍点逐个画成标记。

## 3. 决策

| 决策 | 选择 | 理由 |
| --- | --- | --- |
| 用哪个模型 | **Beat This!**（ISMIR 2024，MIT）的 Rust 移植 crate **`beat-this`** | 上一版是自实现 log-mel 前端 + 自导出 ONNX；直接用社区移植省掉「前端与参考实现是否一致」这一整类问题。默认后端 `rten` 是纯 Rust，不需要 `libonnxruntime`，与本仓库「不引 Python / 不引第二套运行时」一致。 |
| 权重 | **`final0`**（FP32，约 83 MB） | 上游把 FP32 权重单独放在 GitHub Releases 的 `model-large` 里；`beat_this.onnx` 这个资产就是 `final0` 的导出。 |
| 权重分发 | 主模型**不随包**、进「管理器 → 模型」下载列表；**mel 前端（264 KB）随包** | 用户明确要求主模型不随包、自行下载。mel 前端太小（264 KB，与 Basic Pitch 同量级），随包省掉一次下载，且它只是前端、不是「模型本体」。 |
| 速度 | **只写一个全局 BPM**（`SetTempo`），**不写 keyframes** | 这是对齐能成立的前提（§1）。 |
| 拍号 | 分子 = **相邻重拍之间夹着的拍数**的众数；分母固定 4；重拍不足退回 4/4 | Beat This! 只输出「哪些拍是重拍」，不直接给拍号。 |
| 对齐 | 把片段起点**平移到下一条小节线**，使**第一个重拍落在那条小节线上** | 片段起点**不能为负**（`Clip::normalize_start` 夹到 0），所以只能吸附到「下一条」而不是「第 1 小节线」。不裁剪音频、不改内容。 |
| 片段长度 / 速率 | `length` = 素材窗口在新速度下的自然拍长；`rate` 复位 1.0 | 方框与素材同长（放完即停），播放回到原速，所以「网格的一拍」= 「音乐的一拍」。 |

**一次事务**：`SetTempo` + `SetTimeSignature` + `MoveClip` + `ResizeClip` + `SetClipRate` 走
`commands::run_agent_all`，因此是**一笔撤销记录**，与「音频转 MIDI / 降噪 / 分离」同一套管线。

## 4. 实现

**`crates/local-ai/src/models/beat_this.rs`**
- `model_paths()`：两个权重都在位才可用（mel 前端 + final0）。
- `detect(samples, channels, sample_rate, progress) -> TempoDetection`：交错多声道 → 下混单声道 →
  `BeatThis::analyze_audio` → `calculate_bpm` + `time_signature_num`。取消走 `Progress` 回调
  （推理是一整段不可中断的调用，只在两端检查）。
- `time_signature_num()`：把每个重拍映射回拍点序列的下标，取相邻下标之差的众数（并列取较小者）。

**宿主 `src-tauri/src/ai.rs`**
- 新任务 `KIND_TEMPO = "detect_tempo"`；`AiModels` 增加 `beat_this` 字段。
- `TrackInput` 增加 `track` / `clip`（对齐要按它们改片段）。
- `apply_tempo_detection()`：只对片段的**可见窗口**（`offset_frames` / `frames`）做检测；
  算完相位后用 `(bar - phase mod bar) mod bar` 得到新起点。

**MCP**：新工具 `detect_tempo(track)`，同步走同一条 `apply_tempo_detection`。

**前端**：AI 面板新增「节拍检测」卡片，与「音源分离」同一套交互（缺权重时先弹确认框、确认后跳
「管理器 → 模型」下载）。

## 5. 验证

- `crates/local-ai/tests/models_e2e.rs`：合成 8 小节 120 BPM 4/4 循环 → 断言 BPM≈120、4/4、
  重拍数、首重拍贴起点；另若 `test-files/beat-test-120bpm.wav` 在，也直接验它。
- `crates/local-ai/examples/beat_probe.rs`：诊断工具，打印原始拍点 / 重拍 / BPM / 拍号估计，
  用来判断「是模型的问题还是集成的问题」。
- `tools/make-test-song.py`：生成给人工验收用的简单曲子（120 BPM 4/4）。
- `cargo check`：`ai-daw-local-ai` 与 `ai-daw-app` 都通过；`vue-tsc --noEmit` 通过。

## 6. 已知限制

- **适用素材**：只对**节拍稳定**的音乐有效（流行 / 摇滚 / 电子 / 舞曲等）。自由速度（古典散板、
  现场即兴）、氛围 / 环境音、人声朗读、短于 2 秒的素材，测出来的 BPM 没有意义。AI 面板的卡片上
  **常驻一行适用范围说明**（不随任务状态变化），MCP 工具描述里也写了同一句——避免用户试完觉得
  「这功能没用」。
- **拍号靠重拍间隔推**：Beat This! 的拍点很稳，但重拍需要音乐里的**小节级线索**（和声变化、
  吊镲、贝斯根音）。像「四踩底鼓 + 2/4 军鼓」这种缺少和声变化的循环，它会**每两拍**给一个重拍，
  拍号就被推成 2/4。这不是本次集成引入的——是模型在缺少小节线索时的固有模糊（`beat_probe` 可直接看到）。
- **进度条是不确定态**：Beat This! 的推理（内部还包含重采样与频谱）是一整段不可打断的调用，
  crate 没有进度回调，给百分比只会「停在 5% 再跳满」。面板因此对这条任务用**不定长条纹**，
  而不是假的百分比；取消也只在推理两端生效。
- **只处理恒定速度**：有意为之（§1）。要做变速跟随，需要先让时间轴支持「秒 ↔ 拍」的非线性映射。
- **分母固定 4**：6/8、7/8 这类会被读成相近的四分音符拍数。
