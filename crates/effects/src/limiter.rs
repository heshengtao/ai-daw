use ai_daw_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("ceiling", "Ceiling", -12.0, 0.0, -1.0),
    ("release", "Release", 10.0, 500.0, 80.0),
    ("lookahead", "Lookahead", 1.0, 10.0, 5.0),
];

/// 前瞻式峰值限幅器。
///
/// - **前瞻**：`lookahead` 毫秒（默认 5 ms）。增益在峰值到达输出**之前**就压下去，因此不会
///   出现「先过冲再拉回」的破音。代价是整条链多出 `lookahead` 的延迟——挂主总线没问题
///   （末端无需对齐），挂在轨道插入上会让那一轨整体后移（等 M3 的 PDC 解决）。
/// - **真峰值近似**：除了采样峰值，还检查相邻两采样中点（2× 过采样级）——方波/正弦在
///   采样点之间可能高出 1–3 dB，只测采样峰会漏掉。
/// - **释放**平滑，**启动**瞬时（靠前瞻争取时间）。
#[derive(Debug)]
pub struct Limiter {
    sample_rate: f64,
    ceiling_db: f32,
    release_ms: f32,
    lookahead_ms: f32,
    /// 每声道的前瞻延迟线。
    delay: Vec<Vec<f32>>,
    /// 延迟线写指针。
    write: usize,
    /// 延迟帧数（= 前瞻长度）。
    latency: usize,
    /// 当前增益（线性）。
    gain: f32,
    /// 上一帧输入（算采样间峰值用）。
    last: [f32; 2],
    release_coef: f32,
    /// 本块的最大衰减（dB），供电平表读取。
    last_gr: f32,
}

impl Limiter {
    pub fn new() -> Self {
        Limiter {
            sample_rate: 48_000.0,
            ceiling_db: PARAM_TABLE[0].4,
            release_ms: PARAM_TABLE[1].4,
            lookahead_ms: PARAM_TABLE[2].4,
            delay: Vec::new(),
            write: 0,
            latency: 0,
            gain: 1.0,
            last: [0.0; 2],
            release_coef: 0.001,
            last_gr: 0.0,
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.ceiling_db, self.release_ms, self.lookahead_ms]
    }

    fn ceiling_linear(&self) -> f32 {
        10f32.powf(self.ceiling_db / 20.0)
    }

    fn coef(t_ms: f32, sample_rate: f64) -> f32 {
        let seconds = (t_ms.max(0.1) / 1000.0) as f64;
        let n = seconds * sample_rate;
        if n <= 0.0 {
            1.0
        } else {
            (1.0 - (-1.0 / n).exp()) as f32
        }
    }

