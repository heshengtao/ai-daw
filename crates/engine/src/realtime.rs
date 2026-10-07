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
///
/// 采样率不在选项中：设备默认输出采样率才是权威来源，[`AudioHost::start`] 会用它，
/// 并在与 [`ScheduledEvents::sample_rate`] 不一致时报错。
#[derive(Debug, Clone, Copy)]
pub struct RealtimeOptions {
    /// 音频回调请求的最大帧数：用于预分配 scratch 缓冲，并作为固定块大小请求设备。
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
    /// 调度表构建时使用的采样率与设备采样率不一致（会导致事件静默错位）。
    #[error("schedule sample rate {schedule} Hz does not match device rate {device} Hz")]
    SampleRateMismatch {
        /// 构建调度表时使用的采样率（Hz）。
        schedule: f64,
        /// 设备默认输出采样率（Hz）。
        device: f64,
    },
    /// 底层 cpal 流错误。
    #[error("cpal error: {0}")]
    Cpal(String),
}

/// 把交错立体声 `staging`（`frames*2`）映射到设备的 `channels` 路交错 `data`（`frames*channels`）。
///
/// 单声道设备取左右声道均值；其它设备把 L/R 写入前两路，其余通道清零。纯函数，无分配。
pub fn write_interleaved(staging: &[f32], data: &mut [f32], frames: usize, channels: usize) {
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
    ///
    /// 只支持 f32 设备输出；调度表的采样率必须与设备采样率一致。
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

        // 只支持 f32 输出：其它采样格式需要额外的转换路径，这里明确拒绝。
        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(RealtimeError::UnsupportedConfig(format!(
                "device default output format is {:?}, only F32 is supported",
                supported.sample_format()
            )));
        }

        let channels = supported.channels() as usize;
        let mut config = supported.config();

        // 事件表以固定采样率定点；与设备采样率不一致会静默错位，必须拒绝。
        if schedule.sample_rate != config.sample_rate as f64 {
            return Err(RealtimeError::SampleRateMismatch {
                schedule: schedule.sample_rate,
                device: config.sample_rate as f64,
            });
        }

        // 请求固定块大小，使回调内 scratch 的预分配即为上界。`.max(1)` 避免块大小为 0
        // 时回调内的分块循环无法推进（防御性；正常调用方不会传 0）。
        let max_block = options.max_block.max(1);
        config.buffer_size = cpal::BufferSize::Fixed(max_block as u32);

        let (tx, mut rx) = command_queue(options.command_capacity);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let position_cb = Arc::clone(&position);
        let playing_cb = Arc::clone(&playing);

        let mut player = Player::new(schedule, registry, config.sample_rate as f64, max_block);
        let mut staging: Vec<f32> = Vec::with_capacity(max_block * 2);

        let stream = device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    player.drain_commands(&mut rx);

                    let frames = data.len() / channels.max(1);
                    let need = frames * 2;
                    // 一次性增长到设备块大小属可接受的预热；此后不再分配。
                    if staging.len() < need {
                        staging.resize(need, 0.0);
                    }

                    // 即便后端忽略固定块大小，也按 max_block 分块，确保 `Player` 及其
                    // scratch 不会在回调内 resize（分配）。
                    let mut offset = 0usize;
                    while offset < frames {
                        let n = (frames - offset).min(max_block);
                        player.process_block(&mut staging[offset * 2..(offset + n) * 2], n);
                        offset += n;
                    }

                    write_interleaved(&staging[..need], data, frames, channels);

                    // 宿主策略：播放到工程结尾即暂停（而非回卷）。这是 `Player` 不再
                    // 自行自动停止后，「播到结尾即停」行为的归属地。
                    if player.end_frame() > 0 && player.position_frames() >= player.end_frame() {
                        player.apply_command(TransportCommand::Pause);
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

    #[test]
    fn write_interleaved_downmixes_to_mono() {
        // 两帧、双声道输入：L/R 分别为 (1.0, 0.0) 与 (0.0, 1.0)，单声道取均值 0.5。
        let staging = [1.0f32, 0.0, 0.0, 1.0];
        let mut data = [0.0f32; 2];
        write_interleaved(&staging, &mut data, 2, 1);
        assert_eq!(data, [0.5, 0.5]);
    }

    #[test]
    fn write_interleaved_separates_stereo_channels() {
        let staging = [0.25f32, -0.5, 0.75, 1.0];
        let mut data = [0.0f32; 4];
        write_interleaved(&staging, &mut data, 2, 2);
        assert_eq!(data, [0.25, -0.5, 0.75, 1.0]);
    }

    #[test]
    fn write_interleaved_zeroes_extra_channels() {
        // 6 声道设备：ch0=L、ch1=R，其余清零。
        let staging = [0.25f32, -0.5, 0.75, 1.0];
        let mut data = [1.0f32; 12];
        write_interleaved(&staging, &mut data, 2, 6);
        assert_eq!(
            data,
            [0.25, -0.5, 0.0, 0.0, 0.0, 0.0, 0.75, 1.0, 0.0, 0.0, 0.0, 0.0]
        );
    }

    // 依赖真实音频设备的冒烟测试：在无设备环境（CI）应返回错误而非 panic。
    #[test]
    fn start_returns_result_without_panicking() {
        let options = RealtimeOptions { max_block: 2048, command_capacity: 64 };
        match AudioHost::start(schedule(), &PluginRegistry::with_builtins(), options) {
            Ok(mut host) => {
                assert!(host.play());
                assert_eq!(host.position_frames(), 0);
            }
            Err(e) => {
                assert!(
                    matches!(
                        e,
                        RealtimeError::NoDevice
                            | RealtimeError::UnsupportedConfig(_)
                            | RealtimeError::SampleRateMismatch { .. }
                            | RealtimeError::Cpal(_)
                    ),
                    "unexpected error variant: {e:?}"
                );
            }
        }
    }
}
