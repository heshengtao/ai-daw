#![forbid(unsafe_code)]

//! **母带处理**（纯 DSP、离线）：**响度归一 + 真峰值限幅 + 立体声宽度**。
//!
//! 刻意**不依赖任何模型**——母带是交付前的最后一步，用确定性的 DSP 比一个几十 MB 的权重更
//! 可靠、零下载、零许可风险。要「参考匹配」那种风格迁移（Matchering 一路）另说，那是 GPL、
//! 只能走 sidecar，不在本 crate 范围里。
//!
//! 处理顺序（顺序不能换）：
//! 1. **立体声宽度**（M/S）——放大侧信号会抬高峰值，必须排在增益与限幅之前；
//! 2. **增益到目标响度**——量一次积分响度（LUFS），把整体推过去；
//! 3. **真峰值限幅**——把真峰值压到 `ceiling_dbtp` 以下（见 [`limiter`]）；
//! 4. **复测**——回报前后两组数字。
//!
//! 输入输出都是**交错 `f32`**（与 `AudioBuffer::data()` / `DecodedAudio.samples` 同一布局），
//! 支持 1 或 2 声道。整个函数是纯函数、不碰 IO，方便单测。

mod limiter;

use std::fmt;
use std::ops::RangeInclusive;

use ai_daw_analysis::analyze;

/// 默认目标响度（LUFS）——流媒体平台最常用的一档。
pub const DEFAULT_TARGET_LUFS: f64 = -14.0;
/// 默认真峰值上限（dBTP）。留 1 dB 余量是行业惯例（编解码会有采样间峰值）。
pub const DEFAULT_CEILING_DBTP: f64 = -1.0;
/// 默认立体声宽度（1.0 = 不动）。
pub const DEFAULT_WIDTH: f32 = 1.0;

/// 目标响度的可接受区间（超出说明输入有问题，直接夹住而不是报错）。
const TARGET_LUFS_RANGE: RangeInclusive<f64> = -40.0..=0.0;
/// 真峰值上限的可接受区间。
const CEILING_DBTP_RANGE: RangeInclusive<f64> = -12.0..=0.0;
/// 立体声宽度的可接受区间（0 = 纯单声道，2 = 侧信号加倍）。
const WIDTH_RANGE: RangeInclusive<f64> = 0.0..=2.0;

/// 归一增益的上下限（防止极端输入把音量推到荒谬的位置）。
const MAX_GAIN_DB: f64 = 30.0;

/// 母带参数。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MasterOptions {
    /// 目标积分响度（LUFS，负数）。
    pub target_lufs: f64,
    /// 真峰值上限（dBTP，负数或 0）。
    pub ceiling_dbtp: f64,
    /// 立体声宽度：0 = 单声道，1 = 原样，>1 加宽。
    pub stereo_width: f32,
}

impl Default for MasterOptions {
    fn default() -> Self {
        Self {
            target_lufs: DEFAULT_TARGET_LUFS,
            ceiling_dbtp: DEFAULT_CEILING_DBTP,
            stereo_width: DEFAULT_WIDTH,
        }
    }
}

impl MasterOptions {
    /// 把参数夹到合理区间（界面上的输入不设防，统一在这里兜底）。
    pub fn sanitized(&self) -> Self {
        Self {
            target_lufs: clamp_finite(self.target_lufs, &TARGET_LUFS_RANGE, DEFAULT_TARGET_LUFS),
            ceiling_dbtp: clamp_finite(
                self.ceiling_dbtp,
                &CEILING_DBTP_RANGE,
                DEFAULT_CEILING_DBTP,
            ),
            stereo_width: clamp_finite(self.stereo_width as f64, &WIDTH_RANGE, DEFAULT_WIDTH as f64)
                as f32,
        }
    }
}

fn clamp_finite(v: f64, range: &RangeInclusive<f64>, fallback: f64) -> f64 {
    if !v.is_finite() {
        return fallback;
    }
    v.clamp(*range.start(), *range.end())
}

/// 母带处理的前后观测（给界面显示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct MasterReport {
    /// 目标积分响度（LUFS）。
    pub target_lufs: f64,
    /// 处理前的积分响度（LUFS）——立体声宽度已经生效之后量的。
    pub input_lufs: f64,
    /// 处理后的积分响度（LUFS）。因为有限幅，通常会略低于目标。
    pub output_lufs: f64,
    /// 处理前的真峰值（dBTP）。
    pub input_true_peak_dbtp: f64,
    /// 处理后的真峰值（dBTP）。
    pub output_true_peak_dbtp: f64,
    /// 施加的归一增益（dB）。
    pub gain_db: f64,
    /// 限幅器实际施加的最大衰减（dB，0 = 没动）。
    pub reduced_db: f64,
    /// 生效的立体声宽度。
    pub stereo_width: f32,
}

