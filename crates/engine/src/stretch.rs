//! 音频片段的**时值伸缩**（保音高）与**升降调**读取器。
//!
//! 读模型：先对素材做 **WSOLA 时值伸缩** `s = p / rate`（保音高），再按 `q = p · r`
//! **重采样**读取。其中 `p = 2^(半音/12)`、`r = 素材采样率 / 工程采样率`（`source_ratio`）。于是：
//! - `pitch = 0` → `s = 1/rate`：只伸缩 → **变速不变调**；
//! - `rate = 1` → `s = p`：只重采样 → **变调不变速**。
//!
//! **`r` 落在「重采样」那一侧，不进伸缩**：WSOLA 保的是**帧周期**（1 kHz @44.1k 的周期是
//! 44.1 **帧**），而采样率换算要的恰恰是「按输出率重新采样」——读指针每输出帧前进 `p · r`
//! 个 FIFO 帧。这样音高只由 `p` 决定、每输出帧在源上的净推进量是 `rate · r`，时长与音高两头
//! 都对。把 `r` 折进 `s` 就错了：那会把「44.1k 的帧周期」原样留在 48k 的输出里 → 升调 8.8%。
//!
//! 实时安全：状态在**构造时一次性分配**，`next_frame` 只做乘加与 `copy_within`，不分配。

use std::sync::Arc;

use crate::audio::DecodedAudio;

/// WSOLA 窗长（源帧，≈23 ms @44.1k）。
const WINDOW: usize = 1024;
/// 合成步进（50% 重叠；Hann 在 50% 重叠下满足 COLA）。
const HOP: usize = WINDOW / 2;
/// 重叠长度（相关性对齐的模板长度）。
const OVERLAP: usize = WINDOW - HOP;
/// 对齐搜索半径（±源帧）。
///
/// **必须覆盖最低音周期的半周**：WSOLA 靠这个范围内的互相关把「下一段」的相位对齐到上一段的
/// 重叠处；候选段与模板相差一个周期（或其整数倍）都能对齐，但可达到的最大相位差就是 SEARCH。
/// 取 1024 → 到 48 kHz 下约 23 Hz（低于钢琴最低音 A0 = 27.5 Hz），低音 / 底鼓 / 贝斯都能对齐。
/// 早先取 96（只够 ~250 Hz 以上）时，低频段对不齐 → OLA 反相抵消 → 声音被「扭曲」（沙哑 / 空洞）。
const SEARCH: i64 = 1024;
/// 粗搜的候选步长（源帧）。
///
/// **步长必须远小于相关峰宽度**，否则粗搜会整个跳过真正的峰（实测步长 16 时鼓点会被对齐到
/// 附近的**另一个**瞬态上 → 鼓点被复制/吞掉，起音量明显变多）。取 4（@48k ≈ 0.08 ms）后，
/// 任何真实的相关峰都不可能被跳过——峰宽至少要比 4 大得多才有意义。
const COARSE: i64 = 4;
/// 粗搜时相关函数的抽点间隔（源帧）。
///
/// 抽点只让相关峰**变宽**（对「先粗后精」反而是好事），不会移动峰的位置；细搜会在粗搜最优点
/// ±COARSE 内用全抽点重新收敛到真正的峰。
const DECIM: usize = 4;
/// 拉伸后 FIFO 的帧容量（够容纳一个合成块 + 读取余量）。
const FIFO_FRAMES: usize = 8 * HOP;

/// 一条片段的可变速率读指针。
pub struct Stretcher {
    audio: Arc<DecodedAudio>,
    channels: usize,
    /// 可读源窗口 `[lo, hi)`（源帧）。
    lo: u64,
    hi: u64,
    /// 输出每帧在「拉伸后」的推进量（= p · r；`r` 是采样率换算）。
    pitch: f64,
    /// 输出每帧在**源**上的名义推进量（= rate），仅用于 seek 定位。
    rate: f64,
    /// 素材采样率 / 工程采样率（采样率换算；并进 `pitch`，见模块文档）。
    source_ratio: f64,
    /// 每个合成块推进的源帧数（= round(HOP / s)）。
    hop_in: i64,
    /// 下一个合成块的序号。
    block: i64,
    /// 上一块的**源段**（未加窗，交错；模板取其后半段）。
    prev: Vec<f32>,
    /// OLA 累加器（交错）。
    ola: Vec<f32>,
    /// 拉伸后的样本 FIFO（交错；`fifo_frames` 为有效帧数）。
    fifo: Vec<f32>,
    fifo_frames: usize,
    /// FIFO 读位置（帧，带小数）。
    read: f64,
    /// 源是否已耗尽。
    src_done: bool,
}

