# 域：制作 CLAP 插件（clap-plugin-dev）

**先找现成的，再考虑自己做。** 用户说「我想要一个 XX 插件」时，顺序是：

1. `list_plugins(query=...)` 看本机 / 内置有没有对得上的。
2. 没有 → 告诉用户「可以做一个 CLAP 插件」，说清能做到什么、做不到什么，**得到同意再开工**。
3. 做完必须过 `clap-validator`，再装进 AI DAW 走一遍真实链路（`rescan_plugins` → 挂轨 → 出声 → 导出）。

> 本文是「做一个能被 AI DAW 正常加载的 CLAP 插件」的施工手册，不是 DSP 教程。
> 音色/效果算法本身按需求写；这里只保证 **ABI、线程、打包、验证** 不出错。

## 为什么优先做 CLAP

CLAP（**CL**ever **A**udio **P**lugin）由 Bitwig 与 u-he 在 2022 年发起，**MIT 许可**，
是一个稳定的 C ABI：按 CLAP 1.x 编译的插件能被任何 CLAP 1.y 宿主加载。

- **开源社区的事实标准**：Surge XT、Vital、Airwindows、Linux Studio Plugins、DISTRHO、
  ChowDSP、VCV 等开源项目都提供 CLAP。
- **商业插件也大量跟进**：u-he、FabFilter、AudioThing、Audio Damage、Audiority、TAL、
  Plogue、Martinic 等都有 CLAP 版本。
- **宿主**：Bitwig Studio、REAPER、FL Studio、Studio One、Carla、Qtractor、Zrythm、
  MultitrackStudio、MuLab、Plogue Bidule、Anklang 等。
- **它不是万能格式**：Logic 走 AU，Pro Tools 走 AAX，Cubase / Nuendo / Ableton 仍以 VST3 为主，
  截至 2026-10 都没有 CLAP。要发给这些用户，通常得同时出 VST3。

**对本项目来说 CLAP 是最省事的目标**：AI DAW 原生托管 CLAP，跨 macOS / Linux / Windows，
不需要 Steinberg SDK、不需要授权、不需要 Apple 的 AU 验证。先把 CLAP 做对，再按需加 VST3。

## 产物长什么样、装到哪里

