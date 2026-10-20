#![forbid(unsafe_code)]

//! 采样内置乐器：钢琴 / 木吉他 / 电吉他 / 贝斯 / 原声鼓组。
//!
//! 资产是**编译期内嵌**的 SF2/SF3：不依赖资源目录、不依赖安装路径，构造时只解一次
//! `SoundFont`（同类乐器共享 `Arc`），每个轨道实例再建一个 `Synthesizer`。
//!
//! SF3 的 Ogg Vorbis 在 `prepare` 里解码成 PCM；`process` 里只剩读取和混音，
//! 不分配内存、不加锁。

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use ai_daw_plugin_api::{
    IoLayout, MidiEvent, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor,
    PluginError, PluginKind, ProcessContext, TimedMidiEvent,
};
use rustysynth_ext::{SoundFont, Synthesizer, SynthesizerSettings};

pub const PIANO_ID: &str = "builtin.piano";
pub const NYLON_GUITAR_ID: &str = "builtin.nylon_guitar";
pub const ELECTRIC_GUITAR_ID: &str = "builtin.electric_guitar";
pub const BASS_ID: &str = "builtin.bass";
pub const DRUMS_ID: &str = "builtin.drums";

/// 一件内置采样乐器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Piano,
    NylonGuitar,
    ElectricGuitar,
    Bass,
    Drums,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Piano,
        Kind::NylonGuitar,
        Kind::ElectricGuitar,
        Kind::Bass,
        Kind::Drums,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Kind::Piano => PIANO_ID,
            Kind::NylonGuitar => NYLON_GUITAR_ID,
            Kind::ElectricGuitar => ELECTRIC_GUITAR_ID,
            Kind::Bass => BASS_ID,
            Kind::Drums => DRUMS_ID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Piano => "Piano",
            Kind::NylonGuitar => "Nylon Guitar",
            Kind::ElectricGuitar => "Electric Guitar",
            Kind::Bass => "Bass",
            Kind::Drums => "Drum Kit",
        }
    }

    /// 默认增益：按试听时的峰值校准，让挂上轨道就有可用的电平。
    pub fn default_gain(self) -> f32 {
        match self {
            Kind::Piano => 2.5,
            Kind::NylonGuitar => 3.0,
            Kind::ElectricGuitar => 2.2,
            Kind::Bass => 3.0,
            Kind::Drums => 2.8,
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Kind::Piano => PIANO_BYTES,
            Kind::NylonGuitar => NYLON_BYTES,
            Kind::ElectricGuitar => ELECTRIC_BYTES,
            Kind::Bass => BASS_BYTES,
            Kind::Drums => DRUMS_BYTES,
        }
    }

    fn cache(self) -> &'static OnceLock<Option<Arc<SoundFont>>> {
        match self {
            Kind::Piano => &PIANO_FONT,
            Kind::NylonGuitar => &NYLON_FONT,
            Kind::ElectricGuitar => &ELECTRIC_FONT,
            Kind::Bass => &BASS_FONT,
            Kind::Drums => &DRUMS_FONT,
        }
    }
}

static PIANO_BYTES: &[u8] = include_bytes!("../assets/piano.sf3");
static NYLON_BYTES: &[u8] = include_bytes!("../assets/nylon_guitar.sf3");
static ELECTRIC_BYTES: &[u8] = include_bytes!("../assets/electric_guitar.sf3");
static BASS_BYTES: &[u8] = include_bytes!("../assets/bass.sf3");
static DRUMS_BYTES: &[u8] = include_bytes!("../assets/drums.sf2");

static PIANO_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static NYLON_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static ELECTRIC_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static BASS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static DRUMS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();

fn load_font(kind: Kind) -> Option<Arc<SoundFont>> {
    let mut cursor = Cursor::new(kind.bytes());
    match SoundFont::new(&mut cursor) {
        Ok(font) => Some(Arc::new(font)),
        Err(err) => {
            // 资产是编译期内嵌的；走到这里说明构建产物坏了。不要 panic：
            // 返回 None，轨道静音，但工程仍能打开。
            eprintln!("[instruments] {} 载入失败：{err}", kind.id());
            None
        }
    }
}

fn font(kind: Kind) -> Option<Arc<SoundFont>> {
    kind.cache().get_or_init(|| load_font(kind)).clone()
}

/// 一个采样乐器实例。
pub struct SoundFontPlugin {
    kind: Kind,
    synth: Option<Synthesizer>,
    sample_rate: f64,
    gain: f32,
    tone: f32,
    tone_l: f32,
    tone_r: f32,
}

