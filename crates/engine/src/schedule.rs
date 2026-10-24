use std::sync::Arc;

use ai_daw_plugin_api::{MidiEvent, ParamId};

use crate::audio::{DecodedAudio, SampleProvider};
use crate::model::{
    note_tag_key, ArticulationSwitch, AutomationPoint, MixerChannel, MpeConfig, Note,
    PitchExpression, PluginInstance, Project, TrackId, TrackMeta,
};
use crate::player::db_to_linear;
use crate::tempo::TempoMap;

/// 单颗音符最多展开多少个音高表达事件（约 40 秒 @100 Hz，足够任何真实乐句）。
const MAX_EXPRESSION_EVENTS: usize = 4096;
/// 揉弦控制率（Hz）。5–7 Hz 的揉弦在 100 Hz 采样下足够平滑。
const EXPRESSION_CONTROL_HZ: f64 = 100.0;

/// 一条自动化车道的**已解析目标**。
///
/// 键的形式（见 `specs/2026-10-10-plugin-param-automation-design.md` §4）：
/// `gain_db` / `pan` / `cc:<0..=127>` / `inst:<param_id>` / `fx<i>:<param_id>`（`i` 从 0 起）。
///
/// 非 `Copy`：`ParamId` 拥有自己的 `String`。**构造只在控制侧 / 调度构建时发生**
/// （`from_key` 会分配），解析好的目标随 `ScheduledLane` 带进播放器，音频线程只做移动。
#[derive(Debug, Clone, PartialEq)]
pub enum LaneTarget {
    /// 轨道推子增益（dB）。
    GainDb,
    /// 轨道声像（-1..=1）。
    Pan,
    /// MIDI 控制变化（0..=127）：逐块发给轨道乐器。延音踏板 / 调制轮 / 表情等走这里。
    ControlChange(u8),
    /// 通道压力 / 触后（通道 0..=15）：逐块发给轨道乐器。
    ChannelPressure(u8),
    /// 轨道的乐器参数。
    Instrument(ParamId),
    /// 效果链第 `index` 个效果器的参数。
    Effect { index: usize, param: ParamId },
}

impl LaneTarget {
    /// 从键解析。**会分配**（`ParamId` 持有 `String`）——**绝不在音频线程调用**。
    pub fn from_key(key: &str) -> Option<LaneTarget> {
        match key {
            "gain_db" => return Some(LaneTarget::GainDb),
            "pan" => return Some(LaneTarget::Pan),
            _ => {}
        }
        if let Some(rest) = key.strip_prefix("cc:") {
            let controller: u8 = rest.parse().ok()?;
            return (controller <= 127).then_some(LaneTarget::ControlChange(controller));
        }
        if let Some(rest) = key.strip_prefix("at:") {
            let channel: u8 = rest.parse().ok()?;
            return (channel <= 15).then_some(LaneTarget::ChannelPressure(channel));
        }
        if let Some(rest) = key.strip_prefix("inst:") {
            return (!rest.is_empty()).then(|| LaneTarget::Instrument(ParamId(rest.to_string())));
        }
        if let Some(rest) = key.strip_prefix("fx") {
            let (index, param) = rest.split_once(':')?;
            if param.is_empty() {
                return None;
            }
            return Some(LaneTarget::Effect {
                index: index.parse().ok()?,
                param: ParamId(param.to_string()),
            });
        }
        None
    }
}

/// 一条已解析的自动化车道（保留「拍」坐标，求值时用速度表换算）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledLane {
    /// 目标参数。
    pub param: LaneTarget,
    /// 关键点（拍，升序）。
    pub points: Vec<AutomationPoint>,
    /// **插件参数**车道在自动化接管**之前**的值（`get_param` 的快照）。
    ///
    /// 只有 `Instrument` / `Effect` 目标会填它；`GainDb` / `Pan` 恒为 `None`（它们的自动化值活在
    /// 混音计算的局部变量里，移除曲线即回到推子值，不需要快照）。插件参数是直接写进插件实例的，
    /// 移除车道时若不用这个快照写回，插件会停在最后一次求值处——与「读取模式」语义不符。
    ///
    /// 值放在车道里而不是单独的 map：map 要以**克隆的** `ParamId` 作键，会在音频线程上分配。
    pub static_value: Option<f32>,
}

impl ScheduledLane {
    /// 在 `beat` 处求值（不分配）。
    pub fn value_at(&self, beat: f64) -> Option<f32> {
        crate::model::lane_value_at(&self.points, beat)
    }
}

/// 一路已解析的发送：目标解析成**轨道下标**，播放时 O(1) 路由。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledSend {
    /// 目标轨道在 `ScheduledEvents::tracks` 里的下标。
    pub target_index: usize,
    /// 线性发送增益。
    pub gain: f32,
}

/// 一条已定绝对帧的 MIDI 事件。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledMidi {
    /// 自播放起点起的绝对帧位置。
    pub frame: u64,
    /// MIDI 通道（0..=15）。MPE 开启时逐音分配，否则为 0。
    pub channel: u8,
    /// 该帧时刻发出的 MIDI 事件。
    pub event: MidiEvent,
}

/// 一条已定绝对帧、且已接上素材的音频片段。
///
/// 语义（见 `specs/2026-10-09-audio-clip-design.md`）：`length`（拍）只决定时间轴**方框**；
/// 素材按 `rate`（播放速度）播放、**与工程 BPM 解耦**——改 BPM 不改音频；素材放完或方框到头
/// 即停。`pitch_semitones` 由 `stretch::Stretcher` 施加（**变速不变调**）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAudioClip {
    /// 片段起始的绝对帧（含）。
    pub start_frame: u64,
    /// 片段结束的绝对帧（不含）——可听上界。
    pub end_frame: u64,
    /// 素材内的起始帧（crop 起点，已按素材长度夹紧）。
    pub offset_frames: u64,
    /// 素材窗口长度（源帧）：`offset_frames` 起取多少帧。
    pub source_window_frames: u64,
    /// 播放速度倍率（1.0 = 原速）。改它只改时长，不变调。
    pub rate: f64,
    /// **素材采样率 / 工程采样率**（素材率与输出率不同时必须按它重采样读取）。
    ///
    /// 播放器按**输出帧**推进（`start_frame` / `end_frame` 都是输出帧），而素材存的是**它自己
    /// 采样率下**的帧。两者不同时读指针每个输出帧必须前进 `source_ratio` 个源帧——那正是采样率
    /// 换算（用 `lerp_frame` 插值），时长与音高都不变。缺了它就等于「把素材按输出率硬播」：
    /// 44.1 kHz 素材在 48 kHz 工程里会快 8.8% 并升调，尾端还会被方框截掉。
    pub source_ratio: f64,
    /// 升降调（半音，0 = 不变）。改它只变调，不改时长。
    pub pitch_semitones: f64,
    /// 已解码素材（构造播放器前加载完毕，播放路径只读）。
    pub audio: Arc<DecodedAudio>,
}

