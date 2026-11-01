use ai_daw_plugin_api::{
    IoLayout, ParamId, ParamInfo, Plugin, PluginCapabilities, PluginDescriptor, PluginError,
    PluginKind, ProcessContext, TimedMidiEvent,
};

/// 参数表：id、显示名、范围、默认值。顺序即 `save_state` 的字节顺序。
const PARAM_TABLE: [(&str, &str, f32, f32, f32); 7] = [
    ("threshold", "Threshold", -60.0, 0.0, -18.0),
    ("ratio", "Ratio", 1.0, 20.0, 4.0),
    ("attack", "Attack", 0.1, 200.0, 10.0),
    ("release", "Release", 5.0, 2000.0, 150.0),
    ("knee", "Knee", 0.0, 24.0, 6.0),
    ("makeup", "Makeup", -12.0, 24.0, 0.0),
    ("mix", "Mix", 0.0, 1.0, 1.0),
];

/// 立体声联动的前馈压缩器。
///
/// **0 延迟**：没有前瞻，因此不需要延迟补偿就能和别的轨道对齐——这是它默认能挂在任意
/// 插入槽上的前提（带前瞻的限幅器在 M3 的 PDC 之前只建议挂主总线）。
///
/// 检测器取 `max(|L|, |R|)`（立体声联动），所以硬声像的素材不会因为「只压左声道」而让
/// 声像左右晃动。接了侧链时**检测器改用键信号**，增益仍然作用在主信号上——这正是「让贝斯
/// 给人声让路」的做法。
#[derive(Debug)]
pub struct Compressor {
    sample_rate: f64,
    threshold: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    knee: f32,
    makeup: f32,
    mix: f32,
    /// 增益衰减包络（dB，正数）。
    envelope: f32,
    /// 峰值保持检测器（dBFS）：瞬时上升、按 release 回落。
    ///
    /// 少了它，检测器看到的是「每个采样点的瞬时电平」——正弦在过零点附近电平极低，增益会
    /// 跟着周期上下抖，实际衰减只有理论值的一半（听感上是「压不住 + 喘气」）。
    detector_db: f32,
    /// 本块里出现过的最大衰减（供电平表读取）。
    last_gr: f32,
    /// attack / release 的单极点系数（`prepare` 里按采样率算好）。
    attack_coef: f32,
    release_coef: f32,
    /// 键输入是否已就位（上一块是否真的收到了 `Some`）。
    sidechain_live: bool,
}

impl Compressor {
    pub fn new() -> Self {
        Compressor {
            sample_rate: 48_000.0,
            threshold: PARAM_TABLE[0].4,
            ratio: PARAM_TABLE[1].4,
            attack_ms: PARAM_TABLE[2].4,
            release_ms: PARAM_TABLE[3].4,
            knee: PARAM_TABLE[4].4,
            makeup: PARAM_TABLE[5].4,
            mix: PARAM_TABLE[6].4,
            envelope: 0.0,
            detector_db: -120.0,
            last_gr: 0.0,
            attack_coef: 0.01,
            release_coef: 0.001,
            sidechain_live: false,
        }
    }

    fn values(&self) -> [f32; PARAM_TABLE.len()] {
        [
            self.threshold,
            self.ratio,
            self.attack_ms,
            self.release_ms,
            self.knee,
            self.makeup,
            self.mix,
        ]
    }

    /// 单极点系数：`1 - exp(-1 / (秒 * 采样率))`。
    fn coef(t_ms: f32, sample_rate: f64) -> f32 {
        let seconds = (t_ms.max(0.01) / 1000.0) as f64;
        let n = seconds * sample_rate;
        if n <= 0.0 {
            1.0
        } else {
            (1.0 - (-1.0 / n).exp()) as f32
        }
    }

