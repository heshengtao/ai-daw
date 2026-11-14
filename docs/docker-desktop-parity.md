# Docker / 浏览器 / 桌面 三形态一致性排查

排查对象：`ai-daw`（1.7.0）、`ai-cut`（1.0.0）
排查方式：源码逐项比对 + 运行现有无头二进制实测（ai-daw）+ 由 Dockerfile/.dockerignore 推导镜像内容（ai-cut，本机无 docker，未实际构建）。
日期：2026-11-14

**目标口径**：三端功能应当一致；唯一允许的差异只能来自**物理条件**（容器里没有声卡、浏览器里没有原生窗口）。凡是因为"没往这边接"造成的差异，一律算欠债，不算产品边界。

> 2026-11-14 修订：上一版把 ai-cut 的"AI 桌面专属"描述成有意为之，是误读。实际上是上一轮编码把 AI 层整体留在了 `src-tauri`，浏览器端从未接过——同属欠债。

## 0. 先把"三种形态"分开

| 形态 | 说明 | 进程与资源 |
| --- | --- | --- |
| A 桌面 webview | Tauri 窗口里的同一份 Vue 前端 + `invoke` | 桌面进程，全部随包资源/模型/插件宿 host 可用 |
| B 桌面内嵌 web | 桌面进程内起 HTTP 服务（ai-daw `serve_with_app(daemon, Some(app))`，ai-cut `start_local_daemon`），浏览器连它 | **同一进程**，资源相同；差别只在"HTTP 命令面" |
| C Docker 无头 | 只有 `aidawd` / `aicutd` + 前端产物 | 无 AppHandle、无随包资源、无声卡 |

结论：**ai-daw 的 B ≈ A**（同一份实现、同一进程、同一资源，仅 24 条"桌面专属"命令 501）；**C 多出 4 类硬缺口**。
**ai-cut 的 B ≈ C**（都只有 38 条 RPC）；C 还缺随包模型/ORT/CLI。ai-cut 的"web 端"是结构性阉割，不是移植问题。

## 1. 命令面覆盖（排查表 1）

| 项目 | 桌面命令 | web/HTTP 可达 | 真缺 | 说明 |
| --- | --- | --- | --- | --- |
| ai-daw | 247（+`app_ready` 钩子） | 244 个名字（`/commands` = `rpc::COMMANDS` ∪ `cmds::NAMES`） | 1（`save_sound_memory`，前端从不调用） | 其中 24 条返回明确的 501「只在桌面端可用」，13 条"桌面专属"实际由专用 HTTP 分支覆盖 |
| ai-cut | 148（+`app_ready`） | 38（`crates/daemon/src/rpc.rs` 的 match） | 121 个功能命令 | 前端用 `isTauri()` 拦截了绝大部分，不是 400 而是"空值/静默/中文提示" |

ai-daw 的 13 条"桌面专属"逐条核对结果（实测）：`export_audio` / `save_project` / `export_midi` / `export_score` / `render_preview` / `import_project` / `read_import_file` / `import_audio` 由 `crates/daemon/src/server/mod.rs:81-271` 专设分支；`ai_yue_{catalog,install,remove,reveal,cancel}` 由 `runtimes.rs:425-510` 按前缀转发到 `ai_runtime_*`。**只剩 `save_sound_memory` 无 HTTP 入口**（`commands.rs:4992`，MCP 工具走内部实现）。

ai-daw 真正在 web 端被 501 拒绝的 24 条（`crates/daemon/src/server/cmds.rs:522-544`）：插件原生窗口 5、效果器/合成器窗口 3、agent 9、MCP 2、CLI 3、web 访问设置 2。

## 2. ai-daw 逐功能差异（排查表 2）

| 功能 | A 桌面 | B 桌面内嵌 web | C Docker | 类型 | 证据 |
| --- | --- | --- | --- | --- | --- |
| 工程编辑 / 撤销 / 历史 / ops | 同一实现 | 同 | 同 | 等价 | `server/rpc.rs:264,624` |
| 保存 / 打开 / 导入工程 | 原生对话框 | 上传 + 服务端分配路径 | 同 B | 等价（语义变） | `server/mod.rs:123-271`、`api/dialogs.ts` |
| 导入音频（含大文件） | 原生对话框 | `POST /upload` → `import_audio` | 同 B | 等价 | `server/mod.rs:253-271` |
| 导出 WAV/MP3/MIDI/乐谱 | 任意路径 | `<data>/exports/` + `GET /download` | 同 B | 等价（实测 200） | `server/mod.rs:81-188,375-430` |
| 离线试听 | `render_preview` | `/rpc/render_preview` 返回 WAV | 同 B | 等价（实测 RIFF） | `server/mod.rs:190-216` |
| 实时播放 | cpal 声卡 | 有设备时可用 | **无声卡 → 500，降级离线** | 降级 | `audio.rs:302-309,1115-1129`、`stores/transport.ts:97,237` |
| 录音 | 输入设备 | 有设备时可用 | **无输入设备** | 缺失 | `commands.rs:3681-3696`、`BottomBar.vue:456` |
| 插件扫描 VST3 | helper+probe | 可用（进程内有探针） | **缺 `vst3-host-probe` → 报错** | 缺失（实测） | `host-vst3/src/scan.rs:475-530` |
| 插件扫描 CLAP | probe | 可用 | 探针缺失 → 退回进程内（实测可列出） | 降级 | `scan.rs:640-660` |
| 插件参数 / 效果链 | 参数表 | 同 | 同 | 等价 | `server/cmds.rs:496-500` |
| 插件原生 GUI | Tauri 多窗 | 501 | 501 | 缺失（不可无头） | `server/cmds.rs:532-539` |
| VST3 离线渲染隔离 | helper 子进程 | 可用 | **缺 helper → 静音占位，导出仍"成功"** | 降级（静默） | `daemon/plugins.rs:132-174` |
| 本地 AI 运行时(ORT) | 随包 CPU 基线 | 随包可用 | **镜像内无 dylib，且 UI 不给装** | 缺失 | `tauri.conf.json:43`、`runtimes.rs:425-433`、`ManagerPane.vue:354` |
| 随包 3 个 ONNX | 随包 | 可用 | **无** | 缺失 | `tauri.conf.json:44-46` |
| 模型下载后可用 | 是 | 是 | **否：下载成功却永远显示未安装** | 缺失（bug） | `local-ai/src/models/mod.rs:104-130` 只在 `src-tauri/src/lib.rs:465` 调用；`boot.rs:45-66` 未调 |
| 运行时安装目录 | 配置目录 | 配置目录 | **落到 `$TMPDIR`，重启即丢** | 缺失（bug） | `runtimes.rs:102-116`、`backends.rs:93-95` 仅桌面调用 |
| 奏法映射库 300+ | 随包 | 可用 | **空列表** | 缺失（实测 `[]`） | `commands.rs:2145-2153`、`tauri_api.rs:120-134` |
| 音频转 MIDI | Basic Pitch | 有模型用模型 | **静默退回内置 DSP** | 降级 | `ai.rs:643-661` |
| 降噪 | LocalVQE/DSP | 同 | DSP | 降级 | `ai.rs:679-693` |
| 音源分离 | HTDemucs | 同 | 报"权重未安装" | 缺失 | `ai.rs:356-362` |
| 节拍/BPM | Beat This! | 同 | 报"未安装" | 缺失 | `models/beat_this.rs:43-60` |
| 歌声合成/音色替换 | SoulX | 同 | 报"未安装" | 缺失 | `singing.rs:192-195` |
| 母带 / 响度 / 工程分析 | 纯 DSP | 同 | 同（实测 200） | 等价 | `ai.rs:4-8` |
| 写歌智能体 | 内嵌 | 501 | 501 | 缺失 | `server/cmds.rs:522-530` |
| 内嵌 MCP server | autostart | 未启动 + 501 | 同 | 缺失 | `src-tauri/src/lib.rs:439-445` |
| `aidaw` CLI | 随包 | 501 | 无二进制 | 缺失 | `cmds.rs:531,534,542` |
| 文件选择（插件/LoRA/目录） | 原生框 | **返回 `mock://` 假路径** | 同 | 假成功 | `api/dialogs.ts:194,204,215` |
| 自动更新 | Tauri updater | 静默空操作 | 同 | 降级（合理） | `api/updater.ts:40,46,86` |
| 安全边界 | 回环 | `--lan` 后无鉴权 | 无鉴权，`read_import_file` 任意读（实测 `/etc/hosts` 200） | 风险 | `boot.rs:96-110`、`server/mod.rs:234-251` |

## 3. ai-cut 逐功能差异（排查表 3）

| 功能 | A 桌面 | B/C web | 类型 | 证据 |
| --- | --- | --- | --- | --- |
| 工程读写 / 历史 / 素材 / 导出 / 字体 / 共享会话 | 有 | 38 条同名移植 | **等价** | `rpc.rs:267-468`；`Dockerfile:21-30` |
| 字体安装/删除 | 有 | 有（`font_reveal` 显式拒绝） | 等价 | `rpc.rs:292-338` |
| 字幕 ASR | 有 | 面板可见，点击报"字幕生成需要在桌面应用中使用" | 缺失（友好） | `modelApi.ts:103,121,131` |
| AI 朗读 TTS | 有 | 报"AI 朗读需要在桌面应用中使用" | 缺失（友好） | `modelApi.ts:151,163` |
| 音频 AI（降噪/分离/响度/DSP） | 7 条 | 全友好报错 | 缺失 | `audioAiApi.ts:20-77` |
| 视频 DSP | 2 条 | 友好报错 | 缺失 | `videoDspApi.ts:19,30` |
| 视频 AI 超分/补帧/重构 | 5 条 | 友好报错 | 缺失 | `videoAiApi.ts:21-92` |
| AI 出图 / 生视频（sd.cpp） | 3+3 条 | 友好报错 | 缺失（不建议无头） | `imageGenApi.ts:14-24`、`videoGenApi.ts:14-24` |
| 云端出图 / 云端生视频 / 云端 TTS | 13 条 | 配置空、生成拒绝 | 缺失（**纯 HTTP，能补**） | `cloudGenApi.ts:18-61`、`cloudTtsApi.ts:13-45` |
| 模型管理 / 运行库管理 | 13 条 | `catalog=[]` 空页 | 缺失 | `modelApi.ts:24-67`、`runtimeApi.ts:9-36` |
| sd.cpp 引擎 | 9 条 | 清单空；`install/select/set_backend` **无 isTauri 守护**（潜在裸错） | 缺失 | `sdcppApi.ts:13-45` |
| OFX 9 条 / 插件 8 条 | 有 | 探测 `[]`、安装拒绝、其余静默 no-op | 缺失 | `ofxApi.ts:23-132`、`pluginApi.ts:18-66` |
| 变声 | 5 条 | 友好报错 | 缺失 | `voiceChangeApi.ts:14-54` |
| 剪辑智能体（**产品主张**） | 11 条 | 面板可见；配置保存被静默清空；发送后 `running` 永久 true（转圈不止、无报错） | 缺失（静默失败） | `agentApi.ts:24-39,91-105`、`stores/agent.ts:221-233` |
| MCP / 网络代理 / 后端选择 | 14 条 | 开关翻转但不生效 / 探针恒"Tauri 不可用" | 缺失（静默） | `agentApi.ts:112-197`、`SettingsView.vue:311` |
| 数据目录 / 磁盘余量 | 4 条 | 一句"只在桌面应用里可改"；余量 `null` | 缺失（有提示） | `SettingsView.vue:1987-1989`、`storageApi.ts` |
| CLI 挂载 / app_restart / web 访问 | 6 条 | 提示 / 隐藏 | 不适用 | `SettingsView.vue:1859-1863,2256` |
| 浏览器"只读跟随桌面会话 + 编辑副本" | — | Docker 无桌面会话可跟随 | 设计差异（合理） | `README_zh.md:214-219` |

结构性问题：**ai-cut 的 AI 代码全在 `src-tauri/src/{video_ai,audio_ai,asr,tts,voice_change,sdcpp,ofx,image_gen,video_gen,cloud_gen,cloud_tts,models,runtime,backend,agent,mcp}`，`crates/daemon` 只有 38 条最小内核**，且 `crates/daemon/Cargo.toml` 里没有 `ort` / `sherpa` / `tauri`。所以"加几条 RPC"不够，必须先把 AI 层去 Tauri 化。

## 4. "桌面随包 → Docker 也必须随包"对照（排查表 4）

### ai-daw

| 组件 | 桌面随包 | 体积 | Docker 镜像 | 影响 |
| --- | --- | --- | --- | --- |
| ONNX Runtime 基线（Linux `libonnxruntime.so.1.30.0`） | ✅ `resources/ort/` | ~11 MB | ❌（`fetch:ort` 只在 Tauri beforeBuild 跑） | 所有 ONNX 推理不可用 |
| Basic Pitch 权重 | ✅ | 230 KB | ❌ | 音频转 MIDI 降级 DSP |
| LocalVQE 降噪权重 | ✅ | 6.1 MB | ❌ | 人声降噪降级 DSP |
| Beat This! mel 前端 | ✅ | 270 KB | ❌ | 节拍检测不可用 |
| 奏法映射库 300+ YAML | ✅ | 1.5 MB | ❌ | 奏法映射空列表 |
| `vst3-host-helper` | ✅ externalBin | 2.4 MB | ❌ | 离线渲染静音占位 |
| `vst3-host-probe` | ✅ externalBin | 938 KB | ❌ | VST3 扫描报错 |
| `clap-scan-probe` | ✅ externalBin | 504 KB | ❌ | CLAP 扫描无隔离 |
| `aidaw` CLI | ✅ externalBin | 1.7 MB | ❌ | CLI/MCP 残留 |
| 插件注册表快照 | ✅ | 1.8 MB | ✅ `include_str!` 编进二进制 | 无差异 |
| 前端产物 | ✅ | 3.5 MB | ✅ web stage | 无差异 |
| beat_this / soulx / htdemucs（大模型） | ❌ 按需下载 | 83 MB / 1.3 GB / 209 MB | ❌（同桌面，靠下载） | 下载后本应可用，但见 §2 的"下载目录未注册"bug |

### ai-cut

| 组件 | 桌面随包 | 体积 | Docker 镜像 | 影响 |
| --- | --- | --- | --- | --- |
| Silero VAD v5 | ✅ | 2.3 MB | ❌ | 字幕/静音检测 |
| LocalVQE 降噪 | ✅ | 6.1 MB | ❌ | 音频降噪 |
| Real-ESRGAN x4v3 | ✅ | 4.9 MB | ❌ | 视频超分 |
| YOLOX-nano | ✅ | 3.7 MB | ❌ | 主体跟踪/智能裁剪 |
| YuNet 2023mar | ✅ | 233 KB | ❌ | 人脸检测/自动重构 |
| 权重许可证 | ✅ | 44 KB | ❌ | 分发合规 |
| ONNX Runtime 基线 | ✅ `resources/ort/` | ~11 MB | ❌ | 全部 ONNX 推理 |
| ffmpeg / ffprobe | ✅ 静态 6.0 sidecar | 45.6 / 18.2 MB | ⚠️ 换成 `apk ffmpeg 6.1.2` | 能力等价，构建来源不同（见下） |
| `aicut` CLI | ✅ externalBin | 1.6 MB | ❌ | 外部 AI harness 的 MCP 客户端 |
| 前端产物 | ✅ | — | ✅ | 无差异 |

