#![forbid(unsafe_code)]

mod voice;

use daw_party_plugin_api::{
    IoLayout, MidiEvent, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor,
    PluginError, PluginKind, ProcessContext, TimedMidiEvent,
};
use voice::Voice;

const MAX_VOICES: usize = 16;

/// 一次 `process` 内不变的参数快照。
///
/// 在块开始时组装一次，回调内只读传递——音频线程不做任何分配、查表或字符串比较。
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub wave: f32,
    pub sub: f32,
    pub pwm: f32,
    pub glide: f32,
    pub cutoff: f32,
    pub resonance: f32,
    pub filter_env: f32,
    pub key_follow: f32,
    pub hpf: f32,
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    pub lfo_rate: f32,
    pub lfo_delay: f32,
    pub lfo_pitch: f32,
    pub lfo_filter: f32,
    pub gain: f32,
    /// 白噪声混合比（0 = 纯振荡器，1 = 纯噪声）。
    pub noise: f32,
    /// 位深 + 采样率压碎（8-bit / 音效）。
    pub crush: f32,
    /// 失谐伴振的量（弦乐 / 铺底 / 宽主音）。
    pub detune: f32,
    /// FM 相位调制深度。
    pub fm: f32,
    /// FM 调制器频率比。
    pub fm_ratio: f32,
    /// 起音音高扫的幅度（底鼓 / 音效 / 拨弦）。
    pub pitch_env: f32,
    /// 音高扫的时长（0..1 → 5 ms .. 2 s）。
    pub pitch_decay: f32,
    /// 滤波器输出类型（0 = 低通，1 = 带通，2 = 高通）。
    pub filter_type: f32,
}

/// 参数表：id、显示名、范围、默认值。
///
/// 顺序即 `save_state` 的字节顺序；新增参数一律**追加在末尾**，于是「状态比当前参数表短」时
/// 缺的那几个保持默认（见 `load_state`）。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 26] = [
    ("gain", "Level", 0.0, 1.0, 0.8),
    ("attack", "Attack", 0.0, 1.0, 0.01),
    ("release", "Release", 0.0, 4.0, 0.3),
    ("wave", "Wave", 0.0, 3.0, 2.0),
    ("sub", "Sub", 0.0, 1.0, 0.0),
    ("pwm", "PWM", 0.0, 1.0, 0.5),
    ("glide", "Glide", 0.0, 0.5, 0.0),
    ("cutoff", "Cutoff", 0.0, 1.0, 0.7),
    ("resonance", "Resonance", 0.0, 1.0, 0.15),
    ("filterEnv", "Filter Env", 0.0, 1.0, 0.35),
    ("keyFollow", "Key Follow", 0.0, 1.0, 0.5),
    ("hpf", "HPF", 0.0, 1.0, 0.0),
    ("decay", "Decay", 0.0, 2.0, 0.4),
    ("sustain", "Sustain", 0.0, 1.0, 0.75),
    ("lfoRate", "LFO Rate", 0.0, 1.0, 0.75),
    ("lfoDelay", "LFO Delay", 0.0, 1.0, 0.0),
    ("lfoPitch", "LFO Pitch", 0.0, 1.0, 0.0),
    ("lfoFilter", "LFO Filter", 0.0, 1.0, 0.0),
    // ---- 扩展音源能力（追加：老状态短一截时这些保持默认） ----
    ("noise", "Noise", 0.0, 1.0, 0.0),
    ("crush", "Crush", 0.0, 1.0, 0.0),
    ("detune", "Detune", 0.0, 1.0, 0.0),
    ("fm", "FM", 0.0, 1.0, 0.0),
    ("fmRatio", "FM Ratio", 0.5, 8.0, 2.0),
    ("pitchEnv", "Pitch Env", 0.0, 1.0, 0.0),
    ("pitchDecay", "Pitch Decay", 0.0, 1.0, 0.1),
    ("filterType", "Filter Type", 0.0, 2.0, 0.0),
];

