use daw_party_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 3] = [
    ("size", "Size", 0.0, 1.0, 0.6),
    ("damp", "Damp", 0.0, 1.0, 0.4),
    ("mix", "Mix", 0.0, 1.0, 0.3),
];

/// 施罗德混响：4 个并联梳状滤波器 + 2 个串联全通，每个通道各一套。
///
/// 时值取自 Freeverb 的经典刻度（44.1 kHz），右通道整体偏移 [`STEREO_SPREAD`] 个采样获得立体声宽度。
/// **缓冲长度在 `prepare` 里定死**（`size` 只调反馈量，不调长度）——音频回调里不许重新分配。
#[derive(Debug)]
pub struct Reverb {
    sample_rate: f64,
    size: f32,
    damp: f32,
    mix: f32,
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
    /// 是否已经按采样率开过缓冲。
    built: bool,
}

const COMBS: usize = 4;
const ALLPASSES: usize = 2;
const CHANNELS: usize = 2;
const COMB_TUNING: [usize; COMBS] = [1116, 1188, 1277, 1356];
const ALLPASS_TUNING: [usize; ALLPASSES] = [556, 441];
/// Freeverb 的立体声展开：右通道的延迟线整体长这么多。
const STEREO_SPREAD: usize = 23;
const BASE_SR: f64 = 44_100.0;
/// 梳状滤波器的基础反馈（< 1，保证稳定）。
const COMB_FEEDBACK: f32 = 0.84;
const ALLPASS_FEEDBACK: f32 = 0.5;

#[derive(Debug)]
struct Comb {
    buf: Vec<f32>,
    idx: usize,
    /// 一阶低通的记忆（高频阻尼）。
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb { buf: vec![0.0; len.max(1)], idx: 0, store: 0.0 }
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.idx = 0;
        self.store = 0.0;
    }

    fn step(&mut self, input: f32, feedback: f32, damp: f32) -> f32 {
        let out = self.buf[self.idx];
        // 阻尼：反馈路径上的一阶低通，高频衰减得更快（更像真房间）。
        self.store = out * (1.0 - damp) + self.store * damp;
        self.buf[self.idx] = input + self.store * feedback;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

#[derive(Debug)]
struct Allpass {
    buf: Vec<f32>,
    idx: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass { buf: vec![0.0; len.max(1)], idx: 0 }
    }

    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
        self.idx = 0;
    }

    fn step(&mut self, input: f32) -> f32 {
        let bufout = self.buf[self.idx];
        let out = -input + bufout;
        self.buf[self.idx] = input + bufout * ALLPASS_FEEDBACK;
        self.idx = (self.idx + 1) % self.buf.len();
        out
    }
}

