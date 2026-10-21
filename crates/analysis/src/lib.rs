//! 工程音频分析：响度 / 频谱 / 峰值。
//!
//! 输入是**交错（interleaved）f32 采样**（与 `ai_daw_engine::audio::AudioBuffer::data()`
//! 一致），这里只做纯计算，不认识工程与渲染——因此离线导出与实时回调都能复用。
//!
//! - 响度：EBU R128 / ITU-R BS.1770（`ebur128`）——积分响度（LUFS）、短时峰值、响度范围（LU）、
//!   采样峰值（dBFS）与真峰值（dBTP）。
//! - 频谱：Hann 窗 + 50% 交叠的 FFT 平均幅度谱（`rustfft`），单边、按窗和归一化，满幅正弦在
//!   其 bin 上约 0 dB。

use ebur128::{EbuR128, Mode};
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;
use serde::{Deserialize, Serialize};

/// 分析用的 FFT 窗长（采样数）。
pub const FFT_SIZE: usize = 2048;
/// 单边幅度谱的 bin 数（`0 ..= FFT_SIZE/2`）。
pub const SPECTRUM_BINS: usize = FFT_SIZE / 2 + 1;

/// 分析失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    /// 声道数为 0、采样率非法，或采样数不是声道数的整数倍。
    BadFormat(String),
    /// `ebur128` 内部报错（模式/状态问题，正常输入不会走到）。
    Loudness(String),
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalysisError::BadFormat(msg) => write!(f, "invalid audio format: {msg}"),
            AnalysisError::Loudness(msg) => write!(f, "loudness analysis failed: {msg}"),
        }
    }
}

impl std::error::Error for AnalysisError {}

/// 响度与峰值（EBU R128）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Loudness {
    /// 积分响度（LUFS）：整段音频的「整体感觉有多响」。
    pub integrated_lufs: f64,
    /// 短时（3 秒窗）响度的最大值（LUFS）。
    pub short_term_max_lufs: f64,
    /// 响度范围（LU）。
    pub loudness_range_lu: f64,
    /// 采样峰值（dBFS，各通道取最大）。
    pub sample_peak_dbfs: f64,
    /// 真峰值（dBTP，各通道取最大）。
    pub true_peak_dbtp: f64,
}

/// 平均幅度谱。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spectrum {
    /// 每个 bin 覆盖的带宽（Hz）。
    pub bin_hz: f64,
    /// 各 bin 的平均幅度（dB，相对满幅；0 dB = 1.0）。
    pub magnitudes_db: Vec<f64>,
}

/// 一次分析的完整结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// 响度 / 峰值。
    pub loudness: Loudness,
    /// 全曲 RMS（dBFS）。
    pub rms_dbfs: f64,
    /// 平均幅度谱。
    pub spectrum: Spectrum,
}

/// 分段能量（相对满幅正弦，dB）。
///
/// 边界按「听感八度」粗分，够模型判断「太闷 / 太亮 / 太薄」，又不至于把 1025 个 bin 灌给模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandEnergy {
    /// 20–120 Hz：超低音 / 底鼓重量。
    pub sub_db: f64,
    /// 120–500 Hz：低频主体 / 贝斯根音。
    pub bass_db: f64,
    /// 500–2000 Hz：中频 / 人声与和弦的肉。
    pub mid_db: f64,
    /// 2000–6000 Hz：临场 / 拨弦与铜管的锐度。
    pub presence_db: f64,
    /// 6000 Hz 以上：空气感 / 噪声与镲片。
    pub air_db: f64,
}

/// 音色指纹：把一整段音频压成几个可比较的标量。
///
/// 这是给「捏音色」的闭环反馈：模型没法听声音，但可以看频谱重心（亮/暗）、
/// 频谱平坦度（噪声性/乐音性）、波峰因数（冲击力）与分段能量，一次只调一个维度。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timbre {
    /// 频谱重心（Hz）：感知上的「亮度」。
    pub spectral_centroid_hz: f64,
    /// 频谱平坦度（0..1）：接近 0 = 乐音/谐波，接近 1 = 噪声。
    pub spectral_flatness: f64,
    /// 过零率（每帧）：噪声与高频内容的粗代理。
    pub zero_crossing_rate: f64,
    /// 波峰因数（dB）：峰值减 RMS，越大越「有冲击力」。
    pub crest_factor_db: f64,
    /// 分段能量。
    pub bands: BandEnergy,
}

