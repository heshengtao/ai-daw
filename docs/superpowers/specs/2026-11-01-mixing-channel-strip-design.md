# 混音完善（C4）：计量 / 动态处理 / 侧链 / 通道条 — 设计

> **状态（2026-11-01）**：设计稿，待评审。M1/M2 给到可实现粒度；M3 只划边界，需要各自的
> spec。前置是 C3（发送 / 总线 / 自动化），见
> `2026-10-10-mixing-sends-buses-automation-design.md`。
>
> 本稿只描述「补什么、按什么顺序、契约怎么变、怎么验」，不含排期。

## 1 背景

C1/C2/C3 之后，混音的**骨架**已经齐了：推子 / 声像 / 静音 / 独奏、推子后发送 → 总线、
拓扑路由、按拍逐块求值的自动化、离线母带。缺的是**通道条工艺**这一层：动态处理、电平计量、
主总线插入、延迟补偿、侧链。现状是「能摆开，但压不住、看不见、接不上」。

三处最具体的表现：

1. **没有压缩器**。内置效果器只有 EQ（bass / treble / mix 三段）、Drive、Chorus、Delay、
   Reverb（`crates/effects/src/lib.rs`）。人声、鼓组、贝斯都没有可用的动态控制。
2. **没有电平表**。混音台一个通道条上没有任何表（`src/views/MixView.vue`）；全工程只有底部条
   一个主输出峰值条，实现是每 120 ms 取最近 64 个采样求峰值（`src/components/layout/BottomBar.vue`
   的 `setInterval`，后端 `scope_data`）——48 kHz 下等于每 120 ms 只看 1.3 ms 的音频。
3. **侧链接不上**。模型、ABI、宿主 I/O、界面四层都没有「键输入」这个概念，装了第三方侧链
   压缩器也没有线可以接。

## 2 现状盘点（可核对的代码位置）

| 能力 | 现状 | 位置 |
| --- | --- | --- |
| 通道参数 | 只有 `gain_db / pan / mute / solo` | `crates/ipc/src/model/track.rs:20` |
| 声像 | 线性平衡式，无 pan law 选项 | `crates/engine/src/player.rs` 的 `pan_gains` |
| 发送 | `{ target, gain_db }`，**固定推子后** | `crates/ipc/src/model/track.rs:68` |
| 路由顺序 | Kahn 拓扑排序，环上的发送被忽略 | `crates/engine/src/schedule.rs:234` |
| 插入链 | 有，但**无 bypass**、无 A/B | `crates/ipc/src/model/track.rs:585` |
| 主总线 | 只有 gain / pan，**无插入口** | `crates/ipc/src/model/project.rs:96` |
| 动态处理 | 无（限幅器只在离线母带里） | `crates/mastering/src/lib.rs` |
| 计量 | 仅主输出「采样点峰值」，无每轨表 | `src-tauri/src/commands.rs` 的 `scope_data` |
| 自动化目标 | `gain_db / pan / cc / at / inst / fx`，无 mute / send / master / bypass | `crates/engine/src/schedule.rs:26` |
| 插件 ABI | `process()` 只有主声道；无 latency、无 aux 输入 | `crates/plugin-api/src/plugin.rs:60` |
| VST3 声道 | 效果器固定 2 进 2 出，走单总线 `AudioBuffers` | `crates/host-vst3/src/vst3.rs:77` |
| 独奏 | 任一轨 solo 时其余全静音，**总线被连坐** | `crates/engine/src/player.rs:940` |
| 导出 | 16-bit PCM WAV（硬夹 ±1.0）/ MP3 | `crates/engine/src/wav.rs:14` |

C3 的设计文档已把 pre-fader send、send 自动化、多级延迟补偿、**侧链**、反馈环明列为 v1 非目标
（`docs/superpowers/specs/2026-10-10-mixing-sends-buses-automation-design.md` §2）；ROADMAP 也
写着「未做：实时主总线上的效果链」（`docs/superpowers/ROADMAP.md:727`）。所以这不是「漏了」，
是当时有意收的口子——C4 就是把口子收回来。