/// 一条带音高表达的音符，供播放器在「插件不支持逐音表达」时折叠成通道弯音。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchSpan {
    /// MIDI 音高。
    pub note: u8,
    /// 该音所在的 MIDI 通道（MPE / 显式通道）。
    pub channel: u8,
    /// 起音绝对帧。
    pub start_frame: u64,
    /// 收音绝对帧。
    pub end_frame: u64,
    /// 起音拍（绝对）。
    pub start_beat: f64,
    /// 音高表达参数。
    pub expr: PitchExpression,
}

/// 调度期的一颗音符：先把所有音符的起止算出来，再统一分配通道。
#[derive(Debug, Clone, Copy)]
struct NotePlan {
    note: Note,
    /// 所属片段（查奏法标签用）。
    clip: crate::model::ClipId,
    /// 实际发声位置（拍，含 `offset`）。
    onset: f64,
    /// 实际收音位置（拍，含连奏延长与片段裁剪）。
    off: f64,
    start_frame: u64,
    end_frame: u64,
}

/// 单轨的定帧事件表与播放所需信息（自带插件引用与混音参数，使 `Player` 不依赖 `Project`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledTrack {
    /// 轨道标识。
    pub track_id: TrackId,
    /// 轨道使用的乐器插件实例（无则静音/留空）。
    pub instrument: Option<PluginInstance>,
    /// 轨道效果链（按顺序处理，接在乐器之后）。
    pub effects: Vec<PluginInstance>,
    /// 轨道混音参数。
    pub mixer: MixerChannel,
    /// 按帧稳定排序的事件列表。
    pub events: Vec<ScheduledMidi>,
    /// 带音高表达的音符（只含 `expr != NONE` 的音符）。
    pub pitch_spans: Vec<PitchSpan>,
    /// MIDI 弯音范围（半音），来自 `TrackMeta`，缺省 2。
    pub pitch_bend_range: f32,
    /// 音频片段（可能为空）。素材缺失的片段不会出现在这里（按静音处理）。
    pub audio_clips: Vec<ScheduledAudioClip>,
    /// 自动化车道（已解析参数；求值时按速度表换算拍）。
    pub lanes: Vec<ScheduledLane>,
    /// 发送（目标已解析成轨道下标）。
    pub sends: Vec<ScheduledSend>,
}

/// 整首工程的定帧事件表：离线渲染与实时播放共用。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledEvents {
    /// 各轨道的定帧事件表。
    pub tracks: Vec<ScheduledTrack>,
    /// 总输出通道（增益在播放器里作用于各轨混合之后）。
    pub master: MixerChannel,
    /// 工程内容总长度（帧），取所有片段结束位置的最大值。
    pub end_frame: u64,
    /// 构建时使用的速度，供实时播放按拍推进时复用。
    pub tempo: f64,
    /// 速度分段在时间轴上的落点（帧 → BPM，按帧升序，首项为 `(0, tempo_at(0))`）。
    /// 供实时播放逐块取当前速度。
    pub tempo_points: Vec<(u64, f64)>,
    /// 构建该事件表时使用的采样率（Hz）；所有 `frame` 均以此刻度定点。宿主应在
    /// 采样率不一致时拒绝播放，否则事件会被静默错误定时。
    pub sample_rate: f64,
    /// 处理顺序（各轨道在 `tracks` 里的下标）：**被发送的目标排在其来源之后**。
    ///
    /// 由发送关系做拓扑排序；有环时剩余的轨道按原顺序追加（环上的发送会被忽略）。
    /// 播放器按这个顺序逐轨处理，总线才能看到已经汇总好的输入。
    pub order: Vec<usize>,
    /// 速度表：自动化按「拍」求值时，播放器用它把当前帧位置换算成拍。
    pub tempo_map: TempoMap,
}

/// 由发送边算出一个「来源在前、目标在后」的处理顺序。
///
/// `edges[i]` = 从轨道 i 发出的目标下标。用 Kahn 算法，零入度者按**下标升序**取，保证
/// 结果确定；有环时剩余的按下标顺序追加（环上的边会被忽略——宁可少一路发送，也不要死循环）。
pub fn routing_order(edges: &[Vec<usize>]) -> Vec<usize> {
    let n = edges.len();
    let mut indegree = vec![0usize; n];
    for targets in edges {
        for &t in targets {
            if t < n {
                indegree[t] += 1;
            }
        }
    }
    let mut order = Vec::with_capacity(n);
    let mut used = vec![false; n];
    loop {
        // 取一个未使用、入度为 0 的最小下标。
        let mut picked = None;
        for i in 0..n {
            if !used[i] && indegree[i] == 0 {
                picked = Some(i);
                break;
            }
        }
        let Some(i) = picked else { break };
        used[i] = true;
        order.push(i);
        for &t in &edges[i] {
            if t < n && indegree[t] > 0 {
                indegree[t] -= 1;
            }
        }
    }
    // 环上的残余节点：按下标顺序补上（它们的出边已经无法满足拓扑约束）。
    for i in 0..n {
        if !used[i] {
            order.push(i);
        }
    }
    order
}

impl ScheduledEvents {
    /// 从工程构建定帧事件表。事件按帧稳定排序；同帧时保持（clip 顺序 → note 顺序 → NoteOn 先于 NoteOff）的原有次序，以保证与 Plan 1 的渲染一致。
    ///
    /// **不含音频素材**：工程里的音频片段一律按静音处理。要真正出声请用
    /// [`ScheduledEvents::from_project_with_assets`]。
    pub fn from_project(project: &Project, sample_rate: f64) -> Self {
        Self::from_project_with_assets(project, sample_rate, &crate::audio::no_assets())
    }