根因（两仓共通）：**打包清单只有一条流水线（Tauri `bundle.resources` / `externalBin`），Dockerfile 是第二条、且没有被同步维护**。ai-cut 的 `.dockerignore:6` 直接排除 `src-tauri` 整目录，连 `COPY` 都做不到。

## 5. 可以对齐、但目前没做的功能

### 第一档：一行级修复（收益最大）

| # | 内容 | 改动点 |
| --- | --- | --- |
| D1 | 无头进程注册模型搜索目录，否则"下载成功≠已安装" | `crates/daemon/src/boot.rs:45` 后加 `ai_daw_local_ai::models::set_extra_dirs(vec![cfg.data_dir.join("models")])` |
| D2 | 无头进程注册运行时根目录/随包目录/yue 目录，否则装到 `$TMPDIR` 重启即丢 | `boot.rs` 里 `ai_daw_daemon::backends::init_runtime(&app)`（`backends.rs:93-95` 已实现，只有桌面在 `src-tauri/src/lib.rs:407` 调用） |
| D3 | 奏法映射库支持环境变量或数据目录副本 | `commands.rs:2145-2153` 加 env 分支 |
| D4 | ai-daw web 端插件/LoRA/目录选择不再返回 `mock://` 假路径 | `src/api/dialogs.ts:194,204,215` 非 Tauri 返回 `null` + 调用点禁用 |
| D5 | ai-cut 未知命令统一返回"该功能需要桌面版"而不是 `未知命令：X` | `crates/daemon/src/rpc.rs:468` 加桌面专属命令表 |
| D6 | ai-cut 补 `sdcppApi.ts:13-45`、`runtimeApi.ts:13-38`、`storageApi.ts:15-22` 的守卫（目前是裸调用） | 前端 |

### 第二档：镜像随包补齐（用户明确要求的"桌面随包 Docker 也随包"）

| # | 内容 | 改动点 |
| --- | --- | --- |
| P1 | ai-daw：ORT CPU 基线进镜像并注册 | builder 下载 `onnxruntime-linux-x64-1.30.0.tgz`；final `COPY .../libonnxruntime.so.1.30.0 /opt/ort/` + `ENV AIDAW_ORT_BUNDLED=/opt/ort`。**不要放 `/data`**（`VOLUME /data` 会遮住） |
| P2 | ai-daw：3 个随包 ONNX 进镜像 | `COPY --from=builder /src/src-tauri/resources/onnx/ /opt/models/` + `ENV AIDAW_MODELS=/opt/models` |
| P3 | ai-daw：奏法库进镜像 | `COPY .../articulation-maps /opt/articulation-maps` + `ENV AIDAW_ARTICULATION_MAPS=...` |
| P4 | ai-daw：三个插件探针/helper 进镜像，**与 aidawd 同目录** | builder `cargo build -p ai-daw-vst3-helper -p ai-daw-host-vst3 --bins`；final `COPY .../{vst3-host-helper,vst3-host-probe,clap-scan-probe} /usr/local/bin/` |
| P5 | ai-cut：放行并拷入 5 个随包模型 + ORT + 许可证 | 先改 `.dockerignore`（去掉 `src-tauri`，改成 `src-tauri/src`/`src-tauri/target`/`src-tauri/gen`），再 `COPY src-tauri/resources/models /opt/aicut/models`、`COPY .../ort /opt/aicut/ort`；对应 daemon 需支持 `AICUT_MODELS_DIR` / `AICUT_ORT_BUNDLED` |
| P6 | ai-cut：ffmpeg 用桌面同款静态构建而不是 apk 包 | `COPY --from=web /src/node_modules/ffmpeg-static/ffmpeg /usr/local/bin/ffmpeg`（ffprobe 同理）——版本/滤镜集与桌面逐字节一致，省掉一类"我这里导出失败"的漂移 |

### 第三档：中等改造（需要把 AI 层与 Tauri 解耦）

ai-daw：
- `M1` 无头自启 MCP server（`mcp::McpRuntime::autostart`，只监听回环），把 `mcp_status`/`set_mcp_config` 两条 501 换成真实现。注意不要在无头里装 `TauriMainThread`（stub 的 `run_on_main_thread` 只返回 Ok，配合 `main_thread.rs` 的 `rx.recv()` 会死锁）。
- `M2` 无头启用写歌智能体（`AgentConfigRuntime::load` + `AgentRuntime::new(<data>/agent/sessions)`），事件走 SSE。
- `M3` 浏览器录音：`MediaRecorder`/`AudioWorklet` 采 WAV → `POST /upload` → `import_audio`。
- `M4` 无声卡时服务端"离线渲染 + 分块推送"模拟播放，恢复"按播放就出声、播放头会走"的体验（`/audio` 协议不变）。

ai-cut：
- `M5` 云端出图 / 云端生视频 / 云端 TTS（13 条，纯 HTTP，零模型）——`cloud_gen.rs`/`cloud_tts.rs` 只依赖 `AppHandle` 的 emit 与 config dir，换 `Host` 即可。
- `M6` 网络/代理/模型源/HF token（10 条）——`network.rs` 已无 Tauri 导入，只差 daemon 侧的 rustls reqwest。
- `M7` 模型管理器（6 条）+ 运行库管理器（7 条）——`models.rs`/`runtime.rs` 的下载链搬到 `Host`，配 P5 后 Docker 才有"随包 + 可下载"两条腿。
- `M8` ORT 进 daemon（加 `ort` load-dynamic）——M7 之后所有本地推理的前置总开关。
- `M9` 不依赖模型的 ffmpeg 功能：响度、音频 DSP、视频 DSP（`video_dsp.rs` 已是 ffmpeg 管线，直接换 `host.run_stream`）。
- `M10` ASR / TTS / 降噪 / 分离 / 超分 / 补帧 / 重构（依赖 M8 + P5，CPU 可跑但慢，需在 UI 标注）。
- `M11` OFX（`ofx-host` 是纯 Rust + ffmpeg，可进 daemon；musl 下加载第三方 `.ofx` 需实测）。
- `M12` 智能体（**架构级**）：现在写操作走 `tools.rs → ProjectBridge → emit("project-request") → 前端执行 → bridge_respond` 回环，浏览器没有 webview 可回传；要改接 daemon 已有的 `doc_apply` QuickJS 文档引擎（`rpc.rs:405-424`）。

### 第四档：不建议强行对齐（应显式拒绝而不是静默）

- ai-cut `image_gen_*` / `video_gen_*` / `sdcpp_*`：要 GPU + 数 GB 权重，与 176 MB 镜像定位冲突。
- ai-cut `cli_status/install_cli/uninstall_cli`、`app_restart`、`web_access_*`：无头语境无意义。
- ai-daw 插件原生 GUI、自动更新。
- ai-daw 无声卡时的"实时"低延迟（服务端只能模拟）。

## 6. 前端一致性机制本身的问题

1. **判断依据不一致**：ai-cut 用 `isTauri()`（159 处）决定隐藏/报错，ai-daw 用 `isTauri()`（51 处）。两端都缺"服务端能力表"。
   - ai-cut 已经在 `/health` 里给了 `nativeUi: host.has_ui()`（`rpc.rs:101`），但**前端从未读取**（`grep nativeUi src` 为空）。
   - ai-daw 有 `crates/daemon/src/capabilities.rs` 的能力预检（面向 AI 工具），但 UI 也没用它来门控入口。
2. **静默失败比报错更伤**：ai-cut `agentApi.call()` 在非 Tauri 直接返回 fallback（`agentApi.ts:24-27`），于是"保存智能体配置"看起来成功、实际被清空；`agent_send` 返回后 `running` 永真，界面永久转圈。
3. **文档与实现漂移**：ai-daw `README.md:230` 写"96 条命令"（实际 `/commands` 返回 244）；`Dockerfile:4` 注释还写 `#token=`（README 已明确无 token）；ai-cut `README_zh.md:267-268` 写"浏览器会隐藏这些入口"，实际是点击后才拒绝。
4. **安全**：两仓 web 端都无鉴权（设计如此），但 ai-daw 的 `read_import_file` 可读进程任意路径（实测 `/etc/hosts` 返回 200），在 `--lan` + Docker 端口映射下是实际风险；`/download` 有数据目录边界（403），`read_import_file` 没有。

## 7. 建议的验收口径（"体验一致"怎么量化）

1. `/commands`（ai-daw）/ `/health` 能力表（ai-cut）必须能回答"每个功能在本形态可用/降级/不可用"，前端所有门控读它，不再读 `isTauri()`。
2. 每个桌面命令在 web 侧三选一：**真实现** / **明确 501 + 原因 + 替代路径** / **不暴露**。禁止"空值 fallback"。
3. 打包清单单一来源：`tauri.conf.json` 的 `bundle.resources` + `externalBin` 与 Dockerfile 从同一份清单生成（哪怕先写一个 `scripts/check-bundle-parity.mjs` 做静态检查）。
4. 端到端冒烟（同一套脚本跑三种形态）：新建工程 → 导入音频 → 编/剪 → 加插件 → 导出 → 试听；AI 冒烟：转 MIDI、降噪、分离、母带；ai-cut 再加：字幕、导出、字体、云端出图。
5. 镜像体积预算写进 CI（随包模型/ORT/sidecar 合计约 +30 MB ai-daw、+35 MB ai-cut，可接受）。

## 8. 一句话总结

- **ai-daw**：主链（工程→混音→导出→试听）已与桌面等价；差距集中在"随包资源没进镜像"+"无头没初始化资源目录"+"插件 sidecar/声卡/agent/MCP"四类。
- **ai-cut**：web 端只有 38/148 条命令——不是"移植没做完"，而是 **AI 层从未离开 `src-tauri`**。要一致必须先把 AI 层与 Tauri 解耦（M8）+ 随包模型（P5）+ 纯 HTTP 功能先上（M5/M6）。

## 9. 全端一致施工单（修订版）

原则：**同一份后端实现 + 同一份随包清单 + 同一张能力表**。三端差异只能出现在"物理上做不到"的条目，并且必须由能力表显式声明。

### M0 已完成（本仓 ai-daw，2026-11-14）

| 项 | 改动 | 状态 |
| --- | --- | --- |
| D1 模型下载目录注册 | `crates/daemon/src/boot.rs` 新增 `init_headless_dirs`，注册 `<data>/models` | ✅ 已改并实测（`/rpc/model_catalog` 能认到 `<data>/models` 里的文件） |
| D2 运行时根目录注册 | 同上，调用 `backends::init_runtime`（含 ORT 根 + YuE 引擎根 + 后端偏好） | ✅ 已改并实测（`ai_runtime_catalog.dir` 从 `$TMPDIR` 变为 `<data>/runtime/...`） |
| D3 奏法库路径 | `commands.rs::articulation_maps_root` 增加 `AIDAW_ARTICULATION_MAPS` 与 exe 同级候选 | ✅ 已改并实测（环境变量指向后返回 200 份映射） |
| P1 ORT 随包 | Dockerfile builder 拉取 `onnxruntime-linux-x64-1.30.0`，final `COPY` 为 `libonnxruntime.so.1.30.0` + `ENV AIDAW_ORT_BUNDLED=/opt/ort` | ✅ 已改（版本/文件名与 `runtime.rs` 的 `required` 一致；**镜像未构建验证**，本机 Docker Hub 不可达） |
| P2 3 个 ONNX 随包 | `COPY src-tauri/resources/onnx/ /opt/models/` + `ENV AIDAW_MODELS_EXTRA=/opt/models` | ✅ 同上 |
| P3 奏法库随包 | `COPY articulation-maps /opt/articulation-maps` + `ENV AIDAW_ARTICULATION_MAPS=...` | ✅ 同上 |
| P4 插件 sidecar 随包 | builder 构建 `ai-daw-vst3-helper` / `ai-daw-host-vst3 --bins`，final 与 `aidawd` 同目录 | ✅ 三个 bin target 本机构建通过；Linux 链接未验证 |

> 遗留验证项：`docker build` 未跑（`auth.docker.io` 超时）。恢复网络后执行 `docker build -t ai-daw:parity . && docker run --rm -p 28430:28430 ai-daw:parity`，再按 §7 的冒烟清单核对 `/commands`、`list_articulation_maps`、`ai_runtime_catalog`、`plugin_scan_status`。

### M1 能力契约（两端都要，1–2 天）

1. 统一 `/capabilities`：每项 `{ key, title, state: "ok"|"degraded"|"desktop-only"|"missing-component", reason, fix, fallback }`。
   - ai-daw 已有 `crates/daemon/src/capabilities.rs`（面向 AI 工具），扩成 HTTP 端点即可。
   - ai-cut 目前只有 `/health` 的 `nativeUi`，且前端从未读取（`grep nativeUi src` 为空）。
2. 前端删除 `isTauri()` 猜测式门控，改读能力表；`isTauri()` 只保留"传输层选择"一处用途。
3. 禁止静默 fallback：`agentApi.call()` 这类"非 Tauri 返回空值"的写法全部改成显式状态。

### M2 ai-cut 纯 HTTP 功能下沉（3–5 天，零模型、收益最快）

| 模块 | 现在 | 目标 | 需要的替换 |
| --- | --- | --- | --- |
| `cloud_gen.rs`（2784 行） | 仅桌面 | daemon 命令 `cloud_gen_*` | `AppHandle` → `&dyn Host`；`app.emit` → `events::publish`；配置落 `host.config_dir()` |
| `cloud_tts.rs`（1573 行） | 仅桌面 | 同上 5 条 | 同上 |
| `network.rs`（1082 行） | 仅桌面 | `network_*` / 模型源 / HF token | 已无 Tauri 导入，仅需 rustls reqwest（daemon 已有） |
| `audio_measure_loudness` / `video_dsp`（554 行） | 仅桌面 | 纯 ffmpeg | `tauri_plugin_shell` → `Host::run_stream`；`probe_media` 已在 daemon |

### M3 ai-cut 模型与运行时下沉（1–2 周，AI 总开关）

| 模块 | 现状 | 迁移要点 |
| --- | --- | --- |
| `models.rs`（4348 行） | 全程 `AppHandle` + `app.path()` | 抽出 `models_dir/search_dirs/installed_dir/require_dir/run_download` 到 `Host`；`resource_dir()` 用新的 `Host::resource_dir()`（headless 返回 `AICUT_MODELS_DIR`） |
| `runtime.rs`（2379 行） | 随包基线 + CUDA 下载 | 同上；CPU/DirectML/CoreML 走"随包"，CUDA 走"下载"，与 ai-daw 口径对齐 |
| `backend.rs`（1747 行） | ORT 会话创建 | 需要 `ort`（load-dynamic）进 `crates/daemon/Cargo.toml` |
| 镜像 | 无模型/ORT | 放行 `src-tauri/resources/models`（`.dockerignore` 目前整目录排除 `src-tauri`），`COPY` 到 `/opt/aicut/{models,ort}` + `ENV AICUT_MODELS_DIR/AICUT_ORT_BUNDLED` |

