#![forbid(unsafe_code)]

//! 采样内置乐器：钢琴 / 吉他 / 贝斯 / 鼓组 / 弦乐 / 铜管。
//!
//! 资产是**编译期内嵌**的 SF2/SF3：不依赖资源目录、不依赖安装路径，构造时只解一次
//! `SoundFont`（同类乐器共享 `Arc`），每个轨道实例再建一个 `Synthesizer`。
//!
//! SF3 的 Ogg Vorbis 在 `prepare` 里解码成 PCM；`process` 里只剩读取和混音，
//! 不分配内存、不加锁。

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use ai_daw_plugin_api::{
    pitch_bend_midi_bytes, IoLayout, MidiEvent, ParamId, ParamInfo, Plugin, PluginCapabilities,
    PluginDescriptor, PluginError, PluginKind, ProcessContext, TimedMidiEvent,
};
use rustysynth_ext::{SoundFont, Synthesizer, SynthesizerSettings};

pub const PIANO_ID: &str = "builtin.piano";
pub const NYLON_GUITAR_ID: &str = "builtin.nylon_guitar";
pub const ELECTRIC_GUITAR_ID: &str = "builtin.electric_guitar";
pub const BASS_ID: &str = "builtin.bass";
pub const DRUMS_ID: &str = "builtin.drums";
pub const STRING_ENSEMBLE_ID: &str = "builtin.strings";
pub const SLOW_STRINGS_ID: &str = "builtin.slow_strings";
pub const VIOLIN_ID: &str = "builtin.violin";
pub const VIOLA_ID: &str = "builtin.viola";
pub const CELLO_ID: &str = "builtin.cello";
pub const CONTRABASS_ID: &str = "builtin.contrabass";
pub const PIZZICATO_ID: &str = "builtin.pizzicato";
pub const TRUMPET_ID: &str = "builtin.trumpet";
pub const FRENCH_HORNS_ID: &str = "builtin.french_horns";
pub const SYNTH_BRASS_1_ID: &str = "builtin.synth_brass_1";
pub const SYNTH_BRASS_2_ID: &str = "builtin.synth_brass_2";
pub const SYNTH_STRINGS_1_ID: &str = "builtin.synth_strings_1";
pub const SYNTH_STRINGS_2_ID: &str = "builtin.synth_strings_2";

/// 一件内置采样乐器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Piano,
    NylonGuitar,
    ElectricGuitar,
    Bass,
    Drums,
    StringEnsemble,
    SlowStrings,
    Violin,
    Viola,
    Cello,
    Contrabass,
    Pizzicato,
    Trumpet,
    FrenchHorns,
    SynthBrass1,
    SynthBrass2,
    SynthStrings1,
    SynthStrings2,
}

