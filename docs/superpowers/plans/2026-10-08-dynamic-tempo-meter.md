# 动态 BPM 与动态节拍（速度/拍号关键点）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 时间轴支持「关键点」，同时携带 BPM 与拍号，从关键点位置起生效；标尺在变化处显示 BPM，双击加关键点、右键删关键点；网格/标尺/读数按拍号**分段**正确呈现。

**Architecture:** 引擎引入 **`TempoMap`**（分段常量）取代「单一 tempo」的拍↔秒换算：`schedule`/`render` 用它定帧，`Player` 用它逐块取当前 BPM。数据模型在 `Transport` 上增加 `keyframes`（`serde(default)`，老工程零改动）。命令/事件走既有 `Command→Event→apply` 通路（关键点事件是**结构性**的，会重建定帧表）。前端镜像同一套分段算法（`src/utils/tempo.ts`），传输 store 用它做定位换算；网格改为**分段叠层背景**；标尺分段生成小节刻度并渲染关键点徽标。

**Tech Stack:** Rust（`crates/engine`）、Tauri 2、Vue 3 + Pinia + TypeScript。

## Global Constraints

- Rust 验证 = `cargo test -p daw-party-engine`；`src-tauri` 构建用 `cargo build --manifest-path src-tauri/Cargo.toml`。
- 前端**没有测试框架**：验证 = `npm run build` + `npm run dev` 手动核对。
- **向后兼容**：无关键点时，拍↔秒换算与现状**逐位一致**（`render`/`schedule` 输出不变）。这是硬性回归要求。
- 关键点不变量：`beat` 有限且 `> 0`（第 0 拍由 `tempo`/`time_sig_*` 承载）；按 `beat` 升序去重；`tempo ∈ [1,1000]`；拍号经 `Transport::normalize_time_sig`。
- 关键点之间是**分段常量**（不做渐变）；本期关键点支持添加/编辑/删除，**不支持拖动**。
- 「对准关键点」判定用**半格容差**（`|beat − kf.beat| < snapAt(beat)/2`）。
- `Player` 的「不自动停止」契约不变；离线 `render` 不受循环等运行时状态影响。
- 注释与文案用中文；i18n 文案中英各一份。
- 提交信息格式：`feat(tempo): …`。

---

## File Structure

**Rust**
- `crates/engine/src/model/transport.rs` — `Keyframe`、`Transport.keyframes`、归一化/增删。
- `crates/engine/src/model/mod.rs` — 导出 `Keyframe`。
- `crates/engine/src/tempo.rs`（新）— `TempoMap`。
- `crates/engine/src/lib.rs` — `pub mod tempo;`；保留常量版 `beats_to_seconds`。
- `crates/engine/src/schedule.rs` — 用 `TempoMap` 定帧；`tempo_points`。
- `crates/engine/src/render.rs` — 时长用 `TempoMap`。
- `crates/engine/src/player.rs` — `tempo_points` + 游标，逐块 `ctx.tempo`。
- `crates/engine/src/command.rs`、`event.rs`、`apply.rs`、`bus.rs` — 关键点命令/事件。
- `src-tauri/src/commands.rs`、`lib.rs` — `set_keyframe` / `remove_keyframe`。

**前端**
- `src/api/types.ts`、`tauri.ts`、`mock.ts` — 类型与命令。
- `src/utils/tempo.ts`（新）— 分段换算镜像。
- `src/stores/transport.ts` — 定位换算改用 `tempo.ts`。
- `src/stores/project.ts` — 关键点感知的编辑助手。
- `src/stores/timeline.ts` — 分段网格 + `barMarks` + `snapAt`/`barBeatsAt`。
- `src/components/workspace/TimelineRuler.vue` — 分段刻度 + 关键点标记。
- `src/components/layout/BottomBar.vue` — 生效值显示 + 关键点感知编辑 + 徽标。
- `src/views/EditView.vue` — 分段拍号/吸附。

---

### Task 1: `Transport` 增加关键点模型

**Files:**
- Modify: `crates/engine/src/model/transport.rs`
- Modify: `crates/engine/src/model/mod.rs`（加 `pub use transport::{Keyframe, Transport};` 或等价导出）

**Interfaces:**
- Produces:
  - `Keyframe { beat: f64, tempo: f64, time_sig_num: u8, time_sig_den: u8 }`（`Copy + Serialize + Deserialize + PartialEq`）
  - `Transport.keyframes: Vec<Keyframe>`
  - `Transport::MIN_KEYFRAME_BEAT: f64 = 1e-6`
  - `Transport::normalize_keyframe(Keyframe) -> Option<Keyframe>`
  - `Transport::upsert_keyframe(&mut self, Keyframe)`
  - `Transport::remove_keyframe(&mut self, beat: f64)`

- [ ] **Step 1: 写失败测试**

在 `crates/engine/src/model/transport.rs` 的 `mod tests` 里加：

```rust
    #[test]
    fn keyframes_are_normalized_sorted_and_deduped() {
        use super::{Keyframe, Transport};
        let mut t = Transport::default();
        // 越界 beat / tempo / 拍号都要被处理。
        t.upsert_keyframe(Keyframe { beat: 8.0, tempo: 5000.0, time_sig_num: 0, time_sig_den: 0 });
        t.upsert_keyframe(Keyframe { beat: 2.0, tempo: 90.0, time_sig_num: 3, time_sig_den: 4 });
        t.upsert_keyframe(Keyframe { beat: 0.0, tempo: 100.0, time_sig_num: 4, time_sig_den: 4 }); // 被拒
        assert_eq!(t.keyframes.len(), 2, "beat<=0 的关键点不得进入列表");
        assert_eq!(t.keyframes[0].beat, 2.0, "必须按 beat 升序");
        assert_eq!(t.keyframes[1].beat, 8.0);
        assert_eq!(t.keyframes[1].tempo, 1000.0, "tempo 夹到 1000");
        assert_eq!((t.keyframes[1].time_sig_num, t.keyframes[1].time_sig_den), (1, 1));

        // 同 beat 覆盖（容差内视为同一个）。
        t.upsert_keyframe(Keyframe { beat: 2.0 + 1e-9, tempo: 60.0, time_sig_num: 6, time_sig_den: 8 });
        assert_eq!(t.keyframes.len(), 2);
        assert_eq!(t.keyframes[0].tempo, 60.0);
        assert_eq!((t.keyframes[0].time_sig_num, t.keyframes[0].time_sig_den), (6, 8));

        t.remove_keyframe(2.0);
        assert_eq!(t.keyframes.len(), 1);
        assert_eq!(t.keyframes[0].beat, 8.0);
    }

    #[test]
    fn transport_without_keyframes_roundtrips_and_omits_the_field() {
        let t = Transport::default();
        let json = serde_json::to_string(&t).unwrap();
        assert!(!json.contains("keyframes"), "空列表不该写进 JSON");
        let back: Transport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p daw-party-engine keyframes_are_normalized`
Expected: 编译失败（`Keyframe` 不存在）

- [ ] **Step 3: 实现**

在 `crates/engine/src/model/transport.rs` 顶部（`Transport` 定义之前）加：

```rust
/// 时间轴上的一个关键点：从 `beat` 起，BPM 与拍号切换到给定值，直到下一个关键点。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub beat: f64,
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
}
```

`Transport` 加字段（`time_sig_den` 之后）：

```rust
    /// 速度/拍号关键点（按 beat 升序；beat 均 > 0）。第 0 拍由上面三个基准字段承载。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
```

`Transport::default()` 加 `keyframes: vec![]`。

