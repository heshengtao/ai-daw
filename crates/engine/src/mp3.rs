//! MP3 编码（LAME）：把渲染好的立体声缓冲写成文件或字节。
//!
//! 与 [`crate::wav`] 对称——导出时按用户选的格式走其中一个。三点差别值得记住：
//!
//! - **有损**。LAME 是 MP3 的参考实现，但 MP3 终究是有损格式；比特率由调用方给
//!   （默认见 [`DEFAULT_BITRATE_KBPS`]），质量档固定用 LAME 能给的最高档。
//! - **采样率有硬约束**。MPEG-1 只认 32 / 44.1 / 48 kHz，其余采样率在这里**直接报错**，
//!   而不是交给 LAME 悄悄重采样：工程渲染在什么采样率上，导出文件就该标什么采样率。
//! - **首尾有几毫秒的编码器延迟 / 补零**（MP3 帧长固定 1152 个样本，LAME 会在结尾补齐）。
//!   所以 MP3 天然不是「逐位等于渲染结果」的格式——要原样交付就用 WAV。
//!
//! **许可证**：本模块是全仓库唯一的 LGPL 依赖点（`mp3lame-encoder` 封装**原封不动**的
//! LAME 3.100）。例外写在仓库根的 `deny.toml`，声明写在 `NOTICE`。
//!
//! 实时安全无关：本模块只在离线导出时调用，不在音频回调里。

use std::path::Path;

use mp3lame_encoder::{
    Bitrate, Builder, EncodeError, Encoder, FlushNoGap, InterleavedPcm, Quality,
};
use thiserror::Error;

use crate::audio::AudioBuffer;

/// LAME 每次调用最多吃多少帧：一个 MPEG-1 Layer III 帧（每声道 1152 个样本）。
const CHUNK_FRAMES: usize = 1152;

/// 冲尾（flush）前至少要留出的字节数（LAME 的硬要求）。
const FLUSH_CAPACITY: usize = 7200;

/// 导出 MP3 的默认比特率（kbps）。
///
/// 320 = MPEG-1 的上限：导出是把工程交出去的最后一步，默认不该是「能听就行」的质量。
pub const DEFAULT_BITRATE_KBPS: u32 = 320;

/// MP3 可选的比特率（kbps，MPEG-1 的全部合法档位，由高到低）。界面直接照它列选项。
pub const BITRATES_KBPS: [u32; 16] =
    [320, 256, 224, 192, 160, 128, 112, 96, 80, 64, 48, 40, 32, 24, 16, 8];

/// MP3 支持的采样率（Hz）：MPEG-1 的三个值（见模块文档）。
pub const SAMPLE_RATES: [u32; 3] = [48_000, 44_100, 32_000];

