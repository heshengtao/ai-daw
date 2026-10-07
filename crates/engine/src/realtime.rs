use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::command_queue::{command_queue, CommandSender};
use crate::player::Player;
use crate::registry::PluginRegistry;
use crate::schedule::ScheduledEvents;
use crate::transport::TransportCommand;

/// 实时宿主选项。
#[derive(Debug, Clone, Copy)]
pub struct RealtimeOptions {
    /// 目标采样率（Hz）。
    pub sample_rate: f64,
    /// 音频回调请求的最大帧数，用于预分配 scratch 缓冲。
    pub max_block: usize,
    /// 控制侧 → 音频侧命令队列容量。
    pub command_capacity: usize,
}

/// 实时宿主错误。
#[derive(Debug, Error)]
pub enum RealtimeError {
    /// 系统没有可用的默认输出设备。
    #[error("no default output device available")]
    NoDevice,
    /// 设备不支持所需的 f32 输出配置。
    #[error("device does not support an f32 stereo output configuration: {0}")]
    UnsupportedConfig(String),
    /// 底层 cpal 流错误。
    #[error("cpal error: {0}")]
    Cpal(String),
}

/// cpal 实时播放宿主：拥有输出流与控制侧命令发送端。
pub struct AudioHost {
    stream: cpal::Stream,
    commands: CommandSender,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
}

impl AudioHost {
    /// 打开默认输出设备并开始播放工程（初始为暂停态，需调用 `play`）。
    pub fn start(
        schedule: ScheduledEvents,
        registry: &PluginRegistry,
        options: RealtimeOptions,
    ) -> Result<Self, RealtimeError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(RealtimeError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| RealtimeError::UnsupportedConfig(e.to_string()))?;
        let channels = supported.channels() as usize;
        let config = supported.config();
        let frames_hint = options.max_block;

        let (tx, mut rx) = command_queue(options.command_capacity);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let position_cb = Arc::clone(&position);
        let playing_cb = Arc::clone(&playing);

        let mut player = Player::new(schedule, registry, config.sample_rate as f64, frames_hint);
        let mut staging: Vec<f32> = Vec::with_capacity(frames_hint * 2);

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    player.drain_commands(&mut rx);
                    let frames = data.len() / channels.max(1);
                    let need = frames * 2;
                    if staging.len() < need {
                        staging.resize(need, 0.0);
                    }
                    let staging = &mut staging[..need];
                    player.process_block(staging, frames);
                    for frame in 0..frames {
                        let l = staging[frame * 2];
                        let r = staging[frame * 2 + 1];
                        let base = frame * channels;
                        if channels == 1 {
                            data[base] = 0.5 * (l + r);
                        } else {
                            data[base] = l;
                            data[base + 1] = r;
                            for c in 2..channels {
                                data[base + c] = 0.0;
                            }
                        }
                    }
                    position_cb.store(player.position_frames(), Ordering::Relaxed);
                    playing_cb.store(player.is_playing(), Ordering::Relaxed);
                },
                move |err| {
                    eprintln!("audio stream error: {err}");
                },
                None,
            )
            .map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        stream.play().map_err(|e| RealtimeError::Cpal(e.to_string()))?;

        Ok(AudioHost { stream, commands: tx, position, playing })
    }

    /// 发送 `Play` 命令，返回是否成功入队（队列满时不阻塞地返回 `false`）。
    pub fn play(&mut self) -> bool {
        self.commands.send(TransportCommand::Play)
    }

    /// 发送 `Pause` 命令，返回是否成功入队。
    pub fn pause(&mut self) -> bool {
        self.commands.send(TransportCommand::Pause)
    }

    /// 发送 `Stop` 命令（停止并回到起点），返回是否成功入队。
    pub fn stop(&mut self) -> bool {
        self.commands.send(TransportCommand::Stop)
    }

    /// 发送 `Seek` 命令定位到指定帧，返回是否成功入队。
    pub fn seek(&mut self, frame: u64) -> bool {
        self.commands.send(TransportCommand::Seek { frame })
    }

    /// 音频线程最近一次报告的播放位置（帧）。
    pub fn position_frames(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// 音频线程最近一次报告的播放状态。
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }
}

impl Drop for AudioHost {
    /// 宿主析构时显式暂停底层输出流，确保设备在控制侧退出后立即静音。
    fn drop(&mut self) {
        let _ = self.stream.pause();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};

    fn schedule() -> ScheduledEvents {
        let mut p = Project::new("rt");
        let mut track = Track {
            id: TrackId(1),
            name: "Lead".into(),
            instrument: Some(PluginInstance { plugin_id: "builtin.synth".into(), params: Default::default(), state: vec![] }),
            clips: vec![],
            mixer: Default::default(),
        };
        track.clips.push(Clip {
            id: ClipId(1),
            start: 0.0,
            length: 1.0,
            notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100 }],
        });
        p.tracks.push(track);
        ScheduledEvents::from_project(&p, 48_000.0)
    }

    // 依赖真实音频设备的冒烟测试：在无设备环境（CI）应返回错误而非 panic。
    #[test]
    fn start_returns_result_without_panicking() {
        let options = RealtimeOptions { sample_rate: 48_000.0, max_block: 2048, command_capacity: 64 };
        match AudioHost::start(schedule(), &PluginRegistry::with_builtins(), options) {
            Ok(mut host) => {
                assert!(host.play());
                assert_eq!(host.position_frames(), 0);
            }
            Err(e) => {
                assert!(
                    matches!(e, RealtimeError::NoDevice | RealtimeError::UnsupportedConfig(_) | RealtimeError::Cpal(_)),
                    "unexpected error variant: {e:?}"
                );
            }
        }
    }
}