在 `impl Transport` 里（`normalize_time_sig` 之后）加：

```rust
    /// 关键点合法的最小 beat：第 0 拍由基准字段承载，关键点必须在 0 之后。
    pub const MIN_KEYFRAME_BEAT: f64 = 1e-6;

    /// 归一化单个关键点；beat 非法时返回 `None`。
    pub fn normalize_keyframe(kf: Keyframe) -> Option<Keyframe> {
        if !kf.beat.is_finite() || kf.beat <= Self::MIN_KEYFRAME_BEAT {
            return None;
        }
        let (num, den) = Self::normalize_time_sig(kf.time_sig_num, kf.time_sig_den);
        let tempo = if kf.tempo.is_finite() { kf.tempo.clamp(1.0, 1000.0) } else { 120.0 };
        Some(Keyframe { beat: kf.beat, tempo, time_sig_num: num, time_sig_den: den })
    }

    /// 插入或覆盖关键点（同 beat 容差内视为同一个），并保持按 beat 升序。
    pub fn upsert_keyframe(&mut self, kf: Keyframe) {
        let Some(kf) = Self::normalize_keyframe(kf) else { return };
        if let Some(slot) = self
            .keyframes
            .iter_mut()
            .find(|k| (k.beat - kf.beat).abs() <= Self::MIN_KEYFRAME_BEAT)
        {
            *slot = kf;
        } else {
            self.keyframes.push(kf);
        }
        self.keyframes.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
    }

    /// 删除指定 beat 处的关键点（容差内匹配）。
    pub fn remove_keyframe(&mut self, beat: f64) {
        self.keyframes.retain(|k| (k.beat - beat).abs() > Self::MIN_KEYFRAME_BEAT);
    }
```

在 `crates/engine/src/model/mod.rs` 里确保 `Keyframe` 被导出（例如 `pub use transport::{Keyframe, Transport};`）。

- [ ] **Step 4: 运行测试**

Run: `cargo test -p daw-party-engine keyframes_are_normalized transport_without_keyframes`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/engine/src/model/transport.rs crates/engine/src/model/mod.rs
git commit -m "feat(tempo): keyframe model on Transport"
```

---

### Task 2: `TempoMap` 分段换算

**Files:**
- Create: `crates/engine/src/tempo.rs`
- Modify: `crates/engine/src/lib.rs`（加 `pub mod tempo;`）

**Interfaces:**
- Consumes: `crate::model::Transport`。
- Produces:
  - `Segment { start_beat, tempo, num, den }`
  - `TempoMap::from_transport(&Transport) -> TempoMap`
  - `tempo_at(beat) -> f64`、`meter_at(beat) -> (u8,u8)`、`segments() -> &[Segment]`
  - `beats_to_seconds(beats) -> f64`、`seconds_to_beats(secs) -> f64`

- [ ] **Step 1: 写失败测试**

`crates/engine/src/tempo.rs`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Keyframe, Transport};

    #[test]
    fn constant_transport_matches_the_old_formula() {
        let t = Transport::default(); // 120 BPM
        let m = TempoMap::from_transport(&t);
        assert!((m.beats_to_seconds(4.0) - 2.0).abs() < 1e-12);
        assert!((m.seconds_to_beats(2.0) - 4.0).abs() < 1e-12);
        assert_eq!(m.tempo_at(999.0), 120.0);
        assert_eq!(m.meter_at(999.0), (4, 4));
    }

    #[test]
    fn keyframe_switches_tempo_and_meter_from_its_beat() {
        let mut t = Transport::default(); // 120, 4/4
        t.upsert_keyframe(Keyframe { beat: 4.0, tempo: 240.0, time_sig_num: 3, time_sig_den: 4 });
        let m = TempoMap::from_transport(&t);
        assert_eq!(m.tempo_at(0.0), 120.0);
        assert_eq!(m.tempo_at(4.0), 240.0);
        assert_eq!(m.tempo_at(100.0), 240.0);
        assert_eq!(m.meter_at(4.0), (3, 4));
        // 0..4 拍 @120 = 2s；4..8 拍 @240 = 1s → 8 拍共 3s。
        assert!((m.beats_to_seconds(8.0) - 3.0).abs() < 1e-12);
        assert!((m.seconds_to_beats(3.0) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn seconds_to_beats_inverts_beats_to_seconds() {
        let mut t = Transport::default();
        t.upsert_keyframe(Keyframe { beat: 2.0, tempo: 60.0, time_sig_num: 4, time_sig_den: 4 });
        t.upsert_keyframe(Keyframe { beat: 7.5, tempo: 180.0, time_sig_num: 4, time_sig_den: 4 });
        let m = TempoMap::from_transport(&t);
        for beat in [0.0, 0.5, 2.0, 3.25, 7.5, 12.0] {
            let back = m.seconds_to_beats(m.beats_to_seconds(beat));
            assert!((back - beat).abs() < 1e-9, "beat={beat} back={back}");
        }
    }
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p daw-party-engine --lib tempo`
Expected: 编译失败（`TempoMap` 不存在）

- [ ] **Step 3: 实现**

`crates/engine/src/tempo.rs`：

```rust
//! 拍 ↔ 秒的分段换算：整首曲子的速度/拍号可以有关键点。

use crate::model::Transport;

/// 一段「分段常量」：从 `start_beat` 起用 `tempo`/`num`/`den`，直到下一段的 `start_beat`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start_beat: f64,
    pub tempo: f64,
    pub num: u8,
    pub den: u8,
}

/// 拍 ↔ 秒的分段换算表。无关键点时退化为常量换算（与老公式逐位一致）。
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    segments: Vec<Segment>,
}

impl TempoMap {
    /// 从工程传输设置构建；关键点按 beat 升序（第 0 段始终来自基准字段）。
    pub fn from_transport(t: &Transport) -> Self {
        let mut segments = vec![Segment {
            start_beat: 0.0,
            tempo: t.tempo.max(1e-6),
            num: t.time_sig_num.max(1),
            den: t.time_sig_den.max(1),
        }];
        let mut kfs = t.keyframes.clone();
        kfs.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap_or(std::cmp::Ordering::Equal));
        for kf in kfs {
            if kf.beat <= 0.0 {
                continue;
            }
            segments.push(Segment {
                start_beat: kf.beat,
                tempo: kf.tempo.max(1e-6),
                num: kf.time_sig_num.max(1),
                den: kf.time_sig_den.max(1),
            });
        }
        TempoMap { segments }
    }

    /// 所有分段（第 0 段起始拍恒为 0）。
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// `beat` 处生效的分段。
    pub fn segment_at(&self, beat: f64) -> &Segment {
        let beat = if beat > 0.0 { beat } else { 0.0 };
        let mut found = &self.segments[0];
        for s in &self.segments {
            if s.start_beat <= beat {
                found = s;
            } else {
                break;
            }
        }
        found
    }

    /// `beat` 处生效的 BPM。
    pub fn tempo_at(&self, beat: f64) -> f64 {
        self.segment_at(beat).tempo
    }

    /// `beat` 处生效的拍号。
    pub fn meter_at(&self, beat: f64) -> (u8, u8) {
        let s = self.segment_at(beat);
        (s.num, s.den)
    }

    /// 拍 → 秒（分段积分）。
    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        if !(beats > 0.0) {
            return 0.0;
        }
        let mut secs = 0.0;
        for (i, s) in self.segments.iter().enumerate() {
            let start = s.start_beat;
            if beats <= start {
                break;
            }
            let next = self.segments.get(i + 1).map(|n| n.start_beat);
            let end = match next {
                Some(n) if beats > n => n,
                _ => beats,
            };
            secs += (end - start) * 60.0 / s.tempo;
            if next.map(|n| beats <= n).unwrap_or(false) {
                break;
            }
        }
        secs
    }

    /// 秒 → 拍（分段反解）。最后一段视为无限长，因此任何有限 `secs` 都能落到某段内。
    pub fn seconds_to_beats(&self, secs: f64) -> f64 {
        if !(secs > 0.0) {
            return 0.0;
        }
        let mut acc = 0.0;
        for (i, s) in self.segments.iter().enumerate() {
            let next = self.segments.get(i + 1).map(|n| n.start_beat);
            let seg_secs = match next {
                Some(n) => (n - s.start_beat) * 60.0 / s.tempo,
                None => f64::INFINITY,
            };
            if secs <= acc + seg_secs {
                return s.start_beat + (secs - acc) * s.tempo / 60.0;
            }
            acc += seg_secs;
        }
        0.0
    }
}
```

