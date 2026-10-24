//! 音频片段的端到端验证：定帧对齐、混音求和、按比值重采样（变调）、定位、素材缺失。
//!
//! 断言用 `ai-daw-analysis` 做**量化**核对（频谱峰 / 采样峰值 / RMS），而不是
//! 「>0 就算过」——这类测试才抓得住「声音放错位置」或「比值算反」这类错误。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use ai_daw_analysis::{analyze, Report};
use ai_daw_engine::audio::DecodedAudio;
use ai_daw_engine::model::{
    AudioSource, Clip, ClipId, Note, PluginInstance, Project, Track, TrackId,
};
use ai_daw_engine::player::Player;
use ai_daw_engine::registry::PluginRegistry;
use ai_daw_engine::render::{render, render_with_assets, RenderOptions};
use ai_daw_engine::schedule::ScheduledEvents;
use ai_daw_engine::transport::TransportCommand;

const SR: f64 = 48_000.0;
/// 120 BPM 下 1 拍 = 0.5 s = 24000 帧。
const FRAMES_PER_BEAT: u64 = 24_000;

type Assets = HashMap<String, Arc<DecodedAudio>>;

/// 生成一段立体声正弦素材（左右同相）。
fn sine(freq: f64, amp: f64, seconds: f64) -> Arc<DecodedAudio> {
    sine_at(freq, amp, seconds, SR)
}

/// 与 [`sine`] 相同，但**录在给定采样率**上（测采样率换算用）。
fn sine_at(freq: f64, amp: f64, seconds: f64, sr: f64) -> Arc<DecodedAudio> {
    let frames = (seconds * sr) as usize;
    let mut samples = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let s = (amp * (std::f64::consts::TAU * freq * f as f64 / sr).sin()) as f32;
        samples.push(s);
        samples.push(s);
    }
    Arc::new(DecodedAudio {
        samples: samples.into(),
        channels: 2,
        sample_rate: sr,
    })
}

/// 生成一段**单声道阶梯**素材：第 `f` 帧的值 = `f / frames`。
///
/// 正弦每周期都长一样，定位错半秒看不出来；阶梯的每个位置都有唯一值，因此能精确断言「从哪一帧开始播」。
fn ramp(frames: usize) -> Arc<DecodedAudio> {
    let samples: Vec<f32> = (0..frames).map(|f| f as f32 / frames as f32).collect();
    Arc::new(DecodedAudio {
        samples: samples.into(),
        channels: 1,
        sample_rate: SR,
    })
}

fn assets(path: &str, audio: Arc<DecodedAudio>) -> Assets {
    let mut map = Assets::new();
    map.insert(path.to_string(), audio);
    map
}

/// 一条**不挂乐器**的轨道 + 一个音频片段（特意如此：验证无乐器轨道仍要出声）。
fn project_with_audio(path: &str, start_beats: f64, length_beats: f64) -> Project {
    let mut p = Project::new("audio");
    p.transport.tempo = 120.0;
    p.tracks.push(Track {
        meta: None,
        id: TrackId(1),
        name: "Audio".into(),
        instrument: None,
        effects: vec![],
        clips: vec![Clip {
            id: ClipId(1),
            start: start_beats,
            length: length_beats,
            notes: vec![],
            audio: Some(AudioSource {
                path: path.into(),
                offset_frames: 0,
                frames: None,
                rate: 1.0,
                pitch_semitones: 0.0,
            }),
        }],
        mixer: Default::default(),
        sends: vec![],
        automation: vec![],
    });
    p
}

fn options() -> RenderOptions {
    RenderOptions {
        sample_rate: SR,
        tail_seconds: 0.0,
        settle_seconds: 0.0,
    }
}

fn peak_hz(report: &Report) -> f64 {
    let (bin, _) = report.spectrum.magnitudes_db.iter().enumerate().fold(
        (0usize, f64::NEG_INFINITY),
        |(bi, bd), (i, d)| {
            if *d > bd {
                (i, *d)
            } else {
                (bi, bd)
            }
        },
    );
    bin as f64 * report.spectrum.bin_hz
}

fn max_abs(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |a, b| a.max(b.abs()))
}

