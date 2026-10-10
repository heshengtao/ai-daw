# 动态 BPM 与动态节拍（速度/拍号关键点）— 设计文档

- 日期：2026-10-08
- 状态：**已完成（2026-10-10 校准）**（关键点模型 / `TempoMap` / 分段网格 / 标尺 / 底栏全部落地；仅「关键点拖动」按 §2 非目标未做）
- 主题：时间轴上的「关键点」同时携带 **BPM** 与 **拍号**；关键点从自身位置起生效。含引擎分段计时与分段的网格/标尺

## 1. 背景

当前整首曲子只有**一个**速度与**一个**拍号：

- 引擎：`Transport { tempo, time_sig_num, time_sig_den }`；
- 换算：`beats_to_seconds(beats, tempo) = beats × 60 / tempo`（`crates/engine/src/lib.rs`），被 `schedule.rs`（拍→帧）、`render.rs`（离线时长）使用；
- 前端：`stores/transport.ts` 用同一个公式做反向换算 `positionBeats = frames/sr × tempo/60`，`beatsToFrames` 用正向；
- 网格/标尺：`stores/timeline.ts` 由这**唯一**拍号推导。

用户需求：在某时刻调整 BPM（或拍号）后，**该点之后**都按新值走；在时间轴的**关键点**处调整，只影响关键点之后的部分，其它位置调整则是整首。

## 2. 目标与非目标

**目标**

- 时间轴支持**关键点**：双击标尺添加、右键关键点删除。
- 关键点同时携带 **BPM + 拍号**；从关键点位置起生效，直到下一个关键点。
- 修改规则（用户确认：**必须对准关键点**）：
  - 播放头**正好落在某个关键点上** → 改该关键点（从它之后生效）；
  - **其它位置** → 改「基准值」（第 0 拍，也就是整首的默认值）。
- 标尺在 BPM（或拍号）变化的位置显示该处的 **BPM 数值**（拍号变化时一并显示）。
- 播放/渲染/播放头/定位全部按分段正确计时。
- 网格与标尺**分段感知**：不同段落可有不同拍号，小节线与格子随之变化。

**非目标**

- 不做速度/拍号的自动化曲线（渐变）——关键点之间是**分段常量**。
- 不改音符/片段的存储与时值语义（它们始终以「拍」为单位）。
- 不做关键点的拖动（本期只支持「添加 / 编辑 / 删除」；拖动属于后续）。
- 不动合成器界面、插件 ABI。

## 3. 术语与模型

### 3.1 数据模型

`Transport` 增加关键点列表（第 0 拍不用放进列表——它由现有字段承载，保证向后兼容）：

```rust
pub struct Transport {
    pub tempo: f64,          // 第 0 拍（基准）BPM
    pub time_sig_num: u8,    // 第 0 拍（基准）拍号
    pub time_sig_den: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,   // 按 beat 升序，beat > 0
}

pub struct Keyframe {
    pub beat: f64,
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
}
```

**不变量**（在 `apply` 里强制）：`beat` 有限且 `> 0`；按 `beat` 升序去重（同拍覆盖）；`tempo` 夹到 `1..=1000`；拍号经 `Transport::normalize_time_sig` 归一化。

### 3.2 取值与换算

新增 `crates/engine/src/tempo.rs`：

```rust
pub struct Point { pub start_beat: f64, pub tempo: f64, pub num: u8, pub den: u8 }
pub struct TempoMap { points: Vec<Point> }   // 分段常量；points[0].start_beat == 0

impl TempoMap {
    pub fn from_transport(t: &Transport) -> Self;    // 基准段 + 关键点
    pub fn tempo_at(&self, beat: f64) -> f64;        // 生效 BPM
    pub fn meter_at(&self, beat: f64) -> (u8, u8);
    pub fn beats_to_seconds(&self, beats: f64) -> f64;   // 分段积分
    pub fn seconds_to_beats(&self, secs: f64) -> f64;    // 分段反解
    pub fn points(&self) -> &[Point];
}
```

- `beats_to_seconds`：逐段累加 `(min(beats, seg_end) - seg_start) × 60 / bpm`。
- `seconds_to_beats`：逐段累加 `(min(secs, seg_end_s) - seg_start_s) × bpm / 60`。
- 无关键点时，退化为 `beats × 60 / tempo`，**与现状逐位一致**。

`crates/engine/src/lib.rs` 的常量版 `beats_to_seconds(beats, tempo)` **保留**（常量场景/既有测试用）；真正的时间轴换算改用 `TempoMap`。

### 3.3 前端镜像

新增 `src/utils/tempo.ts`，纯函数、与 Rust 侧同一套分段算法（必须逐位对得上）：

