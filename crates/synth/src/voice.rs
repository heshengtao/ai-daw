use crate::Params;

/// 一个声部的包络阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// 振荡器波形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Square,
    Sine,
    Triangle,
}

/// PolyBLEP 修正项：`t` 为相位（0..1），`dt` 为每采样推进的相位量。
///
/// 朴素点采样的方波/锯齿在高音区会把超出奈奎斯特的谐波**折回来**，听起来就是「刺耳」。
/// 这里在跳变点前后用一个二次多项式把台阶补圆：实测 C5 的非谐波能量从 15.5% 降到 1% 量级。
fn poly_blep(t: f64, dt: f64) -> f64 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

impl Wave {
    /// 由参数值（0..=3）取波形；越界按锯齿处理。
    pub fn from_param(value: f32) -> Self {
        match value.round() as i32 {
            1 => Wave::Square,
            2 => Wave::Sine,
            3 => Wave::Triangle,
            _ => Wave::Saw,
        }
    }

    /// 样品点（`phase` 为 0..1 的归一化相位，`dt` 为相位增量）。
    ///
    /// 锯齿 / 方波（脉冲）带 PolyBLEP 抗混叠；正弦与三角本来就干净
    /// （三角的谐波按 1/n² 衰减，混叠低到不值得再上 polyBLAMP）。
    fn sample(self, phase: f64, pulse_width: f64, dt: f64) -> f32 {
        let p = phase - phase.floor();
        match self {
            Wave::Saw => (2.0 * p - 1.0 - poly_blep(p, dt)) as f32,
            Wave::Square => {
                let raw = if p < pulse_width { 1.0 } else { -1.0 };
                // 两个跳变：相位 0 处向上跳、`pulse_width` 处向下跳。
                let falling = (p + 1.0 - pulse_width) % 1.0;
                (raw + poly_blep(p, dt) - poly_blep(falling, dt)) as f32
            }
            Wave::Sine => (p * std::f64::consts::TAU).sin() as f32,
            Wave::Triangle => (4.0 * (p - 0.5).abs() - 1.0) as f32,
        }
    }
}

/// 状态变量滤波器的输出类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    LowPass,
    BandPass,
    HighPass,
}

impl FilterType {
    /// 由参数值（0..=2）取类型；越界按低通处理。
    pub fn from_param(value: f32) -> Self {
        match value.round() as i32 {
            1 => FilterType::BandPass,
            2 => FilterType::HighPass,
            _ => FilterType::LowPass,
        }
    }
}

/// 音高包络时长：0..1 → 5 ms .. 2 s（指数，短促区好调）。
pub fn pitch_decay_secs(t: f32) -> f64 {
    0.005 * 400f64.powf(t.clamp(0.0, 1.0) as f64)
}

/// **每块算一次**的派生常量：只跟参数有关、与采样无关的量都在这里算好，
/// 别在每采样路径上重复（改版后每采样要跑两个振荡器 + 调制器，能省的都要省）。
#[derive(Debug, Clone, Copy)]
pub struct Block {
    /// 状态变量滤波器的阻尼（只由共振决定）。
    pub k: f32,
    /// 主振荡器的脉宽（PWM）。
    pub pulse_width: f64,
    /// 载波相位调制深度（弧度）；0 = 关闭 FM。
    pub fm_index: f64,
    /// 调制器频率比。
    pub fm_ratio: f64,
    /// 音高包络的最大偏移（半音）；0 = 关闭。
    pub pitch_amount: f32,
    /// 音高包络每采样的步长。
    pub pitch_step: f32,
    /// 噪声混合比（0 = 纯振荡器，1 = 纯噪声）。
    pub noise: f32,
    /// 位深压碎的量化档数（2^位数）。
    pub crush_levels: f32,
    pub filter_type: FilterType,
}

impl Block {
    pub fn new(p: &Params, sample_rate: f64) -> Self {
        let pitch_step = if p.pitch_env > 0.0 {
            (1.0 / (pitch_decay_secs(p.pitch_decay) * sample_rate)) as f32
        } else {
            0.0
        };
        Block {
            k: 2.0 - 1.9 * p.resonance,
            pulse_width: (0.5 + (p.pwm - 0.5) * 0.9) as f64,
            // **相位调制深度，单位是周期**（弧度 ÷ 2π）：`fm = 1` 约 4 弧度，是有用且听得出
            // 音色变化的调制量。早先直接把弧度当成周期加在 0..1 的相位上，深度大了六倍多，
            // 出来的不是 FM 而是噪声（钟声预设实测 73% 的能量是混叠）。
            fm_index: p.fm as f64 * 4.0 / std::f64::consts::TAU,
            fm_ratio: p.fm_ratio as f64,
            // 最多 +2 个八度：底鼓、音效、拨弦的起振都靠它。
            pitch_amount: p.pitch_env * 24.0,
            pitch_step,
            noise: p.noise,
            // 位深压碎：`crush = 1` → 4 bit（16 档），与 NES 的 4 bit 音量同量级。
            // 它是**量化**而不是降采样，所以不会引入混叠。
            crush_levels: 2f32.powf(16.0 - p.crush * 12.0),
            filter_type: FilterType::from_param(p.filter_type),
        }
    }
}

