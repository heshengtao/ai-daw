#![forbid(unsafe_code)]

//! 音频素材的**解码与波形包络**。
//!
//! 只做「文件 → 交错 f32」这一件事：解码成 [`ai_daw_engine::audio::DecodedAudio`]，
//! 或算出画波形用的 min/max 包络。**不认识 UI / Tauri**，也不碰播放路径——引擎只认已经
//! 解码好的数据（播放回调里绝不解码，见 `crates/engine/tests/realtime_safety.rs`）。
//!
//! 解码用 `symphonia`（wav / flac / mp3 / ogg-vorbis / aac / m4a）。注意它是 **MPL-2.0**：
//! 仅作为**未修改**的依赖使用，见 `NOTICE` 与 ROADMAP §3。

use std::path::Path;

use ai_daw_engine::audio::DecodedAudio;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// 解码失败的原因。
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    /// 打不开文件（不存在 / 无权限）。
    #[error("打不开音频文件：{0}")]
    Io(#[from] std::io::Error),
    /// 容器格式无法识别。
    #[error("无法识别音频格式：{0}")]
    Probe(String),
    /// 文件里没有音频轨道。
    #[error("文件里没有音频轨道")]
    NoAudioTrack,
    /// 缺少解码必需的信息（采样率或声道未知）。
    #[error("缺少解码参数（采样率/声道未知）")]
    MissingParams,
    /// 解码过程中出错。
    #[error("解码失败：{0}")]
    Decode(String),
}

/// 解码进度：已解出的帧数，以及（容器头能预知时的）总帧数。
///
/// `total_frames` 来自轨道头里的帧数——wav / flac / m4a / 带 Xing 头的 mp3 都能预知，于是界面
/// 可以画**确定进度**；预知不了时给 `None`，界面改画不确定态。
#[derive(Debug, Clone, Copy)]
pub struct DecodeProgress {
    /// 已解出的帧数（每声道采样数）。
    pub frames: u64,
    /// 总帧数；`None` = 容器头没给，进度不可预知。
    pub total_frames: Option<u64>,
}

/// 进度回调的最小间隔（按墙钟节流）：无论格式与文件长短，都不会刷出成百上千条事件。
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(120);

/// 预分配采样的上限，防止头部帧数离谱的文件一次性申请巨量内存（超过就让它自然增长）。
const MAX_PREALLOC_SAMPLES: u64 = 64 << 20;

/// 解码一个音频文件为交错 f32 素材。
///
/// - 声道数收敛到 1 或 2：多声道（5.1 等）取前两个声道当 L/R（v1 不做真正的下混）。
/// - 采样率**保持素材自身的**：调用方按需换算，绝不在这里重采样。
/// - 单个坏帧会被跳过（`DecodeError`）；只有致命错误才让整段素材失败。
pub fn decode(path: impl AsRef<Path>) -> Result<DecodedAudio, AssetError> {
    decode_with_progress(path, |_| {})
}

/// 同 [`decode`]，但解码过程中按 [`PROGRESS_INTERVAL`] 节流回调进度（供界面画进度条）。
pub fn decode_with_progress(
    path: impl AsRef<Path>,
    mut on_progress: impl FnMut(DecodeProgress),
) -> Result<DecodedAudio, AssetError> {
    let path = path.as_ref();
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // 扩展名只是**提示**：真正的判定靠探测（很多文件后缀不可靠）。
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| AssetError::Probe(e.to_string()))?;

    let track = format.default_track(TrackType::Audio).ok_or(AssetError::NoAudioTrack)?;
    let track_id = track.id;
    let total_frames = track.num_frames.filter(|n| *n > 0);
    let params = match track.codec_params.as_ref() {
        Some(CodecParameters::Audio(p)) => p.clone(),
        _ => return Err(AssetError::MissingParams),
    };
    let source_rate = params.sample_rate.ok_or(AssetError::MissingParams)? as f64;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| AssetError::Decode(e.to_string()))?;

    let mut channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(0);
    // 头里给了总帧数就一次性备好缓冲：避免解码途中反复 realloc（大文件能省下几十次整块 memcpy）。
    let expected = total_frames
        .map(|n| n.saturating_mul(channels.max(1) as u64).min(MAX_PREALLOC_SAMPLES))
        .unwrap_or(0) as usize;
    let mut samples: Vec<f32> = Vec::with_capacity(expected);
    let mut last_report = std::time::Instant::now();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(e) => return Err(AssetError::Decode(e.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                channels = decoded.spec().channels().count();
                // 直接把这一包写进总缓冲的**尾部**：省掉「先写临时缓冲、再整块追加」的那次多余 memcpy。
                let n = decoded.samples_interleaved();
                let start = samples.len();
                samples.resize(start + n, 0.0);
                decoded.copy_to_slice_interleaved(&mut samples[start..]);
                if last_report.elapsed() >= PROGRESS_INTERVAL {
                    let frames = samples.len() as u64 / channels.max(1) as u64;
                    on_progress(DecodeProgress { frames, total_frames });
                    last_report = std::time::Instant::now();
                }
            }
            // 坏帧跳过：宁可缺一小段，也不要整段素材报废。
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(AssetError::Decode(e.to_string())),
        }
    }

    if samples.is_empty() {
        return Err(AssetError::Decode("没有解出任何采样".into()));
    }
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let (out_channels, out_samples) = if channels <= 2 {
        (channels, samples)
    } else {
        let mut stereo = Vec::with_capacity(frames * 2);
        for f in 0..frames {
            stereo.push(samples[f * channels]);
            stereo.push(samples[f * channels + 1]);
        }
        (2, stereo)
    };

    on_progress(DecodeProgress { frames: out_samples.len() as u64 / out_channels as u64, total_frames });
    Ok(DecodedAudio {
        samples: out_samples.into(),
        channels: out_channels,
        sample_rate: source_rate,
    })
}