```ts
export interface Segment { startBeat: number; endBeat: number; tempo: number; num: number; den: number }
export function segments(t: Transport): Segment[];
export function tempoAt(beat: number, t: Transport): number;
export function meterAt(beat: number, t: Transport): { num: number; den: number };
export function beatsToSeconds(beats: number, t: Transport): number;
export function secondsToBeats(secs: number, t: Transport): number;
export function barBeatsAt(beat: number, t: Transport): number;   // = num × 4/den
export function barBeatAt(beat: number, t: Transport): { bar: number; beatInBar: number };
```

## 4. 引擎改动

### 4.1 事件与命令

`command.rs` 新增：

```rust
/// 在第 beat 拍插入/更新关键点（同时带 BPM 与拍号）。
SetKeyframe { beat: f64, tempo: f64, num: u8, den: u8 },
/// 删除第 beat 拍的关键点。
RemoveKeyframe { beat: f64 },
```

`SetTempo(f64)` / `SetTimeSignature{num,den}` **保留**（改基准值，第 0 拍）。

`event.rs` 新增（都**不是** `is_mix_only` → 会正常重建定帧事件表）：

```rust
KeyframeSet { actor: Actor, beat: f64, tempo: f64, num: u8, den: u8 },
KeyframeRemoved { actor: Actor, beat: f64 },
```

同步更新 `Event::actor()` 的 `match`。

`apply.rs`：`KeyframeSet` 归一化后 upsert 进 `transport.keyframes`（保持有序、去重、`beat > 0`）；`KeyframeRemoved` 按 `beat`（浮点相等/容差）移除。

`bus.rs::translate`：`SetKeyframe` → 归一化 → `Event::KeyframeSet`；`RemoveKeyframe` → `Event::KeyframeRemoved`。

### 4.2 定帧与播放

- `schedule.rs`：`ScheduledEvents::from_project` 构建 `TempoMap`，`to_frame` 改用它；`ScheduledEvents` 增加 `tempo_points: Vec<(u64 frame, f64 bpm)>`（供播放器逐块取当前速度）。
- `player.rs`：`Player` 持有 `tempo_points` + 游标；每个块开头按 `position` 推进游标，取当前 BPM 填进 `ProcessContext.tempo`（实时安全：只读游标、不分配）。`playing` 停在结尾的判断、`seek` 等逻辑不变。
- `render.rs`：`total_frames` 用 `TempoMap::beats_to_seconds(project.length_beats())`。

### 4.3 Tauri 命令

- `set_keyframe(beat, tempo, num, den) -> Project`、`remove_keyframe(beat) -> Project`，并在 `lib.rs` 的 `generate_handler!` 注册。

## 5. 前端改动

### 5.1 类型与 API

- `src/api/types.ts`：`Transport` 增加 `keyframes: Keyframe[]`；新增 `Keyframe { beat; tempo; time_sig_num; time_sig_den }`。
- `src/api/tauri.ts`、`src/api/mock.ts`：新增 `setKeyframe` / `removeKeyframe`，mock 侧同样维护 `transport.keyframes`（保持预览与桌面行为一致）。

### 5.2 传输与定位（`stores/transport.ts`）

- `positionBeats` = `secondsToBeats(positionFrames / effectiveSampleRate, transport)`。
- `beatsToFrames(beats)` = `beatsToSeconds(beats, transport) × effectiveSampleRate`。
- 这样播放中经过关键点时，播放头速度自然变快/变慢。

### 5.3 关键点感知的编辑（`stores/project.ts`）

新增：

```ts
keyframeAt(beat: number): Keyframe | null;   // |beat - kf.beat| < snap/2 视为「对准」
setKeyframeAt(beat: number, v: { tempo?; num?; den? }): Promise<void>;  // upsert（继承未给的值）
removeKeyframeAt(beat: number): Promise<void>;
applyTempoAt(beat: number, bpm: number): Promise<void>;      // 对准→改关键点；否则→setTempo（基准）
applyMeterAt(beat: number, num: number, den: number): Promise<void>;  // 同上
```

「对准」判定用半格容差（`|beat − kf.beat| < snapAt(beat)/2`）：标尺定位/双击都吸附到网格，因此基本是精确命中；播放暂停在中途时也不会误判。

### 5.4 底栏（`components/layout/BottomBar.vue`）

- BPM 输入框显示**播放头处的生效 BPM**（`tempoAt(positionBeats)`），播放头移动时随之更新；改动调用 `applyTempoAt(positionBeats, bpm)`。
- 拍号控件（`TimeSignatureField`）显示**播放头处的生效拍号**；改动调用 `applyMeterAt(...)`。
- 当播放头**正好在关键点上**时，显示一个「关键点」小徽标——让用户一眼看出这次的修改只影响该点之后。

### 5.5 标尺（`components/workspace/TimelineRuler.vue`）