impl Stretcher {
    /// 为一条片段建读指针（`window_frames` = 源窗口长度，`source_ratio` = 素材率/工程率）。
    pub fn new(
        audio: Arc<DecodedAudio>,
        offset_frames: u64,
        window_frames: u64,
        rate: f64,
        pitch_semitones: f64,
        source_ratio: f64,
    ) -> Self {
        let channels = audio.channels.max(1);
        let ratio = if source_ratio.is_finite() && source_ratio > 0.0 { source_ratio } else { 1.0 };
        let transpose = 2f64.powf(pitch_semitones / 12.0);
        // 读步进 = 升降调 × 采样率换算（两者都是「重采样」）；伸缩只由 `rate` 决定。
        let pitch = transpose * ratio;
        let stretch = transpose / rate; // 输出长 / 输入长
        let hop_in = ((HOP as f64) / stretch).round().max(1.0) as i64;
        Self {
            audio,
            channels,
            lo: offset_frames,
            hi: offset_frames + window_frames,
            pitch,
            rate,
            source_ratio: ratio,
            hop_in,
            block: 0,
            prev: vec![0.0; WINDOW * channels],
            ola: vec![0.0; WINDOW * channels],
            fifo: vec![0.0; FIFO_FRAMES * channels],
            fifo_frames: 0,
            read: 0.0,
            src_done: false,
        }
    }

    /// 重定位到片段的第 `out_frame` 个输出帧。
    pub fn seek(&mut self, out_frame: u64) {
        self.prev.iter_mut().for_each(|v| *v = 0.0);
        self.ola.iter_mut().for_each(|v| *v = 0.0);
        self.fifo_frames = 0;
        self.read = 0.0;
        self.src_done = false;
        // 近似源位置（WSOLA 的净位移在这些帧上是 rate·r 比）；对齐历史从该块重建。
        let src = out_frame as f64 * self.rate * self.source_ratio;
        self.block = (src / self.hop_in as f64).round().max(0.0) as i64;
    }

    /// 取下一个输出帧（交错立体声）。`None` = 素材已放完。
    pub fn next_frame(&mut self) -> Option<(f32, f32)> {
        loop {
            if (self.read as usize) + 2 <= self.fifo_frames {
                break;
            }
            if self.src_done {
                return None;
            }
            self.produce_block();
        }
        let ch = self.channels;
        let i0 = self.read.floor() as usize;
        let t = (self.read - i0 as f64) as f32;
        let (l0, r0) = self.fifo_at(i0);
        let (l1, r1) = self.fifo_at(i0 + 1);
        let out = (l0 + (l1 - l0) * t, r0 + (r1 - r0) * t);
        self.read += self.pitch;
        // FIFO 压缩：读过头一个 HOP 就把已消费的部分丢掉（保容量有界）。
        if self.read >= HOP as f64 {
            let drop = HOP.min(self.fifo_frames);
            let tail = self.fifo_frames - drop;
            if tail > 0 {
                self.fifo.copy_within(drop * ch..self.fifo_frames * ch, 0);
            }
            self.fifo_frames = tail;
            self.read -= drop as f64;
        }
        let _ = ch;
        Some(out)
    }

    #[inline]
    fn fifo_at(&self, frame: usize) -> (f32, f32) {
        let ch = self.channels;
        let at = frame * ch;
        let l = self.fifo.get(at).copied().unwrap_or(0.0);
        let r = if ch > 1 { self.fifo.get(at + 1).copied().unwrap_or(0.0) } else { l };
        (l, r)
    }

