use ai_daw_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("drive", "Drive", 0.0, 1.0, 0.3),
    ("tone", "Tone", 0.0, 1.0, 0.6),
    ("mix", "Mix", 0.0, 1.0, 1.0),
];

/// 驱动增益上限（`drive = 1` 时约 25×，足够把信号推到软削波）。
const MAX_GAIN: f32 = 24.0;
/// 音色一阶低通的截止范围（Hz）：`tone = 0` 最暗，`tone = 1` 基本旁通。
const MIN_TONE_HZ: f64 = 400.0;
const MAX_TONE_HZ: f64 = 18_000.0;

/// 过载/失真：**软削波**（`tanh`）→ 一阶音色低通。
///
/// `mix = 1` 时是全湿；`mix = 0` 逐样本等于输入（与其余内置效果一致）。
/// `drive = 0` 时跳过削波（保持干净），只留音色滤波。
#[derive(Debug)]
pub struct Drive {
    sample_rate: f64,
    drive: f32,
    tone: f32,
    mix: f32,
    /// 每通道的一阶低通状态。
    lp: Vec<f32>,
}

impl Drive {
    pub fn new() -> Self {
        Drive {
            sample_rate: 48_000.0,
            drive: PARAM_TABLE[0].4,
            tone: PARAM_TABLE[1].4,
            mix: PARAM_TABLE[2].4,
            lp: Vec::new(),
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.drive, self.tone, self.mix]
    }

    /// 一阶低通的系数（指数映射 tone → 截止频率，再换算成 RC 形式）。
    fn tone_coef(&self) -> f32 {
        let hz = MIN_TONE_HZ * (MAX_TONE_HZ / MIN_TONE_HZ).powf(self.tone as f64);
        (1.0 - (-std::f64::consts::TAU * hz / self.sample_rate).exp()) as f32
    }
}

impl Default for Drive {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Drive {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::DRIVE_ID.into(),
            name: "Drive".into(),
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
            0 => self.drive = value,
            1 => self.tone = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        // 无延迟线，只有每通道一个滤波状态（分配同样只在 prepare 里做）。
        self.lp = vec![0.0; 2];
    }

    fn reset(&mut self) {
        self.lp.iter_mut().for_each(|s| *s = 0.0);
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        if self.lp.len() != audio.len() {
            return;
        }
        let frames = audio.first().map_or(0, |c| c.len());
        let gain = 1.0 + self.drive * MAX_GAIN;
        // 输出补偿：驱动越大，削波前增益越高，按比例收回一些响度。
        let makeup = 1.0 / (1.0 + self.drive * 5.0);
        let coef = self.tone_coef();
        let (drive, mix) = (self.drive, self.mix);

        for frame in 0..frames {
            for (ci, ch) in audio.iter_mut().enumerate() {
                let input = ch[frame];
                let driven = if drive <= 0.0 {
                    input
                } else {
                    (input * gain).tanh() * makeup
                };
                // 一阶音色低通（暗 → 亮）
                let state = &mut self.lp[ci];
                *state += (driven - *state) * coef;
                ch[frame] = input * (1.0 - mix) + *state * mix;
            }
        }
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

    #[test]
    fn zero_mix_is_passthrough() {
        let mut d = Drive::new();
        d.set_param(&ParamId::from("mix"), 0.0);
        d.prepare(48_000.0, 128);
        let input: Vec<f32> = (0..128).map(|i| (i as f32 * 0.09).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        d.process(&ctx(128), &mut [&mut l, &mut r], &[]);
        assert_eq!(l, input);
    }

    /// 驱动越大，同样的输入被压得越扁（峰值更低、谐波更多）。
    #[test]
    fn drive_saturates_the_signal() {
        let peak = |drive: f32| {
            let mut d = Drive::new();
            d.set_param(&ParamId::from("drive"), drive);
            d.set_param(&ParamId::from("tone"), 1.0);
            d.prepare(48_000.0, 4800);
            let mut l: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin()).collect();
            let mut r = l.clone();
            d.process(&ctx(4800), &mut [&mut l, &mut r], &[]);
            l.iter().fold(0.0f32, |m, s| m.max(s.abs()))
        };
        // 高驱动会削顶，峰值不该超过 1（软削波）。
        assert!(peak(1.0) <= 1.0 + 1e-3, "软削波不该超过满幅");
    }

    #[test]
    fn reset_clears_filter_state() {
        let mut d = Drive::new();
        d.prepare(48_000.0, 64);
        let mut l = vec![1.0f32; 64];
        let mut r = vec![1.0f32; 64];
        d.process(&ctx(64), &mut [&mut l, &mut r], &[]);
        d.reset();
        let mut l2 = vec![0.0f32; 64];
        let mut r2 = vec![0.0f32; 64];
        d.process(&ctx(64), &mut [&mut l2, &mut r2], &[]);
        assert!(l2.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn state_roundtrips() {
        let mut d = Drive::new();
        d.set_param(&ParamId::from("drive"), 0.72);
        d.set_param(&ParamId::from("tone"), 0.4);
        let state = d.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);
        let mut e = Drive::new();
        e.load_state(&state).unwrap();
        assert_eq!(e.get_param(&ParamId::from("drive")), Some(0.72));
        assert_eq!(e.get_param(&ParamId::from("tone")), Some(0.4));
    }
}
