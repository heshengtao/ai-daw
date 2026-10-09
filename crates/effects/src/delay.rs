use daw_party_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("time", "Time", 0.0, 1.0, 0.35),
    ("feedback", "Feedback", 0.0, 0.95, 0.35),
    ("mix", "Mix", 0.0, 1.0, 0.35),
];

/// 延迟时间的映射范围（秒）。
const MIN_SECS: f64 = 0.02;
const MAX_SECS: f64 = 1.0;

/// 立体声延迟：每个通道各一条延迟线。
///
/// 读位置用**线性插值的分数延迟**，所以拧「时间」旋钮时音高是连续的滑，而不是跳变。
#[derive(Debug)]
pub struct Delay {
    sample_rate: f64,
    time: f32,
    feedback: f32,
    mix: f32,
    /// 每个通道一条延迟线（固定 2 条，与 `IoLayout::outputs` 一致）。
    lines: Vec<Vec<f32>>,
    write: usize,
    len: usize,
}

impl Delay {
    pub fn new() -> Self {
        let time = PARAM_TABLE[0].4;
        let feedback = PARAM_TABLE[1].4;
        let mix = PARAM_TABLE[2].4;
        Delay {
            sample_rate: 48_000.0,
            time,
            feedback,
            mix,
            lines: Vec::new(),
            write: 0,
            len: 0,
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.time, self.feedback, self.mix]
    }
}

impl Default for Delay {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Delay {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::DELAY_ID.into(),
            name: "Delay".into(),
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
            0 => self.time = value,
            1 => self.feedback = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        // 唯一一次分配：延迟线按最长延迟 + 2 帧余量开好，之后回调里只写不分配。
        self.len = (sample_rate * MAX_SECS) as usize + 2;
        self.lines = vec![vec![0.0; self.len]; 2];
        self.write = 0;
    }

    fn reset(&mut self) {
        for line in self.lines.iter_mut() {
            line.iter_mut().for_each(|s| *s = 0.0);
        }
        self.write = 0;
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        if self.len == 0 || self.lines.len() != audio.len() {
            return;
        }
        let frames = audio.first().map_or(0, |c| c.len());
        let delay = (MIN_SECS + (MAX_SECS - MIN_SECS) * self.time as f64) * self.sample_rate;
        let delay = delay.clamp(1.0, (self.len - 2) as f64);
        let (feedback, mix) = (self.feedback, self.mix);

        for frame in 0..frames {
            for (ch, line) in audio.iter_mut().zip(self.lines.iter_mut()) {
                let input = ch[frame];
                let read = (self.write as f64 + self.len as f64 - delay) % self.len as f64;
                let base = read.floor();
                let i0 = base as usize % self.len;
                let i1 = (i0 + 1) % self.len;
                let frac = (read - base) as f32;
                let delayed = line[i0] * (1.0 - frac) + line[i1] * frac;
                line[self.write] = input + delayed * feedback;
                // `mix = 0` 必须**逐样本等于输入**，所以这里不做任何额外缩放。
                ch[frame] = input * (1.0 - mix) + delayed * mix;
            }
            self.write = (self.write + 1) % self.len;
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
        if state.len() < 12 {
            return Err(PluginError::StateTooShort { expected: 12, got: state.len() });
        }
        for (index, (id, ..)) in PARAM_TABLE.iter().enumerate() {
            let start = index * 4;
            if start + 4 > state.len() {
                break;
            }
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

    /// 一帧脉冲进去、后面全是静音，返回整段输出。
    fn impulse(params: &[(&str, f32)], frames: usize) -> Vec<f32> {
        let mut d = Delay::new();
        for (id, value) in params {
            d.set_param(&ParamId::from(*id), *value);
        }
        d.prepare(48_000.0, frames);
        let mut l = vec![0.0f32; frames];
        let mut r = vec![0.0f32; frames];
        l[0] = 1.0;
        r[0] = 1.0;
        d.process(&ctx(frames), &mut [&mut l, &mut r], &[]);
        l
    }

    /// `mix = 0` 必须逐样本等于输入（效果器「不起作用」时不能悄悄改信号）。
    #[test]
    fn zero_mix_is_passthrough() {
        let mut d = Delay::new();
        d.set_param(&ParamId::from("mix"), 0.0);
        d.set_param(&ParamId::from("time"), 0.2);
        d.prepare(48_000.0, 64);
        let input: Vec<f32> = (0..64).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut l = input.clone();
        let mut r = input.clone();
        d.process(&ctx(64), &mut [&mut l, &mut r], &[]);
        assert_eq!(l, input);
    }

    /// 回声落在正确的位置（20 ms..1 s 的映射），且反馈为 0 时只有一次回声。
    #[test]
    fn impulse_echoes_at_the_mapped_time() {
        // time = 0.5 → 0.02 + 0.98 × 0.5 = 0.51 s → 24480 帧。
        let out = impulse(&[("time", 0.5), ("mix", 1.0), ("feedback", 0.0)], 30_000);
        let expected = (0.02 + 0.98 * 0.5) * 48_000.0;
        let peak = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(
            (peak as f64 - expected).abs() <= 2.0,
            "回声应当在第 {expected:.0} 帧，实际 {peak}"
        );
    }

    /// 反馈越大尾巴越长。
    #[test]
    fn feedback_lengthens_the_tail() {
        let tail = |feedback: f32| {
            let out = impulse(&[("time", 0.1), ("mix", 1.0), ("feedback", feedback)], 48_000);
            out.iter().filter(|s| s.abs() > 1e-4).count()
        };
        let short = tail(0.0);
        let long = tail(0.9);
        assert!(long > short * 3, "反馈应当让尾巴明显变长：{short} -> {long}");
    }

    /// 重置后不再有尾音（暂停 = 静音的硬要求）。
    #[test]
    fn reset_clears_the_tail() {
        let mut d = Delay::new();
        d.set_param(&ParamId::from("mix"), 1.0);
        d.set_param(&ParamId::from("feedback"), 0.9);
        d.prepare(48_000.0, 4800);
        let mut l = vec![0.0f32; 4800];
        let mut r = vec![0.0f32; 4800];
        l[0] = 1.0;
        r[0] = 1.0;
        d.process(&ctx(4800), &mut [&mut l, &mut r], &[]);
        d.reset();
        let mut l2 = vec![0.0f32; 4800];
        let mut r2 = vec![0.0f32; 4800];
        d.process(&ctx(4800), &mut [&mut l2, &mut r2], &[]);
        assert!(l2.iter().all(|s| *s == 0.0), "重置后不得残留尾音");
    }

    #[test]
    fn state_roundtrips() {
        let mut d = Delay::new();
        d.set_param(&ParamId::from("time"), 0.7);
        d.set_param(&ParamId::from("feedback"), 0.6);
        let state = d.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);
        let mut e = Delay::new();
        e.load_state(&state).unwrap();
        assert_eq!(e.get_param(&ParamId::from("time")), Some(0.7));
        assert_eq!(e.get_param(&ParamId::from("feedback")), Some(0.6));
    }
}
