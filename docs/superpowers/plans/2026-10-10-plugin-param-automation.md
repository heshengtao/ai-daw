# 插件参数自动化（含自适应参数车道）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal：** 让自动化能作用到**任意插件参数**（乐器 + 效果器），并把钢琴卷帘下方的控制器泳道改成**按插件自适应的参数车道**。

**Architecture：** 自动化目标沿用 `Track.automation[].param` 这个字符串键并加命名空间（`gain_db` / `pan` / `inst:<id>` / `fx<i>:<id>`）；引擎在**控制侧**把键解析成 `LaneTarget`，`TransportCommand::SetAutomation` 携带已解析的目标，`Player::refresh_mix` **逐块**对插件实例 `set_param`——音频线程不做分配。前端把曲线画布抽成 `AutomationCurve`，编排的自动化车道与卷帘下方的参数车道共用它。

**Tech Stack：** Rust 2021、serde、`crates/ipc` 契约、Tauri、Vue 3 + Pinia。

设计见 `../specs/2026-10-10-plugin-param-automation-design.md`。

## 执行记录（2026-10-10）

6 个任务全部完成，分支 `feat/plugin-param-automation`。实测数字：

- **引擎** `cargo test -p ai-daw-engine --features realtime` → **162 通过，0 失败**
  （unittests 149 + `audio_clip` 7 + `builtin_effects` 3 + `input_capture_safety` 1 +
  `realtime_safety` 1 + `render_to_wav` 1）；`realtime_safety.rs` 的零分配门禁已覆盖插件参数车道。
- **前端** `npm run build`（vue-tsc + vite）通过；528 模块，无类型错误。
- **i18n** 中英键数 **657 / 657**，两侧键集合相等（删 `lane.volume` / `lane.volumeHint`，
  增 `auto.group.note`）。
- 说明：`Track` / `AutomationLane` / `AutomationPoint` 模型与 `Command` / `Event` 字段未改，
  MCP 工具表仍 33 个（本分支未增删工具）；旧工程零迁移。
- **浏览器端到端验证**（编排者执行；单页内用 hash 导航，因为 `agent-browser open` 会重置 mock 状态）：
  编排的 `inst:cutoff` 曲线与卷帘下方车道读到的是**同一个点**（同键同数据）、在卷帘车道里拖动会写回编排、
  效果器分组出现（延迟的 时间/反馈/混合）、下拉里**没有「音量」**、有音符时「力度」仍是逐音符条、
  曲线原点与编排网格严格对齐（inset 26 / 0）、顶栏底部窗格开关在混音 / 设置页为禁用。
- **最终全分支评审**（最强模型，8 个提交）→「With fixes」：两处 Important 已在本分支修掉——
  ① 移除插件参数曲线时把参数写回静态值（读取模式，`e0c98f5`）；② `mcp.rs` 的 `set_automation`
  描述补上 `inst:` / `fx<i>:` 形式（`2e8ce2a`）；顺带把零分配门禁扩到 `fx<i>:` 车道。
  修复后的一次范围化复评：**全部 ADDRESSED，无新的 Critical / Important**。

### 遗留（评审分诊：均可随本次发布）

- 自动化点上限 4096；仅线性曲线；无 pre-fader send；曲线画布未做纵向值域缩放。
- 效果器按**槽位下标**寻址：重排效果链后曲线跟槽位走（v1 明说的取舍，见设计 §9）。
- 插件参数「恢复静态值」有一个**窄边界**：若在曲线活跃期间发生结构性重建，且该参数既不在工程的
  `params` 里、也未被插件 `state` 复现（主要是外部插件），重建会把当前值当作静态值快照下来。
  彻底消除需要把自动化前的值持久化进工程 / 调度表（v1 不做，见设计 §9）。
- 零分配门禁测到的是 `SetAutomation` 的**添加 / 替换**路径，未单独测「移除并写回」与「添加时
  `get_param`」两个分支（经检视两者均不分配）。
- `laneValueAtOrNull` / `automationTargetsFor` 无自动化测试（本仓库没有前端测试运行器）。
- 文案：`ControllerLane` 仍用 `lane.property`（「控制器」）、`topbar.bottomLane*` 仍写 "controller"
  ——该窗格现在是**参数车道**，措辞待改。
- 杂项：`EditView.applyControllerCurve` 用 `selection.trackId` 而文件其余处用局部 `trackId`；
  `paramLabel` 重复算 `canonicalPluginId`；内置**效果器**参数名在英文界面仍显示中文；
  目标不存在时分组编号会跳号（例如只有「效果器 2」）；零轨工程的自动化车道只剩 30px 表头。

## Global Constraints

- 许可 Apache-2.0-only；**不新增依赖**。
- 引擎 `#![forbid(unsafe_code)]`（`crates/host-vst3` 例外，本计划不碰它）。
- **实时回调零分配**：`crates/engine/tests/realtime_safety.rs` 是硬门禁，必须保持全绿。
- **键在控制侧解析**；音频线程只做移动与写入，绝不构造 `ParamId`。
- 键格式（唯一定义）：`gain_db`、`pan`、`inst:<param_id>`、`fx<i>:<param_id>`（`i` 从 **0** 起）。
- 本计划**不改** `Track` / `AutomationLane` / `AutomationPoint` 模型，也不改 `Command` / `Event` 的字段
  （`param` 仍是 `String`）——旧工程零迁移。
- 每个 Task 末尾全绿；涉及 src-tauri 跑 `cargo check --manifest-path src-tauri/Cargo.toml`；
  前端跑 `npm run build` 且 i18n 中英键数一致。

---

## Task 1: 引擎 —— `LaneTarget` 取代 `LaneParam`（行为不变）

把「封闭的两值枚举」换成「可扩展的目标」，并把键解析从音频线程挪到控制侧。**本任务结束时
`gain_db` / `pan` 的行为必须逐位不变**，插件参数的应用代码在此落地（Task 2 才有测试覆盖它）。

**Files:**
- Modify: `crates/engine/src/schedule.rs`
- Modify: `crates/engine/src/player.rs`
- Modify: `crates/engine/src/transport.rs`
- Modify: `src-tauri/src/audio.rs`

**Interfaces:**
- Consumes: 无（起始任务）。
- Produces:
  - `ai_daw_engine::schedule::LaneTarget`（`#[derive(Debug, Clone, PartialEq)]`，
    变体 `GainDb` / `Pan` / `Instrument(ParamId)` / `Effect { index: usize, param: ParamId }`）；
  - `LaneTarget::from_key(&str) -> Option<LaneTarget>`；
  - `ScheduledLane { param: LaneTarget, points: Vec<AutomationPoint> }`；
  - `TransportCommand::SetAutomation { track, target: LaneTarget, points }`。

