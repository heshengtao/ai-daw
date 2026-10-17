# MIDI 编曲与音乐理论 MCP 能力 — 设计

- 日期：2026-10-17
- 状态：待用户评审
- 范围：MCP、`aidaw` CLI、共享命令管线、MIDI 编曲分析

## 1. 背景

当前 AI DAW 的 MCP 已经可以读写工程、轨道、片段、音符、混音和传输。实际编曲暴露出三个问题：音符只能逐个新增，修改通常需要删除并重建整段片段；没有 swing、量化、音域和批量变换工具；`analyze_project` 只分析渲染后的响度与频谱，不能检查 MIDI 是否与和弦、音域和其他轨道相容。

这使 AI 可以“写出很多音符”，但难以稳定地完成一首结构完整、音域清楚、和声正确的曲子。

## 2. 目标与非目标

### 目标

- 用一次 MCP 调用完成一组 MIDI 音符的新增、替换和变换，并保留可撤销性。
- 支持 swing、量化、力度变换和音域约束。
- 提供基于小节和弦的 MIDI 和声检查。
- 提供轨道音域、密度和重叠统计，帮助主旋律、伴奏和低音分工。
- 补齐片段移动、缩放、切分、复制等常用编辑。
- 让 CLI、MCP、界面共用同一条命令管线和事件模型。

### 非目标

- v1 不自动替 AI 作曲，不生成完整和弦进行或旋律。
- v1 不实现完整的 Schenker 分析、风格分类或主观“好听度”评分。
- v1 不改变现有拍到时间的线性时间轴模型。
- v1 不引入新的 MIDI 文件格式或外部 DAW 工程格式。

## 3. 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 批量写入 | `add_notes` / `replace_notes` 作为一次事务 | 编曲通常一次产生几十到几百个音符，需要一个撤销步骤 |
| 音符定位 | 继续使用片段内拍坐标 | 与现有 `add_note`、编排网格和速度模型一致 |
| swing 表示 | 以拍为单位的 subdivision、ratio、strength | 既能表达 8 分 swing，也能表达较轻的 shuffle |
| 和声输入 | 调用时传入小节范围和 chord symbol | 不强迫旧工程改变数据结构；后续可持久化为工程元数据 |
| 和声结果 | 返回问题列表，不自动删除音符 | 理论判断有上下文依赖，先让 AI 或用户决定如何修正 |
| 音域检查 | 统计与可选修正分开 | 分析不会意外改动作品；修正由明确的变换命令完成 |

## 4. MCP 工具设计

### 4.1 批量 MIDI 编辑

新增工具：

- `add_notes(track, clip, notes)`：批量新增。
- `remove_notes(track, clip, ids)`：按音符 ID 删除。
- `replace_notes(track, clip, notes)`：原子替换片段内全部音符。
- `update_notes(track, clip, updates)`：按 ID 修改音高、位置、时值和力度。
- `transpose_notes(track, clip, semitones, range?)`：批量移调。
- `set_note_velocity(track, clip, value|curve, range?)`：批量设置力度。

`notes` 使用现有字段，并新增稳定的 `id`。批量工具返回变更后的片段摘要和音符数量。单个 `add_note` 保留为兼容接口，并复用同一内部实现。

### 4.2 节奏与律动

新增工具：

- `quantize_notes(track, clip, grid, strength, range?)`
- `apply_swing(track, clip, subdivision, ratio, strength, range?)`
- `humanize_notes(track, clip, timing, velocity, range?, seed?)`

约束：位置不能小于片段起点，时值必须为正，变换后音符不能超出片段范围。`apply_swing` 只改变指定 subdivision 的第二个子拍，不改变小节长度。

### 4.3 音域与编曲统计

新增只读工具：

- `get_track_note_stats(track, clip?)`
- `analyze_track_overlap(tracks, clip?)`

返回音域、平均音高、音符数量、每小节密度、平均力度、重叠音符数和建议的分工冲突。统计不修改工程。

新增可选变换：

- `fit_notes_to_range(track, clip, min_pitch, max_pitch, mode)`

`mode` v1 支持八度折返和报告后不修改两种模式。

### 4.4 和声检查

新增只读工具：

```json
{
  "bars": [
    {"start": 0, "length": 4, "symbol": "Dm9"},
    {"start": 4, "length": 4, "symbol": "G13"}
  ],
  "tracks": [74, 75, 76],
  "policy": {
    "allow_extensions": true,
    "allow_approach_notes": true,
    "allow_non_chord_tone_duration": 0.5
  }
}
```