### M4 ai-cut 本地推理（2–3 周）

`asr.rs`（1079）/ `tts.rs`（1811）/ `audio_ai/` / `video_ai/` / `voice_change/` / `aligner.rs`，依赖 M3。`crates/sherpa-onnx-sys`、`crates/sherpa-separation` 已是独立 workspace、不带 tauri，可直接加进 daemon。CPU-only 下慢，需在能力表里标 `degraded`。

### M5 架构级（按需）

- ai-cut 智能体 / MCP：写操作走 `tools.rs → ProjectBridge → emit("project-request") → 前端执行 → bridge_respond`，浏览器没有 webview。要改成 daemon 侧 `doc_apply`（QuickJS 文档引擎已存在）。
- ai-daw MCP server 无头自启、agent 运行时托管、浏览器录音（MediaRecorder → `/upload` → `import_audio`）、无声卡时的"离线渲染模拟实时播放"。

### 唯一允许保留的差异

| 差异 | 原因 | 呈现方式 |
| --- | --- | --- |
| 原生插件 GUI 不可用（ai-daw） | 无 X 显示 | 能力表 `desktop-only`，前端直接给侧栏参数表，不再重试 12 次 |
| 实时低延迟监听（无声卡） | 无设备 | `degraded`，走离线试听 |
| 系统级操作（ai-cut CLI 挂载、ai-daw 自动更新） | 无进程语义 | `desktop-only` |
| GPU-only 生成（sd.cpp 出图/生视频） | 容器无 GPU 且权重 GB 级 | `degraded`（CPU 可跑但极慢）或明确 `unsupported`，**不能静默** |

## 10. ai-cut 迁移进度

| 里程碑 | 内容 | 状态 | 证据 |
| --- | --- | --- | --- |
| M1 | 能力表契约：`crates/daemon/src/capabilities.rs` + `POST /rpc/capabilities`；前端 `src/platform/capabilities.ts` 启动拉取，写操作走 `requireCapability` 抛人话 | ✅ | `curl /rpc/capabilities` 返回 24 项；`agentApi` 三处静默假成功改掉；531→536 测试全绿 |
| M1.5 | README 口径修正（"桌面专属"→"待迁移"） | ✅ | `README.md` / `README_zh.md` |
| M2-a | `network_*`（10 条）+ HF token + 模型源 + 插件注册表 → daemon；桌面壳改薄转发 | ✅ | 实测 `network_set_config` 落盘 `<配置目录>/network.json` 并可读回；能力表 `network` = `ok` |
| M2-b | `cloud_tts_*`（5 条）下沉（含 `tts_core` 协议/任务锁共享） | ✅ | 实测 `cloud_tts_set_config` 落盘 `tts.json`、`cloud_tts_config` 读回正常；能力表 `cloud_tts` = `ok`；daemon 测试 48 passed |
| M2-c | `cloud_gen_*`（8 条）下沉（含 `image_prep` 共享） | ✅ | 实测 `cloud_gen_presets` 返回 14 套模板、配置落盘 `cloud-gen.json`、`cloud_gen_busy` 正常；能力表 `cloud_gen` = `ok`；daemon 测试 57 passed |
| M2-d | `audio_measure_loudness` + `video_dsp`（纯 ffmpeg）下沉 | ✅ | 新增 `crates/daemon/src/audio_analysis.rs`（decode_segment + ebur128）与 `video_dsp.rs`（`Host::run_stream` + CancellationToken）；实测 1 秒 440Hz/-8dBFS 正弦 → `-9.41 LUFS / -8.72 dBTP`；能力表 `loudness`/`video_dsp` = `ok` |
| M3a | `models.rs`（4348 行）下沉 + `Host::resource_dir()` + 模型 5 条命令上线 | ✅ | daemon 实测 `model_catalog` 36 条、5 个随包模型 `installed=true`（`AICUT_MODELS_DIR` 指向 `src-tauri/resources/models`）；能力表 `model` = `ok`；daemon 93 tests / 桌面 170 tests 全绿 |
| M3b | `runtime.rs`（2379 行）下沉 + 运行库 6 条命令上线 | ✅ | daemon `runtime_catalog` / `runtime_install` / `runtime_remove` / `runtime_cancel` / `runtime_busy` 全部可用；能力表 `runtime` = `ok`；daemon 102 tests（含搬过去的 9 条运行库测试）、桌面 161 tests 全绿 |
| M3b-ort | `ort` load-dynamic 进 daemon + `backend.rs` 会话创建下沉 | ✅ | `crates/daemon/src/ai_session.rs`；桌面只剩薄壳。见 §19.1（含 ort 特性合并的隐蔽坑） |
| M3c | 镜像随包（模型 17 MB + ORT + 许可证） | 🔶 结构已就位 | Dockerfile 已 `COPY resources/models`、`resources/ort`，`.dockerignore` 不再整目录排除 `src-tauri`；`ENV AICUT_RESOURCES/AICUT_MODELS_DIR/AICUT_ORT_BUNDLED`；**镜像未构建**（本机 Docker Hub 不可达） |
| M4 | 本地推理（ASR/TTS/降噪/分离/超分/补帧/变声/对齐） | ✅ | 全部下沉并在 daemon 内实测，见 §19.2 |
| M5 | 智能体 / MCP 改走 `doc_apply` | ✅（工具面子集） | 智能体 11 条命令 + MCP server 已下沉；工具面 11 / 约 40，见 §19.4、§19.6 |

结构说明：`src-tauri` 本来就依赖 `aicut-daemon`，所以不需要新建共享 crate——共用逻辑直接进 daemon，
桌面壳保留 `#[tauri::command]` 薄壳转发（`src-tauri/src/network.rs` 即模板）。
两个执行后端枚举（`BackendPreference` / `SdcppBackend`）随网络配置一起搬进
`crates/daemon/src/ai_backend.rs`，桌面侧 `pub use` 回去，既有调用点不变。

## 11. M3（模型 / 运行时 / ORT 下沉）施工细则

现状盘点（`src-tauri` 侧，全部依赖 `AppHandle`）：

| 文件 | 行数 | Tauri/AppHandle 用法 | 迁移要点 |
| --- | --- | --- | --- |
| `models.rs` | 4348 | `app.path().config_dir()`、`app.path().resource_dir()`、`app.emit("model-progress")`、`app.state::<NetworkRuntime>()` | 抽出 `models_dir/search_dirs/installed/require_dir/download` 到 `Host`；资源目录用新的 `Host::resource_dir()`（无头返回 `AICUT_MODELS_DIR`，缺省 `None`） |
| `runtime.rs` | 2379 | 同上（`runtime-progress`）+ CUDA 套件下载解压 | 同上；`AICUT_ORT_BUNDLED` 已有，补 `Host::resource_dir()` 兜底 |
| `backend.rs` | 1747 | `app` 仅用于拿数据目录与偏好；ORT 会话创建是纯 `ort` | 会话创建搬到 daemon 的 `ai_session` 模块；`ort` 以 `load-dynamic` 加入 daemon（Linux/macOS 差异见 ai-daw 的 `crates/local-ai/Cargo.toml`） |
| 镜像 | — | — | `.dockerignore` 去掉整目录 `src-tauri`（改成 `src-tauri/src`、`target`、`gen`）；`COPY src-tauri/resources/{models,ort}` → `/opt/aicut/…`；`ENV AICUT_MODELS_DIR=/opt/aicut/models`、`AICUT_ORT_BUNDLED=/opt/aicut/ort` |

顺序建议（每步都可独立验证）：
1. `Host::resource_dir()` + `models.rs` 的目录解析先下沉（只读功能：`model_catalog` / `model_probe`），能力表 `model` 先给 `ok`（下载器随后）。
2. `models.rs` 下载链（`model_download` / `model_cancel` / `model_remove` / `model_save_hf_token`）——依赖已下沉的 `network`。
3. `runtime.rs`（清单 / 安装 / 删除 / 探测）+ `ort` 进 daemon（`load-dynamic`，Linux 用 `/opt/aicut/ort/libonnxruntime.so.1.30.0`）。
4. `backend.rs` 的会话创建 + `ai_backend_info/set_backend`；能力表 `runtime` → `ok`。
5. 镜像随包 5 个模型（约 17 MB）+ ORT（约 11 MB）+ 许可证目录。

验收：容器内 `GET /rpc/capabilities` 中 `model`/`runtime` = `ok`；`/rpc/model_catalog` 里 5 个 `bundled=true` 条目 `installed=true`；`/rpc/ai_backend_info` 返回 CPU 可用。

### M3 进度补充（2026-11-14）

- `Host` trait 新增 `resource_dir()`：桌面壳返回 Tauri 资源目录，无头默认读 `AICUT_RESOURCES`。
  模型的搜索顺序因此变成：**下载目录 → 随包资源 → 可执行文件同级 → 开发目录**，桌面与容器同一套。
- `models.rs` 的单元测试里两条"每个视频/图像模型都有规格"的断言依赖桌面的 `video_gen` /
  `image_gen`，已挪到 `src-tauri` 的同名 `spec_coverage` 测试，覆盖没有丢。
- 顺手修掉一个**与本迁移无关的陈旧测试**：`src-tauri/src/web.rs` 的 `info_for(&config, "abc")`
  还在按旧签名（带 token）调用，导致 `cargo test` 根本编不过（`cargo check` 发现不了）。
  现在桌面 `cargo test --lib` 是 170 passed。

### M3 进度补充 2（2026-11-14）

- `runtime_ort`（运行库管理）已整体下沉：清单 / 随包基线判定 / CUDA 成套下载 / 断点续传 /
  `zip`+`tar.gz`+`tar.xz` 解包 / 待删除清理 / 待重启标记。只依赖 `Host` 与 `NetworkRuntime`。
- 「装/删是否需要重启」依赖的**当前执行后端偏好**改成进程级 `OnceLock`：
  桌面壳在 `setup()` 里用 `network.get().backend_preference` 设一次，无头 `aicutd` 从同一份
  `network.json` 设一次——两边语义一致。
- `app_restart` 在 daemon 里明确拒绝（"容器里请重启容器"），而不是假装重启；
  `runtime_reveal` 同理（"需要桌面应用"）。
- 归档解包依赖（`zip` / `tar` / `flate2` / `xz2`）与桌面同版本加入 daemon。

### ai-daw 进度（2026-11-14 第三轮）

| 项 | 内容 | 状态 | 证据 |
| --- | --- | --- | --- |
| ① 命令面 100% | `save_sound_memory` 加入 `tools/rpc_cmds.txt` 并重新生成 `server/cmds.rs` | ✅ | 实测 `/rpc/save_sound_memory` 返回完整 `SoundMemorySummary`；桌面 247 条命令对应 HTTP 覆盖已无缺口 |
| ② 无头 MCP server | `boot::serve()` 里 `init_headless_services`：`McpRuntime::load(<data>/mcp.json)` + `autostart()` + 装入托管表；生成器去掉 `mcp_status`/`set_mcp_config` 的 UNSUPPORTED 与 `McpRuntime` 的 SKIP_STATE | ✅ | 实测容器模式下 MCP 监听 8900，`initialize` 握手返回 `rmcp 3.10.0` + 工具能力；`/rpc/mcp_status` 与 `/rpc/set_mcp_config` 可开可关（关闭即释放端口、删除 `mcp-url` 发现文件） |
| ③ 去掉 `mock://` 假成功 | `api/dialogs.ts` 的 LoRA / 插件包 / 目录选择在浏览器模式改抛明确错误；4 处调用点补 toast | ✅ | `vue-tsc` 通过；不再产出 `mock://…` 假路径 |
| ④ 安全边界 | `read_import_file` 限制在数据目录内（`canonicalize` + 前缀判定） | ✅ | 实测 `/etc/hosts` → 「只能读取数据目录内的文件」；`<data>/…` 正常返回；`..` 逃逸被挡 |

> 说明：`bootstrap()` 是同步的、测试里也在同步上下文调用，而 MCP 启动要 `tokio::spawn`，
> 所以后台服务放在 `serve()`（async、且在开始接客之前）而不是 `bootstrap()`。
> 这样 `cargo test` 不会因为"没有 tokio 运行时"而炸。

### ai-daw 进度（2026-11-14 第四轮）：写歌智能体进无头

| 项 | 内容 | 状态 | 证据 |
| --- | --- | --- | --- |
| `agent_*` 9 条命令 | `init_headless_services` 里装 `AgentConfigRuntime`（`<data>/ai.json`）与 `AgentRuntime`（`<data>/agent/sessions`）；生成器 `SKIP_STATE` 清空并补 `use crate::agent::{...}` | ✅ | 实测 `agent_config` 读、`agent_set_config` 写（落盘 `ai.json`）、`agent_sessions`（`[]`）、`agent_running`（`false`）；桥接表 sync 107 / async 20 / unsupported 12 |
| 工具调用路径 | 智能体的写操作走 `mcp::tools::dispatch`（与内嵌 MCP 同一份实现），**不经 webview 回传** | ✅（结构上已成立） | `crates/daemon/src/agent/mod.rs:662` 起 `dispatch_tool` → `crate::mcp::tools::dispatch` |
| 端到端对话 | 需要一个真的 OpenAI 兼容端点才能跑完整「流式回答 → 工具调用 → 收尾」 | ⏳ 未验证 | 本机没有可用端点；配置 / 会话 / 状态三条链路已实测 |

结论：**ai-daw 的 AI 入口（MCP + 写歌智能体）在容器里已经可用**，剩下的是"需要真模型端点"的验收，
以及浏览器录音、无声卡虚拟播放这两项体验补齐。

### ai-daw 进度（2026-11-14 第五轮）：浏览器录音

| 项 | 内容 | 状态 | 证据 |
| --- | --- | --- | --- |
| 浏览器录音 | `transport.startBrowserRecording` / `stopBrowserRecording`：`getUserMedia` + `MediaRecorder` → 浏览器内解码 → `utils/wav.ts` 编 16-bit WAV → `POST /upload` → `project.importAudio` 落到轨道 | ✅ 代码 + 服务端链路实测 | `vite build` 通过；服务端链路实测：`upload` → `<data>/imports/take.wav` → `import_audio(track=1,start=4)` → 轨道出现片段 |
| 为什么不在服务端录 | 容器里的 daemon 没有声卡，麦克风只存在于浏览器那台机器上；录音必须在前端采集，再作为素材交给后端 | — | — |
| 为什么转 WAV | 容器里导入走 symphonia，webm/opus 不保证可用；WAV 两边都必然支持，且与桌面录音产物同格式 | — | `src/utils/wav.ts` |

