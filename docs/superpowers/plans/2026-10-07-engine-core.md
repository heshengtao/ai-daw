# daw-party 引擎核心实现计划（Plan 1：无头引擎）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立一个无 UI 的 Rust 引擎基础——工程模型 + 命令/事件总线 + 音序器 + 统一插件 ABI + 内置合成器，能把工程离线渲染为逐位一致的 WAV。

**Architecture:** 路线 B。引擎是不依赖 UI 的独立 crate，对外只暴露 Command/Event API；所有变更事件溯源；内置合成器实现与未来宿主插件相同的 ABI；实时与离线共用同一张音频图（本计划先实现离线渲染）。

**Tech Stack:** Rust（edition 2021）、`serde`/`serde_json`、`thiserror`、`hound`。内置合成器第一版用手写正弦振荡器 + 线性 ADSR（确定性、零 DSP 依赖）；`fundsp` 推迟到后续"音色/效果"计划引入。

## Global Constraints

- 工作区根 `Cargo.toml` 使用 `resolver = "2"`，成员为下方 crate。
- 所有 crate：`edition = "2021"`、`license = "Apache-2.0"`、`version = "0.1.0"`。
- 所有 crate 顶部 `#![forbid(unsafe_code)]`（未来 `host-vst3` 除外）。
- 依赖版本下限：`serde = "1"`、`serde_json = "1"`、`thiserror = "1"`、`hound = "3"`。
- 音频样本统一 `f32`；块大小常量 `pub const BLOCK_SIZE: usize = 256;`。
- 时间统一用 **拍（f64 beats）**；秒 = `beats * 60.0 / tempo`。
- 渲染确定性：同一工程 + 同一采样率 → 同一进程内两次渲染逐位相等；**禁止**使用系统时钟或随机数。
- 公共 API 一律加 `///` 文档注释（本项目要求"所有接口可暴露"）。
- 每个任务末尾运行 `cargo test --workspace` 必须全绿，然后提交。
- 提交信息前缀 `feat:` / `test:` / `chore:`，结尾固定带：
  `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`

## 文件结构

- `Cargo.toml` — 工作区根。
- `crates/plugin-api/src/lib.rs` — 重导出。
- `crates/plugin-api/src/descriptor.rs` — `PluginKind`、`IoLayout`、`PluginDescriptor`。
- `crates/plugin-api/src/params.rs` — `ParamId`、`ParamInfo`。
- `crates/plugin-api/src/event.rs` — `MidiEvent`、`TimedMidiEvent`。
- `crates/plugin-api/src/plugin.rs` — `ProcessContext`、`Plugin` trait。
- `crates/plugin-api/src/error.rs` — `PluginError`。
- `crates/synth/src/lib.rs` — `SynthPlugin`（内置合成器）。
- `crates/engine/src/lib.rs` — 重导出与 `BLOCK_SIZE`。
- `crates/engine/src/model/*.rs` — 工程领域模型。
- `crates/engine/src/command.rs` / `event.rs` — 命令与事件。
- `crates/engine/src/bus.rs` — 命令 → 事件 → 应用 + 撤销。
- `crates/engine/src/registry.rs` — 插件注册表。
- `crates/engine/src/sequencer.rs` — 每块 MIDI 事件。
- `crates/engine/src/audio.rs` — `AudioBuffer`。
- `crates/engine/src/render.rs` — 离线渲染。
- `crates/engine/src/wav.rs` — WAV 导出。
- `crates/engine/tests/*.rs` — 端到端测试。

---

### Task 1: 工作区与 `plugin-api` 基础类型

**Files:**
- Create: `Cargo.toml`、`crates/plugin-api/Cargo.toml`、`crates/plugin-api/src/lib.rs`、`crates/plugin-api/src/descriptor.rs`、`crates/plugin-api/src/params.rs`、`crates/plugin-api/src/event.rs`、`crates/plugin-api/src/error.rs`

**Interfaces:**
- Consumes: 无。
- Produces: `PluginKind`、`IoLayout`、`PluginDescriptor`、`ParamId`、`ParamInfo`、`MidiEvent`、`TimedMidiEvent`、`PluginError`。

- [ ] **Step 1: 建立工作区与 crate 骨架**

创建根 `Cargo.toml`：

```toml
[workspace]
resolver = "2"
members = ["crates/plugin-api", "crates/synth", "crates/engine"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
```

创建 `crates/plugin-api/Cargo.toml`：

```toml
[package]
name = "daw-party-plugin-api"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "1"
```

创建其余两个 crate 的最小 `Cargo.toml`（先不加依赖），保证工作区可编译：

`crates/synth/Cargo.toml`：

```toml
[package]
name = "daw-party-synth"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
daw-party-plugin-api = { path = "../plugin-api" }
```

`crates/engine/Cargo.toml`：

```toml
[package]
name = "daw-party-engine"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
daw-party-plugin-api = { path = "../plugin-api" }
daw-party-synth = { path = "../synth" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
hound = "3"
```

（`crates/synth/src/lib.rs`、`crates/engine/src/lib.rs` 先各写一行 `// placeholder` 之外的空文件即可编译失败——改为写 `pub fn _placeholder() {}`，下面步骤会替换。）

- [ ] **Step 2: 写失败测试**

`crates/plugin-api/src/descriptor.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_roundtrips_through_json() {
        let d = PluginDescriptor {
            id: "builtin.synth".into(),
            name: "Builtin Synth".into(),
            kind: PluginKind::Instrument,
            io: IoLayout { inputs: 0, outputs: 2 },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: PluginDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}
```

（`plugin-api` 需要 dev-dependency：在 `crates/plugin-api/Cargo.toml` 加 `[dev-dependencies] serde_json = "1"`。）

- [ ] **Step 3: 运行测试，确认失败**

Run: `cargo test -p daw-party-plugin-api`
Expected: 编译失败（`PluginDescriptor` 未定义）。

- [ ] **Step 4: 实现类型**

`crates/plugin-api/src/descriptor.rs`：

```rust
use serde::{Deserialize, Serialize};

/// 插件类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginKind {
    /// 音源（接收 MIDI，产出音频）。
    Instrument,
    /// 效果（接收音频，产出音频）。
    Effect,
}

/// 通道布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoLayout {
    pub inputs: u16,
    pub outputs: u16,
}

/// 插件描述符。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub kind: PluginKind,
    pub io: IoLayout,
}
```

`crates/plugin-api/src/params.rs`：

```rust
use serde::{Deserialize, Serialize};

/// 参数标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ParamId(pub String);

impl From<&str> for ParamId {
    fn from(s: &str) -> Self {
        ParamId(s.to_string())
    }
}

impl From<String> for ParamId {
    fn from(s: String) -> Self {
        ParamId(s)
    }
}

/// 参数元信息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub automatable: bool,
}
```

