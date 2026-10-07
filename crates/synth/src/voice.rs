/// 一个声部的包络阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Sustain,
    Release,
}

/// 正弦 + 线性 ADSR 的复音声部。
#[derive(Debug, Clone, Copy)]
pub struct Voice {
    pub note: u8,
    pub velocity: f32,
    phase: f64,
    env: f32,
    pub stage: Stage,
}

impl Voice {
    pub const fn idle() -> Self {
        Voice { note: 0, velocity: 0.0, phase: 0.0, env: 0.0, stage: Stage::Idle }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8) {
        self.note = note;
        self.velocity = velocity as f32 / 127.0;
        self.phase = 0.0;
        self.env = 0.0;
        self.stage = Stage::Attack;
    }

    pub fn note_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    /// 返回该声部的单样本输出；`active` 表示仍需继续推进。
    pub fn next_sample(&mut self, sample_rate: f64, attack: f64, release: f64) -> (f32, bool) {
        if self.stage == Stage::Idle {
            return (0.0, false);
        }
        let a = if attack <= 0.0 { 1.0 } else { 1.0 / (attack * sample_rate) };
        let r = if release <= 0.0 { 1.0 } else { 1.0 / (release * sample_rate) };
        match self.stage {
            Stage::Attack => {
                self.env += a as f32;
                if self.env >= 1.0 {
                    self.env = 1.0;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {}
            Stage::Release => {
                self.env -= r as f32;
                if self.env <= 0.0 {
                    self.env = 0.0;
                    self.stage = Stage::Idle;
                    return (0.0, false);
                }
            }
            Stage::Idle => {}
        }
        let freq = 440.0 * 2f64.powf((self.note as f64 - 69.0) / 12.0);
        self.phase += freq / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let s = (self.phase * std::f64::consts::TAU).sin() as f32;
        (s * self.env * self.velocity, true)
    }
}