    /// 合成一个块：对齐 → 加窗 → OLA → 前 `HOP` 帧定稿进 FIFO。
    fn produce_block(&mut self) {
        let ch = self.channels;
        let span = (self.hi - self.lo) as i64;
        let a = self.block * self.hop_in;
        let has_template = self.block > 0;
        self.block += 1;
        if a >= span {
            self.src_done = true;
            return;
        }
        // 对齐搜索：使 `[a+d, a+d+OVERLAP)` 最接近上一块源段的后半段。
        //
        // `prev` 全零时**不做搜索**：那是 `seek` 之后第一块的状态（`block > 0` 但还没有上一块）。
        // 零模板的能量为 0，归一化互相关的分母只剩 1e-9，任何候选的得分都退化成 0/1e-9 = 0——
        // 于是「第一个候选胜出」把 s0 硬拽到 `a − SEARCH`（≈21 ms 的错位、听得见的「咔」）。
        // 静音模板下最优解本来就是**名义位置** `d = 0`，直接跳过搜索即可。
        let mut best_d = 0i64;
        if has_template && self.template_energy() > 1e-12 {
            best_d = self.best_offset(a, span);
        }
        let s0 = (a + best_d).clamp(0, (span - WINDOW as i64).max(0));
        // 取段（越过结尾补零）、存模板、叠加进 OLA。
        for i in 0..WINDOW {
            let off = s0 + i as i64;
            for c in 0..ch {
                let v = if off < span {
                    self.audio.samples[(self.lo as i64 + off) as usize * ch + c]
                } else {
                    0.0
                };
                self.prev[i * ch + c] = v;
                self.ola[i * ch + c] += v * hann(i, WINDOW);
            }
        }
        // 前 HOP 帧定稿。
        let room = self.fifo.len() / ch - self.fifo_frames;
        let take = HOP.min(room);
        for f in 0..take {
            for c in 0..ch {
                self.fifo[(self.fifo_frames + f) * ch + c] = self.ola[f * ch + c];
            }
        }
        self.fifo_frames += take;
        if take < HOP {
            // FIFO 满了（理论上不该发生）：本块剩余丢弃，避免越界。
            self.src_done = true;
            return;
        }
        // OLA 左移 HOP。
        let keep = (WINDOW - HOP) * ch;
        self.ola.copy_within(HOP * ch..WINDOW * ch, 0);
        for v in self.ola[keep..].iter_mut() {
            *v = 0.0;
        }
    }

    /// 对齐偏移搜索：返回使候选段与模板最相似的 `d ∈ [-SEARCH, SEARCH]`（越界候选跳过）。
    ///
    /// **先粗后精**（实时性的关键）：一块只产出 `HOP` 个输出帧，而逐帧全搜要 2049 次互相关 ×
    /// 512 抽头 ≈ 105 万次乘加**每块**——Debug 构建下实测 42–170 µs/输出帧，而实时预算只有
    /// 20.8 µs → 音频回调必然爆音、卡死。粗搜（`COARSE` 步长 + `DECIM` 抽点）把候选压到 ~510 个、
    /// 每个 128 抽头，再在粗搜最优点 ±`COARSE` 内用全抽点精搜（9 个候选）→ 快 ~15 倍。
    ///
    /// 粗搜步长取 4（远小于相关峰宽度）保证**不会跳过真正的峰**：曾用 16 时鼓点会被对到相邻的
    /// 另一个瞬态上，起音点数量明显变多（素材被复制/吞掉）。
    fn best_offset(&self, a: i64, span: i64) -> i64 {
        let mut coarse_best = f64::MIN;
        let mut coarse_d = 0i64;
        let mut d = -SEARCH;
        while d <= SEARCH {
            let s0 = a + d;
            if s0 >= 0 && s0 + WINDOW as i64 <= span {
                let score = self.correlation(s0, DECIM);
                if score > coarse_best {
                    coarse_best = score;
                    coarse_d = d;
                }
            }
            d += COARSE;
        }
        // 精搜：粗搜最优点 ±COARSE 逐帧、全抽点。抽点与全抽点的分数不同量纲，故重新起最优值。
        let lo = (coarse_d - COARSE).max(-SEARCH);
        let hi = (coarse_d + COARSE).min(SEARCH);
        let mut best = f64::MIN;
        let mut best_d = 0i64;
        let mut d = lo;
        while d <= hi {
            let s0 = a + d;
            if s0 >= 0 && s0 + WINDOW as i64 <= span {
                let score = self.correlation(s0, 1);
                if score > best {
                    best = score;
                    best_d = d;
                }
            }
            d += 1;
        }
        best_d
    }

    /// 模板（上一块源段的后半段，左声道）的能量：为 0 表示没有可用的对齐参照。
    fn template_energy(&self) -> f64 {
        let ch = self.channels;
        let mut e = 0.0f64;
        for i in 0..OVERLAP {
            let v = self.prev[(HOP + i) * ch] as f64;
            e += v * v;
        }
        e
    }

    /// 归一化互相关：源 `[s0, s0+OVERLAP)`（左声道）与模板 `prev[HOP..WINDOW)`。
    ///
    /// `decim` = 抽点间隔（1 = 全抽点）。抽点只用于粗搜（省时间），精搜一律全抽点。
    fn correlation(&self, s0: i64, decim: usize) -> f64 {
        let ch = self.channels;
        let base = (self.lo as i64 + s0) as usize;
        let decim = decim.max(1);
        let mut dot = 0.0f64;
        let mut e1 = 0.0f64;
        let mut e2 = 0.0f64;
        let mut i = 0usize;
        while i < OVERLAP {
            let x = self.audio.samples[(base + i) * ch] as f64;
            let y = self.prev[(HOP + i) * ch] as f64;
            dot += x * y;
            e1 += x * x;
            e2 += y * y;
            i += decim;
        }
        dot / (e1.sqrt() * e2.sqrt() + 1e-9)
    }
}