impl SoundFontPlugin {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            synth: None,
            sample_rate: 48_000.0,
            gain: kind.default_gain(),
            tone: 1.0,
            tone_l: 0.0,
            tone_r: 0.0,
        }
    }

    /// 注册表工厂入口。
    pub fn create(kind: Kind) -> Box<dyn Plugin> {
        Box::new(Self::new(kind))
    }

    fn apply_event(&mut self, event: MidiEvent) {
        let Some(synth) = self.synth.as_mut() else {
            return;
        };
        match event {
            MidiEvent::NoteOn { note, velocity } => {
                if velocity == 0 {
                    synth.note_off(0, note as i32);
                } else {
                    synth.note_on(0, note as i32, velocity as i32);
                }
            }
            MidiEvent::NoteOff { note } => synth.note_off(0, note as i32),
            // 采样器没有逐声部增益接口；`per_note_volume` 报 false，忽略该事件。
            MidiEvent::NoteVolume { .. } | MidiEvent::ControlChange { .. } => {}
        }
    }

    fn render_range(&mut self, audio: &mut [&mut [f32]], start: usize, end: usize) {
        if end <= start {
            return;
        }
        let (left, right) = audio.split_at_mut(1);
        let left = &mut left[0][start..end];
        let right = &mut right[0][start..end];

        if let Some(synth) = self.synth.as_mut() {
            synth.render(left, right);
        } else {
            left.fill(0.0);
            right.fill(0.0);
            return;
        }

        let gain = self.gain;
        let alpha = tone_alpha(self.tone, self.sample_rate);
        if alpha >= 1.0 {
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                *l = (*l * gain).clamp(-1.0, 1.0);
                *r = (*r * gain).clamp(-1.0, 1.0);
            }
            return;
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let dry_l = *l * gain;
            let dry_r = *r * gain;
            self.tone_l += alpha * (dry_l - self.tone_l);
            self.tone_r += alpha * (dry_r - self.tone_r);
            *l = self.tone_l.clamp(-1.0, 1.0);
            *r = self.tone_r.clamp(-1.0, 1.0);
        }
    }
}

/// `tone` 是一阶低通：0 → 约 200 Hz，1 → 旁通。
fn tone_alpha(tone: f32, sample_rate: f64) -> f32 {
    if tone >= 0.999 {
        return 1.0;
    }
    let cutoff = 200.0 * 100.0f64.powf(tone.clamp(0.0, 1.0) as f64);
    let alpha = 1.0 - (-std::f64::consts::TAU * cutoff / sample_rate).exp();
    alpha.clamp(0.0, 1.0) as f32
}