`crates/engine/src/lib.rs` 加模块声明（在 `pub mod schedule;` 附近）：

```rust
pub mod tempo;
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p daw-party-engine --lib tempo`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add crates/engine/src/tempo.rs crates/engine/src/lib.rs
git commit -m "feat(tempo): piecewise beat<->second TempoMap"
```

---

### Task 3: 定帧 / 渲染 / 播放接入 `TempoMap`

**Files:**
- Modify: `crates/engine/src/schedule.rs`
- Modify: `crates/engine/src/render.rs`
- Modify: `crates/engine/src/player.rs`

**Interfaces:**
- Consumes: Task 2 的 `TempoMap`。
- Produces:
  - `ScheduledEvents.tempo_points: Vec<(u64, f64)>`（按帧升序，首项 `(0, tempo_at(0))`）
  - `Player` 逐块把当前 BPM 填进 `ProcessContext.tempo`

- [ ] **Step 1: 写回归 + 新行为测试**

`crates/engine/src/schedule.rs` 的 `mod tests` 里加：

```rust
    #[test]
    fn keyframe_shifts_later_event_frames() {
        use crate::model::{Keyframe, Note, Track, TrackId, Clip, ClipId};
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.transport.upsert_keyframe(Keyframe { beat: 2.0, tempo: 240.0, time_sig_num: 4, time_sig_den: 4 });
        p.tracks.push(Track {
            id: TrackId(1), name: "L".into(), instrument: None, effects: vec![],
            clips: vec![Clip { id: ClipId(1), start: 0.0, length: 4.0,
                notes: vec![Note { pitch: 60, start: 3.0, duration: 0.5, velocity: 100, volume: 127 }] }],
            mixer: Default::default(),
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        // 3 拍 = 0..2 @120 (1s) + 2..3 @240 (0.25s) = 1.25s = 60000 帧。
        assert_eq!(s.tracks[0].events[0].frame, 60_000);
        // tempo_points: 帧 0 → 120；帧 48000（2 拍 @120 = 1s）→ 240。
        assert_eq!(s.tempo_points[0], (0, 120.0));
        assert_eq!(s.tempo_points[1], (48_000, 240.0));
    }
```

`crates/engine/src/render.rs` 的 `mod tests` 里加：

```rust
    /// 无关键点时，渲染结果必须与本计划之前逐位一致（回归由现有用例覆盖）。
    /// 有关键点时，时长按分段计算。
    #[test]
    fn render_length_follows_tempo_keyframes() {
        use crate::model::Keyframe;
        let mut project = demo();
        project.transport.upsert_keyframe(Keyframe { beat: 1.0, tempo: 240.0, time_sig_num: 4, time_sig_den: 4 });
        let map = crate::tempo::TempoMap::from_transport(&project.transport);
        let expected_body = map.beats_to_seconds(project.length_beats());
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.1 };
        let buf = render(&project, &PluginRegistry::with_builtins(), &opts);
        let expected = (expected_body * 48_000.0) as usize + (0.1 * 48_000.0) as usize;
        assert!((buf.frames() as i64 - expected as i64).abs() <= 1);
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p daw-party-engine keyframe_shifts_later_event_frames render_length_follows_tempo_keyframes`
Expected: 编译失败（`tempo_points` / `upsert_keyframe` 未接）

- [ ] **Step 3: `schedule.rs` 接入**

- 顶部 `use crate::tempo::TempoMap;`。
- `ScheduledEvents` 结构体加：

```rust
    /// 速度分段在时间轴上的落点（帧 → BPM，按帧升序，首项为 `(0, tempo_at(0))`）。
    /// 供实时播放逐块取当前速度。
    pub tempo_points: Vec<(u64, f64)>,
```

- `from_project` 里把 `let tempo = project.transport.tempo;` 与 `let to_frame = …` 替换为：

```rust
        let map = TempoMap::from_transport(&project.transport);
        let to_frame = |beats: f64| (map.beats_to_seconds(beats) * sample_rate) as u64;
        let mut tempo_points: Vec<(u64, f64)> = vec![(0, map.tempo_at(0.0))];
        for s in map.segments().iter().skip(1) {
            tempo_points.push((to_frame(s.start_beat), s.tempo));
        }
```

- 构造返回值时补 `tempo_points,`（`tempo: map.tempo_at(0.0),` 保留，作为基准）。

- [ ] **Step 4: `render.rs` 接入**

- 顶部 `use crate::tempo::TempoMap;`。
- 把 `let tempo = project.transport.tempo; let body_sec = beats_to_seconds(project.length_beats(), tempo);` 替换为：

```rust
    let map = TempoMap::from_transport(&project.transport);
    let body_sec = map.beats_to_seconds(project.length_beats());
```

- 删掉该文件里对 `beats_to_seconds` 的 `use`（若不再使用），并更新 `render_duration_matches_project_length_plus_tail` 测试里对 `beats_to_seconds` 的期望计算为 `crate::tempo::TempoMap::from_transport(&project.transport).beats_to_seconds(project.length_beats())`。

- [ ] **Step 5: `player.rs` 接入**

- `Player` 结构体加字段：

```rust
    /// 速度分段落点（帧 → BPM，升序）；逐块取当前速度用。
    tempo_points: Vec<(u64, f64)>,
    /// 当前所处速度分段的游标。
    tempo_cursor: usize,
```

- `Player::new` 里 `let tempo = schedule.tempo;` 之后加 `let tempo_points = schedule.tempo_points.clone();`，并在结构体字面量里补 `tempo_points, tempo_cursor: 0,`。
- `seek` 里加：

```rust
        self.tempo_cursor = self
            .tempo_points
            .partition_point(|(f, _)| *f <= frame)
            .saturating_sub(1);
```

- `process_block` 里，在构造 `ctx` **之前**（且在任何 `let Player { … } = self;` 解构之前）加：

```rust
        while self.tempo_cursor + 1 < self.tempo_points.len()
            && self.tempo_points[self.tempo_cursor + 1].0 <= block_start
        {
            self.tempo_cursor += 1;
        }
        let tempo = self
            .tempo_points
            .get(self.tempo_cursor)
            .map(|(_, bpm)| *bpm)
            .unwrap_or(self.tempo);
```

并把 `ctx` 里的 `tempo: self.tempo` 改为 `tempo`。

- [ ] **Step 6: 运行测试**

Run: `cargo test -p daw-party-engine`
Expected: PASS（含既有的 `render_matches_player_at_different_block_size`、`render_is_deterministic_and_audible` 等）

- [ ] **Step 7: 提交**

```bash
git add crates/engine/src/schedule.rs crates/engine/src/render.rs crates/engine/src/player.rs
git commit -m "feat(tempo): schedule, render and player follow the tempo map"
```

---

### Task 4: 关键点命令与事件

**Files:**
- Modify: `crates/engine/src/command.rs`
- Modify: `crates/engine/src/event.rs`
- Modify: `crates/engine/src/apply.rs`
- Modify: `crates/engine/src/bus.rs`

**Interfaces:**
- Produces:
  - `Command::SetKeyframe { beat: f64, tempo: f64, num: u8, den: u8 }`、`Command::RemoveKeyframe { beat: f64 }`
  - `Event::KeyframeSet { actor, beat, tempo, num, den }`、`Event::KeyframeRemoved { actor, beat }`（二者 `is_mix_only() == false`）

- [ ] **Step 1: 写失败测试**

`crates/engine/src/bus.rs` 的 `mod tests` 里加：

```rust
    #[test]
    fn keyframe_commands_are_normalized_and_undoable() {
        let mut e = Engine::new("t");
        e.execute(Command::SetKeyframe { beat: 4.0, tempo: 90.0, num: 3, den: 4 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 1);
        assert_eq!(e.project().transport.keyframes[0].tempo, 90.0);
        assert_eq!((e.project().transport.keyframes[0].time_sig_num, e.project().transport.keyframes[0].time_sig_den), (3, 4));

        // beat<=0 → 安静忽略（列表不变），但命令仍算一次撤销步骤。
        e.execute(Command::SetKeyframe { beat: 0.0, tempo: 200.0, num: 4, den: 4 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 1);

        assert!(e.undo()); // 撤销 beat=0 那次
        assert!(e.undo()); // 撤销 beat=4 那次
        assert_eq!(e.project().transport.keyframes.len(), 0);

        e.execute(Command::SetKeyframe { beat: 4.0, tempo: 90.0, num: 4, den: 4 }, Actor::Human);
        e.execute(Command::RemoveKeyframe { beat: 4.0 }, Actor::Human);
        assert_eq!(e.project().transport.keyframes.len(), 0);
    }

    #[test]
    fn keyframe_events_are_structural() {
        assert!(!Event::KeyframeSet { actor: Actor::Human, beat: 1.0, tempo: 120.0, num: 4, den: 4 }.is_mix_only());
        assert!(!Event::KeyframeRemoved { actor: Actor::Human, beat: 1.0 }.is_mix_only());
    }
```

`crates/engine/src/event.rs` 的 `mod tests` 里，把新事件加入 `new_events_roundtrip_through_json` 的 `events` 列表：

```rust
            Event::KeyframeSet { actor: Actor::Human, beat: 2.0, tempo: 90.0, num: 3, den: 4 },
            Event::KeyframeRemoved { actor: Actor::Human, beat: 2.0 },
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p daw-party-engine keyframe_commands_are_normalized`
Expected: 编译失败（Command/Event 变体不存在）

- [ ] **Step 3: 加命令变体**

`crates/engine/src/command.rs` 在 `SetTimeSignature { num, den }` 之后加：

```rust
    /// 在第 `beat` 拍插入/更新一个关键点（同时带 BPM 与拍号）。`beat <= 0` 会被忽略。
    SetKeyframe { beat: f64, tempo: f64, num: u8, den: u8 },
    /// 删除第 `beat` 拍的关键点（容差内匹配）。
    RemoveKeyframe { beat: f64 },
```

- [ ] **Step 4: 加事件变体**

`crates/engine/src/event.rs` 在 `TimeSignatureChanged { … }` 之后加：

```rust
    /// 关键点被设置（已归一化）。
    KeyframeSet { actor: Actor, beat: f64, tempo: f64, num: u8, den: u8 },
    /// 关键点被删除。
    KeyframeRemoved { actor: Actor, beat: f64 },
```

在 `Event::actor()` 的 `match` 里把两者加入与 `TempoChanged` 同一支：

```rust
            Event::TempoChanged { actor, .. }
            | Event::TimeSignatureChanged { actor, .. }
            | Event::KeyframeSet { actor, .. }
            | Event::KeyframeRemoved { actor, .. }
```

`is_mix_only` 不改（两个新变体天然返回 `false`）。

- [ ] **Step 5: `apply.rs` 应用事件**

在 `apply` 的 `TimeSignatureChanged` 支之后加：

```rust
        Event::KeyframeSet { beat, tempo, num, den, .. } => {
            if let Some(kf) = crate::model::Transport::normalize_keyframe(crate::model::Keyframe {
                beat: *beat,
                tempo: *tempo,
                time_sig_num: *num,
                time_sig_den: *den,
            }) {
                project.transport.upsert_keyframe(kf);
            }
        }
        Event::KeyframeRemoved { beat, .. } => project.transport.remove_keyframe(*beat),
```

- [ ] **Step 6: `bus.rs` 翻译命令**

在 `translate` 的 `Command::SetTimeSignature { .. }` 支之后加：

```rust
            Command::SetKeyframe { beat, tempo, num, den } => vec![Event::KeyframeSet {
                actor: actor.clone(),
                beat: *beat,
                tempo: if tempo.is_finite() { tempo.clamp(1.0, 1000.0) } else { 120.0 },
                num: *num,
                den: *den,
            }],
            Command::RemoveKeyframe { beat } => {
                vec![Event::KeyframeRemoved { actor: actor.clone(), beat: *beat }]
            }
```

（`num`/`den` 的归一化在 `apply` 侧由 `normalize_keyframe` 完成。）

- [ ] **Step 7: 运行测试**

Run: `cargo test -p daw-party-engine`
Expected: PASS

- [ ] **Step 8: 提交**

```bash
git add crates/engine/src/command.rs crates/engine/src/event.rs crates/engine/src/apply.rs crates/engine/src/bus.rs
git commit -m "feat(tempo): set/remove keyframe commands and events"
```

---

### Task 5: Tauri 命令

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 4 的命令。
- Produces: `set_keyframe(context, beat, tempo, num, den) -> Project`、`remove_keyframe(context, beat) -> Project`。

- [ ] **Step 1: 加命令**

`src-tauri/src/commands.rs` 的 `set_time_signature` 之后加（沿用现有 `run` 辅助）：

```rust
/// 在第 `beat` 拍插入/更新关键点（BPM + 拍号）。
#[tauri::command]
pub fn set_keyframe(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    beat: f64,
    tempo: f64,
    num: u8,
    den: u8,
) -> Project {
    run(&app, &state, Command::SetKeyframe { beat, tempo, num, den })
}

/// 删除第 `beat` 拍的关键点。
#[tauri::command]
pub fn remove_keyframe(app: tauri::AppHandle, state: State<'_, AppState>, beat: f64) -> Project {
    run(&app, &state, Command::RemoveKeyframe { beat })
}
```

- [ ] **Step 2: 注册**

`src-tauri/src/lib.rs` 的 `generate_handler!` 里，在 `commands::set_time_signature,` 之后加：

```rust
            commands::set_keyframe,
            commands::remove_keyframe,
```

- [ ] **Step 3: 构建**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 成功

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(tempo): expose keyframe set/remove over IPC"
```

---

### Task 6: 前端类型 / API / mock

**Files:**
- Modify: `src/api/types.ts`
- Modify: `src/api/tauri.ts`
- Modify: `src/api/mock.ts`

**Interfaces:**
- Produces: `Keyframe` TS 类型；`Transport.keyframes`；`api.setKeyframe` / `api.removeKeyframe`；mock 侧同语义。

- [ ] **Step 1: 类型**

`src/api/types.ts`：加

```ts
/** 时间轴上的一个关键点：从 `beat` 起 BPM 与拍号切换。 */
export interface Keyframe {
  beat: number;
  tempo: number;
  time_sig_num: number;
  time_sig_den: number;
}
```

给 `Transport` 加字段：

```ts
  /** 速度/拍号关键点（按 beat 升序）。空表示整首恒定。 */
  keyframes: Keyframe[];
```

- [ ] **Step 2: API**

`src/api/tauri.ts` 加：

```ts
/** 在第 beat 拍插入/更新关键点。 */
export function setKeyframe(
  beat: number,
  tempo: number,
  num: number,
  den: number,
): Promise<Project> {
  return mockEnabled()
    ? Promise.resolve(mockApi.setKeyframe(beat, tempo, num, den))
    : invoke("set_keyframe", { beat, tempo, num, den });
}

/** 删除第 beat 拍的关键点。 */
export function removeKeyframe(beat: number): Promise<Project> {
  return mockEnabled()
    ? Promise.resolve(mockApi.removeKeyframe(beat))
    : invoke("remove_keyframe", { beat });
}
```

（`Project` 已在该文件被 import；`mockApi` 已 import。）

- [ ] **Step 3: mock**

`src/api/mock.ts`：

- `empty()` 与 `sample()` 的 `transport` 各加 `keyframes: [],`。
- 加两个方法（放在 `setTimeSignature` 附近）：

```ts
  setKeyframe: (beat: number, tempo: number, num: number, den: number): Project =>
    commit(() => {
      const kf = { beat, tempo: Math.min(1000, Math.max(1, tempo)), time_sig_num: num, time_sig_den: den };
      state.transport.keyframes = state.transport.keyframes
        .filter((k) => Math.abs(k.beat - beat) > 1e-6)
        .concat(kf)
        .sort((a, b) => a.beat - b.beat);
    }),

  removeKeyframe: (beat: number): Project =>
    commit(() => {
      state.transport.keyframes = state.transport.keyframes.filter((k) => Math.abs(k.beat - beat) > 1e-6);
    }),
```

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 提交**

```bash
git add src/api/types.ts src/api/tauri.ts src/api/mock.ts
git commit -m "feat(tempo): keyframe types, api and mock parity"
```

---

### Task 7: 前端分段换算镜像 `src/utils/tempo.ts`

**Files:**
- Create: `src/utils/tempo.ts`

**Interfaces:**
- Produces: `Segment`、`segments(t)`、`tempoAt(beat, t)`、`meterAt(beat, t)`、`beatsToSeconds(beats, t)`、`secondsToBeats(secs, t)`、`barBeatsAt(beat, t)`、`barBeatAt(beat, t)`。

- [ ] **Step 1: 实现（与 Rust `TempoMap` 同一算法）**

```ts
/**
 * 拍 ↔ 秒的分段换算（镜像 Rust 的 `crates/engine/src/tempo.rs`）。
 *
 * 两个方向都必须与引擎**逐位可对拍**：`beatsToSeconds` 用 `(delta) * 60 / bpm`，
 * `secondsToBeats` 用 `(delta) * bpm / 60`，不要改写数值形式。
 */
import type { Transport } from "@/api/types";

export interface Segment {
  startBeat: number;
  endBeat: number;
  tempo: number;
  num: number;
  den: number;
}

/** 按拍号/速度关键点切段（第 0 段来自基准字段）。 */
export function segments(t: Transport): Segment[] {
  const out: Segment[] = [];
  const kfs = [...(t.keyframes ?? [])].sort((a, b) => a.beat - b.beat);
  let start = 0;
  let tempo = t.tempo;
  let num = t.time_sig_num;
  let den = t.time_sig_den;
  for (const kf of kfs) {
    if (kf.beat <= start) {
      tempo = kf.tempo;
      num = kf.time_sig_num;
      den = kf.time_sig_den;
      continue;
    }
    out.push({ startBeat: start, endBeat: kf.beat, tempo, num, den });
    start = kf.beat;
    tempo = kf.tempo;
    num = kf.time_sig_num;
    den = kf.time_sig_den;
  }
  out.push({ startBeat: start, endBeat: Infinity, tempo, num, den });
  return out;
}

function segmentAt(beat: number, t: Transport): Segment {
  const b = beat > 0 ? beat : 0;
  let found = segments(t)[0]!;
  for (const s of segments(t)) {
    if (s.startBeat <= b) found = s;
    else break;
  }
  return found;
}

export function tempoAt(beat: number, t: Transport): number {
  return segmentAt(beat, t).tempo;
}

export function meterAt(beat: number, t: Transport): { num: number; den: number } {
  const s = segmentAt(beat, t);
  return { num: s.num, den: s.den };
}

export function beatsToSeconds(beats: number, t: Transport): number {
  if (!(beats > 0)) return 0;
  let secs = 0;
  const segs = segments(t);
  for (let i = 0; i < segs.length; i += 1) {
    const s = segs[i]!;
    if (beats <= s.startBeat) break;
    const next = segs[i + 1]?.startBeat;
    const end = next !== undefined && beats > next ? next : beats;
    secs += ((end - s.startBeat) * 60) / s.tempo;
    if (next !== undefined && beats <= next) break;
  }
  return secs;
}

export function secondsToBeats(secs: number, t: Transport): number {
  if (!(secs > 0)) return 0;
  let acc = 0;
  const segs = segments(t);
  for (let i = 0; i < segs.length; i += 1) {
    const s = segs[i]!;
    const next = segs[i + 1]?.startBeat;
    const segSecs = next !== undefined ? ((next - s.startBeat) * 60) / s.tempo : Infinity;
    if (secs <= acc + segSecs) return s.startBeat + ((secs - acc) * s.tempo) / 60;
    acc += segSecs;
  }
  return 0;
}

/** 一小节的长度（四分音符数）。 */
export function barBeatsAt(beat: number, t: Transport): number {
  const s = segmentAt(beat, t);
  return s.num * (4 / Math.max(1, s.den));
}

/** 一个「拍」（拍号单位）的长度（四分音符数）。 */
export function meterBeatAt(beat: number, t: Transport): number {
  return 4 / Math.max(1, segmentAt(beat, t).den);
}

/** 拍 → 小节.拍（跨段累计小节数）。 */
export function barBeatAt(beat: number, t: Transport): { bar: number; beatInBar: number } {
  let bar = 1;
  for (const s of segments(t)) {
    const barBeats = s.num * (4 / Math.max(1, s.den));
    const end = Number.isFinite(s.endBeat) ? s.endBeat : Infinity;
    if (beat < end) {
      const within = beat - s.startBeat;
      return { bar: bar + Math.floor(within / barBeats), beatInBar: Math.floor((within % barBeats) / (4 / Math.max(1, s.den))) + 1 };
    }
    // 本段整段计入小节数（段长未必是整数个小节，按「向上取整的整小节」推进）。
    bar += Math.max(1, Math.round((end - s.startBeat) / barBeats));
  }
  return { bar, beatInBar: 1 };
}
```

> `segments(t)` 被 `segmentAt` 反复调用会重复分配；若在意性能，可在调用方缓存一次 `segments` 传入。先正确，后优化。

- [ ] **Step 2: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 3: 提交**

```bash
git add src/utils/tempo.ts
git commit -m "feat(tempo): frontend piecewise tempo helpers"
```

---

### Task 8: 传输 store 用分段换算定位

**Files:**
- Modify: `src/stores/transport.ts`

**Interfaces:**
- Consumes: Task 7 的 `beatsToSeconds` / `secondsToBeats`。
- Produces: `positionBeats` / `beatsToFrames` 按分段正确。

- [ ] **Step 1: 改换算**

`src/stores/transport.ts`：

- 顶部 `import { beatsToSeconds, secondsToBeats } from "@/utils/tempo";`
- 加一个当前传输设置的 computed：

```ts
  /** 当前工程的传输设置（分段换算用）。 */
  const transportSettings = computed(
    () => project.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] },
  );
```

- `positionBeats` 改为：

```ts
  const positionBeats = computed(() =>
    secondsToBeats(positionFrames.value / effectiveSampleRate.value, transportSettings.value),
  );
```

- `beatsToFrames` 改为：

```ts
  function beatsToFrames(beats: number): number {
    if (!Number.isFinite(beats) || beats <= 0) return 0;
    return Math.round(beatsToSeconds(beats, transportSettings.value) * effectiveSampleRate.value);
  }
```

- 删掉不再使用的 `tempo` computed（若无其它引用；用 `grep -n "tempo" src/stores/transport.ts` 确认）。

- [ ] **Step 2: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 3: 手动核对**

Run: `npm run dev`，用 mock（浏览器预览）：直接改工程数据不方便，先留到 Task 10 一起核对。

- [ ] **Step 4: 提交**

```bash
git add src/stores/transport.ts
git commit -m "feat(tempo): transport seeks and reports position via the tempo map"
```

---

### Task 9: 项目 store 的关键点感知编辑

**Files:**
- Modify: `src/stores/project.ts`

**Interfaces:**
- Consumes: Task 6 的 `api.setKeyframe` / `api.removeKeyframe`、Task 7 的 `tempoAt`/`meterAt`/`meterBeatAt`。
- Produces:
  - `keyframeAt(beat): Keyframe | null`
  - `snapBeat(beat): number`（按所在分段的 1/16 拍吸附）
  - `setKeyframeAt(beat, { tempo?, num?, den? }): Promise<void>`
  - `removeKeyframeAt(beat): Promise<void>`
  - `applyTempoAt(beat, bpm): Promise<void>`
  - `applyMeterAt(beat, num, den): Promise<void>`

- [ ] **Step 1: 实现**

在 `src/stores/project.ts` 里加（`useTransportStore` 不必引入；`api` 已 import；顶部加 `import { meterAt, meterBeatAt, tempoAt } from "@/utils/tempo";`）：

```ts
  // ---- 速度/拍号关键点 ----

  /** 第 `beat` 拍处是否「对准」某个关键点（半格容差）。 */
  function keyframeAt(beat: number): Keyframe | null {
    const t = project.value?.transport;
    if (!t) return null;
    const half = meterBeatAt(beat, t) / 8; // 半个 1/16 格
    return t.keyframes.find((k) => Math.abs(k.beat - beat) < half) ?? null;
  }

  /** 按所在分段的 1/16 拍吸附。 */
  function snapBeat(beat: number): number {
    const t = project.value?.transport;
    if (!t) return beat;
    const s = meterBeatAt(beat, t) / 4;
    return Math.round(beat / s) * s;
  }

  /** 插入/更新关键点；未给的值继承该处的生效值。 */
  function setKeyframeAt(beat: number, v: { tempo?: number; num?: number; den?: number }): Promise<void> {
    const t = project.value?.transport;
    if (!t) return Promise.resolve();
    const cur = meterAt(beat, t);
    return edit(() =>
      api.setKeyframe(
        beat,
        v.tempo ?? tempoAt(beat, t),
        v.num ?? cur.num,
        v.den ?? cur.den,
      ),
    );
  }

  function removeKeyframeAt(beat: number): Promise<void> {
    return edit(() => api.removeKeyframe(beat));
  }

  /** 改 BPM：对准关键点 → 改该关键点；否则 → 改基准（第 0 拍）。 */
  function applyTempoAt(beat: number, bpm: number): Promise<void> {
    if (!Number.isFinite(bpm) || bpm < 20 || bpm > 300) return Promise.resolve();
    const kf = keyframeAt(beat);
    if (kf) return setKeyframeAt(kf.beat, { tempo: bpm });
    return setTempo(bpm);
  }

  /** 改拍号：对准关键点 → 改该关键点；否则 → 改基准（第 0 拍）。 */
  function applyMeterAt(beat: number, num: number, den: number): Promise<void> {
    const kf = keyframeAt(beat);
    if (kf) return setKeyframeAt(kf.beat, { num, den });
    return setTimeSignature(num, den);
  }
```

- [ ] **Step 2: 导出**

在 store 的 `return { … }` 里加：`keyframeAt, snapBeat, setKeyframeAt, removeKeyframeAt, applyTempoAt, applyMeterAt`。确保 `Keyframe` 类型已从 `@/api/types` import。

- [ ] **Step 3: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 4: 提交**

```bash
git add src/stores/project.ts
git commit -m "feat(tempo): keyframe-aware tempo/meter edits"
```

---

### Task 10: 底栏显示生效值 + 关键点感知编辑 + 徽标

**Files:**
- Modify: `src/components/layout/BottomBar.vue`
- Modify: `src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Consumes: Task 9 的 `applyTempoAt`/`applyMeterAt`/`keyframeAt`/`snapBeat`、Task 7 的 `tempoAt`/`meterAt`。
- Produces: BPM 框/拍号控件显示播放头处的**生效值**；改动写在对准的关键点或基准；播放头对准关键点时显示徽标。

- [ ] **Step 1: 显示生效值**

`src/components/layout/BottomBar.vue`：

- 顶部加 `import { meterAt, tempoAt } from "@/utils/tempo";`
- `timeSigNum` / `timeSigDen` 改为按播放头取生效值：

```ts
/** 播放头处的生效拍号与 BPM（随播放头移动）。 */
const atBeat = computed(() => transport.positionBeats);
const timeSigNum = computed(() =>
  meterAt(atBeat.value, store.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] }).num,
);
const timeSigDen = computed(() =>
  meterAt(atBeat.value, store.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] }).den,
);
```

- `tempo` ref 的初值与 `watch` 改为跟随播放头的生效 BPM：

```ts
const tempo = ref(tempoAt(0, store.project?.transport ?? { tempo: 120, time_sig_num: 4, time_sig_den: 4, keyframes: [] }));
watch([() => store.project?.transport, atBeat], () => {
  const t = store.project?.transport;
  if (t) tempo.value = tempoAt(atBeat.value, t);
}, { immediate: true, deep: true });
```

- `onTimeSignature` / BPM 写回改为关键点感知：

```ts
function onTimeSignature(num: number, den: number): void {
  void store.applyMeterAt(atBeat.value, num, den);
}
// BPM 输入框的写回
watch(tempo, (t) => {
  void store.applyTempoAt(atBeat.value, t);
});
```

> 注意：`setTempo` 原本在 `watch(tempo)` 里被调用，这里替换为 `applyTempoAt`；`tempo` 由 `atBeat` 更新时不应触发写回——用一个标志位或比较当前生效值来避免自触发回写：

```ts
let syncing = false;
watch([() => store.project?.transport, atBeat], () => {
  const t = store.project?.transport;
  if (!t) return;
  syncing = true;
  tempo.value = tempoAt(atBeat.value, t);
}, { immediate: true, deep: true });
watch(tempo, (t) => {
  if (syncing) { syncing = false; return; }
  void store.applyTempoAt(atBeat.value, t);
});
```

- [ ] **Step 2: 关键点徽标**

在 `.zone.left` 的 `TimeSignatureField` 之前加：

```html
      <span v-if="onKeyframe" class="keyframe-pill" :title="i18n.t('bottom.keyframeHint')">
        {{ i18n.t("bottom.keyframe") }}
      </span>