/// 母带处理的错误。
#[derive(Debug)]
pub enum MasterError {
    /// 输入布局不对（声道数、长度、采样率）。
    BadFormat(String),
    /// 响度观测失败。
    Analysis(String),
}

impl fmt::Display for MasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MasterError::BadFormat(m) => write!(f, "母带输入不合法：{m}"),
            MasterError::Analysis(m) => write!(f, "母带响度观测失败：{m}"),
        }
    }
}

impl std::error::Error for MasterError {}

/// 对整段立体声（或单声道）混音做母带处理，返回处理后的信号与观测报告。
pub fn master(
    input: &[f32],
    channels: usize,
    sample_rate: f64,
    options: &MasterOptions,
) -> Result<(Vec<f32>, MasterReport), MasterError> {
    if channels == 0 || channels > 2 {
        return Err(MasterError::BadFormat(format!(
            "声道数 {channels} 不支持（1 或 2）"
        )));
    }
    if input.is_empty() || !input.len().is_multiple_of(channels) {
        return Err(MasterError::BadFormat(format!(
            "样本数 {} 不是声道数 {channels} 的整数倍",
            input.len()
        )));
    }
    if !sample_rate.is_finite() || sample_rate <= 0.0 {
        return Err(MasterError::BadFormat(format!(
            "采样率 {sample_rate} 不合法"
        )));
    }

    let opts = options.sanitized();
    let mut buffer = input.to_vec();

    // 1. 立体声宽度：放在增益与限幅之前，否则加宽出来的峰值会被漏掉。
    if channels == 2 {
        apply_width(&mut buffer, opts.stereo_width);
    }

    // 2. 量一次，把整体推到目标响度。静音时积分响度是 -inf，此时不增益。
    let before = analyze(&buffer, channels, sample_rate)
        .map_err(|e| MasterError::Analysis(e.to_string()))?;
    let gain_db = if before.loudness.integrated_lufs.is_finite() {
        (opts.target_lufs - before.loudness.integrated_lufs).clamp(-MAX_GAIN_DB, MAX_GAIN_DB)
    } else {
        0.0
    };
    if gain_db != 0.0 {
        let gain = 10f64.powf(gain_db / 20.0) as f32;
        for sample in buffer.iter_mut() {
            *sample *= gain;
        }
    }

    // 3. 真峰值限幅。
    let ceiling = 10f64.powf(opts.ceiling_dbtp / 20.0) as f32;
    let limited = limiter::limit_true_peak(&mut buffer, channels, sample_rate, ceiling);

    // 4. 复测。
    let after = analyze(&buffer, channels, sample_rate)
        .map_err(|e| MasterError::Analysis(e.to_string()))?;

    let report = MasterReport {
        target_lufs: opts.target_lufs,
        input_lufs: before.loudness.integrated_lufs,
        output_lufs: after.loudness.integrated_lufs,
        input_true_peak_dbtp: before.loudness.true_peak_dbtp,
        output_true_peak_dbtp: after.loudness.true_peak_dbtp,
        gain_db,
        reduced_db: limited.reduction_db,
        stereo_width: opts.stereo_width,
    };
    Ok((buffer, report))
}

