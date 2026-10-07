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
