use crate::{ParamId, ParamInfo, PluginDescriptor, PluginError, TimedMidiEvent};

/// 每次 `process` 的上下文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessContext {
    pub sample_rate: f64,
    pub block_size: usize,
    pub tempo: f64,
    pub playing: bool,
}

/// 统一的插件接口：内置合成器与外部宿主插件都实现它。
pub trait Plugin: Send {
    /// 描述符。
    fn descriptor(&self) -> PluginDescriptor;
    /// 参数列表。
    fn params(&self) -> Vec<ParamInfo>;
    /// 读取参数当前值。
    fn get_param(&self, id: &ParamId) -> Option<f32>;
    /// 设置参数值（实现方自行 clamp）。
    fn set_param(&mut self, id: &ParamId, value: f32);
    /// 采样率/块大小变化时调用。
    fn prepare(&mut self, sample_rate: f64, block_size: usize);
    /// 清空内部状态（延迟线、包络、声部）。
    fn reset(&mut self);
    /// 就地处理一个音频块。
    ///
    /// `audio` 是平面缓冲（每个元素为一个通道的切片，长度均为本块帧数，即
    /// `audio.len() == descriptor().io.outputs`）。宿主在调用前已把本块的输入**预填**
    /// 其中：效果器填上游信号，音源（[`PluginKind::Instrument`]）填静音。
    ///
    /// 插件就地读并写同一块：不得改变通道数或帧数，也不得分配内存。
    /// 音源可以忽略初始内容直接覆写。
    ///
    /// [`PluginKind::Instrument`]: crate::PluginKind::Instrument
    fn process(&mut self, ctx: &ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent]);
    /// 序列化状态。
    fn save_state(&self) -> Vec<u8>;
    /// 反序列化状态。
    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GainPlugin {
        gain: f32,
    }

    impl Plugin for GainPlugin {
        fn descriptor(&self) -> crate::PluginDescriptor {
            crate::PluginDescriptor {
                id: "test.gain".into(),
                name: "Gain".into(),
                kind: crate::PluginKind::Effect,
                io: crate::IoLayout { inputs: 2, outputs: 2 },
            }
        }
        fn params(&self) -> Vec<crate::ParamInfo> {
            vec![crate::ParamInfo {
                id: "gain".into(),
                name: "Gain".into(),
                min: 0.0,
                max: 1.0,
                default: 1.0,
                automatable: true,
            }]
        }
        fn get_param(&self, id: &crate::ParamId) -> Option<f32> {
            (id.0 == "gain").then_some(self.gain)
        }
        fn set_param(&mut self, id: &crate::ParamId, value: f32) {
            if id.0 == "gain" {
                self.gain = value;
            }
        }
        fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
        fn reset(&mut self) {}
        fn process(
            &mut self,
            _ctx: &ProcessContext,
            audio: &mut [&mut [f32]],
            _events: &[crate::TimedMidiEvent],
        ) {
            for ch in audio.iter_mut() {
                for s in ch.iter_mut() {
                    *s *= self.gain;
                }
            }
        }
        fn save_state(&self) -> Vec<u8> {
            self.gain.to_le_bytes().to_vec()
        }
        fn load_state(&mut self, state: &[u8]) -> Result<(), crate::PluginError> {
            if state.len() < 4 {
                return Err(crate::PluginError::StateTooShort { expected: 4, got: state.len() });
            }
            self.gain = f32::from_le_bytes(state[..4].try_into().unwrap());
            Ok(())
        }
    }

    #[test]
    fn effect_processes_prefilled_input_in_place() {
        let mut p = GainPlugin { gain: 0.5 };
        p.prepare(48_000.0, 2);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 2,
            tempo: 120.0,
            playing: true,
        };
        // The host pre-fills the planes with the incoming signal.
        let mut left = [1.0f32, -2.0];
        let mut right = [0.5f32, 0.25];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert_eq!(left, [0.5, -1.0]);
        assert_eq!(right, [0.25, 0.125]);

        let state = p.save_state();
        let mut q = GainPlugin { gain: 0.0 };
        q.load_state(&state).unwrap();
        assert_eq!(q.gain, 0.5);

        assert!(q.load_state(&[0u8; 2]).is_err());
    }
}
