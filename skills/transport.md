# 域：传输（transport）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `transport_play` | — | 播放（停在结尾再按 → 从头重播）。 |
| `transport_pause` | — | 暂停（**暂停 = 静音**：重置乐器与效果链）。 |
| `transport_stop` | — | 停止并回到起点。 |
| `transport_seek` | `frame` | 定位到绝对帧（采样率见 `get_transport`）。 |
| `transport_set_loop` | `on` | 开关循环（播到结尾无缝回起点）。 |

**要点**
- 位置是**帧**；换算成拍要按 `get_transport` 的采样率与工程的 tempo 分段。
- 设备不可用时这些命令会失败——先 `get_transport` 看 `available`。