    /// 由输入电平（dBFS）算目标衰减（dB，正数 = 压下去多少）。
    ///
    /// 软拐点：在 `threshold ± knee/2` 之间用二次曲线过渡（与主流压缩器的 knee 行为一致）。
    fn target_reduction(&self, level_db: f32) -> f32 {
        if !level_db.is_finite() {
            return 0.0;
        }
        let over = level_db - self.threshold;
        let slope = 1.0 - 1.0 / self.ratio.max(1.0);
        if self.knee <= 0.0 {
            return (over * slope).max(0.0);
        }
        let half = self.knee * 0.5;
        if over <= -half {
            0.0
        } else if over >= half {
            over * slope
        } else {
            // 二次插值：拐点两端一阶连续，听感上比硬拐点顺。
            let t = over + half;
            slope * t * t / (2.0 * self.knee)
        }
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for Compressor {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: crate::COMPRESSOR_ID.into(),
            name: "Compressor".into(),
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
            0 => self.threshold = value,
            1 => self.ratio = value,
            2 => {
                self.attack_ms = value;
                self.attack_coef = Self::coef(value, self.sample_rate);
            }
            3 => {
                self.release_ms = value;
                self.release_coef = Self::coef(value, self.sample_rate);
            }
            4 => self.knee = value,
            5 => self.makeup = value,
            6 => self.mix = value,
            _ => {}
        }
    }

    fn prepare(&mut self, sample_rate: f64, _block_size: usize) {
        self.sample_rate = sample_rate;
        self.attack_coef = Self::coef(self.attack_ms, sample_rate);
        self.release_coef = Self::coef(self.release_ms, sample_rate);
        self.reset();
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.detector_db = -120.0;
        self.last_gr = 0.0;
        self.sidechain_live = false;
    }

    fn sidechain_channels(&self) -> u8 {
        2
    }

    fn gain_reduction_db(&self) -> Option<f32> {
        Some(self.last_gr)
    }

    fn process_with_sidechain(
        &mut self,
        ctx: &ProcessContext,
        audio: &mut [&mut [f32]],
        _events: &[TimedMidiEvent],
        key: Option<&[&[f32]]>,
    ) {
        self.process_impl(audio, key);
        let _ = ctx;
    }

