# `aidaw` 上 PATH（F4）— 设计

> **状态（2026-10-10）**：**已完成**（macOS 已真机验证；Windows / Linux 分支逻辑就绪，待对应平台验证）。
> 实现：`src-tauri/src/cli.rs`、`tools/build-aidaw.sh`、设置页「命令行」一节。
> 逐任务记录见 `plans/2026-10-10-cli-path-install.md`。

## 1 背景

Roadmap Stage F 的 **F1 / F2** 已经给了 `aidaw`（MCP 薄客户端）与发现文件 `mcp-url`，
但**只有开发者**能通过 `cargo install --path crates/aidaw` 用上它——那个动作只写进**当前用户**的
`~/.cargo/bin`，跟安装包无关。普通用户装完 App，终端里敲 `aidaw` 是 `command not found`。

F4 要解决的就是这件事：**装完就能随处敲 `aidaw`**，像 Ollama 那样，不需要额外步骤、不需要手动配 PATH。

## 2 目标与非目标

**目标**

- 用户**想要的话**，可以在设置页打开一个开关，把 `aidaw` 挂到 PATH 上，之后在任意终端敲 `aidaw`；
  挂载指向包内二进制，App 升级时命令自动跟着更新。
- **不做任何自动动作**：启动不碰 PATH、不弹密码框。挂载是用户的**显式选择**——点开关的人知道
  「这次输密码是为了挂命令行」，因果关系清楚。
- **不挂也能用**：用包内二进制的**全路径**调用（命令长一点，功能完全一样）。AI 侧拿不到短命令时
  就用全路径，不许因为「没挂载」就说做不到。
- 设置页能看到状态、开关、冲突提示，以及未挂载时要用的全路径。

**非目标（v1 明确不做）**

- **不做自更新**：软链与目录方式天然跟随 App 升级，不需要额外机制。
- **不做退出清理**：当前 `lib.rs` 用 `.run()` 而不是 `.build().run(|_, RunEvent::Exit| …)`，
  不为它改退出流程；卸载交给设置页的卸载按钮 / 平台安装器。
- **AppImage 不做自动安装**（见 §5）。
- 不引入 `tauri-plugin-shell`、不加 capabilities 条目（见 §3）。

## 3 关键决策

| 决策 | 结论 | 理由 |
| --- | --- | --- |
| 二进制怎么进安装包 | **Tauri sidecar**：`bundle.externalBin: ["binaries/aidaw"]` | 官方机制；打包时把带三元组后缀的文件放到**主程序旁边**（macOS 即 `Contents/MacOS/`） |
| 怎么进 PATH | **软链 / PATH 条目指向包内文件**（不是复制） | App 升级后命令自动指向新二进制，**零版本漂移** |
| 判定「装没装」 | 一律按**固定路径 / 注册表**，**不读 PATH** | 从 Finder 启动的 GUI 进程只有最小 PATH，看不到用户 shell 的 PATH——这正是 Ollama「反复弹窗」bug 的根源 |
| macOS 提权 | `osascript -e 'do shell script "…" with administrator privileges'` | 与 Ollama 同款；走 Authorization Services，Hardened Runtime / 公证后仍可用 |
| 是否需要 `tauri-plugin-shell` | **不需要** | 提权只是 Rust 侧 `std::process::Command`；app 自定义命令不受 capability 系统管辖（`mcp_status` 即先例） |
| 挂载入口 | **设置页里的一个显式开关**；启动时**不做任何动作** | 启动就弹密码框体验很差（第一版就是这么做的，被否）；让用户点开关时才输密码，因果关系才清楚 |
| 配置持久化 | **不需要**：没有自动行为，真相全在文件系统 / 注册表 | 少一份 `cli.json`、也少一类「记忆与现实不一致」的 bug |
| 冲突处理 | 已有同名命令**绝不静默覆盖**；开关下面的说明写清「打开会覆盖它」 | 本机就踩得到：开发者可能 `cargo install` 到 `~/.cargo/bin` |

## 4 机制（每平台做什么）

| 平台 | 二进制落点 | 进 PATH 的做法 | 提权 |
| --- | --- | --- | --- |
| macOS | `AI DAW.app/Contents/MacOS/aidaw` | 软链 `/usr/local/bin/aidaw` → 包内二进制 | **要**（一次授权） |
| Windows | `<安装目录>\aidaw.exe` | 安装目录追加进**用户** PATH | 不要 |
| Linux `.deb` / `.rpm` | `/usr/bin/aidaw` | **本就在 PATH 上 → 什么都不做** | 不要 |
| Linux AppImage | 临时挂载点内 | 不支持自动安装（路径每次变） | — |

`/usr/local/bin` 在 Apple Silicon 上常常不存在，所以提权脚本先 `/bin/mkdir -p`。

### 让外部工具找到它：`cli-path` 发现文件

不挂 PATH 也能用，但**别的 AI harness 得知道 `aidaw` 在哪**。不靠「搜」，靠**推导**：