`crates/plugin-api/src/event.rs`：

```rust
use serde::{Deserialize, Serialize};

/// 单个 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    ControlChange { controller: u8, value: u8 },
}

/// 带块内帧偏移的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedMidiEvent {
    pub frame: u32,
    pub event: MidiEvent,
}
```

`crates/plugin-api/src/error.rs`：

```rust
use thiserror::Error;

/// 插件操作错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginError {
    #[error("plugin state is too short: expected at least {expected} bytes, got {got}")]
    StateTooShort { expected: usize, got: usize },
    #[error("invalid plugin state: {0}")]
    InvalidState(String),
}
```

`crates/plugin-api/src/lib.rs`：

```rust
#![forbid(unsafe_code)]

mod descriptor;
mod error;
mod event;
mod params;

pub use descriptor::*;
pub use error::*;
pub use event::*;
pub use params::*;
```

- [ ] **Step 5: 运行测试，确认通过并提交**

Run: `cargo test --workspace`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: scaffold workspace and plugin-api core types

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 2: `plugin-api` 的 `Plugin` trait

**Files:**
- Create: `crates/plugin-api/src/plugin.rs`
- Modify: `crates/plugin-api/src/lib.rs`（加 `mod plugin;` 与 `pub use plugin::*;`）

**Interfaces:**
- Consumes: Task 1 的 `PluginDescriptor`、`ParamInfo`、`ParamId`、`TimedMidiEvent`、`PluginError`。
- Produces: `ProcessContext`、`Plugin` trait。

- [ ] **Step 1: 写失败测试（crate 内 mock 插件）**

在 `crates/plugin-api/src/plugin.rs` 底部写测试；测试用一个把输入原样输出的 `GainPlugin` 验证 trait 契约：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct GainPlugin {
        gain: f32,
    }

    impl Plugin for GainPlugin {
        fn descriptor(&self) -> crate::PluginDescriptor {
            crate::PluginDescriptor {
                id: "test.gain".into(),
                name: "Gain".into(),
                kind: crate::PluginKind::Effect,
                io: crate::IoLayout { inputs: 2, outputs: 2 },
            }
        }
        fn params(&self) -> Vec<crate::ParamInfo> {
            vec![crate::ParamInfo {
                id: "gain".into(),
                name: "Gain".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                automatable: true,
            }]
        }
        fn get_param(&self, id: &crate::ParamId) -> Option<f32> {
            (id.0 == "gain").then_some(self.gain)
        }
        fn set_param(&mut self, id: &crate::ParamId, value: f32) {
            if id.0 == "gain" {
                self.gain = value;
            }
        }
        fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            ctx: &ProcessContext,
            out: &mut [&mut [f32]],
            _events: &[crate::TimedMidiEvent],
        ) {
            for ch in out.iter_mut() {
                for (i, s) in ch.iter_mut().enumerate() {
                    *s = (i as f32 + ctx.block_size as f32) * self.gain;
                }
            }
        }
        fn save_state(&self) -> Vec<u8> {
            self.gain.to_le_bytes().to_vec()
        }
        fn load_state(&mut self, state: &[u8]) -> Result<(), crate::PluginError> {
            if state.len() < 4 {
                return Err(crate::PluginError::StateTooShort { expected: 4, got: state.len() });
            }
            self.gain = f32::from_le_bytes(state[..4].try_into().unwrap());
            Ok(())
        }
    }

    #[test]
    fn plugin_process_writes_output_and_state_roundtrips() {
        let mut p = GainPlugin { gain: 0.5 };
        p.prepare(48_000.0, 4);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 4,
            tempo: 120.0,
            playing: true,
        };
        let mut left = [0.0f32; 4];
        let mut right = [0.0f32; 4];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert_eq!(left[0], 2.0);

        let state = p.save_state();
        let mut q = GainPlugin { gain: 0.0 };
        q.load_state(&state).unwrap();
        assert_eq!(q.gain, 0.5);

        assert!(q.load_state(&[0u8; 2]).is_err());
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-plugin-api plugin::`
Expected: 编译失败（`Plugin`、`ProcessContext` 未定义）。

- [ ] **Step 3: 实现 trait**

`crates/plugin-api/src/plugin.rs` 顶部（`#[cfg(test)]` 之前）：

```rust
use crate::{ParamId, ParamInfo, PluginDescriptor, PluginError, TimedMidiEvent};

/// 每次 `process` 的上下文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext {
    pub sample_rate: f64,
    pub block_size: usize,
    pub tempo: f64,
    pub playing: bool,
}

/// 统一的插件接口：内置合成器与外部宿主插件都实现它。
pub trait Plugin: Send {
    /// 描述符。
    fn descriptor(&self) -> PluginDescriptor;
    /// 参数列表。
    fn params(&self) -> Vec<ParamInfo>;
    /// 读取参数当前值。
    fn get_param(&self, id: &ParamId) -> Option<f32>;
    /// 设置参数值（实现方自行 clamp）。
    fn set_param(&mut self, id: &ParamId, value: f32);
    /// 采样率/块大小变化时调用。
    fn prepare(&mut self, sample_rate: f64, block_size: usize);
    /// 清空内部状态（延迟线、包络、声部）。
    fn reset(&mut self);
    /// 处理一个音频块。`out` 为平面缓冲（通道 × 帧）。
    fn process(&mut self, ctx: &ProcessContext, out: &mut [&mut [f32]], events: &[TimedMidiEvent]);
    /// 序列化状态。
    fn save_state(&self) -> Vec<u8>;
    /// 反序列化状态。
    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError>;
}
```

在 `crates/plugin-api/src/lib.rs` 增加：

```rust
mod plugin;
pub use plugin::*;
```

- [ ] **Step 4: 运行测试，确认通过**

Run: `cargo test -p daw-party-plugin-api`
Expected: PASS（2 个测试）。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add unified Plugin trait and process context

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 3: 内置合成器 `SynthPlugin`

**Files:**
- Create: `crates/synth/src/lib.rs`、`crates/synth/src/voice.rs`

**Interfaces:**
- Consumes: `daw_party_plugin_api::{Plugin, PluginDescriptor, PluginKind, IoLayout, ParamId, ParamInfo, ProcessContext, TimedMidiEvent, MidiEvent, PluginError}`。
- Produces: `SynthPlugin`（`Plugin` 实现），插件 id 固定为 `"builtin.synth"`；参数 id：`"gain"`（0..1，默认 0.8）、`"attack"`（0..1 秒，默认 0.01）、`"release"`（0..4 秒，默认 0.3）。

