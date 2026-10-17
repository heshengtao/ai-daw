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
///
/// # 线程契约
///
/// 宿主在这些线程上调用本 trait 的方法，实现方（尤其是外部插件适配器）必须照此安排自己
/// 的跨线程工作——CLAP 这类格式会**真的去核对**调用线程，违规时轻则刷警告、重则卡死宿主：
///
/// - [`prepare`](Plugin::prepare)：可能在**任意线程**被调用（宿主构建 `Player` 时）。
///   需要主线程的实现（如 CLAP 的 `activate`）必须在这里自行派发。
/// - [`reset`](Plugin::reset) / [`process`](Plugin::process)：在**音频线程**上调用。
///   音频线程专属的工作（如 CLAP 的 `start_processing` / `clap_plugin.reset()`）要放在这里，
///   而且实时路径上**不能加锁、不能分配、不能打日志**。
/// - 构造与 `Drop`、[`save_state`](Plugin::save_state) / [`load_state`](Plugin::load_state)：
///   在控制侧线程上调用，可以慢、可以加锁。
///
/// 因为 `start_processing` 只能在音频线程做，而 `activate` 只能在主线程做，实现方应当把
/// 「激活」与「开始处理」拆成两步：前者在 `prepare` 里派发到主线程，后者推迟到第一次
/// [`process`](Plugin::process)。
pub trait Plugin: Send {
    /// 描述符。
    fn descriptor(&self) -> PluginDescriptor;
    /// 参数列表。
    fn params(&self) -> Vec<ParamInfo>;
    /// 读取参数当前值。
    fn get_param(&self, id: &ParamId) -> Option<f32>;
    /// 设置参数值（实现方自行 clamp）。
    fn set_param(&mut self, id: &ParamId, value: f32);
    /// 采样率/块大小变化时调用（可能在任意线程；见上文的线程契约）。
    fn prepare(&mut self, sample_rate: f64, block_size: usize);
    /// 清空内部状态（延迟线、包络、声部）。在音频线程上调用。
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
    /// 抽干自上次调用以来**插件自身**产生的参数变化（例如用户在插件界面上拧了旋钮）。
    ///
    /// 纯 DSP 插件不产生内部参数变化，默认返回空即可。宿主在音频线程的块之间调用它，
    /// 把结果回推给界面与撤销栈——这是「插件界面 → 工程模型」的回流通道。
    fn take_param_changes(&mut self) -> Vec<(ParamId, f32)> {
        Vec::new()
    }

    /// 在**丢弃宿主之前**把音频处理停掉（默认无需处理）。
    ///
    /// 宿主重建播放器时会把实例「搬」到新播放器复用（保住插件状态与已打开的界面窗口）。
    /// CLAP 要求 `stop_processing` 只在音频线程调用，所以由**回调**在交出实例前调它；
    /// 纯 DSP 插件与 VST3 不需要（默认空操作）。
    fn release_processing(&mut self) {}

    /// 这个实例是不是「插件缺失 / 装不上」时的**静音占位**（默认不是）。
    ///
    /// 宿主重建播放器时会复用 plugin id 一致的旧实例（保住插件内部状态与已打开的原生界面
    /// 窗口）。但占位不是真插件：启动时会话先带着**空目录**恢复一次，届时外部插件只能建成
    /// 占位；目录扫描完成后再重建时若复用这个占位，真插件就永远建不出来——实例没建出来，
    /// 「槽位 → 编辑器句柄」也就登记不上，界面点「打开」只会报「这个槽位没有可打开的插件界面」。
    /// 所以占位**不参与复用**，每次按当前注册表现建。
    fn is_placeholder(&self) -> bool {
        false
    }
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
                capabilities: crate::PluginCapabilities::default(),
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