App 的 bundle id 是 `com.aidaw.app`，配置目录由它算出——macOS `~/Library/Application Support/com.aidaw.app/`、
Linux `~/.config/com.aidaw.app/`、Windows `%APPDATA%\com.aidaw.app\`。App **每次启动**都往里写：

- **`cli-path`** —— 包内 `aidaw` 的绝对路径（本次新增）；
- `mcp-url` —— MCP 端点（F2 已有）。

外部工具的发现顺序（已写进 `skills/aidaw/SKILL.md`）：

1. `command -v aidaw`（用户开了挂载开关）；
2. `<配置目录>/cli-path` ← **确定的，不用搜索**；
3. 都没有 → App 从没启动过：macOS `open -b com.aidaw.app` 起它，回到第 2 步；
4. 退回常见安装位置，或 macOS 上 `mdfind "kMDItemCFBundleIdentifier == 'com.aidaw.app'"`。

为什么不让 `mdfind` 当主路径：它依赖 Spotlight 索引、可能搜到多份、沙箱里可能查不到。
每次启动都重写，所以用户把 `.app` 拖到别处也不会失效；与挂载开关**无关**（不改 PATH、不要密码）。

## 5 状态机

`cli::CliState`：`installed` / `missing` / `conflict` / `system` / `unsupported`。

- `installed`：软链（macOS）或 PATH 条目（Windows）**正指着包内那份**。
- `missing`：没装，可装。
- `conflict`：位置上有东西，但**不是我们** → 报出它在哪，等用户显式点「覆盖安装」。
- `system`：Linux 系统包已经放好（`/usr/bin`），无需动作。
- `unsupported`：环境装不了。`CliHint` 给出原因（前端据此本地化）：
  - `dev_build`：**开发构建**。判据**按序**两条（`looks_like_dev_build`）：
    1. 路径含 `.app/Contents/MacOS/` → **是**真 App，**不是**开发构建。
       这条必须最先判——本机 `tauri build` 的产物落在 `target/release/bundle/macos/AI DAW.app/`，
       少了它会把本地打的包也当成开发构建，**永远装不上**。
    2. 否则看路径里**最后一个 `target` 之后**有没有 `debug` / `release` 段（cargo 产物目录）。
    为什么非要有这条判断：Tauri **也会**把 sidecar 复制进 `target/debug`、`target/release`，
    所以「主程序旁边有没有 `aidaw`」根本区分不出开发构建和已安装的 App。
    不判断的话，`pnpm tauri dev` 会平白弹一次密码框、还会把 `/usr/local/bin/aidaw` 指到临时产物上。
  - `appimage`：AppImage 的挂载路径每次不同；
  - `needs_manual`：Linux 上需要 root，给出可复制的 `sudo ln -sfn …` 命令。

**没有配置文件**：开关是显式的、也没有任何自动行为，状态直接按文件系统 / 注册表判断。
（任何「记忆」（比如「用户拒绝过」）都可能与现实不一致，反而多一类 bug；第一版那份 `cli.json` 已删。）

## 6 已知取舍与缺口

- **AppImage 不自动装**：Linux 的常态是 `.deb` / `.rpm`（那一档自动生效）；AppImage 要做得靠 pkexec
  或 wrapper 脚本，留后续。
- **Windows 未广播 `WM_SETTINGCHANGE`**：已开着的终端要**新开一个**才会拿到新 PATH。
  这是常见行为（很多安装器也如此），设置页文案已说明；要补的话加 `windows-sys` 广播即可。
- **卸载不随 App 卸载自动发生**（macOS 软链 / Windows PATH 条目）：靠设置页卸载按钮或平台安装器。
- **Windows / Linux 分支本机无法编译检查**（`cfg` 分支在 macOS 上不参与编译，仓库也没有 CI），
  必须在对应平台跑一次才算验证过（见 plan 的验证清单）。
- **同一台机器上同时有开发构建与安装包时**，`cli-path` 反映的是**最后启动的那一份**（每次启动都重写）。
  真用户机器上只有一份，不存在这个问题。
- 构建链：`bundle.externalBin` 让 `tauri dev` **也必须**先有 sidecar，否则构建脚本直接报
  `resource path binaries/aidaw-… doesn't exist`。所以 `beforeDevCommand` 里也串了 `build:aidaw`。

## 7 验收

1. **启动不弹任何东西**——这是本次最关键的约束（第一版会弹，被否）。
2. macOS：设置 → 命令行，打开开关 → 弹一次授权 → 终端 `aidaw status` 正常；关掉开关 → 软链消失。
3. 取消授权 → 开关回到关，界面给一行提示（**不**写任何「已拒绝」的记忆）。
4. 预先放一个别的 `/usr/local/bin/aidaw` → 状态 `conflict`，开关说明写清「打开会覆盖它」。
5. `pnpm tauri dev` → 开关禁用，状态 `unsupported`（`dev_build`）。
6. Windows：打开开关后新开终端 `aidaw` 可用；关掉后 PATH 干净。
7. Linux `.deb`：装完即 `/usr/bin/aidaw`，状态 `system`（开关呈开且禁用）。
8. 不开开关时，用设置页给出的**全路径**能正常调用。