- [ ] **Step 1: 写失败测试（键解析）**

在 `crates/engine/src/schedule.rs` 的 `mod tests` 顶部加 `use ai_daw_plugin_api::ParamId;`，并追加：

```rust
    #[test]
    fn lane_target_from_key_parses_every_form() {
        assert_eq!(LaneTarget::from_key("gain_db"), Some(LaneTarget::GainDb));
        assert_eq!(LaneTarget::from_key("pan"), Some(LaneTarget::Pan));
        assert_eq!(
            LaneTarget::from_key("inst:cutoff"),
            Some(LaneTarget::Instrument(ParamId("cutoff".into())))
        );
        assert_eq!(
            LaneTarget::from_key("fx2:mix"),
            Some(LaneTarget::Effect { index: 2, param: ParamId("mix".into()) })
        );
        // 不认识 / 残缺的形式
        assert_eq!(LaneTarget::from_key(""), None);
        assert_eq!(LaneTarget::from_key("nope"), None);
        assert_eq!(LaneTarget::from_key("inst:"), None);
        assert_eq!(LaneTarget::from_key("fx:mix"), None, "下标必须是数字");
        assert_eq!(LaneTarget::from_key("fx1:"), None, "参数名不能为空");
        assert_eq!(LaneTarget::from_key("fx"), None);
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ai-daw-engine lane_target_from_key_parses_every_form`
Expected: 编译失败（`LaneTarget` 不存在）。

- [ ] **Step 3: 在 `schedule.rs` 用 `LaneTarget` 取代 `LaneParam`**

把 `use ai_daw_plugin_api::MidiEvent;` 改成 `use ai_daw_plugin_api::{MidiEvent, ParamId};`，
并把整个 `LaneParam` 定义（含 `impl`）替换为：

```rust
/// 一条自动化车道的**已解析目标**。
///
/// 键的形式（见 `specs/2026-10-10-plugin-param-automation-design.md` §4）：
/// `gain_db` / `pan` / `inst:<param_id>` / `fx<i>:<param_id>`（`i` 从 0 起）。
///
/// 非 `Copy`：`ParamId` 拥有自己的 `String`。**构造只在控制侧 / 调度构建时发生**
/// （`from_key` 会分配），解析好的目标随 `ScheduledLane` 带进播放器，音频线程只做移动。
#[derive(Debug, Clone, PartialEq)]
pub enum LaneTarget {
    /// 轨道推子增益（dB）。
    GainDb,
    /// 轨道声像（-1..=1）。
    Pan,
    /// 轨道的乐器参数。
    Instrument(ParamId),
    /// 效果链第 `index` 个效果器的参数。
    Effect { index: usize, param: ParamId },
}

impl LaneTarget {
    /// 从键解析。**会分配**（`ParamId` 持有 `String`）——**绝不在音频线程调用**。
    pub fn from_key(key: &str) -> Option<LaneTarget> {
        match key {
            "gain_db" => return Some(LaneTarget::GainDb),
            "pan" => return Some(LaneTarget::Pan),
            _ => {}
        }
        if let Some(rest) = key.strip_prefix("inst:") {
            return (!rest.is_empty()).then(|| LaneTarget::Instrument(ParamId(rest.to_string())));
        }
        if let Some(rest) = key.strip_prefix("fx") {
            let (index, param) = rest.split_once(':')?;
            if param.is_empty() {
                return None;
            }
            return Some(LaneTarget::Effect { index: index.parse().ok()?, param: ParamId(param.to_string()) });
        }
        None
    }
}
```

把 `ScheduledLane` 的字段类型换掉（其余不动）：

```rust
pub struct ScheduledLane {
    /// 目标参数。
    pub param: LaneTarget,
    /// 关键点（拍，升序）。
    pub points: Vec<AutomationPoint>,
}
```

把 `ScheduledTrack.lanes` 的文档注释里的「已解析参数」保持原样即可（类型不用改）。

在 `from_project_with_assets` 里把车道的解析换成新函数：

```rust
            // 自动化车道：只保留能被识别的键（未知形式在求值时会被忽略，不入表）。
            let lanes: Vec<ScheduledLane> = track
                .automation
                .iter()
                .filter_map(|lane| {
                    let param = LaneTarget::from_key(&lane.param)?;
                    let points = lane.points.clone();
                    Some(ScheduledLane { param, points })
                })
                .collect();
```

把既有测试 `sends_and_lanes_are_resolved_into_the_schedule` 里的断言
`assert_eq!(s.tracks[0].lanes[0].param, LaneParam::GainDb);` 改成 `LaneTarget::GainDb`。

- [ ] **Step 4: 在 `player.rs` 里应用新目标**

`use crate::schedule::{...}` 那一行把 `LaneParam` 换成 `LaneTarget`。

`refresh_mix` 里「自动化覆盖推子」的整段替换为（**注意**：用块级解构把 `lanes` 与
`plugin` / `effects` 拆开借，否则借用检查器会拒绝；块结束时把标量拷出来，后面照旧写 `track.*`）：

```rust
        // 自动化**覆盖**推子值（读取模式）：有曲线的参数以曲线为准。
        // 拆字段借用：车道只读、插件实例可变，两者是不同字段，可以同时借。
        //
        // 覆盖只作用于**本次混音计算**：`gain_db` / `pan` 字段本身绝不能被写回——那条车道
        // 被移除（或曲线求值失败）后必须回到用户的推子值，写回会把推子值永久吃掉。
        let (mix_gain_db, mix_pan) = {
            let TrackVoice { lanes, plugin, effects, gain_db, pan, .. } = track;
            let (mut mix_gain_db, mut mix_pan) = (*gain_db, *pan);
            for lane in lanes.iter() {
                let Some(value) = lane.value_at(beats) else { continue };
                match &lane.param {
                    LaneTarget::GainDb => mix_gain_db = MixerChannel::normalize_gain_db(value),
                    LaneTarget::Pan => mix_pan = MixerChannel::normalize_pan(value),
                    LaneTarget::Instrument(param) => {
                        if let Some(p) = plugin.as_mut() {
                            p.set_param(param, value);
                        }
                    }
                    LaneTarget::Effect { index, param } => {
                        if let Some(fx) = effects.get_mut(*index) {
                            fx.set_param(param, value);
                        }
                    }
                }
            }
            (mix_gain_db, mix_pan)
        };
        let gain = db_to_linear(mix_gain_db);
        let (l, r) = pan_gains(mix_pan);
        track.left_gain = gain * l;
        track.right_gain = gain * r;
```

`apply_command` 的 `SetAutomation` 分支替换为（不再解析键、不再早退）：

