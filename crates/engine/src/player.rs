use std::collections::HashMap;

use daw_party_plugin_api::{MidiEvent, ParamId, Plugin, PluginKind, ProcessContext, TimedMidiEvent};

use crate::model::MixerChannel;
use crate::registry::PluginRegistry;
use crate::schedule::{ScheduledAudioClip, ScheduledEvents, ScheduledMidi};
use crate::transport::TransportCommand;

/// dB → 线性增益。
pub(crate) fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// 声像 → (左, 右) 增益。
///
/// 用**线性 balance**：居中 = 单位增益，向一侧偏则衰减另一侧（±1 时该侧为 0）。
///
/// 刻意**不用**等功率（`sqrt((1∓pan)/2)`）：引擎内部恒为立体声，等功率律会让「推子归零、
/// 声像居中」的信号凭空掉 3 dB；轨道与 master 各套一次就是 **−6 dB**——导入一段满幅素材
/// 会明显变轻。线性 balance 保证「0 dB 推子 + 居中声像 + 居中 master」= 单位增益。
fn pan_gains(pan: f32) -> (f32, f32) {
    let pan = MixerChannel::normalize_pan(pan);
    ((1.0 - pan).clamp(0.0, 1.0), (1.0 + pan).clamp(0.0, 1.0))
}

/// 每块最多送几个试听音。
///
/// 试听音走的是和参数一样的实时通道，队列里攒不下就直接丢——宁可丢一个试听音，
/// 也不能在音频线程里给 `Vec` 扩容。容量在构造时预留好。
const AUDITION_CAPACITY: usize = 16;

/// 把「单音音量」折进力度：**仅**用于不支持 per-note 音量的插件。
///
/// 非折叠插件（如内置合成器、将来支持 Note Expression 的宿主插件）会独立收到 `NoteVolume`；
/// 其余插件只能接受「力度乘法合成」这一兼容表达，否则该音会丢掉音量信息。
/// 折叠在构造播放器时一次完成——`process_block` 里不做任何分配或变换。
fn fold_note_volume(events: &mut Vec<ScheduledMidi>) {
    let mut pending: HashMap<u8, usize> = HashMap::new();
    let mut out: Vec<ScheduledMidi> = Vec::with_capacity(events.len());
    for e in events.drain(..) {
        match e.event {
            MidiEvent::NoteOn { note, .. } => {
                pending.insert(note, out.len());
                out.push(e);
            }
            MidiEvent::NoteOff { note } => {
                pending.remove(&note);
                out.push(e);
            }
            MidiEvent::NoteVolume { note, value } => {
                if let Some(&on) = pending.get(&note) {
                    if let MidiEvent::NoteOn { velocity, .. } = out[on].event {
                        let folded = if velocity == 0 || value == 0 {
                            0
                        } else {
                            (velocity as u32 * value as u32 / 127).clamp(1, 127) as u8
                        };
                        out[on].event = MidiEvent::NoteOn { note, velocity: folded };
                    }
                }
                // 折叠后不再转发 NoteVolume。
            }
            MidiEvent::ControlChange { .. } => out.push(e),
        }
    }
    *events = out;
}

/// 一条音频片段的运行期状态。
struct AudioPlayback {
    clip: ScheduledAudioClip,
    /// 素材内的当前帧位置。用 `f64` 按比值累加，避免长片段上的整数漂移。
    cursor: f64,
}

/// 片段在 `position` 帧时，素材内的播放位置（源帧）。
///
/// 语义：素材从 `offset_frames` 起**铺满**整段跨度，因此每过一个输出帧前进
/// `source_window_frames / span_frames` 个源帧。缩放片段或改工程速度都会改变这个比值，
/// 于是音频被按比例重采样（**会变调**——v1 不做保音高变速）。
fn audio_cursor(clip: &ScheduledAudioClip, position: u64) -> f64 {
    let offset = clip.offset_frames as f64;
    let span = clip.end_frame.saturating_sub(clip.start_frame);
    if span == 0 || position <= clip.start_frame {
        return offset;
    }
    if position >= clip.end_frame {
        return offset + clip.source_window_frames as f64;
    }
    let ratio = clip.source_window_frames as f64 / span as f64;
    offset + (position - clip.start_frame) as f64 * ratio
}

/// 线性插值取一帧（交错 f32；`channels` 为 1 或 2）。
///
/// 实时安全：纯乘加，不分配。越界读按 0 处理——素材末尾自然衰减，而不是爆音。
#[inline]
fn lerp_frame(samples: &[f32], channels: usize, pos: f64) -> (f32, f32) {
    let base = pos.floor();
    let t = (pos - base) as f32;
    let i0 = base as usize;
    let read = |i: usize| -> (f32, f32) {
        let at = i * channels;
        if channels <= 1 {
            let v = samples.get(at).copied().unwrap_or(0.0);
            (v, v)
        } else {
            (
                samples.get(at).copied().unwrap_or(0.0),
                samples.get(at + 1).copied().unwrap_or(0.0),
            )
        }
    };
    let (l0, r0) = read(i0);
    let (l1, r1) = read(i0 + 1);
    (l0 + (l1 - l0) * t, r0 + (r1 - r0) * t)
}