| 平台 | 构建产物 | 用户级安装目录 |
| --- | --- | --- |
| macOS | `Name.clap/Contents/MacOS/Name` + `Contents/Info.plist`（严格宿主只认 bundle） | `~/Library/Audio/Plug-Ins/CLAP/` |
| Linux | 共享库，文件名 `Name.clap`（ELF） | `~/.clap/`（系统级 `/usr/lib/clap`、`/usr/local/lib/clap`） |
| Windows | DLL，文件名 `Name.clap` | `%LOCALAPPDATA%\Programs\Common\CLAP\`（系统级 `%COMMONPROGRAMFILES%\CLAP`） |

- macOS 上把 `.dylib` 直接改名成 `.clap` 有时能加载，但 Bitwig 等严格宿主会挑刺；
  **一律产出 bundle**。
- 入口符号必须导出：`clap_entry`（C/C++ 用 `CLAP_EXPORT`，Rust 由 `nih_export_clap!` 生成）。
- **AI DAW 的扫描目录**（`crates/host-vst3/src/scan.rs`）：
  - macOS：`~/Library/Audio/Plug-Ins/CLAP`、`~/.clap`、`/Library/Audio/Plug-Ins/CLAP`
  - Linux：`~/.clap`、`/usr/lib/clap`、`/usr/local/lib/clap`
  - Windows：`C:\Program Files\Common Files\CLAP`、`C:\Program Files (x86)\Common Files\CLAP`，以及 `~/.clap`
  - 插件管理器装进应用数据目录的 `plugins/clap` 也会被扫描。
  - **AI DAW 目前不读 `CLAP_PATH`**：本地开发请装进上面任一目录，再 `rescan_plugins`。
  - Windows 注意：标准的用户级目录 `%LOCALAPPDATA%\Programs\Common\CLAP` **不在** AI DAW 的扫描列表里；
    要么放到 `C:\Program Files\Common Files\CLAP`（需要管理员权限），要么用插件管理器目录。

## 最小 ABI 清单（不写框架时照这个对）

一个能被加载、能出声、能被保存的插件至少要有：

1. **entry**：导出的 `clap_entry`，含 `clap_version` / `init` / `deinit` / `get_factory`。
   `get_factory("clap.plugin-factory")` 返回插件工厂。
2. **factory**：`get_plugin_count` / `get_plugin_descriptor` / `create_plugin`，三个都必须线程安全。
   `create_plugin` 里**只能保存 host 指针，不能调用 host**。
3. **descriptor**：`id` 用反向域名（如 `com.example.myplugin`），**发布后永不改**；
   `features` 决定宿主怎么分类。AI DAW 只把含 `"instrument"` 的当乐器，其余都当效果器。
4. **plugin 回调**：`init` / `destroy` / `activate` / `deactivate` / `start_processing` /
   `stop_processing` / `reset` / `process` / `get_extension` / `on_main_thread`。
5. **扩展**（按需，但推荐全给）：
   - `clap.audio-ports`：效果器 2 进 2 出；乐器只有 2 出。
   - `clap.note-ports`：乐器必须有 1 个音符输入；`preferred_dialect` 用 `CLAP_NOTE_DIALECT_CLAP`。
   - `clap.params`：所有可调参数。没有它，AI DAW 的 `describe_plugin` / 参数面板就是空的。
   - `clap.state`：工程保存/恢复。没有它，重开工程参数会丢（宿主不该替你兜底）。
   - 可选：`clap.latency`、`clap.tail`、`clap.gui`、`clap.thread-check`。

### 线程规则（插件会因为违规直接挂住）

| 调用 | 线程 |
| --- | --- |
| `entry.init` / `deinit` / `get_factory` | 任意线程，但不能并发；`init` 要快、禁止弹 GUI |
| `factory.*` | 任意线程，必须线程安全 |
| `plugin.init` / `params.*` / `activate` / `deactivate` / `gui.*` / `destroy` | **主线程** |
| `start_processing` / `stop_processing` / `reset` / `process` / `params.flush`（已激活时） | **音频线程** |
| `state.save` / `state.load` | **主线程** |

`process` 里**不分配内存、不加锁、不写日志、不做文件 IO**。需要分配的东西在 `activate` 里准备好。

### process 的硬要求

- 输入事件按 `event->time`（块内采样偏移）排序，必须逐个处理；至少处理
  `NOTE_ON` / `NOTE_OFF` / `NOTE_CHOKE` / `PARAM_VALUE` / `PARAM_MOD` / `MIDI` / `TRANSPORT`。
- 音频缓冲可能是 `data32` 也可能是 `data64`，**先判空再用**；输入输出可能同址（in-place）。
- 只写 `audio_outputs`，不要假设输入总线一定存在（乐器没有输入）。
- 返回值用 `CLAP_PROCESS_CONTINUE` 最稳；确定没声音时才用 `CONTINUE_IF_NOT_QUIET` / `SLEEP`。

### 参数与状态的两个常见误会

- **CLAP 没有「设置参数」这个调用**。宿主改参数 = 在下一块 `process`（或 `flush`）里送
  `CLAP_EVENT_PARAM_VALUE`；插件必须在这个事件里更新自己的 DSP。AI DAW 的 `set_params` 也走这条路。
- 插件自己改参数（例如 GUI 拧旋钮）要发**输出事件**，再 `host->params->request_flush()`；
  否则宿主界面不会更新。
- `state.save/load` 是字节流：自己定版本号，读的时候处理短读；参数值也建议一起写进去。

## 三条实现路线

### 路线 A：Rust + nih-plug（推荐，最快出可用产物）

nih-plug 支持 CLAP + VST3，自带参数系统、状态、采样率/块大小处理、
`cargo xtask bundle` 跨平台打包。官方仓库目前是**维护模式**，但依旧被大量插件使用；
想要更活跃的社区分支可以看 `nice-plug`。

```bash
# 可选：用官方 cookiecutter 模板起步
cookiecutter gh:robbert-vdh/nih-plug-template
```

**CLAP-only 的 `Cargo.toml`**（关掉默认的 `vst3` feature，避免把 GPLv3 的 VST3 绑定带进来）：

```toml
[package]
name = "aidaw_gain"
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace]
members = ["xtask"]

[lib]
crate-type = ["cdylib"]