## 3 目标与非目标

**目标**

1. 每条轨道（含总线、含主输出）都有**可用的电平表**：左右峰值、削波锁存、动态处理的增益衰减。
2. 有**能用的动态处理器**：压缩器（无侧链）、真峰值限幅器（主总线），参数可自动化。
3. **主总线有插入链**，限幅器可以挂在上面，导出与实时走同一条链。
4. 效果器可以**旁通**，且旁通**不重建播放器**、不断音。
5. **侧链**：内置压缩器可以拿另一条轨道的推子后信号做检测；工程文件里能存、能撤销。
6. 第三方插件（VST3 / CLAP）**能声明侧链接口**，宿主能把键信号送进去；不支持时界面明确禁用。
7. 顺带修掉「solo 把总线一起哑掉」这个语义错误。

**非目标（本稿不做）**

- 环绕 / Atmos / 多声道通道格式。引擎恒为立体声。
- 编组 / VCA / 文件夹轨。M3 列了边界，但要独立 spec。
- 冻结 / 就地渲染（bounce in place）、分轨导出进 GUI。
- 自动化读写模式（touch / latch / write）、曲线形状。
- 硬件控制台（Mackie / HUI）、监听 FX、cue mix。
- 母带风格迁移 / 参考匹配。离线母带维持现状。
- AU 的侧链（当前 AU 适配器只喂一块输入缓冲，要改 pull block 与格式协商，M2 不做）。

## 4 里程碑

| 里程碑 | 内容 | 规模 | 依赖 |
| --- | --- | --- | --- |
| **M1** | 计量 + 旁通 + 压缩器 + 主总线插入 + 实时限幅 + solo 语义 | 中（六块，不动 ABI 的 `process`） | 无 |
| **M2** | 侧链：ABI 默认方法 + 模型字段 + 键缓冲 + VST3/CLAP 适配 + UI | 大（动 ABI 与两个宿主适配器） | M1 的压缩器 |
| **M3** | 发送 pre/post、直接输出、PDC、片段增益/淡化、编组/VCA | 大（拆成多个 spec） | M2 的路由改动 |

M1 单独就能显著改变可用性：有了表和压缩器，混音才有反馈闭环。M2 的真正成本不在 DSP
（键信号就是一路旁路输入），而在**宿主适配**和**路由边**。

## 5 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 主总线插入放哪 | `Project` 新增兄弟字段 `master_effects`，**不**把 `master` 改成结构体 | 全仓 `.master` 有 45 处引用；兄弟字段让旧 JSON 逐字不变、改动面最小 |
| 计量数据通路 | 固定槽位 `MeterBank`（256 槽）+ 原子量，UI 轮询 | 与既有 `ScopeBuffer` 同一套「音频线程写原子、界面线程读」的思路；不加锁、不分配 |
| 峰值累加 | `f32` 位模式做 `fetch_max` | 非负 IEEE754 浮点的位模式按 `u32` 比较是单调的；读侧 `swap(0)` 天然实现「上次读取以来的峰值」 |
| 表头弹道 | 后端只给峰值，弹道 / 保持 / 削波显示全在 UI | 后端保持 RT 路径最简；换刷新率不用动引擎 |
| 旁通 | 走运行时命令，**不重建播放器** | 每次开关都重建会断音、重置插件状态 |
| 压缩器前瞻 | **0 延迟**，不做 lookahead | 一旦有延迟就必须先有 PDC；M3 之前不让内置压缩器引入补偿需求 |
| 限幅器位置 | 只承诺主总线；带 ~5 ms 前瞻 | 主总线是链路末端，整体延迟不需要补偿 |
| 侧链 ABI | 新增带默认实现的 `process_with_sidechain`，不改 `process` 签名 | 现有 impl（内置合成器 / 5 个内置效果 / VST3 / CLAP / AU / 占位插件）全部零改动 |
| 侧链模型 | `PluginInstance.sidechain: Option<SidechainSource>` | 插件的属性，不是轨道的属性；空时不序列化 |
| 键信号取点 | 与发送一致：**推子后** | 用户直觉是「跟着那条轨的推子走」；预推子留给 M3 作为选项 |
| VST3 侧链 | 切到 `vst3-host` 自带的 bus API | 依赖 0.9 已有 `process_bus_audio` / `BusAudioBuffers` / `audio_bus_layout` / `set_bus_arrangements`，现在的适配器只用了单总线 |
| AU 侧链 | 不做，UI 置灰并说明 | 适配器只有一块输入缓冲、pull block 忽略 bus 参数，硬做要改渲染路径 |