/// 把一条音频片段按帧**相加**进 `left/right`。
///
/// 调用点在「乐器之后、效果链之前」：于是音频片段与 MIDI 一样经过轨道效果链、轨道
/// 增益/声像与 master 增益。实时安全：只做乘加与游标推进，不分配。
fn mix_audio_clip(
    playback: &mut AudioPlayback,
    block_start: u64,
    frames: usize,
    left: &mut [f32],
    right: &mut [f32],
) {
    let start_frame = playback.clip.start_frame;
    let end_frame = playback.clip.end_frame;
    let span = end_frame.saturating_sub(start_frame);
    if span == 0 {
        return;
    }
    let ratio = playback.clip.source_window_frames as f64 / span as f64;
    let source_frames = playback.clip.audio.frames() as f64;
    let channels = playback.clip.audio.channels;
    let samples: &[f32] = &playback.clip.audio.samples;
    let mut cursor = playback.cursor;
    for i in 0..frames {
        let frame = block_start + i as u64;
        if frame < start_frame {
            continue;
        }
        if frame >= end_frame || cursor >= source_frames {
            break; // 片段结束，或素材放完（v1 不循环）
        }
        let (l, r) = lerp_frame(samples, channels, cursor);
        left[i] += l;
        right[i] += r;
        cursor += ratio;
    }
    playback.cursor = cursor;
}

struct TrackVoice {
    track_id: crate::model::TrackId,
    /// 乐器。轨道没挂乐器（或插件缺失）时为空——该轨只剩音频片段，仍然要出声。
    plugin: Option<Box<dyn Plugin>>,
    /// 效果链（乐器输出 → effects[0] → …）。构造时定长，`process_block` 内不增删。
    effects: Vec<Box<dyn Plugin>>,
    events: Vec<ScheduledMidi>,
    cursor: usize,
    /// 音频片段（构造时定长；播放时只推进游标）。
    audio: Vec<AudioPlayback>,
    /// 混音参数（可实时更新，不必重建事件表）。
    gain_db: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    /// 由 `refresh_mix` 推出的「本轨当前是否发声」。
    audible: bool,
    left_gain: f32,
    right_gain: f32,
}

/// 实时安全的逐块播放器：构造后 `process_block` 不再分配内存。
///
/// 混音参数（增益 / 声像 / 静音 / 独奏）保存在播放器内部并可通过
/// [`TransportCommand::SetTrackMix`] 实时更新——这是拖动推子不会打断音频的原因。
pub struct Player {
    tracks: Vec<TrackVoice>,
    sample_rate: f64,
    tempo: f64,
    /// 速度分段落点（帧 → BPM，升序）；逐块取当前速度用。
    tempo_points: Vec<(u64, f64)>,
    /// 当前所处速度分段的游标。
    tempo_cursor: usize,
    position: u64,
    end_frame: u64,
    playing: bool,
    /// 循环播放开关。`Player` 自己不改位置；「到结尾回到起点」由宿主执行。
    looping: bool,
    master_gain_db: f32,
    master_pan: f32,
    master_left: f32,
    master_right: f32,
    scratch_left: Vec<f32>,
    scratch_right: Vec<f32>,
    scratch_events: Vec<TimedMidiEvent>,
    /// 待送达的试听音（界面上的键盘按下的音），下一块开头送进对应轨道。
    audition: Vec<(crate::model::TrackId, MidiEvent)>,
}

impl Player {
    /// 界面上的键盘按下的试听音：下一块的第 0 帧直接送给那条轨道的乐器。
    ///
    /// 走的是和参数同一条实时通道。队列满就丢——试听音不值得让音频线程扩容。
    fn push_audition(&mut self, track: crate::model::TrackId, note: u8, velocity: u8, on: bool) {
        if self.audition.len() >= AUDITION_CAPACITY {
            return;
        }
        let event = if on {
            MidiEvent::NoteOn { note, velocity }
        } else {
            MidiEvent::NoteOff { note }
        };
        self.audition.push((track, event));
    }

