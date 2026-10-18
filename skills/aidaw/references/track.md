# 域：轨道（track）

建/删/改轨道，挂换乐器。**所有工具都用 `id` 定位轨道**——`id` 是**分配序号**，不是行号，
也不是「第几条」：新建工程后它从 1 开始，打开旧工程则可能从任意数字继续（老工程里的第一行
就可能是 `id: 93`）。一律以 `get_project` / `add_track` 返回的 `id` 为准。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `add_track` | `name` | 新建一条空轨道；返回事件（含 `id`）。 |
| `remove_track` | `track` | 删除轨道。 |
| `rename_track` | `track`,`name` | 改名。 |
| `set_track_instrument` | `track`,`plugin_id` | 挂/换乐器。`plugin_id` 可用 id 或名字（`JC303`）；回执会说明这件乐器是什么、建议分工与音域。 |
| `set_param` | `track`,`param`,`value` | 设置乐器参数（先 `describe_plugin` 看参数表）。 |
| `apply_preset` | `track`,`params`,`effects` | 一次写整组乐器参数 + 整条效果链（`effects` 是替换）。 |

**要点**
- **要说「第几条轨道」就用 `get_project` 里 `tracks` 数组的顺序**（第 1 个元素 = 界面第一行），
  再把那条轨道的 `id` 传给工具；别把 id 当成序号（`id: 93` 完全可以就是第一行）。
- 挑音色前先 `list_plugins`（`kind="instrument"`）读 `category` / `description` / `roles` /
  `pitch_range`——插件名只是型号，`JC303` 是**单音酸性贝斯**而不是普通合成器。
- 乐器挂上后建议 `set_track_meta` 写 `role` 与 `pitch_range`：`fit_notes_to_range` 会照它折返，
  `analyze_track_overlap` 也会用分工给冲突建议。
- 需要一条「总线」时，直接 `add_track` 出一条空轨，再把别的轨道 `set_sends` 指向它（见 `mix.md`）。
- 乐器缺失（本机没有该插件）时该轨静音，但工程与界面照常。