    fn process(
        &mut self,
        _ctx: &ProcessContext,
        audio: &mut [&mut [f32]],
        _events: &[TimedMidiEvent],
    ) {
        self.process_impl(audio, None);
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

impl Compressor {
    fn process_impl(&mut self, audio: &mut [&mut [f32]], key: Option<&[&[f32]]>) {
        let frames = audio.first().map_or(0, |c| c.len());
        if frames == 0 {
            self.last_gr = 0.0;
            return;
        }
        let channels = audio.len();
        // 真实生效的键（通道数对不上就当没接，避免越界）。
        let key_ok = key.filter(|k| !k.is_empty() && k.iter().all(|c| c.len() >= frames));
        self.sidechain_live = key_ok.is_some();
        let makeup = 10f32.powf(self.makeup / 20.0);
        let mix = self.mix;
        let mut block_gr = 0.0f32;

        for frame in 0..frames {
            // 检测器：立体声联动（取各声道绝对值的最大）；有键就用键。
            let mut peak = 0.0f32;
            match key_ok {
                Some(k) => {
                    for ch in k.iter() {
                        peak = peak.max(ch[frame].abs());
                    }
                }
                None => {
                    for ch in audio.iter() {
                        peak = peak.max(ch[frame].abs());
                    }
                }
            }
            let level_db = if peak > 1e-9 {
                20.0 * peak.log10()
            } else {
                -120.0
            };
            // 峰值保持：上升瞬时跟上，回落按 release 衰减——于是「压多少」由真正的峰值决定，
            // 而不是由采样点恰好落在正弦哪个相位决定。
            self.detector_db = if level_db > self.detector_db {
                level_db
            } else {
                self.detector_db + (level_db - self.detector_db) * self.release_coef
            };
            let target = self.target_reduction(self.detector_db);
            // 包络：起压用 attack，回落用 release（与硬件压缩器一致）。
            let coef = if target > self.envelope {
                self.attack_coef
            } else {
                self.release_coef
            };
            self.envelope += (target - self.envelope) * coef;
            if self.envelope > block_gr {
                block_gr = self.envelope;
            }
            let gain = 10f32.powf(-self.envelope / 20.0) * makeup;
            for ch in audio.iter_mut() {
                let dry = ch[frame];
                let wet = dry * gain;
                ch[frame] = dry * (1.0 - mix) + wet * mix;
            }
        }
        // 释放结束后表头归零（否则表会一直停在最后一帧的读数上）。
        self.last_gr = if self.sidechain_live || block_gr > 0.0 {
            block_gr
        } else {
            self.envelope
        };
        if channels == 0 {
            self.last_gr = 0.0;
        }
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
            .map(|i| amp * (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin())
            .collect()
    }

    fn run(c: &mut Compressor, amp: f32, frames: usize) -> f32 {
        let mut l = sine(amp, frames);
        let mut r = l.clone();
        c.process(&ctx(frames), &mut [&mut l, &mut r], &[]);
        l.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    /// 静态曲线：-20 dBFS 输入、阈值 -30 dB、4:1、硬拐点 → 稳态输出 ≈ -27.5 dBFS。
    #[test]
    fn static_curve_matches_the_textbook() {
        let mut c = Compressor::new();
        c.set_param(&ParamId::from("threshold"), -30.0);
        c.set_param(&ParamId::from("ratio"), 4.0);
        c.set_param(&ParamId::from("knee"), 0.0);
        c.set_param(&ParamId::from("attack"), 0.1);
        c.set_param(&ParamId::from("release"), 5.0);
        c.prepare(48_000.0, 48_000);
        // 先跑 1 秒让包络稳定，再量第二个 1 秒。
        run(&mut c, 0.1, 48_000);
        let peak = run(&mut c, 0.1, 48_000);
        let db = 20.0 * peak.log10();
        assert!((db - (-27.5)).abs() < 0.5, "稳态输出应约 -27.5 dBFS，实际 {db}");
        // 表头读数应与实际衰减一致（约 7.5 dB）。
        let gr = c.gain_reduction_db().unwrap();
        assert!((gr - 7.5).abs() < 0.5, "GR 应约 7.5 dB，实际 {gr}");
    }

    /// 低于阈值时**逐样本等于输入**（ratio 再大也不动）。
    #[test]
    fn below_threshold_is_transparent() {
        let mut c = Compressor::new();
        c.set_param(&ParamId::from("threshold"), -6.0);
        c.prepare(48_000.0, 256);
        let mut l = sine(0.1, 256);
        let mut r = l.clone();
        let before = l.clone();
        c.process(&ctx(256), &mut [&mut l, &mut r], &[]);
        assert_eq!(l, before);
    }

    /// 侧链：检测器听键信号，增益作用在主信号上。
    #[test]
    fn sidechain_ducks_the_main_signal() {
        let mut c = Compressor::new();
        c.set_param(&ParamId::from("threshold"), -20.0);
        c.set_param(&ParamId::from("ratio"), 8.0);
        c.set_param(&ParamId::from("knee"), 0.0);
        c.set_param(&ParamId::from("attack"), 0.1);
        c.prepare(48_000.0, 48_000);
        // 主信号很轻（-40 dBFS），键信号很响（-6 dBFS）。
        let mut l = sine(0.01, 48_000);
        let mut r = l.clone();
        let key = sine(0.5, 48_000);
        let keys: [&[f32]; 2] = [&key, &key];
        c.process_with_sidechain(&ctx(48_000), &mut [&mut l, &mut r], &[], Some(&keys));
        // 跳过起压的过渡段（前 100 ms），量稳态。
        let peak = l[4_800..].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.004, "键很响时主信号应被压下去，实际峰值 {peak}");
        assert!(c.gain_reduction_db().unwrap() > 10.0);
    }

    /// 没有键时侧链不生效（键只是「可选检测源」）。
    #[test]
    fn missing_sidechain_falls_back_to_the_main_signal() {
        let mut c = Compressor::new();
        c.set_param(&ParamId::from("threshold"), -30.0);
        c.set_param(&ParamId::from("knee"), 0.0);
        c.prepare(48_000.0, 256);
        let mut l = sine(0.5, 256);
        let mut r = l.clone();
        c.process_with_sidechain(&ctx(256), &mut [&mut l, &mut r], &[], None);
        assert!(c.gain_reduction_db().unwrap() > 1.0);
    }

    #[test]
    fn reset_clears_the_envelope() {
        let mut c = Compressor::new();
        c.prepare(48_000.0, 256);
        run(&mut c, 0.9, 256);
        assert!(c.gain_reduction_db().unwrap() > 0.0);
        c.reset();
        assert_eq!(c.gain_reduction_db(), Some(0.0));
    }

    #[test]
    fn state_roundtrips() {
        let mut c = Compressor::new();
        c.set_param(&ParamId::from("threshold"), -12.0);
        c.set_param(&ParamId::from("ratio"), 8.0);
        let state = c.save_state();
        let mut d = Compressor::new();
        d.load_state(&state).unwrap();
        assert_eq!(d.get_param(&ParamId::from("threshold")), Some(-12.0));
        assert_eq!(d.get_param(&ParamId::from("ratio")), Some(8.0));
    }
}