    /// 同 [`ScheduledEvents::from_project`]，但把音频片段接到真实素材上。
    ///
    /// 音频片段的绝对帧用与 MIDI **同一个** `to_frame` 闭包算出，保证两者严格对齐。
    /// 素材缺失（`assets.get` 返回 `None`）或窗口为空时该片段被跳过——与「缺失插件用静音占位」
    /// 同一策略：绝不让一段坏素材拖垮整首曲子。
    pub fn from_project_with_assets(
        project: &Project,
        sample_rate: f64,
        assets: &dyn SampleProvider,
    ) -> Self {
        let map = TempoMap::from_transport(&project.transport);
        let to_frame = |beats: f64| (map.beats_to_seconds(beats) * sample_rate) as u64;

        // 把一条奏法切换展开成 MIDI 事件。
        // 简写字段（keyswitch / CC / program）与导入的消息序列都在 `messages()` 里统一；
        // keyswitch 会补一个很短的 NoteOn/NoteOff——多数音源在 NoteOn 时切换奏法。
        let push_articulation = |events: &mut Vec<ScheduledMidi>,
                                 beat: f64,
                                 channel: u8,
                                 switch: &ArticulationSwitch| {
            for message in switch.messages() {
                let at = (beat + f64::from(message.offset_beats)).max(0.0);
                events.push(ScheduledMidi {
                    frame: to_frame(at),
                    channel: message.channel.unwrap_or(channel),
                    event: message.event,
                });
            }
        };
        let mut tempo_points: Vec<(u64, f64)> = vec![(0, map.tempo_at(0.0))];
        for s in map.segments().iter().skip(1) {
            tempo_points.push((to_frame(s.start_beat), s.tempo));
        }

        // 轨道 id → 下标：发送的目标要用下标表达（播放器 O(1) 路由）。
        let index_by_id: std::collections::HashMap<TrackId, usize> = project
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();

        let mut tracks = Vec::with_capacity(project.tracks.len());
        let mut edges: Vec<Vec<usize>> = Vec::with_capacity(project.tracks.len());
        for track in &project.tracks {
            let mut events = Vec::new();
            let mut pitch_spans = Vec::new();
            let mut audio_clips = Vec::new();
            let mut note_plans: Vec<NotePlan> = Vec::new();
            for clip in &track.clips {
                // 音频片段：接素材、定绝对帧。素材缺失 → 整段静音（跳过）。
                if let Some(source) = &clip.audio {
                    let Some(audio) = assets.get(&source.path) else {
                        continue;
                    };
                    let total = audio.frames() as u64;
                    let offset = source.offset_frames.min(total);
                    // 素材窗口 = 显式长度（切分/裁剪记录下来的）或「到结尾」；都要夹到可用范围。
                    let window = source.frames.unwrap_or(total - offset).min(total - offset);
                    if window == 0 {
                        continue;
                    }
                    let start_frame = to_frame(clip.start);
                    let end_frame = to_frame(clip.start + clip.length);
                    if end_frame <= start_frame {
                        continue;
                    }
                    audio_clips.push(ScheduledAudioClip {
                        start_frame,
                        end_frame,
                        offset_frames: offset,
                        source_window_frames: window,
                        rate: crate::model::AudioSource::normalize_rate(source.rate),
                        // 素材与工程采样率不同 → 读指针要按这个比值重采样（见字段文档）。
                        source_ratio: if sample_rate > 0.0 && audio.sample_rate > 0.0 {
                            audio.sample_rate / sample_rate
                        } else {
                            1.0
                        },
                        pitch_semitones: crate::model::AudioSource::normalize_pitch(
                            source.pitch_semitones,
                        ),
                        audio,
                    });
                    continue;
                }
                // 片段是**裁剪窗口**：起点落在方框外的音符不发声，越过方框尾端的在尾端截断。
                // （音频片段同理：`start_frame..end_frame` 已经是窗口。）
                let box_end = clip.start + clip.length;
                for note in &clip.notes {
                    // 裁剪按**记谱位置**判定：`offset` 是演奏姿态（扫弦 / 微调），不该让一个
                    // 明明写在方框里的音符因为偏移而「掉出方框」。
                    let on = clip.start + note.start;
                    if on < clip.start - 1e-9 || on >= box_end - 1e-9 {
                        continue;
                    }
                    // 实际发声位置 = 记谱位置 + 偏移。
                    let onset = on + note.offset;
                    // 连奏：NoteOff 推到「同音高的下一个音」实际发声处（没有下一个就按原时值）。
                    // 判据用**发声位置**（含各自偏移），所以扫弦里同一个音之间也不会有缝。
                    let next_same_pitch = clip
                        .notes
                        .iter()
                        .filter(|other| other.pitch == note.pitch)
                        .map(|other| clip.start + other.start + other.offset)
                        .filter(|sounded| *sounded > onset + 1e-9)
                        .fold(f64::INFINITY, f64::min);
                    let off = if note.legato && next_same_pitch.is_finite() {
                        next_same_pitch.min(box_end)
                    } else {
                        (onset + note.duration).min(box_end)
                    };
                    note_plans.push(NotePlan {
                        note: *note,
                        clip: clip.id,
                        onset,
                        off,
                        start_frame: to_frame(onset),
                        end_frame: to_frame(off),
                    });
                }
            }

            // 通道分配：显式 `note.channel` 优先；MPE 开启时给重叠音符分配不同成员通道；
            // 否则一律主通道 0。全在控制侧一次算完，音频线程不参与。
            note_plans.sort_by(|a, b| {
                a.start_frame
                    .cmp(&b.start_frame)
                    .then_with(|| a.note.pitch.cmp(&b.note.pitch))
            });
            let mpe = track
                .meta
                .as_ref()
                .and_then(|m| m.mpe)
                .map(MpeConfig::normalized);
            let mut member_free: Vec<(u8, u64)> = mpe
                .map(|m| m.member_channels().into_iter().map(|c| (c, 0)).collect())
                .unwrap_or_default();

            for plan in &note_plans {
                let note = plan.note;
                let channel = match note.channel {
                    Some(c) => {
                        let c = c.min(15);
                        if let Some(slot) = member_free.iter_mut().find(|(m, _)| *m == c) {
                            slot.1 = slot.1.max(plan.end_frame);
                        }
                        c
                    }
                    None if mpe.is_some() => {
                        // 先找已经空出来的通道；都被占就抢最早结束的那条（确定性）。
                        let index = member_free
                            .iter()
                            .position(|(_, at)| *at <= plan.start_frame)
                            .unwrap_or_else(|| {
                                member_free
                                    .iter()
                                    .enumerate()
                                    .min_by_key(|(_, (_, at))| *at)
                                    .map(|(i, _)| i)
                                    .unwrap_or(0)
                            });
                        let (ch, at) = &mut member_free[index];
                        *at = plan.end_frame;
                        *ch
                    }
                    None => 0,
                };

                // 奏法：在起音前发出切换（提前量默认 1/64 拍）。标签存在工程的 `note_tags` 里。
                if let Some(meta) = track.meta.as_ref() {
                    let key = note_tag_key(track.id.0, plan.clip.0, note.id);
                    if let Some(name) = project
                        .note_tags
                        .get(&key)
                        .and_then(|tag| tag.articulation.as_deref())
                    {
                        if let Some(switch) = meta.articulations.get(name) {
                            let lead = f64::from(switch.effective_lead());
                            push_articulation(&mut events, plan.onset - lead, channel, switch);
                            if switch.momentary.unwrap_or(false) {
                                if let Some(restore) = meta
                                    .default_articulation
                                    .as_deref()
                                    .and_then(|name| meta.articulations.get(name))
                                {
                                    push_articulation(&mut events, plan.off, channel, restore);
                                }
                            }
                        }
                    }
                }

                events.push(ScheduledMidi {
                    frame: plan.start_frame,
                    channel,
                    // 只送「演奏强度」。插件各自解释它（音色 / 振幅）——这是设计如此。
                    event: MidiEvent::NoteOn {
                        note: note.pitch,
                        velocity: note.velocity,
                    },
                });
                // 起音压力：复音压力（逐音触后）。
                if let Some(pressure) = note.pressure {
                    events.push(ScheduledMidi {
                        frame: plan.start_frame,
                        channel,
                        event: MidiEvent::PolyPressure {
                            note: note.pitch,
                            value: pressure.min(127),
                        },
                    });
                }
                // 单音音量与力度**分开发**：支持 per-note 音量的插件独立处理它，不改变音色。
                // 默认值 127（不衰减）不需要事件。
                if note.volume != Note::MAX_VOLUME {
                    events.push(ScheduledMidi {
                        frame: plan.start_frame,
                        channel,
                        event: MidiEvent::NoteVolume {
                            note: note.pitch,
                            value: note.volume,
                        },
                    });
                }
                // 音高表达：推弦 / 揉弦 / 滑音。展开成逐音事件；不支持逐音的插件由
                // `Player` 折叠成通道弯音。
                if !note.expr.is_none() {
                    let start_frame = plan.start_frame;
                    let end_frame = plan.end_frame;
                    let duration_beats = (plan.off - plan.onset).max(0.0);
                    pitch_spans.push(PitchSpan {
                        note: note.pitch,
                        channel,
                        start_frame,
                        end_frame,
                        start_beat: plan.onset,
                        expr: note.expr,
                    });

                    let mut points: Vec<(u64, i16)> = Vec::new();
                    // 揉弦、以及「有实际音高变化的推弦」都需要连续采样；否则插件只会
                    // 收到起止两点，听到的是跳变而不是滑音。
                    let needs_ramp =
                        note.expr.bend_time > 0.0 && note.expr.bend != note.expr.bend_to;
                    if note.expr.vibrato != 0 || needs_ramp {
                        let step = (sample_rate / EXPRESSION_CONTROL_HZ).round().max(1.0) as u64;
                        let mut frame = start_frame;
                        while frame < end_frame && points.len() + 1 < MAX_EXPRESSION_EVENTS {
                            let beat = map.seconds_to_beats(frame as f64 / sample_rate);
                            let cents = note
                                .expr
                                .cents_at(beat - plan.onset, duration_beats)
                                .round() as i16;
                            points.push((frame, cents));
                            frame = frame.saturating_add(step);
                        }
                        // 保证起音点一定有值（极短音符 / 采样步长大于时值时）。
                        if points.first().map(|p| p.0) != Some(start_frame) {
                            let cents = note.expr.cents_at(0.0, duration_beats).round() as i16;
                            points.insert(0, (start_frame, cents));
                        }
                    } else {
                        // 恒定偏移 / 瞬间到位：只需要起音点。
                        points.push((
                            start_frame,
                            note.expr.cents_at(0.0, duration_beats).round() as i16,
                        ));
                    }

                    // 去掉相邻重复值，减少事件量（推弦保持段只留一条）。
                    let mut last: Option<i16> = None;
                    for (frame, cents) in points {
                        if last == Some(cents) {
                            continue;
                        }
                        last = Some(cents);
                        events.push(ScheduledMidi {
                            frame,
                            channel,
                            event: MidiEvent::NotePitch {
                                note: note.pitch,
                                cents,
                            },
                        });
                    }
                    // 收音前把音高归零：下一颗同音音符不会继承上一颗的弯音。
                    events.push(ScheduledMidi {
                        frame: end_frame,
                        channel,
                        event: MidiEvent::NotePitch {
                            note: note.pitch,
                            cents: 0,
                        },
                    });
                }
                events.push(ScheduledMidi {
                    frame: plan.end_frame,
                    channel,
                    event: MidiEvent::NoteOff { note: note.pitch },
                });
            }
            events.sort_by_key(|e| e.frame);
            // 通用 MIDI 事件表：绝对拍 → 帧，与音符事件一起排序。
            if let Some(meta) = track.meta.as_ref() {
                for item in &meta.midi_events {
                    events.push(ScheduledMidi {
                        frame: to_frame(item.beat.max(0.0)),
                        channel: item.channel.min(15),
                        event: item.event,
                    });
                }
                // 默认奏法：在轨道起点先切换一次（没有音符时也能生效）。
                if let Some(switch) = meta
                    .default_articulation
                    .as_deref()
                    .and_then(|name| meta.articulations.get(name))
                {
                    push_articulation(
                        &mut events,
                        0.0,
                        mpe.as_ref().map(|m| m.master_channel()).unwrap_or(0),
                        switch,
                    );
                }
            }
            events.sort_by_key(|e| e.frame);
            // 自动化车道：解析键 + **结构性校验**——`inst:` 要求本轨有乐器，`fx<i>:`
            // 要求下标在效果链内；不满足就跟未知键一样丢弃（工程里仍保留，装回即恢复）。
            let lanes: Vec<ScheduledLane> = track
                .automation
                .iter()
                .filter_map(|lane| {
                    let param = LaneTarget::from_key(&lane.param)?;
                    match &param {
                        LaneTarget::Instrument(_) if track.instrument.is_none() => return None,
                        LaneTarget::Effect { index, .. } if *index >= track.effects.len() => {
                            return None
                        }
                        _ => {}
                    }
                    let points = lane.points.clone();
                    // 静态值快照由播放器在构造时从**活实例**上采集（这里拿不到插件实例）。
                    Some(ScheduledLane {
                        param,
                        points,
                        static_value: None,
                    })
                })
                .collect();
            // 发送：目标解析成下标；指向不存在轨道或指向自己的发送丢弃。
            let self_index = index_by_id[&track.id];
            let sends: Vec<ScheduledSend> = track
                .sends
                .iter()
                .filter_map(|s| {
                    let target_index = *index_by_id.get(&s.target)?;
                    (target_index != self_index).then_some(ScheduledSend {
                        target_index,
                        gain: db_to_linear(s.gain_db),
                    })
                })
                .collect();
            edges.push(sends.iter().map(|s| s.target_index).collect());
            tracks.push(ScheduledTrack {
                track_id: track.id,
                instrument: track.instrument.clone(),
                effects: track.effects.clone(),
                mixer: track.mixer,
                events,
                pitch_spans,
                pitch_bend_range: track
                    .meta
                    .as_ref()
                    .map(|m| m.effective_pitch_bend_range())
                    .unwrap_or(TrackMeta::DEFAULT_PITCH_BEND_RANGE),
                audio_clips,
                lanes,
                sends,
            });
        }

        let order = routing_order(&edges);

        ScheduledEvents {
            tracks,
            master: project.master,
            end_frame: to_frame(project.length_beats()),
            tempo: map.tempo_at(0.0),
            tempo_points,
            sample_rate,
            order,
            tempo_map: map,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AutomationLane, AutomationPoint, Clip, ClipId, Note, Send, Track, TrackId};
    use ai_daw_plugin_api::ParamId;

    #[test]
    fn lane_target_from_key_parses_every_form() {
        assert_eq!(LaneTarget::from_key("gain_db"), Some(LaneTarget::GainDb));
        assert_eq!(LaneTarget::from_key("pan"), Some(LaneTarget::Pan));
        assert_eq!(
            LaneTarget::from_key("inst:cutoff"),
            Some(LaneTarget::Instrument(ParamId("cutoff".into())))
        );
        assert_eq!(
            LaneTarget::from_key("fx2:mix"),
            Some(LaneTarget::Effect {
                index: 2,
                param: ParamId("mix".into())
            })
        );
        assert_eq!(
            LaneTarget::from_key("cc:64"),
            Some(LaneTarget::ControlChange(64))
        );
        assert_eq!(
            LaneTarget::from_key("cc:0"),
            Some(LaneTarget::ControlChange(0)),
            "CC 从 0 起"
        );
        // 不认识 / 残缺的形式
        assert_eq!(LaneTarget::from_key(""), None);
        assert_eq!(LaneTarget::from_key("nope"), None);
        assert_eq!(LaneTarget::from_key("inst:"), None);
        assert_eq!(LaneTarget::from_key("fx:mix"), None, "下标必须是数字");
        assert_eq!(LaneTarget::from_key("fx1:"), None, "参数名不能为空");
        assert_eq!(LaneTarget::from_key("fx"), None);
        assert_eq!(LaneTarget::from_key("cc:"), None);
        assert_eq!(LaneTarget::from_key("cc:128"), None, "CC 最大 127");
        assert_eq!(LaneTarget::from_key("cc:-1"), None);
        assert_eq!(LaneTarget::from_key("cc:x"), None);
    }

    fn project_with_note() -> Project {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            meta: None,
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch: 60,
                    start: 1.0,
                    duration: 1.0,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: crate::model::PitchExpression::NONE,
                }],
                audio: None,
            }],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
        });
        p
    }

    #[test]
    fn builds_sorted_absolute_frame_events() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert_eq!(s.tracks.len(), 1);
        let ev = &s.tracks[0].events;
        assert_eq!(ev.len(), 2);
        // 120bpm, 48kHz: 1 beat = 0.5s = 24000 frames.
        assert_eq!(ev[0].frame, 24_000);
        assert!(matches!(
            ev[0].event,
            MidiEvent::NoteOn {
                note: 60,
                velocity: 100
            }
        ));
        assert_eq!(ev[1].frame, 48_000);
        assert!(matches!(ev[1].event, MidiEvent::NoteOff { note: 60 }));
    }

    /// 片段方框是**裁剪窗口**：起点在框外的音符不发声，越过框尾的截断在框尾。
    #[test]
    fn the_clip_box_crops_notes() {
        let mut p = project_with_note();
        // 片段长 4 拍：再加一个「起点已在框外」的音符（第 5 拍起）。
        p.tracks[0].clips[0].notes.push(Note {
            id: 0,
            channel: None,
            pressure: None,
            pitch: 62,
            start: 5.0,
            duration: 1.0,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: crate::model::PitchExpression::NONE,
        });
        // 第 1 拍那个音符拉长到第 6 拍 → 越过框尾，应截断在第 4 拍。
        p.tracks[0].clips[0].notes[0].duration = 5.0;

        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let on: Vec<u8> = ev
            .iter()
            .filter_map(|e| match e.event {
                MidiEvent::NoteOn { note, .. } => Some(note),
                _ => None,
            })
            .collect();
        assert_eq!(on, vec![60], "框外的音符不该发声");
        // 120 BPM / 48 kHz → 1 拍 = 24000 帧；第 4 拍 = 96000。
        let off = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOff { note: 60 }))
            .expect("应有 NoteOff");
        assert_eq!(off.frame, 96_000, "越过框尾的音符应截断在框尾");
    }

    /// `offset` 把**实际发声位置**推后，但不改记谱位置，也不影响方框裁剪判定。
    #[test]
    fn note_offset_shifts_the_onset_but_not_the_grid() {
        let mut p = project_with_note();
        // 120 BPM / 48 kHz → 1 拍 = 24000 帧；音符原样是 1.0..2.0 拍。
        p.tracks[0].clips[0].notes[0].offset = 0.25;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let on = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOn { .. }))
            .unwrap();
        assert_eq!(on.frame, 30_000, "1.0 + 0.25 拍 = 30000 帧");
        let off = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOff { .. }))
            .unwrap();
        assert_eq!(
            off.frame, 54_000,
            "时值仍是 1 拍：1.25 + 1.0 = 2.25 拍 = 54000 帧"
        );
    }

    /// 推弦：起音点、到位点、收音前归零，三条事件都在正确的帧上。
    #[test]
    fn pitch_expression_expands_into_note_pitch_events() {
        let mut p = project_with_note();
        p.tracks[0].clips[0].notes[0].expr = crate::model::PitchExpression {
            bend: 0,
            bend_to: 200,
            bend_time: 0.5,
            ..crate::model::PitchExpression::NONE
        };
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let pitches: Vec<(u64, i16)> = ev
            .iter()
            .filter_map(|e| match e.event {
                MidiEvent::NotePitch { cents, .. } => Some((e.frame, cents)),
                _ => None,
            })
            .collect();
        // 120 BPM / 48 kHz：1 拍 = 24000 帧。起音 1.0 拍、到位 1.5 拍、收音 2.0 拍。
        // 推弦必须连续采样（否则插件听到的是跳变）：起点 0，1.5 拍 ≈ 200，收音前归零。
        assert_eq!(pitches.first(), Some(&(24_000, 0)));
        assert_eq!(pitches.last(), Some(&(48_000, 0)));
        assert!(
            pitches
                .iter()
                .any(|(frame, cents)| *frame == 36_000 && (*cents - 200).abs() <= 4),
            "到位点应在 1.5 拍附近达到 200 音分：{pitches:?}"
        );
        let ramp: Vec<i16> = pitches
            .iter()
            .filter(|(frame, _)| *frame >= 24_000 && *frame <= 36_000)
            .map(|(_, cents)| *cents)
            .collect();
        assert!(
            ramp.windows(2).all(|w| w[0] <= w[1]),
            "推弦过程应单调上升：{ramp:?}"
        );
        assert!(pitches.len() > 20, "推弦应按控制率展开：{pitches:?}");
        assert_eq!(s.tracks[0].pitch_spans.len(), 1);

        // 同帧时归零必须排在 NoteOff 之前（稳定的 `sort_by_key` 保持 push 顺序）。
        let reset = ev
            .iter()
            .position(|e| {
                e.frame == 48_000 && matches!(e.event, MidiEvent::NotePitch { cents: 0, .. })
            })
            .unwrap();
        let off = ev
            .iter()
            .position(|e| e.frame == 48_000 && matches!(e.event, MidiEvent::NoteOff { .. }))
            .unwrap();
        assert!(reset < off, "归零事件要先于 NoteOff");
    }

    /// 揉弦按控制率展开成多条事件，而不是只发起止两点。
    #[test]
    fn vibrato_expands_at_the_control_rate() {
        let mut p = project_with_note();
        p.tracks[0].clips[0].notes[0].duration = 2.0;
        p.tracks[0].clips[0].notes[0].expr = crate::model::PitchExpression {
            vibrato: 40,
            vibrato_rate: 5.5,
            ..crate::model::PitchExpression::NONE
        };
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let pitches: Vec<i16> = s.tracks[0]
            .events
            .iter()
            .filter_map(|e| match e.event {
                MidiEvent::NotePitch { cents, .. } => Some(cents),
                _ => None,
            })
            .collect();
        assert!(pitches.len() > 20, "2 拍 @100 Hz 应产生足够多的采样点");
        assert!(pitches.iter().any(|c| *c > 10), "应出现正半周");
        assert!(pitches.iter().any(|c| *c < -10), "应出现负半周");
    }

    /// 连奏：NoteOff 延到**同音高**的下一个音发声处；没有下一个就按原时值。
    #[test]
    fn legato_holds_the_note_until_the_next_same_pitch() {
        let mut p = project_with_note();
        // A：1.0 拍起、0.5 拍长、连奏；B：同音高、2.5 拍起。
        p.tracks[0].clips[0].notes[0].duration = 0.5;
        p.tracks[0].clips[0].notes[0].legato = true;
        p.tracks[0].clips[0].notes.push(Note {
            id: 2,
            channel: None,
            pressure: None,
            pitch: 60,
            start: 2.5,
            duration: 1.0,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: crate::model::PitchExpression::NONE,
        });

        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let offs: Vec<(u64, u8)> = s.tracks[0]
            .events
            .iter()
            .filter_map(|e| match e.event {
                MidiEvent::NoteOff { note } => Some((e.frame, note)),
                _ => None,
            })
            .collect();
        assert_eq!(
            offs,
            vec![(60_000, 60), (84_000, 60)],
            "A 的 NoteOff 必须延到 B 的发声处（2.5 拍 = 60000 帧），而不是 1.5 拍"
        );

        // 只有异音高的下一个音时，连奏不生效（仍按原时值）。
        let mut p2 = project_with_note();
        p2.tracks[0].clips[0].notes[0].duration = 0.5;
        p2.tracks[0].clips[0].notes[0].legato = true;
        p2.tracks[0].clips[0].notes.push(Note {
            id: 2,
            channel: None,
            pressure: None,
            pitch: 62,
            start: 2.5,
            duration: 1.0,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: crate::model::PitchExpression::NONE,
        });
        let s2 = ScheduledEvents::from_project(&p2, 48_000.0);
        let first_off = s2.tracks[0]
            .events
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOff { note: 60 }))
            .unwrap();
        assert_eq!(first_off.frame, 36_000, "异音高不算「下一个」，时值照写");
    }

    #[test]
    fn end_frame_is_project_body_length() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        // clip end = 4 beats = 2.0 s = 96000 frames.
        assert_eq!(s.end_frame, 96_000);
    }

    #[test]
    fn carries_instrument_and_mixer() {
        let s = ScheduledEvents::from_project(&project_with_note(), 48_000.0);
        assert!(s.tracks[0].instrument.is_none());
        assert_eq!(s.tracks[0].mixer.gain_db, 0.0);
    }

    #[test]
    fn carries_master_channel() {
        let mut p = project_with_note();
        p.master.gain_db = -4.5;
        p.master.pan = 0.25;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.master.gain_db, -4.5);
        assert_eq!(s.master.pan, 0.25);
    }

    #[test]
    fn records_the_build_sample_rate() {
        let s = ScheduledEvents::from_project(&project_with_note(), 44_100.0);
        assert_eq!(s.sample_rate, 44_100.0);
    }

    #[test]
    fn note_volume_travels_as_its_own_event() {
        let mut p = project_with_note();
        // 音量 64：力度保持原值，另发一条 NoteVolume。
        p.tracks[0].clips[0].notes[0].volume = 64;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(matches!(
            s.tracks[0].events[0].event,
            MidiEvent::NoteOn {
                note: 60,
                velocity: 100
            }
        ));
        assert!(matches!(
            s.tracks[0].events[1].event,
            MidiEvent::NoteVolume {
                note: 60,
                value: 64
            }
        ));

        // 音量归零：仍要送达（插件据此把该音压到静音）。
        p.tracks[0].clips[0].notes[0].volume = 0;
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(matches!(
            s.tracks[0].events[1].event,
            MidiEvent::NoteVolume { note: 60, value: 0 }
        ));
    }

    #[test]
    fn keyframe_shifts_later_event_frames() {
        use crate::model::{Clip, ClipId, Keyframe, Note, Track, TrackId};
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.transport.upsert_keyframe(Keyframe {
            beat: 2.0,
            tempo: 240.0,
            time_sig_num: 4,
            time_sig_den: 4,
        });
        p.tracks.push(Track {
            meta: None,
            id: TrackId(1),
            name: "L".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch: 60,
                    start: 3.0,
                    duration: 0.5,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: crate::model::PitchExpression::NONE,
                }],
                audio: None,
            }],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        // 3 拍 = 0..2 @120 (1s) + 2..3 @240 (0.25s) = 1.25s = 60000 帧。
        assert_eq!(s.tracks[0].events[0].frame, 60_000);
        // tempo_points: 帧 0 → 120；帧 48000（2 拍 @120 = 1s）→ 240。
        assert_eq!(s.tempo_points[0], (0, 120.0));
        assert_eq!(s.tempo_points[1], (48_000, 240.0));
    }

    #[test]
    fn routing_order_puts_send_targets_after_their_sources() {
        // 0→2、1→2：0 与 1 都排在 2 之前。
        assert_eq!(routing_order(&[vec![2], vec![2], vec![]]), vec![0, 1, 2]);
        // 1→0：0 必须排在 1 之后。
        assert_eq!(routing_order(&[vec![], vec![0]]), vec![1, 0]);
    }

    #[test]
    fn routing_order_survives_cycles() {
        // 环（0↔1）：绝不挂死，两个轨道都仍然被处理。
        let order = routing_order(&[vec![1], vec![0]]);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1]);
    }

    #[test]
    fn sends_and_lanes_are_resolved_into_the_schedule() {
        let mut p = project_with_note();
        p.tracks.push(Track {
            meta: None,
            id: TrackId(2),
            name: "Bus".into(),
            instrument: None,
            effects: vec![],
            clips: vec![],
            mixer: Default::default(),
            sends: vec![],
            automation: vec![],
        });
        // 轨道 1（下标 0）把信号发送给总线（轨道 2，下标 1）。
        p.tracks[0].sends = vec![Send {
            target: TrackId(2),
            gain_db: 0.0,
        }];
        p.tracks[0].automation = vec![AutomationLane {
            param: "gain_db".into(),
            points: vec![AutomationPoint {
                beat: 0.0,
                value: -3.0,
            }],
        }];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.order, vec![0, 1], "发送目标必须排在来源之后");
        assert_eq!(s.tracks[0].sends.len(), 1);
        assert_eq!(s.tracks[0].sends[0].target_index, 1);
        assert!(
            (s.tracks[0].sends[0].gain - 1.0).abs() < 1e-6,
            "0 dB → 单位增益"
        );
        assert_eq!(s.tracks[0].lanes.len(), 1);
        assert_eq!(s.tracks[0].lanes[0].param, LaneTarget::GainDb);

        // 未知参数名不入表；指向不存在轨道的发送被丢掉。
        p.tracks[0].automation = vec![AutomationLane {
            param: "nope".into(),
            points: vec![AutomationPoint {
                beat: 0.0,
                value: 1.0,
            }],
        }];
        p.tracks[0].sends = vec![Send {
            target: TrackId(999),
            gain_db: 0.0,
        }];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(s.tracks[0].lanes.is_empty(), "未知参数不生成车道");
        assert!(s.tracks[0].sends.is_empty(), "悬空发送被丢弃");
        assert_eq!(s.order, vec![0, 1], "没有有效发送时按原顺序");
    }

    #[test]
    fn structurally_impossible_lanes_are_dropped() {
        let mut p = project_with_note(); // 该轨 instrument: None、effects: 空
        p.tracks[0].automation = vec![
            AutomationLane {
                param: "inst:cutoff".into(),
                points: vec![AutomationPoint {
                    beat: 0.0,
                    value: 1.0,
                }],
            },
            AutomationLane {
                param: "fx0:mix".into(),
                points: vec![AutomationPoint {
                    beat: 0.0,
                    value: 1.0,
                }],
            },
            AutomationLane {
                param: "gain_db".into(),
                points: vec![AutomationPoint {
                    beat: 0.0,
                    value: -3.0,
                }],
            },
        ];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(
            s.tracks[0].lanes.len(),
            1,
            "没有乐器 / 没有效果器时，插件参数车道要丢掉"
        );
        assert_eq!(s.tracks[0].lanes[0].param, LaneTarget::GainDb);

        // 装上乐器之后，`inst:` 就该入表。
        p.tracks[0].instrument = Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: Default::default(),
            state: vec![],
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(s.tracks[0]
            .lanes
            .iter()
            .any(|l| matches!(l.param, LaneTarget::Instrument(_))));
    }

    #[test]
    fn channel_pressure_lane_key_parses() {
        assert_eq!(
            LaneTarget::from_key("at:0"),
            Some(LaneTarget::ChannelPressure(0))
        );
        assert_eq!(
            LaneTarget::from_key("at:15"),
            Some(LaneTarget::ChannelPressure(15))
        );
        assert_eq!(LaneTarget::from_key("at:16"), None, "通道最大 15");
        assert_eq!(LaneTarget::from_key("at:"), None);
        assert_eq!(LaneTarget::from_key("at:x"), None);
    }

    /// 没有 MPE 时所有音符走主通道 0；显式 `channel` 永远优先。
    #[test]
    fn notes_default_to_channel_zero_and_honor_an_explicit_channel() {
        let mut p = project_with_note();
        p.tracks[0].clips[0].notes.push(Note {
            id: 0,
            channel: Some(5),
            pressure: None,
            pitch: 64,
            start: 1.0,
            duration: 1.0,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: crate::model::PitchExpression::NONE,
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let channels: Vec<u8> = s.tracks[0]
            .events
            .iter()
            .filter(|e| matches!(e.event, MidiEvent::NoteOn { .. }))
            .map(|e| e.channel)
            .collect();
        assert_eq!(channels, vec![0, 5]);
    }

    /// MPE：同一起点重叠的两颗音必须分到不同成员通道；结束后通道可复用。
    #[test]
    fn mpe_gives_overlapping_notes_distinct_member_channels() {
        use crate::model::{MpeConfig, MpeZone};

        let mut p = project_with_note();
        p.tracks[0].meta = Some(TrackMeta {
            mpe: Some(MpeConfig {
                channels: 4,
                zone: MpeZone::Lower,
            }),
            ..TrackMeta::default()
        });
        let note = |pitch: u8| Note {
            id: 0,
            channel: None,
            pressure: None,
            pitch,
            start: 1.0,
            duration: 1.0,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: crate::model::PitchExpression::NONE,
        };
        p.tracks[0].clips[0].notes = vec![note(60), note(64), note(67)];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let mut channels: Vec<u8> = s.tracks[0]
            .events
            .iter()
            .filter(|e| matches!(e.event, MidiEvent::NoteOn { .. }))
            .map(|e| e.channel)
            .collect();
        channels.sort_unstable();
        assert_eq!(
            channels,
            vec![1, 2, 3],
            "Lower 区成员通道从 1 起，且互不相同"
        );

        // 后一个不重叠的音应复用最小空闲通道。
        p.tracks[0].clips[0].notes.push(Note {
            start: 3.0,
            ..note(72)
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let reused = s.tracks[0]
            .events
            .iter()
            .filter(|e| matches!(e.event, MidiEvent::NoteOn { note: 72, .. }))
            .map(|e| e.channel)
            .next()
            .unwrap();
        assert_eq!(reused, 1, "已结束的音符通道会被复用");
    }

    /// 起音压力生成 PolyPressure 事件，且与 NoteOn 同帧、同通道。
    #[test]
    fn note_pressure_expands_into_poly_pressure() {
        let mut p = project_with_note();
        p.tracks[0].clips[0].notes[0].pressure = Some(80);
        p.tracks[0].clips[0].notes[0].channel = Some(2);
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let on = ev
            .iter()
            .position(|e| matches!(e.event, MidiEvent::NoteOn { note: 60, .. }))
            .unwrap();
        let pressure = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::PolyPressure { note: 60, .. }))
            .unwrap();
        assert_eq!(
            pressure.event,
            MidiEvent::PolyPressure {
                note: 60,
                value: 80
            }
        );
        assert_eq!(pressure.frame, ev[on].frame);
        assert_eq!(pressure.channel, 2);
    }

    /// 奏法标签 → keyswitch：切换必须发生在起音之前（提前量），momentary 结束后恢复默认。
    #[test]
    fn articulation_tags_expand_into_keyswitches_before_the_note() {
        use crate::model::{ArticulationSwitch, NoteTag};

        let mut p = project_with_note();
        // project_with_note 的音符 id 是 0（track 1 / clip 1）。
        p.note_tags.insert(
            "1:1:0".into(),
            NoteTag {
                ornament: None,
                articulation: Some("harmonics".into()),
            },
        );
        let mut articulations = std::collections::BTreeMap::new();
        articulations.insert(
            "harmonics".to_string(),
            ArticulationSwitch {
                keyswitch: Some(24),
                momentary: Some(true),
                ..Default::default()
            },
        );
        articulations.insert(
            "normal".to_string(),
            ArticulationSwitch {
                keyswitch: Some(25),
                ..Default::default()
            },
        );
        p.tracks[0].meta = Some(TrackMeta {
            articulations,
            default_articulation: Some("normal".into()),
            ..TrackMeta::default()
        });

        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let on = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOn { note: 60, .. }))
            .unwrap();
        let switch = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::NoteOn { note: 24, .. }))
            .unwrap();
        assert!(
            switch.frame < on.frame,
            "keyswitch 必须在起音之前：{switch:?} vs {on:?}"
        );
        // momentrary：音符结束后切回默认奏法（keyswitch 25）。起点还有一条默认切换，跳过它。
        let restore = ev
            .iter()
            .filter(|e| matches!(e.event, MidiEvent::NoteOn { note: 25, .. }))
            .find(|e| e.frame >= on.frame + 24_000)
            .unwrap();
        assert!(
            restore.frame >= on.frame + 24_000,
            "恢复应发生在音符结束之后"
        );
    }

    /// 导入的多消息奏法：顺序保留、逐事件通道生效、NoteOn 自动配 NoteOff，
    /// 且同一帧内切换事件必须排在音符起音之前。
    #[test]
    fn imported_articulation_messages_keep_order_and_offsets() {
        use crate::model::{ArticulationEvent, ArticulationSwitch, NoteTag};

        let mut p = project_with_note();
        p.note_tags.insert(
            "1:1:0".into(),
            NoteTag {
                ornament: None,
                articulation: Some("slide".into()),
            },
        );
        let mut articulations = std::collections::BTreeMap::new();
        articulations.insert(
            "slide".to_string(),
            ArticulationSwitch {
                events: vec![
                    ArticulationEvent {
                        offset_beats: 0.0,
                        channel: Some(2),
                        event: MidiEvent::ControlChange {
                            controller: 32,
                            value: 5,
                        },
                    },
                    ArticulationEvent {
                        offset_beats: 0.0,
                        channel: None,
                        event: MidiEvent::NoteOn {
                            note: 24,
                            velocity: 1,
                        },
                    },
                ],
                lead_beats: Some(0.0),
                ..Default::default()
            },
        );
        p.tracks[0].meta = Some(TrackMeta {
            articulations,
            ..TrackMeta::default()
        });

        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let index = |predicate: fn(&ScheduledMidi) -> bool| ev.iter().position(predicate).unwrap();
        let cc = index(|e| {
            matches!(
                e.event,
                MidiEvent::ControlChange {
                    controller: 32,
                    value: 5
                }
            )
        });
        let switch_on = index(|e| matches!(e.event, MidiEvent::NoteOn { note: 24, .. }));
        let switch_off = index(|e| matches!(e.event, MidiEvent::NoteOff { note: 24 }));
        let note_on = index(|e| matches!(e.event, MidiEvent::NoteOn { note: 60, .. }));

        assert_eq!(ev[cc].channel, 2, "逐事件通道覆盖音符通道");
        assert!(cc < note_on, "CC 必须在起音之前");
        assert!(switch_on < note_on, "keyswitch 必须在起音之前");
        assert!(switch_off > switch_on, "自动 NoteOff 必须在 NoteOn 之后");
        assert!(ev[switch_off].frame > ev[switch_on].frame);
    }

    /// 通用事件表与音符事件一起排序，通道按条目走。
    #[test]
    fn midi_events_are_merged_and_sorted() {
        use crate::model::MidiEventAt;

        let mut p = project_with_note();
        p.tracks[0].meta = Some(TrackMeta {
            midi_events: vec![MidiEventAt {
                beat: 3.0,
                channel: 4,
                event: MidiEvent::ControlChange {
                    controller: 64,
                    value: 0,
                },
            }],
            ..TrackMeta::default()
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        let ev = &s.tracks[0].events;
        let cc = ev
            .iter()
            .find(|e| matches!(e.event, MidiEvent::ControlChange { .. }))
            .unwrap();
        assert_eq!(cc.frame, 72_000, "第 3 拍 = 72000 帧");
        assert_eq!(cc.channel, 4);
        assert!(
            ev.windows(2).all(|w| w[0].frame <= w[1].frame),
            "事件必须按帧有序"
        );
    }
}