```

加 computed 与样式：

```ts
const onKeyframe = computed(() => store.keyframeAt(atBeat.value) !== null);
```

```css
.keyframe-pill {
  display: inline-flex;
  align-items: center;
  height: 22px;
  padding: 0 9px;
  border-radius: 999px;
  background-color: rgb(var(--md-tertiary-container));
  color: rgb(var(--md-on-tertiary-container));
  font-size: 10.5px;
  letter-spacing: 0.4px;
  white-space: nowrap;
}
```

- [ ] **Step 3: 文案**

`src/i18n/zh.ts`：`"bottom.keyframe": "关键点", "bottom.keyframeHint": "改动只影响该关键点之后"`；
`src/i18n/en.ts`：`"bottom.keyframe": "Keyframe", "bottom.keyframeHint": "Edits apply from this keyframe onward"`。

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 提交**

```bash
git add src/components/layout/BottomBar.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(tempo): bottom bar edits the effective tempo/meter at the playhead"
```

---

### Task 11: 标尺分段刻度 + 关键点标记（加/删）

**Files:**
- Modify: `src/components/workspace/TimelineRuler.vue`
- Modify: `src/stores/timeline.ts`（只加 `meterSegments`、`barMarks`、`barBeatsAt`、`snapAt`；网格叠层在 Task 12）

**Interfaces:**
- Consumes: Task 7 的 `segments` / `barBeatsAt` / `meterBeatAt`、Task 9 的 `setKeyframeAt`/`removeKeyframeAt`/`keyframeAt`/`snapBeat`。
- Produces: `timeline.meterSegments`、`timeline.barMarks`、`timeline.barBeatsAt(beat)`、`timeline.snapAt(beat)`。

- [ ] **Step 1: `timeline.ts` 加分段与刻度**

在 `src/stores/timeline.ts` 里加（顶部 `import { segments as tempoSegments, barBeatsAt as tempoBarBeatsAt, meterBeatAt } from "@/utils/tempo";`）：

```ts
  /** 拍号分段（用于网格与刻度）。 */
  const meterSegments = computed(() =>
    project.project ? tempoSegments(project.project.transport) : [{ startBeat: 0, endBeat: Infinity, tempo: 120, num: 4, den: 4 }],
  );

  /** 某拍处的拍号长度与吸附步长（供编辑视图使用）。 */
  function barBeatsAt(beat: number): number {
    return project.project ? tempoBarBeatsAt(beat, project.project.transport) : 4;
  }
  function snapAt(beat: number): number {
    return project.project ? meterBeatAt(beat, project.project.transport) / 4 : 0.25;
  }

  /** 小节刻度（跨分段累计小节号）。 */
  const barMarks = computed(() => {
    const z = zoom.value;
    const out: { bar: number; beat: number; width: number }[] = [];
    let bar = 1;
    for (const s of meterSegments.value) {
      const barBeats = s.num * (4 / Math.max(1, s.den));
      const end = Number.isFinite(s.endBeat) ? s.endBeat : totalBeats.value;
      let b = s.startBeat;
      while (b < end - 1e-9) {
        const w = Math.min(barBeats, end - b);
        out.push({ bar, beat: b, width: w * z });
        bar += 1;
        b += barBeats;
      }
    }
    return out;
  });