#[test]
fn audio_clip_renders_at_natural_pitch_and_level() {
    // 1 秒素材 = 120 BPM 下的 2 拍 → 比值正好 1（仅采样率换算）。
    let project = project_with_audio("/tmp/a.wav", 0.0, 2.0);
    let audio = assets("/tmp/a.wav", sine(1_000.0, 1.0, 1.0));
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );

    assert_eq!(buffer.frames(), 48_000, "1 秒素材应当渲染成 48000 帧");

    let report = analyze(buffer.data(), 2, SR).unwrap();
    assert!(
        (peak_hz(&report) - 1_000.0).abs() < 2.0 * report.spectrum.bin_hz,
        "频谱峰应当在 1 kHz，实际 {} Hz",
        peak_hz(&report)
    );
    let peak = report.loudness.sample_peak_dbfs;
    // 推子归零 + 声像居中 + master 居中 = **单位增益**，所以满幅素材出来还是 0 dBFS。
    assert!(peak.abs() < 0.5, "采样峰值应当≈ 0 dBFS，实际 {peak}");
    assert!(
        (report.rms_dbfs + 3.01).abs() < 0.3,
        "RMS 应当≈ -3.01 dBFS，实际 {}",
        report.rms_dbfs
    );
}

#[test]
fn audio_clip_starts_exactly_at_its_start_beat() {
    // 放在第 4 拍（= 2.0 s = 96000 帧）；前面必须是纯静音。
    let project = project_with_audio("/tmp/a.wav", 4.0, 2.0);
    let audio = assets("/tmp/a.wav", sine(1_000.0, 1.0, 1.0));
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );

    let data = buffer.data();
    let start = (4.0 * FRAMES_PER_BEAT as f64) as usize * 2;
    assert_eq!(max_abs(&data[..start]), 0.0, "片段起点之前必须是纯静音");
    assert!(max_abs(&data[start..]) > 0.9, "片段起点之后必须出声");
}

#[test]
fn resizing_the_box_does_not_change_pitch() {
    // 「拍长」只决定方框；素材**原速**播放——缩短方框只会**截断**，不改音高（与工程 BPM 解耦）。
    let project = project_with_audio("/tmp/a.wav", 0.0, 1.0);
    let audio = assets("/tmp/a.wav", sine(1_000.0, 1.0, 1.0));
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );

    assert_eq!(buffer.frames(), 24_000, "方框减半 → 总长减半（截断）");
    let report = analyze(buffer.data(), 2, SR).unwrap();
    assert!(
        (peak_hz(&report) - 1_000.0).abs() < 2.0 * report.spectrum.bin_hz,
        "截断方框不该改变音高（应保持 1 kHz），实际 {} Hz",
        peak_hz(&report)
    );
}

#[test]
fn tempo_change_does_not_change_audio_pitch() {
    // 同一段音频、同一个方框（拍长），只把工程速度翻倍：音频必须仍是 1 kHz（不再被拉伸）。
    let mut project = project_with_audio("/tmp/a.wav", 0.0, 2.0);
    project.transport.tempo = 240.0;
    let audio = assets("/tmp/a.wav", sine(1_000.0, 1.0, 1.0));
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );
    let report = analyze(buffer.data(), 2, SR).unwrap();
    assert!(
        (peak_hz(&report) - 1_000.0).abs() < 2.0 * report.spectrum.bin_hz,
        "改速度不该改变音高（应保持 1 kHz），实际 {} Hz",
        peak_hz(&report)
    );
}

/// 素材采样率 ≠ 工程采样率时，必须按 `素材率 / 工程率` 重采样读取。
///
/// 回归：早先读指针每输出帧只前进 1 个**源帧**，于是 44.1 kHz 素材在 48 kHz 工程里会快
/// 48000/44100 ≈ 8.8% 并升调，尾端还会被方框截掉（听感正是「最后一小节没声音」）。
#[test]
fn material_at_another_sample_rate_keeps_its_duration_and_pitch() {
    // 1 秒 1 kHz 素材，录在 44.1 kHz；方框 = 120 BPM 下 2 拍 = 1 秒（48 kHz 下 48000 帧）。
    let project = project_with_audio("/tmp/a.wav", 0.0, 2.0);
    let audio = assets("/tmp/a.wav", sine_at(1_000.0, 1.0, 1.0, 44_100.0));
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );

    assert_eq!(buffer.frames(), 48_000, "方框仍是 1 秒");
    let report = analyze(buffer.data(), 2, SR).unwrap();
    assert!(
        (peak_hz(&report) - 1_000.0).abs() < 2.0 * report.spectrum.bin_hz,
        "采样率换算不该改变音高（应仍 1 kHz），实际 {} Hz",
        peak_hz(&report)
    );
    // 尾端必须还有声音：素材没被提前放完。
    let tail = &buffer.data()[44_000 * 2..];
    assert!(
        max_abs(tail) > 0.5,
        "素材尾端不该被截掉，实际峰值 {}",
        max_abs(tail)
    );
}

