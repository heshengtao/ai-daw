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
    /// 处理一个音频块。`out` 为平面缓冲（通道 × 帧）。
    fn process(&mut self, ctx: &ProcessContext, out: &mut [&mut [f32]], events: &[TimedMidiEvent]);
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
            ctx: &ProcessContext,
            out: &mut [&mut [f32]],
            _events: &[crate::TimedMidiEvent],
        ) {
            for ch in out.iter_mut() {
                for (i, s) in ch.iter_mut().enumerate() {
                    *s = (i as f32 + ctx.block_size as f32) * self.gain;
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
    fn plugin_process_writes_output_and_state_roundtrips() {
        let mut p = GainPlugin { gain: 0.5 };
        p.prepare(48_000.0, 4);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 4,
            tempo: 120.0,
            playing: true,
        };
        let mut left = [0.0f32; 4];
        let mut right = [0.0f32; 4];
        p.process(&ctx, &mut [&mut left, &mut right], &[]);
        assert_eq!(left[0], 2.0);

        let state = p.save_state();
        let mut q = GainPlugin { gain: 0.0 };
        q.load_state(&state).unwrap();
        assert_eq!(q.gain, 0.5);

        assert!(q.load_state(&[0u8; 2]).is_err());
    }
}