- **分段小节号**：不再用固定 `barPx` 平铺，而是遍历拍号分段，逐段生成小节刻度（段内宽度 = `barBeats(段) × zoom`）。
- **关键点标记**：在每个关键点位置渲染一个徽标，显示该处的 BPM（如 `140`）；若拍号也变了，再显示一行 `3/4`。
- **交互**：
  - 双击标尺空白 → 在该（吸附后的）拍添加关键点，初值继承此刻的生效 BPM/拍号；
  - 右键关键点徽标 → 删除该关键点；
  - 单击/拖动徽标 → 定位播放头（与现有标尺行为一致）。
- 该组件被编排与编辑两个视图共用，因此两处都能看到关键点。

### 5.6 网格（`stores/timeline.ts`）——分段感知

- 新增按拍号分段计算：`segments`（`{ startBeat, endBeat, meterBeat, barBeats }`），`barBeatsAt(beat)`、`snapAt(beat) = meterBeat(beat)/4`。
- `gridBackground` 改为**按段叠层**：每段贡献两层（拍线格 + 小节线），用
  `linear-gradient(...) no-repeat; background-size: 段宽px 100%; background-position-x: 段起点px`
  把 `repeating-linear-gradient` 限制在该段范围内：
  - 无关键点 → 只有一段 → 等价于现状（同时也落实第一份 spec 的「每拍一格」）。
  - 有关键点 → 段数 = 关键点数 + 1，层数很少，不会 DOM 爆炸。
- `snapBeats(beat)` 用 `snapAt(beat)`。
- 小节刻度数据（`barMarks`）按段生成，供标尺使用。
- `contentBars` / `totalBeats` 仍以拍为单位统计。

### 5.7 受影响的调用方

- `EditView.vue`：`beatsPerBar` → `barBeatsAt(clipStart)`；`gridLabel` → 用 `snapAt(clipStart)`；未指定拍号段落时用其所在段的拍号。
- `BottomBar.vue`：`bars` 与 `positionText` 改用分段小节编号（替换当前「只按分子除」的实现）。
- `ControllerLane.vue` / `ArrangeView.vue`：继续用 `timeline.gridBackground`，无需改。

## 6. 兼容与约束

- 老工程没有 `keyframes` 字段 → 反序列化为空 → 行为与现在**逐位一致**。
- 关键点列表为空时，`TempoMap` 退化为常量换算，`render`/`schedule` 输出不变（加回归测试）。
- 音符、片段的起点/时长仍以「拍」为单位，不随 BPM 变化；只有**拍→秒/帧**的换算变。
- `keyframes` 序列化用 snake_case、`beat`/`tempo` 为 f64（与现有 `transport` 字段风格一致）。

## 7. 验收标准

1. 双击标尺某处出现关键点徽标（显示 BPM）；右键徽标可删除。
2. 播放头对准关键点时改 BPM/拍号 → 只影响该点之后；不对准时 → 改整首基准值（底栏有「关键点」提示）。
3. 播放经过关键点后速度真的改变（播放头推进变快/变慢），定位（拍→位置）也按分段正确。
4. 有拍号关键点时，标尺小节线/网格在前后的格子数分别等于各自段的分子；无关键点时与第一份 spec 的「每拍一格」一致。
5. 底栏位置读数在分段拍号下与标尺小节线对齐。
6. 老工程（无关键点）加载后听感与网格完全不变。

## 8. 验证方式

- Rust：`cargo test -p ai-daw-engine`
  - `TempoMap` 单测：常量段 = 旧公式；分段 `beats_to_seconds` / `seconds_to_beats` 互逆；关键点处取值切换；`beat<=0` 的关键点被拒。
  - `schedule` / `render`：加一个关键点后帧位置随时长变化；无关键点时输出与旧版逐位一致。
  - `apply`：关键点归一化（夹紧/有序/去重）、越界与非法值被拒。
- 前端：`npm run build`（`vue-tsc`）通过；`src/utils/tempo.ts` 与 Rust 侧的对拍用例（同一组输入产出同一结果）。
- 手动（浏览器预览 + 桌面端）：双击加关键点、右键删除、对准/不对准改值的行为、网格分段、播放变速。

## 9. 风险

- **分段网格**是本特性最大的一块：用「分段叠层背景」避免逐线 DOM；若段数极多（几十个关键点）层数仍可控。
- **「对准关键点」的判定容差**若太小，用户会觉得「改了没反应」；采用半格容差并在底栏给出「关键点」提示来降低困惑。
- **与第一份 spec 的关系**：第一份 spec 的「每拍一格」是「无拍号关键点」的特例；本 spec 把它推广为分段。实现时先落地「单拍号、每拍一格」，再引入分段，可降低风险。
