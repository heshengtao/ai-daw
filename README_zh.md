<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW —— 本地优先的 AI 数字音频工作站">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-0.1.0-1A6CFF?style=flat-square" alt="版本 0.1.0">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="许可：Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux-1A6CFF?style=flat-square" alt="平台：Windows、macOS、Linux">
</p>

<p align="center">
  <a href="README_zh.md"><b>简体中文</b></a> ·
  <a href="README.md"><b>English</b></a>
</p>

## 什么是 AI DAW？

**AI DAW** 是一款开源、**本地优先的数字音频工作站**，用 **Tauri v2**、**Rust 音频引擎**与 **Vue 3** 界面构建。编排、编辑、混音、母带都在你自己的机器上完成——引擎和每一个 AI 模型都跑在本地，而不是别人的云里。

引擎用 Rust 编写，通过 `cpal` 与声卡通信，采样经无锁环形缓冲传递，实时线程永不阻塞。编排、走带、调度、总线、时间拉伸都在引擎里；界面是 Material You 风格的 Vue 3 应用，通过一份带类型的命令 / 事件契约与引擎同步状态。

本地 AI **完全离线**：节拍检测、音频转 MIDI、人声降噪、音源分离全是本地执行的 ONNX 模型——不要账号、不要 API Key、音频不出本机。内置的 **MCP 服务器**（Model Context Protocol）进一步把工程开放给 AI 代理，让助手能直接读写你的工程。

## 功能

### 🎹 编排与编辑 —— 在真正的时间线上作曲

熟悉的编排视图：多轨道、迷你钢琴卷帘片段、音频波形片段。可跨轨道选中、拖拽、切分、复制、粘贴、删除片段；标尺与网格始终跟着可在曲中任意位置变化的动态速度与拍号。走带提供播放、停止、定位、循环区与节拍器，自动化车道则能为插件或内置合成器暴露的任何参数画出曲线。

- **多轨道编排** —— MIDI 与音频片段并排，跨轨道拖放，框选
- **迷你钢琴卷帘与波形** —— 片段自带音符或波形，一眼读懂整首曲子
- **动态速度与拍号** —— 时间线上任意位置都可变速 / 变拍，网格自动跟随
- **走带与循环** —— 播放、停止、定位、循环区、节拍器
- **自动化车道** —— 绘制参数曲线，用参数浏览器挑选要自动化的目标
- **编辑** —— 切分、移动、复制粘贴、撤销 / 重做，还带一张快捷键速查表

### 🎚️ 混音与母带 —— 把作品做完

调音台每轨都有增益、声像、发送、总线、静音 / 独奏与电平表，另配内置效果器。一条母带链——响度归一、真峰值限幅、立体声宽度——让你不用离开应用就能导出成品。

- **调音台** —— 增益、声像、发送、总线、静音 / 独奏、电平表
- **内置效果器** —— 开箱即用的动态、均衡与时间类效果
- **母带链** —— 响度归一 + 真峰值限幅 + 立体声宽度
- **内置合成器** —— 振荡器、包络、滤波，配实时示波器

### 🧩 插件 —— 带你自己的乐器与效果器

支持 **VST3** 与 **CLAP** 插件。插件管理器可搜索开放注册表、按类型与系统兼容性筛选，一键安装 / 更新 / 卸载——也可以直接导入你本机已装好的插件。

- **VST3 / CLAP 宿主** —— 在任意轨道加载第三方乐器与效果器
- **插件管理器** —— 搜索开放注册表，按乐器 / 效果与兼容性筛选，安装 / 更新 / 卸载
- **本地导入** —— 把管理器指向你已经在别处装好的插件
- **插件界面与参数** —— 打开插件自带的编辑器，或从检查器与自动化车道驱动其参数

### 🤖 本地 AI 模型 —— 离线，跑在你自己机器上

每个 AI 功能都是一个本地 ONNX 模型，默认走 CPU，硬件支持时可选 **CoreML**（macOS）与 **DirectML**（Windows）执行后端。权重由应用内的**模型管理器**按需下载，任何东西都不上传。

- **节拍检测** —— Beat This!（ISMIR 2024）一次推理给出全局 BPM 与拍号
- **音频转 MIDI** —— Basic Pitch 把录下的演奏变成可编辑的 MIDI 音符
- **人声降噪 / 去混响** —— LocalVQE 处理人声素材
- **音源分离** —— HTDemucs v4，可选 4 / 6 分轨
- **执行后端** —— 默认 CPU，可用时走 CoreML / DirectML

### 🔌 MCP 服务器 —— 让 AI 代理操作 DAW

内置的 **MCP（Model Context Protocol）服务器**通过 Streamable HTTP 把工程开放给 AI 代理，且只监听 `127.0.0.1`。在**设置 → AI / MCP**里开启、复制接入地址，让你的代理连上——它可以加片段、改音符、调增益等等，每一处改动都会实时反映到界面上。

- **MCP over HTTP** —— 只监听回环地址的接入点
- **实时改工程** —— 代理的操作会立刻以正常的工程改动出现
- **`aidaw` 命令行** —— 一个 MCP 薄客户端，可在设置里一键挂到 `PATH`

## 安装

预编译安装包将发布在 [GitHub Releases](https://github.com/heshengtao/ai-daw/releases) 页面；在那之前请从源码构建。

### 环境要求

- Rust（stable）与 Cargo
- Node.js ≥ 18 与 [pnpm](https://pnpm.io/)
- Tauri v2 所需的系统依赖 —— 见 <https://tauri.app/start/prerequisites/>

### 从源码构建

```bash
git clone https://github.com/heshengtao/ai-daw.git
cd ai-daw

pnpm install
pnpm tauri dev        # 开发模式运行
pnpm tauri build      # 打包发布版本
```

其它常用命令：

```bash
pnpm build            # 对前端做类型检查并构建
pnpm build:aidaw      # 构建 aidaw CLI（见 tools/build-aidaw.sh）
```

### 平台支持

| 平台 | 状态 |
| --- | --- |
| Windows (x86_64) | ✅ 支持 |
| macOS（Apple Silicon / Intel） | ✅ 支持 |
| Linux (x86_64) | ✅ 支持 |

## 常见问题

### AI DAW 是免费的吗？

是。AI DAW 以 Apache 2.0 开源，没有付费档位，也没有功能阉割。

### 能离线使用吗？

能。引擎、界面和所有 AI 模型都在本地运行。唯一的联网行为是下载插件 / 模型包（由你决定）与拉取插件注册表。

### 需要显卡吗？

不需要。AI 模型默认走 CPU；macOS 可用 CoreML、Windows 可用 DirectML（需硬件支持），两者都是可选项。

### 支持哪些插件格式？

VST3 与 CLAP。

### AI 代理通过 MCP 服务器能做什么？

开启 MCP 服务器后，代理可以通过一个只监听回环地址的 HTTP 接入点读取工程并发起改动——加片段、改音符、调增益等等。`aidaw` 命令行就是同一个服务器的一个薄客户端。

### AI 模型权重放在哪里？

一个小体积的 log-mel 前端随应用提供；较大的权重由应用内的模型管理器按需下载，存放在仓库之外，因此永远不会被提交进版本库。

## 许可

Apache-2.0，见 [LICENSE](LICENSE)。

## 第三方许可

本项目建立在第三方开源软件之上。改编自其它项目的文件，以及许可不在宽松默认集合内的依赖，都记录在 [NOTICE](NOTICE) 中。