[dependencies]
nih_plug = { git = "https://github.com/robbert-vdh/nih-plug.git", default-features = false, features = ["assert_process_allocs"] }

[profile.release]
lto = "thin"
strip = "symbols"
```

`xtask/Cargo.toml` + `xtask/src/main.rs` + `.cargo/config.toml`（少了这一行 `cargo xtask` 会报
`no such command`）：

```toml
# .cargo/config.toml
[alias]
xtask = "run --package xtask --release --"
```

```toml
# xtask/Cargo.toml
[package]
name = "xtask"
version = "0.1.0"
edition = "2021"

[dependencies]
nih_plug_xtask = { git = "https://github.com/robbert-vdh/nih-plug.git" }
```

```rust
fn main() -> nih_plug_xtask::Result<()> {
    nih_plug_xtask::main()
}
```

**最小立体声增益效果器 `src/lib.rs`**：

```rust
use nih_plug::prelude::*;
use std::sync::Arc;

struct AidawGain {
    params: Arc<AidawGainParams>,
}

#[derive(Params)]
struct AidawGainParams {
    #[id = "gain"]
    pub gain: FloatParam,
}

impl Default for AidawGain {
    fn default() -> Self {
        Self { params: Arc::new(AidawGainParams::default()) }
    }
}

impl Default for AidawGainParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new(
                "Gain",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-30.0),
                    max: util::db_to_gain(30.0),
                    factor: FloatRange::gain_skew_factor(-30.0, 30.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

impl Plugin for AidawGain {
    const NAME: &'static str = "AIDAW Gain";
    const VENDOR: &'static str = "AIDAW";
    const URL: &'static str = "https://github.com/heshengtao/ai-daw";
    const EMAIL: &'static str = "noreply@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        for channel_samples in buffer.iter_samples() {
            let gain = self.params.gain.smoothed.next();
            for sample in channel_samples {
                *sample *= gain;
            }
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for AidawGain {
    const CLAP_ID: &'static str = "com.aidaw.examples.gain";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Stereo gain example");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
        ClapFeature::Utility,
    ];
}

nih_export_clap!(AidawGain);
```

构建与安装：

```bash
cargo xtask bundle aidaw_gain --release
# 产物：target/bundled/aidaw_gain.clap

# macOS
mkdir -p ~/Library/Audio/Plug-Ins/CLAP
cp -R target/bundled/aidaw_gain.clap ~/Library/Audio/Plug-Ins/CLAP/
# Linux
cp target/bundled/aidaw_gain.clap ~/.clap/
```

**改成乐器（合成器）时只改这几处**：

```rust
const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
    main_input_channels: None,
    main_output_channels: NonZeroU32::new(2),
    ..AudioIOLayout::const_default()
}];
const MIDI_INPUT: MidiConfig = MidiConfig::Basic;

// ClapPlugin 里必须让 AI DAW 认出这是乐器：
const CLAP_FEATURES: &'static [ClapFeature] = &[
    ClapFeature::Instrument,
    ClapFeature::Synthesizer,
    ClapFeature::Stereo,
];
```

然后在 `process` 里用 `context.next_event()` 读 `NoteEvent::NoteOn/NoteOff/PolyPressure`。
复音、参数调制、每音表达可以直接抄 nih-plug 的 `plugins/examples/poly_mod_synth`；
单音最小例子抄 `plugins/examples/sine`。

> **许可证**：nih-plug 本体是 ISC；默认的 `vst3` feature 会引入 GPLv3 的 `vst3-sys`。
> 只做 CLAP 时务必 `default-features = false`。要同时发 VST3，就得接受 GPLv3 兼容要求，
> 或者换成不依赖该绑定的方案。

### 路线 B：C++ + 官方 clap-helpers

- 只需要官方头文件 `free-audio/clap` 的 `include/clap`，加一层 header-only 的
  `free-audio/clap-helpers`（`clap/helpers/plugin.hh`）就能少写大量胶水。
- 跨格式发布用 `free-audio/clap-wrapper`（把 CLAP 包成 VST3 / AU / AAX）；
  JUCE 项目用 `clap-juce-extensions`；iPlug2 原生支持 CLAP。
- 带 WebView 界面的例子看 `signalsmith-clap-cpp`。

要点：入口仍必须导出 `clap_entry`；CMake 里把产物命名成 `.clap`；
macOS 必须打成 bundle，`Contents/Info.plist` 至少给 `CFBundleExecutable`、
`CFBundleIdentifier`、`CFBundlePackageType`（`BNDL`）。

```cmake
add_library(aidaw_gain MODULE src/plugin.c)
target_include_directories(aidaw_gain PRIVATE third_party/clap/include)
set_target_properties(aidaw_gain PROPERTIES PREFIX "" OUTPUT_NAME "aidaw_gain")

