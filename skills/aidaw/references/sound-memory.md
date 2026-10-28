# 域：音色记忆库（sound-memory）

用户捏出满意的音色后，可以把**整条音色链**存进记忆库：乐器参数 + 有序效果链 +
轨道增益/声像。记忆跨工程；下次做同类音色先查这里，别从零重捏。

**人声也在这个库里**：歌声合成的参考声线存成带 `voice` 字段、`vocal` 标签的条目
（`list_voices` / `save_voice` 是它的投影）。人声条目没有乐器/效果链，**不能** `apply_sound_memory`
到轨道；要唱歌就在 `sing_song` 里用 `reference_voice` 引用。

## 工具

| 工具 | 用途 |
| --- | --- |
| `list_sound_memories(query?, tag?, plugin_id?, limit?)` | 查摘要（id / 名称 / 描述 / 标签 / 乐器 / 效果链） |
| `get_sound_memory(id)` | 看完整快照（含参数与插件状态） |
| `save_sound_memory(track, name, description?, tags?, id?)` | 从**当前轨道**抓快照；传 `id` = 覆盖已有记忆的内容 |
| `update_sound_memory(id, name?, description?, tags?, role?)` | 只改元数据，不动声音 |
| `delete_sound_memory(id)` | 删除 |
| `apply_sound_memory(id, track)` | 应用到轨道（一个撤销步）；人声条目会被拒绝 |

`save_sound_memory` **不需要**传参数或效果链：后端直接读轨道，参数再多也不会漏。

## 铁律：默认不保存

- 只有用户**明确要求**（「记住这个音色」「存进音色库」）才保存。
- 用户表达满意（「这个音色真好」「就用这个」）时，可以问**一次**
  「要存进音色库吗？」；用户没确认就不存，也不要反复追问。
- 绝不自动删除 / 清理；`delete_sound_memory` 只响应用户明确要求。
- 保存时补一句中文 `description` 和 2–5 个 `tags`（角色 / 风格 / 听感，例如
  `pad`、`warm`、`wide`、`cinematic`），不要只写插件名。

## 典型用法

```text
# 用户说「把这个音色记住」
save_sound_memory(track=3, name="温暖电影感 Pad",
  description="慢起音、宽立体声，适合副歌铺底", tags=["pad","warm","wide"])

# 下次要同类音色：先查
list_sound_memories(query="温暖 pad")

# 应用到当前轨道
apply_sound_memory(id="s1760000000000", track=5)
```

## 应用语义

- 一次事务写入：乐器、整条效果链、`gain_db` / `pan`，只占**一个撤销步**。
- 效果链是**整体替换**，不是叠加。
- 不保存发送、自动化、片段、静音/独奏。
- 外部插件的二进制状态会尽量恢复；它不参与撤销/重做重放（引擎现有语义）。
- 快照里的插件在本机不存在时，应用后会得到一条「缺失插件」轨，和打开老工程一样。

## 查找建议

- 按听感 / 角色搜：`query="warm"`、`tag="pad"`。
- 按乐器或效果器筛：`plugin_id="builtin.synth"` / `plugin_id="builtin.reverb"`。
- 列表只给摘要；要参数细节再 `get_sound_memory(id)`。
