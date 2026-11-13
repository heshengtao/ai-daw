//! MIDI 输入：枚举本机端口、维护一条输入连接，并把消息广播给前端。
//!
//! 架构上刻意**不放进音频线程**：`midir` 的回调由操作系统的 MIDI 线程驱动，它只需要把
//! 解析后的消息通过 Tauri 事件送出去；真正的发声仍走既有的 `audition_note` 命令队列，
//! 因此这里不需要碰实时音频回调，也不会破坏「音频回调零分配」那套约束。
//!
//! 设备生命周期与音频输入一致：**用户选了才打开**（避免启动时就占用端口），关闭 / 换设备
//! 时先丢掉旧连接再建新的。前端把选过的设备名记在本地，下次启动会重新打开它。

use std::sync::Mutex;

use midir::{MidiInput, MidiInputConnection};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// MIDI 消息广播的前端事件名。
pub const EVENT_MIDI: &str = "engine://midi";

/// 一个可用的 MIDI 输入端口。
#[derive(Debug, Clone, Serialize)]
pub struct MidiInputDeviceInfo {
    /// 端口显示名（`midir` 给出的系统名称）。
    pub name: String,
}

/// 解析后的一条 MIDI 消息。
///
/// 只广播前端会用到的几类：音符、CC、弯音、音色切换。系统实时消息（时钟 / 走带）不转发，
/// 它们频率高、又和工程的走带没有关系，转发只会白白占满事件通道。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MidiMessage {
    NoteOn {
        note: u8,
        velocity: u8,
        channel: u8,
    },
    NoteOff {
        note: u8,
        velocity: u8,
        channel: u8,
    },
    ControlChange {
        controller: u8,
        value: u8,
        channel: u8,
    },
    /// 14 位弯音值（0..=16383，8192 = 居中）。
    PitchBend {
        value: u16,
        channel: u8,
    },
    ProgramChange {
        program: u8,
        channel: u8,
    },
}

/// 一条带来源设备名的 MIDI 输入事件。
#[derive(Debug, Clone, Serialize)]
pub struct MidiInputEvent {
    /// 消息来自哪个端口（前端用来在设备被换掉后忽略迟到的旧消息）。
    pub device: String,
    pub message: MidiMessage,
}

/// midi 输入连接的管理器（放在 `AppState` 里）。
#[derive(Default)]
pub struct MidiInputManager {
    /// 当前打开的连接；`None` = 没有占用任何端口。
    ///
    /// `MidiInputConnection` 在 drop 时关闭端口，所以换设备只要把它换掉即可。
    connection: Mutex<Option<MidiInputConnection<()>>>,
    /// 当前连接对应的设备名。
    device: Mutex<Option<String>>,
}

impl MidiInputManager {
    /// 当前选中的设备名。
    pub fn device(&self) -> Option<String> {
        self.device.lock().unwrap().clone()
    }

    /// 打开（或替换 / 关闭）输入端口。
    ///
    /// `None` = 关闭当前连接、释放端口；`Some(name)` = 打开同名端口（找不到就报错）。
    /// 先关旧连接再开新连接：两个端口同时打开会让同一颗键响两遍。
    pub fn set_device(&self, app: &AppHandle, device: Option<String>) -> Result<(), String> {
        *self.connection.lock().unwrap() = None;
        *self.device.lock().unwrap() = None;

        let Some(name) = device else {
            return Ok(());
        };

        let input = MidiInput::new("AI DAW").map_err(|e| format!("无法初始化 MIDI：{e}"))?;
        let ports = input.ports();
        let index = ports
            .iter()
            .position(|port| input.port_name(port).ok().as_deref() == Some(name.as_str()))
            .ok_or_else(|| format!("找不到 MIDI 输入设备「{name}」"))?;
        let port = &ports[index];

        let app = app.clone();
        let event_device = name.clone();
        let connection = input
            .connect(
                &port,
                "ai-daw-midi-in",
                move |_stamp, bytes, _| {
                    let Some(message) = parse_message(bytes) else {
                        return;
                    };
                    // 目前界面只用音符。弯音 / CC 可能以 1 kHz 的频率涌进来，全部转发会把
                    // Tauri 事件通道灌满；等界面真的用上它们（比如画 CC 曲线）再放开。
                    if !matches!(
                        message,
                        MidiMessage::NoteOn { .. } | MidiMessage::NoteOff { .. }
                    ) {
                        return;
                    }
                    let _ = app.emit(
                        EVENT_MIDI,
                        MidiInputEvent {
                            device: event_device.clone(),
                            message,
                        },
                    );
                },
                (),
            )
            .map_err(|e| format!("无法打开 MIDI 输入「{name}」：{e}"))?;

        *self.connection.lock().unwrap() = Some(connection);
        *self.device.lock().unwrap() = Some(name);
        Ok(())
    }
}

