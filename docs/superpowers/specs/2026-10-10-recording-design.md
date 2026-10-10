# 录音（C2）— 设计

> **状态（2026-10-10）**：**已完成**。引擎采集（`crates/engine/src/recorder.rs`）、宿主命令、
> 前端底部条录音键/设备选择/电平表均已落地；引擎侧真机采集已验证。逐任务记录见
> `plans/2026-10-10-recording.md`。

## 1 背景

Roadmap Stage C 的 C2 / Plan 5：**录音（cpal 输入流 → 写入 `AudioClip`）**。C1 已把音频片段
（`Clip.audio` / `AudioSource` / `AddAudioClip` / `AssetCache` / 「素材铺满片段跨度」播放路径）落地，
录音因此是**在既有音频片段能力上加一条采集路径**，而不是新模型。开源零件调研见
`../research/2026-10-10-open-source-survey.md` §2（cpal + rtrb 已在本项目依赖里，无 drop-in 录音器）。

## 2 目标与非目标

**目标**

- 从音频输入设备（麦克风 / 线路输入）录制，落到**当前选中轨道**的播放头处，成为普通**音频片段**。
- 可**选择输入设备**，并在录前**看到输入电平**。
- 录制结果可播放、可撤销（一次录音 = 一个撤销步骤）。

**非目标（v1 明确不做）**

- **软件监听**（把输入直接送回输出）：反馈/啸叫风险大，默认不做。
- **录音延迟补偿**、punch in/out、多 take / 叠录、录音臂（arm）状态。
- 保音高变速（沿用 C1 决策：片段拍长是唯一真相，缩放/改速度会变调）。
- MIDI 输入（Roadmap 独立条目，用 `midir`，不在此）。
- 素材库 / 相对路径：录音落**绝对路径**（与导入一致）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 输入流归属 | **引擎 `AudioHost`**（与输出流同一条音频线程） | 项目既定边界：引擎拥有音频 IO、`cpal` 流在专用音频线程；新回调因此能被既有「零分配」门禁覆盖 |
| IPC 契约 | **零变更** | 复用 `AudioSource` / `AddAudioClip` / `AssetCache` / 「铺满」播放路径；录音只是「产出一个 WAV + 加一条音频片段」 |
| 采样率 | **保留输入设备原生采样率** | 不引入 `rubato`；播放侧按 C1 的比值换算（与 C1 §3 一致） |
| 落盘位置 | **应用数据目录 `recordings/`** | 对应 `AudioSource` 注释里的既定意图（「录音落在应用数据目录」）；与工程文件解耦 |
| 监听 | **不做** | 避免反馈；用户可自行用设备监听 |
| 延迟补偿 | **不做** | v1 从播放头对齐，接受往返延迟的固定偏移 |
| 输入流生命周期 | **选定设备即打开**（供电平表），录音只是**开始追加样本** | 录前要能看电平；避免「按下录音才建流」的抖动 |

## 4 架构与数据流

```
输入回调（实时线程，零分配）
  └─ 转成交错 f32（f32 / i16 / u16）→ 推进 rtrb 环
  └─ 下混推一份到 input ScopeBuffer（电平表，原子写）
        │
        ▼  （无锁 SPSC）
  消费线程：从环里取帧，追加进 Vec<f32>
        │
        ▼  （record_stop）
  DecodedAudio {samples, channels, sample_rate = 输入设备采样率}
        │
        ├─ 写 WAV：app_data_dir()/recordings/rec-<epochms>.wav（16-bit PCM，复用 hound）
        ├─ 插入 AssetCache（**免一次重解码**）
        └─ AddAudioClip { track, start = 捕获起点(拍), length = 自然时长(拍), source }
```

新增 **`crates/engine/src/recorder.rs`**（`realtime` feature 下）：

- `InputDeviceInfo { name, sample_rate, channels, transport, is_default }`、`list_input_devices()`、
  `default_input_device()`。`transport` 是连接类型短名（macOS 查 CoreAudio `TransportType`：内置 /
  蓝牙 / 虚拟…，其他平台为 `None`）——蓝牙耳机、虚拟声卡在 macOS 上也会注册输入端点，靠它区分。
- `Recorder`：
  - `open(device: Option<String>, ring_capacity, scope) -> Result<Recorder, RealtimeError>`：
    取输入设备（名字匹配，否则默认），按 `default_input_config` 的采样格式建 `build_input_stream`
    （f32 / i16 / u16 三种），回调**只做转换 + `rtrb` push + ScopeBuffer 写**，不分配、不加锁。
  - `start()` / `stop() -> Option<DecodedAudio>`：`start` 置原子旗标让回调开始追加；
    `stop` 停流、join 消费线程、把 `Vec<f32>` 封成 `DecodedAudio`（空则 `None`）。
  - `level()`：读 input `ScopeBuffer`（供 Tauri 命令）。
- 声道收敛与解码一致：1 → 单声道；≥2 → 取前两声道。环满 → **丢帧并计数**（绝不阻塞回调）。

`AudioHost` 增加输入生命周期（`realtime.rs`）：