impl Reverb {
    pub fn new() -> Self {
        Reverb {
            sample_rate: 48_000.0,
            size: PARAM_TABLE[0].4,
            damp: PARAM_TABLE[1].4,
            mix: PARAM_TABLE[2].4,
            combs: Vec::new(),
            allpasses: Vec::new(),
            built: false,
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [self.size, self.damp, self.mix]
    }

    /// 一个通道的一次混响采样。
    fn channel_sample(&mut self, ch: usize, input: f32) -> f32 {
        // size 调的是**反馈量**（0.50..0.84）：越大尾音越长。长度不变，所以不需要重新分配。
        let feedback = COMB_FEEDBACK * (0.6 + 0.4 * self.size);
        let damp = self.damp * 0.9;
        let mut acc = 0.0;
        for n in 0..COMBS {
            acc += self.combs[ch * COMBS + n].step(input, feedback, damp);
        }
        for n in 0..ALLPASSES {
            acc = self.allpasses[ch * ALLPASSES + n].step(acc);
        }
        acc
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Reverb {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::REVERB_ID.into(),
            name: "Reverb".into(),
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
            0 => self.size = value,
            1 => self.damp = value,
            2 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        let scale = sample_rate / BASE_SR;
        self.combs = Vec::with_capacity(CHANNELS * COMBS);
        for ch in 0..CHANNELS {
            for tuning in COMB_TUNING {
                let len = (tuning as f64 * scale) as usize + ch * STEREO_SPREAD + 1;
                self.combs.push(Comb::new(len));
            }
        }
        self.allpasses = Vec::with_capacity(CHANNELS * ALLPASSES);
        for ch in 0..CHANNELS {
            for tuning in ALLPASS_TUNING {
                let len = (tuning as f64 * scale) as usize + ch * STEREO_SPREAD + 1;
                self.allpasses.push(Allpass::new(len));
            }
        }
        self.built = true;
    }

    fn reset(&mut self) {
        for c in self.combs.iter_mut() {
            c.clear();
        }
        for a in self.allpasses.iter_mut() {
            a.clear();
        }
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
        if !self.built || audio.len() < CHANNELS {
            return;
        }
        let frames = audio.first().map_or(0, |c| c.len());
        let mix = self.mix;
        for frame in 0..frames {
            for ch in 0..CHANNELS {
                let input = audio[ch][frame];
                let wet = self.channel_sample(ch, input);
                // `mix = 0` 逐样本等于输入。
                audio[ch][frame] = input * (1.0 - mix) + wet * mix;
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

    /// 一帧脉冲进去，返回整段输出（左右各一份）。
    fn impulse(params: &[(&str, f32)], frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut r = Reverb::new();
        for (id, value) in params {
            r.set_param(&ParamId::from(*id), *value);
        }
        r.prepare(48_000.0, frames);
        let mut l = vec![0.0f32; frames];
        let mut rt = vec![0.0f32; frames];
        l[0] = 1.0;
        rt[0] = 1.0;
        r.process(&ctx(frames), &mut [&mut l, &mut rt], &[]);
        (l, rt)
    }

    #[test]
    fn zero_mix_is_passthrough() {
        let mut r = Reverb::new();
        r.set_param(&ParamId::from("mix"), 0.0);
        r.prepare(48_000.0, 64);
        let input: Vec<f32> = (0..64).map(|i| (i as f32 * 0.1).sin()).collect();
        let mut l = input.clone();
        let mut rt = input.clone();
        r.process(&ctx(64), &mut [&mut l, &mut rt], &[]);
        assert_eq!(l, input);
    }

    /// 尾音：有能量、随时间衰减、且始终有限（稳定）。
    #[test]
    fn tail_decays_and_stays_finite() {
        let (l, _) = impulse(&[("mix", 1.0), ("size", 1.0), ("damp", 0.2)], 96_000);
        assert!(l.iter().all(|s| s.is_finite()), "不得出现 NaN / Inf");
        let energy = |a: usize, b: usize| -> f64 {
            l[a..b].iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
        };
        let (early, mid, late) = (
            energy(0, 24_000),
            energy(24_000, 48_000),
            energy(72_000, 96_000),
        );
        assert!(early > 1e-3, "混响应当有尾音：{early}");
        assert!(mid < early, "尾音应当衰减：{early} -> {mid}");
        assert!(late < mid * 0.5, "继续衰减：{mid} -> {late}");
    }

    /// 左右通道不一样宽（立体声展开），否则听起来是单声道。
    #[test]
    fn channels_are_decorrelated() {
        let (l, r) = impulse(&[("mix", 1.0)], 24_000);
        let diff: f64 = l
            .iter()
            .zip(r.iter())
            .map(|(a, b)| ((*a - *b) as f64).powi(2))
            .sum();
        assert!(diff > 1e-4, "左右通道应当有差异（立体声宽度）：{diff}");
    }

    /// 重置后不再有尾音。
    #[test]
    fn reset_clears_the_tail() {
        let mut r = Reverb::new();
        r.set_param(&ParamId::from("mix"), 1.0);
        r.prepare(48_000.0, 4800);
        let mut l = vec![0.0f32; 4800];
        let mut rt = vec![0.0f32; 4800];
        l[0] = 1.0;
        rt[0] = 1.0;
        r.process(&ctx(4800), &mut [&mut l, &mut rt], &[]);
        r.reset();
        let mut l2 = vec![0.0f32; 4800];
        let mut rt2 = vec![0.0f32; 4800];
        r.process(&ctx(4800), &mut [&mut l2, &mut rt2], &[]);
        assert!(l2.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn state_roundtrips() {
        let mut r = Reverb::new();
        r.set_param(&ParamId::from("size"), 0.9);
        r.set_param(&ParamId::from("damp"), 0.1);
        let state = r.save_state();
        assert_eq!(state.len(), PARAM_TABLE.len() * 4);
        let mut e = Reverb::new();
        e.load_state(&state).unwrap();
        assert_eq!(e.get_param(&ParamId::from("size")), Some(0.9));
        assert_eq!(e.get_param(&ParamId::from("damp")), Some(0.1));
    }
}
