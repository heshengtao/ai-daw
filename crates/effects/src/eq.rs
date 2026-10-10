use ai_daw_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
///
/// `bass` / `treble` 是**有符号**的增益量：0 = 不动，±1 = 该频段加/减 6 dB。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("bass", "Bass", -1.0, 1.0, 0.0),
    ("treble", "Treble", -1.0, 1.0, 0.0),
    ("mix", "Mix", 0.0, 1.0, 1.0),
];

/// 低/高两段的分频点（Hz）：一阶低通取出「低频」，其余归「高频」。
const SPLIT_HZ: f64 = 700.0;

/// 简易两段 EQ（低频搁架 / 高频搁架）。
///
/// 用一阶低通把信号拆成低频与高频两支，各自加/减增益后再合起来。
/// **中性位置逐样本等于输入**（`bass = treble = 0` 时两支的贡献都是 0），所以它可以安全地
/// 常驻在链上做音色微调。
#[derive(Debug)]
pub struct Eq {
    sample_rate: f64,
    bass: f32,
    treble: f32,
    mix: f32,
    /// 每通道的一阶低通状态（低频分量）。
    low: Vec<f32>,
}

impl Eq {
    pub fn new() -> Self {
        Eq {
            sample_rate: 48_000.0,
            bass: PARAM_TABLE[0].4,
            treble: PARAM_TABLE[1].4,
            mix: PARAM_TABLE[2].4,
            low: Vec::new(),
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.bass, self.treble, self.mix]
    }

    fn split_coef(&self) -> f32 {
        (1.0 - (-std::f64::consts::TAU * SPLIT_HZ / self.sample_rate).exp()) as f32
    }
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Eq {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::EQ_ID.into(),
            name: "EQ".into(),
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
            0 => self.bass = value,
            1 => self.treble = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        self.low = vec![0.0; 2];
    }

    fn reset(&mut self) {
        self.low.iter_mut().for_each(|s| *s = 0.0);
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        if self.low.len() != audio.len() {
            return;
        }
        let frames = audio.first().map_or(0, |c| c.len());
        let coef = self.split_coef();
        let (bass, treble, mix) = (self.bass, self.treble, self.mix);

        for frame in 0..frames {
            for (ci, ch) in audio.iter_mut().enumerate() {
                let x = ch[frame];
                let state = &mut self.low[ci];
                *state += (x - *state) * coef;
                let low = *state;
                let high = x - low;
                // 两支的贡献在中性位置（bass = treble = 0）都是 0 → 逐样本等于输入。
                let shaped = x + bass * low + treble * high;
                ch[frame] = x * (1.0 - mix) + shaped * mix;
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

    /// 中性位置（bass = treble = 0、mix = 1）必须逐样本等于输入。
    #[test]
    fn flat_is_passthrough() {
        let mut e = Eq::new();
        e.prepare(48_000.0, 256);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.11).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        e.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        assert_eq!(l, input);
    }

    /// 低频搁架对**直流**（纯低频）作用明显：bass = 1 时输出约为输入的两倍。
    #[test]
    fn bass_boost_lifts_the_low_end() {
        let process = |bass: f32| {
            let mut e = Eq::new();
            e.set_param(&ParamId::from("bass"), bass);
            e.prepare(48_000.0, 48_000);
            let mut l = vec![0.25f32; 48_000];
            let mut r = vec![0.25f32; 48_000];
            e.process(&ctx(48_000), &mut [&mut l, &mut r], &[]);
            *l.last().unwrap()
        };
        assert!(process(1.0) > 0.45, "低频提升应当让直流更大");
        assert!(process(-1.0) < 0.05, "低频衰减应当把直流压低");
    }

    #[test]
    fn reset_clears_filter_state() {
        let mut e = Eq::new();
        e.set_param(&ParamId::from("bass"), 1.0);
        e.prepare(48_000.0, 64);
        let mut l = vec![1.0f32; 64];
        let mut r = vec![1.0f32; 64];
        e.process(&ctx(64), &mut [&mut l, &mut r], &[]);
        e.reset();
        let mut l2 = vec![0.0f32; 64];
        let mut r2 = vec![0.0f32; 64];
        e.process(&ctx(64), &mut [&mut l2, &mut r2], &[]);
        assert!(l2.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn state_roundtrips() {
        let mut e = Eq::new();
        e.set_param(&ParamId::from("bass"), 0.5);
        e.set_param(&ParamId::from("treble"), -0.5);
        let state = e.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);
        let mut d = Eq::new();
        d.load_state(&state).unwrap();
        assert_eq!(d.get_param(&ParamId::from("bass")), Some(0.5));
        assert_eq!(d.get_param(&ParamId::from("treble")), Some(-0.5));
    }
}
