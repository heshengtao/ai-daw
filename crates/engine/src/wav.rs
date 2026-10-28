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
pub fn write_wav(
    path: impl AsRef<Path>,
    buffer: &AudioBuffer,
    sample_rate: u32,
) -> Result<(), EngineError> {
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

/// 把交错缓冲编码为 16-bit PCM WAV 字节。
pub fn wav_bytes(buffer: &AudioBuffer, sample_rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: buffer.channels() as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, spec).expect("cursor never fails");
        for &s in buffer.data() {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .expect("cursor never fails");
        }
        writer.finalize().expect("cursor never fails");
    }
    out.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Engine;
    use crate::command::Command;
    use crate::event::{Actor, Event};
    use crate::model::Note;
    use crate::registry::PluginRegistry;
    use crate::render::{render, RenderOptions};

    /// 最小的 demo 工程：一条轨道 + 合成器 + 一个含单音的音符。
    fn demo() -> crate::model::Project {
        let mut e = Engine::new("wav-bytes");
        let ev = e.execute(
            Command::AddTrack {
                name: "Lead".into(),
            },
            Actor::Human,
        );
        let track = match ev[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        e.execute(
            Command::SetTrackInstrument {
                track,
                plugin_id: "builtin.synth".into(),
            },
            Actor::Human,
        );
        let ev = e.execute(
            Command::AddClip {
                track,
                start: 0.0,
                length: 1.0,
            },
            Actor::Human,
        );
        let clip = match ev[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!(),
        };
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch: 69,
                    start: 0.0,
                    duration: 0.5,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: crate::model::PitchExpression::NONE,
                    lyric: None,
                },
            },
            Actor::Human,
        );
        e.project().clone()
    }

    #[test]
    fn wav_bytes_starts_with_riff_and_exceeds_header() {
        let project = demo();
        let opts = RenderOptions {
            sample_rate: 44_100.0,
            tail_seconds: 0.1,
            settle_seconds: 0.0,
        };
        let buffer = render(&project, &PluginRegistry::with_builtins(), &opts);

        let bytes = wav_bytes(&buffer, 44_100);

        assert_eq!(
            &bytes[..4],
            b"RIFF",
            "wav bytes must start with the RIFF magic"
        );
        assert!(
            bytes.len() > 44,
            "wav bytes must be longer than the 44-byte header"
        );
    }
}
