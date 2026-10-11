# 域：插件与预设（plugin）

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `list_plugins` | — | 本机已扫描到的 VST3/CLAP 插件目录（含 `id` / `name` / `kind` / `has_editor`）。 |
| `apply_preset` | `track`,`preset_id` | 套用一条用户预设到轨道。 |

**要点**
- 内置音色：`builtin.synth` 是合成器本体；`builtin.synth.lead/.pluck/.bass/.pad` 等是**预设音色**（独立 id）。
- 内置效果器 id：`builtin.delay` / `builtin.reverb` / `builtin.chorus` / `builtin.drive` / `builtin.eq`。
- 加载外部插件的原生界面只在桌面端可用；MCP 侧重参数与状态。