impl Plugin for SoundFontPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.kind.id().into(),
            name: self.kind.name().into(),
            kind: PluginKind::Instrument,
            io: IoLayout { inputs: 0, outputs: 2 },
            capabilities: PluginCapabilities {
                per_note_volume: false,
                has_editor: false,
            },
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        vec![
            ParamInfo {
                id: "gain".into(),
                name: "音量".into(),
                min: 0.0,
                max: 4.0,
                default: self.kind.default_gain(),
                automatable: true,
            },
            ParamInfo {
                id: "tone".into(),
                name: "音色".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                automatable: true,
            },
        ]
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        match id.0.as_str() {
            "gain" => Some(self.gain),
            "tone" => Some(self.tone),
            _ => None,
        }
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        if !value.is_finite() {
            return;
        }
        match id.0.as_str() {
            "gain" => self.gain = value.clamp(0.0, 4.0),
            "tone" => self.tone = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, block_size: usize) {
        self.sample_rate = sample_rate.max(1.0);
        self.tone_l = 0.0;
        self.tone_r = 0.0;
        let Some(font) = font(self.kind) else {
            self.synth = None;
            return;
        };
        let mut settings = SynthesizerSettings::new(self.sample_rate.round() as i32);
        settings.block_size = block_size.clamp(8, 1024);
        // 空间感交给轨道效果链；采样器自己的 reverb/chorus 会重复染色。
        settings.enable_reverb_and_chorus = false;
        self.synth = Synthesizer::new(&font, &settings).ok();
    }

    fn reset(&mut self) {
        if let Some(synth) = self.synth.as_mut() {
            synth.reset();
        }
        self.tone_l = 0.0;
        self.tone_r = 0.0;
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        audio: &mut [&mut [f32]],
        events: &[TimedMidiEvent],
    ) {
        if audio.len() < 2 {
            for channel in audio.iter_mut() {
                channel.fill(0.0);
            }
            return;
        }
        let frames = audio[0].len().min(audio[1].len());
        if frames == 0 {
            return;
        }

        // 试听事件会被引擎追加到块尾、frame = 0；先统一应用 frame 0 的事件，
        // 再把其余（引擎按 frame 排好序的）事件按段渲染。
        for ev in events.iter().filter(|e| e.frame == 0) {
            self.apply_event(ev.event);
        }

        let mut cursor = 0usize;
        for ev in events.iter().filter(|e| e.frame > 0) {
            let frame = (ev.frame as usize).min(frames);
            if frame > cursor {
                self.render_range(audio, cursor, frame);
                cursor = frame;
            }
            self.apply_event(ev.event);
        }
        if cursor < frames {
            self.render_range(audio, cursor, frames);
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&self.gain.to_le_bytes());
        out.extend_from_slice(&self.tone.to_le_bytes());
        out
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        // 只认自己的 8 字节（gain + tone）。宿主换乐器时，上一件乐器的状态可能被
        // 写进新实例；如果长度不对（例如合成器的 104 字节），直接拒绝，保持默认，
        // 免得把别的插件的二进制误读成增益/音色。
        if state.len() != 8 {
            return Err(PluginError::InvalidState(format!(
                "采样乐器的状态应为 8 字节（gain + tone），实际 {} 字节（可能来自另一件乐器）",
                state.len()
            )));
        }
        let gain = f32::from_le_bytes(state[..4].try_into().unwrap());
        let tone = f32::from_le_bytes(state[4..8].try_into().unwrap());
        if !gain.is_finite() || !(0.0..=4.0).contains(&gain) {
            return Err(PluginError::InvalidState(format!("gain={gain} 不在 [0, 4] 内")));
        }
        if !tone.is_finite() || !(0.0..=1.0).contains(&tone) {
            return Err(PluginError::InvalidState(format!("tone={tone} 不在 [0, 1] 内")));
        }
        self.gain = gain;
        self.tone = tone;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(kind: Kind, note: u8, velocity: u8) -> Vec<f32> {
        let mut plugin = SoundFontPlugin::new(kind);
        plugin.prepare(48_000.0, 256);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 256,
            tempo: 120.0,
            playing: true,
        };
        let mut left = vec![0.0f32; 2048];
        let mut right = vec![0.0f32; 2048];
        let events = [TimedMidiEvent {
            frame: 0,
            event: MidiEvent::NoteOn { note, velocity },
        }];
        plugin.process(&ctx, &mut [&mut left, &mut right], &events);
        left
    }

    #[test]
    fn every_builtin_asset_loads_and_makes_sound() {
        for (kind, note) in [
            (Kind::Piano, 60u8),
            (Kind::NylonGuitar, 60),
            (Kind::ElectricGuitar, 60),
            (Kind::Bass, 40),
            (Kind::Drums, 36),
        ] {
            let out = render(kind, note, 100);
            let peak = out.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak > 1e-4, "{} 应当发声（peak={peak}）", kind.id());
        }
    }

    #[test]
    fn drum_keys_are_distinct() {
        let kick = render(Kind::Drums, 36, 110);
        let hat = render(Kind::Drums, 42, 110);
        let peak = |v: &[f32]| v.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak(&kick) > 1e-3);
        assert!(peak(&hat) > 1e-3);
        let diff = kick.iter().zip(hat.iter()).map(|(a, b)| (a - b).abs()).sum::<f32>();
        assert!(diff > 1e-2, "36 与 42 应该是不同的鼓件");
    }

    #[test]
    fn descriptor_and_params_are_stable() {
        let p = SoundFontPlugin::new(Kind::Piano);
        let d = p.descriptor();
        assert_eq!(d.id, PIANO_ID);
        assert_eq!(d.kind, PluginKind::Instrument);
        assert_eq!(d.io.outputs, 2);
        assert_eq!(p.get_param(&"gain".into()), Some(Kind::Piano.default_gain()));
        assert_eq!(p.get_param(&"tone".into()), Some(1.0));
        assert_eq!(p.params().len(), 2);
    }

    #[test]
    fn state_round_trips() {
        let mut a = SoundFontPlugin::new(Kind::NylonGuitar);
        a.set_param(&"gain".into(), 2.25);
        a.set_param(&"tone".into(), 0.4);
        let state = a.save_state();
        let mut b = SoundFontPlugin::new(Kind::NylonGuitar);
        b.load_state(&state).unwrap();
        assert_eq!(b.get_param(&"gain".into()), Some(2.25));
        assert_eq!(b.get_param(&"tone".into()), Some(0.4));
        assert!(b.load_state(&[0, 1]).is_err());
        // 合成器的状态（104 字节）不能被误读成 gain/tone。
        assert!(b.load_state(&[0u8; 104]).is_err());
    }

    #[test]
    fn missing_font_is_silent_not_a_panic() {
        // 正常资产一定加载成功；这里只验证「没有 synth 时 process 会清零」。
        let mut p = SoundFontPlugin::new(Kind::Piano);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 64,
            tempo: 120.0,
            playing: true,
        };
        let mut left = [1.0f32; 64];
        let mut right = [1.0f32; 64];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert!(left.iter().all(|s| *s == 0.0));
        assert!(right.iter().all(|s| *s == 0.0));
    }
}