工具名：`analyze_harmony`。

输出：

- 和弦外音及其小节、拍位、持续时值。
- 不推荐的延伸音。
- 导音未解决和悬挂音。
- 轨道间同音区重叠。
- 每小节冲突等级和可读解释。

v1 支持常见三和弦、七和弦、maj7、m7、m7b5、9、11、13、属七降九及常见变化音。无法识别的 symbol 返回明确错误，不静默放宽规则。

## 5. 片段编辑补齐

新增：

- `move_clip`
- `resize_clip`
- `split_clip`
- `duplicate_clip`
- `merge_clips`

所有操作都经过现有命令总线，事件包含旧值和新值，支持撤销与重做。片段边界、音符相对坐标和音频素材窗口必须在同一事务中保持一致。

## 6. 和弦与段落元数据

第一版不强制修改老工程 JSON。新增可选工程字段：

- `harmony`: 小节范围、和弦符号和来源。
- `sections`: 段落名称、起止拍和角色。
- `track_roles`: `lead`、`chords`、`bass`、`drums` 等。
- `pitch_range`: 轨道目标音域。

旧工程缺少这些字段时，工具继续接受显式参数。字段为空时不序列化，保持旧工程 JSON 形状稳定。

## 7. 实现边界

- `src-tauri/src/commands.rs`：新增批量音符、片段编辑和变换命令，复用现有 `run_agent` / `run_agent_all`。
- `src-tauri/src/mcp.rs`：暴露 MCP 工具、参数校验和错误文本。
- `crates/ipc`：新增音符 ID、批量请求、和弦分析结果和统计结果类型。
- `crates/engine`：实现 swing、量化和纯函数分析；实时播放路径不增加逐块分配。
- `src/api/tauri.ts`、`src/api/mock.ts`：补齐前端 API 与浏览器 mock。
- `skills/aidaw/references/`：新增 `composition.md`，并更新 note、clip、project 文档。
- `crates/aidaw`：无需增加专用子命令；CLI 自动通过 `tools/list` 发现新 MCP 工具。

## 8. 验收标准

### 命令与数据

- 批量新增 1000 个音符只产生一个撤销步骤。
- 替换片段音符后，音符 ID、相对拍坐标和工程快照一致。
- 删除、更新、移调和力度变换可撤销、可重做。
- 片段移动、缩放、切分和复制不破坏音频窗口或 MIDI 相对坐标。

### 律动与音域

- 8 分 swing 在 ratio=0.66 时把每拍第二个子拍放到约 2/3 拍。
- strength=0 不改变音符位置，strength=1 完整应用模板。
- 音域统计能准确报告最低音、最高音、密度和跨轨重叠。
- `fit_notes_to_range` 的八度折返不改变音符所属音级。

### 和声

- Dm9、G13、Cmaj9、Fmaj7、Bm7b5、E7b9、m7、属七降九等测试用例能识别构成音和允许延伸音。
- 明确的和弦外音能返回小节、拍位、音高和持续时间。
- 允许的短经过音不会被误报为严重冲突。
- 未知和弦符号返回可操作错误。

### 工程质量

- Rust 单元测试覆盖归一化、边界、和弦解析和纯函数变换。
- MCP 工具快照测试覆盖参数 schema 和错误响应。
- `cargo test`、`cargo check`、前端 `npm run build` 通过。
- `skills/aidaw` 文档与实际工具列表一致。

## 9. 分阶段实施

1. **阶段一：批量 MIDI 与稳定音符 ID**
   - 数据类型、命令总线、MCP、CLI 工具表、撤销测试。
2. **阶段二：音符变换与片段编辑**
   - 移调、力度、量化、swing、片段移动/复制/切分。
3. **阶段三：统计与和声检查**
   - 音域、密度、跨轨冲突、和弦解析和报告。
4. **阶段四：编曲元数据与 skill 文档**
   - 段落、轨道角色、目标音域、`composition.md` 和前端 API/mock。

## 10. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 批量操作占用过多内存 | 限制单次请求大小，先做 schema 校验，命令内部预分配 |
| 音符 ID 破坏旧工程 | 旧音符加载时按片段顺序补 ID，JSON 字段可选 |
| 和声规则过于武断 | 输出等级和解释，不自动删除；允许显式 policy |
| swing 改动导致片段越界 | 变换后夹在片段范围内，并返回被夹住的数量 |
| 实时线程出现分配 | 变换只在编辑线程执行；播放调度使用已归一化数据 |
| MCP 与界面语义分叉 | 所有写操作复用同一 command/event 管线和共享类型 |