- [ ] **Step 1: 写失败测试**

`crates/synth/src/lib.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use daw_party_plugin_api::{MidiEvent, Plugin, ProcessContext, TimedMidiEvent};

    fn ctx(block: usize) -> ProcessContext {
        ProcessContext { sample_rate: 48_000.0, block_size: block, tempo: 120.0, playing: true }
    }

    #[test]
    fn silent_without_notes() {
        let mut s = SynthPlugin::new();
        s.prepare(48_000.0, 64);
        let mut l = [1.0f32; 64];
        let mut r = [1.0f32; 64];
        s.process(&ctx(64), &mut [&mut l, &mut r], &[]);
        assert!(l.iter().all(|x| *x == 0.0) && r.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn note_on_produces_signal_then_decays_after_note_off() {
        let mut s = SynthPlugin::new();
        s.prepare(48_000.0, 256);
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        s.process(
            &ctx(256),
            &mut [&mut l, &mut r],
            &[TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 100 } }],
        );
        let peak = l.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.05, "peak too low: {peak}");

        s.process(
            &ctx(256),
            &mut [&mut l, &mut r],
            &[TimedMidiEvent { frame: 0, event: MidiEvent::NoteOff { note: 69 } }],
        );
        for _ in 0..2000 {
            let mut l = [0.0f32; 256];
            let mut r = [0.0f32; 256];
            s.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        }
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        s.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        let peak = l.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak < 1e-4, "should have decayed to silence, peak={peak}");
    }

    #[test]
    fn renders_440hz_within_tolerance() {
        let mut s = SynthPlugin::new();
        s.prepare(48_000.0, 48_000);
        let mut l = [0.0f32; 48_000];
        let mut r = [0.0f32; 48_000];
        s.process(
            &ctx(48_000),
            &mut [&mut l, &mut r],
            &[TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }],
        );
        let crossings = l.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((crossings as i64 - 440).abs() <= 2, "crossings={crossings}");
    }

    #[test]
    fn state_roundtrips() {
        let mut s = SynthPlugin::new();
        s.set_param(&"gain".into(), 0.25);
        let state = s.save_state();
        let mut t = SynthPlugin::new();
        t.load_state(&state).unwrap();
        assert_eq!(t.get_param(&"gain".into()), Some(0.25));
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-synth`
Expected: 编译失败（`SynthPlugin` 未定义）。

- [ ] **Step 3: 实现声部**

`crates/synth/src/voice.rs`：

```rust
/// 一个声部的包络阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Sustain,
    Release,
}

/// 正弦 + 线性 ADSR 的复音声部。
#[derive(Debug, Clone, Copy)]
pub struct Voice {
    pub note: u8,
    pub velocity: f32,
    phase: f64,
    env: f32,
    pub stage: Stage,
}

impl Voice {
    pub const fn idle() -> Self {
        Voice { note: 0, velocity: 0.0, phase: 0.0, env: 0.0, stage: Stage::Idle }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.note = note;
        self.velocity = velocity as f32 / 127.0;
        self.phase = 0.0;
        self.env = 0.0;
        self.stage = Stage::Attack;
    }

    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// 返回该声部的单样本输出；`active` 表示仍需继续推进。
    pub fn next_sample(&mut self, sample_rate: f64, attack: f64, release: f64) -> (f32, bool) {
        if self.stage == Stage::Idle {
            return (0.0, false);
        }
        let a = if attack <= 0.0 { 1.0 } else { 1.0 / (attack * sample_rate) };
        let r = if release <= 0.0 { 1.0 } else { 1.0 / (release * sample_rate) };
        match self.stage {
            Stage::Attack => {
                self.env += a as f32;
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {}
            Stage::Release => {
                self.env -= r as f32;
                if self.env <= 0.0 {
                    self.env = 0.0;
                    self.stage = Stage::Idle;
                    return (0.0, false);
                }
            }
            Stage::Idle => {}
        }
        let freq = 440.0 * 2f64.powf((self.note as f64 - 69.0) / 12.0);
        self.phase += freq / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let s = (self.phase * std::f64::consts::TAU).sin() as f32;
        (s * self.env * self.velocity, true)
    }
}
```

- [ ] **Step 4: 实现合成器**

`crates/synth/src/lib.rs` 顶部（`#[cfg(test)]` 之前）：

```rust
#![forbid(unsafe_code)]

mod voice;

use daw_party_plugin_api::{
    IoLayout, MidiEvent, ParamId, ParamInfo, Plugin, PluginDescriptor, PluginError, PluginKind,
    ProcessContext, TimedMidiEvent,
};
use voice::Voice;

const MAX_VOICES: usize = 16;

/// 内置合成器：唯一内置音色插件。
pub struct SynthPlugin {
    voices: [Voice; MAX_VOICES],
    sample_rate: f64,
    gain: f32,
    attack: f32,
    release: f32,
}

impl Default for SynthPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SynthPlugin {
    pub fn new() -> Self {
        SynthPlugin {
            voices: [Voice::idle(); MAX_VOICES],
            sample_rate: 48_000.0,
            gain: 0.8,
            attack: 0.01,
            release: 0.3,
        }
    }

    fn handle(&mut self, event: MidiEvent) {
        match event {
            MidiEvent::NoteOn { note, velocity } => {
                if velocity == 0 {
                    for v in self.voices.iter_mut().filter(|v| v.note == note) {
                        v.note_off();
                    }
                    return;
                }
                let slot = self
                    .voices
                    .iter()
                    .position(|v| v.stage == voice::Stage::Idle)
                    .unwrap_or(0);
                self.voices[slot].note_on(note, velocity);
            }
            MidiEvent::NoteOff { note } => {
                for v in self.voices.iter_mut().filter(|v| v.note == note) {
                    v.note_off();
                }
            }
            MidiEvent::ControlChange { .. } => {}
        }
    }
}

impl Plugin for SynthPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: "builtin.synth".into(),
            name: "Builtin Synth".into(),
            kind: PluginKind::Instrument,
            io: IoLayout { inputs: 0, outputs: 2 },
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        vec![
            ParamInfo { id: "gain".into(), name: "Gain".into(), min: 0.0, max: 1.0, default: 0.8, automatable: true },
            ParamInfo { id: "attack".into(), name: "Attack".into(), min: 0.0, max: 1.0, default: 0.01, automatable: true },
            ParamInfo { id: "release".into(), name: "Release".into(), min: 0.0, max: 4.0, default: 0.3, automatable: true },
        ]
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        match id.0.as_str() {
            "gain" => Some(self.gain),
            "attack" => Some(self.attack),
            "release" => Some(self.release),
            _ => None,
        }
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        match id.0.as_str() {
            "gain" => self.gain = value.clamp(0.0, 1.0),
            "attack" => self.attack = value.clamp(0.0, 1.0),
            "release" => self.release = value.clamp(0.0, 4.0),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.voices = [Voice::idle(); MAX_VOICES];
    }

    fn process(&mut self, ctx: &ProcessContext, out: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
        let frames = out.first().map_or(0, |c| c.len());
        for frame in 0..frames {
            for ev in events.iter().filter(|e| e.frame as usize == frame) {
                self.handle(ev.event);
            }
            let mut mix = 0.0f32;
            for v in self.voices.iter_mut() {
                let (s, _) = v.next_sample(self.sample_rate, self.attack as f64, self.release as f64);
                mix += s;
            }
            mix = (mix * self.gain).clamp(-1.0, 1.0);
            for ch in out.iter_mut() {
                ch[frame] = mix;
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12);
        out.extend_from_slice(&self.gain.to_le_bytes());
        out.extend_from_slice(&self.attack.to_le_bytes());
        out.extend_from_slice(&self.release.to_le_bytes());
        out
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.len() < 12 {
            return Err(PluginError::StateTooShort { expected: 12, got: state.len() });
        }
        self.gain = f32::from_le_bytes(state[0..4].try_into().unwrap());
        self.attack = f32::from_le_bytes(state[4..8].try_into().unwrap());
        self.release = f32::from_le_bytes(state[8..12].try_into().unwrap());
        Ok(())
    }
}
```