## 6 数据契约与向后兼容

全部新增字段都遵守仓库既有的「旧工程 JSON 形状逐字不变」纪律：

| 变更 | 缺省 | 序列化 | 旧工程 |
| --- | --- | --- | --- |
| `PluginInstance.enabled: bool` | `true` | 仅 `false` 时写 | 读入为 true |
| `PluginInstance.sidechain: Option<SidechainSource>` | `None` | 仅 `Some` 时写 | 读入为 None |
| `Project.master_effects: Vec<PluginInstance>` | `[]` | 仅非空时写 | 读入为空 |
| `Send.mode: SendMode`（M3） | `Post` | 仅 `Pre` 时写 | 读入为 Post |
| `Track.output: Option<TrackId>`（M3） | `None` | 仅 `Some` 时写 | 读入为 None |

wire-shape 测试逐条断言（既有工程样例 + 新字段不出现）。

**实现成本提示**：`PluginInstance` 加字段会波及全仓 36 处结构体字面量（19 处在 `player.rs`
的测试里，其余散布在 `apply.rs` / `mcp.rs` / 各测试文件）。建议同时补一个
`PluginInstance::new(plugin_id)` 与 `impl Default`，把新增改成机械替换；**不要**为了省这点改动
把旁通 / 侧链挂到「按槽位下标索引的平行数组」上——下标不随效果器重排移动，会出现「换了顺序
旁通跟着走」的错位。

## 7 M1：看得见、压得住

### 7.1 计量（`crates/engine/src/meter.rs`）

```rust
pub const METER_SLOTS: usize = 256; // 与轨道序号一一对应；超出不计量

pub struct MeterSlot {
    peak_l: AtomicU32,  // f32 位模式，0.0 = 无信号
    peak_r: AtomicU32,
    gr_db: AtomicU32,   // 正的衰减量（dB），0 = 没压
    clipped: AtomicBool,
}

pub struct MeterBank {
    tracks: Box<[MeterSlot]>, // 下标 = 工程轨道序号
    master: MeterSlot,
}
```

- **写（音频线程）**：每块处理完一轨后调 `publish(slot, l, r, gr)`：两次 `fetch_max`（位模式）、
  一次 `fetch_max`（GR）、峰值 ≥ 1.0 时 `clipped.fetch_or(true)`。全是原子操作，零分配。
- **读（控制侧）**：`read_track(slot)` / `read_master()` 用 `swap(0)` 取走并清零；`clear_clip()`
  由界面的「点表清零」调用。
- **取点**：推子与声像**之后**、进 master / 总线**之前**——表头显示的就是这条轨真正送出去的东西。
  总线轨的表因此显示汇总后的输出。
- **GR 取点**：本轨效果链跑完后，取各动态处理器 `gain_reduction_db()` 的**最大值**
  （即压得最狠的那个）。一轨挂多个压缩器时只显示一个数，文档写明。
- **主输出表**：master 推子之后（= 你听到的），单独一个 `MeterSlot`。
- 容量：`METER_SLOTS = 256`，超出部分不计量（工程规模远未到；宁可不动）。
- **接线**：`MeterBank` 在 `AudioEngine::idle` 里创建（与 `scope` 同构，`Arc` 共享），随每次
  `Player` 重建传进去；槽位号 = 该轨在 `project.tracks` 里的下标，宿主读表时按同一份工程
  顺序换算成 `TrackId`。