/// 中/侧（M/S）立体声宽度。`width == 1.0` 时原样返回。
fn apply_width(buffer: &mut [f32], width: f32) {
    if (width - 1.0).abs() < 1e-6 {
        return;
    }
    for frame in buffer.chunks_exact_mut(2) {
        let mid = (frame[0] + frame[1]) * 0.5;
        let side = (frame[0] - frame[1]) * 0.5 * width;
        frame[0] = mid + side;
        frame[1] = mid - side;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// 双声道同相正弦（振幅 `amp`），`frames` 帧，1 kHz。
    fn sine(frames: usize, amp: f32, sample_rate: f64) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        for n in 0..frames {
            let v = (2.0 * PI * 1000.0 * n as f64 / sample_rate).sin() as f32 * amp;
            out.push(v);
            out.push(v);
        }
        out
    }

    /// 归一增益把整体推到目标响度附近（此例峰值远低于 ceiling，限幅不介入）。
    #[test]
    fn normalizes_toward_the_target_loudness() {
        let sr = 48_000.0;
        let input = sine(48_000, 0.1, sr);
        let (_, report) = master(&input, 2, sr, &MasterOptions::default()).expect("母带应当成功");
        assert!(
            report.gain_db > 0.0,
            "安静素材应当被推大：{} dB",
            report.gain_db
        );
        assert!(
            (report.output_lufs - DEFAULT_TARGET_LUFS).abs() < 0.5,
            "输出 {} LUFS 偏离目标 {} LUFS 太多",
            report.output_lufs,
            DEFAULT_TARGET_LUFS
        );
        assert_eq!(report.reduced_db, 0.0, "不该触发限幅");
    }

    /// 满幅素材 + 目标 0 LUFS → 限幅介入，真峰值守在 ceiling 以下。
    #[test]
    fn limiter_holds_the_ceiling_on_loud_material() {
        let sr = 48_000.0;
        let input = sine(48_000, 1.0, sr);
        let opts = MasterOptions {
            target_lufs: 0.0,
            ceiling_dbtp: -1.0,
            stereo_width: 1.0,
        };
        let (_, report) = master(&input, 2, sr, &opts).expect("母带应当成功");
        assert!(
            report.reduced_db > 0.5,
            "应当真的压了：{} dB",
            report.reduced_db
        );
        assert!(
            report.output_true_peak_dbtp <= opts.ceiling_dbtp + 0.2,
            "输出真峰值 {} dBTP 超过上限 {} dBTP",
            report.output_true_peak_dbtp,
            opts.ceiling_dbtp
        );
    }

    /// 宽度 0 把纯侧信号（L = +1 / R = -1）收成单声道。
    #[test]
    fn width_zero_collapses_to_mono() {
        let mut buffer: Vec<f32> = (0..128).flat_map(|_| [1.0f32, -1.0]).collect();
        apply_width(&mut buffer, 0.0);
        assert!(buffer.iter().all(|s| s.abs() < 1e-6), "侧信号应当被收干净");
    }

    /// 宽度 1 是恒等变换。
    #[test]
    fn width_one_is_a_no_op() {
        let original: Vec<f32> = (0..64).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut buffer = original.clone();
        apply_width(&mut buffer, 1.0);
        assert_eq!(buffer, original);
    }

    /// 静音不能产生 NaN / Inf，也不该试图增益。
    #[test]
    fn silence_stays_silent() {
        let input = vec![0.0f32; 4800];
        let (out, report) =
            master(&input, 2, 48_000.0, &MasterOptions::default()).expect("应当成功");
        assert!(out.iter().all(|s| s.is_finite()), "输出出现了非有限值");
        assert_eq!(report.gain_db, 0.0);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    /// 单声道也能处理。
    #[test]
    fn handles_mono() {
        let sr = 48_000.0;
        let input: Vec<f32> = (0..48_000)
            .map(|n| (2.0 * PI * 1000.0 * n as f64 / sr).sin() as f32 * 0.2)
            .collect();
        let (out, report) = master(&input, 1, sr, &MasterOptions::default()).expect("应当成功");
        assert_eq!(out.len(), input.len());
        assert!((report.output_lufs - DEFAULT_TARGET_LUFS).abs() < 0.5);
    }

    /// 参数会被夹到合理区间，非有限值回落到默认。
    #[test]
    fn options_are_sanitized() {
        let wild = MasterOptions {
            target_lufs: f64::NAN,
            ceiling_dbtp: 99.0,
            stereo_width: 50.0,
        }
        .sanitized();
        assert_eq!(wild.target_lufs, DEFAULT_TARGET_LUFS);
        assert_eq!(wild.ceiling_dbtp, 0.0);
        assert_eq!(wild.stereo_width, 2.0);
    }

    /// 非法输入被拒绝（而不是悄悄跑出垃圾）。
    #[test]
    fn rejects_malformed_input() {
        let opts = MasterOptions::default();
        assert!(
            master(&[0.0; 3], 2, 48_000.0, &opts).is_err(),
            "长度不是声道数的整数倍"
        );
        assert!(
            master(&[0.0; 4], 3, 48_000.0, &opts).is_err(),
            "声道数不支持"
        );
        assert!(master(&[], 2, 48_000.0, &opts).is_err(), "空输入");
        assert!(master(&[0.0; 4], 2, 0.0, &opts).is_err(), "采样率为 0");
    }
}