未验证：真实浏览器 + 麦克风的端到端（需要 `getUserMedia` 设备；本机无头环境无法跑）。
用 Chrome 的 `--use-fake-device-for-media-stream` 可以自动化，但需要 puppeteer/playwright（当前未安装）。

## 12. ai-cut M5（智能体 / MCP 脱离 webview）施工细则

### 现状：桥接方法面 52 个

`src/core/mcpBridge.ts` 里 `handle(method)` 的分支就是全部方法。按归属分三类：

| 类别 | 方法 | 无头实现路径 |
| --- | --- | --- |
| **文档读写**（10） | `project.get` / `project.apply` / `project.apply_batch` / `project.undo` / `project.clip` / `project.context` / `project.find_clips` / `project.assets` / `project.markers` / `note.list` | 直接落到 daemon 已有的 `doc_snapshot` / `doc_apply`（QuickJS 跑同一份 `assets/domain.js`）；**这是智能体最常用的一类，应最先做** |
| **导出 / 渲染 / 字幕 / 影片**（12） | `export` / `export.frame` / `export.storyboard` / `storyboard` / `contact_sheet` / `render.manifest` / `render.range` / `render.status` / `subtitle.import` / `subtitle.export` / `film.import` / `film.export` | 部分已有 daemon 实现（`export_video` / `write_export_text_image` / `probe_media`）；其余需把 `src/core/export/*` 的纯计算部分下沉，或让前端把「已生成的滤镜图 / 帧图」当参数传进来（与现有 `export_video` 同思路） |
| **AI / 媒体处理**（30） | `cut` / `denoise` / `separate` / `transcribe` / `translate` / `voiceover` / `upscale` / `interpolate` / `reframe` / `process_audio` / `process_video` / `loudness` / `compare.takes` / `ai.*` / `download_model` … | 依赖 M4（本地推理）与已下沉的 `cloud_*` / `video_dsp` / `audio_analysis`；每一条都是"把桌面的实现搬过来"的具体工作 |

### 建议顺序

1. **M5a 文档类十方法**：daemon 新增 `agent_bridge`，把 `project.*` / `note.list` 映射到 `doc_apply` / `doc_snapshot`，并用一个"方法表"明确声明每个方法的实现状态。
2. **M5b 智能体运行时**：`agent_*`（11 条）下沉到 daemon（`config.rs` / `store.rs` / `openai.rs` 基本无 Tauri 依赖，`mod.rs` 只多一个 `AppHandle` 用于发事件）。做完这一步，浏览器里能"对话 + 读工程"。
3. **M5c 处理类方法**：按 M4 进度逐条接；未实现的返回结构化错误（`ok:false` + 原因），不再让前端干等。
4. **M5d MCP server**：ai-cut 的 MCP（`mcp.rs` 677 行）挂在 Tauri 侧，与智能体共用同一份 `tools.rs`；等 M5a–c 完成后把 `tools.rs` 的 dispatch 换成 daemon 版即可。

### 为什么不能只改 `bridge_respond`

`bridge_respond` 只是"Rust 等 webview 回话"的收尾；真正的问题在 `ProjectBridge::request` 依赖 `app.emit("project-request")` —— 浏览器里没有 webview 接单。所以必须**替换整个 request 的落地方式**，而不是补一条命令。

### M5a 已完成（2026-11-14）

`crates/daemon/src/agent_bridge.rs` + RPC `bridge_request` / `bridge_methods`：

| 方法 | 状态 | 落地方式 |
| --- | --- | --- |
| `project.get` / `project.snapshot` | ✅ | `doc_snapshot()` |
| `project.context` | ✅ | `doc_view("overview")` |
| `project.clip` | ✅ | `doc_view("clip", {clipId})` |
| `project.apply` | ✅ | `doc_apply`（payload 即一条 Command） |
| `project.apply_batch` | ✅ | 逐条 apply，失败带序号 |
| `project.undo` / `find_clips` / `assets` / `markers` / `note.list` | ✗（明确报错） | 需扩 domain 视图 |

实测（容器模式）：
```
/rpc/doc_load {"version":1,...}           → {"ok":true,"revision":0}
/rpc/bridge_request {"method":"project.context"} → 概览 JSON（含后续读法提示）
/rpc/bridge_request {"method":"project.get"}     → 完整工程
/rpc/bridge_request {"method":"project.markers"} → 明确"尚未实现（见 §12）"
```

### 本轮结论（诚实记录）

- 这一项无法在一轮里完成：52 个方法里 30 个依赖尚未下沉的 AI/媒体实现，硬做会产出"看起来能用、实际一半失败"的半成品。
- 本轮改为把方法清单、分类、顺序写清楚，并把两仓完整状态重新验证一遍（见下），避免在上下文耗尽前留下半成品。

## 13. 状态矩阵（2026-11-14 收口核验）

两仓在本轮末重新跑了一遍，确认没有留半成品：

| 检查 | ai-cut | ai-daw |
| --- | --- | --- |
| daemon `cargo test --lib` | 102 passed / 2 ignored | 230 passed / 8 ignored |
| 桌面 crate `cargo check` | 0 error | 0 error |
| 前端 `vue-tsc` | 通过 | 通过 |
| 前端 `vitest` | 536 passed / 76 files | （无 vitest 用例） |
| 前端 `vite build` | 通过（早前） | 通过 |
| daemon 是否引入 tauri | 否 | 否（`--no-default-features`） |

### 各形态当前能力（✓ 可用 / ◐ 降级 / ✗ 不可用）

> 这一节是**收口快照**；逐轮的证据与细节见 §19 / §20（ai-cut M3b-ort / M4 / M5d、ai-daw 虚拟时钟）。
> 表里已经按最新实现更新，不再保留过渡期的旧结论。

| 能力 | 桌面 | 浏览器连桌面 | Docker |
| --- | --- | --- | --- |
| ai-daw 工程/编辑/导出/离线试听 | ✓ | ✓ | ✓ |
| ai-daw 实时播放/录音 | ✓ | ✓（浏览器录音） | ✓（**虚拟时钟**：声音走 `/audio`，浏览器当声卡） |
| ai-daw MCP + 写歌智能体 | ✓ | ✓ | ✓ |
| ai-daw 插件扫描/宿主 | ✓ | ✓ | ◐ 扫描可用、探针随包；原生插件 GUI 需显示器 |
| ai-daw 本地 AI（降噪 / 转 MIDI） | ✓ | ✓ | ✓（随包 ORT + 3 个模型，容器内实测降噪） |
| ai-cut 工程/媒体/导出/字体 | ✓ | ✓ | ✓ |
| ai-cut 网络/云端朗读/云端出图/响度/视频处理 | ✓ | ✓ | ✓ |
| ai-cut 模型与运行库管理 | ✓ | ✓ | ✓ |
| ai-cut 本地推理（ASR/TTS/降噪/分离/超分/补帧/变声/对齐） | ✓ | ✓ | ✓（同一份 daemon 实现；容器内实测降噪 / 超分 / ASR / TTS） |
| ai-cut 智能体 / MCP | ✓ 全量工具面 | ✓ 13 个工具 | ✓ 13 个工具（文档类 + 响度 + 视频处理 + 素材预检 + 模型清单） |
| ai-cut OFX | ✓ 含整段烘焙 | ◐ 单帧 / 图片 / 转场预览 | ◐ 同上；整段烘焙明确拒绝 |
| ai-cut 插件安装/导入/删除 | ✓ | ✓ | ✓（reveal / open 明确拒绝） |
| 两侧原生插件 GUI | ✓ | ✗ | ✗（物理不可对齐） |

### 唯一"物理上无法对齐"的三项（应保持明确拒绝，不再假装）

1. 原生插件 GUI 窗口（没有显示器）。
2. 系统级集成：ai-cut 的 CLI 挂载 / `app_restart`；ai-daw 的 `aidaw` CLI 安装、自动更新。
3. 需要 GPU 且权重 GB 级的本地生成（ai-cut 的 sd.cpp 出图/生视频）——CPU 上也"能跑"，但慢到不具备可用性，标 `degraded` 而不是假装支持。

## 14. M5b 已执行（原交接清单，留档）

现状：ai-cut 的 `agent_*` 仍是桌面专属；`crates/daemon/src/agent_bridge.rs` 已提供 5 个文档方法的无头落地。

迁移面（已核实，很小）：

| 文件 | 行数 | Tauri 耦合 | 处理 |
| --- | --- | --- | --- |
| `src-tauri/src/agent/config.rs` | 73 | **0** | 直接搬 |
| `src-tauri/src/agent/store.rs` | 163 | **0** | 直接搬 |
| `src-tauri/src/agent/openai.rs` | 304 | **0** | 直接搬（reqwest 已在 daemon） |
| `src-tauri/src/agent/mod.rs` | 802 | 4 处：1×`app.emit("agent-event")`、1×`tools::dispatch_with_cancel(&app,&bridge,…)`、2×`AppHandle` 字段/参数 | `emit` → `crate::events::publish("agent-event", …)`；工具调用 → `agent_bridge::call(&doc, method, payload)`；去掉 `AppHandle` 字段，`AgentState::new` 改吃 `DocHandle` + `Arc<NetworkRuntime>` |

步骤：
1. `cp src-tauri/src/agent/{config,store,openai}.rs crates/daemon/src/agent/` + 搬 `mod.rs`，按上表替换 4 处。
2. `crates/daemon/src/agent_tools.rs`：把 `tools.rs` 里**文档类**工具的映射抄过来（`get_project→project.context`、`get_context→project.context`、`get_clip→project.clip`、`apply_command→project.apply`、`apply_commands→project.apply_batch`），其余工具名返回 `"工具 X 尚未在无头版本实现"`。
3. `RpcCtx` 增加 `agent: Arc<AgentState>`；`aicutd` 用 `host.config_dir()/agent.json`、`host.data_dir()/agent/sessions` 构造；桌面壳传入 `app.manage` 的同一实例。
4. `rpc.rs` 补 11 条 `agent_*`；带 `State<Arc<AgentState>>` 参数的用 `&ctx.agent`。
5. 能力表 `agent` 改 `degraded`（能对话/读工程/写命令，但工具面未全），并在 `caps.reason` 里说明缺哪些工具。
6. 验证：`/rpc/agent_set_config` 落盘 → `/rpc/agent_config` 读回 → 指向本地假 OpenAI 端点跑一轮 `agent_send`（用 `get_project` 工具）→ 断言 `agent-event` SSE 里有 `tool_call` + `tool_result`。

## 15. 本轮收口

- 本轮（第 10 轮）没有新增代码：M5b 的迁移面已核实并写成上表，但按剩余上下文判断，硬做会在编译/验证中途耗尽预算，留下半成品。
- 两仓状态停在上一轮的绿点：ai-cut daemon 104 tests / 桌面 check 0 error / 前端 536 tests；ai-daw daemon 230 tests / 桌面 check 0 error / `vue-tsc`+`vite build` 通过。
- 下一轮从 §14 第 1 步开始即可，无需重新调研。

### M5b 实际完成情况（2026-11-14）

`crates/daemon/src/agent/`（`config.rs` / `store.rs` / `openai.rs` / `mod.rs`，1342 行）+ `agent_tools.rs`：

| 项 | 结果 |
| --- | --- |
| Tauri 耦合 | 4 处全清：`app.emit` → `events::publish`；`tools::dispatch_with_cancel` → `agent_tools::dispatch`；去掉 `AppHandle` 字段；`tauri::async_runtime::spawn` → `tokio::spawn` |
| 工具分发 | 文档类 5 个工具（`get_project` / `get_context` / `get_clip` / `apply_command` / `apply_commands`）→ `agent_bridge`；其余工具**不列进清单**（模型不会调用），调用时也明确报错指向 §12 |
| 技能文档 | `include_str!("../../../skills/aicut/SKILL.md")`；Dockerfile 构建阶段补 `COPY skills ./skills` |
| RPC | 11 条 `agent_*` 全部可用；`RpcCtx` 新增 `agent`；`aicutd` 与桌面壳各建一份（共用同一份 `DocHandle` 与 `NetworkRuntime`） |
| 能力表 | `agent` → `degraded`（能对话/读工程/写命令，AI 处理类工具未接），原因 i18n：`caps.reason.agentPartialTools` |

实测（容器模式）：
```
/rpc/agent_set_config → {"api_key":"sk-test",...}      落盘 <data>/ai.json
/rpc/agent_sessions   → []  → 发送后 [{"id":"...","title":"你好","count":1}]
/rpc/agent_send "你好" → 返回会话 id；端点不可达时 running 复位为 false，会话落盘 user+assistant 两节点
```

### 顺带修掉一个**会让所有客户端请求崩掉**的 bug

`aicutd` 之前没有安装 rustls crypto provider，而 daemon 的 reqwest 用的是
`rustls-no-provider`：**第一次建 Client 会直接 panic**（智能体对话、云端 TTS/出图、模型下载都会走到）。
现在 `aicutd` 启动时先 `rustls::crypto::ring::default_provider().install_default()`。
这个 bug 是 M5b 实测才暴露出来的——之前那些功能都只验证过"配置读写"，没有真正发出 HTTPS 请求。

### M5c 第一批（2026-11-14）：工具面从 5 个扩到 9 个

- `src/domain/host.ts` 的 `view()` 新增 4 个只读视图：`track` / `find_clips` / `assets` / `markers`，
  全部复用 `core/projectView` 的既有实现（与桌面 `tools.rs` 同一套分页/筛选语义）；domain bundle 重新构建（359.8 KiB）。
- `agent_bridge` 对应新增 `project.track` / `project.find_clips` / `project.assets` / `project.markers`，
  方法表里从 `unsupported` 翻成 `ok`（只剩 `project.undo` 与 `note.list` 仍是 unsupported）。
- `agent_tools` 的工具清单从 5 个扩到 9 个（新增 `get_track` / `find_clips` / `get_assets` / `get_markers`），
  并把桌面 `tools.rs` 里的同名 `ToolDef`（描述 + 参数 schema）原样抄过来——模型看到的工具语义不变。
- 顺手修掉一个参数归一化 bug：`payload` 缺省是 `null` 时，领域层会把 `args` 当 `null` 读（`cannot read
  property 'from' of null`）。`agent_bridge::view` 现在把 `null` 归一成 `{}`，`host.ts` 也顺手加了 `"null"` 判断。

实测：
```
/rpc/bridge_request {"method":"project.markers"}                     → {markers:[], total:0, ...}
/rpc/bridge_request {"method":"project.assets"}                      → {assets:[], total:0, ...}
/rpc/bridge_request {"method":"project.find_clips","payload":{"from":0,"to":10}} → {clips:[], ...}
/rpc/bridge_request {"method":"project.track","payload":{"trackId":1}} → null（空工程没有轨道，符合预期）
```

### M5c 第二批（2026-11-14）：第一个"AI 类"工具