IPC：新增命令 `mixer_meters()` → 

```rust
struct MeterSnapshot { master: MeterValue, tracks: Vec<TrackMeter> }
struct MeterValue { peak_l: f32, peak_r: f32, gr_db: f32, clipped: bool }
struct TrackMeter { track: u64, value: MeterValue }
```

轮询 30 Hz（页面不可见时停），单帧 ≤ 256 × 28 B ≈ 7 KB。**不用事件推送**：现在没有高频事件
通道，为一个表新开一条推送路径不划算；将来要换，接口形状不变。

UI 侧：弹道（峰值 -20 dB/s 回落）、峰值保持（1.5 s）、削波锁存（点击清除）全在前端做。

### 7.2 效果器旁通

- `PluginInstance.enabled`（见 §6）。
- 命令 `SetEffectEnabled { track, effect, enabled }`；事件 `EffectEnabledChanged`，
  `is_mix_only = true` → `TransportCommand::SetEffectEnabled`，音频线程**跳过**该效果器但不销毁
  实例（状态保留，再打开时接着用）。
- 调度期也带 `enabled`，重建后保持。
- 副作用：开关瞬间电平会跳。M1 接受（与主流 DAW 的硬旁通一致）；若实测有爆音，再加 5 ms
  斜坡，**不在本稿加复杂度**。
- 主总线插入同样支持（`SetMasterEffectEnabled`）。

### 7.3 内置压缩器（`crates/effects/src/compressor.rs`）

插件 id `builtin.compressor`。参数：

| 参数 | 范围 | 说明 |
| --- | --- | --- |
| `threshold` | -60…0 dB | 起压点 |
| `ratio` | 1…20 | 1 = 不压 |
| `attack` | 0.1…200 ms | |
| `release` | 5…2000 ms | |
| `knee` | 0…24 dB | 软拐点 |
| `makeup` | -12…24 dB | 补偿增益 |
| `mix` | 0…100 % | 并行压缩（干湿） |

实现要点：

- **立体声联动**检测：`max(|L|, |R|)` → 峰值检测 → dB 域、按 `knee` 算目标衰减 → 单极点平滑
  （attack / release 系数在 `prepare` 里按采样率算好）。0 延迟、无前瞻。
- 每采样几次乘加，不需要 SIMD。
- 新增 ABI 默认方法（见下），压缩器返回当前 GR：

```rust
// plugin-api
fn gain_reduction_db(&self) -> Option<f32> { None }
```

不破坏任何现有 impl；合成器 / 外部插件返回 `None`（VST3 没有统一的 GR 查询接口）。

### 7.4 主总线插入链

- `Project.master_effects: Vec<PluginInstance>`（§6）。
- **顺序**：所有轨道加总 → master 插入链 → master 推子 / 声像。选「推子前」与 Cubase /
  Pro Tools 的 master 段一致：推子始终是最后的整体电平，插入看到的是未衰减的汇总信号。
  （要反过来也只是一行代码，见 §13 开放问题 1。）
- 命令：`AddMasterEffect / RemoveMasterEffect / MoveMasterEffect / SetMasterEffectParam /
  SetMasterEffectEnabled`；事件 `MasterEffectsChanged` 是**结构性**的（重建），参数与旁通是
  **mix-only**（不重建）。
- 运行时命令新增 `SetMasterEffectParam { effect, param, value }` /
  `SetMasterEffectEnabled { effect, enabled }`——不复用 `SetParam { track }` 的哨兵 id，避免
  「魔法 TrackId(0)」。
- 单测：主总线插入链的输出 == 离线渲染同一工程的结果（引擎本来就是同一套 `Player`，这条是回归）。

### 7.5 实时限幅器

- 插件 id `builtin.limiter`：前瞻 ~5 ms、4× 过采样真峰值检测、瞬时启动 / ~50 ms 释放、
  参数 `ceiling`（-12…0 dBTP）与 `release`。