```rust
            TransportCommand::SetAutomation { track, target, points } => {
                // 目标已在控制侧解析好；这里就地过滤 + 排序（`retain` / `sort_unstable_by`
                // 都不分配），再整体替换该目标的车道——音频线程因此不必扩容。
                let mut points = points;
                points.retain(|p| p.beat.is_finite() && p.value.is_finite());
                points.sort_unstable_by(|a, b| {
                    a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal)
                });
                let empty = points.is_empty();
                for voice in self.tracks.iter_mut() {
                    if voice.track_id == track {
                        voice.lanes.retain(|lane| lane.param != target);
                        if !empty {
                            voice.lanes.push(ScheduledLane {
                                param: target,
                                points: std::mem::take(&mut points),
                            });
                        }
                        break;
                    }
                }
                self.has_automation = self.tracks.iter().any(|t| !t.lanes.is_empty());
                self.refresh_mix();
            }
```

`player.rs` 的测试模块里 `t.automation = vec![AutomationLane { param: "gain_db".into(), .. }]`
这类**字符串键**不用改；只有直接构造 `ScheduledLane` / `LaneParam` 的地方需要改（当前没有）。

- [ ] **Step 5: 改 `transport.rs`**

把 `SetAutomation` 变体的字段换掉（含文档注释）：

```rust
    /// 实时更新一条自动化车道（`points` 为空 = 删除该车道）。
    ///
    /// 自动化是「仅混音」变更：不重建播放器，只把新曲线推给音频线程，逐块求值。
    ///
    /// `target` 是**控制侧解析好的**目标（`schedule::LaneTarget`）；音频线程不做键解析，
    /// 因为那会给 `ParamId` 分配内存，破坏零分配门禁。
    SetAutomation {
        /// 目标轨道。
        track: crate::model::TrackId,
        /// 已解析的自动化目标。
        target: crate::schedule::LaneTarget,
        /// 关键点（拍，升序）。
        points: Vec<crate::model::AutomationPoint>,
    },
```

同时把文件顶部的注释「`SetParam` 携带的参数名是唯一的所有权字段」改成
「`SetParam` 的参数名与 `SetAutomation` 的目标是所有权字段——都在控制侧分配妥当，实时侧只做移动」。

- [ ] **Step 6: 在 `audio.rs` 控制侧解析键**

把 `sync_events` 里的 `AutomationChanged` 分支替换为：

```rust
                // 自动化是「仅混音」变更：把新曲线直接推给音频线程，逐块求值——不必重建。
                // 键在这里（控制侧）解析成目标：音频线程拿到的是已经建好的 `ParamId`，只做移动。
                Event::AutomationChanged { track, param, points, .. } => {
                    if let Some(target) = ai_daw_engine::schedule::LaneTarget::from_key(param) {
                        self.queue_if_available(TransportCommand::SetAutomation {
                            track: *track,
                            target,
                            points: points.clone(),
                        });
                    }
                }
```

- [ ] **Step 7: 跑引擎全量测试 + 宿主 check**