/// 一个声部：DCO（主振 + 失谐副振 + 低八度副振 + 噪声 + FM）→ HPF → VCF → 压碎 → VCA。
///
/// 每个声部自带包络与滤波器状态，所以按键之间互不干扰；LFO 是**全局**的（老式模拟合成器
/// 只有一个 LFO），由 [`crate::SynthPlugin`] 推进后把当前值传进来。
#[derive(Debug, Clone, Copy)]
pub struct Voice {
    pub note: u8,
    velocity: f32,
    /// 单音音量（0.0..=1.0，1.0 = 不衰减）；由 `NoteVolume` 事件设定，与力度互不影响。
    volume: f32,
    /// 该声部的音高偏移（音分）；由通道弯音 / 逐音弯音事件设定。
    bend_cents: f32,
    /// 滑音的目标频率与当前频率（Hz）。
    target_freq: f64,
    freq: f64,
    phase: f64,
    sub_phase: f64,
    /// 失谐伴振的相位。
    det_phase: f64,
    /// FM 调制器的相位。
    fm_phase: f64,
    /// 白噪声的 xorshift 状态。
    rng: u32,
    /// 音高包络当前值（1 → 0）。
    pitch_env: f32,
    env: f32,
    pub stage: Stage,
    /// Cytomic 型状态变量滤波器的两个积分状态。
    ic1: f32,
    ic2: f32,
    /// 一阶高通的上一拍输入。
    hpf_prev_in: f32,
    hpf_prev_out: f32,
}

