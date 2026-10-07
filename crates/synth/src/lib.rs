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
    /// 构造合成器，使用默认参数（gain 0.8、attack 0.01 s、release 0.3 s）。
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

    fn process(&mut self, _ctx: &ProcessContext, out: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
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