- [ ] **Step 5: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-synth`
Expected: PASS（4 个测试）。

```bash
git add -A
git commit -F - <<'EOF'
feat: add built-in synth plugin implementing the plugin ABI

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 4: 工程领域模型

**Files:**
- Create: `crates/engine/src/model/mod.rs`、`crates/engine/src/model/project.rs`、`crates/engine/src/model/track.rs`、`crates/engine/src/model/clip.rs`、`crates/engine/src/model/transport.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `serde`。
- Produces: `TrackId`、`ClipId`、`Note`、`Clip`、`PluginInstance`、`MixerChannel`、`Track`、`Transport`、`Project`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/model/project.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::track::{MixerChannel, Track};
    use crate::model::Clip;

    #[test]
    fn project_roundtrips_through_json() {
        let mut p = Project::new("demo");
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            clips: vec![],
            mixer: MixerChannel::default(),
        });
        let json = serde_json::to_string_pretty(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
        assert_eq!(back.tracks[0].name, "Lead");
    }

    #[test]
    fn length_beats_is_max_clip_end() {
        let mut p = Project::new("demo");
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            clips: vec![Clip { id: crate::model::ClipId(1), start: 2.0, length: 3.0, notes: vec![] }],
            mixer: MixerChannel::default(),
        });
        assert_eq!(p.length_beats(), 5.0);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine model::`
Expected: 编译失败。

- [ ] **Step 3: 实现模型**

`crates/engine/src/model/track.rs`：

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::clip::Clip;

/// 轨道标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TrackId(pub u64);

/// 插件实例（工程内保存的插件引用与状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginInstance {
    pub plugin_id: String,
    pub params: BTreeMap<String, f32>,
    pub state: Vec<u8>,
}

/// 混音通道。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MixerChannel {
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Default for MixerChannel {
    fn default() -> Self {
        MixerChannel { gain_db: 0.0, pan: 0.0, mute: false, solo: false }
    }
}

/// 轨道。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub instrument: Option<PluginInstance>,
    pub clips: Vec<Clip>,
    pub mixer: MixerChannel,
}
```

`crates/engine/src/model/clip.rs`：

```rust
use serde::{Deserialize, Serialize};

/// 片段标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClipId(pub u64);

/// 音符；位置与时长单位为拍。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub start: f64,
    pub duration: f64,
    pub velocity: u8,
}

/// MIDI 片段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub id: ClipId,
    pub start: f64,
    pub length: f64,
    pub notes: Vec<Note>,
}
```

`crates/engine/src/model/transport.rs`：

```rust
use serde::{Deserialize, Serialize};

/// 传输设置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transport {
    pub tempo: f64,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
}

impl Default for Transport {
    fn default() -> Self {
        Transport { tempo: 120.0, time_sig_num: 4, time_sig_den: 4 }
    }
}
```

`crates/engine/src/model/project.rs`：

```rust
use serde::{Deserialize, Serialize};

use super::track::{MixerChannel, Track, TrackId};
use super::transport::Transport;

/// 工程：领域聚合根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub transport: Transport,
    pub tracks: Vec<Track>,
    pub master: MixerChannel,
}

impl Project {
    pub fn new(name: impl Into<String>) -> Self {
        Project {
            name: name.into(),
            transport: Transport::default(),
            tracks: vec![],
            master: MixerChannel::default(),
        }
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// 工程内容总时长（拍），取所有片段结束位置的最大值。
    pub fn length_beats(&self) -> f64 {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + c.length)
            .fold(0.0f64, f64::max)
    }
}
```

`crates/engine/src/model/mod.rs`：

```rust
pub mod clip;
pub mod project;
pub mod track;
pub mod transport;

pub use clip::{Clip, ClipId, Note};
pub use project::Project;
pub use track::{MixerChannel, PluginInstance, Track, TrackId};
pub use transport::Transport;
```

`crates/engine/src/lib.rs`：

```rust
#![forbid(unsafe_code)]

pub mod model;

/// 音频块帧数。
pub const BLOCK_SIZE: usize = 256;