Run:
```bash
cargo test -p ai-daw-engine --features realtime
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: 引擎测试全绿（含既有的 141 条 + 新增的键解析）；`cargo check` 无错。

- [ ] **Step 8: 提交**

```bash
git add crates/engine/src/schedule.rs crates/engine/src/player.rs crates/engine/src/transport.rs src-tauri/src/audio.rs
git commit -m "refactor(engine): 自动化目标改为可扩展的 LaneTarget，键在控制侧解析"
```

---

## Task 2: 引擎 —— 结构性校验 + 插件参数真正生效

让 `inst:` / `fx<i>:` 落到插件实例上，并证明它**覆盖静态值**。同时丢弃结构性不可能的车道。

**Files:**
- Modify: `crates/engine/src/schedule.rs`
- Modify: `crates/engine/src/player.rs`（只在 `mod tests` 里加测试插件与测试）

**Interfaces:**
- Consumes: Task 1 的 `LaneTarget`、`ScheduledLane`。
- Produces: 无新公开接口（只是让已有目标真的生效）。

- [ ] **Step 1: 写失败测试（结构性丢弃）**

在 `crates/engine/src/schedule.rs` 的 `mod tests` 追加：

```rust
    #[test]
    fn structurally_impossible_lanes_are_dropped() {
        let mut p = project_with_note(); // 该轨 instrument: None、effects: 空
        p.tracks[0].automation = vec![
            AutomationLane { param: "inst:cutoff".into(), points: vec![AutomationPoint { beat: 0.0, value: 1.0 }] },
            AutomationLane { param: "fx0:mix".into(), points: vec![AutomationPoint { beat: 0.0, value: 1.0 }] },
            AutomationLane { param: "gain_db".into(), points: vec![AutomationPoint { beat: 0.0, value: -3.0 }] },
        ];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.tracks[0].lanes.len(), 1, "没有乐器 / 没有效果器时，插件参数车道要丢掉");
        assert_eq!(s.tracks[0].lanes[0].param, LaneTarget::GainDb);

        // 装上乐器之后，`inst:` 就该入表。
        p.tracks[0].instrument = Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: Default::default(),
            state: vec![],
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(s.tracks[0].lanes.iter().any(|l| matches!(l.param, LaneTarget::Instrument(_))));
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p ai-daw-engine structurally_impossible_lanes_are_dropped`
Expected: FAIL（当前 `inst:` / `fx0:` 也会入表，`lanes.len()` 是 3）。

- [ ] **Step 3: 在调度构建里加结构性校验**

`schedule.rs` 的车道解析改成：

```rust
            // 自动化车道：解析键 + **结构性校验**——`inst:` 要求本轨有乐器，`fx<i>:`
            // 要求下标在效果链内；不满足就跟未知键一样丢弃（工程里仍保留，装回即恢复）。
            let lanes: Vec<ScheduledLane> = track
                .automation
                .iter()
                .filter_map(|lane| {
                    let param = LaneTarget::from_key(&lane.param)?;
                    match &param {
                        LaneTarget::Instrument(_) if track.instrument.is_none() => return None,
                        LaneTarget::Effect { index, .. } if *index >= track.effects.len() => return None,
                        _ => {}
                    }
                    let points = lane.points.clone();
                    Some(ScheduledLane { param, points })
                })
                .collect();
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p ai-daw-engine structurally_impossible_lanes_are_dropped`
Expected: PASS。

- [ ] **Step 5: 写失败测试（乐器参数生效 + 覆盖静态值）**

在 `crates/engine/src/player.rs` 的 `mod tests` 里，**测试插件区**（`struct HalfGain;` 附近）加：

```rust
    /// 测试用「参数即电平」插件：`level` 参数就是输出。
    ///
    /// **一个结构兼两种角色**（音源 / 效果器），两个自动化测试共用同一份夹具——
    /// 注册表按 plugin id 建实例，所以两个角色各注册一个 id、各自指定 `kind`。
    struct ParamLevel {
        level: f32,
        kind: PluginKind,
    }

    impl Plugin for ParamLevel {
        fn descriptor(&self) -> PluginDescriptor {
            PluginDescriptor {
                id: "test.paramlevel".into(),
                name: "ParamLevel".into(),
                kind: self.kind,
                // 音源没有输入总线，效果器有。
                io: IoLayout {
                    inputs: if self.kind == PluginKind::Effect { 2 } else { 0 },
                    outputs: 2,
                },
                capabilities: Default::default(),
            }
        }
        fn params(&self) -> Vec<ParamInfo> {
            vec![ParamInfo {
                id: ParamId("level".into()),
                name: "Level".into(),
                min: 0.0,
                max: 1.0,
                default: 0.0,
                automatable: true,
            }]
        }
        fn get_param(&self, _id: &ParamId) -> Option<f32> {
            Some(self.level)
        }
        fn set_param(&mut self, id: &ParamId, value: f32) {
            if id.0 == "level" {
                self.level = value;
            }
        }
        fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
        fn reset(&mut self) {}
        fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
            for channel in audio.iter_mut() {
                match self.kind {
                    // 音源：把缓冲直接写成参数值。
                    PluginKind::Instrument => channel.fill(self.level),
                    // 效果器：就地乘上参数值。
                    PluginKind::Effect => {
                        for sample in channel.iter_mut() {
                            *sample *= self.level;
                        }
                    }
                }
            }
        }
        fn save_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn load_state(&mut self, _state: &[u8]) -> Result<(), PluginError> {
            Ok(())
        }
    }

    #[test]
    fn instrument_param_automation_overrides_the_static_value() {
        let mut registry = PluginRegistry::new();
        registry.register("test.param.instrument", |_track| {
            Box::new(ParamLevel { level: 0.0, kind: PluginKind::Instrument })
        });

        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "T".into(),
            instrument: Some(PluginInstance {
                plugin_id: "test.param.instrument".into(),
                // 静态值 0.2；曲线要求 0.8 → 输出应当是 0.8（自动化覆盖静态值）。
                params: [("level".to_string(), 0.2)].into_iter().collect(),
                state: vec![],
            }),
            effects: vec![],
            clips: vec![],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![AutomationLane {
                param: "inst:level".into(),
                points: vec![AutomationPoint { beat: 0.0, value: 0.8 }],
            }],
        });

        let schedule = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(schedule.tracks[0].lanes.len(), 1, "inst: 目标要入表");
        let mut player = Player::new(schedule, &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!((peak(&out) - 0.8).abs() < 0.05, "乐器参数自动化应覆盖静态值（峰值 {}）", peak(&out));
    }

    #[test]
    fn effect_param_automation_overrides_the_static_value() {
        let mut registry = PluginRegistry::new();
        registry.register("test.stateful", |_track| Box::new(StateTone { level: 0.0 }));
        registry.register("test.param.effect", |_track| {
            Box::new(ParamLevel { level: 1.0, kind: PluginKind::Effect })
        });

        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        let mut t = tone_track(1, "T", 255); // 乐器满幅 1.0
        t.effects = vec![PluginInstance {
            plugin_id: "test.param.effect".into(),
            // 静态 0.25；曲线要求 0.5 → 输出应当是 0.5。
            params: [("level".to_string(), 0.25)].into_iter().collect(),
            state: vec![],
        }];
        t.automation = vec![AutomationLane {
            param: "fx0:level".into(),
            points: vec![AutomationPoint { beat: 0.0, value: 0.5 }],
        }];
        p.tracks.push(t);

        let schedule = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(schedule.tracks[0].lanes.len(), 1, "fx0: 目标要入表");
        let mut player = Player::new(schedule, &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!((peak(&out) - 0.5).abs() < 0.05, "效果器参数自动化应覆盖静态值（峰值 {}）", peak(&out));
    }
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p ai-daw-engine --features realtime`
Expected: 全绿（含上面两条新测试）。

- [ ] **Step 7: 提交**

```bash
git add crates/engine/src/schedule.rs crates/engine/src/player.rs
git commit -m "feat(engine): 插件参数自动化生效（乐器/效果器），丢弃结构性不可能的车道"
```

---

## Task 3: 实时安全门禁覆盖插件参数车道

把「`SetAutomation` 携带插件参数目标时 `apply_command` 仍零分配」钉进硬门禁。

**Files:**
- Modify: `crates/engine/tests/realtime_safety.rs`

**Interfaces:**
- Consumes: Task 1 的 `TransportCommand::SetAutomation { track, target, points }`。
- Produces: 无（只是测试）。

- [ ] **Step 1: 在门禁测试里加参与者**

`realtime_safety.rs` 的 `schedule()` 里那条轨道的 `automation` 追加一条 **`inst:` 车道**
（该轨乐器是 `builtin.synth`，参数 `cutoff` 确实存在）：

```rust
        automation: vec![
            ai_daw_engine::model::AutomationLane {
                param: "gain_db".into(),
                points: vec![
                    ai_daw_engine::model::AutomationPoint { beat: 0.0, value: 0.0 },
                    ai_daw_engine::model::AutomationPoint { beat: 8.0, value: -12.0 },
                ],
            },
            // 插件参数车道也要在门禁覆盖范围内：`refresh_mix` 会逐块对乐器 `set_param`。
            ai_daw_engine::model::AutomationLane {
                param: "inst:cutoff".into(),
                points: vec![
                    ai_daw_engine::model::AutomationPoint { beat: 0.0, value: 0.2 },
                    ai_daw_engine::model::AutomationPoint { beat: 8.0, value: 0.9 },
                ],
            },
        ],
```

- [ ] **Step 2: 在第二阶段加一条插件参数目标的 `apply_command` 断言**

把「第二阶段」末尾（既有的 `SetAutomation` 断言之后）追加：

```rust
    // 插件参数目标同样必须在音频线程上零分配：目标在测量前就解析好（`from_key` 会分配，
    // 那是控制侧的事），进到音频线程的只有移动与 `set_param`。
    let command = TransportCommand::SetAutomation {
        track: TrackId(1),
        target: ai_daw_engine::schedule::LaneTarget::from_key("inst:cutoff").expect("键能解析"),
        points: vec![
            ai_daw_engine::model::AutomationPoint { beat: 0.0, value: 0.3 },
            ai_daw_engine::model::AutomationPoint { beat: 4.0, value: 0.8 },
        ],
    };
    ALLOCATIONS.store(0, Ordering::SeqCst);
    player.apply_command(command);
    player.process_block(&mut out, MAX_BLOCK);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    assert_eq!(
        allocations, 0,
        "插件参数车道的 SetAutomation 必须在音频线程上零分配（saw {allocations}）"
    );
```

- [ ] **Step 3: 跑门禁**

Run: `cargo test -p ai-daw-engine --features realtime --test realtime_safety`
Expected: PASS（saw 0）。

- [ ] **Step 4: 提交**

```bash
git add crates/engine/tests/realtime_safety.rs
git commit -m "test(engine): 零分配门禁覆盖插件参数自动化车道"
```

---

## Task 4: 前端数据层 —— `automatable`、精选表、共享目标列表

**Files:**
- Modify: `src/data/plugins.ts`
- Modify: `src/stores/plugins.ts`
- Modify: `src/components/workspace/panes/SynthPanel.vue`
- Create: `src/utils/automation.ts`

**Interfaces:**
- Produces:
  - `ParamSpec.automatable?: boolean`；
  - `COMMON_INSTRUMENT_PARAMS: Record<string, string[]>`、`SYNTH_PARAM_LABELS: Record<string, string>`（`data/plugins.ts`）；
  - `src/utils/automation.ts`：
    - `interface AutomationTarget { key: string; label: string; group: string; min: number; max: number; step: number; digits: number; unit?: string }`
    - `automationTargetsFor(track: Track | null, paramsOf: (id: string | null | undefined) => ParamSpec[], t: (key: string, vars?: Record<string, string | number>) => string): AutomationTarget[]`
    - `laneValueAtOrNull(points: AutomationPoint[], beat: number): number | null`

- [ ] **Step 1: `ParamSpec` 加字段并透传**

`src/data/plugins.ts` 的 `ParamSpec` 加一个字段（放在 `digits` 之后）：

```ts
  /** 展示小数位。 */
  digits?: number;
  /** 是否可自动化（缺省视为可自动化）。外部插件由后端 `ParamInfo` 给出。 */
  automatable?: boolean;
```

`src/stores/plugins.ts` 的 `toSpec` 里把后端的值带上：

```ts
function toSpec(raw: BackendParam): ParamSpec {
  const span = raw.max - raw.min;
  return {
    id: raw.id,
    name: raw.name,
    min: raw.min,
    max: raw.max,
    step: span > 0 ? span / 100 : 0.01,
    default: raw.default,
    digits: 2,
    automatable: raw.automatable,
  };
}
```

- [ ] **Step 2: 提取合成器参数显示名 + 精选表**

`src/components/workspace/panes/SynthPanel.vue` 里那张 `const LABELS: Record<string, string> = { ... }`
**整块删掉**，改成从数据层引入：

```ts
import { SYNTH_PARAM_LABELS as LABELS } from "@/data/plugins";
```

（`SynthPanel` 里其余引用 `LABELS` 的地方不用改。）

`src/data/plugins.ts` 在 `SYNTH_PARAMS` 定义**之后**追加两样东西：

```ts
/**
 * 内置合成器参数 id → 显示名（i18n 键）。
 *
 * 与 `SynthPanel.vue` 共用同一张表（那边直接 `import { SYNTH_PARAM_LABELS as LABELS }`）——
 * 参数下拉与旋钮面板显示的名字必须一致。
 */
export const SYNTH_PARAM_LABELS: Record<string, string> = {
  wave: "synth.wave",
  sub: "synth.sub",
  pwm: "synth.pwm",
  glide: "synth.glide",
  hpf: "synth.hpf",
  cutoff: "synth.brightness",
  resonance: "synth.sharpness",
  filterEnv: "synth.filterEnvName",
  keyFollow: "synth.keyFollow",
  attack: "synth.attack",
  decay: "synth.decay",
  sustain: "synth.sustain",
  release: "synth.release",
  lfoRate: "synth.lfoRate",
  lfoDelay: "synth.lfoDelay",
  lfoPitch: "synth.lfoPitch",
  lfoFilter: "synth.lfoFilter",
  gain: "synth.gain",
  noise: "synth.noise",
  crush: "synth.crush",
  detune: "synth.detune",
  fm: "synth.fm",
  fmRatio: "synth.fmRatio",
  pitchEnv: "synth.pitchEnv",
  pitchDecay: "synth.pitchDecay",
  filterType: "synth.filterType",
};

/**
 * 自动化下拉里**精选**的参数（插件 id → 参数 id 数组）。
 *
 * 有表就只列表里的（顺序即展示顺序），没有表的插件列全部可自动化参数。
 * 想加/减内置合成器的可选参数，**只改这里**。
 */
export const COMMON_INSTRUMENT_PARAMS: Record<string, string[]> = {
  [SYNTH_ID]: ["cutoff", "resonance", "attack", "decay", "sustain", "release", "detune", "lfoRate"],
};
```

- [ ] **Step 3: 新建 `src/utils/automation.ts`**

```ts
/**
 * 自动化目标的**唯一来源**：编排视图的自动化车道与钢琴卷帘下方的参数车道共用它。
 *
 * 键的格式见 `specs/2026-10-10-plugin-param-automation-design.md` §4：
 * `gain_db` / `pan` / `inst:<param_id>` / `fx<i>:<param_id>`（`i` 从 **0** 起；
 * 下拉里的分组标题从 1 起显示，换算只在这里做一次）。
 */
import type { AutomationPoint, Track } from "@/api/types";
import {
  canonicalPluginId,
  COMMON_INSTRUMENT_PARAMS,
  type ParamSpec,
  SYNTH_ID,
  SYNTH_PARAM_LABELS,
} from "@/data/plugins";

/** 一条可自动化的目标（下拉项 + 值域）。 */
export interface AutomationTarget {
  /** 写进 `Track.automation[].param` 的键。 */
  key: string;
  /** 显示名（已翻译）。 */
  label: string;
  /** 下拉里的分组标题（已翻译）。 */
  group: string;
  min: number;
  max: number;
  step: number;
  digits: number;
  unit?: string;
}

type Translate = (key: string, vars?: Record<string, string | number>) => string;
type ParamsOf = (pluginId: string | null | undefined) => ParamSpec[];

/** 某插件**可自动化**的参数：有精选表就只取表里的（按表顺序），否则取全部。 */
function automatableParams(pluginId: string, paramsOf: ParamsOf): ParamSpec[] {
  const id = canonicalPluginId(pluginId);
  const all = paramsOf(id).filter((p) => p.automatable !== false);
  const common = COMMON_INSTRUMENT_PARAMS[id];
  if (!common) return all;
  const byId = new Map(all.map((p) => [p.id, p]));
  return common.map((pid) => byId.get(pid)).filter((p): p is ParamSpec => p !== undefined);
}

/** 参数显示名：内置合成器走 i18n（英文界面才不会是中文名），其余用插件自报的名字。 */
function paramLabel(pluginId: string, spec: ParamSpec, t: Translate): string {
  const key = SYNTH_PARAM_LABELS[spec.id];
  return canonicalPluginId(pluginId) === SYNTH_ID && key ? t(key) : spec.name;
}

function fromSpec(key: string, label: string, group: string, spec: ParamSpec): AutomationTarget {
  return {
    key,
    label,
    group,
    min: spec.min,
    max: spec.max,
    step: spec.step,
    digits: spec.digits ?? 2,
    unit: spec.unit,
  };
}

/**
 * 一条轨道当前可自动化的全部目标，顺序：轨道混音 → 乐器 → 效果器 1..n。
 *
 * `paramsOf` 传 `usePluginsStore().paramsOf`（内置已就绪、外部插件可能还没加载完，
 * 调用方负责对乐器调用一次 `ensureParams`）。
 */
export function automationTargetsFor(
  track: Track | null,
  paramsOf: ParamsOf,
  t: Translate,
): AutomationTarget[] {
  if (!track) return [];
  const out: AutomationTarget[] = [
    { key: "gain_db", label: t("auto.gain"), group: t("auto.group.track"), min: -60, max: 12, step: 0.5, digits: 1, unit: "dB" },
    { key: "pan", label: t("auto.pan"), group: t("auto.group.track"), min: -1, max: 1, step: 0.05, digits: 2 },
  ];

  const instrumentId = track.instrument?.plugin_id;
  if (instrumentId) {
    for (const spec of automatableParams(instrumentId, paramsOf)) {
      out.push(fromSpec(`inst:${spec.id}`, paramLabel(instrumentId, spec, t), t("auto.group.instrument"), spec));
    }
  }

  track.effects.forEach((fx, i) => {
    const group = t("auto.group.effect", { n: i + 1 });
    for (const spec of automatableParams(fx.plugin_id, paramsOf)) {
      out.push(fromSpec(`fx${i}:${spec.id}`, paramLabel(fx.plugin_id, spec, t), group, spec));
    }
  });

  return out;
}

/** 折线求值（镜像引擎 `lane_value_at`：相邻点线性插值、两端夹住；空返回 `null`）。 */
export function laneValueAtOrNull(points: AutomationPoint[], beat: number): number | null {
  const first = points[0];
  if (!first) return null;
  if (beat <= first.beat) return first.value;
  const last = points[points.length - 1]!;
  if (beat >= last.beat) return last.value;
  for (let i = 0; i + 1 < points.length; i += 1) {
    const a = points[i]!;
    const b = points[i + 1]!;
    if (beat >= a.beat && beat <= b.beat) {
      const span = b.beat - a.beat;
      if (span <= 1e-9) return b.value;
      return a.value + (b.value - a.value) * ((beat - a.beat) / span);
    }
  }
  return last.value;
}
```

- [ ] **Step 4: 加 i18n 分组键**

`src/i18n/zh.ts` 的自动化段追加：

```ts
  "auto.group.track": "轨道",
  "auto.group.instrument": "乐器",
  "auto.group.effect": "效果器 {n}",
```

`src/i18n/en.ts` 的对应位置追加：

```ts
  "auto.group.track": "Track",
  "auto.group.instrument": "Instrument",
  "auto.group.effect": "Effect {n}",
```

- [ ] **Step 5: 构建 + 键数一致**

Run:
```bash
npm run build
node -e '
const fs=require("fs");
const keys=(p)=>new Set([...fs.readFileSync(p,"utf8").matchAll(/^\s*"([^"]+)":/gm)].map(m=>m[1]));
const en=keys("src/i18n/en.ts"), zh=keys("src/i18n/zh.ts");
console.log(en.size, zh.size, [...en].filter(k=>!zh.has(k)), [...zh].filter(k=>!en.has(k)));
'
```
Expected: 构建通过；两个数字相等、两个数组为空。

- [ ] **Step 6: 提交**

```bash
git add src/data/plugins.ts src/stores/plugins.ts src/components/workspace/panes/SynthPanel.vue src/utils/automation.ts src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(ui): 插件参数可自动化元数据 + 共享的自动化目标列表"
```

---

## Task 5: 抽出 `AutomationCurve`，编排车道改用它（外观与行为不变）

把曲线画布从 `AutomationLane.vue` 整块搬进新组件，两个车道之后共用它。

**Files:**
- Create: `src/components/workspace/AutomationCurve.vue`
- Modify: `src/components/workspace/AutomationLane.vue`
- Modify: `src/views/ArrangeView.vue`（把 `TIMELINE_INSET_PX` 传进车道）

**Interfaces:**
- Produces: `AutomationCurve.vue`，props `{ points: AutomationPoint[]; min: number; max: number; step: number; digits: number; unit?: string; height: number; insetPx: number }`，
  emits `(e: "apply", points: AutomationPoint[])`。
  `AutomationLane.vue` 的 props 从 `{ height }` 变成 `{ height: number; insetPx: number }`——
  **`26` 这个常量只留在 `ArrangeView`**，由它传下来（不要在两处各写一个 26）。
- Consumes: `laneValueAtOrNull`（Task 4）。

- [ ] **Step 1: 建 `AutomationCurve.vue`**

把现在 `AutomationLane.vue` 里这些**原样搬过来**（逻辑不动，只把值域/刻度换成 props）：
刻度列、网格画布、折线（两端补抽头）、关键点、加点 / 拖点 / 右键删、播放头、
`plain()` / `sortedCopy()` / `yOf()` / `valueFromY()` / `beatAt()` / `pointTitle()`、以及
`onMounted/onUnmounted` 的 `pointerup` 监听。

关键改动：

```vue
<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { capturePointer } from "@/utils/pointer";
import { useTimelineStore } from "@/stores/timeline";
import { useTransportStore } from "@/stores/transport";
import type { AutomationPoint } from "@/api/types";

const props = defineProps<{
  points: AutomationPoint[];
  min: number;
  max: number;
  step: number;
  digits: number;
  unit?: string;
  /** 车道本体高度（px）。 */
  height: number;
  /** 画布左边缘相对第 0 拍的偏移：编排 26（TIMELINE_INSET_PX），卷帘车道 0。 */
  insetPx: number;
}>();

const emit = defineEmits<{ (e: "apply", points: AutomationPoint[]): void }>();

const timeline = useTimelineStore();
const transport = useTransportStore();

/** 拖动中的草稿（升序）；松手才 emit。 */
const draft = ref<AutomationPoint[] | null>(null);
const shown = computed(() => draft.value ?? props.points);

// yOf / valueFromY 改用 props.min / props.max / props.step / props.height
// beatAt 用 props.insetPx 代替原来的 TIMELINE_INSET_PX 常量
// onPointerUp 末尾：emit("apply", draftPoints.map(plain))
// onPointContextMenu 末尾：emit("apply", pointsWithOneRemoved)
</script>
```

（`formatValue` / `scaleLabel` / `pointTitle` 保留，读数里用 `props.digits` / `props.unit`。
原来的 `param` / `SPECS` / `PARAMS` / `Md3Select` / `XMarkIcon` / `clearLane` /
`valueAtPlayhead` / `store.setAutomation` / `layout` / `i18n` **都不要搬**——那些留在车道外壳里。）

- [ ] **Step 2: `AutomationLane.vue` 改成「外壳 + 曲线」**

`AutomationLane.vue` 保留：表头（下拉 / 名称 / 读数 / 清空 / 收起）、`param` 状态、
`trackId`、`stored`（从工程取该键的点）、`valueAtPlayhead`、`clearLane`，
并把 `layout.notify` 的错误处理留在 `apply(points)` 的回调里。

新增 import：`import AutomationCurve from "./AutomationCurve.vue";`、
`import { usePluginsStore } from "@/stores/plugins";`、
`import { automationTargetsFor, laneValueAtOrNull } from "@/utils/automation";`，
并删掉现在已搬进曲线的那些（`capturePointer`、`XMarkIcon` 若不再用、本地 `laneValueAt`、`plain`/`sortedCopy` 等）。

改动要点：

```ts
const plugins = usePluginsStore();

// 目标列表来自共享模块；下拉选项分组显示。
const targets = computed(() => automationTargetsFor(selection.track, plugins.paramsOf, i18n.t));
const chooserOptions = computed(() =>
  targets.value.map((t) => ({ id: t.key, label: t.label, group: t.group })),
);
const param = ref<string>("gain_db");
const current = computed(() => targets.value.find((t) => t.key === param.value) ?? null);
const stored = computed<AutomationPoint[]>(() => {
  const lane = selection.track?.automation?.find((l) => l.param === param.value);
  return lane?.points ?? [];
});
const valueAtPlayhead = computed(() =>
  laneValueAtOrNull(stored.value, transport.positionBeats),
);

async function applyPoints(points: AutomationPoint[]): Promise<void> {
  const id = selection.trackId;
  if (id === null) return;
  try {
    await store.setAutomation(id, param.value, points);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

/** 外部插件是懒加载的：进车道时确保乐器的参数表已在。 */
watch(
  () => selection.track?.instrument?.plugin_id,
  (id) => void plugins.ensureParams(id),
  { immediate: true },
);

/** 选中的目标被删掉时（乐器/效果器没了）回落到 gain_db。 */
watch(targets, (list) => {
  if (!list.some((t) => t.key === param.value)) param.value = "gain_db";
});
```

模板里的画布整块换成：

```vue
      <AutomationCurve
        v-if="current"
        :points="stored"
        :min="current.min"
        :max="current.max"
        :step="current.step"
        :digits="current.digits"
        :unit="current.unit"
        :height="props.height"
        :inset-px="props.insetPx"
        @apply="applyPoints"
      />
```

`AutomationLane` 的 props 声明改成 `defineProps<{ height: number; insetPx: number }>()`；
`ArrangeView.vue` 里把它传进来（**常量仍只定义在 `ArrangeView`**）：

```vue
      <AutomationLane :height="layout.automationHeight" :inset-px="TIMELINE_INSET_PX" />
```

- [ ] **Step 3: 构建 + 浏览器回归**

Run: `npm run build`
Expected: 通过。

浏览器（`npm run dev`，mock 后端）手验：编排视图打开自动化车道 → 选「增益」→
划线 / 拖点 / 右键删 / 清空 / 读数 / 网格与编排网格对齐——**与改动前逐项一致**。

- [ ] **Step 4: 提交**

```bash
git add src/components/workspace/AutomationCurve.vue src/components/workspace/AutomationLane.vue
git commit -m "refactor(ui): 抽出 AutomationCurve，自动化车道改用共享曲线"
```

---

## Task 6: 卷帘下方泳道改成参数车道 + 端到端验证 + 文档

**Files:**
- Modify: `src/components/workspace/ControllerLane.vue`
- Modify: `src/views/EditView.vue`
- Modify: `src/i18n/zh.ts`、`src/i18n/en.ts`（删 `lane.volume` / `lane.volumeHint`）
- Modify: `docs/superpowers/ROADMAP.md`、`docs/superpowers/specs/2026-10-10-plugin-param-automation-design.md`、本文件

**Interfaces:**
- Consumes: `AutomationCurve`（Task 5）、`automationTargetsFor` / `laneValueAtOrNull`（Task 4）。
- Produces: `ControllerLane` 新 props `{ notes, selectedIndices, targetKey, targets, offsetBeats, contentHeight, curvePoints }`，
  emits `update:targetKey` / `select` / `apply` / `delete` / `applyCurve`。

- [ ] **Step 1: `ControllerLane.vue` 改成参数车道**

- 删掉 `SPECS` / `ControllerProperty` / `OPTIONS` 与 `volume` 相关的一切；
  `力度` 变成一份内联 spec（`min 0 / max 127 / step 1 / digits 0`，`get: (n) => n.velocity`、
  `patch: (v) => ({ velocity: Math.round(v) })`），键固定为 `"velocity"`。
- 新 props / emits：

```ts
const props = defineProps<{
  notes: Note[];
  selectedIndices: number[];
  /** 当前选中的目标键：`"velocity"` 或自动化目标键（`gain_db` / `inst:cutoff` / `fx0:mix` …）。 */
  targetKey: string;
  /** 可选目标（力度由本组件自己加在最前面）。 */
  targets: AutomationTarget[];
  offsetBeats: number;
  contentHeight: number;
  /** 非 `velocity` 目标时的曲线点（来自 `Track.automation`）。 */
  curvePoints: AutomationPoint[];
}>();

const emit = defineEmits<{
  (e: "update:targetKey", value: string): void;
  (e: "select", indices: number[]): void;
  (e: "apply", patch: Partial<Note>): void;
  (e: "delete", indices: number[]): void;
  (e: "applyCurve", points: AutomationPoint[]): void;
}>();
```

- 下拉选项：第一组「音符属性」放 `力度`，其余按 `props.targets` 的 `group` 分组
  （复用 `Md3Select` 的 `group` 字段）。
- 新增 import：`import { automationTargetsFor, type AutomationTarget } from "@/utils/automation";`、
  `import AutomationCurve from "@/components/workspace/AutomationCurve.vue";`（`automationTargetsFor`
  用于把 `props.targets` 之外的东西补齐，这里其实只用类型；如果 TS 报未使用就直接只导入类型）。
- 选中目标的定义（曲线的值域与刻度列都取自它）：

```ts
/** 当前选中目标的定义；`velocity` 由本组件自己提供，不在 `props.targets` 里。 */
const curveTarget = computed(() => props.targets.find((t) => t.key === props.targetKey) ?? null);
```

- 模板分支：`targetKey === "velocity"` → 现有直角条那套（**一字不改**）；
  否则 → `<AutomationCurve v-if="curveTarget" :points="curvePoints" :min="curveTarget.min" :max="curveTarget.max" :step="curveTarget.step" :digits="curveTarget.digits" :unit="curveTarget.unit" :height="contentHeight + 6" :inset-px="0" @apply="(p) => emit('applyCurve', p)" />`，
  左侧 60px 的刻度列显示 `curveTarget.min` / `curveTarget.max`（`velocity` 时仍显示 0 / 127）。

- [ ] **Step 2: `EditView.vue` 接上**

先补 import（`AutomationPoint` 加到已有的 `@/api/types` 类型导入里）：

```ts
import { usePluginsStore } from "@/stores/plugins";
import { automationTargetsFor } from "@/utils/automation";
import type { AutomationPoint } from "@/api/types";
```

```ts
const plugins = usePluginsStore();

/** 卷帘下方泳道当前选中的目标键（`velocity` = 逐音符力度）。 */
const controllerTargetKey = ref<string>("velocity");
/** 可自动化的目标：轨道混音 + 乐器 + 效果器。 */
const controllerTargets = computed(() =>
  automationTargetsFor(selection.track, plugins.paramsOf, i18n.t),
);
/** 选中目标当前的曲线点。 */
const controllerCurvePoints = computed<AutomationPoint[]>(() => {
  const key = controllerTargetKey.value;
  const lane = selection.track?.automation?.find((l) => l.param === key);
  return lane?.points ?? [];
});

/** 提交曲线（一次拖拽 = 一条撤销记录）。 */
async function applyControllerCurve(points: AutomationPoint[]): Promise<void> {
  const t = selection.trackId;
  if (t === null) return;
  try {
    await store.setAutomation(t, controllerTargetKey.value, points);
  } catch (err) {
    layout.notify(i18n.t("error.setAutomation", { error: String(err) }));
  }
}

// 进视图 / 换轨道时确保乐器参数表已在（外部插件懒加载）。
watch(
  () => selection.track?.instrument?.plugin_id,
  (id) => void plugins.ensureParams(id),
  { immediate: true },
);
// 目标消失（乐器/效果器被删）时回落到力度。
watch(controllerTargets, (list) => {
  if (controllerTargetKey.value !== "velocity" && !list.some((t) => t.key === controllerTargetKey.value)) {
    controllerTargetKey.value = "velocity";
  }
});
```

模板：

```vue
      <ControllerLane
        :notes="clip?.notes ?? []"
        :selected-indices="selection.noteIndices"
        :target-key="controllerTargetKey"
        :targets="controllerTargets"
        :offset-beats="clipStart"
        :content-height="layout.laneHeight"
        :curve-points="controllerCurvePoints"
        @update:target-key="(k: string) => (controllerTargetKey = k)"
        @select="selection.setNoteSelection($event)"
        @apply="applyToSelection"
        @delete="deleteNotes"
        @apply-curve="applyControllerCurve"
      />
```

删掉旧的 `controllerProperty` 及其 `import { ControllerLane, type ControllerProperty }`
里的类型导入（改成只 `import ControllerLane`）。

- [ ] **Step 3: 删掉「音量」相关的 i18n 键**

`src/i18n/zh.ts` / `en.ts` 各删两行：`"lane.volume"`、`"lane.volumeHint"`。

- [ ] **Step 4: 构建 + 键数一致**

Run:
```bash
npm run build
node -e '
const fs=require("fs");
const keys=(p)=>new Set([...fs.readFileSync(p,"utf8").matchAll(/^\s*"([^"]+)":/gm)].map(m=>m[1]));
const en=keys("src/i18n/en.ts"), zh=keys("src/i18n/zh.ts");
console.log(en.size, zh.size, [...en].filter(k=>!zh.has(k)), [...zh].filter(k=>!en.has(k)));
'
```
Expected: 构建通过；两个数字相等、两个数组为空。

- [ ] **Step 5: 端到端手工验证（浏览器预览）**

`npm run dev` 后在浏览器里逐条确认：

1. 编排视图：自动化车道选 `乐器 → 亮度(cutoff)` 画一条曲线；
2. 切到编辑视图（双击片段进钢琴卷帘）：卷帘下方车道选同一个 `亮度`，**看到的是同一条曲线**；
3. 反过来在卷帘车道里拖一下，回编排看也是同一条（同一份数据）；
4. 效果器：给轨道加一个内置延迟 → 车道下拉出现「效果器 1 → 时间/反馈/混合」，画一条能用；
5. 选「力度」仍是逐音符直角条；下拉里**没有「音量」**；
6. 顶栏底部窗格按钮在混音 / 设置页显示为禁用。

- [ ] **Step 6: 更新文档**

- `docs/superpowers/ROADMAP.md`：C3 段落补一句「自动化目标已扩展到 `inst:` / `fx<i>:`，见
  `specs+plans/2026-10-10-plugin-param-automation*`」；§7 文档索引加入本 spec 与 plan。
- `docs/superpowers/specs/2026-10-10-plugin-param-automation-design.md`：横幅状态改成「✅ 完成（日期）」。
- `skills/mix.md`：把 `set_automation` 那一节补一句——`param` 除 `gain_db` / `pan` 外，还可以是
  `inst:<参数id>`（轨道乐器）或 `fx<i>:<参数id>`（效果链第 i 个，i 从 0 起）；参数表可用
  `plugin_params` 查（带 `automatable` 标记）。
- 本文件顶部加执行记录（实际通过的测试数、键数）。

- [ ] **Step 7: 提交**

```bash
git add src/components/workspace/ControllerLane.vue src/views/EditView.vue src/i18n/zh.ts src/i18n/en.ts docs/superpowers/
git commit -m "feat(ui): 卷帘下方泳道改为按插件自适应的参数车道（力度 + 插件参数曲线）"
```

---

## 自查记录

- **Spec 覆盖**：§3→Task 1/4；§4→Task 1/4；§5→Task 1/2；§6→Task 1/3；§7→Task 4/5/6；
  §8 验收→Task 2/3/4/6；§9 风险→Task 3（零分配）与 Task 2（结构性丢弃）。
- **占位符扫描**：无 TBD / 「类似上文」；每个代码步骤都给了可直接粘贴的代码。
- **类型一致性**：`LaneTarget::from_key`、`ScheduledLane.param: LaneTarget`、
  `TransportCommand::SetAutomation { target }`、`automationTargetsFor`、`laneValueAtOrNull`、
  `AutomationCurve` 的 props 在 Task 1/4/5/6 之间一致。
