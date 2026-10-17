# AI DAW

> 本地优先的 AI 数字音频工作站 ｜ A local-first, AI-native digital audio workstation

用 **Tauri v2 + Rust 音频引擎 + Vue 3** 构建的桌面 DAW。实时音频引擎跑在 Rust 侧，
界面用 Material You 风格自绘；本地 AI 模型（节拍检测、音频转 MIDI、人声降噪、音源分离）
全部离线推理，另内置一个 MCP 服务器，让 AI 代理可以直接操作工程。

A desktop DAW built with **Tauri v2 + a Rust audio engine + Vue 3**. All local AI
models run offline; a built-in MCP server lets AI agents drive the project directly.

---

## 功能

**编排与编辑**
- 多轨道编排视图、迷你钢琴卷帘、音频波形片段、框选 / 拖拽 / 剪切 / 复制粘贴
- 动态速度与拍号、走带循环、标尺与网格自动对齐
- 内置合成器（振荡器 / 包络 / 滤波）、参数自动化车道

**混音**
- 调音台：增益、声像、发送、总线、静音 / 独奏
- 内置效果器、母带处理（响度归一 + 真峰值限幅 + 立体声宽度）

**插件**
- VST3 / CLAP 插件宿主
- 插件管理器：搜索、按类型与兼容性筛选、一键安装 / 更新 / 卸载、本地导入

**本地 AI（离线，ONNX）**
- **节拍检测** — Beat This!（ISMIR 2024）：一次推理给出全局 BPM + 拍号
- **音频转 MIDI** — Basic Pitch
- **人声降噪 / 去混响** — LocalVQE
- **音源分离** — HTDemucs v4，可选 4 / 6 分轨

**AI 代理接入**
- 内嵌 MCP 服务器（Streamable HTTP，只监听 `127.0.0.1`），AI 代理可直接读写工程
- `aidaw` 命令行工具（MCP 薄客户端），可一键挂到 `PATH`

## 技术栈

| 层 | 选型 |
| --- | --- |
| 桌面外壳 | Tauri v2 |
| 音频引擎 | Rust（`cpal` 实时音频、`rtrb` 无锁环、`hound` / Symphonia 解码） |
| 本地推理 | `ort`（ONNX Runtime）/ `rten`（纯 Rust） |
| 插件宿主 | VST3 / CLAP |
| AI 代理 | `rmcp` + `axum`（MCP over HTTP） |
| 前端 | Vue 3 + Vite + TypeScript + Pinia + Tailwind（Material You） |

## 仓库结构

```
crates/
  engine/        实时音频引擎：播放、调度、总线、走带、时间拉伸
  ipc/           前后端命令与事件的共享契约（serde 模型）
  synth/         内置合成器
  effects/       内置效果器
  asset/         音频解码与素材管理
  analysis/      频谱 / 峰值 / RMS 量化分析
  host-vst3/     VST3 / CLAP 插件宿主
  plugin-api/    插件抽象
  local-ai/      本地 AI 模型（Beat This!、Basic Pitch、LocalVQE、HTDemucs）
  mastering/     母带处理（响度归一、真峰值限幅、立体声宽度）
  aidaw/         aidaw 命令行（MCP 薄客户端）
src-tauri/       应用外壳：Tauri 命令、MCP 服务器、插件管理器、模型管理器
src/             Vue 3 前端（视图、组件、Pinia stores、i18n）
tools/           构建与模型脚本（fetch-models.sh、build-aidaw.sh 等）
docs/            设计稿与路线图（docs/superpowers）
```

## 快速开始

**环境要求**
- Rust（stable）与 Cargo
- Node.js ≥ 18 与 [pnpm](https://pnpm.io/)
- Tauri v2 所需的系统依赖，见 <https://tauri.app/start/prerequisites/>

**开发**

```bash
pnpm install
pnpm tauri dev
```

**构建**

```bash
pnpm tauri build      # 打包桌面应用
pnpm build            # 仅做类型检查 + 前端构建
```

**命令行工具**

```bash
pnpm build:aidaw      # 构建 aidaw CLI（见 tools/build-aidaw.sh）
```

## 本地 AI 模型

模型权重**不进仓库**，按需下载：

- `mel_spectrogram.onnx`（约 264 KB 的 log-mel 前端）**随安装包分发**
- 其余权重（如 Beat This! 约 83 MB、Demucs、LocalVQE、Basic Pitch）在
  **「管理器 → 可下载 → 模型」**里按需下载，或在设置里配置镜像 / 代理后拉取
- 开发期也可以用 `tools/fetch-models.sh` 预先拉取到仓库根的 `models/`（该目录已被忽略）

## 许可

本项目以 **Apache License 2.0** 授权，见 [LICENSE](./LICENSE)。

再分发时请一并保留 [NOTICE](./NOTICE)：其中记录了改编自其它开源项目的文件
（均已按其许可授权）以及第三方依赖里非宽松许可的说明。

## 致谢

界面设计系统（Material You 令牌与组件层）改编自
[Exameow](https://github.com/heshengtao/exameow)；Demucs 的 STFT 前端改编自
[stem-splitter-core](https://github.com/gentij/stem-splitter-core)。详见 [NOTICE](./NOTICE)。
