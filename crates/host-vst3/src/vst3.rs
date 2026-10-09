//! VST3 后端：用 `vst3-host` 托管 VST3 实例，适配成本项目的 [`Plugin`] trait。
//!
//! 为什么不用 truce-rack 的 VST3 后端：它把音频总线排列写死成「1 立体声输入 + 1 立体声输出」
//! 并丢弃 `setBusArrangements` 的失败，且没有实现 `IComponentHandler`——实测没有音频输入总线的
//! 音源（如 KR-106）会静音。`vst3-host` 能正确托管这类音源（实测峰值 1.21 vs 0）。

use std::sync::{Arc, Mutex};

use daw_party_plugin_api::{
    IoLayout, MidiEvent as HostEvent, ParamId, ParamInfo, Plugin, PluginCapabilities,
    PluginDescriptor, PluginError, PluginKind, ProcessContext, TimedMidiEvent,
};
use vst3_host::midi::{MidiChannel, MidiEvent as Vst3Midi};
use vst3_host::plugin::PluginInfo;
use vst3_host::AudioBuffers;

/// 引擎侧恒为立体声（`Player` 每轨固定送左右两条平面缓冲）。
const ENGINE_CHANNELS: usize = 2;

/// 编辑器句柄：与音频线程**共享同一个实例**。
///
/// 原生编辑器必须附着在正在发声的那个实例上，所以这里用 `Arc<Mutex<..>>` 把所有权拆开：
/// 音频线程按块加锁处理，界面线程按需加锁开窗/查询。
pub type Vst3EditorHandle = Arc<Mutex<vst3_host::Plugin>>;

struct ParamMeta {
    rack_id: u32,
    id: ParamId,
    name: String,
    /// 插件自报的**归一化**默认值。
    ///
    /// 必须传下去：界面上的「恢复默认参数」就是把它整组写回。早先这里被硬写成 0.0，
    /// 于是对 VST3 点一下「恢复默认」＝把**所有**参数清零，乐器当场哑掉
    /// （CLAP 那边一直报的是真实默认值，所以只有 VST3 出事）。
    default: f32,
    automatable: bool,
}

/// 一个被托管的 VST3 实例。
pub struct Vst3HostedPlugin {
    inner: Vst3EditorHandle,
    id: String,
    name: String,
    kind: PluginKind,
    io: IoLayout,
    capabilities: PluginCapabilities,
    params: Vec<ParamMeta>,
    sample_rate: f64,
    block_size: usize,
    started: bool,
    buffers: AudioBuffers,
    pending_param_changes: Vec<(ParamId, f32)>,
}

impl Vst3HostedPlugin {
    /// 从已加载的 `vst3-host` 插件构造适配器。
    pub fn from_loaded(id: String, plugin: vst3_host::Plugin) -> Self {
        let info = plugin.info().clone();
        // VST3 的类别字符串形如 `"Instrument|Synth"` / `"Fx"`。
        let kind = if info.category.contains("Instrument") {
            PluginKind::Instrument
        } else {
            PluginKind::Effect
        };
        let inputs = if kind == PluginKind::Effect { ENGINE_CHANNELS } else { 0 };
        let io = IoLayout { inputs: inputs as u16, outputs: ENGINE_CHANNELS as u16 };
        let capabilities = PluginCapabilities { per_note_volume: false, has_editor: info.has_gui };

        let params = plugin
            .get_parameters()
            .unwrap_or_default()
            .into_iter()
            .map(|p| ParamMeta {
                rack_id: p.id,
                id: ParamId(p.id.to_string()),
                name: p.name,
                default: p.default as f32,
                automatable: p.can_automate,
            })
            .collect();

        Vst3HostedPlugin {
            inner: Arc::new(Mutex::new(plugin)),
            id,
            name: info.name,
            kind,
            io,
            capabilities,
            params,
            sample_rate: 48_000.0,
            block_size: 512,
            started: false,
            buffers: AudioBuffers::new(inputs, ENGINE_CHANNELS, 512, 48_000.0),
            pending_param_changes: Vec::new(),
        }
    }

    /// 给界面用的编辑器句柄（与音频线程共享同一实例）。
    pub fn editor_handle(&self) -> Vst3EditorHandle {
        Arc::clone(&self.inner)
    }

    /// 是否自带原生编辑器。
    pub fn has_editor(&self) -> bool {
        self.inner.lock().map(|p| p.has_editor()).unwrap_or(false)
    }

    /// 原生编辑器尺寸（尚无视图时返回 `None`）。
    pub fn editor_size(&mut self) -> Option<(u32, u32)> {
        let plugin = self.inner.lock().ok()?;
        plugin
            .get_editor_size()
            .ok()
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(w, h)| (w as u32, h as u32))
    }
}

impl Plugin for Vst3HostedPlugin {
    fn descriptor(&self) -> PluginDescriptor {
        PluginDescriptor {
            id: self.id.clone(),
            name: self.name.clone(),
            kind: self.kind,
            io: self.io,
            capabilities: self.capabilities,
        }
    }

    fn params(&self) -> Vec<ParamInfo> {
        // vst3-host 的参数一律是归一化值（0..=1）。
        self.params
            .iter()
            .map(|p| ParamInfo {
                id: p.id.clone(),
                name: p.name.clone(),
                min: 0.0,
                max: 1.0,
                default: p.default,
                automatable: p.automatable,
            })
            .collect()
    }