- `AgentState` 增加 `Arc<dyn Host>`（桌面 `TauriHost` / 无头 `HeadlessHost`），工具可以跑 ffmpeg 了。
- `agent_tools` 新增 `measure_loudness`：直接调已下沉的 `audio_analysis::audio_measure_loudness`，
  不经桥、不需要 ORT——这是第一个真正"干活"的 AI 类工具（其余 AI 工具仍卡在 M4）。
- 工具定义从桌面 `tools.rs` 原样抄过来（参数 schema 一致），清单 9 → 10。
- 首尾 shell 都改成传 host：`aicutd` 用 `HeadlessHost::new(data, config)`，桌面壳用 `TauriHost`。

实测：`/rpc/audio_measure_loudness` 在容器模式返回 `-9.41 LUFS / -8.72 dBTP`（1 秒 440Hz 正弦）；
agent 侧的 `measure_loudness` 走同一条实现，完整链路需要真 LLM 端点才能触发（配置/会话/工具分发已分别实测）。

### M5c 第三批（2026-11-14）：视频处理工具

- `AgentState` 增加 `Arc<VideoDspState>`（与 RPC 共用同一份互斥/取消），工具层因此能跑视频处理。
- `agent_tools` 新增 `process_video`：解析 `path/start/duration/fps/options` → `video_dsp::video_dsp_process`，
  返回 `{path, duration}`；工具定义与桌面同 schema，清单 10 → 11。
- 两个 shell 都把已有的 `video_dsp` 实例传给 agent（不是新建一份），避免同一时间跑两个转码。

**端到端实测（容器模式 + 真 ffmpeg）**：
```
ffmpeg -f lavfi -i testsrc=320x240:rate=30:duration=1 in.mp4
POST /rpc/video_dsp_process {"path":…,"start":0,"duration":1,"fps":30,"options":{"denoise":"light"}}
→ {"duration":1.0,"path":"<data>/cache/video-dsp-…mp4"}   ← 真的产出了文件
```

### 前端门控收尾（2026-11-14）：最后两个"裸调用"模块

审计里列出的裸调用点是 `sdcppApi` / `runtimeApi` / `storageApi`（没有守卫，浏览器里会拿到
`未知命令`）。`runtimeApi` 早前已改；本轮补齐剩下两个：

- `sdcppApi.sdcppInstall` → `requireCapability("sdcpp", …)`（该能力表状态是 `unsupported`）；
- `storageApi.dataDirMigrate` / `dataDirCleanupPrevious` → `requireCapability("data_dir", …)`。

现在 ai-cut 前端所有写操作都经过能力表，读操作保留兜底值以便设置页照常渲染；
`isTauri()` 只剩传输层判断与二进制通道（`render_preview` / `read_import_file`）两处正当用途。
验证：`vue-tsc` 通过、`vitest` 536 passed。

## 17. 网络与构建环境（2026-11-14 修正）

之前误判"网络不可达"。真实情况：

| 通道 | 事实 | 怎么用 |
| --- | --- | --- |
| macOS 系统代理 | `127.0.0.1:46591`（HTTP/HTTPS）、`127.0.0.1:46590`（SOCKS） | `export HTTPS_PROXY=http://127.0.0.1:46591 HTTP_PROXY=…`；`cargo` 认这两个变量，`curl` 也认 |
| Docker 守护进程 | `http.docker.internal:3128`（Docker Desktop 自带代理） | `docker pull` 正常 |
| BuildKit | **不走守护进程代理**，`auth.docker.io` 被解析到 `199.96.63.53` 并超时 | 用经典构建器：`DOCKER_BUILDKIT=0 docker build …`；或先 `docker pull` 好全部基础镜像 |

已完成的解锁：
- `cargo add ort` 成功（`ort v2.0.0-rc.15`），`cargo build --manifest-path crates/daemon/Cargo.toml --lib` 通过；
- `rust:1-bookworm` / `node:22-slim` / `debian:bookworm-slim` 均已拉取。

教训：排查"网络不通"要同时看 `env`、`scutil --proxy`、`~/.cargo/config.toml`、`docker info` 四处，只看 `env` 会得出错误结论。

## 18. 两仓镜像验证完成（2026-11-14）

### ai-daw（Debian bookworm-slim）

```
容器 health → {"ok":true,"version":"1.7.0"}
/opt/ort/libonnxruntime.so.1.30.0 → aarch64 ✓
[runtime] 已加载 /opt/ort/libonnxruntime.so.1.30.0（cpu）
[backend] 建会话 localvqe-ns.onnx（LocalVQE）→ CPU
/rpc/ai_start enhance → "已生成人声降噪轨道（LocalVQE 模型）"
/usr/local/bin → aidawd + vst3-host-helper + vst3-host-probe + clap-scan-probe
/opt/articulation-maps → 8DIO / IK / Ample Sound …
/opt/models → basic-pitch + localvqe + mel_spectrogram
```
镜像 286 MB。

### ai-cut（Debian bookworm-slim，非 Alpine）

```
/rpc/ai_ort_probe → {"loaded":true,"build":"ORT Build Info: … build type=Release"}
/rpc/model_catalog → 5 个 bundled 全 installed=true
ffmpeg 7.0.2-static / ffprobe N-66595-static（与桌面同款静态包）
/rpc/audio_measure_loudness → {"integrated_lufs":-14.92,"true_peak_dbtp":-14.23}
```
镜像 386 MB（Alpine 版 253 MB，但 ORT 加载失败）。

### 踩坑记录（都只有真构建/真运行才暴露）

1. **ORT 架构**：Dockerfile 曾硬编码 linux-x64，Apple Silicon 镜像里装的是 x86_64 的 .so；
   `ldd` 只报 "not a dynamic executable"，不说明原因。改为按 `TARGETARCH` 选择 `x64`/`aarch64`。
2. **Docker ARG 作用域**：全局 `ARG` 只对 `FROM` 可见，`RUN` 里必须在该阶段重新 `ARG`，
   否则静默回落到默认值（表现为"改了却没用"）。
3. **Alpine + gcompat 加载不了 glibc 版 ORT**：`dlopen failed`，只能换 Debian 底座。
4. **Debian 的 ffmpeg 会拖 406 MB 依赖层**：改为复用桌面 sidecar 的静态 ffmpeg/ffprobe，省 400 MB 且版本与桌面一致。
5. **构建网络**：BuildKit 不走 Docker 守护进程代理（`auth.docker.io` 被劫持到 `199.96.63.53`）；
   可用组合是 `DOCKER_BUILDKIT=0` + `--build-arg HTTP_PROXY=http://http.docker.internal:3128`。

## 19. ai-cut M3b-ort + M4 本地推理下沉完成（2026-11-14 第六轮）

### 19.1 M3b-ort：会话创建进 daemon

| 项 | 改动 | 证据 |
| --- | --- | --- |
| `crates/daemon/src/ai_session.rs`（1633 行） | `backend.rs` 整体下沉：EP 选择 / CoreML·DirectML·CUDA 注册 / 运行时懒加载 / 设备画像 / `selftest`；`AppHandle` → `&dyn Host`，`crate::runtime::*` → `crate::runtime_ort::*` | daemon 内 `commit(apply(session_builder()))` 真实建出会话（见下） |
| `src-tauri/src/backend.rs` | 变薄壳：`pub use aicut_daemon::ai_session::*` + `init_runtime(AppHandle)` / `ai_backend_info` / `ai_set_backend` 三个转发 | 桌面 85 tests / `cargo check` 0 error |
| ort 依赖按平台对齐 | daemon 的 `ort` 特性与 `src-tauri` 逐平台一致（macOS `coreml` 静态、Win/Linux `load-dynamic`） | **这是个隐蔽的真坑**：特性合并取并集，daemon 若在任何平台多写 `load-dynamic`，macOS 桌面版就会从静态链接变成运行时加载、启动后 AI 全挂。已在 Cargo.toml 里写明原因 |
| `ort_probe` 平台化 | macOS 静态链接 → 直接回报 `loaded:true` + `ort::info()`；Win/Linux 仍 `init_from` 探 dylib | `/rpc/ai_ort_probe` 两形态都能答 |
| RPC | `ai_backend_info` / `ai_set_backend` 上线，前端 `agentApi` 的 `act()` 门控直接可用 | 实测 `ai_backend_info` 返回 Apple M4 画像 + 5 个后端选项 |

### 19.2 M4：本地推理全部下沉（asr / tts / audio_ai / video_ai / voice_change / aligner）

搬进 `crates/daemon` 的模块（都是「`AppHandle` → `&dyn Host`、`sidecar::run_sidecar` → `Host::run`、
`tauri::async_runtime::spawn_blocking` → `tokio::task::spawn_blocking`、`app.emit` → `host.emit`」）：

| 模块 | 行数 | 依赖 |
| --- | --- | --- |
| `asr.rs` + `aligner.rs` | 1079 + 414 | sherpa-onnx（VAD + Paraformer/SenseVoice/Whisper）+ whisper 对齐器（tokenizers） |
| `tts.rs` + `zipvoice.rs` | 1759 + 332 | ort（MOSS 五张图）+ sherpa-onnx `OfflineTts`（ZipVoice） |
| `audio_ai/`（mod + demucs + demucs6s + localvqe + resample） | 800 + 355 + 141 + 148 + 147 | ort + sherpa-separation（UVR） + ffmpeg DSP |
| `video_ai/`（mod + detect + frames + esrgan + rife） | 1831 + 371 + 304 + 88 + 69 | ort + 直接 spawn ffmpeg（帧级流式） |
| `voice_change/`（mod + meanvc2 + voices + refs） | 295 + 493 + 236 + 153 | ort + 音色库文件 |

新增依赖（`crates/daemon/Cargo.toml`）：`sherpa-onnx`、`sherpa-separation`、`rustfft`、`tokenizers`、`png`，
并把 `[patch.crates-io] sherpa-onnx-sys` 指到本地（与桌面同一份，不链接 sherpa 自带的 ORT）。

**容器内实测（macOS 宿主机跑 `aicutd` + `AICUT_RESOURCES` 指到随包模型，等价于镜像内路径）**：

```
/rpc/audio_ai_denoise            localvqe          → audio-ai-denoise-*.wav（1.0s）
/rpc/audio_ai_separate           uvr-mdx-9482      → vocals + accompaniment
/rpc/audio_ai_separate           htdemucs-ort-v1   → drums / bass / other / vocals（1s 音频 5s 出结果）
/rpc/video_ai_upscale_image      realesr-general   → 64×48 → 256×192 PNG
/rpc/video_ai_interpolate        rife-v4.7         → 10fps/10 帧 → 20fps/20 帧
/rpc/video_ai_samples             yolox-nano        → motion + objects 采样
/rpc/asr_transcribe              paraformer-zh     → 「今天天气不错」「我们来测试语音识别」
/rpc/asr_transcribe_words        whisper-base      → 词级时间 + qwen3-aligner 精修后 timed:true
/rpc/tts_synthesize              zipvoice          → 2.21s WAV，再喂回 ASR 得到「你好世界这是容器里的朗读测试」
/rpc/voice_change_convert        meanvc2-int8      → 2.74s WAV（mean -24.9 dB / peak -2.6 dB）
```

### 19.3 这一轮修掉的真 bug

1. **`apply_command` 参数多包了一层**：`agent_tools` 把整个 `args` 当 `{command: …}` 传，
   而桥接层还会再取一次 `payload.command`，领域层于是收到 `{command:{…}}`，
   报 `stale revision: expected 0, got undefined`。修好后 MCP 的写命令能真的落库（revision 0 → 1）。
2. **MeanVC2 缓存形状按层不同**（`voice_change/meanvc2.rs`）：代码用 `kv[0].len()` 的长度去套全部
   8 张 KV 缓存张量，但 DiT 前两层是滑窗（缓存随步长增长）、后两层定长，第二块之后就会
   `shape [1,2,36,64] (4608) vs data 3072`。改成**每层按自己的长度**建张量后变声可跑。
   这个 bug 桌面版同样存在（同一份源码），之前只是没人拿 3 秒以上的片段试过。
3. **`aicutd` 的 host 类型**：`Arc<HeadlessHost>` 在 `Arc::clone` 处不会自动退化成
   `Arc<dyn Host>`，统一成 trait object 之后 agent / MCP / 各命令共用一份。

### 19.4 M5d：MCP server 进 daemon

- `crates/daemon/src/mcp.rs`：整个 Streamable HTTP / JSON-RPC 服务下沉（配置 `mcp.json`、
  发现文件 `mcp-url`、端口冲突回落临时端口、graceful stop）。`AppHandle`/`ProjectBridge` →
  `Arc<dyn Host>` + `DocHandle` + `Arc<VideoDspState>`，工具分发改为
  `agent_tools::dispatch`（与内置智能体**同一份**工具面）。
- **谁管 MCP**：daemon 定义 `McpControl` trait，桌面壳用 `DesktopMcp` 包住自己那份全量
  `McpRuntime` 实现它。`/rpc/mcp_status` / `/rpc/set_mcp_config` 因此在两种形态里都指向
  “当前真正在跑的那个服务”，不会出现两个 MCP 抢同一个端口。
- 容器实测：`initialize` 返回工具清单，`tools/list` 与 `tools/call` 正常，
  `apply_command` 能改工程（见 19.3）。
- 能力表 `mcp`：`desktop_only` → **`degraded`**，原因文案 `caps.reason.mcpPartialTools`
  （桌面那份仍是 `tools.rs` 的全量工具面，无头这份是文档类 + 响度 + 视频处理）。
- MCP 的 `instructions` 改成**只描述真的存在的工具**：桌面与无头的工具面不同，写错会让模型
  反复调用不存在的工具。

### 19.5 前端门控收尾

- `modelApi` / `audioAiApi` / `videoAiApi` / `voiceChangeApi` 里的 `isTauri()` 守卫全部删除：
  这些命令现在都有 HTTP 实现，浏览器里不需要再"提前拒绝"。
- 能力表翻转：`asr` / `tts` / `audio_ai` / `video_ai` / `voice_change` 从 `missing` → `ok`，
  新增 `aligner` = `ok`。模型没装时的报错来自模型层（"模型尚未安装：xxx"），不是能力缺失。
- `vue-tsc` 通过、`vitest` 536 passed、daemon 195 tests、桌面 85 tests。

### 19.6 仍然剩下的（下一轮）

| 项 | 现状 | 说明 |
| --- | --- | --- |
| ofx / plugin 下沉 | ✅ 已完成（见 §20.2） | 单帧 / 图片 / 转场预览 + 插件管理可用；整段烘焙仍明确拒绝 |
| 全量工具面 | 13 / 约 40 | 见 §21：本轮把 M4 落地后能"纯后端"跑的两个工具（`probe_media` / `list_ai_models`）接上；其余（导出 / 渲染 / 字幕 / 影片 / 对比 / 审片 / 片段级 AI）的实现住在前端 TS 的命令总线上，要么下沉 `core/export/*`，要么维持桌面全量 |
| ai-daw 无声卡虚拟播放 | ✅ 已完成（见 §20.1） | 虚拟时钟 + `/audio`，容器内实测 |