- **复用而非重写**：把 `crates/mastering/src/limiter.rs` 的算法抽成流式 `Limiter`
  （`prepare` / `process`），放进 `crates/effects`，`mastering` 改为依赖 `ai-daw-effects` 调用
  同一实现做离线批量处理。依赖方向无环（`effects` 只依赖 `plugin-api`）。
- 延迟约 5 ms（240 样本 @48k）。放主总线无需补偿；**放轨道插入会让那一轨整体后移**，M3 有 PDC
  之前，UI 上给一句提示（不禁止）。
- 默认**不加**任何隐式限幅：用户不加就没有。沿用既有原则——不做用户没有要求的处理。

### 7.6 独奏语义修正

现状 `want = !mute && (!any_solo || solo)`（`player.rs:940`）：solo 一条人声，人声的混响总线
一起哑，等于 solo 听不到空间。

改为：先算「必须发声的集合」= 所有 solo 轨 ∪ 它们在路由图上的**传递下游**（send target 与
M3 的 output target）。总线的 `audible` 由「是否在集合里」决定，而不是由「自己有没有 solo」决定。

回归测试：solo 源轨 → 总线仍有输出；取消 solo → 恢复原状；不影响既有
`solo_mutes_every_other_track`。

### 7.7 界面

- **整页混音台**（`src/views/MixView.vue`）通道条从上到下补三块：
  1. 表头（L/R 双条 + 顶部削波点 + 刻度只画 -48 / -24 / -12 / -6 / 0）；
  2. 插入槽（折叠，最多显示 4 个 + 「…」；点开进检查器）；
  3. 发送槽（M1 只读展示「目标 + 增益」，M3 才可编辑）。
- **右面板通道条**（`src/components/workspace/panes/MixerPanel.vue`）：加小尺寸表 + 主输出
  插入列表（与整页共用同一份数据与组件）。
- 主输出条：表 + 插入槽 + 推子。
- 新增组件 `components/common/ChannelMeter.vue`（纯展示，吃 `MeterValue`）与
  `components/common/InsertSlot.vue`（旁通开关 + 名称 + 打开参数）。
- 轮询收在 store（`stores/project.ts` 或新建 `stores/meters.ts`），页面卸载 / 隐藏时停表。

## 8 M2：侧链

### 8.1 ABI

```rust
// plugin-api::Plugin，全部带默认实现
fn sidechain_channels(&self) -> u8 { 0 }        // 0 = 不支持；>0 = 键输入声道数

fn process_with_sidechain(
    &mut self,
    ctx: &ProcessContext,
    audio: &mut [&mut [f32]],
    events: &[TimedMidiEvent],
    key: Option<&[&[f32]]>,
) {
    let _ = key;
    self.process(ctx, audio, events);           // 默认忽略键信号
}
```

引擎对**效果器**调 `process_with_sidechain`，对乐器仍调 `process`。默认实现保证：所有既有
插件（含不支持的第三方）行为不变。

### 8.2 模型与命令

```rust
pub struct SidechainSource { pub track: TrackId }
// PluginInstance 增：
pub sidechain: Option<SidechainSource>,
```

- 只对**效果链槽位**有意义；乐器位上出现就在命令层丢弃。
- 命令 `SetEffectSidechain { track, effect, source: Option<TrackId> }`（整体替换，可撤销）；
  事件 `EffectSidechainChanged` 是**结构性**的——它改变路由图，必须重建。
- 归一化：自己给自己、目标轨道不存在 → 丢弃（与发送的「自发送丢弃」同一套）。**目标插件暂不
  支持侧链时不丢弃**——用户换了插件就生效，运行期按「哑侧链」处理。

### 8.3 调度与键缓冲（`crates/engine/src/schedule.rs`）