impl Voice {
    pub const fn idle() -> Self {
        Voice {
            note: 0,
            velocity: 0.0,
            volume: 1.0,
            bend_cents: 0.0,
            target_freq: 0.0,
            freq: 0.0,
            phase: 0.0,
            sub_phase: 0.0,
            det_phase: 0.0,
            fm_phase: 0.0,
            rng: 0x9E37_79B9,
            pitch_env: 0.0,
            env: 0.0,
            stage: Stage::Idle,
            ic1: 0.0,
            ic2: 0.0,
            hpf_prev_in: 0.0,
            hpf_prev_out: 0.0,
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8, glide: f32) {
        let from = if self.stage == Stage::Idle { None } else { Some(self.freq) };
        self.note = note;
        self.velocity = velocity as f32 / 127.0;
        // 新音先按「不衰减」起，随后的 `NoteVolume`（若有）再覆盖——复用声部因此不会残留旧音量。
        self.volume = 1.0;
        self.target_freq = note_freq(note);
        // 滑音：接着上一个音的音高滑过去；否则直接到位。
        self.freq = match from {
            Some(prev) if glide > 0.0 => prev,
            _ => self.target_freq,
        };
        self.phase = 0.0;
        self.sub_phase = 0.0;
        self.det_phase = 0.0;
        self.fm_phase = 0.0;
        self.pitch_env = 1.0;
        // 噪声按音高播种：否则 16 个声部的噪声完全相干，叠起来只会变得更响而不是更厚。
        let seed = 0x9E37_79B9 ^ (note as u32).wrapping_mul(2_654_435_761);
        self.rng = if seed == 0 { 1 } else { seed };
        self.env = 0.0;
        self.stage = Stage::Attack;
    }

    /// 设定该声部的单音音量（0..=127）。
    pub fn set_volume(&mut self, value: u8) {
        self.volume = value.min(127) as f32 / 127.0;
    }

    /// 设定该声部的音高偏移（音分）。
    pub fn set_bend(&mut self, cents: f32) {
        self.bend_cents = if cents.is_finite() {
            cents.clamp(-2400.0, 2400.0)
        } else {
            0.0
        };
    }

    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// 双极性白噪声（xorshift32）。声部自带状态，回调里不分配。
    fn white(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// 推进一个采样并返回输出。
    pub fn next_sample(&mut self, sample_rate: f64, p: &Params, b: &Block, lfo: f32) -> f32 {
        if self.stage == Stage::Idle {
            return 0.0;
        }
        let sr = sample_rate;
        let secs = |t: f32| -> f64 { (t.max(0.0) as f64) * sr };

        match self.stage {
            Stage::Attack => {
                let a = secs(p.attack);
                self.env += if a <= 1.0 { 1.0 } else { (1.0 / a) as f32 };
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                let d = secs(p.decay);
                let step = if d <= 1.0 { 1.0 } else { ((1.0 - p.sustain) / d as f32).max(0.0) };
                self.env -= step;
                if self.env <= p.sustain {
                    self.env = p.sustain;
                    self.stage = if p.sustain <= 0.0 { Stage::Idle } else { Stage::Sustain };
                }
            }
            Stage::Sustain => {}
            Stage::Release => {
                let r = secs(p.release);
                self.env -= if r <= 1.0 { 1.0 } else { (1.0 / r) as f32 };
                if self.env <= 0.0 {
                    self.env = 0.0;
                    self.stage = Stage::Idle;
                    return 0.0;
                }
            }
            Stage::Idle => {}
        }

        // ---- 音高：滑音 + LFO 颤音 + 音高包络（起音扫频） ----
        if p.glide > 0.0 {
            let step = 1.0 / secs(p.glide).max(1.0);
            let diff = self.target_freq - self.freq;
            if diff.abs() <= step {
                self.freq = self.target_freq;
            } else {
                self.freq += diff.signum() * step;
            }
        }
        let mut semis = p.lfo_pitch * lfo + self.bend_cents / 100.0;
        if b.pitch_step > 0.0 {
            semis += self.pitch_env * b.pitch_amount;
            self.pitch_env = (self.pitch_env - b.pitch_step).max(0.0);
        }
        let f = self.freq * 2f64.powf((semis / 12.0) as f64);
        let dt = f / sr;
        self.phase = (self.phase + dt).fract();
        self.sub_phase = (self.sub_phase + dt * 0.5).fract();

        // ---- 振荡器：主振 +（可选）失谐伴振；主振相位受 FM 调制 ----
        // 用**相位调制**而不是频率调制：任何载波波形（锯齿/方波/正弦…）都能直接用。
        let fmd = if b.fm_index > 0.0 {
            (std::f64::consts::TAU * self.fm_phase).sin() * b.fm_index
        } else {
            0.0
        };
        self.fm_phase = (self.fm_phase + dt * b.fm_ratio).fract();

        let wave = Wave::from_param(p.wave);
        let main = wave.sample(self.phase + fmd, b.pulse_width, dt);
        let mut sig = if p.detune > 0.0 {
            // 失谐量很小（±1%），相位增量随之微调，抗混叠的 `dt` 也用同一个。
            let dt_det = dt * (1.0 + p.detune as f64 * 0.02);
            self.det_phase = (self.det_phase + dt_det).fract();
            // 两个振荡器各取一半：失谐是「变厚」，不该顺带变响。
            0.5 * (main + wave.sample(self.det_phase + fmd, b.pulse_width, dt_det))
        } else {
            main
        };
        if p.sub > 0.0 {
            sig += p.sub * Wave::Square.sample(self.sub_phase, 0.5, dt * 0.5);
        }

        // ---- 噪声：与振荡器**混合**（`noise = 1` 得到纯噪声，闭镲/军鼓要的正是这个） ----
        if b.noise > 0.0 {
            let w = self.white();
            sig = sig * (1.0 - b.noise) + w * b.noise;
        }

        // ---- 高通（一阶） ----
        if p.hpf > 0.0 {
            let fc = 20.0 * 2f32.powf(p.hpf * 7.0);
            let rc = 1.0 / (std::f64::consts::TAU * fc as f64);
            let dt = 1.0 / sr;
            let alpha = (rc / (rc + dt)) as f32;
            let out = alpha * (self.hpf_prev_out + sig - self.hpf_prev_in);
            self.hpf_prev_in = sig;
            self.hpf_prev_out = out;
            sig = out;
        }

        // ---- 状态变量滤波（2 极点）：截止 = 基础截止 × 包络量 × 键跟随 × LFO ----
        let octaves = self.note as f32 / 12.0 - 5.0;
        let bend = p.filter_env * self.env * 7.0 + p.key_follow * octaves + p.lfo_filter * lfo * 2.0;
        let fc = (20.0 * 900.0f32.powf(p.cutoff) * 2f32.powf(bend))
            .clamp(20.0, (sr * 0.45) as f32);
        let g = (std::f64::consts::PI * fc as f64 / sr).tan() as f32;
        let a1 = 1.0 / (1.0 + g * (g + b.k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = sig - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let mut sig = match b.filter_type {
            FilterType::LowPass => v2,
            FilterType::BandPass => v1,
            FilterType::HighPass => v3 - b.k * v1,
        };

        // ---- 位深压碎：放在滤波**之后**，保留数字硬边而不是被滤波器抹平 ----
        if p.crush > 0.0 {
            sig = (sig * b.crush_levels).round() / b.crush_levels;
        }

        sig * self.env * self.velocity * self.volume
    }
}

/// MIDI 音高 → 频率（A4 = 69 = 440 Hz）。
pub fn note_freq(note: u8) -> f64 {
    440.0 * 2f64.powf((note as f64 - 69.0) / 12.0)
}