## 20. 无声卡虚拟播放（ai-daw）+ OFX / 插件下沉（ai-cut）（2026-11-14 第七轮）

### 20.1 ai-daw：没有声卡也能「听着玩」

问题：容器里没有音频设备，`cpal` 打不开流 → `available=false` → 浏览器只能离线试听，
而 DAW 的核心体验就是「边改边听」。

做法（`crates/engine/src/realtime.rs`）：

- 把 cpal 回调里的渲染逻辑抽成 `RenderCore::render`（位置 / 循环 / 节拍器 / 电平表 /
  混音抽头 / 插件实例全在里面），真设备与**虚拟时钟**共用同一份实现。
- 新增 `AudioHost::start_virtual(...)`：后台线程按「一块的时长」对齐绝对时间轴睡
  （`next += step`，不是 `sleep(step)` 累加，否则时钟会越走越慢），逐块调 `render`。
- `AudioHost.stream` 变成 `sink: Sink::{Device, Virtual}`；`Drop` 两条路径都干净停机。
- `crates/daemon/src/audio.rs`：`resolve_sample_rate` 失败且没有指定设备时自动切虚拟时钟
  （48 kHz / 立体声），并把 `virtual_output` 写进 `TransportSnapshot`；
  `AIDAW_AUDIO=virtual` 可在有声卡的机器上手工强制。
- 前端：`transport.virtualOutput` + 底栏「虚拟时钟」胶囊（tooltip 说明「本机不出声、
  声音走网页」），不再让人以为设备坏了。

**容器实测（`ai-daw:virtual`，无 `/dev/snd`）**：

```
/health                                → {"audio":true,...}
/rpc/transport_status                  → {"available":true,"virtual_output":true,"sample_rate":48000}
/rpc/transport_play → 1.5 s 后         → {"playing":true,"position_frames":71680}   ← 48 kHz 实时推进
WS /audio                              → 94 个二进制块 / 1.54 MB / 4 s，start_frame 单调递增
```

也就是说：**浏览器成了声卡**。这是「无声卡」这条物理约束下能给到的最好结果。

### 20.2 ai-cut：OFX 与插件管理下沉

| 项 | 结果 |
| --- | --- |
| `plugins.rs`（1913 行） | ✅ 整体下沉。注册表拉取 / 下载 sha256 校验 / 归档解包 / 托管目录 / 外部目录扫描（macOS `~/Library/OFX/Plugins`、Windows `Common Files/OFX`、Linux `/usr/OFX/Plugins` + `OFX_PLUGIN_PATH`）全部走 `Host`；RPC 8 条 |
| `plugin_reveal` / `plugin_open` | 无头版**明确拒绝**（「在文件管理器中显示需要桌面应用」/「请手动打开插件主页：<url>」），桌面版保留系统调用 |
| `ofx.rs` 单帧路径 | ✅ `ofx_probe` / `ofx_render_frame` / `ofx_render_preview` / `ofx_source_time` / `ofx_render_transition` / `ofx_bake_image` 下沉（`ofx-host` 是纯计算库，链接与桌面同一份） |
| `ofx_bake_clip` / `ofx_bake_transition` | ⛔ 无头版**明确拒绝**：桌面版靠 `--ofx-bake-worker` **重新拉起自己**分块并行编码，这套进程模型不属于无头后端（镜像里没有那个可执行文件，也不该 fork 自己）。单帧预览 / 图片烘焙仍可用 |
| 能力表 | `ofx` / `plugin`：`desktop_only` → `degraded`，原因文案分别说明「整段烘焙」与「文件管理器」这两条 |

为什么没有把桌面 `ofx.rs` 也改成薄壳：整段烘焙那条链（worker 子进程 + 分段合并 + 进度文件）
要保留在桌面侧，而共享的类型与 `FramePipeline` 必须两边都有。为了不让桌面版出现回归，
本轮**只新增 daemon 侧实现**，桌面 `ofx.rs` 保持原样；两边在「单帧路径」上会有重复，
已在 §19.6 / 本节记录为已知债（下一步可以把 `FramePipeline` 与参数构造抽到 daemon，桌面只留 worker）。

### 20.3 前端门控

`pluginApi` / `ofxApi` 里的 `isTauri()` 守卫全部删除：命令现在有 HTTP 实现，浏览器里
直接由后端回答（能力表 `degraded` 允许调用，桌面独占的那两条返回结构化错误）。
`pickPluginBundle` / `pickPluginFile` 保留 `isTauri()`——浏览器没有原生文件框。

### 20.4 本轮验证

| 检查 | ai-cut | ai-daw |
| --- | --- | --- |
| daemon `cargo test` | 206 passed / 5 ignored | 230 passed / 8 ignored（`--no-default-features`） |
| 桌面 `cargo test` / `check` | 见下 | 0 error |
| 前端 `vue-tsc` | 通过 | 通过 |
| 前端 `vitest` | 536 passed | —（无用例） |
| 前端 `vite build` | 通过 | 通过 |
| 镜像 | `ai-cut:m4c` 431 MB（含 13 工具版本），容器内实测（见下） | `ai-daw:virtual2`，容器内实测虚拟时钟 + `/audio` + ORT 推理 |

### 20.5 镜像内实测（两个都重跑，发现了**架构静默错误**）

**跑出来的第一个错误**：`ai-cut:m4` 里 `/rpc/ai_ort_probe → dlopen failed`，
`audio_ai_denoise` 报 ``OrtGetApiBase` must be present in ONNX Runtime dylib`。

根因不是代码，是**镜像里的 .so 架构不对**：

```
docker exec … uname -m                            → aarch64
od -An -tu2 -j18 -N2 /opt/aicut/ort/libonnxruntime.so.1.30.0 → 62   # 62 = x86_64
```

`TARGETARCH` 是 **BuildKit 的自动参数**。本机只能用经典构建器（`DOCKER_BUILDKIT=0`，
因为 BuildKit 拉 `auth.docker.io` 超时），而经典构建器**不会设置 `TARGETARCH`**，
`case "${TARGETARCH:-amd64}"` 就静默落到了 `x64`。上一轮"镜像验证通过"其实是在
x86_64 的 .so + arm64 的容器这种组合下**没测到 ORT** 才漏过去的。

修法（两个 Dockerfile 同改）：

1. 架构以**构建阶段真实架构**为准：`${TARGETARCH:-$(dpkg --print-architecture)}`；
2. 下载后立刻做 **ELF 头自检**（`od -j18`，62 = x86_64 / 183 = aarch64），
   不对就 `exit 1`——把这类问题挡在构建期，而不是留到运行期的 `dlopen failed`。

修复后实测：

| 检查 | ai-cut | ai-daw |
| --- | --- | --- |
| ORT 架构 | 183（aarch64） | 183（aarch64） |
| ORT 可加载 | `ai_ort_probe → loaded:true`（Release 构建） | `[runtime] 已加载 /opt/ort/libonnxruntime.so.1.30.0` |
| 本地推理 | 降噪（LocalVQE）→ 1.0s WAV；超分（Real-ESRGAN）64×48 → 128×96；ASR（Paraformer）→ 「今天天气不错」「我们来测试语音识别」；TTS（ZipVoice）→ 1.49s WAV | LocalVQE 降噪 → `已生成人声降噪轨道` |
| MCP | `/rpc/mcp_status` running，11 个工具 | （上一轮已验证） |
| 虚拟时钟 | — | `virtual_output:true`，播放位置实时推进，`/audio` 收 PCM |
| 镜像体积 | 431 MB（比 386 MB 多的是 sherpa 静态库 + ofx-host） | 286 MB |

> 顺带记一笔：容器里的模型下载走 `network_set_config` 的代理；容器**没有** `curl`，
> 排查时别指望 `docker exec curl`。本轮 ASR/TTS 的模型是用 `docker cp` 预置进
> `<data>/models` 的（等价于用户点「下载」完成之后的状态）。

## 21. 工具面扩到 13 个（ai-cut M5c 续，2026-11-14 第八轮）

> 镜像已重建为 `ai-cut:m4c`（431 MB），容器内 `/rpc/mcp_status` = running。
> 注意：MCP 只监听容器内的 `127.0.0.1:50563`（设计如此），所以**从宿主机 publish 端口也接不到**——
> 要么在同一容器网络里访问，要么用 `docker exec` 进去；容器里没有 curl，验证时改用
> `/rpc/mcp_status` 或宿主机上的同名 daemon。

M4 落地之后，之前"依赖尚未下沉推理"的工具终于可以纯后端实现了。本轮先挑**语义本来就是文件级**
的两个（与桌面 `tools.rs` 同名同参数，不新增 API 概念）：

| 工具 | 实现 | 容器实测 |
| --- | --- | --- |
| `probe_media` | `crate::analysis::probe`（元数据 + 运动采样 + 可选目标框 / 转写） | `/data/in.mp4` → `duration:1.0 / hasVideo:true / fps:10 / samples…` |
| `list_ai_models` | `crate::models::model_catalog`（按 feature 过滤，只回 `id/name/feature/family/installed/bundled`） | `feature=asr` → 5 条，`whisper-base-int8 installed:true` |

为了这两个工具，`agent_tools::dispatch` 的 `host` 参数从 `&dyn Host` 变成 `&Arc<dyn Host>`
（`analysis::probe` 会在内部把宿主交给 `video_ai` / `asr`，它们要 `Arc`）；调用点只有
`agent/mod.rs` 与 `mcp.rs` 两处，都是顺手改。

**顺带修掉一个前端真 bug**：能力表里 `caps.reason.agentPartialTools` / `mcpPartialTools` /
`ofxBakeDesktopOnly` / `pluginRevealDesktopOnly` 四个 i18n 键**从来没有被写进 `i18n/{zh,en}.ts`**
（之前的插入锚点不存在，脚本静默没写入）。表现是设置页的能力说明直接显示原始 key。
现在四个键都补上了（中英各 4 条），并用脚本按「daemon 用到的 key → i18n 是否定义」逐条核对过。

**仍然没接的**（都是片段级：要改时间线，得走命令总线而不是文件）：
`denoise_audio` / `separate_stems` / `process_audio` / `transcribe` / `cut_filler_words` /
`translate_audio` / `upscale_video` / `interpolate_video` / `smart_reframe` / `voice_change`。
它们的"算法"已经全部在 daemon 里，缺的只是「取片段 → 跑模型 → `asset.add` + `clip.add` 落回时间线」
这一层；每条约 80–120 行，`apply_command` 的批量接口已经可用。

## 22. 片段级 AI 工具的施工细则（下一轮直接照着做）

目标：把 ai-cut 的 10 个片段级 AI 工具接进无头工具面（算法**全部已下沉**，缺的只有"回写时间线"这一层）。

### 22.1 需要的三步

1. **读片段**：`agent_bridge::call(doc, "project.clip", {"clipId": …})` 返回
   `{ revision, clip, track, asset, neighbors }`，其中：
   - `clip.start` / `clip.duration` / `clip.in`（秒）——模型只需要 `start..start+duration` 这一段；
   - `asset.path`（本机绝对路径）、`asset.kind`（video/image/audio）、`asset.duration`。
   模型一律按**片段区间**跑（与桌面 `tools.rs` 的语义一致），不要拿整条素材。
2. **跑模型**（都已经是 daemon 函数，直接 await / spawn_blocking）：
   `audio_ai::audio_ai_denoise` / `audio_ai_separate` / `audio_ai::audio_dsp_process` /
   `asr::asr_transcribe`(+`aligner`) / `audio_ai::translate_text` /
   `video_ai::video_ai_upscale` / `video_ai_interpolate` / `video_ai_reframe` /
   `voice_change::voice_change_convert`。
   输出统一是"缓存目录里的文件 + 时长"。
3. **回写时间线**：用两条 `Command` 走 `doc.apply`（`agent_bridge::apply_command` 的 payload 就是一条命令）：
   ```jsonc
   // 1) 登记素材（字段校验见 src/core/commands/index.ts 的 asAsset）
   {"kind":"asset.add","payload":{"asset":{
      "id":"<uuid>","name":"…","kind":"audio|video|image","src":"<绝对路径>",
      "duration":<秒>,"hash":"","width":null,"height":null,"fps":null,"hasAudio":true,
      "hasVideo":false,"createdAt":<ms>
   }},"baseRevision":<rev>}
   // 2) 落到轨道（asNewClip：start/duration/assetId 必填，in 缺省 0）
   {"kind":"clip.add","payload":{"trackId":"<新轨或目标轨>","clip":{
      "assetId":"<上一步 id>","start":<秒>,"duration":<秒>,"in":0
   }},"baseRevision":<rev+1>}
   ```
   新轨用 `{"kind":"track.add","payload":{"type":"audio|video","name":"…"}}`。
   批量时用 `apply_commands`（一个撤销步、一个 revision）。
4. **静音原片**（`separate_stems` / `denoise_audio` 的桌面语义）：再补一条
   `{"kind":"clip.update","payload":{"clipId":"…","patch":{"mute":true}}}`；不确定字段时
   先 `get_clip` 看当前 patch 形状，别凭空造字段。

### 22.2 每条工具的参数（与桌面 `tools.rs` 同名同参）

| 工具 | 参数 | 回写 |
| --- | --- | --- |
| `denoise_audio` | `clipId`, `modelId?` | 新音轨 + 静音原片 |
| `separate_stems` | `clipId`, `modelId?` | 每条 stem 一条轨 |
| `process_audio` | `clipId`, `effect`, `preset` | 替换/新增音轨 |
| `transcribe` | `clipId`/`assetId`, `language?`, `modelId?`, `insertSubtitles?`, `trackName?` | 字幕轨（文字片段，用 `subtitle.import` 命令更省事） |
| `cut_filler_words` | `clipId`, `minPause?`, `pauseKeep?`, `sentenceKeep?`, `dryRun?` … | 多次 `clip.trim` / `clip.remove` |
| `translate_audio` | `clipId`, `target`, `dub?`, `asrModelId?`, `ttsModelId?` | 译文字幕轨（+ 配音轨） |
| `upscale_video` / `interpolate_video` / `smart_reframe` | `clipId`, `modelId?`, `scale`/`factor`/`aspect` … | 新视频轨覆盖原区间 |
| `voice_change` | `clipIds[]`, `voiceId`, `modelId` | 每条一份新音频 |

### 22.3 验收（不用真模型也能测一半）

- ✅ **已在本轮跑通**（容器同款 daemon，`apply_commands` 一次三条）：

  ```
  track.add   → ok, revision 1
  asset.add   → ok, revision 2
  clip.add    → ok, revision 3
  /rpc/doc_snapshot → assets 里出现那条 1.0s 的音频素材
  ```

  过程中修掉一个真 bug：**批量命令的 `baseRevision` 注入**。领域层要求每条命令都带基线，
  之前要么没注入（`stale revision: expected N, got undefined`），要么整批沿用批前基线
  （第二条起全部 `stale revision: expected 1, got 0`）。现在规则是：第一条用调用方给的
  `baseRevision`（没有就取当前 revision），之后每条取「那一刻」的 revision。同时把工具描述
  从「原子批量」改成实话——**逐条应用、失败即停、不回滚**（无头版没有撤销栈）。