```

并在 store 的 `return { … }` 里导出 `meterSegments`、`barMarks`、`barBeatsAt`、`snapAt`。

- [ ] **Step 2: 标尺渲染分段刻度 + 关键点**

`src/components/workspace/TimelineRuler.vue`：

- 顶部加 `import { useProjectStore } from "@/stores/project"; const project = useProjectStore();`，并保留现有 `timeline` / `transport`。
- 把现有 `v-for="bar in timeline.totalBars"` 的 `.bar-mark` 块替换为按 `barMarks` 渲染：

```html
    <span
      v-for="m in timeline.barMarks"
      :key="m.bar"
      class="bar-mark"
      :style="{ width: `${m.width}px` }"
    >
      {{ m.bar }}
    </span>

    <button
      v-for="kf in keyframes"
      :key="kf.beat"
      class="kf-mark"
      :style="{ left: `${kf.beat * timeline.zoom}px` }"
      :title="`${kf.tempo.toFixed(0)} BPM · ${kf.time_sig_num}/${kf.time_sig_den}`"
      @contextmenu.prevent.stop="removeKeyframe(kf.beat)"
    >
      <span class="kf-badge">{{ kf.tempo.toFixed(0) }}</span>
      <span class="kf-meter">{{ kf.time_sig_num }}/{{ kf.time_sig_den }}</span>
    </button>