impl Kind {
    pub const ALL: [Kind; 18] = [
        Kind::Piano,
        Kind::NylonGuitar,
        Kind::ElectricGuitar,
        Kind::Bass,
        Kind::Drums,
        Kind::StringEnsemble,
        Kind::SlowStrings,
        Kind::Violin,
        Kind::Viola,
        Kind::Cello,
        Kind::Contrabass,
        Kind::Pizzicato,
        Kind::Trumpet,
        Kind::FrenchHorns,
        Kind::SynthBrass1,
        Kind::SynthBrass2,
        Kind::SynthStrings1,
        Kind::SynthStrings2,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Kind::Piano => PIANO_ID,
            Kind::NylonGuitar => NYLON_GUITAR_ID,
            Kind::ElectricGuitar => ELECTRIC_GUITAR_ID,
            Kind::Bass => BASS_ID,
            Kind::Drums => DRUMS_ID,
            Kind::StringEnsemble => STRING_ENSEMBLE_ID,
            Kind::SlowStrings => SLOW_STRINGS_ID,
            Kind::Violin => VIOLIN_ID,
            Kind::Viola => VIOLA_ID,
            Kind::Cello => CELLO_ID,
            Kind::Contrabass => CONTRABASS_ID,
            Kind::Pizzicato => PIZZICATO_ID,
            Kind::Trumpet => TRUMPET_ID,
            Kind::FrenchHorns => FRENCH_HORNS_ID,
            Kind::SynthBrass1 => SYNTH_BRASS_1_ID,
            Kind::SynthBrass2 => SYNTH_BRASS_2_ID,
            Kind::SynthStrings1 => SYNTH_STRINGS_1_ID,
            Kind::SynthStrings2 => SYNTH_STRINGS_2_ID,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Piano => "Piano",
            Kind::NylonGuitar => "Nylon Guitar",
            Kind::ElectricGuitar => "Electric Guitar",
            Kind::Bass => "Bass",
            Kind::Drums => "Drum Kit",
            Kind::StringEnsemble => "String Ensemble",
            Kind::SlowStrings => "Slow Strings",
            Kind::Violin => "Violin",
            Kind::Viola => "Viola",
            Kind::Cello => "Cello",
            Kind::Contrabass => "Contrabass",
            Kind::Pizzicato => "Pizzicato",
            Kind::Trumpet => "Trumpet",
            Kind::FrenchHorns => "French Horns",
            Kind::SynthBrass1 => "Synth Brass 1",
            Kind::SynthBrass2 => "Synth Brass 2",
            Kind::SynthStrings1 => "Synth Strings 1",
            Kind::SynthStrings2 => "Synth Strings 2",
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
            Kind::StringEnsemble => 3.0,
            Kind::SlowStrings => 3.0,
            Kind::Violin => 2.2,
            Kind::Viola => 2.1,
            Kind::Cello => 2.1,
            Kind::Contrabass => 2.5,
            Kind::Pizzicato => 1.2,
            Kind::Trumpet => 2.2,
            Kind::FrenchHorns => 2.1,
            Kind::SynthBrass1 => 1.4,
            Kind::SynthBrass2 => 1.0,
            Kind::SynthStrings1 => 2.1,
            Kind::SynthStrings2 => 2.3,
        }
    }

    /// 这件乐器用哪个内嵌资产；四件独奏弦乐共享同一个 SoundFont。
    fn asset(self) -> Asset {
        match self {
            Kind::Piano => Asset::Piano,
            Kind::NylonGuitar => Asset::NylonGuitar,
            Kind::ElectricGuitar => Asset::ElectricGuitar,
            Kind::Bass => Asset::Bass,
            Kind::Drums => Asset::Drums,
            Kind::StringEnsemble | Kind::SlowStrings => Asset::Strings,
            Kind::Violin | Kind::Viola | Kind::Cello | Kind::Contrabass => Asset::StringQuartet,
            Kind::Pizzicato => Asset::Pizzicato,
            Kind::Trumpet => Asset::Trumpet,
            Kind::FrenchHorns => Asset::FrenchHorns,
            Kind::SynthBrass1 => Asset::SynthBrass1,
            Kind::SynthBrass2 => Asset::SynthBrass2,
            Kind::SynthStrings1 => Asset::SynthStrings1,
            Kind::SynthStrings2 => Asset::SynthStrings2,
        }
    }

    /// 资产里的 (bank, program)。同一个 SoundFont 可以有多个预设。
    fn preset(self) -> (i32, i32) {
        match self {
            Kind::Piano
            | Kind::NylonGuitar
            | Kind::ElectricGuitar
            | Kind::Bass
            | Kind::Drums
            | Kind::SynthBrass1
            | Kind::SynthBrass2
            | Kind::SynthStrings1
            | Kind::SynthStrings2 => (0, 0),
            Kind::StringEnsemble => (0, 48),
            Kind::SlowStrings => (0, 49),
            Kind::Violin => (0, 40),
            Kind::Viola => (0, 41),
            Kind::Cello => (0, 42),
            Kind::Contrabass => (0, 43),
            Kind::Pizzicato => (0, 45),
            Kind::Trumpet => (0, 56),
            Kind::FrenchHorns => (0, 60),
        }
    }
}

/// 内嵌的 SoundFont 资产。缓存按资产共享，四件独奏弦乐只解一次。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asset {
    Piano,
    NylonGuitar,
    ElectricGuitar,
    Bass,
    Drums,
    StringQuartet,
    Strings,
    Pizzicato,
    Trumpet,
    FrenchHorns,
    SynthBrass1,
    SynthBrass2,
    SynthStrings1,
    SynthStrings2,
}