if(APPLE)
  set_target_properties(aidaw_gain PROPERTIES
    BUNDLE TRUE
    BUNDLE_EXTENSION "clap"
    MACOSX_BUNDLE_INFO_PLIST "${CMAKE_CURRENT_SOURCE_DIR}/Info.plist.in")
else()
  set_target_properties(aidaw_gain PROPERTIES SUFFIX ".clap")
endif()
```

### 路线 C：纯 C 最小实现

直接以官方 `src/plugin-template.c` 为起点，只依赖 `include/clap/clap.h`。
适合「一个文件、没有构建系统负担」的小效果器。代价是参数、状态、端口、GUI 都要自己接线，
出错面比前两条路线大；除非用户明确要求零依赖 C，否则优先 A 或 B。

## 装进 AI DAW 的验收闭环

```text
写代码 → 构建 .clap → 装到标准目录 → rescan_plugins
      → list_plugins(query="名字")          # 确认能看到、分类对
      → set_track_instrument / add_effect    # 挂到轨道
      → describe_plugin                      # 确认参数表完整
      → set_params / set_effect_params       # 改参数，确认有反应
      → 写音符 / export_wav / analyze_project # 确认真的出声、电平正常
      → 保存工程 → 重开 → 参数是否恢复        # 确认 state 扩展有效
```

AI DAW 侧的行为口径：

- **分类**：descriptor 的 `features` 里有 `"instrument"` → 乐器；否则 → 效果器。
- **参数**：`describe_plugin` 读的是 `clap.params`。`count` 返回 0 就什么都看不到。
- **参数值按宿主范围传**：AI DAW 读写的是 CLAP 的 `min_value..max_value`。nih-plug 常见的是
  **0..1 归一值**（dB / Hz 只出现在显示文本里），所以 `set_params` 要照 `describe_plugin`
  给的范围传，不要按物理量猜。
- **改参数**：`set_params` / `set_effect_params` 会把 `CLAP_EVENT_PARAM_VALUE` 送进下一块
  `process`；插件不处理这个事件，界面就会「调了没反应」。若插件带平滑，值会分几块才到位，
  不要用「第一块没变」判定失败。
- **保存**：工程保存/恢复依赖 `clap.state`；没实现就会丢参数。
- **崩溃代价**：AI DAW 的 CLAP 是**进程内加载**（VST3 只有离线渲染有 helper 隔离），
  插件崩了会带走整个应用。所以必须先跑 validator，再挂进工程。
- **音高表达**：CLAP 走通道弯音（MIDI 1.0），不是 VST3 那种逐音 note expression。
- **自建插件的描述**：新插件默认会走「未收录具体型号」的诚实兜底（`src-tauri/src/plugin_info.rs`
  的 `KNOWN` 表按名字片段匹配）。功能和参数不受影响；想让 `list_plugins` 里出现专门的中文音色
  描述，可以顺手把条目加进 `KNOWN`，但这不是插件能用的前提。

## 验证工具

```bash
# 安装（Rust 工具链）
cargo install --git https://github.com/free-audio/clap-validator

# 全量测试，只看失败
clap-validator validate ~/Library/Audio/Plug-Ins/CLAP/aidaw_gain.clap --only-failed