```

- `<script setup>` 加：

```ts
const keyframes = computed(() => project.project?.transport.keyframes ?? []);
const atBeat = (clientX: number) => timeline.snapBeats(timeline.beatsAt(clientX));
function addKeyframe(e: MouseEvent): void {
  const el = root.value;
  if (!el) return;
  const beat = atBeat(e.clientX); // 见下方说明：可用标尺元素自身的 rect
  if (beat <= 0) return;
  void project.setKeyframeAt(beat, {});
}
function removeKeyframe(beat: number): void {
  void project.removeKeyframeAt(beat);
}
```

- 标尺根元素加 `@dblclick="addKeyframe"`，并给 `ref` 命名（现有根元素已有 `ref="el"`，把它的 rect 用于 `beatsAt`：`const beat = timeline.snapBeats((e.clientX - el.getBoundingClientRect().left) / timeline.zoom)`）。双击不应触发两次 seek 的副作用——现有的 `pointerdown` 会把播放头定位到该处，这对「双击加关键点」是**期望**的（先定位再添加，初值继承该处生效值）。
- 样式（scoped）加：

```css
.kf-mark {
  position: absolute;
  top: 2px;
  transform: translateX(-50%);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  padding: 2px 6px;
  border: 0;
  border-radius: 8px;
  background-color: rgb(var(--md-tertiary-container));
  color: rgb(var(--md-on-tertiary-container));
  font-size: 9.5px;
  line-height: 1.1;
  cursor: pointer;
}
.kf-mark .kf-meter {
  font-size: 8px;
  opacity: 0.75;
}
```

- [ ] **Step 3: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 4: 手动核对**

Run: `npm run dev`（mock）：双击标尺非 0 位置 → 出现徽标；右键徽标 → 消失；标尺小节号在关键点前后按各自拍号变化。

- [ ] **Step 5: 提交**

```bash
git add src/components/workspace/TimelineRuler.vue src/stores/timeline.ts
git commit -m "feat(tempo): segment-aware ruler with keyframe markers"
```

---

### Task 12: 网格改为分段叠层

**Files:**
- Modify: `src/stores/timeline.ts`
- Modify: `src/views/ArrangeView.vue`、`src/views/EditView.vue`、`src/components/workspace/ControllerLane.vue`（把 `gridBackground` 换成 `gridStyle` 内联绑定）

**Interfaces:**
- Consumes: Task 11 的 `meterSegments`。
- Produces: `timeline.gridStyle`（一个内联样式对象，含 `backgroundImage`/`backgroundPosition`/`backgroundSize`/`backgroundRepeat`）；保留 `gridBackground` 作为兼容别名或删除。

- [ ] **Step 1: `gridStyle`**

在 `src/stores/timeline.ts` 里加：

```ts
  /**
   * 分段叠层的网格样式。
   *
   * 每段一对层：拍线格（每拍一格）+ 小节线；用 `background-size` 把 repeating 渐变限制在该段内，
   * 于是不同拍号的段落各自成立。无关键点时只有一段，等价于「整首一套网格」。
   */
  const gridStyle = computed(() => {
    const z = zoom.value;
    const images: string[] = [];
    const sizes: string[] = [];
    const positions: string[] = [];
    const repeats: string[] = [];
    for (const s of meterSegments.value) {
      const startPx = s.startBeat * z;
      const widthPx = Number.isFinite(s.endBeat) ? (s.endBeat - s.startBeat) * z : contentWidth.value;
      const cell = Math.max(2, z * (4 / Math.max(1, s.den)));
      const bar = Math.max(2, z * (s.num * (4 / Math.max(1, s.den))));
      images.push(`repeating-linear-gradient(to right, var(--grid-line) 0 1px, transparent 1px ${cell}px)`);
      positions.push(`${startPx}px 0`);
      sizes.push(`${Math.max(1, widthPx)}px 100%`);
      repeats.push("no-repeat");
      images.push(`repeating-linear-gradient(to right, var(--grid-bar) 0 1px, transparent 1px ${bar}px)`);
      positions.push(`${startPx}px 0`);
      sizes.push(`${Math.max(1, widthPx)}px 100%`);
      repeats.push("no-repeat");
    }
    return {
      backgroundImage: images.join(", "),
      backgroundPosition: positions.join(", "),
      backgroundSize: sizes.join(", "),
      backgroundRepeat: repeats.join(", "),
    };
  });