impl Asset {
    fn bytes(self) -> &'static [u8] {
        match self {
            Asset::Piano => PIANO_BYTES,
            Asset::NylonGuitar => NYLON_BYTES,
            Asset::ElectricGuitar => ELECTRIC_BYTES,
            Asset::Bass => BASS_BYTES,
            Asset::Drums => DRUMS_BYTES,
            Asset::StringQuartet => STRING_QUARTET_BYTES,
            Asset::Strings => STRINGS_BYTES,
            Asset::Pizzicato => PIZZICATO_BYTES,
            Asset::Trumpet => TRUMPET_BYTES,
            Asset::FrenchHorns => FRENCH_HORNS_BYTES,
            Asset::SynthBrass1 => SYNTH_BRASS_1_BYTES,
            Asset::SynthBrass2 => SYNTH_BRASS_2_BYTES,
            Asset::SynthStrings1 => SYNTH_STRINGS_1_BYTES,
            Asset::SynthStrings2 => SYNTH_STRINGS_2_BYTES,
        }
    }

    fn cache(self) -> &'static OnceLock<Option<Arc<SoundFont>>> {
        match self {
            Asset::Piano => &PIANO_FONT,
            Asset::NylonGuitar => &NYLON_FONT,
            Asset::ElectricGuitar => &ELECTRIC_FONT,
            Asset::Bass => &BASS_FONT,
            Asset::Drums => &DRUMS_FONT,
            Asset::StringQuartet => &STRING_QUARTET_FONT,
            Asset::Strings => &STRINGS_FONT,
            Asset::Pizzicato => &PIZZICATO_FONT,
            Asset::Trumpet => &TRUMPET_FONT,
            Asset::FrenchHorns => &FRENCH_HORNS_FONT,
            Asset::SynthBrass1 => &SYNTH_BRASS_1_FONT,
            Asset::SynthBrass2 => &SYNTH_BRASS_2_FONT,
            Asset::SynthStrings1 => &SYNTH_STRINGS_1_FONT,
            Asset::SynthStrings2 => &SYNTH_STRINGS_2_FONT,
        }
    }
}

static PIANO_BYTES: &[u8] = include_bytes!("../assets/piano.sf3");
static NYLON_BYTES: &[u8] = include_bytes!("../assets/nylon_guitar.sf3");
static ELECTRIC_BYTES: &[u8] = include_bytes!("../assets/electric_guitar.sf3");
static BASS_BYTES: &[u8] = include_bytes!("../assets/bass.sf3");
static DRUMS_BYTES: &[u8] = include_bytes!("../assets/drums.sf2");
static STRING_QUARTET_BYTES: &[u8] = include_bytes!("../assets/string_quartet.sf3");
static STRINGS_BYTES: &[u8] = include_bytes!("../assets/strings.sf3");
static PIZZICATO_BYTES: &[u8] = include_bytes!("../assets/pizzicato.sf3");
static TRUMPET_BYTES: &[u8] = include_bytes!("../assets/trumpet.sf3");
static FRENCH_HORNS_BYTES: &[u8] = include_bytes!("../assets/french_horns.sf3");
static SYNTH_BRASS_1_BYTES: &[u8] = include_bytes!("../assets/synth_brass_1.sf3");
static SYNTH_BRASS_2_BYTES: &[u8] = include_bytes!("../assets/synth_brass_2.sf3");
static SYNTH_STRINGS_1_BYTES: &[u8] = include_bytes!("../assets/synth_strings_1.sf3");
static SYNTH_STRINGS_2_BYTES: &[u8] = include_bytes!("../assets/synth_strings_2.sf3");

static PIANO_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static NYLON_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static ELECTRIC_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static BASS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static DRUMS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static STRING_QUARTET_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static STRINGS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static PIZZICATO_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static TRUMPET_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static FRENCH_HORNS_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static SYNTH_BRASS_1_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static SYNTH_BRASS_2_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static SYNTH_STRINGS_1_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();
static SYNTH_STRINGS_2_FONT: OnceLock<Option<Arc<SoundFont>>> = OnceLock::new();

fn load_font(kind: Kind) -> Option<Arc<SoundFont>> {
    let asset = kind.asset();
    let mut cursor = Cursor::new(asset.bytes());
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
    kind.asset().cache().get_or_init(|| load_font(kind)).clone()
}