    fn get_param(&self, id: &ParamId) -> Option<f32> {
        let meta = self.params.iter().find(|p| &p.id == id)?;
        let plugin = self.inner.lock().ok()?;
        plugin.get_parameter(meta.rack_id).ok().map(|v| v as f32)
    }

    fn set_param(&mut self, id: &ParamId, value: f32) {
        if let Some(meta) = self.params.iter().find(|p| &p.id == id) {
            if let Ok(mut plugin) = self.inner.lock() {
                let _ = plugin.set_parameter(meta.rack_id, value as f64);
            }
        }
    }

    fn prepare(&mut self, sample_rate: f64, block_size: usize) {
        self.sample_rate = sample_rate;
        self.block_size = block_size.max(1);
        let inputs = self.io.inputs as usize;
        self.buffers = AudioBuffers::new(inputs, ENGINE_CHANNELS, self.block_size, sample_rate);
        self.started = match self.inner.lock() {
            Ok(mut plugin) => plugin.start_processing().is_ok(),
            Err(_) => false,
        };
    }

    fn reset(&mut self) {
        // 只做「丢弃悬挂声部」这一件事：`midi_panic()` 就是全音符关（all-notes-off）。
        //
        // **绝不能在这里 stop/start_processing**，两个原因都是实测踩过的：
        // - `start_processing()` 一旦失败，`started` 就永久为 false，这个乐器此后**再不出声**
        //   （错误还被 `let _` 吞掉）。表现出来正是「按一下播放有声音、停了再按就没声音了」。
        // - 它要拿实例锁，而**开插件界面时主线程会长时间持锁**；实时线程在这儿干等会卡住。
        //   隔壁 CLAP 后端早先就是这套写法而中招，`clap.rs::reset` 里留着那段记录。
        //
        // 所以用 `try_lock`：拿不到就这一块先不清（下一块再来），绝不阻塞实时线程。
        let Ok(mut plugin) = self.inner.try_lock() else { return };
        let _ = plugin.midi_panic();
    }

    fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], events: &[TimedMidiEvent]) {
        let frames = audio.first().map_or(0, |c| c.len());
        if !self.started || frames == 0 || audio.len() < ENGINE_CHANNELS {
            return;
        }

        // 界面线程可能正持有实例（开原生窗口、查询尺寸）：这一块先静音，下一块继续。
        let Ok(mut plugin) = self.inner.try_lock() else { return };

        // 先把本块事件排进队列（`send_midi_event_at` 作用于「下一个处理块」）。
        let last = frames.saturating_sub(1);
        for e in events {
            let offset = (e.frame as usize).min(last);
            let event = match e.event {
                HostEvent::NoteOn { note, velocity } => {
                    Vst3Midi::NoteOn { channel: MidiChannel::Ch1, note, velocity }
                }
                HostEvent::NoteOff { note } => {
                    Vst3Midi::NoteOff { channel: MidiChannel::Ch1, note, velocity: 0 }
                }
                // 折叠路径已在 `Player` 里完成；本适配器声明不支持 per-note 音量。
                HostEvent::NoteVolume { .. } | HostEvent::ControlChange { .. } => continue,
            };
            let _ = plugin.send_midi_event_at(event, offset as i32);
        }

        // 效果器：把上游信号拷进输入平面。
        let inputs = self.io.inputs as usize;
        for i in 0..inputs.min(self.buffers.inputs.len()) {
            let n = frames.min(self.buffers.inputs[i].len());
            self.buffers.inputs[i][..n].copy_from_slice(&audio[i][..n]);
        }

        // 块大小可能小于 prepare 时的上界——按本块帧数直接处理。
        if self.buffers.block_size != frames || self.buffers.outputs[0].len() != frames {
            self.buffers.block_size = frames;
            for ch in self.buffers.inputs.iter_mut() {
                ch.resize(frames, 0.0);
            }
            for ch in self.buffers.outputs.iter_mut() {
                ch.resize(frames, 0.0);
            }
        }
        for ch in self.buffers.outputs.iter_mut() {
            ch.fill(0.0);
        }

        if plugin.process_audio(&mut self.buffers).is_err() {
            for ch in audio.iter_mut() {
                ch[..frames].fill(0.0);
            }
            return;
        }

        for i in 0..ENGINE_CHANNELS {
            let n = frames.min(self.buffers.outputs[i].len());
            audio[i][..n].copy_from_slice(&self.buffers.outputs[i][..n]);
        }

        // 收集插件界面产生的参数变化。
        self.pending_param_changes.clear();
        for (raw_id, value) in plugin.get_parameter_changes() {
            if let Some(meta) = self.params.iter().find(|p| p.rack_id == raw_id) {
                self.pending_param_changes.push((meta.id.clone(), value as f32));
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        match self.inner.lock() {
            Ok(plugin) => plugin.save_state().unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError> {
        let mut plugin = self
            .inner
            .lock()
            .map_err(|_| PluginError::InvalidState("插件实例锁中毒".into()))?;
        plugin
            .load_state(state)
            .map_err(|e| PluginError::InvalidState(e.to_string()))
    }

    fn take_param_changes(&mut self) -> Vec<(ParamId, f32)> {
        std::mem::take(&mut self.pending_param_changes)
    }
}

/// VST3 元数据 → 本项目的分类。
pub fn kind_of(info: &PluginInfo) -> PluginKind {
    if info.category.contains("Instrument") {
        PluginKind::Instrument
    } else {
        PluginKind::Effect
    }
}