/// 拍 → 秒。
pub fn beats_to_seconds(beats: f64, tempo: f64) -> f64 {
    beats * 60.0 / tempo
}
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine model::`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add project domain model with json roundtrip

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 5: 命令 / 事件与状态应用

**Files:**
- Create: `crates/engine/src/command.rs`、`crates/engine/src/event.rs`、`crates/engine/src/apply.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `model::{Project, Track, Clip, Note, PluginInstance, TrackId, ClipId, MixerChannel}`。
- Produces: `Command`、`Event`、`Actor`、`apply(project, event)`、`EngineError`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/apply.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::event::{Actor, Event};
    use crate::model::{Note, Project};

    #[test]
    fn add_track_then_clip_then_note() {
        let mut p = Project::new("t");
        let t = p.alloc_track_id();
        apply(&mut p, &Event::TrackAdded { actor: Actor::Human, id: t, name: "Keys".into() });
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.track(t).unwrap().name, "Keys");

        let c = p.alloc_clip_id();
        apply(&mut p, &Event::ClipAdded { actor: Actor::Human, track: t, clip: c, start: 0.0, length: 4.0 });
        assert_eq!(p.track(t).unwrap().clips.len(), 1);

        apply(
            &mut p,
            &Event::NoteAdded {
                actor: Actor::Human,
                track: t,
                clip: c,
                note: Note { pitch: 60, start: 0.0, duration: 1.0, velocity: 100 },
            },
        );
        assert_eq!(p.track(t).unwrap().clips[0].notes.len(), 1);

        let _ = Command::SetTempo(140.0);
    }

    #[test]
    fn set_tempo_event_updates_transport() {
        let mut p = Project::new("t");
        apply(&mut p, &Event::TempoChanged { actor: Actor::Human, tempo: 90.0 });
        assert_eq!(p.transport.tempo, 90.0);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine apply::`
Expected: 编译失败。

- [ ] **Step 3: 实现**

`crates/engine/src/command.rs`：

```rust
use crate::model::{ClipId, Note, TrackId};

/// 命令：期望的状态变更请求。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    SetTempo(f64),
    AddTrack { name: String },
    RemoveTrack(TrackId),
    SetTrackInstrument { track: TrackId, plugin_id: String },
    AddClip { track: TrackId, start: f64, length: f64 },
    RemoveClip { track: TrackId, clip: ClipId },
    AddNote { track: TrackId, clip: ClipId, note: Note },
    SetGain { track: TrackId, gain_db: f32 },
    SetPan { track: TrackId, pan: f32 },
}
```

`crates/engine/src/event.rs`：

```rust
use crate::model::{ClipId, Note, TrackId};

/// 变更来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Human,
    Agent(String),
}

/// 事件：一次已发生、可重放的状态变更。
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    TempoChanged { actor: Actor, tempo: f64 },
    TrackAdded { actor: Actor, id: TrackId, name: String },
    TrackRemoved { actor: Actor, id: TrackId },
    TrackInstrumentSet { actor: Actor, track: TrackId, plugin_id: String },
    ClipAdded { actor: Actor, track: TrackId, clip: ClipId, start: f64, length: f64 },
    ClipRemoved { actor: Actor, track: TrackId, clip: ClipId },
    NoteAdded { actor: Actor, track: TrackId, clip: ClipId, note: Note },
    GainChanged { actor: Actor, track: TrackId, gain_db: f32 },
    PanChanged { actor: Actor, track: TrackId, pan: f32 },
}

impl Event {
    pub fn actor(&self) -> &Actor {
        match self {
            Event::TempoChanged { actor, .. }
            | Event::TrackAdded { actor, .. }
            | Event::TrackRemoved { actor, .. }
            | Event::TrackInstrumentSet { actor, .. }
            | Event::ClipAdded { actor, .. }
            | Event::ClipRemoved { actor, .. }
            | Event::NoteAdded { actor, .. }
            | Event::GainChanged { actor, .. }
            | Event::PanChanged { actor, .. } => actor,
        }
    }
}
```

`crates/engine/src/apply.rs`：

```rust
use crate::event::Event;
use crate::model::{Clip, PluginInstance, Project};

/// 把一个事件应用到工程状态。
pub fn apply(project: &mut Project, event: &Event) {
    match event {
        Event::TempoChanged { tempo, .. } => project.transport.tempo = *tempo,
        Event::TrackAdded { id, name, .. } => {
            project.tracks.push(crate::model::Track {
                id: *id,
                name: name.clone(),
                instrument: None,
                clips: vec![],
                mixer: crate::model::MixerChannel::default(),
            });
        }
        Event::TrackRemoved { id, .. } => project.tracks.retain(|t| t.id != *id),
        Event::TrackInstrumentSet { track, plugin_id, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.instrument = Some(PluginInstance {
                    plugin_id: plugin_id.clone(),
                    params: Default::default(),
                    state: vec![],
                });
            }
        }
        Event::ClipAdded { track, clip, start, length, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.push(Clip { id: *clip, start: *start, length: *length, notes: vec![] });
            }
        }
        Event::ClipRemoved { track, clip, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.clips.retain(|c| c.id != *clip);
            }
        }
        Event::NoteAdded { track, clip, note, .. } => {
            if let Some(t) = project.track_mut(*track) {
                if let Some(c) = t.clips.iter_mut().find(|c| c.id == *clip) {
                    c.notes.push(*note);
                }
            }
        }
        Event::GainChanged { track, gain_db, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.gain_db = *gain_db;
            }
        }
        Event::PanChanged { track, pan, .. } => {
            if let Some(t) = project.track_mut(*track) {
                t.mixer.pan = *pan;
            }
        }
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod apply;
pub mod command;
pub mod event;
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add command/event model and state application

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 6: 命令总线与撤销

**Files:**
- Create: `crates/engine/src/bus.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `Command`、`Event`、`Actor`、`apply`、`Project`。
- Produces: `Engine`（`new`、`project`、`execute`、`undo`、`undo_depth`）。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/bus.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::event::Actor;

    #[test]
    fn execute_returns_events_and_mutates_state() {
        let mut e = Engine::new("demo");
        let events = e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], crate::event::Event::TrackAdded { .. }));
        assert_eq!(e.project().tracks.len(), 1);
    }

    #[test]
    fn undo_reverts_last_change() {
        let mut e = Engine::new("demo");
        e.execute(Command::AddTrack { name: "Bass".into() }, Actor::Human);
        e.execute(Command::SetTempo(150.0), Actor::Human);
        assert_eq!(e.project().transport.tempo, 150.0);
        assert!(e.undo());
        assert_eq!(e.project().transport.tempo, 120.0);
        assert!(e.undo());
        assert!(e.project().tracks.is_empty());
        assert!(!e.undo());
    }

    #[test]
    fn add_clip_uses_allocated_ids_and_is_consistent() {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Keys".into() }, Actor::Human);
        let track = match ev[0] { crate::event::Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 8.0 }, Actor::Human);
        let clip = match ev[0] { crate::event::Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        assert_eq!(e.project().track(track).unwrap().clips[0].id, clip);
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine bus::`
Expected: 编译失败（`Engine` 未定义）。

- [ ] **Step 3: 实现总线**

`crates/engine/src/bus.rs`：