```

导出 `gridStyle`（保留 `gridBackground` 直到 Task 12 Step 2 全部改完再删）。

- [ ] **Step 2: 三个消费方改用内联 `gridStyle`**

- `ArrangeView.vue`：`.lane` 的 `:style` 改为 `:style="{ height: `${LANE_H}px`, ...timeline.gridStyle }"`；删掉 CSS 里的 `background-image: v-bind("timeline.gridBackground");`。
- `EditView.vue`：`.roll` 的 `:style` 改为 `{ width: rollWidth, height: rollHeight, ...timeline.gridStyle }`；删掉 CSS 里的 `background-image: v-bind(...)`。
- `ControllerLane.vue`：`.canvas` 的 `:style` 加上 `...timeline.gridStyle`；删掉 CSS 里的 `background-image: v-bind(...)`。

- [ ] **Step 3: 删掉 `gridBackground`**

Run: `grep -rn "gridBackground" src/`
Expected: 无输出后，从 `src/stores/timeline.ts` 删除 `gridBackground` computed 与其导出。

- [ ] **Step 4: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 5: 手动核对**

Run: `npm run dev`：无关键点时，网格与计划 B 的「每拍一格」完全一致；加一个拍号关键点（如第 8 拍 3/4）后，第 8 拍之后的格子变成每小节 3 格，且编排/编辑/控制器泳道一致。

- [ ] **Step 6: 提交**

```bash
git add src/stores/timeline.ts src/views/ArrangeView.vue src/views/EditView.vue src/components/workspace/ControllerLane.vue
git commit -m "feat(tempo): segment-aware grid via layered backgrounds"
```

---

### Task 13: 编辑视图与底栏读数的分段修正

**Files:**
- Modify: `src/views/EditView.vue`
- Modify: `src/components/layout/BottomBar.vue`

**Interfaces:**
- Consumes: Task 11 的 `timeline.barBeatsAt` / `timeline.snapAt`、Task 7 的 `barBeatAt`。

- [ ] **Step 1: `EditView` 用所在段的拍号与吸附**

- `beatsPerBar` 改为 `const beatsPerBar = computed(() => timeline.barBeatsAt(clipStart.value));`
- `gridLabel` 改为按所在段的吸附步长计算：`const gridLabel = computed(() => { const snap = timeline.snapAt(clipStart.value); return `1/${Math.round(4 / snap)}`; });`
- `pointAt` 里的 `timeline.snapBeats(...)` 保持不变（`snapBeats` 会用 `snapAt` —— 在 Task 11/Task 12 里把 `timeline.snapBeats` 的实现改为 `Math.round(beat / snapAt(beat)) * snapAt(beat)`）。
- `onPointerMove` 的 `move` 分支里 `Math.round((p.beat - d.originBeat) / timeline.snap)` 改为用 `timeline.snapAt(p.beat)`。

> 若 Task 11/12 未改 `snapBeats` 的实现，就在本步一并改为用 `snapAt`。

- [ ] **Step 2: `BottomBar` 读数分段**

把 Task 10 里保留的 `bars` / `positionText` 改为用分段：

```ts
import { barBeatAt, barBeatsAt } from "@/utils/tempo";