/// 音色指纹：从交错采样里算 [`Timbre`]。
pub fn timbre(
    interleaved: &[f32],
    channels: usize,
    sample_rate: f64,
) -> Result<Timbre, AnalysisError> {
    if channels == 0 {
        return Err(AnalysisError::BadFormat("channels must be >= 1".into()));
    }
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(AnalysisError::BadFormat(format!("bad sample rate: {sample_rate}")));
    }
    if interleaved.len() % channels != 0 {
        return Err(AnalysisError::BadFormat(format!(
            "sample count {} is not a multiple of {} channels",
            interleaved.len(),
            channels
        )));
    }

    let frames = interleaved.len() / channels;
    let mono = mono(interleaved, channels, frames);
    let spectrum = spectrum(&mono, sample_rate);
    let bin_hz = spectrum.bin_hz;

    let amps: Vec<f64> = spectrum
        .magnitudes_db
        .iter()
        .map(|db| if db.is_finite() { 10f64.powf(db / 20.0) } else { 0.0 })
        .collect();

    // 用**功率**加权：谱底噪（约 -90 dB）在幅度加权下会把重心明显抬高，
    // 而听感上的「亮度」更接近能量分布。
    let total: f64 = amps.iter().map(|a| a * a).sum();
    let centroid = if total > 0.0 {
        amps.iter()
            .enumerate()
            .map(|(k, a)| k as f64 * bin_hz * a * a)
            .sum::<f64>()
            / total
    } else {
        0.0
    };

    // 几何平均 / 算术平均：噪声的谱更平，值趋近 1；纯正弦远小于 1。
    let eps = 1e-12;
    let flatness = if amps.is_empty() {
        0.0
    } else {
        let mean = amps.iter().sum::<f64>() / amps.len() as f64;
        if mean <= 0.0 {
            0.0
        } else {
            let log_mean = amps.iter().map(|a| (a + eps).ln()).sum::<f64>() / amps.len() as f64;
            (log_mean.exp() / mean).clamp(0.0, 1.0)
        }
    };

    let crossings = mono.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    let zero_crossing_rate = if mono.is_empty() { 0.0 } else { crossings as f64 / mono.len() as f64 };

    let peak = mono.iter().fold(0.0f64, |acc, x| acc.max(x.abs() as f64));
    let rms = if mono.is_empty() {
        0.0
    } else {
        (mono.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / mono.len() as f64).sqrt()
    };
    let crest_factor_db = if peak > 0.0 && rms > 0.0 { 20.0 * (peak / rms).log10() } else { 0.0 };

    let band = |lo: f64, hi: f64| -> f64 {
        let mut energy = 0.0f64;
        for (k, amp) in amps.iter().enumerate() {
            let f = k as f64 * bin_hz;
            if f >= lo && f < hi {
                energy += amp * amp;
            }
        }
        if energy > 0.0 { (10.0 * energy.log10()).max(-120.0) } else { -120.0 }
    };

    Ok(Timbre {
        spectral_centroid_hz: centroid,
        spectral_flatness: flatness,
        zero_crossing_rate,
        crest_factor_db,
        bands: BandEnergy {
            sub_db: band(20.0, 120.0),
            bass_db: band(120.0, 500.0),
            mid_db: band(500.0, 2_000.0),
            presence_db: band(2_000.0, 6_000.0),
            air_db: band(6_000.0, sample_rate / 2.0),
        },
    })
}