/// LFO 速率映射：0..1 → 0.05..20 Hz（指数，慢速区好调）。
fn lfo_hz(rate: f32) -> f64 {
    0.05 * 400f64.powf(rate as f64)
}

/// 内置合成器：DCO（主振 + 副振）→ HPF → VCF → VCA 的复音减法合成器。
///
/// 「预设」不是另一套 DSP，而是同一套信号链配上不同的默认参数——注册表按预设 id 各注册一个
/// 工厂（见 `daw_party_engine::registry`），工程里保存的 `params` 再覆盖默认值。
pub struct SynthPlugin {
    voices: [Voice; MAX_VOICES],
    sample_rate: f64,
    params: Params,
    /// 全局 LFO（老式模拟合成器只有一个）。
    lfo_phase: f64,
    /// 距最近一次按键的采样数：LFO 的淡入延迟用它。
    lfo_age: f64,
    /// 该实例对外声明的插件 id（预设实例报自己的 id，而不是 `builtin.synth`）。
    id: &'static str,
    name: &'static str,
}

impl Default for SynthPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SynthPlugin {
    /// 构造合成器，使用全部默认参数。
    pub fn new() -> Self {
        Self::preset("builtin.synth", "Builtin Synth", &[])
    }

    /// 用给定的默认参数构造合成器（沿用默认 id）。
    pub fn with_params(gain: f32, attack: f32, release: f32) -> Self {
        Self::preset(
            "builtin.synth",
            "Builtin Synth",
            &[("gain", gain), ("attack", attack), ("release", release)],
        )
    }

    /// 用给定的 id / 名称 / 参数补丁构造一个预设实例。
    ///
    /// 补丁里没提到的参数取 [`default_params`] 的运行默认值。
    pub fn preset(id: &'static str, name: &'static str, patch: &[(&str, f32)]) -> Self {
        let mut synth = SynthPlugin {
            voices: [Voice::idle(); MAX_VOICES],
            sample_rate: 48_000.0,
            params: default_params(),
            lfo_phase: 0.0,
            lfo_age: 0.0,
            id,
            name,
        };
        for (id, value) in patch {
            synth.set_param(&ParamId::from(*id), *value);
        }
        synth
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
                self.voices[slot].note_on(note, velocity, self.params.glide);
                // LFO 的延迟从「最近一次按键」重新计时。
                self.lfo_age = 0.0;
            }
            MidiEvent::NoteOff { note } => {
                for v in self.voices.iter_mut().filter(|v| v.note == note) {
                    v.note_off();
                }
            }
            // 单音音量独立于力度：只改该声部的增益，不碰音色。
            MidiEvent::NoteVolume { note, value } => {
                for v in self.voices.iter_mut().filter(|v| v.note == note) {
                    v.set_volume(value);
                }
            }
            MidiEvent::ControlChange { .. } => {}
        }
    }

    /// 当前参数值（按表的顺序）。
    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [
            self.params.gain,
            self.params.attack,
            self.params.release,
            self.params.wave,
            self.params.sub,
            self.params.pwm,
            self.params.glide,
            self.params.cutoff,
            self.params.resonance,
            self.params.filter_env,
            self.params.key_follow,
            self.params.hpf,
            self.params.decay,
            self.params.sustain,
            self.params.lfo_rate,
            self.params.lfo_delay,
            self.params.lfo_pitch,
            self.params.lfo_filter,
            self.params.noise,
            self.params.crush,
            self.params.detune,
            self.params.fm,
            self.params.fm_ratio,
            self.params.pitch_env,
            self.params.pitch_decay,
            self.params.filter_type,
        ]
    }
}

fn default_params() -> Params {
    Params {
        wave: 2.0,
        sub: 0.0,
        pwm: 0.5,
        glide: 0.0,
        cutoff: 0.7,
        resonance: 0.15,
        filter_env: 0.35,
        key_follow: 0.5,
        hpf: 0.0,
        attack: 0.01,
        decay: 0.4,
        sustain: 0.75,
        release: 0.3,
        lfo_rate: 0.75,
        lfo_delay: 0.0,
        lfo_pitch: 0.0,
        lfo_filter: 0.0,
        gain: 0.8,
        noise: 0.0,
        crush: 0.0,
        detune: 0.0,
        fm: 0.0,
        fm_ratio: 2.0,
        pitch_env: 0.0,
        pitch_decay: 0.1,
        filter_type: 0.0,
    }
}

