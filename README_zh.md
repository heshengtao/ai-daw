<p align="center">
  <img src="./assets/readme/hero.svg" width="100%" alt="AI DAW —— 本地优先的 AI 数字音频工作站">
</p>

<p align="center">
  <img src="https://img.shields.io/badge/version-1.0.0-1A6CFF?style=flat-square" alt="版本 1.0.0">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-1A6CFF?style=flat-square" alt="许可：Apache-2.0"></a>
  <img src="https://img.shields.io/badge/platforms-Windows%20/%20macOS%20/%20Linux-1A6CFF?style=flat-square" alt="平台：Windows、macOS、Linux">
</p>

<p align="center">
  <a href="README_zh.md"><b>简体中文</b></a> ·
  <a href="README.md"><b>English</b></a>
</p>

## 什么是 AI DAW？

**AI DAW 是一款小巧、AI 原生的桌面音乐工作室**——写作、编排、混音、母带都在一个轻量应用里完成，同时还能让 AI 代理和你一起在工程里干活。

它是一款真正的 DAW：多轨道时间线、钢琴卷帘、带发送与总线的调音台、自动化车道、内置合成器，以及 VST3 / CLAP 插件宿主。它同时也是**为被模型操控而设计的**——随仓库附带的 Skill 能教会任意 AI Harness 通过 MCP 驱动正在运行的软件，代理可以建轨、写音符、调混音、做分析，而你在界面上实时看着它发生。

一切都在你自己的电脑上运行。不要账号、不要 API Key、音频不出本机。

<p align="center">
  <img src="./assets/screenshots/01-arrange.png" width="100%" alt="AI DAW 的编排视图：三条轨道的 MIDI 片段、时间标尺、检查器与 AI 面板">
</p>

## 为什么这件事值得做

模型之所以学会写代码，是因为它们能通过编程智能体框架**真的把代码跑起来**；模型之所以学会 3D 建模，是因为它们能通过 Blender MCP 操控真实的建模软件。这两者都让模型拿到了真实的工具、真实的结果和真实的反馈闭环——快速迭代正是从这里来的。

音乐领域一直缺这个闭环。模型可以生成音频，但它没法像人一样**在一个工程上工作**：没有可编辑的编排、没有可推的推子、没有撤销历史，也无从判断自己上一步改动是不是把东西改坏了。

**AI DAW 就是为填上这个缺口做的一次小巧而认真的尝试。** 它是一款真正的 DAW，AI 代理可以通过 Skill 与 MCP 来操作它，因此它可以用来：

- **检验模型到底懂不懂写歌**——不是看它生成的音频像不像样，而是看它能不能搭起编曲、选出合适的声部、把混音修好。
- **采集真实的写歌轨迹**——代理的每一次修改都落在正常的工程里、进入正常的撤销历史，于是一次会话就成了「模型如何思考音乐」的数据。
- **朝着「模型能像写代码一样写歌」再走一步**——给它一个会回话的工具。

## 轻量化是设计目标

小巧不是妥协，而是特性。

- **体积小**——没有 Electron、不依赖云、不打包模型全家桶。macOS 版整包约 38 MB；作为参照，Ableton Live、Logic Pro、FL Studio 这类主流 DAW 的安装体积通常在 1–3 GB，AI DAW 只有它们的 2%–4%，**体积缩小了 95% 以上**。
- **启动快**——开机不加载任何重东西，AI 模型只在你用到时才引入。
- **高算力的活交给该干的地方**——音乐生成、音色转换这类重模型**刻意不做**本地集成。由你的 AI Harness 生成音频，产物可以直接导入 DAW，做编辑、混音与母带后处理。
- **生来可扩展**——让外部 AI 模型接进工作流的接入接口已在路线图上。

## 用任意 AI Harness 驱动它

AI DAW 附带一个 **Skill**（`skills/aidaw`），它教会 AI 代理如何操作正在运行的软件——读写工程、轨道、片段与音符；混音（增益、声像、发送、自动化）；传输与录音；响度与频谱分析；导出 WAV；扫描插件。它通过内置的 **MCP** 服务器工作，因此任何支持 Agent Skill 的 Harness 都能直接用。

**方式一 · 命令行安装**

```bash
npx skills add heshengtao/ai-daw
```

**方式二 · 让你的代理自己装**

```
Install this Skill: https://github.com/heshengtao/ai-daw
```

代理做的每一次修改都走与界面完全相同的命令管线：进入撤销历史、显示在界面上、并署名给代理——所以你随时能看到它做了什么，也能随时撤销。

## 功能

### 🎹 编排与编辑 —— 在真正的时间线上作曲

怎么想的就怎么摆。跨轨道拖拽片段，在音乐变化处切分，标尺与网格跟着可在曲中任意位置改变的速度与拍号走。MIDI 片段自带音符预览、音频片段自带波形，整首歌一眼就能读出来。

- **多轨道编排** —— MIDI 与音频片段并排，跨轨道拖放，框选
- **看得懂的钢琴卷帘** —— 片段直接显示音符；编辑器提供力度、缩放与键盘
- **动态速度与拍号** —— 任意位置变速 / 变拍，网格自动跟随
- **走带与循环** —— 播放、停止、定位、划定循环区、节拍器
- **自动化车道** —— 为插件或内置合成器暴露的任何参数画曲线
- **完整的编辑工具** —— 切分、移动、复制粘贴、撤销 / 重做，还带一张快捷键速查表

<p align="center">
  <img src="./assets/screenshots/02-edit.png" width="100%" alt="钢琴卷帘编辑器：一段和弦片段的音符与底部的力度车道">
</p>

### 🎚️ 混音与母带 —— 把作品做完

