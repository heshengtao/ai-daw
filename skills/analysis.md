# 域：分析与导出（analysis）

这是 AI 的「听感」闭环：渲染当前的工程 → 得到量化指标 → 据此调整。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `analyze_project` | `sample_rate` | 离线渲染并返回响度（LUFS/LRA/真峰值）+ RMS + 平均幅度谱。 |
| `export_wav` | `path`,`sample_rate` | 渲染并导出 WAV（含 0.5 s 尾音）。 |

**要点**
- `analyze_project` 与导出**逐位一致**（同一张图、同一套确定性渲染）。
- 响度类字段在静音时是 `null`（-∞ 经 JSON）。
- 闭环示例：`analyze_project` 看到响度偏低 → `set_master_gain` → 再分析一次确认。