/// 周期型 Hann 窗。
#[inline]
fn hann(i: usize, n: usize) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_mono(freq: f64, secs: f64, sr: f64) -> Arc<DecodedAudio> {
        let n = (secs * sr) as usize;
        let samples: Vec<f32> = (0..n)
            .map(|i| (0.7 * (std::f64::consts::TAU * freq * i as f64 / sr).sin()) as f32)
            .collect();
        Arc::new(DecodedAudio { samples: samples.into(), channels: 1, sample_rate: sr })
    }

    /// 峰值频率（用 crates/analysis 的频谱）。
    fn peak_hz(samples: &[f32], sr: f64) -> f64 {
        let report = ai_daw_analysis::analyze(samples, 1, sr).unwrap();
        let (bin, _) = report
            .spectrum
            .magnitudes_db
            .iter()
            .enumerate()
            .fold((0usize, f64::NEG_INFINITY), |(bi, bd), (i, d)| {
                if *d > bd { (i, *d) } else { (bi, bd) }
            });
        bin as f64 * report.spectrum.bin_hz
    }

    /// 峰值频率，但**只在 `[low, high]` 频带内**找：低频段的周期太长，用粗粒度的分析谱会
    /// 在倍频 / 次频上误判；这里用细网格 DFT 直接量该频带内的最强分量。
    fn peak_hz_in(samples: &[f32], sr: f64, low: f64, high: f64) -> f64 {
        let n = 800;
        let mut best = (0.0f64, f64::MIN);
        for k in 0..=n {
            let f = low + (high - low) * k as f64 / n as f64;
            let mut re = 0.0f64;
            let mut im = 0.0f64;
            for (i, v) in samples.iter().enumerate() {
                let ph = std::f64::consts::TAU * f * i as f64 / sr;
                re += *v as f64 * ph.cos();
                im += *v as f64 * ph.sin();
            }
            let m = (re * re + im * im).sqrt();
            if m > best.1 {
                best = (f, m);
            }
        }
        best.0
    }

    fn read_all(st: &mut Stretcher, max: usize) -> Vec<f32> {
        let mut out = Vec::new();
        while out.len() < max {
            match st.next_frame() {
                Some((l, _)) => out.push(l),
                None => break,
            }
        }
        out
    }

    #[test]
    fn unity_rate_and_pitch_is_near_identity() {
        let sr = 44_100.0;
        let audio = sine_mono(1_000.0, 0.5, sr);
        let mut st = Stretcher::new(audio, 0, 22_050, 1.0, 0.0, 1.0);
        let out = read_all(&mut st, 22_050 + 100);
        // 时长不变、音高不变。
        assert!((out.len() as i64 - 22_050).abs() < 1_500, "长度应≈原长：{}", out.len());
        assert!((peak_hz(&out, sr) - 1_000.0).abs() < 40.0, "音高应≈1kHz：{}", peak_hz(&out, sr));
    }

    #[test]
    fn rate_two_halves_duration_and_keeps_pitch() {
        let sr = 44_100.0;
        let audio = sine_mono(1_000.0, 1.0, sr);
        let mut st = Stretcher::new(audio, 0, 44_100, 2.0, 0.0, 1.0);
        let out = read_all(&mut st, 60_000);
        assert!((out.len() as i64 - 22_050).abs() < 2_500, "速度×2 → 长度≈一半：{}", out.len());
        assert!((peak_hz(&out, sr) - 1_000.0).abs() < 40.0, "变速不变调：{}", peak_hz(&out, sr));
    }

    #[test]
    fn pitch_up_keeps_duration_and_doubles_frequency() {
        let sr = 44_100.0;
        let audio = sine_mono(1_000.0, 0.5, sr);
        let mut st = Stretcher::new(audio, 0, 22_050, 1.0, 12.0, 1.0);
        let out = read_all(&mut st, 60_000);
        assert!((out.len() as i64 - 22_050).abs() < 1_500, "变调不变速：{}", out.len());
        assert!((peak_hz(&out, sr) - 2_000.0).abs() < 60.0, "+12 半音 → 2kHz：{}", peak_hz(&out, sr));
    }

    /// 低频也要能正确升降调（回归：对齐搜索半径太窄时，低频段对不齐 → 输出被扭曲、音高乱掉）。
    #[test]
    fn low_frequency_pitch_shift_is_correct() {
        let sr = 44_100.0;
        let f0 = 82.41; // 低音 E（贝斯最低空弦）
        let audio = sine_mono(f0, 1.0, sr);
        for (semis, exp) in [(12.0, 164.83), (-5.0, 61.74)] {
            let mut st = Stretcher::new(audio.clone(), 0, 44_100, 1.0, semis, 1.0);
            let out = read_all(&mut st, 60_000);
            // 期望频率 ±15% 的频带内找峰：够宽以容纳 ±12 与 -5 两个目标，又排除了倍频 / 次频。
            let peak = peak_hz_in(&out, sr, exp * 0.85, exp * 1.15);
            assert!(
                (peak - exp).abs() < exp * 0.03,
                "低音 {f0}Hz {semis:+.0} 半音应 ≈ {exp:.1}Hz，实际 {peak:.1}Hz",
            );
        }
    }

    /// 素材采样率 ≠ 工程采样率：读指针按 `source_ratio` 重采样 → **时长与音高都不变**。
    ///
    /// 回归：早先每输出帧只前进 `rate`（漏掉采样率换算），44.1 kHz 素材在 48 kHz 工程里会快
    /// 8.8% 并升调。这里用 `rate = 2` 走 Stretcher 路径（`rate = 1` 由播放器直读处理）。
    #[test]
    fn sample_rate_conversion_keeps_duration_and_pitch() {
        let src_sr = 44_100.0;
        let out_sr = 48_000.0;
        let audio = sine_mono(1_000.0, 1.0, src_sr); // 44100 源帧 = 1.0 s
        let mut st = Stretcher::new(audio, 0, 44_100, 2.0, 0.0, src_sr / out_sr);
        let out = read_all(&mut st, 60_000);
        // 1 s 素材、速度 ×2 → 0.5 s；按 48 kHz 计 = 24000 个输出帧。
        assert!((out.len() as i64 - 24_000).abs() < 2_500, "长度应≈24000：{}", out.len());
        let pk = peak_hz(&out, out_sr);
        assert!((pk - 1_000.0).abs() < 40.0, "变速 + 采样率换算都不该改音高：{pk}");
    }

    /// `seek` 之后**第一块**的取段位置必须落在名义位置上。
    ///
    /// 回归：`seek` 把 `prev` 清零，但 `block > 0` 仍让 `has_template` 为真——那时模板能量为 0，
    /// 归一化互相关的分母只剩 1e-9，所有候选得分退化成 0，于是「第一个候选胜出」把取段硬拽到
    /// `a − SEARCH`（≈21 ms 错位）。定位、循环回卷、每次结构性重建（重建后会 seek 到当前位置）
    /// 都会踩一脚，听感是「咔」的一声。
    #[test]
    fn seek_lands_on_the_nominal_position_not_a_search_radius_offset() {
        // 阶梯素材：第 f 帧的值 = f / total → 输出值直接反推取到的源帧。
        let total = 480_000usize;
        let ramp: Vec<f32> = (0..total).map(|f| f as f32 / total as f32).collect();
        let audio = Arc::new(DecodedAudio { samples: ramp.into(), channels: 1, sample_rate: 48_000.0 });

        let rate = 2.0;
        let at = 48_000u64; // 输出第 48000 帧
        let mut st = Stretcher::new(audio, 0, total as u64, rate, 0.0, 1.0);
        st.seek(at);
        let out = read_all(&mut st, 2_000);

        // 名义分析位置（与 `Stretcher::seek` 同一套算法算出）：`block = round(src / hop_in)`。
        let hop_in = ((HOP as f64) / (1.0 / rate)).round() as i64; // HOP / stretch
        let src = (at as f64 * rate).round();
        let a = (src / hop_in as f64).round() as i64 * hop_in;
        // 第一块没有重叠（`ola` 清零后只叠了本块）：`fifo[j] = x[s0 + j] · hann(j)`，除掉窗就是源帧。
        let j = HOP - 1;
        let read_src = (out[j] as f64 * total as f64 / hann(j, WINDOW) as f64).round() as i64 - j as i64;
        assert!(
            (read_src - a).abs() <= 8,
            "seek 后第一块应取源 ≈{a}（名义位置），实际 {read_src}（差 {}，旧实现会偏 −{SEARCH}）",
            read_src - a,
        );
    }
}
