# 域：轨道（track）

建/删/改轨道，挂换乐器。新增轨道会拿到一个自增 `id`（后续所有操作都用它定位）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_track` | `name` | 新建一条空轨道；返回事件（含 `id`）。 |
| `remove_track` | `track` | 删除轨道。 |
| `rename_track` | `track`,`name` | 改名。 |
| `set_track_instrument` | `track`,`plugin_id` | 挂/换乐器；`builtin.synth*` 是内置合成器及预设。 |
| `set_param` | `track`,`param`,`value` | 设置乐器参数（先 `plugin_params` 看参数表）。 |
| `apply_preset` | `track`,`preset_id` | 套用一条用户预设（整套音色 + 效果链，一次撤销步骤）。 |

**要点**
- 需要一条「总线」时，直接 `add_track` 出一条空轨，再把别的轨道 `set_sends` 指向它（见 `mix.md`）。
- 乐器缺失（本机没有该插件）时该轨静音，但工程与界面照常。
