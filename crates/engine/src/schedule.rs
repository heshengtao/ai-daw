use std::sync::Arc;

use ai_daw_plugin_api::{MidiEvent, ParamId};

use crate::audio::{DecodedAudio, SampleProvider};
use crate::model::{AutomationPoint, MixerChannel, Note, PluginInstance, Project, TrackId};
use crate::player::db_to_linear;
use crate::tempo::TempoMap;

/// 一条自动化车道的**已解析目标**。
///
/// 键的形式（见 `specs/2026-10-10-plugin-param-automation-design.md` §4）：
/// `gain_db` / `pan` / `inst:<param_id>` / `fx<i>:<param_id>`（`i` 从 0 起）。
///
/// 非 `Copy`：`ParamId` 拥有自己的 `String`。**构造只在控制侧 / 调度构建时发生**
/// （`from_key` 会分配），解析好的目标随 `ScheduledLane` 带进播放器，音频线程只做移动。
#[derive(Debug, Clone, PartialEq)]
pub enum LaneTarget {
    /// 轨道推子增益（dB）。
    GainDb,
    /// 轨道声像（-1..=1）。
    Pan,
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
        if let Some(rest) = key.strip_prefix("inst:") {
            return (!rest.is_empty()).then(|| LaneTarget::Instrument(ParamId(rest.to_string())));
        }
        if let Some(rest) = key.strip_prefix("fx") {
            let (index, param) = rest.split_once(':')?;
            if param.is_empty() {
                return None;
            }
            return Some(LaneTarget::Effect { index: index.parse().ok()?, param: ParamId(param.to_string()) });
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
    /// 该帧时刻发出的 MIDI 事件。
    pub event: MidiEvent,
}

/// 一条已定绝对帧、且已接上素材的音频片段。
///
/// 语义（见 `specs/2026-10-09-audio-clip-design.md`）：片段的 `length`（拍）决定时间轴跨度，
/// 素材**铺满**该跨度（`ratio = source_window_frames / span_frames`）——缩放或改速度会按比例
/// 重采样，因此会变调（v1 不做保音高变速）。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduledAudioClip {
    /// 片段起始的绝对帧（含）。
    pub start_frame: u64,
    /// 片段结束的绝对帧（不含）。
    pub end_frame: u64,
    /// 素材内的起始帧（crop 起点，已按素材长度夹紧）。
    pub offset_frames: u64,
    /// 本次要铺开的源帧数（= 素材总帧数 − `offset_frames`）。
    pub source_window_frames: u64,
    /// 已解码素材（构造播放器前加载完毕，播放路径只读）。
    pub audio: Arc<DecodedAudio>,
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
            let mut audio_clips = Vec::new();
            for clip in &track.clips {
                // 音频片段：接素材、定绝对帧。素材缺失 → 整段静音（跳过）。
                if let Some(source) = &clip.audio {
                    let Some(audio) = assets.get(&source.path) else { continue };
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
                        audio,
                    });
                    continue;
                }
                for note in &clip.notes {
                    let on = clip.start + note.start;
                    let off = on + note.duration;
                    events.push(ScheduledMidi {
                        frame: to_frame(on),
                        // 只送「演奏强度」。插件各自解释它（音色 / 振幅）——这是设计如此。
                        event: MidiEvent::NoteOn { note: note.pitch, velocity: note.velocity },
                    });
                    // 单音音量与力度**分开发**：支持 per-note 音量的插件独立处理它，不改变音色。
                    // 默认值 127（不衰减）不需要事件。
                    if note.volume != Note::MAX_VOLUME {
                        events.push(ScheduledMidi {
                            frame: to_frame(on),
                            event: MidiEvent::NoteVolume { note: note.pitch, value: note.volume },
                        });
                    }
                    events.push(ScheduledMidi {
                        frame: to_frame(off),
                        event: MidiEvent::NoteOff { note: note.pitch },
                    });
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
                        LaneTarget::Effect { index, .. } if *index >= track.effects.len() => return None,
                        _ => {}
                    }
                    let points = lane.points.clone();
                    Some(ScheduledLane { param, points })
                })
                .collect();
            // 发送：目标解析成下标；指向不存在轨道或指向自己的发送丢弃。
            let self_index = index_by_id[&track.id];
            let sends: Vec<ScheduledSend> = track
                .sends
                .iter()
                .filter_map(|s| {
                    let target_index = *index_by_id.get(&s.target)?;
                    (target_index != self_index)
                        .then_some(ScheduledSend { target_index, gain: db_to_linear(s.gain_db) })
                })
                .collect();
            edges.push(sends.iter().map(|s| s.target_index).collect());
            tracks.push(ScheduledTrack {
                track_id: track.id,
                instrument: track.instrument.clone(),
                effects: track.effects.clone(),
                mixer: track.mixer,
                events,
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
    use crate::model::{
        AutomationLane, AutomationPoint, Clip, ClipId, Note, Send, Track, TrackId,
    };
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
            Some(LaneTarget::Effect { index: 2, param: ParamId("mix".into()) })
        );
        // 不认识 / 残缺的形式
        assert_eq!(LaneTarget::from_key(""), None);
        assert_eq!(LaneTarget::from_key("nope"), None);
        assert_eq!(LaneTarget::from_key("inst:"), None);
        assert_eq!(LaneTarget::from_key("fx:mix"), None, "下标必须是数字");
        assert_eq!(LaneTarget::from_key("fx1:"), None, "参数名不能为空");
        assert_eq!(LaneTarget::from_key("fx"), None);
    }

    fn project_with_note() -> Project {
        let mut p = Project::new("p");
        p.transport.tempo = 120.0;
        p.tracks.push(Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 1.0, duration: 1.0, velocity: 100, volume: 127 }],
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
        assert!(matches!(ev[0].event, MidiEvent::NoteOn { note: 60, velocity: 100 }));
        assert_eq!(ev[1].frame, 48_000);
        assert!(matches!(ev[1].event, MidiEvent::NoteOff { note: 60 }));
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
            MidiEvent::NoteOn { note: 60, velocity: 100 }
        ));
        assert!(matches!(
            s.tracks[0].events[1].event,
            MidiEvent::NoteVolume { note: 60, value: 64 }
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
        p.transport.upsert_keyframe(Keyframe { beat: 2.0, tempo: 240.0, time_sig_num: 4, time_sig_den: 4 });
        p.tracks.push(Track {
            id: TrackId(1),
            name: "L".into(),
            instrument: None,
            effects: vec![],
            clips: vec![Clip {
                id: ClipId(1),
                start: 0.0,
                length: 4.0,
                notes: vec![Note { pitch: 60, start: 3.0, duration: 0.5, velocity: 100, volume: 127 }],
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
        p.tracks[0].sends = vec![Send { target: TrackId(2), gain_db: 0.0 }];
        p.tracks[0].automation = vec![AutomationLane {
            param: "gain_db".into(),
            points: vec![AutomationPoint { beat: 0.0, value: -3.0 }],
        }];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.order, vec![0, 1], "发送目标必须排在来源之后");
        assert_eq!(s.tracks[0].sends.len(), 1);
        assert_eq!(s.tracks[0].sends[0].target_index, 1);
        assert!((s.tracks[0].sends[0].gain - 1.0).abs() < 1e-6, "0 dB → 单位增益");
        assert_eq!(s.tracks[0].lanes.len(), 1);
        assert_eq!(s.tracks[0].lanes[0].param, LaneTarget::GainDb);

        // 未知参数名不入表；指向不存在轨道的发送被丢掉。
        p.tracks[0].automation = vec![AutomationLane {
            param: "nope".into(),
            points: vec![AutomationPoint { beat: 0.0, value: 1.0 }],
        }];
        p.tracks[0].sends = vec![Send { target: TrackId(999), gain_db: 0.0 }];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(s.tracks[0].lanes.is_empty(), "未知参数不生成车道");
        assert!(s.tracks[0].sends.is_empty(), "悬空发送被丢弃");
        assert_eq!(s.order, vec![0, 1], "没有有效发送时按原顺序");
    }

    #[test]
    fn structurally_impossible_lanes_are_dropped() {
        let mut p = project_with_note(); // 该轨 instrument: None、effects: 空
        p.tracks[0].automation = vec![
            AutomationLane { param: "inst:cutoff".into(), points: vec![AutomationPoint { beat: 0.0, value: 1.0 }] },
            AutomationLane { param: "fx0:mix".into(), points: vec![AutomationPoint { beat: 0.0, value: 1.0 }] },
            AutomationLane { param: "gain_db".into(), points: vec![AutomationPoint { beat: 0.0, value: -3.0 }] },
        ];
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert_eq!(s.tracks[0].lanes.len(), 1, "没有乐器 / 没有效果器时，插件参数车道要丢掉");
        assert_eq!(s.tracks[0].lanes[0].param, LaneTarget::GainDb);

        // 装上乐器之后，`inst:` 就该入表。
        p.tracks[0].instrument = Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: Default::default(),
            state: vec![],
        });
        let s = ScheduledEvents::from_project(&p, 48_000.0);
        assert!(s.tracks[0].lanes.iter().any(|l| matches!(l.param, LaneTarget::Instrument(_))));
    }
}