    /// 按当前采样率重建延迟线与系数（`prepare` 里调用，可分配内存）。
    fn rebuild(&mut self) {
        let latency = ((self.lookahead_ms as f64 / 1000.0) * self.sample_rate).round() as usize;
        self.latency = latency.max(1);
        self.delay = vec![vec![0.0f32; self.latency.max(2)]; 2];
        self.write = 0;
        self.gain = 1.0;
        self.last = [0.0; 2];
        self.release_coef = Self::coef(self.release_ms, self.sample_rate);
    }
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Limiter {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::LIMITER_ID.into(),
            name: "Limiter".into(),
            kind: PluginKind::Effect,
            io: IoLayout { inputs: 2, outputs: 2 },
            capabilities: PluginCapabilities::default(),
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        PARAM_TABLE
            .iter()
            .map(|(id, name, min, max, default)| ParamInfo {
                id: (*id).into(),
                name: (*name).into(),
                min: *min,
                max: *max,
                default: *default,
                automatable: true,
            })
            .collect()
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        let index = PARAM_TABLE.iter().position(|(k, ..)| *k == id.0.as_str())?;
        Some(self.values()[index])
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        let Some(index) = PARAM_TABLE.iter().position(|(k, ..)| *k == id.0.as_str()) else {
            return;
        };
        let (_, _, min, max, _) = PARAM_TABLE[index];
        if !value.is_finite() {
            return;
        }
        let value = value.clamp(min, max);
        match index {
            0 => self.ceiling_db = value,
            1 => {
                self.release_ms = value;
                self.release_coef = Self::coef(value, self.sample_rate);
            }
            2 => {
                self.lookahead_ms = value;
                self.rebuild();
            }
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        self.rebuild();
    }

    fn reset(&mut self) {
        for line in self.delay.iter_mut() {
            line.iter_mut().for_each(|s| *s = 0.0);
        }
        self.write = 0;
        self.gain = 1.0;
        self.last = [0.0; 2];
        self.last_gr = 0.0;
    }

    fn gain_reduction_db(&self) -> Option<f32> {
        Some(self.last_gr)
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        audio: &mut [&mut [f32]],
        _events: &[TimedMidiEvent],
    ) {
        let frames = audio.first().map_or(0, |c| c.len());
        if frames == 0 || self.latency == 0 || self.delay.len() < audio.len() {
            return;
        }
        let ceiling = self.ceiling_linear();
        let len = self.latency.max(2);
        let mut block_gr = 0.0f32;
        for frame in 0..frames {
            // 1) 先看新进来的采样（含采样间峰值），据此决定这一帧要压多少。
            let mut peak = 0.0f32;
            for fd in 0..2 {
                let x = audio.get(fd).map_or(0.0, |ch| ch[frame]);
                let mid = (x + self.last[fd]) * 0.5;
                peak = peak.max(x.abs()).max(mid.abs());
            }
            let desired = if peak > ceiling && peak > 0.0 {
                (ceiling / peak).min(1.0)
            } else {
                1.0
            };
            // 2) 启动瞬时、释放平滑。
            if desired < self.gain {
                self.gain = desired;
            } else {
                self.gain += (desired - self.gain) * self.release_coef;
            }
            let gr_db = if self.gain > 1e-9 {
                -20.0 * self.gain.log10()
            } else {
                120.0
            };
            if gr_db > block_gr {
                block_gr = gr_db;
            }
            // 3) 输出的是**延迟线里**的旧采样，乘上刚算好的增益。
            let write = self.write;
            for fd in 0..2 {
                let Some(ch) = audio.get_mut(fd) else { continue };
                let x = ch[frame];
                let delayed = self.delay[fd][write];
                self.delay[fd][write] = x;
                ch[frame] = delayed * self.gain;
                self.last[fd] = x;
            }
            self.write += 1;
            if self.write >= len {
                self.write = 0;
            }
        }
        self.last_gr = block_gr;
    }

    fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(PARAM_TABLE.len() * 4);
        for v in self.values() {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        if state.len() < PARAM_TABLE.len() * 4 {
            return Err(PluginError::StateTooShort {
                expected: PARAM_TABLE.len() * 4,
                got: state.len(),
            });
        }
        for (index, (id, ..)) in PARAM_TABLE.iter().enumerate() {
            let start = index * 4;
            let value = f32::from_le_bytes(state[start..start + 4].try_into().unwrap());
            self.set_param(&ParamId::from(*id), value);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(frames: usize) -> ProcessContext {
        ProcessContext { sample_rate: 48_000.0, block_size: frames, tempo: 120.0, playing: true }
    }

    fn sine(amp: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| amp * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    #[test]
    fn loud_signal_is_held_under_the_ceiling() {
        let mut l = sine(1.0, 48_000);
        let mut r = l.clone();
        let mut lim = Limiter::new();
        lim.set_param(&ParamId::from("ceiling"), -1.0);
        lim.prepare(48_000.0, 48_000);
        lim.process(&ctx(48_000), &mut [&mut l, &mut r], &[]);
        // 跳过前 200 ms（释放尚未稳定），量后面的部分。
        let peak = l[9_600..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let db = 20.0 * peak.log10();
        assert!(db <= -0.9, "峰值应守在 -1 dBFS 以下，实际 {db}");
        assert!(lim.gain_reduction_db().unwrap() > 0.5);
    }

    #[test]
    fn quiet_signal_passes_through_after_the_lookahead() {
        let mut lim = Limiter::new();
        lim.prepare(48_000.0, 256);
        let input: Vec<f32> = (0..256).map(|i| 0.1 * (i as f32 * 0.01).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        lim.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        // 延迟 5 ms（240 帧）之后应当逐样本还原（增益恒为 1）。
        assert_eq!(l[240..], input[..16]);
        assert_eq!(lim.gain_reduction_db(), Some(0.0));
    }

    #[test]
    fn reset_clears_delay_and_gain() {
        let mut l = vec![1.0f32; 4_800];
        let mut r = l.clone();
        let mut lim = Limiter::new();
        lim.prepare(48_000.0, 4_800);
        lim.process(&ctx(4_800), &mut [&mut l, &mut r], &[]);
        lim.reset();
        assert_eq!(lim.gain_reduction_db(), Some(0.0));
    }

    #[test]
    fn state_roundtrips() {
        let mut lim = Limiter::new();
        lim.set_param(&ParamId::from("ceiling"), -3.0);
        let state = lim.save_state();
        let mut d = Limiter::new();
        d.load_state(&state).unwrap();
        assert_eq!(d.get_param(&ParamId::from("ceiling")), Some(-3.0));
    }
}
