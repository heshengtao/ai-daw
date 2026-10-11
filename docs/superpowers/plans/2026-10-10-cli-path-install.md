# `aidaw` 上 PATH（F4）— 实施计划

- 日期：2026-10-10
- 状态：✅ **完成**（macOS 已真机验证；Windows / Linux 待对应平台验证）
- 依据：`specs/2026-10-10-cli-path-install-design.md`；`ROADMAP.md` §5 **Stage F · F4**。

## 1 任务

1. ✅ **打包链**：`tools/build-aidaw.sh`（编译 `crates/aidaw` → `src-tauri/binaries/aidaw-<triple>`）、
   `package.json` 的 `build:aidaw`、`tauri.conf.json` 的 `bundle.externalBin` 与
   `beforeBuildCommand`、`src-tauri/.gitignore` 忽略 `/binaries/`。
   - `aidaw` 在**根** workspace 而 `src-tauri` 是独立 workspace，脚本必须在仓库根 `cargo build -p aidaw`。
2. ✅ **`src-tauri/src/cli.rs`**：`CliStatus`/`CliState`/`CliHint`（**无状态**——开关是显式的，
   真相在文件系统 / 注册表，没有 `CliRuntime`、也没有 `cli.json`）；`probe()` / `classify()`
   （纯函数，可单测）+ `status()` / `install()` / `uninstall()`；
   macOS 提权软链（`osascript`）；Windows 用户 PATH（`winreg`，保留 `REG_EXPAND_SZ`）；
   Linux `system` / `needs_manual` / AppImage 分支。
3. ✅ **接线**：`lib.rs` 的 `mod cli` + `invoke_handler` 注册 `cli_status`、`install_cli`、`uninstall_cli`。
   **`setup` 里没有任何 CLI 相关代码**（不做自动挂载）。
4. ✅ **前端**：`SettingsView.vue` 新增「命令行」一节——**一个开关**（开 = 挂载 / 关 = 卸载）
   + 状态说明 + 命令位置 + 未挂载时的**全路径**（可复制）+ 手工命令；
   `api/tauri.ts` + `api/mock.ts` + `api/types.ts`；i18n zh / en 各一组 `settings.cli.*`。
5. ✅ **发现文件 `cli-path`**：启动时把包内 `aidaw` 的**绝对路径**写进应用配置目录（与 `mcp-url` 并列），
   供外部 AI harness 按 bundle id 推导后直接读取——不用搜索、不依赖 PATH、也不需要用户开挂载开关。
   内核 `write_path_file(dir, bundled)` 是纯函数（2 条单测）。
   顺带修了构建链：`bundle.externalBin` 会让 `tauri dev` 在缺 sidecar 时**直接报错**
   （`resource path binaries/aidaw-… doesn't exist`，实测），所以 `beforeDevCommand` 也串上
   `build:aidaw`，全新 clone 也能一把跑起来。
6. ✅ **文档**：本 spec + plan；`ROADMAP.md` F4 状态与文档索引。

## 2 决策落地（与 spec §3 对应）

- 提权不用插件：`std::process::Command::new("/usr/bin/osascript")`，**零 capability 变更**。
- **没有任何自动行为**：挂载只由设置页的开关触发，启动时不碰 PATH、不弹密码框
  （v1 曾在启动时自动挂载 1 秒后弹授权，体验太差被否）。取消授权 → 开关回到关，**不持久化**任何
  「拒绝过」状态——那种记忆容易和现实不一致。
- 开发构建**显式排除**：`looks_like_dev_build()` —— 先认 `.app/Contents/MacOS/`（本机 `tauri build`
  的产物也落在 `target/release/bundle/…` 里，**不先认包就会把本地打的包误判成开发构建**），
  再回退到「路径里最后一个 `target` 之后有没有 `debug` / `release`」。
  （不能只靠「旁边有没有 sidecar」——Tauri 也会把 sidecar 复制进 `target/debug` 与 `target/release`，
  实测确认；少了这条判断，`pnpm tauri dev` 会弹密码框。）

## 3 验证

**本机已完成（macOS）**

| 项 | 结果 |
| --- | --- |
| `bash tools/build-aidaw.sh` | ✅ 产出 `src-tauri/binaries/aidaw-aarch64-apple-darwin`（可执行） |
| `cargo test --lib cli::tests` | ✅ 14 通过（转义 3 + 状态分类 7 + 开发构建判定 2 + 发现文件 2） |
| `npx tauri build --bundles app` | ✅ 打包成功；`AI DAW.app/Contents/MacOS/aidaw` 就位、可执行（`aidaw 0.1.0`）、已签名 |
| `npx vue-tsc --noEmit` | ✅ 无类型错误 |
| `cargo check` / `cargo clippy`（src-tauri） | ✅ `cli.rs` 无告警（其余告警为既有） |
| 发现文件（真机） | ✅ App 启动后写出 `…/com.aidaw.app/cli-path`，内容即包内 `aidaw` 绝对路径，可直接执行 |
| `tauri dev` 缺 sidecar | ✅ 修前报 `resource path binaries/aidaw-aarch64-apple-darwin doesn't exist`；`beforeDevCommand` 串上 `build:aidaw` 后自动补齐并正常启动 |
| `build-aidaw.sh` 在 UTF-8 locale 下 | ✅ 修前在 `LC_ALL=zh_CN.UTF-8`（用户环境）报 `TRIPLE?: unbound variable`：bash 3.2 把 `"$TRIPLE）"` 里的全角括号**吃进了变量名**。已全部改成 `${VAR}` 并在脚本里写了原因；`zh_CN.UTF-8` / `en_US.UTF-8` / `C` 三种 locale 均通过，`tauri dev` 端到端跑通 |

**待对应平台完成**

| 项 | 说明 |
| --- | --- |
| Windows | `pnpm tauri build` → 打开开关 → 新开终端 `aidaw status`；关掉后 PATH 干净 |
| Linux `.deb` / `.rpm` | 装完 `/usr/bin/aidaw` 即在 PATH 上，状态应为 `system` |
| macOS 端到端（**需要人工点开关 + 授权**） | 打开打包好的 `.app` → **启动不应弹任何东西** → 设置 → 命令行开开关 → 终端 `aidaw` 可用 → 关开关后软链消失 |
| macOS 开发构建 | `pnpm tauri dev` 状态应为 `unsupported`（`dev_build`）、开关禁用 |
| 全路径回退 | 不开开关时，用设置页给的 `…/AI DAW.app/Contents/MacOS/aidaw` 能直接调 |

> `cfg(windows)` 分支在 macOS 上**不参与编译**，仓库又没有 CI，所以 Windows / Linux 的代码
> 只能算「逻辑就绪」。这是本次改动最大的不确定项。
