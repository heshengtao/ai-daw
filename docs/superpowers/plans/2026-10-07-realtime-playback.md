# daw-party 实时播放与传输控制实现计划（Plan 2）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让同一张音频图在 cpal 上实时播放，提供播放/暂停/停止/定位的传输控制，且实时回调内不分配内存（realtime-safe），并与离线渲染逐位一致。

**Architecture:** 引入"定帧事件表"（`ScheduledEvents`）作为离线与实时的共用输入；把逐块处理抽成 `Player`，离线 `render` 改为循环调用 `Player::process_block`，从而**结构性地保证离线 == 实时**（同一份调度、同一份逐样本 DSP 数学）。控制线程与音频线程之间用 rtrb 无锁 SPSC 队列传递传输命令。cpal 设备宿主放在 `realtime` feature 之后，避免无音频设备的环境（CI）构建失败。

**Tech Stack:** 在 Plan 1 基础上新增 `cpal = "0.18"`（optional，feature `realtime`）与 `rtrb = "0.4"`（无锁 SPSC 环形缓冲）。

## Global Constraints

- 沿用 Plan 1 全部约束：`edition = "2021"`、`license = "Apache-2.0"`、所有 crate `#![forbid(unsafe_code)]`、样本 `f32`、`pub const BLOCK_SIZE: usize = 256`、音乐时间用拍（`f64`）、渲染确定性、公共 API 加 `///`、提交结尾带固定 trailer。
- 新增依赖版本：`cpal = "0.18"`（optional）、`rtrb = "0.4"`（非 optional）。
- **实时路径禁止**在音频回调内做堆分配、加锁、系统调用、日志。`Player` 的所有 scratch 缓冲在构造时预留容量。
- **离线与实时必须逐位一致**：同一 `ScheduledEvents` + 同一采样率 → 相同样本，与块大小无关。
- 事件帧语义沿用 Plan 1：半开区间 `[block_start, block_end)`，帧偏移 `floor((t_sec - block_start_sec) * sample_rate)`，且恒有 `0 <= frame < block_frames`。
- 每个任务末尾 `cargo test --workspace` 必须全绿、输出无 warning；任务 6 另需 `cargo test --workspace --features realtime`。
- 提交信息前缀 `feat:` / `fix:` / `chore:`，结尾固定带：
  `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`

## 文件结构

- `crates/engine/src/schedule.rs`（新）— `ScheduledEvents` / `ScheduledTrack` / `ScheduledMidi`。
- `crates/engine/src/player.rs`（新）— `Player`（逐块处理、传输状态、realtime-safe）。
- `crates/engine/src/transport.rs`（新）— `TransportCommand`。
- `crates/engine/src/command_queue.rs`（新）— rtrb 包装：`CommandSender` / `CommandReceiver` / `command_queue`。
- `crates/engine/src/realtime.rs`（新，`#[cfg(feature = "realtime")]`）— cpal 设备宿主 `AudioHost`、`RealtimeOptions`、`RealtimeError`。
- `crates/engine/src/render.rs`（改）— 改为基于 `ScheduledEvents` + `Player`。
- `crates/engine/src/bus.rs`（改）— 新增 `Engine::from_project`。
- `crates/engine/src/lib.rs`（改）— 挂载新模块。
- `crates/engine/Cargo.toml`（改）— `rtrb`、可选 `cpal`、`[features] realtime`。
- `crates/engine/examples/play_demo.rs`（新，`realtime` feature）— 实时播放 demo。

---

### Task 1: 定帧事件表 `ScheduledEvents`

**Files:**
- Create: `crates/engine/src/schedule.rs`
- Modify: `crates/engine/src/lib.rs`（加 `pub mod schedule;`）