#[test]
fn seek_realigns_the_audio_cursor() {
    // 阶梯素材：第 f 帧 = f / 48000。定位到 24000 帧后，第一个样本必须正好是 0.5。
    let project = project_with_audio("/tmp/ramp.wav", 0.0, 2.0);
    let audio = assets("/tmp/ramp.wav", ramp(48_000));
    let schedule = ScheduledEvents::from_project_with_assets(&project, SR, &audio);
    let mut player = Player::new(schedule, &PluginRegistry::with_builtins(), SR, 64);

    player.apply_command(TransportCommand::Seek { frame: 24_000 });
    player.apply_command(TransportCommand::Play);

    let mut out = vec![0.0f32; 64 * 2];
    player.process_block(&mut out, 64);

    assert!(
        (out[0] - 0.5).abs() < 1e-3,
        "定位后第一帧应当取自素材 0.5 s 处，实际 {}",
        out[0]
    );
    assert!(
        (out[1] - 0.5).abs() < 1e-3,
        "单声道素材应当复制到左右两声道"
    );
}

#[test]
fn paused_player_does_not_play_audio_clips() {
    // 音频片段是时间轴内容：暂停（没 Play）时不该出声。
    let project = project_with_audio("/tmp/a.wav", 0.0, 2.0);
    let audio = assets("/tmp/a.wav", sine(1_000.0, 1.0, 1.0));
    let schedule = ScheduledEvents::from_project_with_assets(&project, SR, &audio);
    let mut player = Player::new(schedule, &PluginRegistry::with_builtins(), SR, 64);

    let mut out = vec![0.0f32; 64 * 2];
    player.process_block(&mut out, 64);
    assert_eq!(max_abs(&out), 0.0, "未播放时音频片段必须静音");
}

#[test]
fn missing_asset_is_silent_and_does_not_change_length() {
    let project = project_with_audio("/tmp/gone.wav", 0.0, 2.0);
    let empty = Assets::new();
    let buffer = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &empty,
    );
    assert_eq!(buffer.frames(), 48_000, "素材缺失不应改变时间轴长度");
    assert_eq!(max_abs(buffer.data()), 0.0, "素材缺失应当静音");
}

#[test]
fn audio_and_instrument_sum_on_the_same_track() {
    // 同一条轨道：乐器（一段 MIDI 音符）+ 音频片段。两者必须是**相加**而不是互相覆盖。
    let mut project = project_with_audio("/tmp/a.wav", 0.0, 2.0);
    project.tracks[0].instrument = Some(PluginInstance {
        plugin_id: "builtin.synth".into(),
        params: Default::default(),
        state: vec![],
    });
    project.tracks[0].clips.push(Clip {
        id: ClipId(2),
        start: 0.0,
        length: 2.0,
        notes: vec![Note {
            id: 0,
            channel: None,
            pressure: None,
            pitch: 69,
            start: 0.0,
            duration: 1.5,
            velocity: 100,
            volume: 127,
            offset: 0.0,
            legato: false,
            expr: ai_daw_engine::model::PitchExpression::NONE,
        }],
        audio: None,
    });

    let audio = assets("/tmp/a.wav", sine(1_000.0, 0.5, 1.0));
    let with_audio = render_with_assets(
        &project,
        &PluginRegistry::with_builtins(),
        &options(),
        &audio,
    );
    let synth_only = render(&project, &PluginRegistry::with_builtins(), &options());

    assert_eq!(
        with_audio.frames(),
        synth_only.frames(),
        "音频片段不该改变总长度"
    );
    let a = analyze(with_audio.data(), 2, SR).unwrap();
    let b = analyze(synth_only.data(), 2, SR).unwrap();
    assert!(
        b.rms_dbfs > f64::NEG_INFINITY,
        "合成器本身应当出声（否则这条测试没有意义）"
    );
    assert!(
        a.rms_dbfs > b.rms_dbfs + 1.0,
        "叠加音频片段后 RMS 应当明显升高：加音频 {} vs 仅合成器 {}",
        a.rms_dbfs,
        b.rms_dbfs
    );
}