/// MP3 编码错误。
#[derive(Debug, Error)]
pub enum Mp3Error {
    #[error("MP3 不支持 {0} Hz 采样率（MPEG-1 只支持 32000 / 44100 / 48000）")]
    UnsupportedSampleRate(u32),
    #[error("MP3 不支持 {0} kbps 比特率")]
    UnsupportedBitrate(u32),
    #[error("MP3 导出要求立体声，当前缓冲是 {0} 声道")]
    UnsupportedChannels(usize),
    #[error("MP3 编码器初始化失败：{0}")]
    Build(#[from] mp3lame_encoder::BuildError),
    #[error("MP3 编码失败：{0}")]
    Encode(#[from] EncodeError),
    #[error("MP3 写盘失败：{0}")]
    Io(#[from] std::io::Error),
}

/// 把交错立体声缓冲编码为 MP3 字节。
///
/// `sample_rate` 必须落在 [`SAMPLE_RATES`] 里，`bitrate_kbps` 必须落在 [`BITRATES_KBPS`] 里
/// （越界一律报错，不做静默夹取——导出文件的质量参数不该被悄悄改掉）。
pub fn mp3_bytes(
    buffer: &AudioBuffer,
    sample_rate: u32,
    bitrate_kbps: u32,
) -> Result<Vec<u8>, Mp3Error> {
    if buffer.channels() != 2 {
        return Err(Mp3Error::UnsupportedChannels(buffer.channels()));
    }
    let mut encoder = builder(sample_rate, bitrate_kbps)?;

    // 输出体积的粗估：比特率 × 时长。只是给 `Vec` 一个起点，LAME 写多少就是多少。
    let seconds = buffer.frames() as f64 / sample_rate as f64;
    let estimate = (bitrate_kbps as f64 * 1000.0 / 8.0 * seconds) as usize;
    let mut out: Vec<u8> = Vec::with_capacity(estimate + FLUSH_CAPACITY);

    for chunk in buffer.data().chunks(CHUNK_FRAMES * 2) {
        encode_chunk(&mut encoder, chunk, &mut out)?;
    }
    // 收尾：把 LAME 内部还留着的最后一帧吐出来（帧长固定，不足的补零）。
    out.reserve(FLUSH_CAPACITY);
    encoder.flush_to_vec::<FlushNoGap>(&mut out)?;
    Ok(out)
}

/// 把交错立体声缓冲写成 MP3 文件。
pub fn write_mp3(
    path: impl AsRef<Path>,
    buffer: &AudioBuffer,
    sample_rate: u32,
    bitrate_kbps: u32,
) -> Result<(), Mp3Error> {
    let bytes = mp3_bytes(buffer, sample_rate, bitrate_kbps)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// 按采样率 / 比特率建一个 CBR 编码器。
fn builder(sample_rate: u32, bitrate_kbps: u32) -> Result<Encoder, Mp3Error> {
    if !SAMPLE_RATES.contains(&sample_rate) {
        return Err(Mp3Error::UnsupportedSampleRate(sample_rate));
    }
    // `Builder::new` 返回 `None` = LAME 分配不出状态结构（几乎只可能是内存耗尽）。
    let builder = Builder::new().ok_or(mp3lame_encoder::BuildError::NoMem)?;
    Ok(builder
        .with_num_channels(2)?
        .with_sample_rate(sample_rate)?
        .with_brate(bitrate(bitrate_kbps)?)?
        .with_quality(Quality::Best)?
        .build()?)
}

/// kbps → LAME 的档位枚举。不在档位表里的值报错（而不是夹到最近的一档）。
fn bitrate(kbps: u32) -> Result<Bitrate, Mp3Error> {
    Ok(match kbps {
        8 => Bitrate::Kbps8,
        16 => Bitrate::Kbps16,
        24 => Bitrate::Kbps24,
        32 => Bitrate::Kbps32,
        40 => Bitrate::Kbps40,
        48 => Bitrate::Kbps48,
        64 => Bitrate::Kbps64,
        80 => Bitrate::Kbps80,
        96 => Bitrate::Kbps96,
        112 => Bitrate::Kbps112,
        128 => Bitrate::Kbps128,
        160 => Bitrate::Kbps160,
        192 => Bitrate::Kbps192,
        224 => Bitrate::Kbps224,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        other => return Err(Mp3Error::UnsupportedBitrate(other)),
    })
}

/// 送一块 PCM 给 LAME，把编出来的字节追加到 `out`。
///
/// `encode_to_vec` 只往 `Vec` 的**剩余容量**里写，所以必须先 `reserve`。缓冲不够时 LAME 返回
/// `BufferTooSmall`（不会静默截断），这里加倍重试一次——正常情况下 1.25 × 样本数 + 7200 的
/// 上界永远够，这条分支是兜底。
fn encode_chunk(encoder: &mut Encoder, chunk: &[f32], out: &mut Vec<u8>) -> Result<(), Mp3Error> {
    let per_channel = chunk.len() / 2;
    let mut extra = per_channel + per_channel / 4 + FLUSH_CAPACITY;
    loop {
        out.reserve(extra);
        match encoder.encode_to_vec(InterleavedPcm(chunk), out) {
            Ok(_) => return Ok(()),
            Err(EncodeError::BufferTooSmall) if extra < 1 << 26 => extra *= 2,
            Err(err) => return Err(err.into()),
        }
    }
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

    /// 最小的 demo 工程：一条轨道 + 合成器 + 一个含单音的音符（与 `wav.rs` 的用例同形）。
    fn demo() -> crate::model::Project {
        let mut e = Engine::new("mp3-demo");
        let ev = e.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match ev[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        e.execute(
            Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() },
            Actor::Human,
        );
        let ev = e.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
        let clip = match ev[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!(),
        };
        e.execute(
            Command::AddNote {
                track,
                clip,
                note: Note { id: 0, pitch: 69, start: 0.0, duration: 2.0, velocity: 100, volume: 127, offset: 0.0, legato: false },
            },
            Actor::Human,
        );
        e.project().clone()
    }

    fn rendered() -> AudioBuffer {
        let opts = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5, settle_seconds: 0.0 };
        render(&demo(), &PluginRegistry::with_builtins(), &opts)
    }

    /// 编出来的第一帧必须是 MPEG 音频同步头（`0xFF Ex`）。
    fn starts_with_frame_sync(bytes: &[u8]) -> bool {
        bytes.len() > 4 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0
    }

    #[test]
    fn mp3_bytes_are_a_plausible_cbr_stream() {
        let buffer = rendered();
        let bytes = mp3_bytes(&buffer, 44_100, DEFAULT_BITRATE_KBPS).expect("应当编码成功");

        assert!(starts_with_frame_sync(&bytes), "开头应当是 MPEG 帧同步头：{:02X?}", &bytes[..4]);
        // 时长 2 拍 @120bpm = 1s 正文 + 0.5s 尾音 ≈ 1.5s；320 kbps → ≈ 60 kB。
        let seconds = buffer.frames() as f64 / 44_100.0;
        let expected = DEFAULT_BITRATE_KBPS as f64 * 1000.0 / 8.0 * seconds;
        let actual = bytes.len() as f64;
        assert!(
            (actual - expected).abs() < expected * 0.2,
            "CBR 体积应当≈比特率×时长：期望≈{expected:.0}，实际 {actual:.0}"
        );
    }

    #[test]
    fn lower_bitrate_produces_a_smaller_file() {
        let buffer = rendered();
        let big = mp3_bytes(&buffer, 44_100, 320).unwrap();
        let small = mp3_bytes(&buffer, 44_100, 64).unwrap();
        assert!(small.len() < big.len(), "64 kbps 必须比 320 kbps 小");
    }

    #[test]
    fn out_of_range_parameters_are_rejected_not_clamped() {
        let buffer = rendered();
        // 96 kHz 不是 MPEG-1 支持的采样率：报错，而不是被 LAME 悄悄重采样成 48k。
        assert!(matches!(
            mp3_bytes(&buffer, 96_000, 320),
            Err(Mp3Error::UnsupportedSampleRate(96_000))
        ));
        // 100 kbps 不在档位表里。
        assert!(matches!(
            mp3_bytes(&buffer, 44_100, 100),
            Err(Mp3Error::UnsupportedBitrate(100))
        ));
        // 单声道缓冲：本模块只负责立体声（引擎渲染出来的一直是立体声）。
        let mono = AudioBuffer::silence(1, 128);
        assert!(matches!(mp3_bytes(&mono, 44_100, 320), Err(Mp3Error::UnsupportedChannels(1))));
    }

    #[test]
    fn write_mp3_puts_bytes_on_disk() {
        let buffer = rendered();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp3");

        write_mp3(&path, &buffer, 44_100, DEFAULT_BITRATE_KBPS).expect("应当写盘成功");
        let bytes = std::fs::read(&path).unwrap();
        assert!(starts_with_frame_sync(&bytes));
    }

    /// 空缓冲也要能走完「编码 → 冲尾」而不 panic（边界：没有内容可导出的工程）。
    #[test]
    fn empty_buffer_does_not_panic() {
        let buffer = AudioBuffer::silence(2, 0);
        let bytes = mp3_bytes(&buffer, 44_100, 128).expect("空缓冲不该报错");
        assert!(bytes.is_empty() || starts_with_frame_sync(&bytes));
    }
}