    /// `max_block` 为回调可能请求的最大帧数；所有 scratch 缓冲按此预留，避免回调内分配。
    ///
    /// `scratch_events` 的上界不是帧数而是「单轨事件总数」：一个块最多只能取走某轨尚未
    /// 消费的事件，因此事件数不会超过该轨事件表的总长度，也就不可能超过各轨长度的最大值。
    pub fn new(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        sample_rate: f64,
        max_block: usize,
    ) -> Self {
        let tempo = schedule.tempo;
        let tempo_points = schedule.tempo_points.clone();
        let end_frame = schedule.end_frame;
        let master = schedule.master;
        // 单块内某轨最多发出的事件数 = 该轨事件总数上界（游标单调前进，每块只消费一次）。
        let max_track_events = schedule.tracks.iter().map(|t| t.events.len()).max().unwrap_or(0);
        let mut tracks = Vec::with_capacity(schedule.tracks.len());
        for st in &schedule.tracks {
            // 乐器是**可选**的：没挂乐器（或插件缺失）的轨道仍然存在，只为它的音频片段出声。
            let instrument = st.instrument.as_ref();
            let mut plugin: Option<Box<dyn Plugin>> =
                instrument.and_then(|i| registry.create(&i.plugin_id, st.track_id));
            // 不支持 per-note 音量的插件：把音量折进力度，保住旧听感。
            let mut events = st.events.clone();
            if let (Some(p), Some(instance)) = (plugin.as_mut(), instrument) {
                p.prepare(sample_rate, max_block);
                for (id, value) in &instance.params {
                    p.set_param(&id.clone().into(), *value);
                }
                if !p.descriptor().capabilities.per_note_volume {
                    fold_note_volume(&mut events);
                }
            }
            // 效果链：只收「效果」类插件。缺失或类型不符的条目直接跳过——
            // 让整条链静音比「少一个效果」糟糕得多。
            let mut effects = Vec::with_capacity(st.effects.len());
            for fx in &st.effects {
                let Some(mut fx_plugin) = registry.create(&fx.plugin_id, st.track_id) else { continue };
                if fx_plugin.descriptor().kind != PluginKind::Effect {
                    continue;
                }
                fx_plugin.prepare(sample_rate, max_block);
                for (id, value) in &fx.params {
                    fx_plugin.set_param(&id.clone().into(), *value);
                }
                effects.push(fx_plugin);
            }
            // 音频片段：素材已在调度前加载好，这里只克隆 `Arc` 并定出初始游标。
            let audio: Vec<AudioPlayback> = st
                .audio_clips
                .iter()
                .map(|clip| AudioPlayback { clip: clip.clone(), cursor: audio_cursor(clip, 0) })
                .collect();
            tracks.push(TrackVoice {
                track_id: st.track_id,
                plugin,
                effects,
                events,
                cursor: 0,
                audio,
                gain_db: st.mixer.gain_db,
                pan: st.mixer.pan,
                mute: st.mixer.mute,
                solo: st.mixer.solo,
                // 初值故意留 false：`refresh_mix` 首次调用时若本轨应发声也不会误触发 reset。
                audible: false,
                left_gain: 0.0,
                right_gain: 0.0,
            });
        }
        let mut player = Player {
            tracks,
            sample_rate,
            tempo,
            tempo_points,
            tempo_cursor: 0,
            position: 0,
            end_frame,
            playing: false,
            looping: false,
            master_gain_db: master.gain_db,
            master_pan: master.pan,
            master_left: 1.0,
            master_right: 1.0,
            scratch_left: vec![0.0; max_block],
            scratch_right: vec![0.0; max_block],
            scratch_events: Vec::with_capacity(max_track_events.max(64) + AUDITION_CAPACITY),
            audition: Vec::with_capacity(AUDITION_CAPACITY),
        };
        player.refresh_mix();
        player
    }

    /// 当前是否正在播放。
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// 当前是否开启循环播放。
    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// 当前位置（自播放起点起的帧数）。
    pub fn position_frames(&self) -> u64 {
        self.position
    }

    /// 工程内容结束帧（自播放起点起）；0 表示未设置。
    ///
    /// `Player` 自身不会在到达此帧时停止 —— 是否「播放到结尾即停」由宿主（如实时
    /// 回调或离线渲染循环）决定。离线渲染因此可以越过工程正文继续渲染音符释放/衰减
    /// 的尾音，避免结尾处的爆音。
    pub fn end_frame(&self) -> u64 {
        self.end_frame
    }

    /// 定位到指定帧：重置所有插件并重算事件游标（已在播放中的音符不会被重新触发）。
    pub fn seek(&mut self, frame: u64) {
        self.position = frame;
        self.tempo_cursor = self
            .tempo_points
            .partition_point(|(f, _)| *f <= frame)
            .saturating_sub(1);
        for track in self.tracks.iter_mut() {
            track.cursor = track.events.partition_point(|e| e.frame < frame);
            if let Some(plugin) = track.plugin.as_mut() {
                plugin.reset();
            }
            // 音频片段的游标同样要跟着播放头重算，否则定位后片段会错位。
            for playback in track.audio.iter_mut() {
                let cursor = audio_cursor(&playback.clip, frame);
                playback.cursor = cursor;
            }
        }
    }

    /// 重新计算各轨的可听性与左右增益，并更新总输出。
    ///
    /// 独奏语义：只要有任意一轨独奏，其余轨道全部静音。可听性发生变化时重置插件，
    /// 避免解除静音后残留悬挂的声部（音符的 NoteOff 可能已被丢弃）。
    ///
    /// 实时安全：只做算术与字段写入，不分配内存。
    fn refresh_mix(&mut self) {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        for track in self.tracks.iter_mut() {
            let want = !track.mute && (!any_solo || track.solo);
            if track.audible && !want {
                if let Some(plugin) = track.plugin.as_mut() {
                    plugin.reset();
                }
            }
            track.audible = want;
            let gain = db_to_linear(track.gain_db);
            let (l, r) = pan_gains(track.pan);
            track.left_gain = gain * l;
            track.right_gain = gain * r;
        }
        let master = db_to_linear(self.master_gain_db);
        let (l, r) = pan_gains(self.master_pan);
        self.master_left = master * l;
        self.master_right = master * r;
    }