impl Plugin for SynthPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.id.into(),
            name: self.name.into(),
            kind: PluginKind::Instrument,
            io: IoLayout { inputs: 0, outputs: 2 },
            // 内置合成器能单独处理单音音量（每个声部一个增益）；它的「界面」是前端那套
            // 合成器面板，不是原生窗口，因此 `has_editor` 为 false。
            capabilities: PluginCapabilities { per_note_volume: true, has_editor: false },
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        PARAM_TABLE
            .iter()
            .map(|(id, name, min, max, default)| ParamInfo {
                id: (*id).into(),
                name: (*name).into(),
                min: *min,
                max: *max,
                default: *default,
                automatable: true,
            })
            .collect()
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        let index = PARAM_TABLE.iter().position(|(k, ..)| *k == id.0.as_str())?;
        Some(self.values()[index])
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        let Some(index) = PARAM_TABLE.iter().position(|(k, ..)| *k == id.0.as_str()) else {
            return;
        };
        let (_, _, min, max, _) = PARAM_TABLE[index];
        let value = if value.is_finite() { value.clamp(min, max) } else { return };
        let p = &mut self.params;
        match index {
            0 => p.gain = value,
            1 => p.attack = value,
            2 => p.release = value,
            3 => p.wave = value,
            4 => p.sub = value,
            5 => p.pwm = value,
            6 => p.glide = value,
            7 => p.cutoff = value,
            8 => p.resonance = value,
            9 => p.filter_env = value,
            10 => p.key_follow = value,
            11 => p.hpf = value,
            12 => p.decay = value,
            13 => p.sustain = value,
            14 => p.lfo_rate = value,
            15 => p.lfo_delay = value,
            16 => p.lfo_pitch = value,
            17 => p.lfo_filter = value,
            18 => p.noise = value,
            19 => p.crush = value,
            20 => p.detune = value,
            21 => p.fm = value,
            22 => p.fm_ratio = value,
            23 => p.pitch_env = value,
            24 => p.pitch_decay = value,
            25 => p.filter_type = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
    }

    fn reset(&mut self) {
        self.voices = [Voice::idle(); MAX_VOICES];
        self.lfo_phase = 0.0;
        self.lfo_age = 0.0;
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
        let frames = audio.first().map_or(0, |c| c.len());
        let sr = self.sample_rate;
        let params = self.params;
        // 与采样无关的派生常量（滤波器阻尼、脉宽、FM/压碎/音高包络的系数）算一次就够：
        // 扩展后每采样要跑两个振荡器 + 一个调制器，能提出循环的量都要提出来。
        let block = voice::Block::new(&params, sr);

        for frame in 0..frames {
            for ev in events.iter().filter(|e| e.frame as usize == frame) {
                self.handle(ev.event);
            }

            // ---- 全局 LFO：三角波，按 delay 淡入 ----
            self.lfo_phase = (self.lfo_phase + lfo_hz(params.lfo_rate) / sr).fract();
            let tri = 4.0 * (self.lfo_phase - 0.5).abs() - 1.0;
            let delay_samples = (params.lfo_delay as f64) * 2.0 * sr;
            let fade = if delay_samples <= 1.0 {
                1.0
            } else {
                (self.lfo_age / delay_samples).min(1.0)
            };
            self.lfo_age += 1.0;
            let lfo = (tri * fade) as f32;

            let mut mix = 0.0f32;
            for v in self.voices.iter_mut() {
                mix += v.next_sample(sr, &params, &block, lfo);
            }
            mix = (mix * params.gain).clamp(-1.0, 1.0);
            for ch in audio.iter_mut() {
                ch[frame] = mix;
            }
        }
    }

    /// 状态 = 参数表顺序的 f32 小端序列。
    ///
    /// 前三个是 gain / attack / release，所以**老工程的 12 字节状态仍能读**（见 `load_state`）。
    fn save_state(&self) -> Vec<u8> {
        let values = self.values();
        let mut out = Vec::with_capacity(values.len() * 4);
        for v in values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.len() < 12 {
            return Err(PluginError::StateTooShort { expected: 12, got: state.len() });
        }
        // 允许多余/缺失：按可用的个数恢复，多出来的忽略，少掉的保持默认。
        for (index, (id, ..)) in PARAM_TABLE.iter().enumerate() {
            let start = index * 4;
            if start + 4 > state.len() {
                break;
            }
            let value = f32::from_le_bytes(state[start..start + 4].try_into().unwrap());
            self.set_param(&ParamId::from(*id), value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daw_party_plugin_api::{MidiEvent, Plugin, ProcessContext, TimedMidiEvent};

    fn ctx(block: usize) -> ProcessContext {
        ProcessContext { sample_rate: 48_000.0, block_size: block, tempo: 120.0, playing: true }
    }

    /// 跑若干块并返回峰值；`events` **只在第一块**送入（之后按键状态由包络自己维持）。
    fn peak_of(params: &[(&str, f32)], events: &[TimedMidiEvent], blocks: usize) -> f32 {
        let (peak, ..) = run(params, events, blocks);
        peak
    }

    /// 跑若干块，返回 (整段峰值, 均方根, 最后一块的峰值)。
    fn run(params: &[(&str, f32)], events: &[TimedMidiEvent], blocks: usize) -> (f32, f32, f32) {
        let mut s = SynthPlugin::new();
        for (id, value) in params {
            s.set_param(&ParamId::from(*id), *value);
        }
        s.prepare(48_000.0, 256);
        let mut peak = 0.0f32;
        let mut tail = 0.0f32;
        let mut square_sum = 0.0f64;
        let mut samples = 0u64;
        for block in 0..blocks {
            let mut l = [0.0f32; 256];
            let mut r = [0.0f32; 256];
            let evs = if block == 0 { events } else { &[] };
            s.process(&ctx(256), &mut [&mut l, &mut r], evs);
            tail = l.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            peak = peak.max(tail);
            square_sum += l.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>();
            samples += l.len() as u64;
        }
        let rms = (square_sum / samples.max(1) as f64).sqrt() as f32;
        (peak, rms, tail)
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
    fn note_volume_attenuates_independently_of_velocity() {
        let note_on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 100 } }];
        let loud = peak_of(&[], &note_on, 1);
        let soft = peak_of(
            &[],
            &[
                TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 100 } },
                TimedMidiEvent { frame: 0, event: MidiEvent::NoteVolume { note: 69, value: 64 } },
            ],
            1,
        );
        assert!(soft < loud * 0.7, "volume must attenuate: {loud} -> {soft}");
    }

    /// 滤波器的截止真的在起作用：关小截止，输出必须明显变轻（高频被削掉）。
    #[test]
    fn cutoff_changes_the_signal() {
        let note_on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        let open = peak_of(&[("cutoff", 1.0), ("filterEnv", 0.0)], &note_on, 4);
        let closed = peak_of(&[("cutoff", 0.05), ("filterEnv", 0.0)], &note_on, 4);
        assert!(closed < open * 0.5, "closing the filter must soften: {open} -> {closed}");
    }

    /// 共振有意义：拉满共振，输出不会爆掉也不会静音。
    #[test]
    fn resonance_stays_stable() {
        let note_on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 45, velocity: 127 } }];
        let peak = peak_of(&[("cutoff", 0.35), ("resonance", 1.0), ("filterEnv", 0.8)], &note_on, 40);
        assert!(peak > 0.01 && peak <= 1.0, "peak out of range: {peak}");
    }

    /// 每种波形都出声，且强弱关系符合各自的形状（峰值会被削波，所以比均方根）。
    #[test]
    fn waveforms_differ() {
        let note_on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 57, velocity: 127 } }];
        let common: &[(&str, f32)] = &[("gain", 0.3), ("filterEnv", 0.0), ("cutoff", 1.0)];
        let rms = |wave: f32| {
            let mut params = common.to_vec();
            params.push(("wave", wave));
            run(&params, &note_on, 4).1
        };
        let (saw, square, sine, triangle) = (rms(0.0), rms(1.0), rms(2.0), rms(3.0));
        for (name, value) in [("saw", saw), ("square", square), ("sine", sine), ("triangle", triangle)] {
            assert!(value > 0.01, "{name} must sound: {value}");
        }
        // 方波(RMS=1) > 正弦(0.707) > 锯齿/三角(≈0.577)。
        assert!(square > sine * 1.15, "square vs sine: {square} / {sine}");
        assert!(sine > triangle * 1.1, "sine vs triangle: {sine} / {triangle}");
    }

    /// 释音为 0 时按键一放就静音；延音为 0 时衰减完就静音。
    #[test]
    fn zero_sustain_ends_the_note_during_decay() {
        let note_on = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        let params = [("sustain", 0.0), ("decay", 0.05)];
        let (peak, _, _) = run(&params, &note_on, 1);
        assert!(peak > 0.01, "attack must sound");
        // 60 块 = 0.32 s，远长于 0.05 s 的衰减 → 尾音必须是静音。
        let (_, _, tail) = run(&params, &note_on, 60);
        assert!(tail < 0.001, "sustain=0 must decay to silence: {tail}");
    }

    #[test]
    fn renders_440hz_within_tolerance() {
        let mut s = SynthPlugin::new();
        // 正弦 + 全开截止，过零率才等于基频（锯齿的谐波会让过零变多）。
        s.set_param(&ParamId::from("wave"), 2.0);
        s.set_param(&ParamId::from("cutoff"), 1.0);
        s.set_param(&ParamId::from("filterEnv"), 0.0);
        s.set_param(&ParamId::from("decay"), 2.0);
        s.set_param(&ParamId::from("sustain"), 1.0);
        s.prepare(48_000.0, 48_000);
        let mut l = [0.0f32; 48_000];
        let mut r = [0.0f32; 48_000];
        s.process(
            &ctx(48_000),
            &mut [&mut l, &mut r],
            &[TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }],
        );
        let crossings = l.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((crossings as i64 - 440).abs() <= 4, "crossings={crossings}");
    }

    #[test]
    fn state_roundtrips_every_param() {
        let mut s = SynthPlugin::new();
        s.set_param(&ParamId::from("gain"), 0.25);
        s.set_param(&ParamId::from("cutoff"), 0.33);
        s.set_param(&ParamId::from("wave"), 3.0);
        s.set_param(&ParamId::from("lfoPitch"), 0.5);
        let state = s.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);

        let mut t = SynthPlugin::new();
        t.load_state(&state).unwrap();
        for (id, ..) in PARAM_TABLE.iter() {
            assert_eq!(t.get_param(&ParamId::from(*id)), s.get_param(&ParamId::from(*id)), "{id}");
        }
    }

    /// 老工程存的是 12 字节（gain / attack / release），必须仍然读得懂。
    #[test]
    fn legacy_twelve_byte_state_still_loads() {
        let mut state = Vec::new();
        state.extend_from_slice(&0.5f32.to_le_bytes());
        state.extend_from_slice(&0.2f32.to_le_bytes());
        state.extend_from_slice(&0.9f32.to_le_bytes());

        let mut t = SynthPlugin::new();
        t.load_state(&state).unwrap();
        assert_eq!(t.get_param(&ParamId::from("gain")), Some(0.5));
        assert_eq!(t.get_param(&ParamId::from("attack")), Some(0.2));
        assert_eq!(t.get_param(&ParamId::from("release")), Some(0.9));
        // 其余参数保持默认。
        assert_eq!(t.get_param(&ParamId::from("cutoff")), Some(0.7));
    }

    #[test]
    fn preset_reports_its_own_identity_and_defaults() {
        let s = SynthPlugin::preset("builtin.synth.pad", "Pad", &[("gain", 0.5), ("attack", 0.4), ("release", 1.5)]);
        assert_eq!(s.descriptor().id, "builtin.synth.pad");
        assert_eq!(s.descriptor().name, "Pad");
        assert_eq!(s.get_param(&ParamId::from("gain")), Some(0.5));
        assert_eq!(s.get_param(&ParamId::from("attack")), Some(0.4));
        assert_eq!(s.get_param(&ParamId::from("release")), Some(1.5));
    }

    /// 参数越界被夹紧，非有限值被丢弃。
    #[test]
    fn params_are_clamped_and_reject_nan() {
        let mut s = SynthPlugin::new();
        s.set_param(&ParamId::from("cutoff"), 5.0);
        assert_eq!(s.get_param(&ParamId::from("cutoff")), Some(1.0));
        s.set_param(&ParamId::from("cutoff"), f32::NAN);
        assert_eq!(s.get_param(&ParamId::from("cutoff")), Some(1.0));
        // 未知参数名不得 panic。
        s.set_param(&ParamId::from("nope"), 1.0);
    }

    /// 默认音色必须是正弦（新手一上手就是一个干净的基础音）。
    #[test]
    fn default_wave_is_sine() {
        let s = SynthPlugin::new();
        assert_eq!(s.get_param(&ParamId::from("wave")), Some(2.0));
        // 参数表里 `wave` 的**元数据**默认值也必须是正弦（2.0），与运行值一致。
        let info = s
            .params()
            .into_iter()
            .find(|p| p.id == ParamId::from("wave"))
            .expect("参数表里应有 wave");
        assert_eq!(info.default, 2.0);
    }

    // ---- 扩展音源能力：逐参数量化断言 ----

    /// 渲染成一整段单声道样本（频谱/过零率这类分析用）。
    fn render(params: &[(&str, f32)], events: &[TimedMidiEvent], blocks: usize) -> Vec<f32> {
        let mut s = SynthPlugin::new();
        for (id, value) in params {
            s.set_param(&ParamId::from(*id), *value);
        }
        s.prepare(48_000.0, 256);
        let mut out = Vec::with_capacity(blocks * 256);
        for block in 0..blocks {
            let mut l = [0.0f32; 256];
            let mut r = [0.0f32; 256];
            let evs = if block == 0 { events } else { &[] };
            s.process(&ctx(256), &mut [&mut l, &mut r], evs);
            out.extend_from_slice(&l);
        }
        out
    }

    /// 上升过零次数：基频的粗略代理（纯净单音下 ≈ 基频 × 秒数）。
    fn crossings(data: &[f32]) -> usize {
        data.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count()
    }

    /// 排序去重后相邻电平的**最小正间隔**：量化步长的直接度量（越小 = 越连续）。
    fn min_level_gap(data: &[f32]) -> f64 {
        let mut v: Vec<f64> = data.iter().map(|x| *x as f64).collect();
        v.sort_by(|a, b| a.partial_cmp(b).expect("样本必须有限"));
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v.windows(2).map(|w| w[1] - w[0]).fold(f64::MAX, f64::min)
    }

    /// 稳定平顶的渲染配方：起音 0、延音 1、衰减拉长 → 包络恒为 1，VCA 只是个常数。
    fn flat() -> Vec<(&'static str, f32)> {
        vec![
            ("gain", 1.0),
            ("attack", 0.0),
            ("decay", 2.0),
            ("sustain", 1.0),
            ("cutoff", 1.0),
            ("filterEnv", 0.0),
            ("keyFollow", 0.0),
            ("wave", 2.0),
        ]
    }

    fn with(base: &[(&'static str, f32)], extra: &[(&'static str, f32)]) -> Vec<(&'static str, f32)> {
        let mut out = base.to_vec();
        out.extend_from_slice(extra);
        out
    }

    /// 噪声是**混合**：`noise = 1` 得到纯噪声（宽带），`noise = 0` 是干净的正弦。
    #[test]
    fn noise_replaces_the_oscillator_with_broadband() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        let clean = render(&with(&flat(), &[("noise", 0.0)]), &note, 20);
        let noisy = render(&with(&flat(), &[("noise", 1.0)]), &note, 20);
        let (c, n) = (crossings(&clean), crossings(&noisy));
        // 440 Hz、20 块 ≈ 0.107 s → 干净正弦约 47 次过零。
        assert!(c < 60, "干净正弦不该有过多次过零：{c}");
        assert!(n > c * 8, "纯噪声应当异常频繁地过零：{c} -> {n}");
        // 带宽上去之后，能量不该同时崩掉。
        let rms = |d: &[f32]| (d.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / d.len() as f64).sqrt();
        assert!(rms(&noisy) > rms(&clean) * 0.5, "噪声不该比正弦轻太多");
    }

    /// 位深压碎真的在量化：电平的最小间隔变成 1/8（3 bit），而不是连续值。
    #[test]
    fn crush_quantizes_the_output() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        let clean = render(&with(&flat(), &[("crush", 0.0)]), &note, 8);
        let crushed = render(&with(&flat(), &[("crush", 1.0)]), &note, 8);
        let (c, k) = (min_level_gap(&clean), min_level_gap(&crushed));
        assert!(c < 0.01, "未压碎的正弦电平应当基本连续：最小间隔 {c}");
        assert!(
            (k - 0.125).abs() < 1e-4,
            "拉满压碎后电平步长应当是 1/8（3 bit）：{k}"
        );
    }

    /// 压碎 = 0 时逐样本等于改版前的输出（新算子不许悄悄改变默认音色）。
    #[test]
    fn defaults_are_byte_for_byte_unchanged() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 60, velocity: 100 } }];
        // 只用改版前就存在的参数。
        let old = render(&[("gain", 0.8), ("wave", 2.0), ("cutoff", 0.7), ("lfoPitch", 0.3)], &note, 6);
        // 显式把新参数钉在默认值上，结果必须一致。
        let pinned = render(
            &[
                ("gain", 0.8),
                ("wave", 2.0),
                ("cutoff", 0.7),
                ("lfoPitch", 0.3),
                ("noise", 0.0),
                ("crush", 0.0),
                ("detune", 0.0),
                ("fm", 0.0),
                ("pitchEnv", 0.0),
            ],
            &note,
            6,
        );
        assert_eq!(old, pinned, "新参数在默认值下不得改变输出");
    }

    /// 失谐靠**拍频**验证：两个失谐的振荡器会此消彼长，窗口峰值随之起伏。
    #[test]
    fn detune_beats_against_the_main_oscillator() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        // 拉满失谐 = +2% ≈ 34 音分 → 440 Hz 上约 8.8 Hz 拍频（周期 114 ms）。
        // 窗口必须**明显短于一个拍周期**，否则每个窗口的峰值都能赶上一次相长干涉，测不出起伏。
        let spread = |detune: f32| -> f32 {
            let data = render(&with(&flat(), &[("detune", detune)]), &note, 40);
            let windows = 16;
            let win = data.len() / windows;
            let peaks: Vec<f32> = (0..windows)
                .map(|i| data[i * win..(i + 1) * win].iter().fold(0.0f32, |a, b| a.max(b.abs())))
                .collect();
            let (lo, hi) = (
                peaks.iter().cloned().fold(f32::MAX, f32::min),
                peaks.iter().cloned().fold(0.0f32, f32::max),
            );
            if hi <= 0.0 { 0.0 } else { lo / hi }
        };
        let dry = spread(0.0);
        let wet = spread(1.0);
        assert!(dry > 0.9, "不失谐时各窗口峰值应当基本一致：{dry}");
        assert!(wet < 0.5, "失谐应当产生明显的拍频起伏：{wet}");
    }

    /// FM 产生边带：同样的音高下，过零次数显著增多。
    #[test]
    fn fm_adds_sidebands() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 69, velocity: 127 } }];
        let clean = render(&with(&flat(), &[("fm", 0.0)]), &note, 20);
        let fm = render(&with(&flat(), &[("fm", 0.9), ("fmRatio", 3.0)]), &note, 20);
        let (c, m) = (crossings(&clean), crossings(&fm));
        assert!(m > c * 2, "FM 边带应当明显增加过零：{c} -> {m}");
    }

    /// FM 深度拉满也不许削波爆掉。
    #[test]
    fn extreme_fm_stays_bounded() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 45, velocity: 127 } }];
        let data = render(
            &with(&flat(), &[("fm", 1.0), ("fmRatio", 8.0), ("resonance", 1.0), ("noise", 0.3)]),
            &note,
            10,
        );
        assert!(data.iter().all(|x| x.is_finite()), "不得出现 NaN / Inf");
        let peak = data.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak <= 1.0, "输出必须被限幅：{peak}");
    }

    /// 音高包络是**下扫**：前段过零率明显高于后段。
    #[test]
    fn pitch_envelope_sweeps_down() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 57, velocity: 127 } }];
        // pitchDecay = 0.5 → 0.005 × 400^0.5 = 0.1 s 扫完（+2 八度 → 0）。
        let data = render(&with(&flat(), &[("pitchEnv", 1.0), ("pitchDecay", 0.5)]), &note, 40);
        let head = crossings(&data[0..4800]);
        let tail = crossings(&data[5400..10200]);
        assert!(head > tail * 2, "起音应当是下扫：前段 {head} vs 后段 {tail}");

        // 关掉音高包络则前后一致。
        let flat_data = render(&with(&flat(), &[("pitchEnv", 0.0)]), &note, 40);
        let (h2, t2) = (crossings(&flat_data[0..4800]), crossings(&flat_data[5400..10200]));
        assert!((h2 as i64 - t2 as i64).abs() <= 4, "关掉后前后应当一致：{h2} / {t2}");
    }

    /// 高通真的把低频削掉：同一颗低音用高通播会明显更轻。
    #[test]
    fn filter_type_high_pass_removes_the_low_end() {
        let note = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 40, velocity: 127 } }];
        let rms = |data: &[f32]| {
            (data.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / data.len() as f64).sqrt()
        };
        let base = with(&flat(), &[("cutoff", 0.5), ("wave", 0.0)]);
        let lp = rms(&render(&with(&base, &[("filterType", 0.0)]), &note, 20));
        let hp = rms(&render(&with(&base, &[("filterType", 2.0)]), &note, 20));
        assert!(lp > 0.05, "低通下低音应当有能量：{lp}");
        assert!(hp < lp * 0.5, "高通应当把低音压下去：{lp} -> {hp}");
    }

    /// 新增参数也要能往返。
    #[test]
    fn state_roundtrips_the_extended_params() {
        let mut s = SynthPlugin::new();
        for (id, value) in [
            ("noise", 0.42),
            ("crush", 0.13),
            ("detune", 0.77),
            ("fm", 0.31),
            ("fmRatio", 5.5),
            ("pitchEnv", 0.61),
            ("pitchDecay", 0.23),
            ("filterType", 2.0),
        ] {
            s.set_param(&ParamId::from(id), value);
        }
        let state = s.save_state();
        let mut t = SynthPlugin::new();
        t.load_state(&state).unwrap();
        for (id, ..) in PARAM_TABLE.iter() {
            assert_eq!(t.get_param(&ParamId::from(*id)), s.get_param(&ParamId::from(*id)), "{id}");
        }
    }

    /// 状态比当前参数表短：多出来的（新增）参数保持默认，而不是报错或读到垃圾。
    #[test]
    fn shorter_state_keeps_new_params_at_defaults() {
        let mut s = SynthPlugin::new();
        s.set_param(&ParamId::from("noise"), 0.9);
        let state = s.save_state();
        let short = &state[..state.len() - 8]; // 砍掉最后两个（pitchDecay / filterType）

        let mut t = SynthPlugin::new();
        t.load_state(short).unwrap();
        assert_eq!(t.get_param(&ParamId::from("pitchDecay")), Some(0.1), "缺失的参数保持默认");
        assert_eq!(t.get_param(&ParamId::from("filterType")), Some(0.0));
        assert_eq!(t.get_param(&ParamId::from("noise")), Some(0.9), "短的也照读");
    }
}