- `arm_input(device) -> Result<(), RealtimeError>` / `disarm_input()`：建/停输入流 + 电平。
- `start_recording()` / `stop_recording() -> Option<DecodedAudio>`：开始/结束追加。
- 输入流与输出流相互独立，重建输出宿主（工程编辑）**不影响**输入流与正在进行的录音。

## 5 宿主（src-tauri）

- `AudioEngine` 增加 `AudioMessage::{ArmInput, DisarmInput, StartRecording, StopRecording(回复通道)}`
  与对应发送方法；`run_audio_thread` 转发到 `AudioHost`。
- 新命令（全部 `actor = Human` 的写操作走既有 `run`）：
  - `list_input_devices() -> Vec<InputDeviceInfo>`
  - `set_input_device(device: Option<String>)`（`None` = 关闭输入流）
  - `input_level(points) -> Vec<f32>`（读 input `ScopeBuffer`）
  - `record_start(track, start_frame, device) -> Result<(), String>`：若目标设备尚未 arm 则先 arm
    （已 arm 同一设备就复用，换设备则重建输入流）+ 开始录制 + **走带播放**
  - `record_stop() -> Result<Option<Project>, String>`：停流 → 写 WAV（`spawn_blocking`）→
    `AssetCache::insert` → `AddAudioClip`（起点/长度按捕获起点与自然时长换算）→ 返回工程
- `AssetCache` 增加 `insert(path, Arc<DecodedAudio>)`（录音刚落盘时免去重解码）。

## 6 前端

- `BottomBar.vue` 传输键中间组新增**录音键**（在「停止」右侧）：空闲 → 红点；录制中 → 高亮 +
  计时。按下 `record_start`，再按 / 按停止 → `record_stop`。
- 录音键旁：**输入设备下拉**（`list_input_devices`，带连接类型标注）+ **输入电平表**（`input_level` 轮询画条）。
- **输入流按需打开**：启动/挂载时**不**自动 arm 输入设备（否则一启动就占用麦克风、弹权限提示）。
  只有用户显式选设备、或点录音（`record_start` 会 arm 默认设备）时才打开输入流；录音前没显式选设备时，
  录完（`record_stop`）即释放，不让麦克风常开。输入流没开时电平表归零、不发 IPC。
- `stores/transport.ts`：`recording` 状态 + `startRecording(track)` / `stopRecording()`；
  录制中禁用会打断采集的动作（切设备等）。
- `api/tauri.ts` + `api/mock.ts`：上述命令与浏览器预览的 mock（造一条假录音）。
- i18n：中英各补录音相关键（**键集必须一致**）。

## 7 错误处理

| 场景 | 处理 |
| --- | --- |
| 无输入设备 / 打开失败 | `arm_input` / `record_start` 返回可读错误，不进入录制，界面提示 |
| 录制中输入设备被拔 / 流错误 | 结束本次录制，**保留已采集部分**（非空则照常成片段），提示原因 |
| 环满（消费跟不上） | 丢弃新帧并计数；不阻塞实时回调，界面可提示「有丢帧」 |
| `record_stop` 时缓冲为空 | 不生成片段、不写文件，返回 `Ok(None)`，界面提示「没有录到内容」 |
| 写盘失败 | 返回错误，不进撤销栈，不挂空片段 |

## 8 验收标准

1. **引擎单测**：`Recorder` 用假输入源（测试里直接往生产端推）验证「环 → 组装」逐样本一致；
   采样率与声道（1/2）收敛正确；环满丢帧**不 panic**且计数正确；空录制返回 `None`。
2. **实时安全**：`tests/realtime_safety.rs` 的门禁**扩展覆盖输入回调**（转换 + push 期间零分配）。
3. **宿主**：`record_start` → 推入已知样本 → `record_stop` 得到 WAV + 一条音频片段（可用临时目录/回环设备）。
4. **前端**：`npm run build` 通过；i18n 中英键数一致。
5. **真机端到端**：选设备 → 电平表随声音动 → 录音 → 停止 → 轨道出现音频片段、可播放、
   一次撤销即移除；导入/播放/导出等既有能力不回归。

## 9 验证方式

```
cargo test --workspace
cargo test -p ai-daw-engine --features realtime
cargo test -p ai-daw-engine --features realtime --test realtime_safety
cargo check --manifest-path src-tauri/Cargo.toml
npm run build            # + i18n 中英键数一致
```

## 10 风险

| 风险 | 缓解 |
| --- | --- |
| 输入回调破坏「零分配」 | 回调只做转换 + `rtrb` push + 原子写；门禁扩展到输入路径 |
| 输入 / 输出不同时钟造成漂移 | v1 接受（无监听，用户按听感对齐）；保音高/对齐留后续 |
| 设备只支持非 f32 格式 | 明确支持 f32 / i16 / u16 三种并转换；不支持的格式给出可读错误 |
| 器件往返延迟导致录音偏移 | v1 不做补偿；作为已知取舍记录，后续再加手动/自动补偿 |
| 录音文件随撤销成为孤儿 | 接受：撤销只移除片段，不删文件（与导入素材同一策略）；素材库/清理留后续 |
| 录制中编辑工程触发输出宿主重建 | 输入流独立于输出流，重建输出不影响采集 |