/// 一个采样乐器实例。
pub struct SoundFontPlugin {
    kind: Kind,
    synth: Option<Synthesizer>,
    sample_rate: f64,
    /// 通道弯音范围（半音）；由 `Plugin::set_pitch_bend_range` 设置。
    pitch_bend_range: f32,
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
            pitch_bend_range: 2.0,
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
            // 采样器没有逐声部变调接口；`per_note_pitch` 报 false，逐音事件由引擎折叠成通道弯音。
            // 这里仍防御性地处理，避免漏掉。
            MidiEvent::PitchBend { cents } => {
                let bytes = pitch_bend_midi_bytes(cents, self.pitch_bend_range);
                synth.process_midi_message(0, 0xE0, bytes[1] as i32, bytes[2] as i32);
            }
            MidiEvent::NotePitch { cents, .. } => {
                let bytes = pitch_bend_midi_bytes(cents, self.pitch_bend_range);
                synth.process_midi_message(0, 0xE0, bytes[1] as i32, bytes[2] as i32);
            }
            MidiEvent::ControlChange { controller, value } => {
                synth.process_midi_message(0, 0xB0, controller as i32, value as i32);
            }
            // 采样器没有逐声部增益接口；`per_note_volume` 报 false，忽略该事件。
            MidiEvent::NoteVolume { .. } => {}
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
                per_note_pitch: false,
                channel_pitch_bend: true,
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
        if let Some(synth) = self.synth.as_mut() {
            let (bank, patch) = self.kind.preset();
            if bank != 0 {
                synth.process_midi_message(0, 0xB0, 0x00, bank);
            }
            synth.process_midi_message(0, 0xC0, patch, 0);
        }
    }

    fn set_pitch_bend_range(&mut self, semitones: f32) {
        self.pitch_bend_range = if semitones.is_finite() && semitones > 0.0 {
            semitones.clamp(0.01, 24.0)
        } else {
            2.0
        };
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
            (Kind::StringEnsemble, 60),
            (Kind::SlowStrings, 60),
            (Kind::Violin, 72),
            (Kind::Viola, 60),
            (Kind::Cello, 48),
            (Kind::Contrabass, 36),
            (Kind::Pizzicato, 60),
            (Kind::Trumpet, 67),
            (Kind::FrenchHorns, 60),
            (Kind::SynthBrass1, 60),
            (Kind::SynthBrass2, 60),
            (Kind::SynthStrings1, 60),
            (Kind::SynthStrings2, 60),
        ] {
            let out = render(kind, note, 100);
            let peak = out.iter().fold(0.0f32, |a, b| a.max(b.abs()));
            assert!(peak > 1e-4, "{} 应当发声（peak={peak}）", kind.id());
        }
    }

    /// 多预设资产必须挂对 program：四件独奏弦乐共享一个 SoundFont，只靠 preset 区分。
    #[test]
    fn multi_preset_fonts_select_the_right_program() {
        assert_eq!(Kind::StringEnsemble.preset(), (0, 48));
        assert_eq!(Kind::SlowStrings.preset(), (0, 49));
        assert_eq!(Kind::Violin.preset(), (0, 40));
        assert_eq!(Kind::Viola.preset(), (0, 41));
        assert_eq!(Kind::Cello.preset(), (0, 42));
        assert_eq!(Kind::Contrabass.preset(), (0, 43));
        assert_eq!(Kind::Pizzicato.preset(), (0, 45));
        assert_eq!(Kind::Trumpet.preset(), (0, 56));
        assert_eq!(Kind::FrenchHorns.preset(), (0, 60));
        // 四件独奏弦乐共享资产；弦乐群奏与慢弦乐也共享资产。
        assert_eq!(Kind::Violin.asset(), Kind::Contrabass.asset());
        assert_eq!(Kind::StringEnsemble.asset(), Kind::SlowStrings.asset());
    }

    /// 通道弯音必须真的改变采样器的播放音高（rustysynth 的 0xE0 路径）。
    #[test]
    fn channel_pitch_bend_changes_the_sampler_output() {
        let render = |events: &[TimedMidiEvent]| {
            let mut plugin = SoundFontPlugin::new(Kind::ElectricGuitar);
            plugin.prepare(48_000.0, 256);
            let ctx = ProcessContext {
                sample_rate: 48_000.0,
                block_size: 256,
                tempo: 120.0,
                playing: true,
            };
            let mut left = vec![0.0f32; 8192];
            let mut right = vec![0.0f32; 8192];
            plugin.process(&ctx, &mut [&mut left, &mut right], events);
            left
        };
        let base = render(&[TimedMidiEvent {
            frame: 0,
            event: MidiEvent::NoteOn { note: 60, velocity: 100 },
        }]);
        let bent = render(&[
            TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 60, velocity: 100 } },
            TimedMidiEvent { frame: 0, event: MidiEvent::PitchBend { cents: 200 } },
        ]);
        let diff: f32 = base.iter().zip(&bent).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff > 1.0, "弯音应当改变波形（diff={diff}）");

        let crossings = |x: &[f32]| x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!(
            crossings(&bent) > crossings(&base),
            "上推应当提高过零率：base={} bent={}",
            crossings(&base),
            crossings(&bent)
        );
    }

    #[test]
    fn every_kind_has_a_unique_id() {
        let mut ids: Vec<&str> = Kind::ALL.iter().map(|k| k.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), Kind::ALL.len(), "Kind::ALL 里有重复 id");
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