**Interfaces:**
- Consumes: `crate::beats_to_seconds`、`crate::model::{Project, TrackId, MixerChannel, PluginInstance}`、`daw_party_plugin_api::MidiEvent`。
- Produces: `ScheduledMidi { frame: u64, event: MidiEvent }`、`ScheduledTrack { track_id, instrument: Option<PluginInstance>, mixer: MixerChannel, events: Vec<ScheduledMidi> }`、`ScheduledEvents { tracks: Vec<ScheduledTrack>, end_frame: u64, tempo: f64 }`、`ScheduledEvents::from_project(&Project, sample_rate: f64) -> ScheduledEvents`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/schedule.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, Track, TrackId};

    fn project_with_note() -> Project {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration: 1.0, velocity: 100 }],
            }],
            mixer: Default::default(),
        });
        p
    }

    #[test]
    fn builds_sorted_absolute_frame_events() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert_eq!(s.tracks.len(), 1);
        let ev = &s.tracks[0].events;
        assert_eq!(ev.len(), 2);
        // 120bpm, 48kHz: 1 beat = 0.5s = 24000 frames.
        assert_eq!(ev[0].frame, 24_000);
        assert!(matches!(ev[0].event, MidiEvent::NoteOn { note: 60, velocity: 100 }));
        assert_eq!(ev[1].frame, 48_000);
        assert!(matches!(ev[1].event, MidiEvent::NoteOff { note: 60 }));
    }

    #[test]
    fn end_frame_is_project_body_length() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        // clip end = 4 beats = 2.0 s = 96000 frames.
        assert_eq!(s.end_frame, 96_000);
    }

    #[test]
    fn carries_instrument_and_mixer() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert!(s.tracks[0].instrument.is_none());
        assert_eq!(s.tracks[0].mixer.gain_db, 0.0);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine schedule::`
Expected: 编译失败（`ScheduledEvents` 未定义）。

- [ ] **Step 3: 实现**

`crates/engine/src/schedule.rs` 顶部：

```rust
use daw_party_plugin_api::MidiEvent;

use crate::beats_to_seconds;
use crate::model::{MixerChannel, PluginInstance, Project, TrackId};

/// 一条已定绝对帧的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledMidi {
    pub frame: u64,
    pub event: MidiEvent,
}

/// 单轨的定帧事件表与播放所需信息（自带插件引用与混音参数，使 `Player` 不依赖 `Project`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTrack {
    pub track_id: TrackId,
    pub instrument: Option<PluginInstance>,
    pub mixer: MixerChannel,
    pub events: Vec<ScheduledMidi>,
}

/// 整首工程的定帧事件表：离线渲染与实时播放共用。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledEvents {
    pub tracks: Vec<ScheduledTrack>,
    pub end_frame: u64,
    pub tempo: f64,
}