- 再用已随包的 `localvqe` 跑 `denoise_audio` 的完整链路（模型在镜像里就有）。
- 别忘了能力表 `agent` / `mcp` 的原因文案与 MCP `instructions` 一起更新。

### 22.4 为什么这轮没做

本轮上下文预算已经不足以支撑「10 条工具 ×（取片段 + 跑模型 + 两条命令回写 + 失败回滚）」这种
需要反复编译验证的批量工作。把契约、字段校验位置、验收方式写死在这里，下一个会话可以直接开工，
不需要重新调研。

### 22.5 第一个片段级工具已落地：`denoise_audio`（round 36）

`agent_tools` 里新增 `denoise_audio`（工具面 13 → 14），完整链路在 daemon 内跑通：

```
get_clip(c-1) → asset.src / clip.start / clip.duration
audio_ai_denoise(localvqe) → 缓存 WAV
apply_commands [track.add]
apply_commands [asset.add, clip.add]   ← 复用 §22.1 的命令形状
```

**实测（容器同款 daemon + 随包 localvqe）**：

```
tracks: [('原声', 1 clip), ('AI 降噪 · in.wav', 1 clip)]
assets: ['in.wav', 'AI 降噪.wav']
```

**这轮踩到的两个坑（都写进本节，后面 9 个工具别再撞）**：

1. `project.clip` 视图里素材路径的字段名是 **`src`**，不是 `path`（`assetSummary` 直接透传
   `MediaAsset`）。第一版按 `asset.path` 取，报「这个片段没有素材」。
2. ~~`clip.update` 的轨道解析~~ **这条是我自己挖的坑，已澄清**：`clip.update` 的轨道来自
   **片段自己的 `trackId` 字段**（patch 里带 `trackId` 会被拒：「不支持修改 id / trackId」）。
   我最初手写测试工程时片段里没有 `trackId`，于是报「未知轨道：undefined」——用命令
   （`clip.add`）加进来的片段自带 `trackId`，`{"clipId":…, "patch":{"muted":true}}` 一次就生效。
   教训：**片段级工具的测试工程必须用命令搭**（`track.add` + `asset.add` + `clip.add`），
   手写 JSON 会缺 `trackId` 这类派生字段。

修正后 `denoise_audio` 的完整实测：

```
tracks: [('原声', [('c34710ab', muted: true)]), ('AI 降噪 · in.wav', [('3fa46d27', -)])]
assets: ['in.wav', 'AI 降噪.wav']
```

即「新轨 + 新素材 + 新片段 + 原片静音」四步全通——剩下 9 个工具照这个骨架换模型与落点即可。

工具描述与 `TOOLS` 诊断表已同步；`cargo test` 211 passed。

### 22.6 第二个片段级工具：`process_audio`（round 38，工具面 14 → 15）

与 `denoise_audio` 同骨架，只换"跑什么"：

```
get_clip → asset.src / start / duration
audio_ai::audio_dsp_process(effect, preset)      ← 纯 ffmpeg 滤镜，无模型
apply_commands [track.add]
apply_commands [asset.add, clip.add, clip.update{muted:true}]
```

实测（容器同款 daemon）：

```
tracks: [('原声', [('5894ae3c', muted: true)]), ('AI denoise/light · in.wav', [('09621338', -)])]
assets: ['in.wav', 'AI denoise.wav']
```

`cargo test` 211 passed；MCP `tools/list` = 15。

**剩下 8 个的差异点只有两处**（骨架不用再改）：
- **多个输出**（`separate_stems` 4/6 条、`voice_change` 每条片段一份）：把 `asset.add`+`clip.add`
  对每个输出各来一遍即可，仍放在同一批 `apply_commands` 里（一个撤销步）。
- **覆盖原区间**（`upscale_video` / `interpolate_video` / `smart_reframe`）：落点用 `clip.add`
  到**新视频轨**（`track.add {type:"video"}`），再 `clip.update {muted:true}` 关原片段声音；
  想"替换"就再加一条 `clip.remove`——注意那一步不可回滚，放在用户显式要求时再做。

## 23. 交接摘要（round 39 收口）

### 当前绿点（最后一次实跑）

| 检查 | ai-cut | ai-daw |
| --- | --- | --- |
| daemon `cargo test` | 211 passed / 5 ignored | 230 passed / 8 ignored |
| 桌面 `cargo test` / `check` | 0 error | 1 passed / 0 error |
| `vue-tsc` | 通过 | 通过 |
| `vitest` | 536 passed | —（无用例） |
| `vite build` | 通过 | 通过 |
| 镜像 | `ai-cut:m4c` 431 MB | `ai-daw:virtual2` 286 MB |

### 三形态现在的差异只剩这些（全部有明确理由）

| 能力 | 差异 | 归类 |
| --- | --- | --- |
| 原生插件 GUI（VST3/CLAP/OFX 编辑器） | 容器/浏览器没有窗口 | **物理** |
| 系统级：文件管理器 reveal、拉起系统浏览器、cli.mount、自动更新、app_restart | 没有桌面会话 | **物理** |
| GPU 生成（sd.cpp 出图/生视频） | 容器无 GPU | **物理**（标 `unsupported`） |
| 插件级 AI 工具 8 个 / OFX 整段烘焙 | 未实现（骨架与配方已就绪，见 §22） | **工程量** |

### 下一步（按收益排序）

1. 8 个片段级工具：照 §22.6 的骨架，`separate_stems`（多输出）→ `upscale_video` 系（覆盖区间）
   → `transcribe`/`cut_filler_words`/`translate_audio`（走 `subtitle.import` 命令更省事）→ `voice_change`。
2. OFX 整段烘焙：用 `video_ai` 里现成的 `Command` 流式解码/编码替换桌面的 worker 模型（§20.2）。
3. `ofx.rs` 单帧路径去重：把 `FramePipeline` 与参数构造抽进 daemon，桌面只留 worker。
4. 提交：两仓改动都还没提交（ai-daw 25 文件 / ai-cut 92 文件），建议按里程碑拆 8 个左右：
   能力表 / 网络·云端 / 响度·视频 / 模型·运行库 / ORT 会话 / 本地推理 / 智能体·MCP / OFX·插件 / ai-daw 虚拟时钟。

## 24. 片段级 AI 工具全部到位 + OFX 整段烘焙（2026-11-14 第十轮）

这一轮把 §22 / §23 列的两条尾巴做完了，并且顺手修掉了三个**参数面不一致**的旧工具。

### 24.1 关键决定：落位规则只写一次

之前无头版是「Rust 里拼命令」把模型结果接回时间线，桌面版是「TS 里调纯函数」——两份实现，
于是出现了静默漂移：桌面对**纯音频片段**是"就地换素材"（`replaceClipAudio`），无头版却
另开一轨并静音原片。听起来差不多，用起来差别很大（轨道数、撤销步数、片段时长都对不上）。

现在加了一层 `AICUT.op(kind, args)`：把 `core/aiActions` 里那批**纯函数**（`addStemTracks` /
`applyProcessedAudio` / `applyCuts` / `insertCueTrack` / `placeVoiceover` / `placeConvertedVoice` /
`replaceClipVideo` / `loudnessGain` + `cutAnalysis` 全套）原样暴露给无头运行时。
分工变成：

```
Rust：跑模型（asr / tts / audio_ai / video_ai / ofx）→ 产出「文件 + 时长 + 元数据」
TS  ：把结果接回文档（新建轨 / 替换素材 / 建字幕轨 / 剪口落地 / 增益写回）
```

`op` 的约定：`assets` 先登记素材再落位；整个操作原子（抛错不改文档、revision 不变）；
改一次文档 → revision +1（等于桌面的一次 `dispatch`）。

顺带修掉两个旧问题：

- **素材字段是手写的假形状**：`{hash, createdAt, hasVideo}` 在 `MediaAsset` 里根本不存在，
  而真字段 `addedAt` 又没写，素材库显示 `undefined`。现在统一走 `doc_ops::asset_for`，
  按 `importAssetAt` 的真形状（`id/name/kind/src/duration/addedAt/width/height/fps/hasAudio`）构造。
- **单帧路径与整段烘焙的 alpha / 时间规则**仍然共用 daemon 里那一份，没有第二份实现。

### 24.2 工具面 15 → 23：8 个片段级 AI 全上线

| 工具 | 参数（与桌面完全同名同参） | 落位 |
| --- | --- | --- |
| `transcribe` | `clipId` / `assetId`、`language`、`modelId`、`insertSubtitles`、`trackName` | 顶部字幕轨（`insertCueTrack`） |
| `cut_filler_words` | `clipId`、`minPause`、`pauseKeep`、`sentenceKeep`、`includeSoft`、`dryRun`、`customWords`、`disabledWords`、`trimPad` | `applyCuts` + 重铺「口播字幕」 |
| `separate_stems` | `clipId`、`modelId` | 源轨下方逐条分轨，源片段静音 |
| `translate_audio` | `clipId`、`target`、`dub`、`asrModelId`、`ttsModelId`、`providerId`、`voiceId` | 译文字幕轨（+ 译音轨） |
| `synthesize_voiceover` | `clipIds` / `text`、`start`、`modelId`、`providerId`、`voiceId`、`referenceAssetId`、`referenceText` | 朗读轨（重叠自动开新轨） |
| `upscale_video` | `clipId`、`modelId`、`scale` | 替换片段引用（`in` 归零、时长夹到新素材） |
| `interpolate_video` | `clipId`、`modelId`、`factor` | 同上 |
| `reframe_video` | `clipId`、`mode`、`subject`、`aspect`、`height`、`smooth` | 同上 |

三处**只有无头版才需要**的桥接（桌面走 i18n 直接 `t(...)`）：

- `op("translate_keys", {keys})`：轨道名 / 标签用桌面那份字典，两个形态文案一致；
- `op("map_cues", {cues, clipId, fps})`：素材内时间 → 时间线时间（帧吸附 + 窗口裁剪）；
- `op("apply_loudness", {clipId, report, targetLufs, apply})`：增益算法与写回都在领域层。

### 24.3 修掉三个"参数面不一致"的旧工具

这三个之前能用，但**参数与桌面不同**，模型按桌面文档调用会直接失败：

| 工具 | 之前 | 现在（= 桌面） |
| --- | --- | --- |
| `measure_loudness` | 要 `path` / `start` / `duration`（裸文件） | `clipId` + `targetLufs` + `apply`，增益由 `loudnessGain` 算并写回片段 |
| `process_video` | 要 `path` / `start` / `duration` / `fps`，结果只返回路径 | `clipId` + `options`，结果**替换原片段** |
| `denoise_audio` / `process_audio` | 一律"新轨 + 静音原片" | 走 `applyProcessedAudio`：纯音频片段就地换素材，视频片段才另开一轨 |

### 24.4 OFX 整段烘焙（片段 + 转场）

`ofx_bake_clip` / `ofx_bake_transition` 现在是真实现，不再是「桌面专属」的显式拒绝：

- 逐帧解码（`decode_frame` 带 fit 滤镜）→ 效果链 `Session` → 写 rawvideo 进 ffmpeg；
- 预览走 H.264，导出走 ProRes 4444（`bake_encoder_args`，与桌面同一份参数）；
- 先写临时文件、成功后 rename，失败 / 取消自动清理；
- 取消走 `ofx_cancel_bake` 的 id 集合，帧边界生效；
- 进度事件 `ofx-bake-progress`（`rendering` / `done`，带 `done/total`）。

**一个真实的坑**：OpenFX 示例插件 Invert 对 `kOfxActionInstanceChanged` 返回
`kOfxStatErrMissingHostFeature`（status 4），整段烘焙直接失败。原因是宿主对**非 Retimer**
效果也去问 `SourceTime`。按规范 `SourceTime` 只有 Retimer 必填，所以现在：效果链首位不是
Retimer 时用恒等映射（`source_time = time`），不问插件。Retimer 链路仍然照旧。

能力表里 `ofx` 从 `degraded` 翻成 `ok`（剩下的只有"插件原生 GUI 窗口"这一条物理限制）。

### 24.5 这一轮跑出来的实测（真模型，不是 mock）

新增 `crates/daemon/tests/tools_e2e.rs`：默认跳过，`AICUT_E2E_DIR=<有 models/ media/ 的目录>`
时执行。每个用例都走 `agent_tools::dispatch`（智能体与 MCP 的同一条入口）→ 真模型 →
领域层落位 → 断言文档形状。

```
separate_stems_end_to_end      UVR-MDX-VOC-FT      ok（3 轨，源片段 muted）
process_audio_replaces_clip    ffmpeg DSP          ok（1 轨，assetId 已换）
transcribe_writes_a_caption    Paraformer + VAD    ok
upscale_video_replaces_clip    Real-ESRGAN         ok（in 归零）
interpolate_video              RIFE v4.7           ok
reframe_video_tracks_subject   YOLOX-nano          ok
synthesize_voiceover_lands     MOSS-TTS-nano       ok（0.5s 起点）
ofx_bake_clip_encodes_video    OpenFX Invert       ok（首帧亮度 255-x，真反相）
```

另有 11 个**纯领域**用例进 `crates/daemon/tests/parity.rs`（`AICUT.op` 的形状契约）：
字幕轨在最上方、纯音频就地换素材、分轨插在源轨下方且源片段静音、替换视频 `in` 归零、
`map_cues` 帧吸附 + 窗口裁剪、`cut_filler` dryRun 不改文档、`translate_keys` 中英一致、
未知 op 明确报错。

### 24.6 绿点（本轮实跑）

| 检查 | ai-cut |
| --- | --- |
| daemon `cargo test --lib` | 212 passed / 5 ignored |
| daemon `cargo test --test parity` | 11 passed |
| daemon `cargo test --test tools_e2e`（真模型 + 真 OFX 插件） | 9 passed |
| 桌面 `cargo test --lib` | 74 passed / 1 ignored |
| `vue-tsc` | 通过 |
| `vitest` | 536 passed |

领域 bundle 从 399.4 KiB / 35 模块长到 401.6 KiB；`crates/daemon` 仍然**不含任何 tauri 依赖**。

### 24.7 编辑宏也接上了（工具面 23 → 34）

桌面这 11 个宏工具本质就是「扁平参数 → 一条命令」，所以无头版照抄命令形状即可，
`baseRevision` 由 `apply_domain_command` 注入（调用方显式给了就用它的）：

| 工具 | 命令 | 参数形状 |
| --- | --- | --- |
| `append_clip` | `clip.append` | 原样透传（`assetId` / `trackId` / `start` / `duration` / `in`） |
| `set_clip_speed` | `clip.speed` | 原样透传 |
| `duplicate_clips` | `clip.duplicate` | 原样透传（`offset` / `targetTrackId`） |
| `ripple_delete` | `clip.ripple_delete` | 原样透传 |
| `add_marker` / `remove_marker` | `marker.add` / `marker.remove` | 原样透传 |
| `update_marker` | `marker.update` | 扁平字段收进 `patch` |
| `set_clip_color` | `clip.color` | 五色调参数收进 `patch` |
| `set_clip_transition` | `clip.transition` | 扁平字段收进 `transition`，`remove=true` 传 null |
| `set_clip_keyframes` | `clip.keyframes` | `remove=true` 传 null |
| `import_asset` | （新增 `op("add_assets")`） | `path` + 可选 `name`，走真 ffprobe |