```rust
use crate::apply::apply;
use crate::command::Command;
use crate::event::{Actor, Event};
use crate::model::{ClipId, Project, TrackId};

/// 引擎：持有工程状态、执行命令、产出事件、支持撤销。
pub struct Engine {
    project: Project,
    history: Vec<Vec<Event>>,
    next_track_id: u64,
    next_clip_id: u64,
}

impl Engine {
    pub fn new(name: impl Into<String>) -> Self {
        Engine {
            project: Project::new(name),
            history: vec![],
            next_track_id: 1,
            next_clip_id: 1,
        }
    }

    fn alloc_track_id(&mut self) -> TrackId {
        let id = TrackId(self.next_track_id);
        self.next_track_id += 1;
        id
    }

    fn alloc_clip_id(&mut self) -> ClipId {
        let id = ClipId(self.next_clip_id);
        self.next_clip_id += 1;
        id
    }

    pub fn project(&self) -> &Project {
        &self.project
    }

    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }

    /// 执行命令：产出事件、应用到状态、记录历史。
    pub fn execute(&mut self, command: Command, actor: Actor) -> Vec<Event> {
        let events = self.translate(&command, &actor);
        for ev in &events {
            apply(&mut self.project, ev);
        }
        if !events.is_empty() {
            self.history.push(events.clone());
        }
        events
    }

    /// 撤销最后一次命令；成功返回 true。
    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            None => false,
            Some(events) => {
                self.project = Project::new(self.project.name.clone());
                let remaining: Vec<Vec<Event>> = self.history.clone();
                for batch in remaining {
                    for ev in batch {
                        apply(&mut self.project, &ev);
                    }
                }
                let _ = events;
                true
            }
        }
    }

    fn translate(&mut self, command: &Command, actor: &Actor) -> Vec<Event> {
        match command {
            Command::SetTempo(tempo) => {
                vec![Event::TempoChanged { actor: actor.clone(), tempo: *tempo }]
            }
            Command::AddTrack { name } => {
                let id = self.alloc_track_id();
                vec![Event::TrackAdded { actor: actor.clone(), id, name: name.clone() }]
            }
            Command::RemoveTrack(track) => {
                vec![Event::TrackRemoved { actor: actor.clone(), id: *track }]
            }
            Command::SetTrackInstrument { track, plugin_id } => {
                vec![Event::TrackInstrumentSet {
                    actor: actor.clone(),
                    track: *track,
                    plugin_id: plugin_id.clone(),
                }]
            }
            Command::AddClip { track, start, length } => {
                let clip = self.alloc_clip_id();
                vec![Event::ClipAdded { actor: actor.clone(), track: *track, clip, start: *start, length: *length }]
            }
            Command::RemoveClip { track, clip } => {
                vec![Event::ClipRemoved { actor: actor.clone(), track: *track, clip: *clip }]
            }
            Command::AddNote { track, clip, note } => {
                vec![Event::NoteAdded { actor: actor.clone(), track: *track, clip: *clip, note: *note }]
            }
            Command::SetGain { track, gain_db } => {
                vec![Event::GainChanged { actor: actor.clone(), track: *track, gain_db: *gain_db }]
            }
            Command::SetPan { track, pan } => {
                vec![Event::PanChanged { actor: actor.clone(), track: *track, pan: *pan }]
            }
        }
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod bus;
pub use bus::Engine;
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add command bus with event sourcing and undo

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 7: 插件注册表

**Files:**
- Create: `crates/engine/src/registry.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `daw_party_plugin_api::Plugin`、`daw_party_synth::SynthPlugin`。
- Produces: `PluginRegistry`（`with_builtins`、`register`、`create`、`ids`）。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/registry.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use daw_party_plugin_api::Plugin;

    #[test]
    fn builtins_include_synth() {
        let r = PluginRegistry::with_builtins();
        assert!(r.ids().contains(&"builtin.synth".to_string()));
    }

    #[test]
    fn create_returns_configured_instance() {
        let r = PluginRegistry::with_builtins();
        let p = r.create("builtin.synth").expect("synth exists");
        assert_eq!(p.descriptor().id, "builtin.synth");
        assert_eq!(p.get_param(&"gain".into()), Some(0.8));
    }

    #[test]
    fn create_unknown_returns_none() {
        let r = PluginRegistry::with_builtins();
        assert!(r.create("nope").is_none());
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine registry::`
Expected: 编译失败。

- [ ] **Step 3: 实现注册表**

`crates/engine/src/registry.rs`：

```rust
use std::collections::HashMap;

use daw_party_plugin_api::Plugin;

type Factory = Box<dyn Fn() -> Box<dyn Plugin> + Send + Sync>;

/// 插件注册表：按 id 创建插件实例。
#[derive(Default)]
pub struct PluginRegistry {
    factories: HashMap<String, Factory>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        PluginRegistry { factories: HashMap::new() }
    }

    /// 内置插件：目前只有合成器。
    pub fn with_builtins() -> Self {
        let mut r = PluginRegistry::new();
        r.register("builtin.synth", || Box::new(daw_party_synth::SynthPlugin::new()));
        r
    }

    pub fn register(&mut self, id: &str, factory: impl Fn() -> Box<dyn Plugin> + Send + Sync + 'static) {
        self.factories.insert(id.to_string(), Box::new(factory));
    }

    pub fn create(&self, id: &str) -> Option<Box<dyn Plugin>> {
        self.factories.get(id).map(|f| f())
    }

    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.factories.keys().cloned().collect();
        v.sort();
        v
    }
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod registry;
pub use registry::PluginRegistry;
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add plugin registry with builtin synth

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 8: 音序器（每块 MIDI 事件）

**Files:**
- Create: `crates/engine/src/sequencer.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `model::{Track, Clip, Note}`、`daw_party_plugin_api::{MidiEvent, TimedMidiEvent}`。
- Produces: `collect_track_events(track, block_start_beats, block_frames, sample_rate, tempo) -> Vec<TimedMidiEvent>`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/sequencer.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, Track, TrackId};
    use daw_party_plugin_api::MidiEvent;

    fn track_with_note() -> Track {
        Track {
            id: TrackId(1),
            name: "t".into(),
            instrument: None,
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration: 1.0, velocity: 100 }],
            }],
            mixer: Default::default(),
        }
    }

    #[test]
    fn note_on_and_off_land_in_correct_block() {
        let t = track_with_note();
        // 120bpm, 48kHz: 1 beat = 0.5s = 24000 frames.
        let events = collect_track_events(&t, 0.0, 48_000, 48_000.0, 120.0);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].frame, 24_000);
        assert!(matches!(events[0].event, MidiEvent::NoteOn { note: 60, velocity: 100 }));
        assert_eq!(events[1].frame, 48_000);
        assert!(matches!(events[1].event, MidiEvent::NoteOff { note: 60 }));
    }

    #[test]
    fn notes_outside_block_are_excluded() {
        let t = track_with_note();
        let events = collect_track_events(&t, 0.25, 1_000, 48_000.0, 120.0);
        assert!(events.is_empty());
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine sequencer::`
Expected: 编译失败。

- [ ] **Step 3: 实现音序器**