impl ScheduledEvents {
    /// 从工程构建定帧事件表。事件按帧稳定排序；同帧时保持（clip 顺序 → note 顺序 → NoteOn 先于 NoteOff）的原有次序，以保证与 Plan 1 的渲染一致。
    pub fn from_project(project: &Project, sample_rate: f64) -> Self {
        let tempo = project.transport.tempo;
        let to_frame = |beats: f64| (beats_to_seconds(beats, tempo) * sample_rate) as u64;

        let mut tracks = Vec::with_capacity(project.tracks.len());
        for track in &project.tracks {
            let mut events = Vec::new();
            for clip in &track.clips {
                for note in &clip.notes {
                    let on = clip.start + note.start;
                    let off = on + note.duration;
                    events.push(ScheduledMidi {
                        frame: to_frame(on),
                        event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                    });
                    events.push(ScheduledMidi {
                        frame: to_frame(off),
                        event: MidiEvent::NoteOff { note: note.pitch },
                    });
                }
            }
            events.sort_by_key(|e| e.frame);
            tracks.push(ScheduledTrack {
                track_id: track.id,
                instrument: track.instrument.clone(),
                mixer: track.mixer,
                events,
            });
        }

        ScheduledEvents { tracks, end_frame: to_frame(project.length_beats()), tempo }
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod schedule;
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine schedule::`
Expected: PASS（3 个测试）。

```bash
git add -A
git commit -F - <<'EOF'
feat: add scheduled events table shared by offline and realtime

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 2: 实时安全 `Player`

**Files:**
- Create: `crates/engine/src/player.rs`、`crates/engine/src/transport.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: Task 1 的 `ScheduledEvents`、`crate::registry::PluginRegistry`、`daw_party_plugin_api::{Plugin, ProcessContext, TimedMidiEvent}`。
- Produces: `TransportCommand`；`Player::new(ScheduledEvents, &PluginRegistry, sample_rate: f64, max_block: usize) -> Player`；`process_block(&mut self, out: &mut [f32], frames: usize)`；`play/pause/stop/seek(frame)`；`is_playing() -> bool`；`position_frames() -> u64`；`apply_command(TransportCommand)`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/player.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Track, TrackId};
    use crate::registry::PluginRegistry;
    use crate::schedule::ScheduledEvents;
    use crate::transport::TransportCommand;
    use crate::model::Project;

    fn schedule() -> ScheduledEvents {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100 }],
            }],
            mixer: Default::default(),
        });
        ScheduledEvents::from_project(&p, 48_000.0)
    }

    #[test]
    fn silent_until_play() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        let mut out = vec![1.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(!player.is_playing());
    }

    #[test]
    fn plays_and_advances_position() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().any(|s| s.abs() > 0.01), "should be audible");
        assert_eq!(player.position_frames(), 512);
    }

    #[test]
    fn stop_pauses_and_rewinds() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        player.apply_command(TransportCommand::Stop);
        assert!(!player.is_playing());
        assert_eq!(player.position_frames(), 0);
    }

    #[test]
    fn seek_moves_cursor_without_playing_early_notes() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        // Note ends at beat 0.5 = 0.25s = 12000 frames; seek past it -> silence.
        player.apply_command(TransportCommand::Seek { frame: 20_000 });
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(player.position_frames(), 20_512);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine player::`
Expected: 编译失败（`Player`、`TransportCommand` 未定义）。

- [ ] **Step 3: 实现传输命令**

`crates/engine/src/transport.rs`：

```rust
/// 运行时传输命令（不属于可撤销的工程编辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportCommand {
    Play,
    Pause,
    Stop,
    Seek { frame: u64 },
}
```

- [ ] **Step 4: 实现 `Player`**

`crates/engine/src/player.rs` 顶部（`#[cfg(test)]` 之前）：

```rust
use daw_party_plugin_api::{Plugin, ProcessContext, TimedMidiEvent};

use crate::registry::PluginRegistry;
use crate::schedule::{ScheduledEvents, ScheduledMidi};
use crate::transport::TransportCommand;

/// dB → 线性增益。
pub(crate) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

struct TrackVoice {
    plugin: Box<dyn Plugin>,
    events: Vec<ScheduledMidi>,
    cursor: usize,
    left_gain: f32,
    right_gain: f32,
    muted: bool,
}

/// 实时安全的逐块播放器：构造后 `process_block` 不再分配内存。
pub struct Player {
    tracks: Vec<TrackVoice>,
    sample_rate: f64,
    tempo: f64,
    position: u64,
    end_frame: u64,
    playing: bool,
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    scratch_events: Vec<TimedMidiEvent>,
}

impl Player {
    /// `max_block` 为回调可能请求的最大帧数；所有 scratch 缓冲按此预留，避免回调内分配。
    pub fn new(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        sample_rate: f64,
        max_block: usize,
    ) -> Self {
        let tempo = schedule.tempo;
        let end_frame = schedule.end_frame;
        let mut tracks = Vec::with_capacity(schedule.tracks.len());
        for st in &schedule.tracks {
            let Some(instance) = &st.instrument else { continue };
            let Some(mut plugin) = registry.create(&instance.plugin_id) else { continue };
            if st.mixer.mute {
                continue;
            }
            plugin.prepare(sample_rate, max_block);
            for (id, value) in &instance.params {
                plugin.set_param(&id.clone().into(), *value);
            }
            let gain = db_to_linear(st.mixer.gain_db);
            let pan = st.mixer.pan.clamp(-1.0, 1.0);
            tracks.push(TrackVoice {
                plugin,
                events: st.events.clone(),
                cursor: 0,
                left_gain: gain * ((1.0 - pan) * 0.5).sqrt(),
                right_gain: gain * ((1.0 + pan) * 0.5).sqrt(),
                muted: st.mixer.mute,
            });
        }
        Player {
            tracks,
            sample_rate,
            tempo,
            position: 0,
            end_frame,
            playing: false,
            scratch_left: vec![0.0; max_block],
            scratch_right: vec![0.0; max_block],
            scratch_events: Vec::with_capacity(max_block.max(64)),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// 定位到指定帧：重置所有插件并重算事件游标（已在播放中的音符不会被重新触发）。
    pub fn seek(&mut self, frame: u64) {
        self.position = frame;
        for track in self.tracks.iter_mut() {
            track.cursor = track.events.partition_point(|e| e.frame < frame);
            track.plugin.reset();
        }
    }

    /// 应用一条传输命令。
    pub fn apply_command(&mut self, command: TransportCommand) {
        match command {
            TransportCommand::Play => self.playing = true,
            TransportCommand::Pause => self.playing = false,
            TransportCommand::Stop => {
                self.playing = false;
                self.seek(0);
            }
            TransportCommand::Seek { frame } => self.seek(frame),
        }
    }

    /// 填充一个交错立体声块（`out.len() >= frames * 2`）。实时安全：不分配内存。
    pub fn process_block(&mut self, out: &mut [f32], frames: usize) {
        debug_assert!(out.len() >= frames * 2);
        let out = &mut out[..frames * 2];
        out.fill(0.0);
        if !self.playing {
            return;
        }

        let block_start = self.position;
        let block_end = block_start + frames as u64;
        let ctx = ProcessContext {
            sample_rate: self.sample_rate,
            block_size: frames,
            tempo: self.tempo,
            playing: true,
        };

        let Player { tracks, scratch_left, scratch_right, scratch_events, .. } = self;

        for track in tracks.iter_mut() {
            if track.muted {
                continue;
            }
            scratch_events.clear();
            while let Some(ev) = track.events.get(track.cursor).copied() {
                if ev.frame >= block_end {
                    break;
                }
                if ev.frame >= block_start {
                    scratch_events.push(TimedMidiEvent {
                        frame: (ev.frame - block_start) as u32,
                        event: ev.event,
                    });
                }
                track.cursor += 1;
            }

            if scratch_left.len() < frames {
                scratch_left.resize(frames, 0.0);
            }
            if scratch_right.len() < frames {
                scratch_right.resize(frames, 0.0);
            }
            let left = &mut scratch_left[..frames];
            let right = &mut scratch_right[..frames];
            left.fill(0.0);
            right.fill(0.0);

            track.plugin.process(&ctx, &mut [left, right], scratch_events);

            for i in 0..frames {
                out[i * 2] += left[i] * track.left_gain;
                out[i * 2 + 1] += right[i] * track.right_gain;
            }
        }

        self.position = block_end;
        if self.end_frame > 0 && self.position >= self.end_frame {
            self.position = self.end_frame;
            self.playing = false;
        }
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod player;
pub mod transport;
```

- [ ] **Step 5: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine player::`
Expected: PASS（4 个测试）。

```bash
git add -A
git commit -F - <<'EOF'
feat: add realtime-safe block player with transport state

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 3: 无锁命令队列

**Files:**
- Create: `crates/engine/src/command_queue.rs`
- Modify: `crates/engine/src/lib.rs`、`crates/engine/src/player.rs`、`crates/engine/Cargo.toml`

**Interfaces:**
- Consumes: `rtrb::{RingBuffer, Producer, Consumer}`、`TransportCommand`。
- Produces: `command_queue(capacity: usize) -> (CommandSender, CommandReceiver)`；`CommandSender::send(&mut self, TransportCommand) -> bool`；`CommandReceiver::drain(&mut self, impl FnMut(TransportCommand))`；`Player::drain_commands(&mut self, &mut CommandReceiver)`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/command_queue.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportCommand;

    #[test]
    fn send_then_drain_in_order() {
        let (mut tx, mut rx) = command_queue(8);
        assert!(tx.send(TransportCommand::Play));
        assert!(tx.send(TransportCommand::Seek { frame: 1234 }));
        let mut seen = Vec::new();
        rx.drain(|c| seen.push(c));
        assert_eq!(seen, vec![TransportCommand::Play, TransportCommand::Seek { frame: 1234 }]);
        // queue is now empty
        let mut empty = Vec::new();
        rx.drain(|c| empty.push(c));
        assert!(empty.is_empty());
    }

    #[test]
    fn full_queue_reports_failure_without_blocking() {
        let (mut tx, _rx) = command_queue(1);
        assert!(tx.send(TransportCommand::Play));
        assert!(!tx.send(TransportCommand::Pause));
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine command_queue::`
Expected: 编译失败（`command_queue` 未定义）。

- [ ] **Step 3: 加入依赖**

`crates/engine/Cargo.toml` 的 `[dependencies]` 增加：

```toml
rtrb = "0.4"
```

- [ ] **Step 4: 实现**

先在 `crates/engine/src/player.rs` 的 `impl Player` 中增加：

```rust
    /// 从无锁队列取出所有待处理传输命令并应用。
    pub fn drain_commands(&mut self, receiver: &mut crate::command_queue::CommandReceiver) {
        receiver.drain(|command| self.apply_command(command));
    }
```

`crates/engine/src/command_queue.rs` 顶部：

```rust
use rtrb::{Consumer, Producer, RingBuffer};

use crate::transport::TransportCommand;

/// 控制侧发送端（非实时线程使用）。
pub struct CommandSender {
    producer: Producer<TransportCommand>,
}

/// 音频侧接收端（实时线程使用，`drain` 为 lock-free / wait-free）。
pub struct CommandReceiver {
    consumer: Consumer<TransportCommand>,
}

/// 建立一对无锁 SPSC 命令通道。
pub fn command_queue(capacity: usize) -> (CommandSender, CommandReceiver) {
    let (producer, consumer) = RingBuffer::new(capacity);
    (CommandSender { producer }, CommandReceiver { consumer })
}

impl CommandSender {
    /// 非阻塞发送；队列满时返回 false（命令被丢弃，永不阻塞）。
    pub fn send(&mut self, command: TransportCommand) -> bool {
        self.producer.push(command).is_ok()
    }
}

impl CommandReceiver {
    /// 取出当前所有命令并按顺序回调。实时安全。
    pub fn drain(&mut self, mut apply: impl FnMut(TransportCommand)) {
        while let Ok(command) = self.consumer.pop() {
            apply(command);
        }
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod command_queue;
```

- [ ] **Step 5: 运行测试，确认通过并提交**

Run: `cargo test --workspace`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add lock-free transport command queue

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 4: 离线渲染复用 `Player`（保证离线 == 实时）

**Files:**
- Modify: `crates/engine/src/render.rs`
- Modify: `crates/engine/src/lib.rs`（移除 `pub mod sequencer;`）
- Delete: `crates/engine/src/sequencer.rs`（职责已由 `ScheduledEvents` + `Player` 取代）

**Interfaces:**
- Consumes: `ScheduledEvents`（Task 1）、`Player`（Task 2）、`crate::audio::AudioBuffer`、`crate::registry::PluginRegistry`、`crate::BLOCK_SIZE`。
- Produces: `RenderOptions`、`render(...)`（签名不变）。

- [ ] **Step 1: 加"离线 == 实时"一致性测试（本任务是行为保持型重构）**

本任务把 `render` 改为基于 `Player` 的薄循环，**不引入新可观测行为**，所以没有"新功能式"的红测试。保护网是 Plan 1 既有的 render 测试，加上下面这条更强的一致性测试。在 `crates/engine/src/render.rs` 的测试模块中保留 `render_duration_matches_project_length_plus_tail` 与 `render_is_deterministic_and_audible`，并新增：

```rust
    #[test]
    fn render_matches_player_at_different_block_size() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.2 };
        let registry = PluginRegistry::with_builtins();
        let buffer = render(&project, &registry, &opts);

        let schedule = crate::schedule::ScheduledEvents::from_project(&project, opts.sample_rate);
        let mut player = crate::player::Player::new(schedule, &registry, opts.sample_rate, 4096);
        player.apply_command(crate::transport::TransportCommand::Play);

        let total = buffer.frames();
        let mut out = vec![0.0f32; total * 2];
        let mut off = 0usize;
        while off < total {
            let frames = 128usize.min(total - off);
            player.process_block(&mut out[off * 2..(off + frames) * 2], frames);
            off += frames;
        }
        assert_eq!(buffer.data(), out.as_slice(), "offline render must equal realtime Player output");
    }
```

- [ ] **Step 2: 跑这条测试**

Run: `cargo test -p daw-party-engine render::render_matches_player_at_different_block_size`
Expected: **PASS**。这是行为保持型重构的一致性测试：重构前它验证旧路径与 `Player` 逐位一致（若不一致，说明存在真实 bug，必须先查明再重构）；重构后它锁定 `render` 即 `Player` 的输出。此步骤不追求"红"，因为本任务不引入新行为。

- [ ] **Step 3: 重写 `render` 为 `Player` 之上的薄循环**

把 `crates/engine/src/render.rs` 的实现体替换为（保留 `RenderOptions` 与 `render` 签名、`use` 相应调整）：

```rust
use crate::audio::AudioBuffer;
use crate::beats_to_seconds;
use crate::model::Project;
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::transport::TransportCommand;
use crate::BLOCK_SIZE;

/// 离线渲染选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    pub sample_rate: f64,
    pub tail_seconds: f64,
}

/// 把工程离线渲染为立体声交错缓冲。与实时播放共用 `ScheduledEvents` + `Player`，逐位一致。
pub fn render(project: &Project, registry: &PluginRegistry, options: &RenderOptions) -> AudioBuffer {
    let sr = options.sample_rate;
    let tempo = project.transport.tempo;
    let body_sec = beats_to_seconds(project.length_beats(), tempo);
    let total_frames = ((body_sec + options.tail_seconds) * sr) as usize;

    let schedule = ScheduledEvents::from_project(project, sr);
    let mut player = Player::new(schedule, registry, sr, BLOCK_SIZE);
    player.apply_command(TransportCommand::Play);

    let mut buffer = AudioBuffer::silence(2, total_frames);
    let data = buffer.data_mut();
    let mut block_start = 0usize;
    while block_start < total_frames {
        let frames = BLOCK_SIZE.min(total_frames - block_start);
        let start = block_start * 2;
        let end = start + frames * 2;
        player.process_block(&mut data[start..end], frames);
        block_start += frames;
    }
    buffer
}
```

删除 render.rs 中不再使用的私有 `db_to_linear`（已迁移到 `player.rs`）。同时删除 `crates/engine/src/sequencer.rs` 并从 `lib.rs` 移除 `pub mod sequencer;`：`collect_track_events` 重构后已无任何调用方，其绝对帧语义由 `ScheduledEvents`（Task 1）与 `Player` 的游标切片承担——半开窗口与"帧恒在 `[0, block_frames)` 内"的保证现在是结构性的（不再涉及取整）。

- [ ] **Step 4: 运行全部测试，确认通过并提交**

Run: `cargo test --workspace`
Expected: PASS（既有 render 测试保持绿色 + 新块大小无关性测试）。

```bash
git add -A
git commit -F - <<'EOF'
refactor: render offline via Player for exact realtime parity

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 5: `Engine::from_project`（载入工程 + id 计数器恢复）

**Files:**
- Modify: `crates/engine/src/bus.rs`

**Interfaces:**
- Consumes: `crate::model::Project`。
- Produces: `Engine::from_project(project: Project) -> Engine`。

- [ ] **Step 1: 写失败测试**

在 `crates/engine/src/bus.rs` 测试模块追加：

```rust
    #[test]
    fn from_project_recovers_id_counters() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        e.execute(Command::AddTrack { name: "B".into() }, Actor::Human);
        let project = e.project().clone();

        let mut loaded = Engine::from_project(project);
        let ev = loaded.execute(Command::AddTrack { name: "C".into() }, Actor::Human);
        let id = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        // must not collide with the two loaded tracks
        assert_eq!(id.0, 3);
        assert_eq!(loaded.project().tracks.len(), 3);
    }

    #[test]
    fn from_empty_project_starts_ids_at_one() {
        let mut e = Engine::from_project(crate::model::Project::new("empty"));
        let ev = e.execute(Command::AddTrack { name: "A".into() }, Actor::Human);
        let id = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        assert_eq!(id.0, 1);
    }
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine bus::from_`
Expected: 编译失败（`from_project` 未定义）。

- [ ] **Step 3: 实现**

在 `crates/engine/src/bus.rs` 的 `impl Engine` 内加入：

```rust
    /// 从已存在的工程构建引擎，并把 id 计数器恢复到不冲突的高水位。
    pub fn from_project(project: Project) -> Self {
        let next_track_id = project.tracks.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
        let next_clip_id = project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.id.0)
            .max()
            .unwrap_or(0)
            + 1;
        Engine { project, history: vec![], next_track_id, next_clip_id }
    }
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test --workspace`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add Engine::from_project with id counter recovery

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 6: cpal 实时设备宿主（`realtime` feature）

**Files:**
- Create: `crates/engine/src/realtime.rs`、`crates/engine/examples/play_demo.rs`
- Modify: `crates/engine/src/lib.rs`、`crates/engine/Cargo.toml`

**Interfaces:**
- Consumes: `cpal`、`Player`、`command_queue::{command_queue, CommandSender}`、`ScheduledEvents`、`PluginRegistry`、`TransportCommand`。
- Produces: `RealtimeOptions { sample_rate: f64, max_block: usize, command_capacity: usize }`；`RealtimeError`；`AudioHost::start(ScheduledEvents, &PluginRegistry, RealtimeOptions) -> Result<AudioHost, RealtimeError>`；`AudioHost::{play, pause, stop, seek}(&mut self) -> bool`；`AudioHost::position_frames(&self) -> u64`；`AudioHost::is_playing(&self) -> bool`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/realtime.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};