# 随机参数 / 音符 / 传输压力测试（实验性，但很能抓崩溃）
clap-validator fuzz -j4 -d10m30s ~/Library/Audio/Plug-Ins/CLAP/aidaw_gain.clap
```

跑完先按失败项区分「自己的代码」和「框架已知问题」：

- **nih-plug 的 state 扩展有已知短板**（2026-10 用官方 gain 示例核对，v0.4.1）：
  - `state-invalid-random` 会 **SIGABRT**：它把流开头的 8 字节长度当真，`Vec::with_capacity`
    直接按随机数分配。
  - `state-reproducibility-basic/binary/buffered` 失败：`load()` 之后没有向宿主
    `request_rescan(CLAP_PARAM_RESCAN_VALUES)`，宿主缓存的参数值仍是旧的。
  - 这几项**不是你的 process / params 写错**，官方示例同样中招。
- 其余 init / activate / start_processing / process / deactivate / destroy、参数查询、
  端口布局、随机块大小/采样率等应当全绿。`process` / `params` 类测试失败才是要改自己代码的信号。
- macOS 上 validator 的官方构建未签名，必要时 `xattr -dr com.apple.quarantine`。

### 修掉 nih-plug 的 state 问题（已验证 34/34）

**结论：上面 4 项是 nih-plug 的实现问题，不是插件代码。** 改动都在
`src/wrapper/clap/wrapper.rs`，两处。方便的话顺手给上游提 issue/PR——官方仓库在维护模式，
合并可能慢，所以本地 patch 仍然值得留着。

1. `ext_state_load()` 直接信任流里的 8 字节长度，随机字节会触发巨大分配。加长度上限 +
   `try_reserve_exact`，失败就返回 `false`：
2. `set_state_inner()` 恢复参数后只更新了 GUI，没有让宿主刷新参数缓存。补一个
   `Task::RescanParamValues`：

```rust
// 1) ext_state_load()：把原来的 Vec::with_capacity(length) + spare_capacity_mut()
//    替换成下面这段（关键是不信任 length、分配失败不 abort）。
let length = u64::from_le_bytes(length_bytes);

const MAX_STATE_SIZE: u64 = 512 * 1024 * 1024;
if length > MAX_STATE_SIZE {
    nih_debug_assert_failure!("State is too large: {} bytes", length);
    return false;
}
let mut read_buffer: Vec<u8> = Vec::new();
if read_buffer.try_reserve_exact(length as usize).is_err() {
    nih_debug_assert_failure!("Could not allocate {} bytes for state", length);
    return false;
}
read_buffer.resize(length as usize, 0);
if !read_stream(&*stream, read_buffer.as_mut_slice()) {
    nih_debug_assert_failure!(
        "Error or end of stream while reading the state buffer from the stream."
    );
    return false;
}
```

```rust
// 2) set_state_inner()：在现有的 ParameterValuesChanged 之后补两行。
let task_posted = self.schedule_gui(Task::ParameterValuesChanged);
nih_debug_assert!(task_posted, "The task queue is full, dropping task...");

let task_posted = self.schedule_gui(Task::RescanParamValues);
nih_debug_assert!(task_posted, "The task queue is full, dropping task...");
```

**应用方式（二选一）**

- **Fork**：fork `robbert-vdh/nih-plug`，改这两处，把 `Cargo.toml` 的 `git = ...` 指向你的
  fork 并钉一个 `rev`（`nih_plug` 与 `nih_plug_xtask` 都指过去）。
- **Vendor（已验证）**：把 nih-plug 源码复制到项目的 `vendor/nih-plug`，依赖改成 path。
  **必须把它排除出父 workspace**，否则 Cargo 会报 `multiple workspace roots found`：

```toml
# 根 Cargo.toml
[workspace]
members = ["xtask"]
exclude = ["vendor/nih-plug"]

[dependencies]
nih_plug = { path = "vendor/nih-plug", default-features = false, features = ["assert_process_allocs"] }