    /// 让所有音源立刻停止发声（等价于 all-notes-off）。
    ///
    /// 重置乐器**与效果链**（延迟/混响不清的话同样会继续响），并丢掉还没送达的试听音。
    /// 插件实例本身**不销毁**：界面上的键盘、插件自己的界面预览都还要靠它出声。
    ///
    /// 这里也是「暂停即静音」的实现：不这么做的话，暂停时那个音的 NoteOff 永远送不到，
    /// 插件会一直挂着它。
    fn silence(&mut self) {
        for track in self.tracks.iter_mut() {
            if let Some(plugin) = track.plugin.as_mut() {
                plugin.reset();
            }
            for fx in track.effects.iter_mut() {
                fx.reset();
            }
        }
        self.audition.clear();
    }

    /// 应用一条传输命令。
    pub fn apply_command(&mut self, command: TransportCommand) {
        match command {
            TransportCommand::Play => {
                // 已经停在结尾：再按播放应当从头重播，而不是「按了没反应」。
                // 宿主在播到 `end_frame` 时会暂停（位置留在结尾），这条判断就是它的配对。
                if self.end_frame > 0 && self.position >= self.end_frame {
                    self.seek(0);
                }
                self.playing = true;
            }
            // 暂停 = **静音**。只把 `playing` 置 false 是不够的：长音的 NoteOff 再也送不到，
            // 插件会一直挂着那个音——听上去就是「暂停了还在响」。所以顺手把音收掉。
            TransportCommand::Pause => {
                self.playing = false;
                self.silence();
            }
            TransportCommand::Stop => {
                self.playing = false;
                self.seek(0);
                self.silence();
            }
            TransportCommand::Seek { frame } => self.seek(frame),
            TransportCommand::SetTrackMix { track, gain_db, pan, mute, solo } => {
                let mut touched = false;
                for voice in self.tracks.iter_mut() {
                    if voice.track_id == track {
                        voice.gain_db = MixerChannel::normalize_gain_db(gain_db);
                        voice.pan = MixerChannel::normalize_pan(pan);
                        voice.mute = mute;
                        voice.solo = solo;
                        touched = true;
                    }
                }
                if touched {
                    self.refresh_mix();
                }
            }
            TransportCommand::SetMasterGain { gain_db } => {
                self.master_gain_db = MixerChannel::normalize_gain_db(gain_db);
                self.refresh_mix();
            }
            TransportCommand::Audition { track, note, velocity, on } => {
                self.push_audition(track, note, velocity, on);
            }
            TransportCommand::SetLoop(on) => self.looping = on,
            TransportCommand::SetParam { track, effect, param, value } => {
                // 非有限值会让插件内部出现 NaN，直接拒绝。
                if !value.is_finite() {
                    return;
                }
                // 参数名已在控制侧分配好，这里只做移动——实时路径不分配内存。
                let id = ParamId(param);
                for voice in self.tracks.iter_mut().filter(|v| v.track_id == track) {
                    match effect {
                        None => {
                            if let Some(plugin) = voice.plugin.as_mut() {
                                plugin.set_param(&id, value);
                            }
                        }
                        Some(index) => {
                            if let Some(fx) = voice.effects.get_mut(index) {
                                fx.set_param(&id, value);
                            }
                        }
                    }
                }
            }
        }
    }

    /// 从无锁队列取出所有待处理传输命令并应用。
    pub fn drain_commands(&mut self, receiver: &mut crate::command_queue::CommandReceiver) {
        receiver.drain(|command| self.apply_command(command));
    }