- `ScheduledEffect { instance, enabled, key: Option<KeyTap { source_index }>, sidechain_channels }`。
- 键缓冲：`Vec<Vec<f32>>`，每个带 key 的效果一条（`2 * max_block`），**构建期**分配。
- 路由边：在既有 `edges`（发送）之外，追加 `source → dest` 的侧链边，一并进 `routing_order`。
  成环时 `routing_order` 会忽略环上的边——那几条 key 缓冲必须清零（哑侧链），不能留上一块的
  残留信号。
- 处理：
  1. 源轨推子后 → 写进每条以它为源的 key 缓冲（与发送拷贝同一处，一次循环）；
  2. 目的轨效果链走到带 key 的槽时传 `Some(&key_buf)`。
- 键信号是推子后：跟随源轨的**推子 / 静音 / 独奏**。文档与 UI 提示都写明（「推子拉到 -∞ 就没有键」）。

### 8.4 宿主适配

| 格式 | 做法 | 风险 |
| --- | --- | --- |
| 内置 | 压缩器 `sidechain_channels() = 2`，检测器用键信号、增益作用在主信号 | 低 |
| VST3 | 切到 bus API：`audio_bus_layout()` 读输入总线；有第二输入总线且为侧链型（`kAux`）时 `set_bus_arrangements` + `set_bus_active`，每块 `BusAudioBuffers` 写 bus0 = 主信号、bus1 = 键信号；`process_bus_audio` | **中高**：适配器要整体换通路，KR-106 / 编辑器 / 进程隔离测试必须全绿 |
| CLAP | `AudioPorts::with_capacity(2, 2)`（2 声道 × 2 端口），按插件声明的输入端口数决定；键信号进 port 1 | 中：`clack-host` 的端口 API 需要探针确认 |
| AU | 不做。适配器的 `InputBuffer` 只有一块立体声输入，pull block 忽略 `bus` 参数 | — |

**建议**：VST3 的 bus 切换单独一个提交 + 一次探针实验（用一个真侧链压缩器验证 bus 索引与
`kAux` 类型），确认后再动 `vst3.rs` 的主路径。探针实验的结论写回本稿 §13。

### 8.5 界面

- 检查器 / 混音台的插入槽：插件支持侧链时出现 `SC` 徽标；点开是「侧链来源：无 / 轨 A / 轨 B…」
  下拉（列出除自身外的所有轨道）。
- 不支持侧链的插件：入口**可见但禁用**，tooltip 说明原因（与参数页「可见但禁用」的既有约定一致）。
- 选中后插入槽上的 `SC` 徽标点亮；M2 不画连线（画线是纯装饰，留给后续）。
- 工程里存了 sidechain 但插件缺失 / 不支持 → 忽略，不阻塞播放（与 `MissingPlugin` 的降级一致）。

## 9 M3 边界（只划范围，需独立 spec）

1. **发送增强**：`Send.mode: Pre | Post`、`Send.mute`；预推子取点在效果链之后、推子之前。
2. **直接输出**：`Track.output: Option<TrackId>`（None = master），让「只进总线不进主输出」
   成为可能；路由边、环检测、solo 下游一并扩展。
3. **PDC**：`Plugin::latency_samples() -> u32`（VST3 `get_latency_samples`、CLAP `latency`）；
   每轨补偿延迟 = `max_latency - 本轨 latency`，补偿点在推子 / 发送取点**之前**，保证直连与
   发送两条路径对齐。延迟只在结构性变更时重算（参数导致的延迟变化不追，文档写明）。
4. **片段增益 / 淡入淡出**：`Clip.gain_db`、`AudioSource.fade_in / fade_out`（拍）。
5. **编组 / VCA / solo-safe、通道条增强（相位反转、输入 trim、pan law）、自动化扩展、导出
   位深与抖动**——各自独立 spec。

## 10 实时安全契约

新增的音频线程工作只允许三种操作：原子读写、预分配缓冲的拷贝、纯算术。

- 计量：`fetch_max` / `fetch_or`，不 `format!`、不 `Mutex`。
- 压缩器 / 限幅器：`prepare` 里算系数、`process` 里只做乘加；限幅器的过采样状态在 `prepare`
  分配。