`crates/engine/src/sequencer.rs`：

```rust
use daw_party_plugin_api::{MidiEvent, TimedMidiEvent};

use crate::beats_to_seconds;
use crate::model::Track;

/// 收集某轨道在给定块内应发出的 MIDI 事件（带块内帧偏移）。
/// `block_start_beats` 为块起点的全局拍位置；`block_frames` 为块帧数。
pub fn collect_track_events(
    track: &Track,
    block_start_beats: f64,
    block_frames: usize,
    sample_rate: f64,
    tempo: f64,
) -> Vec<TimedMidiEvent> {
    let block_start_sec = beats_to_seconds(block_start_beats, tempo);
    let block_end_sec = block_start_sec + block_frames as f64 / sample_rate;
    let mut out = Vec::new();

    for clip in &track.clips {
        for note in &clip.notes {
            let start_beat = clip.start + note.start;
            let end_beat = start_beat + note.duration;
            let start_sec = beats_to_seconds(start_beat, tempo);
            let end_sec = beats_to_seconds(end_beat, tempo);

            if start_sec >= block_start_sec && start_sec < block_end_sec {
                let frame = ((start_sec - block_start_sec) * sample_rate).round() as u32;
                out.push(TimedMidiEvent {
                    frame,
                    event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                });
            }
            if end_sec >= block_start_sec && end_sec < block_end_sec {
                let frame = ((end_sec - block_start_sec) * sample_rate).round() as u32;
                out.push(TimedMidiEvent { frame, event: MidiEvent::NoteOff { note: note.pitch } });
            }
        }
    }

    out.sort_by_key(|e| e.frame);
    out
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod sequencer;
```

- [ ] **Step 4: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add block-level midi sequencer

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 9: 音频缓冲与离线渲染

**Files:**
- Create: `crates/engine/src/audio.rs`、`crates/engine/src/render.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `Project`、`PluginRegistry`、`sequencer::collect_track_events`、`beats_to_seconds`、`BLOCK_SIZE`。
- Produces: `AudioBuffer`（`silence`、`frames`、`channels`、`sample`、`data`、`peak`）、`RenderOptions`（`sample_rate`、`tail_seconds`）、`render(project, registry, options) -> AudioBuffer`。

- [ ] **Step 1: 写失败测试**

`crates/engine/src/render.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Engine;
    use crate::command::Command;
    use crate::event::{Actor, Event};
    use crate::model::Note;
    use crate::registry::PluginRegistry;

    fn demo() -> crate::model::Project {
        let mut e = Engine::new("demo");
        let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
        e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 2.0 }, Actor::Human);
        let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
        e.execute(
            Command::AddNote { track, clip, note: Note { pitch: 69, start: 0.0, duration: 1.0, velocity: 100 } },
            Actor::Human,
        );
        e.project().clone()
    }

    #[test]
    fn render_duration_matches_project_length_plus_tail() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.5 };
        let buf = render(&project, &PluginRegistry::with_builtins(), &opts);
        let expected = (beats_to_seconds(project.length_beats(), project.transport.tempo) * 48_000.0) as usize
            + (0.5 * 48_000.0) as usize;
        assert!((buf.frames() as i64 - expected as i64).abs() <= 1);
        assert_eq!(buf.channels(), 2);
    }

    #[test]
    fn render_is_deterministic_and_audible() {
        let project = demo();
        let opts = RenderOptions { sample_rate: 48_000.0, tail_seconds: 0.2 };
        let r = PluginRegistry::with_builtins();
        let a = render(&project, &r, &opts);
        let b = render(&project, &r, &opts);
        assert_eq!(a.data, b.data, "render must be bit-identical");
        assert!(a.peak() > 0.05, "should be audible");
        assert!(a.data.iter().all(|s| s.is_finite()));
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine render::`
Expected: 编译失败。

- [ ] **Step 3: 实现音频缓冲**

`crates/engine/src/audio.rs`：

```rust
/// 交错（interleaved）音频缓冲。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    channels: usize,
    frames: usize,
    data: Vec<f32>,
}