/// 波形的 min/max 包络：每个桶两个值（先 min 后 max），长度 = `buckets * 2`。
///
/// 直接给画波形用：每个像素列取该列覆盖帧范围内最低与最高的采样。多声道先按帧取平均，
/// 得到一条包络。空素材或 `buckets == 0` 时返回全 0。
pub fn peaks(audio: &DecodedAudio, buckets: usize) -> Vec<f32> {
    peaks_range(audio, 0, audio.frames() as u64, buckets)
}

/// 与 [`peaks`] 相同，但只取素材的 `[offset_frames, offset_frames + frames)` 这一段。
///
/// 片段（`Clip.audio`）只引用素材的一个窗口，波形也必须只画这个窗口——否则被切分/裁剪过的
/// 片段会显示**整段素材**，看上去像「被压缩的完整波形」。
pub fn peaks_range(
    audio: &DecodedAudio,
    offset_frames: u64,
    frames: u64,
    buckets: usize,
) -> Vec<f32> {
    if buckets == 0 {
        return Vec::new();
    }
    let total = audio.frames() as u64;
    let start_f = offset_frames.min(total);
    let end_f = start_f.saturating_add(frames).min(total);
    let span = end_f.saturating_sub(start_f);
    let channels = audio.channels.max(1);
    if span == 0 {
        return vec![0.0; buckets * 2];
    }
    let mut out = Vec::with_capacity(buckets * 2);
    for b in 0..buckets {
        let start = start_f + (b as u64 * span) / buckets as u64;
        let end = (start_f + ((b as u64 + 1) * span) / buckets as u64).clamp(start + 1, end_f);
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for f in start..end {
            let base = (f as usize) * channels;
            let mut sum = 0.0f32;
            for c in 0..channels {
                sum += audio.samples.get(base + c).copied().unwrap_or(0.0);
            }
            let v = sum / channels as f32;
            if v < lo {
                lo = v;
            }
            if v > hi {
                hi = v;
            }
        }
        out.push(lo);
        out.push(hi);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    /// 生成一段 WAV 素材（16-bit PCM，供测试解码）。
    fn write_wav(dir: &std::path::Path, name: &str, rate: u32, channels: u16, seconds: f64, freq: f64) -> PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let frames = (seconds * rate as f64) as u32;
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for f in 0..frames {
            let v = (std::f64::consts::TAU * freq * f as f64 / rate as f64).sin()
                * 0.9
                * i16::MAX as f64;
            let v = v as i16;
            for _ in 0..channels {
                writer.write_sample(v).unwrap();
            }
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn decodes_a_mono_wav_with_its_own_sample_rate() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_wav(dir.path(), "mono.wav", 44_100, 1, 0.25, 1_000.0);

        let audio = decode(&path).unwrap();
        assert_eq!(audio.channels, 1);
        assert_eq!(audio.sample_rate, 44_100.0);
        assert_eq!(audio.frames(), 11_025, "0.25 s @ 44.1 kHz");
        let peak = audio.samples.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.85 && peak <= 1.0, "满幅素材的峰值应当接近 0.9，实际 {peak}");
    }

    #[test]
    fn decodes_a_stereo_wav_with_two_channels() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_wav(dir.path(), "stereo.wav", 48_000, 2, 0.1, 1_000.0);

        let audio = decode(&path).unwrap();
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 48_000.0);
        assert_eq!(audio.frames(), 4_800);
        assert_eq!(audio.samples.len(), 9_600, "交错：帧数 × 声道数");
    }

    #[test]
    fn decode_reports_progress_ending_at_the_track_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_wav(dir.path(), "progress.wav", 44_100, 2, 0.5, 1_000.0);

        let mut reports: Vec<DecodeProgress> = Vec::new();
        let audio = decode_with_progress(&path, |p| reports.push(p)).unwrap();
        assert_eq!(audio.frames(), 22_050, "0.5 s @ 44.1 kHz");

        let last = reports.last().expect("至少要回调一次进度");
        assert_eq!(last.frames, 22_050, "最终报告应落在素材总长上");
        assert!(reports.iter().all(|p| p.frames <= 22_050), "进度只增不减、不越界");
        // wav 头里有总帧数：给值时必须等于真实长度（拿不到也属正常，界面画不确定态）。
        assert!(matches!(last.total_frames, None | Some(22_050)));
    }

    #[test]
    fn peaks_bracket_the_signal_in_every_bucket() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_wav(dir.path(), "peaks.wav", 48_000, 1, 0.5, 1_000.0);
        let audio = decode(&path).unwrap();

        let buckets = 16;
        let env = peaks(&audio, buckets);
        assert_eq!(env.len(), buckets * 2, "每桶 min/max 两个值");
        // 0.5 s 里有 500 个 1 kHz 周期，每个桶都跨了整周期 → min≈-峰值、max≈+峰值。
        for b in 0..buckets {
            let lo = env[b * 2];
            let hi = env[b * 2 + 1];
            assert!(lo < -0.8, "第 {b} 桶的 min 应当接近负峰，实际 {lo}");
            assert!(hi > 0.8, "第 {b} 桶的 max 应当接近正峰，实际 {hi}");
        }
    }

    #[test]
    fn peaks_handle_empty_input_without_panicking() {
        let empty = DecodedAudio { samples: Arc::from(Vec::<f32>::new()), channels: 2, sample_rate: 48_000.0 };
        assert_eq!(peaks(&empty, 4), vec![0.0; 8]);
        assert!(peaks(&empty, 0).is_empty());
    }

    /// 只画**窗口**：切分/裁剪后的片段不能显示整段素材。
    #[test]
    fn peaks_range_covers_only_the_requested_window() {
        // 阶梯素材：前 100 帧静音，后 100 帧满幅。
        let mut samples = vec![0.0f32; 100];
        samples.extend(std::iter::repeat(1.0f32).take(100));
        let audio = DecodedAudio { samples: Arc::from(samples), channels: 1, sample_rate: 48_000.0 };

        // 取后半段：每个桶的 max 都是 1.0。
        let second = peaks_range(&audio, 100, 100, 4);
        for b in 0..4 {
            assert!((second[b * 2 + 1] - 1.0).abs() < 1e-6, "第 {b} 桶应是满幅");
        }
        // 取前半段：全 0。
        assert_eq!(peaks_range(&audio, 0, 100, 4), vec![0.0; 8]);
        // 整段分两桶：第一桶 0、第二桶 1。
        let whole = peaks_range(&audio, 0, 200, 2);
        assert!(whole[1].abs() < 1e-6, "第一桶应静音");
        assert!((whole[3] - 1.0).abs() < 1e-6, "第二桶应满幅");
        // 越界窗口收敛，不 panic。
        assert_eq!(peaks_range(&audio, 500, 100, 2), vec![0.0; 4]);
    }

    #[test]
    fn missing_file_is_an_error() {
        let err = decode("/definitely/not/here.wav").unwrap_err();
        assert!(matches!(err, AssetError::Io(_)), "应当是 IO 错误，实际 {err}");
    }

    #[test]
    fn non_audio_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-audio.txt");
        std::fs::write(&path, b"hello, this is definitely not audio").unwrap();
        let err = decode(&path).unwrap_err();
        assert!(
            matches!(err, AssetError::Probe(_) | AssetError::NoAudioTrack | AssetError::Decode(_)),
            "应当是「认不出格式」类错误，实际 {err}"
        );
    }
}
