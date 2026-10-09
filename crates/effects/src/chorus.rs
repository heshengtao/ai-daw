use daw_party_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("rate", "Rate", 0.0, 1.0, 0.35),
    ("depth", "Depth", 0.0, 1.0, 0.5),
    ("mix", "Mix", 0.0, 1.0, 0.5),
];

/// LFO 速率映射范围（Hz）。
const MIN_RATE_HZ: f64 = 0.05;
const MAX_RATE_HZ: f64 = 6.0;
/// 基础延迟与最大调制深度（毫秒）。左右声道 LFO 相位错开 1/4 周期，撑出立体声宽度。
const BASE_DELAY_MS: f64 = 12.0;
const MAX_DEPTH_MS: f64 = 7.0;

/// 立体声合唱：每个通道一条**被 LFO 调制的**短延迟线。
///
/// 与延迟/混响一样：所有缓冲区只在 `prepare` 里分配，`process` 内不分配、不加锁。
/// 调制读数用线性插值的分数延迟，所以 LFO 扫动时是平滑的音高起伏，而不是台阶。
#[derive(Debug)]
pub struct Chorus {
    sample_rate: f64,
    rate: f32,
    depth: f32,
    mix: f32,
    lines: Vec<Vec<f32>>,
    write: usize,
    len: usize,
    /// LFO 相位（0..1），左右声道按固定偏移错开。
    phase: f64,
}

impl Chorus {
    pub fn new() -> Self {
        Chorus {
            sample_rate: 48_000.0,
            rate: PARAM_TABLE[0].4,
            depth: PARAM_TABLE[1].4,
            mix: PARAM_TABLE[2].4,
            lines: Vec::new(),
            write: 0,
            len: 0,
            phase: 0.0,
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.rate, self.depth, self.mix]
    }
}

impl Default for Chorus {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Chorus {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::CHORUS_ID.into(),
            name: "Chorus".into(),
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
            0 => self.rate = value,
            1 => self.depth = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        // 唯一一次分配：延迟线按最长延迟（基础 + 最大深度）+ 2 帧余量开好。
        let max_ms = BASE_DELAY_MS + MAX_DEPTH_MS + 2.0;
        self.len = (sample_rate * max_ms / 1000.0) as usize + 2;
        self.lines = vec![vec![0.0; self.len]; 2];
        self.write = 0;
        self.phase = 0.0;
    }

    fn reset(&mut self) {
        for line in self.lines.iter_mut() {
            line.iter_mut().for_each(|s| *s = 0.0);
        }
        self.write = 0;
        self.phase = 0.0;
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        if self.len == 0 || self.lines.len() != audio.len() {
            return;
        }
        let frames = audio.first().map_or(0, |c| c.len());
        let rate_hz = MIN_RATE_HZ + (MAX_RATE_HZ - MIN_RATE_HZ) * self.rate as f64;
        let depth_samples = MAX_DEPTH_MS * self.depth as f64 * self.sample_rate / 1000.0;
        let base_samples = BASE_DELAY_MS * self.sample_rate / 1000.0;
        let (mix, len) = (self.mix, self.len);

        for frame in 0..frames {
            for (ci, ch) in audio.iter_mut().enumerate() {
                let line = &mut self.lines[ci];
                let input = ch[frame];
                // 右声道 LFO 相位 +0.25，让两侧的调制不同步 → 更宽。
                let ch_phase = if ci == 0 { 0.0 } else { 0.25 };
                let lfo = ((self.phase + ch_phase) * std::f64::consts::TAU).sin();
                let delay = (base_samples + depth_samples * (0.5 + 0.5 * lfo))
                    .clamp(1.0, (len - 2) as f64);

                let read = (self.write as f64 + len as f64 - delay) % len as f64;
                let i0 = read.floor() as usize % len;
                let i1 = (i0 + 1) % len;
                let frac = (read - read.floor()) as f32;
                let delayed = line[i0] * (1.0 - frac) + line[i1] * frac;

                line[self.write] = input;
                // `mix = 0` 必须逐样本等于输入：这里不做任何额外缩放。
                ch[frame] = input * (1.0 - mix) + delayed * mix;
            }
            self.write = (self.write + 1) % len;
            self.phase = (self.phase + rate_hz / self.sample_rate).fract();
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

    /// `mix = 0` 必须逐样本等于输入。
    #[test]
    fn zero_mix_is_passthrough() {
        let mut c = Chorus::new();
        c.set_param(&ParamId::from("mix"), 0.0);
        c.prepare(48_000.0, 256);
        let input: Vec<f32> = (0..256).map(|i| (i as f32 * 0.07).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        c.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        assert_eq!(l, input);
    }

    /// 全湿时输出不应超过输入太多（LFO 只是延时混合，不放大）。
    #[test]
    fn wet_output_stays_bounded() {
        let mut c = Chorus::new();
        c.set_param(&ParamId::from("mix"), 1.0);
        c.set_param(&ParamId::from("depth"), 1.0);
        c.prepare(48_000.0, 4800);
        let mut l: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.8).collect();
        let mut r = l.clone();
        c.process(&ctx(4800), &mut [&mut l, &mut r], &[]);
        assert!(l.iter().all(|s| s.abs() <= 1.0), "合唱不该放大到爆表");
    }

    /// 重置后不再有余音。
    #[test]
    fn reset_clears_the_tail() {
        let mut c = Chorus::new();
        c.set_param(&ParamId::from("mix"), 1.0);
        c.prepare(48_000.0, 4800);
        let mut l = vec![0.0f32; 4800];
        let mut r = vec![0.0f32; 4800];
        l[0] = 1.0;
        r[0] = 1.0;
        c.process(&ctx(4800), &mut [&mut l, &mut r], &[]);
        c.reset();
        let mut l2 = vec![0.0f32; 4800];
        let mut r2 = vec![0.0f32; 4800];
        c.process(&ctx(4800), &mut [&mut l2, &mut r2], &[]);
        assert!(l2.iter().all(|s| *s == 0.0), "重置后不得残留尾音");
    }

    #[test]
    fn state_roundtrips() {
        let mut c = Chorus::new();
        c.set_param(&ParamId::from("rate"), 0.8);
        c.set_param(&ParamId::from("depth"), 0.4);
        let state = c.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);
        let mut e = Chorus::new();
        e.load_state(&state).unwrap();
        assert_eq!(e.get_param(&ParamId::from("rate")), Some(0.8));
        assert_eq!(e.get_param(&ParamId::from("depth")), Some(0.4));
    }
}