/// 枚举本机 MIDI 输入端口。
///
/// 每次调用都新建一个短命的 `MidiInput`：枚举本身不占端口，也不会影响已经打开的那条连接。
/// 初始化失败（例如系统 MIDI 服务没起来）时返回空表，界面表现为「没有可用设备」。
pub fn list_devices() -> Vec<MidiInputDeviceInfo> {
    let Ok(input) = MidiInput::new("AI DAW") else {
        return Vec::new();
    };
    input
        .ports()
        .iter()
        .filter_map(|port| input.port_name(port).ok())
        .map(|name| MidiInputDeviceInfo { name })
        .collect()
}

/// 把原始 MIDI 字节解析成一条消息；不认识的（系统实时 / sysex / 数据不完整）返回 `None`。
///
/// 只处理**带状态字节**的完整消息。`midir` 在多数平台会按消息边界回调，因此运行状态
/// （running status）不需要在这里补；真收到残包时宁可丢掉，也不要猜出错误的音高。
pub(crate) fn parse_message(bytes: &[u8]) -> Option<MidiMessage> {
    let status = *bytes.first()?;
    if status < 0x80 {
        return None;
    }
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    match kind {
        0x80 => Some(MidiMessage::NoteOff {
            note: *bytes.get(1)?,
            velocity: *bytes.get(2)?,
            channel,
        }),
        // Note On 的 velocity = 0 按规范等价于 Note Off（很多键盘靠这个省字节）。
        0x90 => {
            let note = *bytes.get(1)?;
            let velocity = *bytes.get(2)?;
            Some(if velocity == 0 {
                MidiMessage::NoteOff {
                    note,
                    velocity: 0,
                    channel,
                }
            } else {
                MidiMessage::NoteOn {
                    note,
                    velocity,
                    channel,
                }
            })
        }
        0xB0 => Some(MidiMessage::ControlChange {
            controller: *bytes.get(1)?,
            value: *bytes.get(2)?,
            channel,
        }),
        0xC0 => Some(MidiMessage::ProgramChange {
            program: *bytes.get(1)?,
            channel,
        }),
        0xE0 => {
            let lsb = *bytes.get(1)? as u16;
            let msb = *bytes.get(2)? as u16;
            Some(MidiMessage::PitchBend {
                value: (msb << 7) | lsb,
                channel,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_on_and_off() {
        assert_eq!(
            parse_message(&[0x90, 60, 100]),
            Some(MidiMessage::NoteOn {
                note: 60,
                velocity: 100,
                channel: 0
            })
        );
        assert_eq!(
            parse_message(&[0x83, 60, 40]),
            Some(MidiMessage::NoteOff {
                note: 60,
                velocity: 40,
                channel: 3
            })
        );
    }

    #[test]
    fn note_on_with_zero_velocity_is_note_off() {
        assert_eq!(
            parse_message(&[0x90, 64, 0]),
            Some(MidiMessage::NoteOff {
                note: 64,
                velocity: 0,
                channel: 0
            })
        );
    }

    #[test]
    fn parses_cc_program_and_pitch_bend() {
        assert_eq!(
            parse_message(&[0xB1, 1, 64]),
            Some(MidiMessage::ControlChange {
                controller: 1,
                value: 64,
                channel: 1
            })
        );
        assert_eq!(
            parse_message(&[0xC2, 5]),
            Some(MidiMessage::ProgramChange {
                program: 5,
                channel: 2
            })
        );
        assert_eq!(
            parse_message(&[0xE0, 0x00, 0x40]),
            Some(MidiMessage::PitchBend {
                value: 8192,
                channel: 0
            })
        );
    }

    #[test]
    fn ignores_realtime_and_truncated_messages() {
        assert_eq!(parse_message(&[0xF8]), None); // clock
        assert_eq!(parse_message(&[0x90, 60]), None); // 缺 velocity
        assert_eq!(parse_message(&[0x01, 0x02]), None); // 没有状态字节
        assert_eq!(parse_message(&[]), None);
    }
}