# xtask/Cargo.toml
[dependencies]
nih_plug_xtask = { path = "../vendor/nih-plug/nih_plug_xtask" }
```

> `512 MiB` 是给普通插件的上限；如果 state 里真的存了大型采样数据，按需调大，但别去掉上限。

**验证结果**（2026-10-22，macOS arm64，nih-plug `de421011`，clap-validator 0.4.1）：
不打补丁 `30 passed / 4 failed`；打完后 **`34 passed / 0 failed`**（另有 10 项 skipped），
`state-invalid-random` 与三个 `state-reproducibility-*` 都通过。

**不想打补丁的临时办法**：在项目根放 `clap-validator.toml` 排除这几项，并在 README 里
写明原因；同时仍然要在 AI DAW 里实测「保存 → 重开 → 参数恢复」：

```toml
[test]
state-invalid-random = false
state-reproducibility-basic = false
state-reproducibility-binary = false
state-reproducibility-buffered = false
```

**最小模板的实测结果**（2026-10-22，macOS arm64，nih-plug `de421011`）：按上面的
`Cargo.toml` + `lib.rs` 构建出 `target/bundled/aidaw_gain.clap`（Mach-O bundle，导出
`_clap_entry`）；AI DAW 宿主扫描为 `clap:com.aidaw.examples.gain`、2 进 2 出、1 个 Gain
参数（0..1）；`set_param` 经 CLAP 事件收敛到 -30 dB；state 77 字节往返正常。
validator：不打补丁 30 通过 / 4 失败，打上上面的补丁后 34 通过 / 0 失败。

## 常见故障对照

| 现象 | 先查什么 |
| --- | --- |
| `list_plugins` 里没有 | 装错目录；macOS 不是 bundle；架构不对（arm64 vs x86_64）；没导出 `clap_entry`；没 `rescan_plugins` |
| 能看到但没有参数 | 没暴露 `clap.params`；`count` 返回 0；`get_info` 没填 `name` / 范围；参数查询没在主线程 |
| 参数调了没反应 | `process` 没处理 `CLAP_EVENT_PARAM_VALUE`；`set_params` 之后没有重新渲染 |
| 乐器没声音 | 音频端口写成输入了；没有 `note-ports`；`process` 没写 `audio_outputs`；音符事件没按 `time` 处理 |
| 效果器没声音 | 只声明了输出没声明输入；输入缓冲 `data32` / `data64` 判空写错；返回了 `SLEEP` |
| 重开工程参数丢了 | 没实现 `clap.state`，或 `save/load` 没写参数 |
| validator 的 `state-*` 失败/崩溃 | 先用官方 nih-plug 示例对照：`state-invalid-random` 与 `state-reproducibility-*` 是 nih-plug 的已知问题（随机长度分配、load 后不 rescan），不是你的 DSP；打上面的两处补丁可修（已验证 34/34） |
| 一开插件界面就崩 | GUI 的 `destroy` 晚于父视图释放；`set_parent` 之后没在主线程做 GUI 调用 |
| 播放时随机崩 / 卡顿 | `process` 里分配内存、加锁、打日志；`activate` 里没预分配 |
| 宿主里音高/自动化不对 | 参数 id 不稳定；`CLAP_ID` 改过；`flush` 与 `process` 并发 |

## 交付前检查清单

- [ ] `clap-validator` 跑过；用 nih-plug 的话打上 state 补丁后应为 0 失败，或明确排除并记录原因
- [ ] `CLAP_ID` 是稳定的反向域名，版本号写进 descriptor
- [ ] 效果器 2 进 2 出、乐器 2 出 + 1 个 note input
- [ ] 所有参数有稳定 id、有限范围、合理默认值、可读的 `value_to_text`
- [ ] `state` 能完整往返，工程保存/重开参数不丢
- [ ] 44.1 / 48 / 96 kHz，64–2048 块大小都试过；`reset` 后没有残留
- [ ] `process` 实时安全；`activate/deactivate` 可重复调用
- [ ] macOS 产物是 bundle；分发给别人时按目标架构构建并 ad-hoc 签名
- [ ] README 写清构建、安装路径、参数含义、已知限制
- [ ] 在 AI DAW 里用**新工程**完整走一遍验收闭环

## 来源

- CLAP 官方仓库与规范：<https://github.com/free-audio/clap>（MIT）
- 入口与扩展：<https://github.com/free-audio/clap/blob/main/include/clap/entry.h>
- 最小 C 模板：<https://github.com/free-audio/clap/blob/main/src/plugin-template.c>
- 宿主 / 插件列表：<https://cleveraudio.org/hosts-and-plug-ins/>、<https://clapdb.tech>
- clap-validator：<https://github.com/free-audio/clap-validator>
- nih-plug：<https://github.com/robbert-vdh/nih-plug>（ISC；VST3 绑定 GPLv3）
- nih-plug 模板：<https://github.com/robbert-vdh/nih-plug-template>
- C++ helpers / wrapper：<https://github.com/free-audio/clap-helpers>、<https://github.com/free-audio/clap-wrapper>
- 打包与安装路径参考：<https://truce.audio/docs/formats/clap/>

> 生态与版本信息核对于 **2026-10-21**。宿主支持会变，交付前建议再看一眼 clapdb 的最新条目。