- 侧链：key 缓冲在构建期分配（`2 * max_block`），`process` 里只做 `copy_from_slice`。
- `realtime_safety.rs` 扩展：门禁场景加**压缩器 + 限幅器 + 侧链 + 计量**；并单独断言
  `SetEffectEnabled` / `SetMasterEffectParam` / `SetMasterEffectEnabled` 的 `apply_command`
  零分配。（`SetEffectSidechain` 是结构性变更，走重建，不在音频线程上应用。）

## 11 验收标准

**M1**

1. `mixer_meters` 在播放时返回非零峰值；停止后归零；削波能锁存并清零。
2. 单测：推子 -6 dB → 表头读数 ≈ -6 dBFS；静音轨表头为 0。
3. 压缩器：-20 dBFS 输入、阈值 -30 dB、比 4:1、knee 0、无 makeup → 稳态输出 ≈ **-27.5 dBFS**
   （容差 0.5 dB）；`gain_reduction_db()` 与此一致。
4. 限幅器：0 dBFS 素材 + ceiling -1 → 真峰值 ≤ -0.8 dBTP（复用 `mastering` 的采样间峰值用例）。
5. 旁通：在**同一个 `Player` 实例**上连发 `SetEffectEnabled`，前后两块输出不同、且都没有零块
   （证明不重建、不断音）。
6. 主总线插入：`master_effects` 为空时导出与现状**逐位一致**（回归基线）。
7. solo 源轨 → 总线可听；既有 solo 测试全绿。
8. `npm run build` + `cargo test -p ai-daw-engine --features realtime` 全绿。

**M2**

1. 内置压缩器接侧链：源轨静音 → GR 回到 0（键没了）；源轨出声 → GR 跟随，主信号输出仍受控。
2. 侧链成环：A 的压缩器键取 B、B 的键取 A → 不崩、不挂死，两者都当哑侧链（GR 恒 0）。
3. 命令层：自侧链、悬空轨道、乐器位来源一律丢弃；`SetEffectSidechain` 可撤销。
4. 旧工程（无 `sidechain` 字段）加载后行为不变；wire-shape 测试通过。
5. VST3 侧链探针：至少一个真侧链压缩器能收到键信号（手动验证，结论回写本稿）。
6. 不支持侧链的插件：UI 入口禁用；即使工程里被写入了 `sidechain` 也不影响播放。

## 12 风险

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 计量写路径引入分配或锁 | 音频线程咔哒 / 门禁失败 | 只用原子；门禁纳入计量场景 |
| VST3 切 bus API 回归 | 外部乐器 / 编辑器 / 进程隔离全挂 | 单独提交 + 探针先行 + 既有 KR-106 测试为准入 |
| 限幅器 4× 过采样 CPU | 主总线 CPU 上升 | 只在主总线默认不开；实测 > 3% 再降过采样阶数 |
| 主总线插入改变导出基线 | 「同一工程两次渲染逐位一致」被破坏 | 空链时短路（不进任何处理），并用基线测试锁住 |
| 侧链路由边让拓扑排序出现环 | 死循环 / 信号错位 | `routing_order` 已能忽略环；key 缓冲额外清零 |
| 压缩器 0 延迟听感偏「紧」 | 用户觉得不如第三方 | 文档说明；lookahead 作为 M3（依赖 PDC）的可选项 |
| 表头 30 Hz 轮询的 IPC 频率 | 主线程抖动 | 单帧 7 KB、页面隐藏即停；必要时降 20 Hz |

## 13 开放问题（评审要拍的）

1. **主总线插入在推子前还是推子后？** 本稿选「前」（与 Cubase / Pro Tools 一致）。若希望
   「推子后限幅」更符合流媒体交付直觉，改 `player.rs` 一行。
2. **表头取点是推子后还是可切预推子？** 本稿固定推子后（所见即所听）。预推子作为 M3 的开关。
3. **VST3 侧链的 bus 索引与类型**：是 `kAux` 还是第二 `kMain`？需要在 M2 开工前用真插件探针
   验证，结论回写 §8.4。