    /// 填充一个交错立体声块（`out.len() >= frames * 2`）。实时安全：不分配内存。
    ///
    /// 纯 DSP：只渲染 `frames` 帧并推进 `position`，绝不自行改变 `playing`（到达
    /// `end_frame` 也不会停止）；「播到结尾即停」是宿主的策略。
    ///
    /// **停止时也照样跑插件**（只是不发时间线上的事件、不推进位置）：插件界面上的虚拟键盘、
    /// 试听按钮、以及效果器的尾音都在插件**内部**发声，宿主不调用 `process` 就什么也听不到。
    /// 代价是空闲时也要占一点 CPU——这是「点插件键盘立刻有声音」换来的。
    pub fn process_block(&mut self, out: &mut [f32], frames: usize) {
        debug_assert!(out.len() >= frames * 2);
        let out = &mut out[..frames * 2];
        out.fill(0.0);

        let playing = self.playing;
        let block_start = self.position;
        let block_end = block_start + frames as u64;

        while self.tempo_cursor + 1 < self.tempo_points.len()
            && self.tempo_points[self.tempo_cursor + 1].0 <= block_start
        {
            self.tempo_cursor += 1;
        }
        let tempo = self
            .tempo_points
            .get(self.tempo_cursor)
            .map(|(_, bpm)| *bpm)
            .unwrap_or(self.tempo);

        let ctx = ProcessContext {
            sample_rate: self.sample_rate,
            block_size: frames,
            tempo,
            playing,
        };

        let Player { tracks, scratch_left, scratch_right, scratch_events, audition, .. } = self;

        for track in tracks.iter_mut() {
            // 即便本轨当前不可听，也要推进事件游标：否则解除静音时会把积压的历史
            // 音符一次性补发出去。停止时游标原地不动，恢复播放才不会跳音。
            scratch_events.clear();
            if playing {
                while let Some(ev) = track.events.get(track.cursor).copied() {
                    if ev.frame >= block_end {
                        break;
                    }
                    if ev.frame >= block_start {
                        scratch_events.push(TimedMidiEvent {
                            frame: (ev.frame - block_start) as u32,
                            event: ev.event,
                        });
                    }
                    track.cursor += 1;
                }
            }

            // 试听音（界面上的键盘）与传输无关：停了也能弹。
            for (id, event) in audition.iter() {
                if *id == track.track_id {
                    scratch_events.push(TimedMidiEvent { frame: 0, event: *event });
                }
            }

            if !track.audible {
                continue;
            }

            if scratch_left.len() < frames {
                scratch_left.resize(frames, 0.0);
            }
            if scratch_right.len() < frames {
                scratch_right.resize(frames, 0.0);
            }
            let left = &mut scratch_left[..frames];
            let right = &mut scratch_right[..frames];
            left.fill(0.0);
            right.fill(0.0);

            // 乐器（可选）与音频片段**相加**（缓冲此前已清零），再一起过效果链：
            // 于是音频片段与 MIDI 一样经过轨道效果、轨道增益/声像与 master 增益。
            if let Some(plugin) = track.plugin.as_mut() {
                plugin.process(&ctx, &mut [&mut left[..], &mut right[..]], scratch_events);
            }
            // 音频片段是**时间轴内容**：暂停时不出声（与「插件仍然跑、好让界面键盘与尾音能发声」
            // 不同——音频片段没有这种需求）。
            if playing {
                for playback in track.audio.iter_mut() {
                    mix_audio_clip(playback, block_start, frames, left, right);
                }
            }
            // 效果链：输入就是上一级的输出（就地处理）。效果器不收 MIDI 事件。
            for fx in track.effects.iter_mut() {
                fx.process(&ctx, &mut [&mut left[..], &mut right[..]], &[]);
            }

            for i in 0..frames {
                out[i * 2] += left[i] * track.left_gain;
                out[i * 2 + 1] += right[i] * track.right_gain;
            }
        }

        // 总输出在「所有轨道混合之后」施加。
        let ml = self.master_left;
        let mr = self.master_right;
        for frame in out.chunks_exact_mut(2) {
            frame[0] *= ml;
            frame[1] *= mr;
        }

        // 试听音只在本块送一次：没匹配到任何轨道的也一并丢掉。
        self.audition.clear();

        if playing {
            self.position = block_end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};
    use crate::registry::PluginRegistry;
    use crate::schedule::ScheduledEvents;
    use crate::transport::TransportCommand;
    use daw_party_plugin_api::{IoLayout, ParamId, ParamInfo, PluginDescriptor, PluginError};

    /// 测试用效果器：把信号砍半（就地处理，与引擎契约一致）。
    struct HalfGain;

    impl Plugin for HalfGain {
        fn descriptor(&self) -> PluginDescriptor {
            PluginDescriptor {
                id: "test.half".into(),
                name: "Half".into(),
                kind: PluginKind::Effect,
                io: IoLayout { inputs: 2, outputs: 2 },
                capabilities: Default::default(),
            }
        }
        fn params(&self) -> Vec<ParamInfo> {
            Vec::new()
        }
        fn get_param(&self, _id: &ParamId) -> Option<f32> {
            None
        }
        fn set_param(&mut self, _id: &ParamId, _value: f32) {}
        fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
        fn reset(&mut self) {}
        fn process(&mut self, _ctx: &ProcessContext, audio: &mut [&mut [f32]], _events: &[TimedMidiEvent]) {
            for channel in audio.iter_mut() {
                for sample in channel.iter_mut() {
                    *sample *= 0.5;
                }
            }
        }
        fn save_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn load_state(&mut self, _state: &[u8]) -> Result<(), PluginError> {
            Ok(())
        }
    }

    fn project() -> Project {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: Default::default(),
        });
        p
    }

    fn schedule() -> ScheduledEvents {
        ScheduledEvents::from_project(&project(), 48_000.0)
    }

    fn peak(out: &[f32]) -> f32 {
        out.iter().fold(0.0f32, |a, b| a.max(b.abs()))
    }

    #[test]
    fn fold_note_volume_merges_volume_into_the_note_on_and_drops_the_event() {
        let mut events = vec![
            ScheduledMidi { frame: 10, event: MidiEvent::NoteOn { note: 60, velocity: 100 } },
            ScheduledMidi { frame: 10, event: MidiEvent::NoteVolume { note: 60, value: 64 } },
            ScheduledMidi { frame: 20, event: MidiEvent::NoteOff { note: 60 } },
        ];
        fold_note_volume(&mut events);
        assert_eq!(events.len(), 2, "NoteVolume 必须被折叠掉");
        // 100 × 64 / 127 ≈ 50。
        assert!(matches!(events[0].event, MidiEvent::NoteOn { note: 60, velocity: 50 }));
        assert!(matches!(events[1].event, MidiEvent::NoteOff { note: 60 }));
    }

    #[test]
    fn effect_chain_processes_the_instrument_output() {
        let mut registry = PluginRegistry::with_builtins();
        registry.register("test.half", |_track| Box::new(HalfGain));

        let peak_with = |effects: Vec<PluginInstance>| {
            let mut p = project();
            p.tracks[0].effects = effects;
            let schedule = ScheduledEvents::from_project(&p, 48_000.0);
            let mut player = Player::new(schedule, &registry, 48_000.0, 512);
            player.apply_command(TransportCommand::Play);
            let mut out = vec![0.0f32; 512 * 2];
            player.process_block(&mut out, 512);
            peak(&out)
        };

        let dry = peak_with(vec![]);
        let wet = peak_with(vec![PluginInstance {
            plugin_id: "test.half".into(),
            params: Default::default(),
            state: vec![],
        }]);
        assert!(dry > 0.01, "baseline must be audible: {dry}");
        assert!(wet < dry * 0.75, "效果链必须作用在乐器输出上：{dry} -> {wet}");
    }

    /// 效果链里放一个「非效果」插件（如乐器）：必须被跳过，而不是静音整条链。
    #[test]
    fn non_effect_plugin_in_the_chain_is_skipped_not_muted() {
        let registry = PluginRegistry::with_builtins();
        let mut p = project();
        p.tracks[0].effects = vec![PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: Default::default(),
            state: vec![],
        }];
        let schedule = ScheduledEvents::from_project(&p, 48_000.0);
        let mut player = Player::new(schedule, &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "乐器插在效果位不得让整条链静音");
    }

    /// 实时参数变更直接作用于已装载的实例，不重建播放器。
    #[test]
    fn live_param_change_alters_output_without_rebuild() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut loud = vec![0.0f32; 512 * 2];
        player.process_block(&mut loud, 512);
        let baseline = peak(&loud);
        assert!(baseline > 0.01, "baseline must be audible");

        // 把乐器增益压到 0.05，然后从头再播放一个块。
        player.apply_command(TransportCommand::SetParam {
            track: TrackId(1),
            effect: None,
            param: "gain".into(),
            value: 0.05,
        });
        player.apply_command(TransportCommand::Seek { frame: 0 });
        player.apply_command(TransportCommand::Play);
        let mut quiet = vec![0.0f32; 512 * 2];
        player.process_block(&mut quiet, 512);
        assert!(
            peak(&quiet) < baseline * 0.5,
            "instrument gain must apply live: {baseline} -> {}",
            peak(&quiet)
        );
    }

    /// 界面上的试听音（合成器界面里的键盘）：停止时也要能弹响。
    #[test]
    fn audition_notes_sound_while_stopped() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 24_000);
        let mut out = vec![0.0f32; 512 * 2];

        player.process_block(&mut out, 512);
        assert_eq!(peak(&out), 0.0, "没按键时应当是静的");

        player.apply_command(TransportCommand::Audition {
            track: TrackId(1),
            note: 69,
            velocity: 120,
            on: true,
        });
        player.process_block(&mut out, 512);
        let first = peak(&out);
        assert!(first > 0.01, "试听音必须发声: {first}");

        out.fill(0.0);
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "松开之前应当持续发声");
        assert!(!player.is_playing(), "试听不该启动传输");

        // 松开 → 走释音，几百毫秒后静音。
        player.apply_command(TransportCommand::Audition {
            track: TrackId(1),
            note: 69,
            velocity: 0,
            on: false,
        });
        for _ in 0..400 {
            player.process_block(&mut out, 512);
        }
        assert!(peak(&out) < 0.001, "松开后要停下来");
    }

    /// 循环开关本身不改变播放状态与位置，只是被记下来。
    #[test]
    fn set_loop_toggles_the_flag_without_touching_playback() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        assert!(!player.is_looping(), "默认不循环");
        player.apply_command(TransportCommand::SetLoop(true));
        assert!(player.is_looping());
        assert!(!player.is_playing(), "开关不该启动播放");
        assert_eq!(player.position_frames(), 0, "开关不该移动位置");
        player.apply_command(TransportCommand::SetLoop(false));
        assert!(!player.is_looping());
    }

    #[test]
    fn silent_until_play() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        let mut out = vec![1.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert!(!player.is_playing());
    }

    /// 停止时插件**仍然要被调用**。
    ///
    /// 插件界面上的虚拟键盘、试听按钮、效果器尾音都是在插件内部发声的，宿主不跑
    /// `process` 就什么都听不到——这正是「打开插件界面点键盘没声音」的原因。
    #[test]
    fn keeps_plugins_running_while_stopped() {
        /// 测试用「自鸣」插件：不管有没有事件都往外送信号，模拟插件内部的试听键盘。
        struct Drone;

        impl Plugin for Drone {
            fn descriptor(&self) -> PluginDescriptor {
                PluginDescriptor {
                    id: "test.drone".into(),
                    name: "Drone".into(),
                    kind: PluginKind::Instrument,
                    io: IoLayout { inputs: 0, outputs: 2 },
                    capabilities: Default::default(),
                }
            }
            fn params(&self) -> Vec<ParamInfo> {
                Vec::new()
            }
            fn get_param(&self, _id: &ParamId) -> Option<f32> {
                None
            }
            fn set_param(&mut self, _id: &ParamId, _value: f32) {}
            fn prepare(&mut self, _sample_rate: f64, _block_size: usize) {}
            fn reset(&mut self) {}
            fn process(
                &mut self,
                _ctx: &ProcessContext,
                audio: &mut [&mut [f32]],
                _events: &[TimedMidiEvent],
            ) {
                for channel in audio.iter_mut() {
                    channel.fill(0.5);
                }
            }
            fn save_state(&self) -> Vec<u8> {
                Vec::new()
            }
            fn load_state(&mut self, _state: &[u8]) -> Result<(), PluginError> {
                Ok(())
            }
        }

        let mut registry = PluginRegistry::new();
        registry.register("test.drone", |_track| Box::new(Drone));
        let mut p = project();
        p.tracks[0].instrument =
            Some(PluginInstance { plugin_id: "test.drone".into(), params: Default::default(), state: vec![] });
        let schedule = ScheduledEvents::from_project(&p, 48_000.0);
        let mut player = Player::new(schedule, &registry, 48_000.0, 24_000);

        // 没按播放：时间线不动，但插件照跑 → 听得到。
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "停止时也要能听到插件自己发出的声音");
        assert_eq!(player.position_frames(), 0, "停止时位置不得推进");
        assert!(!player.is_playing());

        // 光标没动 → 恢复播放时音符照常按时间线发出。
        player.apply_command(TransportCommand::Play);
        player.process_block(&mut out, 512);
        assert_eq!(player.position_frames(), 512);
        assert!(player.is_playing());
    }

    #[test]
    fn plays_and_advances_position() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().any(|s| s.abs() > 0.01), "should be audible");
        assert_eq!(player.position_frames(), 512);
    }

    #[test]
    fn stop_pauses_and_rewinds() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        player.apply_command(TransportCommand::Stop);
        assert!(!player.is_playing());
        assert_eq!(player.position_frames(), 0);
    }

    #[test]
    fn seek_moves_cursor_without_playing_early_notes() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        // Note ends at beat 0.5 = 0.25s = 12000 frames; seek past it -> silence.
        player.apply_command(TransportCommand::Seek { frame: 20_000 });
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(player.position_frames(), 20_512);
    }

    #[test]
    fn drain_commands_applies_queued_transport_commands() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        let (mut tx, mut rx) = crate::command_queue::command_queue(8);
        assert!(tx.send(TransportCommand::Seek { frame: 1_024 }));
        assert!(tx.send(TransportCommand::Play));
        player.drain_commands(&mut rx);
        assert!(player.is_playing());
        assert_eq!(player.position_frames(), 1_024);
        // Draining an empty queue must leave the transport untouched.
        player.drain_commands(&mut rx);
        assert!(player.is_playing());
        assert_eq!(player.position_frames(), 1_024);
    }

    /// 播放中途静音：同一位置继续推进，但输出立刻变成静音；解除静音后重新发声。
    #[test]
    fn live_mute_silences_and_unmute_restores_output() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);

        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "baseline must be audible");

        // 立刻静音：后面的块必须全零。
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(1),
            gain_db: 0.0,
            pan: 0.0,
            mute: true,
            solo: false,
        });
        out.fill(1.0);
        player.process_block(&mut out, 512);
        assert_eq!(peak(&out), 0.0, "muted track must be silent");
        // 位置照常推进（静音不等于停止）。
        assert_eq!(player.position_frames(), 1_024);

        // 解除静音：音符已经过去（0.5 拍 = 12000 帧内），所以先 seek 回起点再验证。
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(1),
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
        });
        player.apply_command(TransportCommand::Seek { frame: 0 });
        player.apply_command(TransportCommand::Play);
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "unmuted track must sound again");
    }

    /// 暂停 = **静音**：挂着的长音必须立刻停掉。
    ///
    /// 只把 `playing` 置 false 的话，那个音的 NoteOff 永远送不到，插件会一直挂着它——
    /// 听上去就是「暂停了还在响」。
    #[test]
    fn pause_silences_sustained_notes() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);

        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "播放中必须出声（否则这条测试没有意义）");

        player.apply_command(TransportCommand::Pause);
        assert!(!player.is_playing());
        out.fill(1.0);
        player.process_block(&mut out, 512);
        assert_eq!(peak(&out), 0.0, "暂停之后必须完全静音");
    }

    /// 「0 dB 推子 + 声像居中 + master 居中」= **单位增益**。
    ///
    /// 曾经的等功率律会让轨道与 master 各掉 3 dB，整链 −6 dB：导入一段满幅素材会明显变轻。
    #[test]
    fn centered_pan_and_zero_fader_are_unity_gain() {
        let registry = PluginRegistry::with_builtins();
        let player = Player::new(schedule(), &registry, 48_000.0, 512);
        let track = &player.tracks[0];
        assert!((track.left_gain - 1.0).abs() < 1e-6, "左增益应为 1，实际 {}", track.left_gain);
        assert!((track.right_gain - 1.0).abs() < 1e-6, "右增益应为 1，实际 {}", track.right_gain);
        assert!((player.master_left - 1.0).abs() < 1e-6, "master 左应为 1，实际 {}", player.master_left);
        assert!((player.master_right - 1.0).abs() < 1e-6, "master 右应为 1，实际 {}", player.master_right);
    }

    /// 声像硬偏（线性 balance）：一侧保留、另一侧归零。
    #[test]
    fn hard_pan_keeps_one_side_and_mutes_the_other() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(1),
            gain_db: 0.0,
            pan: 1.0,
            mute: false,
            solo: false,
        });
        assert_eq!(player.tracks[0].left_gain, 0.0, "硬右应当把左声道清零");
        assert!((player.tracks[0].right_gain - 1.0).abs() < 1e-6);
    }

    #[test]
    fn live_gain_change_scales_output_without_rebuild() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut loud = vec![0.0f32; 512 * 2];
        player.process_block(&mut loud, 512);
        let baseline = peak(&loud);

        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(1),
            gain_db: -20.0,
            pan: 0.0,
            mute: false,
            solo: false,
        });
        player.apply_command(TransportCommand::Seek { frame: 0 });
        player.apply_command(TransportCommand::Play);
        let mut quiet = vec![0.0f32; 512 * 2];
        player.process_block(&mut quiet, 512);
        let quieter = peak(&quiet);

        // -20 dB ≈ 0.1 倍。允许一定裕度（首块包络尚未到达稳态）。
        assert!(quieter < baseline * 0.5, "gain must drop: {baseline} -> {quieter}");
    }

    #[test]
    fn solo_mutes_every_other_track() {
        let mut p = project();
        p.tracks.push(Track {
            id: TrackId(2),
            name: "Bass".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(2),
                start: 0.0,
                length: 1.0,
                notes: vec![Note { pitch: 40, start: 0.0, duration: 0.5, velocity: 100, volume: 127 }],
                audio: None,
            }],
            mixer: Default::default(),
        });
        let registry = PluginRegistry::with_builtins();
        let schedule = ScheduledEvents::from_project(&p, 48_000.0);
        let mut player = Player::new(schedule, &registry, 48_000.0, 512);

        // 只让第 2 轨独奏，第 1 轨应当被压掉；第 2 轨仍然发声。
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(2),
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: true,
        });
        assert!(!player.tracks[0].audible);
        assert!(player.tracks[1].audible);

        // 取消独奏后两轨都恢复。
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(2),
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
        });
        assert!(player.tracks[0].audible && player.tracks[1].audible);
    }

    #[test]
    fn master_gain_scales_the_sum() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        let baseline = peak(&out);

        player.apply_command(TransportCommand::SetMasterGain { gain_db: -20.0 });
        player.apply_command(TransportCommand::Seek { frame: 0 });
        player.apply_command(TransportCommand::Play);
        let mut out2 = vec![0.0f32; 512 * 2];
        player.process_block(&mut out2, 512);
        assert!(peak(&out2) < baseline * 0.5);
    }

    /// 播到结尾后再按播放：必须从头重播，而不是「按了没反应」。
    #[test]
    fn play_after_the_end_restarts_from_the_beginning() {
        let registry = PluginRegistry::with_builtins();
        // 片段长 1 拍 @120bpm/48kHz = 24000 帧，正好等于 end_frame。
        let mut player = Player::new(schedule(), &registry, 48_000.0, 24_000);
        player.apply_command(TransportCommand::Play);
        let mut out = vec![0.0f32; 24_000 * 2];
        player.process_block(&mut out, 24_000);
        assert_eq!(player.position_frames(), player.end_frame());

        // 宿主策略：到达结尾即暂停（位置留在结尾）。
        player.apply_command(TransportCommand::Pause);
        assert!(!player.is_playing());
        assert_eq!(player.position_frames(), 24_000);

        // 再按播放 → 回到起点并重新发声。
        player.apply_command(TransportCommand::Play);
        assert_eq!(player.position_frames(), 0, "must rewind before restarting");
        assert!(player.is_playing());
        let mut again = vec![0.0f32; 512 * 2];
        player.process_block(&mut again, 512);
        assert!(peak(&again) > 0.01, "restart must be audible");
    }

    /// 起点处的 Play 不得多此一举地 seek（否则离线渲染的逐位一致性会被破坏）。
    #[test]
    fn play_at_the_start_does_not_rewind() {
        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Seek { frame: 100 });
        player.apply_command(TransportCommand::Play);
        assert_eq!(player.position_frames(), 100);
    }

    #[test]
    fn unknown_track_in_mix_command_is_ignored() {        let registry = PluginRegistry::with_builtins();
        let mut player = Player::new(schedule(), &registry, 48_000.0, 512);
        player.apply_command(TransportCommand::Play);
        player.apply_command(TransportCommand::SetTrackMix {
            track: TrackId(999),
            gain_db: -60.0,
            pan: 0.0,
            mute: true,
            solo: false,
        });
        let mut out = vec![0.0f32; 512 * 2];
        player.process_block(&mut out, 512);
        assert!(peak(&out) > 0.01, "不存在的轨道不得影响既有轨道");
    }
}