impl AudioBuffer {
    pub fn silence(channels: usize, frames: usize) -> Self {
        AudioBuffer { channels, frames, data: vec![0.0; channels * frames] }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn data(&self) -> &[f32] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [f32] {
        &mut self.data
    }

    pub fn sample(&self, channel: usize, frame: usize) -> f32 {
        self.data[frame * self.channels + channel]
    }

    pub fn set_sample(&mut self, channel: usize, frame: usize, value: f32) {
        self.data[frame * self.channels + channel] = value;
    }

    pub fn peak(&self) -> f32 {
        self.data.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }
}
```

- [ ] **Step 4: 实现离线渲染**

`crates/engine/src/render.rs`：

```rust
use daw_party_plugin_api::ProcessContext;

use crate::audio::AudioBuffer;
use crate::beats_to_seconds;
use crate::model::Project;
use crate::registry::PluginRegistry;
use crate::sequencer::collect_track_events;
use crate::BLOCK_SIZE;

/// 离线渲染选项。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderOptions {
    pub sample_rate: f64,
    pub tail_seconds: f64,
}

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// 把工程离线渲染为立体声交错缓冲。
pub fn render(project: &Project, registry: &PluginRegistry, options: &RenderOptions) -> AudioBuffer {
    let sr = options.sample_rate;
    let tempo = project.transport.tempo;
    let body_sec = beats_to_seconds(project.length_beats(), tempo);
    let total_frames = ((body_sec + options.tail_seconds) * sr) as usize;

    let mut master = AudioBuffer::silence(2, total_frames);

    for track in &project.tracks {
        if track.mixer.mute {
            continue;
        }
        let Some(instance) = &track.instrument else { continue };
        let Some(mut plugin) = registry.create(&instance.plugin_id) else { continue };
        plugin.prepare(sr, BLOCK_SIZE);
        for (id, value) in &instance.params {
            plugin.set_param(&id.clone().into(), *value);
        }

        let gain = db_to_linear(track.mixer.gain_db);
        let pan = track.mixer.pan.clamp(-1.0, 1.0);
        let left_gain = gain * ((1.0 - pan) * 0.5).sqrt();
        let right_gain = gain * ((1.0 + pan) * 0.5).sqrt();

        let mut block_start = 0usize;
        while block_start < total_frames {
            let frames = BLOCK_SIZE.min(total_frames - block_start);
            let block_start_beats = (block_start as f64 / sr) * tempo / 60.0;
            let events = collect_track_events(track, block_start_beats, frames, sr, tempo);

            let mut left = vec![0.0f32; frames];
            let mut right = vec![0.0f32; frames];
            let ctx = ProcessContext {
                sample_rate: sr,
                block_size: frames,
                tempo,
                playing: true,
            };
            plugin.process(&ctx, &mut [&mut left, &mut right], &events);

            for i in 0..frames {
                master.set_sample(0, block_start + i, master.sample(0, block_start + i) + left[i] * left_gain);
                master.set_sample(1, block_start + i, master.sample(1, block_start + i) + right[i] * right_gain);
            }
            block_start += frames;
        }
    }

    master
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod audio;
pub mod render;
```

- [ ] **Step 5: 运行测试，确认通过并提交**

Run: `cargo test -p daw-party-engine`
Expected: PASS。

```bash
git add -A
git commit -F - <<'EOF'
feat: add audio buffer and deterministic offline render

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 10: WAV 导出与端到端测试

**Files:**
- Create: `crates/engine/src/wav.rs`、`crates/engine/tests/render_to_wav.rs`
- Modify: `crates/engine/src/lib.rs`

**Interfaces:**
- Consumes: `AudioBuffer`、`hound`。
- Produces: `write_wav(path: impl AsRef<Path>, buffer: &AudioBuffer, sample_rate: u32) -> Result<(), EngineError>`、`EngineError`。

- [ ] **Step 1: 写失败测试**

`crates/engine/tests/render_to_wav.rs`：

```rust
use daw_party_engine::audio::AudioBuffer;
use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Note;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render, RenderOptions};
use daw_party_engine::wav::write_wav;

fn demo() -> daw_party_engine::model::Project {
    let mut e = Engine::new("e2e");
    let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
    let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
    e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
    let ev = e.execute(Command::AddClip { track, start: 0.0, length: 2.0 }, Actor::Human);
    let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
    // a simple C major arpeggio
    for (i, pitch) in [60u8, 64, 67, 72].into_iter().enumerate() {
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 },
            },
            Actor::Human,
        );
    }
    e.project().clone()
}

#[test]
fn renders_project_to_readable_wav() {
    let project = demo();
    let opts = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5 };
    let buffer = render(&project, &PluginRegistry::with_builtins(), &opts);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.wav");
    write_wav(&path, &buffer, 44_100).unwrap();

    let reader = hound::WavReader::open(&path).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_rate, 44_100);
    assert_eq!(reader.len(), buffer.frames() as u32 * 2);

    let _ = AudioBuffer::silence(2, 0);
}
```

在 `crates/engine/Cargo.toml` 加 dev-dependency：

```toml
[dev-dependencies]
hound = "3"
tempfile = "3"
```

- [ ] **Step 2: 运行测试，确认失败**

Run: `cargo test -p daw-party-engine --test render_to_wav`
Expected: 编译失败（`write_wav` 未定义）。

- [ ] **Step 3: 实现 WAV 导出**

`crates/engine/src/wav.rs`：

```rust
use std::path::Path;

use thiserror::Error;

use crate::audio::AudioBuffer;

/// 引擎错误。
#[derive(Debug, Error)]
pub enum EngineError {
    #[error("wav io error: {0}")]
    Wav(#[from] hound::Error),
}

/// 把交错缓冲写成 16-bit PCM WAV。
pub fn write_wav(path: impl AsRef<Path>, buffer: &AudioBuffer, sample_rate: u32) -> Result<(), EngineError> {
    let spec = hound::WavSpec {
        channels: buffer.channels() as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for &s in buffer.data() {
        let clamped = s.clamp(-1.0, 1.0);
        writer.write_sample((clamped * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
```

在 `crates/engine/src/lib.rs` 增加：

```rust
pub mod wav;
```

- [ ] **Step 4: 运行全部测试，确认通过**

Run: `cargo test --workspace`
Expected: PASS（含 `render_to_wav` 端到端测试）。

- [ ] **Step 5: 写一个把 demo 存成工程文件的可执行示例并提交**

创建 `crates/engine/examples/render_demo.rs`：

```rust
use daw_party_engine::bus::Engine;
use daw_party_engine::command::Command;
use daw_party_engine::event::{Actor, Event};
use daw_party_engine::model::Note;
use daw_party_engine::registry::PluginRegistry;
use daw_party_engine::render::{render, RenderOptions};
use daw_party_engine::wav::write_wav;

fn main() {
    let mut e = Engine::new("demo");
    let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
    let track = match ev[0] { Event::TrackAdded { id, .. } => id, _ => unreachable!() };
    e.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
    let ev = e.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
    let clip = match ev[0] { Event::ClipAdded { clip, .. } => clip, _ => unreachable!() };
    for (i, pitch) in [60u8, 64, 67, 72, 67, 64].into_iter().enumerate() {
        e.execute(
            Command::AddNote { track, clip, note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 } },
            Actor::Human,
        );
    }
    let buffer = render(e.project(), &PluginRegistry::with_builtins(), &RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5 });
    write_wav("demo.wav", &buffer, 44_100).unwrap();
    println!("wrote demo.wav ({} frames)", buffer.frames());
}
```

Run: `cargo run -p daw-party-engine --example render_demo`
Expected: 打印 `wrote demo.wav (...)` 且生成 `demo.wav`。把 `demo.wav` 加入 `.gitignore`。

```bash
git add -A
git commit -F - <<'EOF'
feat: add wav export, e2e test, and render demo example

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## 自检

**Spec 覆盖：**
- 工程模型（JSON 事件溯源）→ Task 4、5、6。
- 命令/事件总线 + actor 标签 → Task 5、6。
- 音序器 → Task 8。
- 统一插件 ABI → Task 1、2。
- 内置合成器实现同一 ABI → Task 3。
- 插件注册表 → Task 7。
- 离线渲染（逐位一致）→ Task 9。
- 导出 WAV → Task 10。
- 确定性测试 → Task 9、10。
- 未覆盖（属后续计划）：cpal 实时播放、VST3 宿主、UI、音频片段/录音、AI 接口层。

**类型一致性：** `Plugin` trait 方法（`prepare`/`reset`/`process`/`save_state`/`load_state`/`params`/`get_param`/`set_param`/`descriptor`）在 Task 2 定义，Task 3、7、9 一致使用；`Event` 的字段名与 `apply` 的匹配臂一致；`RenderOptions`、`AudioBuffer`、`PluginRegistry` 的方法名在 Task 9、10 一致。

**已知取舍：** 内置合成器第一版为手写正弦 + 线性 ADSR（确定性优先、零 DSP 依赖），`fundsp` 留待后续音色/效果计划引入；撤销采用"重放历史"实现（工程规模小，简单正确），后续可优化为逆操作；**id 计数器保存在 `Engine` 且单调递增**，避免撤销重放时的 id 复用与实体重复。