/// 线性幅度 → dB（0 或负值 → -∞）。
fn to_db(x: f64) -> f64 {
    if x > 0.0 {
        20.0 * x.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// 对一整段交错采样做分析。
///
/// `sample_rate` 为采样率（Hz）。空输入会得到「全静音」的报告（响度/峰值/RMS 为 -∞，频谱全 -∞）。
pub fn analyze(
    interleaved: &[f32],
    channels: usize,
    sample_rate: f64,
) -> Result<Report, AnalysisError> {
    if channels == 0 {
        return Err(AnalysisError::BadFormat("channels must be >= 1".into()));
    }
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(AnalysisError::BadFormat(format!("bad sample rate: {sample_rate}")));
    }
    if interleaved.len() % channels != 0 {
        return Err(AnalysisError::BadFormat(format!(
            "sample count {} is not a multiple of {} channels",
            interleaved.len(),
            channels
        )));
    }

    let frames = interleaved.len() / channels;
    let rate = sample_rate.round() as u32;

    Ok(Report {
        loudness: loudness(interleaved, channels, rate, frames)?,
        rms_dbfs: rms_dbfs(interleaved),
        spectrum: spectrum(&mono(interleaved, channels, frames), sample_rate),
    })
}

/// 全曲 RMS（dBFS）。
fn rms_dbfs(interleaved: &[f32]) -> f64 {
    if interleaved.is_empty() {
        return f64::NEG_INFINITY;
    }
    let sum_sq: f64 = interleaved.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    to_db((sum_sq / interleaved.len() as f64).sqrt())
}

/// 把交错采样下混成单声道（各通道求平均）。
fn mono(interleaved: &[f32], channels: usize, frames: usize) -> Vec<f32> {
    if channels == 1 {
        return interleaved.to_vec();
    }
    let inv = 1.0 / channels as f32;
    (0..frames)
        .map(|f| {
            let base = f * channels;
            interleaved[base..base + channels].iter().sum::<f32>() * inv
        })
        .collect()
}

/// 响度与峰值：按 100 ms 推进以追踪短时（3 s 窗）响度的最大值。
fn loudness(
    interleaved: &[f32],
    channels: usize,
    rate: u32,
    frames: usize,
) -> Result<Loudness, AnalysisError> {
    let mode = Mode::I | Mode::LRA | Mode::TRUE_PEAK;
    let mut ebur = EbuR128::new(channels as u32, rate, mode)
        .map_err(|e| AnalysisError::Loudness(e.to_string()))?;

    let mut short_term_max = f64::NEG_INFINITY;
    if frames > 0 {
        let step = (rate as usize / 10).max(1);
        let mut f = 0;
        while f < frames {
            let f_end = (f + step).min(frames);
            ebur.add_frames_f32(&interleaved[f * channels..f_end * channels])
                .map_err(|e| AnalysisError::Loudness(e.to_string()))?;
            if let Ok(st) = ebur.loudness_shortterm() {
                if st.is_finite() && st > short_term_max {
                    short_term_max = st;
                }
            }
            f = f_end;
        }
    }

    let integrated_lufs = if frames > 0 {
        ebur.loudness_global().map_err(|e| AnalysisError::Loudness(e.to_string()))?
    } else {
        f64::NEG_INFINITY
    };
    let loudness_range_lu = if frames > 0 {
        ebur.loudness_range().unwrap_or(0.0)
    } else {
        0.0
    };

    // 两个峰值函数都返回**线性**值，需换算成 dB（见 ebur128 文档）。
    let mut sample_peak = 0.0f64;
    let mut true_peak = 0.0f64;
    for c in 0..channels as u32 {
        if let Ok(v) = ebur.sample_peak(c) {
            sample_peak = sample_peak.max(v);
        }
        if let Ok(v) = ebur.true_peak(c) {
            true_peak = true_peak.max(v);
        }
    }

    Ok(Loudness {
        integrated_lufs: if integrated_lufs.is_finite() { integrated_lufs } else { f64::NEG_INFINITY },
        short_term_max_lufs: short_term_max,
        loudness_range_lu,
        sample_peak_dbfs: to_db(sample_peak),
        true_peak_dbtp: to_db(true_peak),
    })
}

/// Hann 窗（周期型，分母用 `n`）。
fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            0.5 - 0.5 * x.cos()
        })
        .collect()
}

