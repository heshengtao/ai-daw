# 域：分析与导出（analysis）

这是 AI 的「听感」闭环：渲染当前的工程 → 得到量化指标 → 据此调整。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `analyze_project` | `sample_rate` | 离线渲染并返回响度（LUFS/LRA/真峰值）+ RMS + 平均幅度谱。 |
| `export_wav` | `path`,`sample_rate` | 渲染并导出 WAV（含 0.5 s 尾音）。 |
| `master_project` | `path`,`target_lufs`,`ceiling_dbtp`,`stereo_width` | **母带处理**（纯 DSP）：响度归一 + 真峰值限幅 + 立体声宽度，写成 WAV，并返回处理前后的响度与真峰值。 |
| `detect_tempo` | `track` | **节拍检测**（Beat This!）：对该轨第一段音频推出**单一全局 BPM + 拍号**写进工程，并把片段首重拍对齐到小节线（速度 / 拍号 / 片段位置**一次事务**，可整体撤销）。需要已安装 Beat This! 模型（`final0`，约 83 MB，在「管理器 → 模型」下载）；没有权重会报错。 |

**要点**
- `analyze_project` 与导出**逐位一致**（同一张图、同一套确定性渲染）。
- 响度类字段在静音时是 `null`（-∞ 经 JSON）。
- 闭环示例：`analyze_project` 看到响度偏低 → `set_master_gain` → 再分析一次确认。
- `master_project` 的默认值：`target_lufs` = -14、`ceiling_dbtp` = -1、`stereo_width` = 1（原样）。
  想「响度到 -14 且不削顶」就一次调用搞定，不用手算增益；返回的 `report` 里有 `gain_db`（施加的
  归一增益）与 `reduced_db`（限幅器压了多少），据此判断是否需要回头调整混音。
- `detect_tempo` **只写一个全局速度**（不写逐小节速度图）——时间轴按「拍」线性绘制，只有恒定
  速度下网格 / 波形才稳定对齐。拍号是「相邻重拍之间的拍数」的众数（分母固定 4）；重拍不足时退回 4/4。
  它同时把片段起点吸附到**下一条小节线**（片段起点不能为负）。