先做平衡，再做响度。通道条提供增益、声像、发送、总线、静音 / 独奏与电平表；总线上的母带链负责响度、峰值与立体声宽度，不用离开应用就能导出成品。

- **调音台** —— 增益、声像、发送、总线、静音 / 独奏、电平表
- **内置效果器** —— 动态、均衡与时间类效果，随手加到任意轨道
- **母带链** —— 响度归一、真峰值限幅、立体声宽度
- **内置合成器** —— 振荡器、包络、滤波，配实时示波器

<p align="center">
  <img src="./assets/screenshots/03-mix.png" width="100%" alt="调音台：总输出与各轨道的通道条，含推子、声像与静音 / 独奏">
</p>

### 🧩 插件 —— 带你自己的乐器与效果器

AI DAW 支持 **VST3** 与 **CLAP** 插件，你已有的工具可以直接带过来。插件管理器能搜索开放注册表，按类型与系统兼容性筛选，一键安装 / 更新 / 卸载——也可以直接指向你本机已经装好的插件。

- **VST3 / CLAP 宿主** —— 在任意轨道加载第三方乐器与效果器
- **插件管理器** —— 搜索注册表，按乐器 / 效果与平台筛选，安装 / 更新 / 卸载
- **本地导入** —— 把已经在别处装好的插件接进来
- **界面与参数** —— 打开插件自带的界面，或从检查器与自动化车道驱动它的参数

<p align="center">
  <img src="./assets/screenshots/04-plugins.png" width="100%" alt="插件面板：内置合成器与效果器列表，可按内置 / 外部插件筛选">
</p>

### 🤖 本地 AI —— 把音频变成能继续加工的材料

这些 AI 功能是为真实的作品准备的，不是演示品。把一段素材交给它，拿回来的是能接着编辑的东西：可以挪动的 MIDI 音符、更干净的人声、可以重新配比的分轨。全部跑在你自己的硬件上——默认 CPU，可选 CoreML（macOS）与 DirectML（Windows）加速。

- **节拍检测** —— Beat This!（ISMIR 2024）一次推理得出全局 BPM 与拍号
- **音频转 MIDI** —— Basic Pitch 把录下的演奏变成可编辑的音符
- **人声降噪 / 去混响** —— LocalVQE 处理人声素材
- **音源分离** —— HTDemucs v4 拆成 4 / 6 条分轨，方便重新配比
- **生来离线** —— 不要账号、不要 API Key、不上传任何东西

<p align="center">
  <img src="./assets/screenshots/05-ai-panel.png" width="100%" alt="AI 面板：MCP 服务器状态，以及音频转 MIDI、人声降噪、音源分离、节拍检测的入口">
</p>

### 🔌 MCP 服务器 —— 让 AI 代理在你的工程里干活

打开一个开关，你的工程就变成 AI 助手可以操作的对象。内置的 **MCP（Model Context Protocol）服务器**把会话通过只监听 `127.0.0.1` 的 HTTP 端点开放出去——接上你的代理、给它一个任务，就能看它加片段、改音符、调电平，界面实时跟着变。`aidaw` 命令行就是同一个服务器的薄客户端。

- **MCP over HTTP** —— 只监听回环地址的接入点
- **实时改工程** —— 代理的操作会立刻以正常的工程改动出现
- **`aidaw` 命令行** —— 可在设置里一键挂到 `PATH`

<p align="center">
  <img src="./assets/screenshots/06-settings-ai-mcp.png" width="100%" alt="设置 → AI / MCP：执行后端选择与 MCP 服务器接入地址">
</p>

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
| macOS（Apple Silicon） | ✅ 支持（.dmg 首次打开见「App 已损坏」说明） |
| Linux (x86_64 / arm64) | ✅ 支持 |

> **macOS 提示「App 已损坏，无法打开」？** 应用为自签名（未公证），从网络下载会带上隔离属性。在终端执行下面这行命令后即可正常打开：
>
> ```bash
> xattr -cr "/Applications/AI DAW.app"
> ```

> **暂不支持 Intel Mac。** ONNX Runtime 不提供 `x86_64-apple-darwin` 的预编译二进制，
> 本地 AI 模型无法为 Intel macOS 链接。

## 常见问题

### AI DAW 是免费的吗？

是。AI DAW 以 Apache 2.0 开源，没有付费档位，也没有功能阉割。

### 能离线使用吗？

能。应用与所有 AI 模型都在本地运行。唯一的联网行为是下载插件包与 AI 模型（由你发起），以及拉取插件注册表。

### 需要显卡吗？

不需要。AI 模型默认走 CPU；macOS 可用 CoreML、Windows 可用 DirectML（需硬件支持），两者都是可选项。

### 哪些 AI 模型随应用一起提供？

两个小模型随包：**Basic Pitch**（音频转 MIDI）与 **LocalVQE**（人声降噪），装好即可用。较大的模型——**Beat This!**（节拍检测）与 **HTDemucs**（音源分离，4 / 6 分轨）——首次使用时由应用内的模型管理器按需下载，存放在源码树之外。

### 支持哪些插件格式？

VST3 与 CLAP。

### 让代理驱动 DAW，需要软件一直开着吗？

需要。Skill 与 `aidaw` 命令行是通过 MCP 与**正在运行**的 AI DAW 通信的，所以先启动软件（用 `aidaw status` 可以确认是否已启动）。

## 许可

Apache-2.0，见 [LICENSE](LICENSE)。

## 第三方许可

本项目建立在第三方开源软件之上。改编自其它项目的文件，以及许可不在宽松默认集合内的依赖，都记录在 [NOTICE](NOTICE) 中。
