# 域：分析与导出（analysis）

这是 AI 的「听感」闭环：渲染当前的工程 → 得到量化指标 → 据此调整。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `analyze_project` | `sample_rate` | 离线渲染并返回响度（LUFS/LRA/真峰值）+ RMS + 平均幅度谱。 |
| `export_wav` | `path`,`sample_rate` | 渲染并导出 WAV（含 0.5 s 尾音）。 |
| `master_project` | `path`,`target_lufs`,`ceiling_dbtp`,`stereo_width` | **母带处理**（纯 DSP）：响度归一 + 真峰值限幅 + 立体声宽度，写成 WAV，并返回处理前后的响度与真峰值。 |

**要点**
- `analyze_project` 与导出**逐位一致**（同一张图、同一套确定性渲染）。
- 响度类字段在静音时是 `null`（-∞ 经 JSON）。
- 闭环示例：`analyze_project` 看到响度偏低 → `set_master_gain` → 再分析一次确认。
- `master_project` 的默认值：`target_lufs` = -14、`ceiling_dbtp` = -1、`stereo_width` = 1（原样）。
  想「响度到 -14 且不削顶」就一次调用搞定，不用手算增益；返回的 `report` 里有 `gain_db`（施加的
  归一增益）与 `reduced_db`（限幅器压了多少），据此判断是否需要回头调整混音。
