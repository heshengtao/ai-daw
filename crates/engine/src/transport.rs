/// 运行时传输命令（不属于可撤销的工程编辑）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportCommand {
    /// 开始或恢复播放。
    Play,
    /// 暂停并保留当前位置。
    Pause,
    /// 停止并回到起点。
    Stop,
    /// 定位到指定帧。
    Seek { frame: u64 },
}