/// 平均幅度谱：Hann 窗 + 50% 交叠，平均功率后开方，单边并按窗和归一化。
fn spectrum(mono: &[f32], sample_rate: f64) -> Spectrum {
    let bin_hz = sample_rate / FFT_SIZE as f64;
    let total = mono.len();
    if total == 0 {
        return Spectrum { bin_hz, magnitudes_db: vec![f64::NEG_INFINITY; SPECTRUM_BINS] };
    }

    let window = hann(FFT_SIZE);
    let win_sum: f64 = window.iter().sum();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    // 各 bin 的功率累加（全复数 bin，最后只取单边）。
    let mut power = vec![0.0f64; FFT_SIZE];
    let mut windows = 0usize;
    let hop = FFT_SIZE / 2;
    let mut start = 0usize;
    loop {
        let mut buf: Vec<Complex<f64>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];
        for (i, slot) in buf.iter_mut().enumerate() {
            let s = if start + i < total { mono[start + i] as f64 } else { 0.0 };
            *slot = Complex::new(s * window[i], 0.0);
        }
        fft.process(&mut buf);
        for (k, c) in buf.iter().enumerate() {
            power[k] += c.norm_sqr();
        }
        windows += 1;
        if start + FFT_SIZE >= total {
            break;
        }
        start += hop;
    }

    let scale = 1.0 / (windows as f64 * win_sum * win_sum);
    let mut magnitudes_db = vec![0.0f64; SPECTRUM_BINS];
    for k in 0..SPECTRUM_BINS {
        let mut amp = (power[k] * scale).sqrt();
        // 单边谱：除 DC 与 Nyquist 外交叠能量折到正频率，故乘以 2。
        if k != 0 && k != FFT_SIZE / 2 {
            amp *= 2.0;
        }
        magnitudes_db[k] = to_db(amp);
    }

    Spectrum { bin_hz, magnitudes_db }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成 `seconds` 秒、`freq` Hz、峰值 `amp` 的立体声正弦（左右同相）。
    fn sine(freq: f64, amp: f64, seconds: f64, rate: f64) -> Vec<f32> {
        let frames = (seconds * rate) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for f in 0..frames {
            let s = (amp * (std::f64::consts::TAU * freq * f as f64 / rate).sin()) as f32;
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn silence_reports_neg_infinity() {
        let report = analyze(&vec![0.0f32; 4800 * 2], 2, 48_000.0).unwrap();
        assert_eq!(report.rms_dbfs, f64::NEG_INFINITY);
        assert_eq!(report.loudness.integrated_lufs, f64::NEG_INFINITY);
        assert_eq!(report.loudness.sample_peak_dbfs, f64::NEG_INFINITY);
        assert!(report.spectrum.magnitudes_db.iter().all(|d| *d == f64::NEG_INFINITY));
    }

    #[test]
    fn empty_input_is_silent_not_an_error() {
        let report = analyze(&[], 2, 48_000.0).unwrap();
        assert_eq!(report.rms_dbfs, f64::NEG_INFINITY);
        assert_eq!(report.loudness.integrated_lufs, f64::NEG_INFINITY);
        assert_eq!(report.spectrum.magnitudes_db.len(), SPECTRUM_BINS);
    }

    #[test]
    fn full_scale_sine_reads_zero_dbfs_peak_and_zero_lufs() {
        let samples = sine(1_000.0, 1.0, 1.0, 48_000.0);
        let report = analyze(&samples, 2, 48_000.0).unwrap();
        // 采样峰值应贴近 0 dBFS（离散相位不一定正好命中波峰，给点余量）。
        assert!(report.loudness.sample_peak_dbfs > -0.2, "peak={}", report.loudness.sample_peak_dbfs);
        // 真峰值 ≥ 采样峰值。
        assert!(report.loudness.true_peak_dbtp >= report.loudness.sample_peak_dbfs - 1e-9);
        // 满幅正弦的 RMS ≈ -3.01 dBFS。
        assert!((report.rms_dbfs + 3.01).abs() < 0.1, "rms={}", report.rms_dbfs);
        // 标定：满幅 1 kHz 正弦（RMS -3.01 dBFS）在 BS.1770 下约 0 LUFS——K 加权在 1 kHz
        // 附近约 +3 dB，正好抵消正弦的波峰因数。
        assert!(report.loudness.integrated_lufs.abs() < 1.0, "lufs={}", report.loudness.integrated_lufs);
    }

    #[test]
    fn spectrum_peaks_at_the_sine_bin() {
        // 取正好落在 bin 上的频率，避免窗漏（scalloping）压低峰值。
        let freq = 43.0 * 48_000.0 / FFT_SIZE as f64;
        let samples = sine(freq, 1.0, 0.5, 48_000.0);
        let report = analyze(&samples, 2, 48_000.0).unwrap();
        let (peak_bin, peak_db) = report
            .spectrum
            .magnitudes_db
            .iter()
            .enumerate()
            .fold((0usize, f64::NEG_INFINITY), |(bi, bd), (i, d)| if *d > bd { (i, *d) } else { (bi, bd) });
        let peak_hz = peak_bin as f64 * report.spectrum.bin_hz;
        assert!((peak_hz - freq).abs() < report.spectrum.bin_hz, "peak_hz={peak_hz} freq={freq}");
        // 单边归一化后满幅正弦在其 bin 上约 0 dB。
        assert!(peak_db.abs() < 0.3, "peak_db={peak_db}");
    }

    #[test]
    fn louder_signal_has_higher_lufs() {
        let quiet = analyze(&sine(440.0, 0.1, 1.0, 48_000.0), 2, 48_000.0).unwrap();
        let loud = analyze(&sine(440.0, 1.0, 1.0, 48_000.0), 2, 48_000.0).unwrap();
        assert!(loud.loudness.integrated_lufs > quiet.loudness.integrated_lufs + 15.0);
    }

    #[test]
    fn timbre_of_a_sine_is_tonal_and_centered_at_the_fundamental() {
        let freq = 43.0 * 48_000.0 / FFT_SIZE as f64;
        let samples = sine(freq, 1.0, 0.5, 48_000.0);
        let t = timbre(&samples, 2, 48_000.0).unwrap();
        assert!((t.spectral_centroid_hz - freq).abs() < 10.0, "centroid={}", t.spectral_centroid_hz);
        assert!(t.spectral_flatness < 0.2, "纯正弦不该像噪声：{}", t.spectral_flatness);
        assert!(t.bands.mid_db > -3.0, "mid={}", t.bands.mid_db);
        assert!(t.bands.air_db < -40.0, "air={}", t.bands.air_db);
    }

    #[test]
    fn timbre_of_noise_is_flat_and_high() {
        // 确定性伪随机噪声（xorshift）：谱平坦度必须显著高于正弦。
        let mut state = 0x1234_5678u32;
        let mut samples = Vec::with_capacity(48_000 * 2);
        for _ in 0..48_000 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let s = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            samples.push(s);
            samples.push(s);
        }
        let t = timbre(&samples, 2, 48_000.0).unwrap();
        assert!(t.spectral_flatness > 0.5, "噪声应当平坦：{}", t.spectral_flatness);
        assert!(t.spectral_centroid_hz > 5_000.0, "噪声重心应当高：{}", t.spectral_centroid_hz);
        assert!(t.bands.air_db > -30.0, "air={}", t.bands.air_db);
    }

    #[test]
    fn ragged_input_is_rejected() {
        assert!(matches!(
            analyze(&[0.0f32; 5], 2, 48_000.0),
            Err(AnalysisError::BadFormat(_))
        ));
        assert!(matches!(
            analyze(&[0.0f32; 4], 0, 48_000.0),
            Err(AnalysisError::BadFormat(_))
        ));
        assert!(matches!(
            analyze(&[0.0f32; 4], 2, 0.0),
            Err(AnalysisError::BadFormat(_))
        ));
    }
}