4. **压缩器要不要 lookahead**：要就得先有 PDC。本稿按「不要」设计。
5. **一轨多个动态处理器时的 GR 显示**：本稿取最大值。要不要改成「逐个槽位单独显示」——
   那需要 per-effect 计量通道，成本翻倍。

## 14 实现记录（2026-11-01）

### 已落地

| 项 | 位置 | 备注 |
| --- | --- | --- |
| 通道电平表（每轨 + 主输出、削波锁存、GR） | `crates/engine/src/meter.rs` | 固定 256 槽；`fetch_max` 位模式；读侧 `swap(0)` |
| 前端弹道 / 峰值保持 / 削波显示 | `src/stores/meters.ts` + `components/common/ChannelMeter.vue` | 30 Hz 轮询；页面隐藏即停 |
| 效果器旁通（不重建、不断音） | `PluginInstance.enabled` + `SetEffectEnabled` | 混音类事件，走 `TransportCommand` |
| 内置压缩器（0 延迟、软拐点、立体声联动、可接侧链） | `crates/effects/src/compressor.rs` | 7 个参数；GR 经 `gain_reduction_db()` 进表 |
| 内置限幅器（5 ms 前瞻、采样间峰值、释放平滑） | `crates/effects/src/limiter.rs` | 建议挂主总线；挂轨道插入会整体后移（等 PDC） |
| 主总线插入链 | `Project.master_effects` + `player.rs` 主总线段 | 顺序：总加 → 插入 → 推子；空链逐位等价于旧行为 |
| 侧链（内置压缩器） | `sidechain_channels()` / `process_with_sidechain` / `effect_keys` / 键缓冲 | 键信号推子后；自侧链在命令层丢弃 |
| 发送 pre/post + 哑音 | `SendMode` / `Send.mute` | 预推子取点在效果链之后、推子之前 |
| 独奏语义修正 | `player.rs::refresh_mix` | solo 轨 + 其传递下游（总线）保持可听 |
| 混音台重做 | `src/views/MixView.vue` | 固定 112 px 通道条；名字省略号（修掉「长名字撑宽推子」） |
| 右面板 | `MixerPanel.vue` | 表 / 插入（旁通 + 侧链）/ 发送（pre-post + 静音）/ 主总线插入（就地调参） |
| 检查器 | `InspectorPanel.vue` | 效果行的旁通开关 + 侧链来源下拉 |

### 按计划推迟

- **外部插件（VST3 / CLAP / AU）的侧链**：适配器仍返回 `sidechain_channels() == 0`。
  `vst3-host 0.9` 的 bus API 可用，但切换通路要单独提交并先用真侧链压缩器做探针（§8.4）。
- **PDC**：`Plugin` 仍无 `latency_samples()`。因此限幅器挂轨道插入会让该轨后移——UI 不禁止，
  只建议挂主总线。
- **M3 其余项**：直接输出、编组 / VCA / solo-safe、片段增益与淡化、自动化扩展。
- **压缩器 lookahead**：依赖 PDC，未做。
- **GR 粒度**：每轨一个读数（取链上最大值），不区分槽位。

### 验证

- `cargo test -p ai-daw-engine --features realtime`：256 个单测 + 6 组集成测试全绿。
  `realtime_safety.rs` 的门禁场景已加入**压缩器 / 限幅器 / 侧链 / 计量**，并新增
  `SetEffectEnabled` 与主总线参数 / 旁通的零分配断言。
- `cargo test -p ai-daw-ipc`：51 项，含「新字段不改变旧 JSON 形状」的 wire-shape 用例。
- `cargo test -p ai-daw-effects`：32 项（压缩器静态曲线 -27.5 dBFS、侧链压 12 dB、
  限幅器守住 -1 dBFS）。
- `cargo test -p ai-daw-mastering`：14 项不变（离线母带未改）。
- `npm run build`（vue-tsc + vite）通过。