    fn schedule() -> ScheduledEvents {
        let mut p = Project::new("rt");
        let mut track = Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            clips: vec![],
            mixer: Default::default(),
        };
        track.clips.push(Clip {
            id: ClipId(1),
            start: 0.0,
            length: 1.0,
            notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100 }],
        });
        p.tracks.push(track);
        ScheduledEvents::from_project(&p, 48_000.0)
    }

    // 依赖真实音频设备的冒烟测试：在无设备环境（CI）应返回错误而非 panic。
    #[test]
    fn start_returns_result_without_panicking() {
        let options = RealtimeOptions { sample_rate: 48_000.0, max_block: 2048, command_capacity: 64 };
        match AudioHost::start(schedule(), &PluginRegistry::with_builtins(), options) {
            Ok(mut host) => {
                assert!(host.play());
                assert_eq!(host.position_frames(), 0);
            }
            Err(e) => {
                // No device in this environment is an acceptable outcome.
                assert!(!format!("{e}").is_empty());
            }
        }
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine --features realtime realtime::`
Expected: 编译失败（`AudioHost` 未定义 / feature 未配置）。

- [ ] **Step 3: 配置 Cargo feature**

`crates/engine/Cargo.toml` 增加：

```toml
[features]
default = []
realtime = ["dep:cpal"]

[dependencies]
cpal = { version = "0.18", optional = true }
```

- [ ] **Step 4: 实现设备宿主**

`crates/engine/src/realtime.rs` 顶部：

```rust
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::command_queue::{command_queue, CommandSender};
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::transport::TransportCommand;

/// 实时宿主选项。
#[derive(Debug, Clone, Copy)]
pub struct RealtimeOptions {
    pub sample_rate: f64,
    pub max_block: usize,
    pub command_capacity: usize,
}

/// 实时宿主错误。
#[derive(Debug, Error)]
pub enum RealtimeError {
    #[error("no default output device available")]
    NoDevice,
    #[error("device does not support an f32 stereo output configuration: {0}")]
    UnsupportedConfig(String),
    #[error("cpal error: {0}")]
    Cpal(String),
}

/// cpal 实时播放宿主：拥有输出流与控制侧命令发送端。
pub struct AudioHost {
    stream: cpal::Stream,
    commands: CommandSender,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
}

impl AudioHost {
    /// 打开默认输出设备并开始播放工程（初始为暂停态，需调用 `play`）。
    pub fn start(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
    ) -> Result<Self, RealtimeError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(RealtimeError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| RealtimeError::UnsupportedConfig(e.to_string()))?;
        let channels = supported.channels() as usize;
        let config = supported.config();
        let frames_hint = options.max_block;

        let (tx, mut rx) = command_queue(options.command_capacity);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let position_cb = Arc::clone(&position);
        let playing_cb = Arc::clone(&playing);

        let mut player = Player::new(schedule, registry, config.sample_rate.0 as f64, frames_hint);
        let mut staging: Vec<f32> = Vec::with_capacity(frames_hint * 2);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    player.drain_commands(&mut rx);
                    let frames = data.len() / channels.max(1);
                    let need = frames * 2;
                    if staging.len() < need {
                        staging.resize(need, 0.0);
                    }
                    let staging = &mut staging[..need];
                    player.process_block(staging, frames);
                    for frame in 0..frames {
                        let l = staging[frame * 2];
                        let r = staging[frame * 2 + 1];
                        let base = frame * channels;
                        if channels == 1 {
                            data[base] = 0.5 * (l + r);
                        } else {
                            data[base] = l;
                            data[base + 1] = r;
                            for c in 2..channels {
                                data[base + c] = 0.0;
                            }
                        }
                    }
                    position_cb.store(player.position_frames(), Ordering::Relaxed);
                    playing_cb.store(player.is_playing(), Ordering::Relaxed);
                },
                move |err| {
                    eprintln!("audio stream error: {err}");
                },
                None,
            )
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        stream.play().map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        Ok(AudioHost { stream, commands: tx, position, playing })
    }

    pub fn play(&mut self) -> bool {
        self.commands.send(TransportCommand::Play)
    }

    pub fn pause(&mut self) -> bool {
        self.commands.send(TransportCommand::Pause)
    }

    pub fn stop(&mut self) -> bool {
        self.commands.send(TransportCommand::Stop)
    }

    pub fn seek(&mut self, frame: u64) -> bool {
        self.commands.send(TransportCommand::Seek { frame })
    }

    pub fn position_frames(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
#[cfg(feature = "realtime")]
pub mod realtime;
```

- [ ] **Step 5: 写实时播放示例**

`crates/engine/examples/play_demo.rs`：

```rust
use std::thread;
use std::time::Duration;

use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Note;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::realtime::{AudioHost, RealtimeOptions};
use daw_party_engine::schedule::ScheduledEvents;

fn main() {
    let mut engine = Engine::new("realtime demo");
    let ev = engine.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
    let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
    engine.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
    let ev = engine.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
    let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
    for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 60].into_iter().enumerate() {
        engine.execute(
            Command::AddNote { track, clip, note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 } },
            Actor::Human,
        );
    }

    let schedule = ScheduledEvents::from_project(engine.project(), 48_000.0);
    let registry = PluginRegistry::with_builtins();
    let options = RealtimeOptions { sample_rate: 48_000.0, max_block: 2048, command_capacity: 64 };

    let mut host = match AudioHost::start(schedule, &registry, options) {
        Ok(host) => host,
        Err(e) => {
            eprintln!("could not start audio: {e}");
            return;
        }
    };

    println!("playing 2 bars...");
    host.play();
    thread::sleep(Duration::from_secs(4));
    host.stop();
    println!("stopped at frame {}", host.position_frames());
}
```

- [ ] **Step 6: 运行测试与示例并提交**

Run: `cargo test --workspace && cargo test --workspace --features realtime`
Expected: 全绿（`realtime` 测试在无设备环境接受错误路径）。

Run: `cargo run -p daw-party-engine --features realtime --example play_demo`
Expected: 能听到琶音；或打印 "could not start audio: ..."（无设备时）。

```bash
git add -A
git commit -F - <<'EOF'
feat: add cpal realtime host behind the realtime feature

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## 自检

