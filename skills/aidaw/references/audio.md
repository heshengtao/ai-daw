# 域：音频（audio）

导入/录制音频、看波形、本地 AI（转 MIDI / 降噪 / 分离）。

| 工具 | 参数 | 说明 |
| --- | --- | --- |
| `import_audio` | `path`,`track`,`start` | 把本机音频文件（wav/flac/mp3/ogg/m4a）导入成音频片段，落在 `start` 拍。 |
| `audio_waveform` | `path`,`buckets` | 波形包络（每桶 min/max 两个值），用来确认素材内容。 |
| `list_input_devices` | — | 本机可用的音频输入设备。 |
| `set_input_device` | `device?` | 选择输入设备；不传 = 关闭输入流、释放麦克风。 |
| `record_start` | `track`,`start`,`device?` | 开始录音：打开输入设备并从 `start` 拍走带采集。 |
| `record_stop` | — | 停带、写 WAV，并把录音作为一条可撤销的音频片段挂到轨道上。**必须配对调用**。 |
| `transcribe_audio` | `track` | 音频转 MIDI：结果写成**源轨道下面的新 MIDI 轨**（一次撤销）。有 Basic Pitch 模型就用，否则内置 DSP。 |
| `enhance_audio` | `track`,`mode?` | 降噪：结果写成**源轨道下面的新音频轨**（原轨不动）。`mode=voice`（缺省，人声/去混响，优先 LocalVQE）或 `music`（乐器底噪，内置 DSP）。 |
| `separate_stems` | `track`,`model?` | 音源分离：把该轨第一段音频拆成**鼓 / 贝斯 / 其他 / 人声**四条分轨，插到源轨道下面（一次事务，可整体撤销）。需要已安装 HTDemucs 模型（约 209 MB，在「管理器 → 模型」下载）；没有权重会报错，**没有内置兜底**。 |
| `list_models` | — | 列出本地 AI 模型：id / 用途 / 体积 / 是否随包 / 是否已安装 / 可下载源。 |
| `download_model` | `id`,`confirm` | **高风险**：下载模型权重（可能数百 MB）。必须用户明确同意且 `confirm=true`；否则会被拒绝。 |

**模型下载（高风险，必须先问用户）**

`transcribe_audio` / `enhance_audio` / `separate_stems` / `detect_tempo` 依赖本地模型。
Basic Pitch / LocalVQE 随包；HTDemucs、Beat This 等大模型要按需下载：

1. 先 `list_models` 看 `installed` / `downloadable` / `approx_bytes`。
2. 没装时**先把「需要哪个模型、多大、下载后能做什么、不下载能用什么内置兜底」告诉用户**。
3. 只有用户明确同意后，才调用 `download_model(id, confirm=true)`。下载在后台进行，
   之后用 `list_models` 看 `installed`；不要假报「已下载完成」。
4. 用户不想下载就退回内置方案：转 MIDI / 降噪都有 DSP 兜底；音源分离**没有兜底**，
   只能等模型或换做法（如手工分轨）。

**要点**
- 片段的拍长按素材**自然时长**换算（导入时不变调）；之后拖宽窄或改速度会按比例重采样（**变调**）。
- 素材缺失/损坏时该片段静音，不影响整曲。
- 录完不调用 `record_stop` 不会产生片段；录音期间输入设备被占用，结束时（或不录了）可 `set_input_device` 不传参数释放。
- `transcribe_audio` / `enhance_audio` 处理的是该轨道**第一段音频**，结果都插在源轨下面，原轨不动，便于 A/B。
- `separate_stems` 的结果是四条新的**音频轨**（源轨不动，便于 A/B 对比）；四条轨一次撤销即可整体回退。