const bars = computed(() => {
  const t = store.project?.transport;
  if (!t) return 0;
  const clips = (store.project?.tracks ?? []).flatMap((x) => x.clips);
  const beats = clips.reduce((m, c) => Math.max(m, c.start + c.length), 0);
  if (beats <= 0) return 0;
  // 末尾所在小节的序号即小节总数（末尾在最后一段内）。
  return barBeatAt(beats, t).bar + (beats % barBeatsAt(beats, t) > 1e-9 ? 1 : 0);
});

const positionText = computed(() => {
  const t = store.project?.transport;
  if (!t) return "1.1";
  const p = barBeatAt(transport.positionBeats, t);
  return `${p.bar}.${p.beatInBar}`;
});
```

- [ ] **Step 3: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 4: 手动核对**

Run: `npm run dev`：加一个 6/8 关键点后，标尺小节号、底栏「小节.拍」、卷帘新建片段的默认长度都按所在段落正确。

- [ ] **Step 5: 提交**

```bash
git add src/views/EditView.vue src/components/layout/BottomBar.vue
git commit -m "feat(tempo): edit view and readouts follow meter segments"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-08-dynamic-tempo-meter-design.md`）**
- 3.1 数据模型（`Keyframe`/`Transport.keyframes`/不变量）→ Task 1。
- 3.2 `TempoMap` 分段换算 → Task 2。
- 4.2 定帧/播放/渲染接入 → Task 3。
- 4.1 命令与事件 → Task 4。
- 4.3 Tauri 命令 → Task 5。
- 5.1 类型与 API → Task 6。
- 3.3 前端镜像 → Task 7。
- 5.2 传输定位 → Task 8。
- 5.3 关键点感知编辑（含半格容差）→ Task 9。
- 5.4 底栏生效值与徽标 → Task 10、13。
- 5.5 标尺分段刻度 + 关键点标记 + 双击/右键 → Task 11。
- 5.6 分段网格叠层 → Task 12。
- 5.7 受影响调用方（EditView、BottomBar 读数）→ Task 13。

**Placeholder 扫描**：无 TBD/TODO；算法与数据均为具体实现。

**类型一致性**：`Segment`（Rust `tempo.rs` / TS `tempo.ts`）字段一致；`tempo_points` 在 Task 3 定义并在同任务 `player.rs` 使用；`meterSegments`/`barMarks`/`barBeatsAt`/`snapAt` 在 Task 11 定义、Task 12/13 使用；`applyTempoAt`/`applyMeterAt`/`keyframeAt`/`snapBeat` 在 Task 9 定义、Task 10 使用。

**已知粗糙处（不阻塞，但实现者需知）**：`src/utils/tempo.ts` 的 `segmentAt` 每次调用都重建 `segments`（正确但重复分配）；若性能敏感，先缓存再优化。`barBeatAt` 对「段长方能整除小节」做了取整近似，仅用于人读的读数，不参与计时。