`import_asset` 直接复用 `doc_ops::asset_for`：扩展名白名单 + ffprobe 探测 + 真字段形状，
遇到不支持的扩展名明确报错（不登记坏素材）。

`tools_e2e` 里新增 `editing_macros_drive_the_timeline`：导入 → 追加（自动建轨）→ 变速 →
标记增删改 → 复制 → 波纹删除 → 坏类型报错，全链路真跑。

### 24.8 还没做的（都是工程量，不是物理限制）

工具面对齐进度：**34 / 62**。剩下 28 个按性质分四类：

1. **导入 / 导出小件**：`import_subtitles`、`export_subtitles`、`extract_audio`
   （前两个要 SRU/VTT 解析与写盘，第三个要 ffmpeg 抽音轨 + 登记素材）。
2. **工程生命周期**：`new_project` / `open_project` / `save_project` / `undo` / `redo` / `aicut_help`。
   前三个 daemon 已有对应 RPC，`undo` / `redo` 需要给无头文档引擎补撤销栈（README 里已说明
   无头版没有撤销栈——这是个真差异，要么加栈要么在工具描述里写清楚）。
3. **导出 / 渲染**：`export_video`、`render_frame`、`render_storyboard`、`render_range`、
   `render_manifest`、`render_status`、`compare_takes`、`render_contact_sheet`。
   `export.rs` 已在 daemon 里，缺的是把这些工具的参数映射过去。
4. **任务与协作**：`job_start` / `job_status` / `job_cancel` / `cancel_ai`（无头版是同步等待，
   需要引入 job 注册表）、`download_model`、审片批注 4 个、影片结构 import/export 2 个。

## 25. 工具面 62 / 62 对齐（2026-11-15 第十一轮）

上一轮结束时是 34 / 62。这一轮把剩下的 28 个工具全部接上，**无头版与桌面版现在是同一份工具面**。

### 25.1 新增的 28 个工具

| 类别 | 工具 | 落地方式 |
| --- | --- | --- |
| 工程生命周期 | `new_project` / `open_project` / `save_project` | `op("new_project")`（同一个 `makeProject`）+ 直接读写 JSON；进程级记录当前路径 |
| 历史 | `undo` / `redo` | 领域引擎里的**快照栈**（不可变更新，存引用即可，上限 50 步） |
| 帮助 | `aicut_help` | `skills/aicut` 的 10 份文档，`include_str!` 编进去 |
| 任务 | `cancel_ai` / `job_start` / `job_status` / `job_cancel` | 进程级 job 注册表；`job_start` 把同一个 `dispatch` 丢到后台 |
| 模型 | `download_model` | `models::model_download`（gated 时把 auth 原样返回） |
| 导入 / 导出 | `import_subtitles` / `export_subtitles` / `extract_audio` | `op("place_subtitles")` / `op("serialize_subtitles")` / `op("extract_audio")` |
| 导出 | `export_video` / `render_range` / `render_frame` / `render_status` | `op("compile_export")`（同一个 `buildExportPlan`）+ `export::export_video` |
| 审片 | `render_storyboard` / `render_contact_sheet` / `compare_takes` / `render_manifest` | 静帧循环 / `op("contact_layout")` + xstack / ffmpeg ssim + `op("parse_ssim")` / 逐场导出 |
| 批注 | `add_review_note` / `list_review_notes` / `update_review_note` / `remove_review_note` | `marker.add` / `markers` 视图（`noteOnly`）/ `marker.update` / `marker.remove` |
| 影片结构 | `import_film_manifest` / `export_film_manifest` | `op("film_import")` / `op("film_export")`（同一个 `parseFilmManifest` / `buildManifestCommands` / `projectToManifest`） |

### 25.2 三个"看起来能用、其实在撒谎"的 bug

1. **`apply_command` 一直没注入 `baseRevision`**。领域层回 `stale revision: expected N, got undefined`，
   工具却把 `{ok:false, error:…}` 当成成功返回——模型会以为写进去了。现在统一走
   `apply_domain_command`（注入基线 + `ok:false` 变错误）。这条是这轮最有价值的修复：
   之前所有"单条命令"的工具调用其实都被静默丢弃了。
2. **审片批注的 severity**：领域层只接受 `blocker / high / medium / low`，我一开始写的
   `info / minor / major` 会被拒。工具 schema 现在与桌面一致。
3. **`marker.add` 的 id 得调用方给**：`Affected` 只有 clip / track / asset 三类 id，
   照桌面那样自己生成 uuid 再返回。

另外加了一道**构建期护栏**：`scripts/build-domain.mjs` 会检查 bundle 的依赖图里有没有
`vue` / `@vue/*` / `src/stores/*`，有就直接失败。这轮就是被它抓住的——`plan.ts` 从
`@/i18n`（Vue 版）取 `t()`，导致 QuickJS 里 `process is not defined`，整个文档引擎起不来。
改成从 `@/i18n/translate`（纯查表）取，行为完全一致。同时把 `buildContactLayout` 抽到
独立的纯模块 `core/export/contactLayout.ts`。

### 25.3 有意的行为差异（写在这里，不藏在代码里）

| 项 | 桌面 | 无头 | 原因 |
| --- | --- | --- | --- |
| 导出里的文字图层 | Canvas 渲染后再合成 | **跳过并返回 warning**（`export.warnMissingTextImage`） | 容器里没有 Canvas；工具结果里如实带 warnings，不假装成功 |
| `render_manifest` 缓存 | 内容哈希，命中跳过 | 每次都重渲染（`cached: 0`） | 缓存依赖前端 `fileStamp`/`pathExists` 那套；结果一致，只是慢 |
| OFX 整段烘焙并行 | 分段多进程 | 单进程顺序 | 容器里不起子进程；逐帧语义同一份 |
| 撤销栈位置 | Pinia store | 引擎快照栈 | 无头没有 UI，但两边都是"快照式撤销"，上限 50 步 |

### 25.4 能力表翻成 `ok`

`agent` / `mcp` / `ofx` 三个 key 从 `degraded` 改成 `ok`，对应的三条 i18n 原因文案删掉
（不再有需要解释的降级）。现在能力表里剩下的 `degraded` / `desktop_only` / `unsupported`：

- `desktop_only`：`plugin.reveal` / `plugin.open`、`cli`、`data_dir`（文件管理器 / 浏览器 / 路径类）
- `unsupported`：`image_gen`、`video_gen`、`sdcpp`（**本地生成器只在桌面实现**，不是"容器没有 GPU"，见 §25.7）

### 25.5 绿点（本轮实跑）

| 检查 | 结果 |
| --- | --- |
| daemon `cargo test --lib` | 212 passed / 5 ignored |
| daemon `--test parity` | 11 passed |
| daemon `--test tools_e2e`（真模型 + 真 OFX + 真导出） | 11 passed |
| 桌面 `cargo test --lib` | 74 passed / 1 ignored |
| `vue-tsc` | 通过 |
| `vitest` | 536 passed |
| `vite build` | 通过 |

`tools_e2e` 新增 `session_history_and_review_tools_work`：新建工程 → 命令 → 撤销/重做 →
审片批注增删改 → 文字片段导出 SRT 再导入 → 保存/打开 → 导出 film.json → 导入 film.json →
后台任务轮询到 done，全链路真跑。

领域 bundle 467 KiB / 45 个模块；`crates/daemon` 仍然不含任何 tauri 依赖。

### 25.6 用真实二进制 + MCP 走了一遍（不是只跑测试）

起真 `aicutd`（`--data-dir /tmp/aicut-run`），用 curl 走 MCP Streamable HTTP：

```
health            → {"ok":true,"nativeUi":false,...}
/rpc/capabilities → project..mcp 全 ok；plugin degraded；cli/data_dir desktop_only；image_gen/video_gen/sdcpp unsupported
tools/list        → 62 个工具（与桌面 tools.rs 逐名相同）
new_project       → {"name":"MCP 验证","fps":25,"width":1920,"height":1080,"revision":1}
import_asset      → {"assetId":"676c…","path":"…/clip.mp4"}       ← 真 ffprobe
append_clip       → ok，revision 3，自动建轨
render_status     → {"busy":false,"fraction":0.0}
export_video      → 129 KB mp4，1920×1080 / 25fps / 2s，warnings []
undo              → {"ok":true,"changed":true,"revision":4}
aicut_help        → 返回 render 域全文（skills/aicut/references/render.md）
```

**这一步抓到一个真 bug**：无头进程启动时文档引擎是空的，MCP 客户端一连上，
每个工具都回 `project not loaded`（桌面前端总会先 `doc_load`，容器里没人做这件事）。
现在 `aicutd` 启动时先载入一份空白工程——用领域层同一个 `newProject()`，规格与桌面「新建」
逐字段一致。

### 25.7 更正：容器可以有 GPU，"GPU 生成"不是物理限制

之前把「GPU 生成」归到"物理上不可对齐"是**说错了**，这里更正并记下来。

- **容器本身可以有 GPU**：Linux 宿主装 NVIDIA 容器运行时（或 CDI）后 `docker run --gpus all`
  就能直通；Windows 宿主走 WSL2 也有同样的路径；Intel / AMD 可以用 `/dev/dri` 直通做 VA-API。
  "容器没有 GPU"是个错误的一般化——这轮在 macOS 上构建，Docker Desktop 的 Linux 虚拟机确实
  不透传 Apple GPU，但那是**宿主平台**的限制，不是 Docker 的结构限制。
- **ai-cut 的 ONNX Runtime CUDA 路径在容器里是支持的代码**：`runtime_catalog` 里就有
  `microsoft.ml.onnxruntime.gpu.linux` 的 CUDA 12 / CUDA 13 运行库包
  （`libonnxruntime_providers_cuda.so` + cudart / cuBLAS / cuFFT / cuDNN），装到配置目录后
  `backend::preload_cuda` 会按裸文件名加载。默认镜像是 **CPU 版**，但这条路没有关。
- **真正的缺口是"生成器没下沉"**：本地出图 / 出视频走 `src-tauri/src/sdcpp.rs`
  （Windows 的 CPU / Vulkan / CUDA 构建 + macOS 的 Metal 构建），daemon 里没有对应实现。
  所以 `image_gen` / `video_gen` / `sdcpp` 三个能力现在标 `unsupported`，理由是
  **本地生成器只有桌面版**（云端出图 `cloud_gen_*` 三形态都可用），而不是"物理上没有 GPU"。
- 真的要做的量：把 sd.cpp 的二进制与权重打包路径搬进 daemon（`ai_backend::SdcppBackend`
  已经有类型），镜像里按 CPU 基线随包、CUDA / Vulkan 后端按需下载——和 `runtime_ort` 同一套思路。
  做完之后，**有 GPU 的 Linux 容器**就能跑；macOS 的 Docker Desktop 里仍然只能 CPU（极慢）。

顺带把 README 的能力表也改成了两行：插件 GUI / 系统级操作是物理限制；本地生成是"未下沉"。

## 26. 版本号 1.1.0 / 1.8.0 + 按版本重建镜像并实测（2026-11-15 第十二轮）

### 26.1 版本号

- ai-cut：`1.0.0 → 1.1.0`（`package.json`、`tauri.conf.json`、`src-tauri/Cargo.toml`、
  `crates/aicut`、`crates/daemon` —— 最后这个原先是 `0.1.0`，容器 `/health` 报的就是它，
  跟着一起对齐，否则镜像里会自称 0.1.0）
- ai-daw：`1.7.0 → 1.8.0`（`package.json`、`tauri.conf.json`、`src-tauri/Cargo.toml`、workspace 版本）

两个镜像按版本号重建：`ai-cut:1.1.0`（434 MB）、`ai-daw:1.8.0`（286 MB）。

### 26.2 这轮实测抓到的三个真问题

1. **镜像里的本地推理是坏的**：容器里 `/opt/aicut/ort/libonnxruntime.so.1.30.0` 是带版本号的
   文件名，而加载器只按裸名 `libonnxruntime.so` 找 → `找不到随包的 ONNX Runtime`，
   任何模型一建会话就 panic（`Failed to load ONNX Runtime dylib`）。
   修法两层：`ai_session::ort_library_in` 先认裸名、再扫 `libonnxruntime.so*`（多版本取名字最大的），
   Dockerfile 再补一个符号链接。修完实测容器里 `upscale_video`（随包 Real-ESRGAN）通过。
2. **容器里的 MCP 外部客户端连不上**：MCP 只绑 `127.0.0.1`，Docker 发布端口也到不了。
   现在 `--lan` 时 MCP 一起绑 `0.0.0.0`（桌面壳保持回环）。两仓都改了。
3. **镜像里的旧工具面**：验证时先看到 `tools/list` = 15，一度以为构建缓存没刷新——实际是
   宿主上还留着一个旧的 `aicutd` 占着 50563，curl 打到了它。教训写在这里：
   容器验证前先 `pkill -f aicutd` 并给 MCP 换一个不同宿主端口（这次用 `15063:50563`）。

### 26.3 容器实测结果

```
ai-cut:1.1.0   /health → {"version":"1.1.0"}            ✔ 版本与镜像标签一致
               /rpc/capabilities → 26 项，除物理项外全 ok
               MCP tools/list → 62 个（从宿主 127.0.0.1:15063 连入）
               new_project / import_asset（真 ffprobe）/ append_clip
               upscale_video（随包 Real-ESRGAN，ORT 修复验证）→ OK，duration 2.0
               export_video → /data/out.mp4，147 KB，ffprobe 2.000s，warnings []
               undo → revision 5

ai-daw:1.8.0   /health → {"version":"1.8.0","audio":true}
               MCP tools/list → 140 个（从宿主 127.0.0.1:15064 连入）
               transport_status → virtual_output: true（容器无声卡，虚拟时钟顶替）
               transport_play → playing: true，位置 505856 → 604160 帧（≈2s，48kHz 实时）
```

### 26.4 关于 GPU 的更正（承接 §25.7）

「GPU 生成」不在物理限制清单里，它的真实状态是：**容器可以有 GPU，缺的是把生成器下沉**。
`runtime_catalog` 里已经有 `onnxruntime.gpu.linux` 的 CUDA 12 / 13 运行库包，宿主编排
`--gpus all` 之后 ORT 的 CUDA EP 在容器里可用；`image_gen` / `video_gen` / `sdcpp` 之所以
还是 `unsupported`，是因为本地生成器（sd.cpp）只有桌面实现。macOS 的 Docker Desktop
不透传 Apple GPU，这台机器上验证不了 GPU 路径，但那是宿主平台限制。