**Spec 覆盖：**
- "cpal 实时播放同一张图" → Task 1/2/4/6（共用 `ScheduledEvents` + `Player`）。
- 传输控制（播放/暂停/停止/定位）→ Task 2（`TransportCommand`）、Task 3（无锁队列）、Task 6（`AudioHost` 对外 API）。
- 实时安全（回调内不分配）→ Task 2 的预分配 scratch、Task 3 的无锁队列。
- 离线 == 实时 → Task 4 的 `render_matches_player_at_different_block_size` 一致性测试（离线 `render` 与实时 `Player` 在 128 帧块下逐位一致）。
- 评审遗留的 `Engine` 载入路径 → Task 5。
- 未覆盖（后续计划）：VST3 宿主（Plan 3）、UI（Plan 4）、录音（Plan 5）、AI 接口（Plan 7）。

**类型一致性：** `ScheduledEvents/ScheduledTrack/ScheduledMidi`（Task 1）在 Task 2、4、6 一致使用；`Player::new/process_block/apply_command/seek/position_frames/is_playing/drain_commands` 在 Task 2、3、4、6 一致使用；`TransportCommand` 变体在 Task 2、3、6 一致；`command_queue/CommandSender/CommandReceiver`（Task 3）在 Task 6 一致使用。

**已知取舍：**
- `seek` 不追踪"已在发声的音符"（不重新触发、也不补发 NoteOff），首次版本可接受，实时播放中跳转可能有个别挂音；后续可引入活跃音符追踪。
- `Player::new` 会 `clone` 每轨事件表（控制线程一次性开销，不在回调内）。
- 被替换的 `Player`/调度在音频线程释放仍是已知的非严格 RT-safe 释放点；Plan 2 采用"命令队列 + 原地状态机"规避了每次换调度，后续如需热重载调度再引入归还式缓冲池。
- 回调内 `eprintln!` 仅用于流错误回调（非音频数据路径）。
