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

    /// 样品点（`phase` 为 0..1 的归一化相位）。
    ///
    /// 锯齿 / 方波没做抗混叠：内置合成器是演示音源，先把面板上的控制都做成真实的。
    fn sample(self, phase: f64, pulse_width: f64) -> f32 {
        let p = phase.fract();
        match self {
            Wave::Saw => (2.0 * p - 1.0) as f32,
            Wave::Square => {
                if p < pulse_width {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Sine => (p * std::f64::consts::TAU).sin() as f32,
            Wave::Triangle => (4.0 * (p - 0.5).abs() - 1.0) as f32,
        }
    }
}

/// 一个声部：DCO（主振 + 低八度副振）→ VCF → VCA。
///
/// 每个声部自带包络与滤波器状态，所以按键之间互不干扰；LFO 是**全局**的（老式模拟合成器
/// 只有一个 LFO），由 [`crate::SynthPlugin`] 推进后把当前值传进来。
#[derive(Debug, Clone, Copy)]
pub struct Voice {
    pub note: u8,
    velocity: f32,
    /// 单音音量（0.0..=1.0，1.0 = 不衰减）；由 `NoteVolume` 事件设定，与力度互不影响。
    volume: f32,
    /// 滑音的目标频率与当前频率（Hz）。
    target_freq: f64,
    freq: f64,
    phase: f64,
    sub_phase: f64,
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
            target_freq: 0.0,
            freq: 0.0,
            phase: 0.0,
            sub_phase: 0.0,
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
        self.env = 0.0;
        self.stage = Stage::Attack;
    }

    /// 设定该声部的单音音量（0..=127）。
    pub fn set_volume(&mut self, value: u8) {
        self.volume = value.min(127) as f32 / 127.0;
    }

    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// 推进一个采样并返回输出。
    pub fn next_sample(&mut self, sample_rate: f64, p: &Params, lfo: f32) -> f32 {
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

        // ---- 音高：滑音 + LFO 颤音 ----
        if p.glide > 0.0 {
            let step = 1.0 / secs(p.glide).max(1.0);
            let diff = self.target_freq - self.freq;
            if diff.abs() <= step {
                self.freq = self.target_freq;
            } else {
                self.freq += diff.signum() * step;
            }
        }
        let pitch_bend = 2f64.powf((p.lfo_pitch * lfo) as f64 / 12.0);
        let f = self.freq * pitch_bend;
        self.phase += f / sr;
        self.phase = self.phase.fract();
        self.sub_phase += f * 0.5 / sr;
        self.sub_phase = self.sub_phase.fract();

        // ---- 振荡器 ----
        let pulse_width = (0.5 + (p.pwm - 0.5) * 0.9) as f64;
        let mut sig = Wave::from_param(p.wave).sample(self.phase, pulse_width);
        if p.sub > 0.0 {
            sig += p.sub * Wave::Square.sample(self.sub_phase, 0.5);
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

        // ---- 低通（状态变量，2 极点）：截止 = 基础截止 × 包络量 × 键跟随 × LFO ----
        let octaves = self.note as f32 / 12.0 - 5.0;
        let bend = p.filter_env * self.env * 7.0 + p.key_follow * octaves + p.lfo_filter * lfo * 2.0;
        let fc = (20.0 * 900.0f32.powf(p.cutoff) * 2f32.powf(bend))
            .clamp(20.0, (sr * 0.45) as f32);
        let g = (std::f64::consts::PI * fc as f64 / sr).tan() as f32;
        let k = 2.0 - 1.9 * p.resonance;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = sig - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;

        v2 * self.env * self.velocity * self.volume
    }
}

/// MIDI 音高 → 频率（A4 = 69 = 440 Hz）。
pub fn note_freq(note: u8) -> f64 {
    440.0 * 2f64.powf((note as f64 - 69.0) / 12.0)
}
