# Web 客户端 M1（daemon 抽取 + `/rpc`）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 AI DAW 的后端从 Tauri 外壳里解耦成可嵌入的 `crates/daemon`，并让一个无窗口进程能通过 HTTP `/rpc` 读写工程。

**Architecture:** 整体搬迁 `src-tauri/src` 到新 crate（路径不变，`crate::` 引用继续有效），新增 `Host`/`UiHost` trait 把"发事件、找数据目录、开原生窗口"抽象掉；Tauri 只作为其中一个实现。命令层保留 `#[tauri::command]` 薄壳指向 `*_impl(&Daemon, …)`，HTTP 分发走同一批 `*_impl`。M1 只迁移 project/track/clip/note/transport/mixer 六组命令，其余命令继续可用但仍是 Tauri 签名（过渡态）。

**Tech Stack:** Rust 2021、Tauri 2、axum 0.8、tokio、serde、`rtrb`（M2 才用）

**Spec:** `docs/superpowers/specs/2026-11-13-web-client-design.md`

## Global Constraints

- Rust edition 2021；crate 版本跟随 workspace（`version.workspace = true`）。
- `crates/daemon` 加入根 workspace；`src-tauri` 是**独立 workspace**，只通过 path 依赖引用 daemon。
- 桌面版每一个 task 结束时都必须还能 `cd src-tauri && cargo build`。
- 不允许改前端 `src/`（M1 纯后端）；`invoke` 的命令名与参数名保持不变。
- 迁移期间 `crates/daemon` 允许依赖 `tauri`（为了 `#[tauri::command]` 宏与 `State` 类型），但 `ai-daw` 运行时**不得初始化**任何 Tauri App。
- 新增代码注释用中文，与现有风格一致；每个 `pub` 项都要有 doc 注释。
- 命令实现函数统一命名 `<name>_impl`，放在原文件里，Tauri 薄壳紧挨着它。

## 文件结构

| 路径 | 职责 |
| --- | --- |
| `crates/daemon/Cargo.toml` | 新 crate 的依赖清单（从 `src-tauri/Cargo.toml` 抄，去掉插件） |
| `crates/daemon/src/host.rs` | `Host` / `UiHost` trait、`emit()` 自由函数、`FakeHost`（测试） |
| `crates/daemon/src/daemon.rs` | `Daemon` 容器：`Arc<AppState>` + `Arc<dyn Host>` + `Config` |
| `crates/daemon/src/server/mod.rs` | axum `Router`、鉴权、`/health` |
| `crates/daemon/src/server/rpc.rs` | `dispatch()`：命令名 → `*_impl` 的 match 表 |
| `crates/daemon/src/boot.rs` | `bootstrap(host, cfg) -> Arc<Daemon>`：会话、预设、插件目录、管理器 |
| `crates/daemon/src/bin/ai-daw.rs` | 无窗口入口：解析参数、建 `HeadlessHost`、起服务 |
| `src-tauri/src/tauri_host.rs` | `TauriHost`：`Host` + `UiHost` 的 Tauri 实现 |
| `src-tauri/src/lib.rs` | 只保留 `run()`、建窗、窗口事件、`TauriHost` 装配 |

---

### Task 1: 建立 `crates/daemon` 并整体迁入后端源码

**Files:**
- Create: `crates/daemon/Cargo.toml`
- Move: `src-tauri/src/*.rs`、`src-tauri/src/agent/` → `crates/daemon/src/`（`lib.rs`、`main.rs` 除外）
- Modify: `Cargo.toml`（根，workspace members）
- Modify: `src-tauri/Cargo.toml`（加 path 依赖）
- Modify: `src-tauri/src/lib.rs`（`crate::X::` → `ai_daw_daemon::X::`）

**Interfaces:**
- Produces: crate `ai_daw_daemon`（lib 名 `ai_daw_daemon`），导出 `pub mod commands; pub mod state; pub mod paths; …`（与现在 `lib.rs` 里的 `mod` 列表一致，全部改成 `pub mod`）。

- [ ] **Step 1: 搬目录**

```bash
cd /Users/ailm32442/Documents/github/ai-daw
mkdir -p crates/daemon
git mv src-tauri/src crates/daemon/src
mkdir -p src-tauri/src
git mv crates/daemon/src/lib.rs src-tauri/src/lib.rs
git mv crates/daemon/src/main.rs src-tauri/src/main.rs
ls crates/daemon/src | head
```

期望：`crates/daemon/src` 里有 `commands.rs`、`state.rs`、`paths.rs`、`mcp.rs`、`agent/` 等，**没有** `lib.rs`/`main.rs`。

- [ ] **Step 2: 写 `crates/daemon/Cargo.toml`**

内容从 `src-tauri/Cargo.toml` 抄 `[dependencies]`，去掉 `tauri-plugin-dialog`、`tauri-plugin-updater`、`tauri-plugin-process` 与 `[build-dependencies]`，加上 `[lib]`：

```toml
[package]
name = "ai-daw-daemon"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "AI DAW 的后端：工程模型、音频引擎、插件宿主、MCP 与 HTTP 服务"

[lib]
name = "ai_daw_daemon"
path = "src/lib.rs"

[dependencies]
# 过渡期保留：命令层还要用 `#[tauri::command]` 宏与 `State` 类型；`ai-daw`
# 运行时不初始化 App（见 host.rs 的说明）。
tauri = { version = "2", features = [] }
chrono = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
ai-daw-engine = { path = "../engine", features = ["realtime"] }
ai-daw-plugin-api = { path = "../plugin-api" }
ai-daw-host-vst3 = { path = "../host-vst3" }
ai-daw-analysis = { path = "../analysis" }
ai-daw-asset = { path = "../asset" }
ai-daw-local-ai = { path = "../local-ai" }
ai-daw-mastering = { path = "../mastering" }
rmcp = { version = "3", features = ["server", "macros", "schemars", "transport-streamable-http-server"] }
axum = "0.8"
tokio = { version = "1", features = ["net"] }
tokio-util = "0.7"
schemars = "1"
midir = "0.11"
reqwest = { version = "0.13", default-features = false, features = ["json", "stream", "native-tls"] }
futures-util = { version = "0.3", default-features = false }
ureq = { version = "3", default-features = false, features = ["native-tls", "gzip", "socks-proxy"] }
sha2 = "0.10"
zip = { version = "2", default-features = false, features = ["deflate"] }
tar = "0.4"
flate2 = "1"
xz2 = "0.1"
base64 = "0.22"

[target.'cfg(windows)'.dependencies]
winreg = "0.56"

[dev-dependencies]
tempfile = "3"
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

- [ ] **Step 3: 根 `Cargo.toml` 加成员**

```toml
members = ["crates/ipc", "crates/plugin-api", "crates/synth", "crates/instruments", "crates/effects", "crates/engine", "crates/asset", "crates/host-vst3", "crates/analysis", "crates/local-ai", "crates/mastering", "crates/aidaw", "crates/vst3-helper", "crates/daemon"]
```

- [ ] **Step 4: `src-tauri/Cargo.toml` 加依赖，去掉已迁走的**

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-dialog = "2"
tauri-plugin-updater = "2"
tauri-plugin-process = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ai-daw-daemon = { path = "../crates/daemon" }
ai-daw-host-vst3 = { path = "../crates/host-vst3" }
ai-daw-local-ai = { path = "../crates/local-ai" }
chrono = "0.4"
```

其余依赖（engine、asset、analysis、mastering、rmcp、axum、ureq、zip…）全部删掉 —— 它们现在归 daemon。**编译报错会逐条告诉你漏了哪个**。

- [ ] **Step 5: 在 `crates/daemon/src/lib.rs` 建模块表**

```rust
#![forbid(unsafe_code)]

//! AI DAW 的后端：工程模型、音频引擎、插件宿主、本地 AI、MCP 与 HTTP 服务。
//!
//! 这个 crate **不拥有窗口**：需要原生能力（对话框、插件编辑器窗口、主线程泵）
//! 时通过 [`host::Host`] 拿，桌面端与无窗口进程各给一份实现。

pub mod abc;
pub mod agent;
pub mod ai;
pub mod articulation_map;
pub mod assets;
pub mod audio;
pub mod backends;
pub mod capabilities;
pub mod cli;
pub mod commands;
pub mod cue_sheet;
pub mod factory_presets;
pub mod history;
pub mod job;
pub mod logging;
pub mod main_thread;
pub mod manager;
pub mod mcp;
pub mod midi_import;
pub mod midi_input;
pub mod models;
pub mod paths;
pub mod pick;
pub mod plugin_dirs;
pub mod plugin_info;
pub mod plugins;
pub mod presets;
pub mod quarantine;
pub mod runtimes;
pub mod session;
pub mod singing;
pub mod sound_memories;
pub mod state;
pub mod svc;
pub mod voices;
pub mod yue;
pub mod yue_engine;
pub mod yue_lyrics;
pub mod yue_midi;
pub mod yue_transcribe;
```

（`host` 与 `daemon` 两个模块分别在 Task 2、Task 3 加进来；这一任务先不加。）

- [ ] **Step 6: 改 `src-tauri/src/lib.rs` 的引用**

```bash
cd /Users/ailm32442/Documents/github/ai-daw
# 1) `crate::xxx::` 形式的引用
perl -pi -e 's/\bcrate::(commands|state|paths|session|logging|manager|mcp|agent|ai|audio|assets|backends|capabilities|cli|cue_sheet|factory_presets|history|job|main_thread|midi_import|midi_input|models|pick|plugin_dirs|plugin_info|plugins|presets|quarantine|runtimes|singing|sound_memories|svc|voices|yue\w*|articulation_map|host)::/ai_daw_daemon::$1::/g' src-tauri/src/lib.rs
# 2) `generate_handler!` 里的裸模块名（`commands::app_ready` 这种，前面没有 crate::）
perl -pi -e 's/^(\s+)(commands|paths|ai|models|manager|runtimes|cli|mcp|plugins|presets|sound_memories|voices|singing|svc|yue|yue_engine|yue_lyrics|yue_midi|yue_transcribe|backends|capabilities|midi_input|session|job|plugin_dirs|plugin_info|cue_sheet|articulation_map|quarantine)::/$1ai_daw_daemon::$2::/' src-tauri/src/lib.rs
grep -c "ai_daw_daemon::" src-tauri/src/lib.rs
git diff --stat src-tauri/src/lib.rs
```

- [ ] **Step 7: 逐条修编译错误**

```bash
cargo build -p ai-daw-daemon 2>&1 | head -40
cd src-tauri && cargo build 2>&1 | head -40
```

常见剩余错误：`pub(crate)` 类型跨 crate 不可见 → 改成 `pub`；`crate::` 在 shell 里指向 shell 自己要保留的符号。

- [ ] **Step 8: 提交**

```bash
git add -A
git commit -m "refactor(daemon): 后端源码整体迁入 crates/daemon，桌面壳只留窗口"
```

---

### Task 2: `Host` / `UiHost` trait 与两份实现

**Files:**
- Create: `crates/daemon/src/host.rs`
- Create: `src-tauri/src/tauri_host.rs`
- Modify: `src-tauri/src/lib.rs`（`mod tauri_host;`）

**Interfaces:**
- Produces:
  - `ai_daw_daemon::host::{Host, UiHost, emit, PickRequest, FakeHost}`
  - `fn emit<S: Serialize + ?Sized>(host: &dyn Host, event: &'static str, payload: &S)`
  - `trait Host { fn emit_value(&self, event: &'static str, payload: Value); fn data_dir(&self) -> PathBuf; fn ui(&self) -> Option<&dyn UiHost>; }`
  - `tauri_host::TauriHost::new(app: tauri::AppHandle) -> TauriHost`

- [ ] **Step 1: 写失败测试**（`crates/daemon/src/host.rs` 末尾）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct Payload {
        value: u32,
    }

    #[test]
    fn fake_host_records_events_with_json_payload() {
        let host = FakeHost::default();
        emit(&host, "engine://event", &Payload { value: 7 });
        let events = host.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "engine://event");
        assert_eq!(events[0].1["value"], 7);
    }

    #[test]
    fn fake_host_has_data_dir_and_no_ui() {
        let host = FakeHost::default();
        assert!(host.data_dir().ends_with("fake-data"));
        assert!(host.ui().is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib host::tests 2>&1 | tail -5
```

期望：编译失败，`cannot find type FakeHost` / `emit`。

- [ ] **Step 3: 实现 `host.rs`**

```rust
//! 宿主抽象：后端需要的一切"进程外部"能力都从这里拿。
//!
//! 桌面端提供原生对话框与插件编辑器窗口；无窗口进程（`ai-daw`）把这些能力
//! 明确降级为 `None`——**不静默失败**，调用方据此给出「此操作需要桌面端」的提示。

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

/// 一次"选文件"请求。桌面端据此弹原生对话框，无窗口进程直接拒绝。
#[derive(Debug, Clone)]
pub struct PickRequest {
    /// 判断用的语义（`project` / `audio` / `midi` / `export`），不是给用户看的文案。
    pub kind: &'static str,
    /// 允许的扩展名；空表示不限。
    pub extensions: Vec<&'static str>,
}

/// 原生能力的抽象。**只有桌面端能实现成有意义的行为**。
pub trait UiHost: Send + Sync {
    /// 弹系统对话框选一个文件；用户取消返回 `Ok(None)`。
    fn pick_file(&self, request: PickRequest) -> Result<Option<PathBuf>, String>;
    /// 打开某个插件的原生编辑器窗口。
    fn open_plugin_editor(&self, track: u64, slot: i32) -> Result<(), String>;
    /// 把一段活派到 UI 主线程（CLAP 的 init / 参数枚举必须在主线程）。
    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>);
}

/// 后端与外壳之间唯一的契约。
pub trait Host: Send + Sync + 'static {
    /// 广播一个事件。桌面端发给 webview，无窗口进程发给 SSE（M3）。
    fn emit_value(&self, event: &'static str, payload: Value);
    /// 应用数据目录（会话缓存、插件缓存、token 都落在这里）。
    fn data_dir(&self) -> PathBuf;
    /// 没有原生能力时返回 `None`。
    fn ui(&self) -> Option<&dyn UiHost>;
}

/// 便捷包装：序列化失败只记日志，绝不 panic —— 事件广播失败不该让一次编辑失败。
pub fn emit<S: Serialize + ?Sized>(host: &dyn Host, event: &'static str, payload: &S) {
    match serde_json::to_value(payload) {
        Ok(value) => host.emit_value(event, value),
        Err(err) => eprintln!("[host] 序列化事件 {event} 失败：{err}"),
    }
}

/// 测试用宿主：只记事件，不碰磁盘、不开窗口。
#[derive(Default)]
pub struct FakeHost {
    pub events: std::sync::Mutex<Vec<(String, Value)>>,
    pub dir: PathBuf,
}

impl FakeHost {
    pub fn with_dir(dir: impl Into<PathBuf>) -> Self {
        Self { events: Default::default(), dir: dir.into() }
    }
}

impl Host for FakeHost {
    fn emit_value(&self, event: &'static str, payload: Value) {
        self.events.lock().unwrap().push((event.to_string(), payload));
    }

    fn data_dir(&self) -> PathBuf {
        if self.dir.as_os_str().is_empty() {
            PathBuf::from("fake-data")
        } else {
            self.dir.clone()
        }
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        None
    }
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib host::tests 2>&1 | tail -5
```

期望：2 passed。

- [ ] **Step 5: 写 `src-tauri/src/tauri_host.rs`**

```rust
//! `Host` / `UiHost` 的桌面实现：事件转给 webview，原生能力走 Tauri。

use std::path::PathBuf;

use ai_daw_daemon::host::{Host, PickRequest, UiHost};
use serde_json::Value;
use tauri::{Emitter, Manager};

#[derive(Clone)]
pub struct TauriHost {
    app: tauri::AppHandle,
}

impl TauriHost {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl Host for TauriHost {
    fn emit_value(&self, event: &'static str, payload: Value) {
        // 建窗 / 关窗那一瞬间 webview 可能还没准备好；广播失败只影响这一次推送。
        let _ = self.app.emit(event, payload);
    }

    fn data_dir(&self) -> PathBuf {
        self.app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        Some(self)
    }
}

impl UiHost for TauriHost {
    fn pick_file(&self, _request: PickRequest) -> Result<Option<PathBuf>, String> {
        // 桌面端的文件选择走前端 `@tauri-apps/plugin-dialog`（见 src/api/dialogs.ts），
        // 后端这里不需要弹窗：命令拿到的是前端选好的路径。
        Ok(None)
    }

    fn open_plugin_editor(&self, _track: u64, _slot: i32) -> Result<(), String> {
        Err("插件编辑器窗口由前端命令 plugin_window_open 打开".into())
    }

    fn run_on_main_thread(&self, task: Box<dyn FnOnce() + Send>) {
        let _ = self.app.run_on_main_thread(task);
    }
}
```

- [ ] **Step 6: 在 shell 里注册模块并编译**

`src-tauri/src/lib.rs` 顶部加：

```rust
mod tauri_host;
```

```bash
cd src-tauri && cargo build 2>&1 | tail -20
```

期望：编译通过（`allow(dead_code)` 可以先加在 `tauri_host` 模块上，Task 9 用掉后删）。

- [ ] **Step 7: 提交**

```bash
git add -A && git commit -m "feat(daemon): Host/UiHost 抽象与 Tauri 实现"
```

---

### Task 3: `Daemon` 容器 + 编辑入口改走 `Host`

**Files:**
- Create: `crates/daemon/src/daemon.rs`
- Modify: `crates/daemon/src/lib.rs`（`pub mod daemon;`）
- Modify: `crates/daemon/src/commands.rs:56-130`（`run_as` / `run_all` / `close_stale_plugin_windows`）

**Interfaces:**
- Produces:
  - `pub struct Daemon { state: Arc<AppState>, host: Arc<dyn Host>, cfg: Config }`
  - `Daemon::new(state: Arc<AppState>, host: Arc<dyn Host>, cfg: Config) -> Arc<Daemon>`
  - `Daemon::state(&self) -> &AppState`
  - `Daemon::host(&self) -> &Arc<dyn Host>`
  - `Commands::run_all(d: &Daemon, actor: Actor, commands: Vec<Command>) -> (Project, Vec<Event>)`
- Consumes: Task 2 的 `Host`、`emit`

- [ ] **Step 1: 写失败测试**

`crates/daemon/src/daemon.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::FakeHost;

    #[test]
    fn daemon_shares_state_and_host() {
        let state = Arc::new(AppState::bootstrap(None, Vec::new()));
        let host = Arc::new(FakeHost::default());
        let daemon = Daemon::new(Arc::clone(&state), host, Config::default());
        assert!(Arc::ptr_eq(daemon.state_arc(), &state));
        assert!(daemon.host().ui().is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib daemon::tests 2>&1 | tail -5
```

期望：`cannot find type Config`。

- [ ] **Step 3: 实现 `daemon.rs`**

```rust
//! 后端容器：一份共享状态 + 一个宿主实现 + 一份配置。
//!
//! 桌面端与 `ai-daw` 都构造它，命令实现只认它——不认 Tauri、也不认 axum。

use std::path::PathBuf;
use std::sync::Arc;

use crate::host::Host;
use crate::state::AppState;

/// 服务配置。M1 只用得到前两项，其余在 M3/M4 落地。
#[derive(Debug, Clone)]
pub struct Config {
    /// 应用数据目录（会话、插件缓存、token）。
    pub data_dir: PathBuf,
    /// 是否绑到 0.0.0.0（局域网）。默认只监听回环。
    pub lan: bool,
    /// HTTP 端口；`None` = 不启动服务（纯桌面模式）。
    pub port: Option<u16>,
}

impl Default for Config {
    fn default() -> Self {
        Self { data_dir: PathBuf::from("."), lan: false, port: None }
    }
}

/// 后端实例。**唯一**持有 `AppState` 的地方。
pub struct Daemon {
    state: Arc<AppState>,
    host: Arc<dyn Host>,
    cfg: Config,
}

impl Daemon {
    pub fn new(state: Arc<AppState>, host: Arc<dyn Host>, cfg: Config) -> Arc<Self> {
        Arc::new(Self { state, host, cfg })
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn state_arc(&self) -> &Arc<AppState> {
        &self.state
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }
}
```

- [ ] **Step 4: 把 `run_all` 改成吃 `&Daemon`**

`crates/daemon/src/commands.rs` 现在：

```rust
fn run_as(app: &tauri::AppHandle, state: &State<'_, AppState>, actor: Actor, command: Command) -> (Project, Vec<Event>)
fn run_all(app: &tauri::AppHandle, state: &State<'_, AppState>, actor: Actor, commands: Vec<Command>) -> (Project, Vec<Event>)
```

改成：

```rust
fn run_as(d: &Daemon, actor: Actor, command: Command) -> (Project, Vec<Event>) {
    run_all(d, actor, vec![command])
}

fn run_all(d: &Daemon, actor: Actor, commands: Vec<Command>) -> (Project, Vec<Event>) {
    let state = d.state();
    let (mut project, events) = {
        let mut engine = state.engine.lock().unwrap();
        let events: Vec<Event> = engine.execute_all(commands, actor);
        (engine.project().clone(), events)
    };
    for event in &events {
        crate::host::emit(&**d.host(), EVENT_ENGINE, event);
    }
    let structural = events.iter().any(is_structural);
    if structural {
        capture_plugin_states(state);
        project = state.engine.lock().unwrap().project().clone();
        close_stale_plugin_windows(d);
    }
    state.audio.sync_events(&project, &events);
    (project, events)
}
```

`close_stale_plugin_windows` 原来收 `(&AppHandle, &State<AppState>)`，现在是 `(d: &Daemon)`；里面 `app.get_webview_window(...)` 的调用改成：

```rust
let Some(ui) = d.host().ui() else { return };  // 无窗口：没有窗口可关
```

需要窗口操作的地方后面接 `ui`。M1 里 `UiHost` 还没有"关窗"能力，先在 `UiHost` 加一条：

```rust
/// 关掉某个插件编辑器窗口（插件被换掉时用）。
fn close_plugin_editor(&self, track: u64, slot: i32);
```

`TauriHost` 的实现：查 `crate::` 里现成的关窗逻辑（`commands::close_plugin_window` 之类的辅助函数）并调用；`FakeHost` 不需要（`ui()` 返回 `None`，走上面的 early return）。

- [ ] **Step 5: 让未迁移命令继续工作（过渡态）**

未迁移的命令仍然需要 `&tauri::AppHandle` 与 `&AppState`。**不需要任何桥接函数**：Tauri 的 `State<T>`
会在 deref 时一路走到 `T`，所以 `&state`（`State<'_, Arc<Daemon>>`）可以直接传给收 `&Daemon` 的函数。

**这一步必须一次做完两件事，否则中间提交跑不起来**：

(a) 把两份状态都托管进去。改 `src-tauri/src/lib.rs` 的 `setup()`：

```rust
let state = AppState::bootstrap(session, cached);
let daemon = ai_daw_daemon::daemon::Daemon::new(
    std::sync::Arc::new(state),
    std::sync::Arc::new(crate::tauri_host::TauriHost::new(app.handle().clone())),
    ai_daw_daemon::daemon::Config { data_dir: app.path().app_data_dir().unwrap_or_default(), lan: false, port: Some(28430) },
);
app.manage(std::sync::Arc::clone(&daemon));            // State<'_, Arc<Daemon>>
app.manage(std::sync::Arc::clone(daemon.state_arc())); // State<'_, Arc<AppState>>（未迁移命令用）
```

`setup()` 里其余 `app.state::<AppState>()` 全部改成 `app.state::<Arc<AppState>>()`（约 10 处）。

(b) 给未迁移的命令留一条**行为等价**的旧入口，避免两条路径漂移：

```rust
/// 过渡期：未迁移的命令还拿着 `AppHandle`，从托管状态里取同一个 `Daemon`。
///
/// 与迁移后的 `run()` 走的是同一份 `AppState` 和同一个 `Host`，只是入口不同。
fn run_legacy(app: &tauri::AppHandle, command: Command) -> Project {
    use tauri::Manager;
    let daemon = app.state::<std::sync::Arc<Daemon>>();
    run(&daemon, command)
}
```

未迁移的命令把 `run(&app, &state, cmd)` 改成 `run_legacy(&app, cmd)`（函数一旦不再用 `state`，
签名里的 `state: State<'_, Arc<AppState>>` 就一起删掉，编译器会指出所有残留）。

- [ ] **Step 6: 把未迁移命令的 `State` 类型机械改一遍**

```bash
perl -pi -e "s/State<'_, AppState>/State<'_, Arc<AppState>>/g" crates/daemon/src/commands.rs
perl -pi -e "s/&State<'_, AppState>/&State<'_, Arc<AppState>>/g" crates/daemon/src/commands.rs
```

（`state.engine` 这类字段访问靠 `Deref` 自动生效，函数体不用改。）

- [ ] **Step 6: 编译**

```bash
cargo build -p ai-daw-daemon 2>&1 | head -30
```

- [ ] **Step 7: 提交**

```bash
git add -A && git commit -m "refactor(daemon): Daemon 容器，编辑入口改走 Host"
```

---

### Task 4: 迁移 project / track 命令

**Files:**
- Modify: `crates/daemon/src/commands.rs`

**Interfaces:**
- Produces: 每个命令一对 `fn <name>_impl(d: &Daemon, …) -> T` + `#[tauri::command] fn <name>(state: State<'_, Arc<Daemon>>, …) -> T`
- Consumes: `Daemon`、`run_all`

**要迁移的命令（按文件顺序）：**
`get_project`（无 `_impl`，直接读 `d.state()`）、`add_track`、`remove_track`、`rename_track`、`set_track_instrument`、`set_param`、`set_params`、`set_tempo`、`set_time_signature`、`set_keyframe`、`remove_keyframe`、`new_project`、`rename_project`、`current_project_path`、`session_restored`、`cache_session`、`project_history`、`delete_project_history`、`restore_project_history`、`localize_starter_tracks`、`history_depths`、`ops_list`、`ops_revert_to`、`ops_reapply_to`、`undo`、`redo`、`set_track_meta`、`set_mpe`、`set_project_harmony`、`set_project_sections`、`list_articulation_maps`、`import_articulation_map`

- [ ] **Step 1: 写失败测试**

`crates/daemon/src/commands.rs` 的测试模块里（若没有就新建 `#[cfg(test)] mod tests`）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{Config, Daemon};
    use crate::host::FakeHost;

    fn test_daemon() -> (Arc<Daemon>, Arc<FakeHost>) {
        // `bootstrap(None, vec![])` 走 idle 音频宿主：不开设备，CI 里也能跑。
        let state = Arc::new(AppState::bootstrap(None, Vec::new()));
        let host = Arc::new(FakeHost::default());
        (Daemon::new(state, host.clone(), Config::default()), host)
    }

    #[test]
    fn add_track_appends_and_emits_one_event() {
        let (d, host) = test_daemon();
        let before = get_project_impl(&d).tracks.len();
        let project = add_track_impl(&d, "Lead".into());
        assert_eq!(project.tracks.len(), before + 1);
        assert_eq!(project.tracks.last().unwrap().name, "Lead");
        let events = host.events.lock().unwrap();
        assert!(events.iter().any(|(event, _)| event == EVENT_ENGINE));
    }

    #[test]
    fn undo_after_add_track_restores_previous_length() {
        let (d, _host) = test_daemon();
        let before = get_project_impl(&d).tracks.len();
        add_track_impl(&d, "Lead".into());
        undo_impl(&d);
        assert_eq!(get_project_impl(&d).tracks.len(), before);
    }

    #[test]
    fn new_project_replaces_tracks() {
        let (d, _host) = test_daemon();
        let project = new_project_impl(&d, Some(vec!["A".into()]), Some(140.0));
        assert_eq!(project.tracks.len(), 1);
        assert_eq!(project.transport.tempo, 140.0);
    }
}
```

（`FakeHost.events` 需要类型上的 `pub`：Task 2 已给。**注意**：`Daemon::host()` 返回 `&Arc<dyn Host>`，
 拿不到 `FakeHost` 的字段，所以测试里必须从 `test_daemon()` 拿那份 `Arc<FakeHost>`。）

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib commands::tests 2>&1 | tail -8
```

期望：`cannot find function get_project_impl`。

- [ ] **Step 3: 按模板改造每个命令**

改造前：

```rust
#[tauri::command]
pub fn add_track(app: tauri::AppHandle, state: State<'_, Arc<AppState>>, name: String) -> Project {
    run(&app, &state, Command::AddTrack { name })
}
```

改造后：

```rust
/// 新增一条轨道。
pub fn add_track_impl(d: &Daemon, name: String) -> Project {
    run(d, Command::AddTrack { name })
}

/// 新增一条轨道（Tauri 入口）。
#[tauri::command]
pub fn add_track(state: State<'_, Arc<Daemon>>, name: String) -> Project {
    add_track_impl(&state, name)
}
```

规则：

1. 去掉 `app: tauri::AppHandle` 参数；
2. `State<'_, Arc<AppState>>` → `State<'_, Arc<Daemon>>`（只留在薄壳上）；
3. 函数体里的 `state` → `d.state()`，`&app, &state` → `d`；
4. 只读命令（`get_project`、`history_depths`、`ops_list`、`current_project_path`、`session_restored`、`list_articulation_maps`）不需要发事件，直接 `d.state()` 读。

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib commands::tests 2>&1 | tail -8
```

期望：3 passed。

- [ ] **Step 5: 保证桌面壳仍可编译**

```bash
cd src-tauri && cargo build 2>&1 | tail -20
```

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "refactor(daemon): 迁移 project/track 命令到 Daemon"
```

---

### Task 5: 迁移 clip / note 命令

**Files:**
- Modify: `crates/daemon/src/commands.rs`

**命令名单：**
`set_clip_rate`、`trim_clip`、`clip_bounds`、`add_clip`、`add_midi_clip`、`add_audio_clip`、`remove_clip`、`remove_clips`、`move_clip`、`move_clips`、`resize_clip`、`duplicate_clip`、`merge_clips`、`add_note`、`add_notes`、`set_note`、`set_notes`、`replace_notes`、`update_notes`、`remove_note`、`remove_notes`、`remove_notes_by_id`、`transpose_notes`、`set_note_velocity`、`quantize_notes`、`apply_swing`、`strum_notes`、`humanize_notes`、`fit_notes_to_range`、`get_track_note_stats`、`analyze_track_overlap`、`analyze_harmony`、`set_note_tag`、`set_note_tags`、`set_midi_events`、`set_articulations`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn add_notes_then_quantize_moves_starts_to_grid() {
    let (d, _host) = test_daemon();
    let project = new_project_impl(&d, Some(vec!["Piano".into()]), Some(120.0));
    let track = project.tracks[0].id;
    let clip = add_clip_impl(&d, track, 0.0, 4.0);
    let clip_id = clip.tracks[0].clips.last().unwrap().id;

    // 两个不在网格上的音符：起点 0.13 / 1.37
    set_notes_impl(
        &d,
        clip_id,
        vec![
            Note { pitch: 60, start: 0.13, duration: 0.5, velocity: 100, volume: 127 },
            Note { pitch: 62, start: 1.37, duration: 0.5, velocity: 100, volume: 127 },
        ],
    );
    quantize_notes_impl(&d, clip_id, 0.25, 1.0, Vec::new());

    let notes = get_project_impl(&d)
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == clip_id)
        .unwrap()
        .notes
        .clone();
    for note in notes {
        let grid = (note.start / 0.25).round() * 0.25;
        assert!((note.start - grid).abs() < 1e-9, "note at {} not on grid", note.start);
    }
}
```

**注意**：`set_notes_impl` / `quantize_notes_impl` / `add_clip_impl` 的参数必须与迁移后的真实签名一致；实现前先用 `grep -n "pub fn quantize_notes" -A6 crates/daemon/src/commands.rs` 核对一遍，按实际参数调整测试调用（这是本任务唯一的适配点）。

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib commands::tests::add_notes_then_quantize 2>&1 | tail -5
```

- [ ] **Step 3: 迁移本组 37 个命令**

规则与 Task 4 相同。本组有一个额外差异：音符类命令通常返回 `Project` 并且参数是一个结构体，
用 `quantize_notes` 举例：

改造前：

```rust
#[tauri::command]
pub fn quantize_notes(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    clip: u64,
    grid: f64,
    strength: f64,
    notes: Vec<u64>,
) -> Project {
    run(
        &app,
        &state,
        Command::QuantizeNotes { clip, grid, strength, notes },
    )
}
```

改造后：

```rust
/// 把片段里的音符起点量化到网格。
pub fn quantize_notes_impl(d: &Daemon, clip: u64, grid: f64, strength: f64, notes: Vec<u64>) -> Project {
    run(d, Command::QuantizeNotes { clip, grid, strength, notes })
}

/// 把片段里的音符起点量化到网格（Tauri 入口）。
#[tauri::command]
pub fn quantize_notes(
    state: State<'_, Arc<Daemon>>,
    clip: u64,
    grid: f64,
    strength: f64,
    notes: Vec<u64>,
) -> Project {
    quantize_notes_impl(&state, clip, grid, strength, notes)
}
```**

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib commands::tests 2>&1 | tail -8
```

- [ ] **Step 5: 提交**

```bash
git add -A && git commit -m "refactor(daemon): 迁移 clip/note 命令到 Daemon"
```

---

### Task 6: 迁移 transport / mixer / meters / 混音命令

**Files:**
- Modify: `crates/daemon/src/commands.rs`

**命令名单：**
`set_gain`、`set_pan`、`set_mute`、`set_solo`、`set_master_gain`、`set_sends`、`set_automation`、`monitor_track_mix`、`transport_status`、`transport_play`、`transport_pause`、`transport_stop`、`transport_seek`、`transport_set_loop`、`transport_set_loop_range`、`transport_set_metronome`、`retry_audio`、`list_output_devices`、`set_output_device`、`list_input_devices`、`set_input_device`、`input_level`、`list_midi_inputs`、`set_midi_input`、`midi_input_device`、`record_start`、`mixer_meters`、`clear_meter_clip`、`scope_data`、`audition_note`、`audition_midi_events`、`app_ready`

**注意**：`app_ready` 里含窗口显示逻辑（`get_webview_window("main").show()`）。迁移时把"显示窗口"拆出去：

```rust
pub fn app_ready_impl(d: &Daemon) {
    if let Some(ui) = d.host().ui() {
        ui.show_main_window();          // 新增到 UiHost
    }
    d.state().start_audio();
}

#[tauri::command]
pub fn app_ready(state: State<'_, Arc<Daemon>>) {
    app_ready_impl(&state);
}
```

`UiHost` 只加一条：

```rust
/// 首帧画完后把主窗口显示出来（窗口创建时是隐藏的，避免白屏）。
fn show_main_window(&self);
```

`TauriHost` 实现：`self.app.get_webview_window("main").map(|w| { let _ = w.show(); });`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn set_gain_updates_track_and_emits_event() {
    let (d, host) = test_daemon();
    let project = new_project_impl(&d, Some(vec!["Bass".into()]), Some(120.0));
    let track = project.tracks[0].id;
    let events_before = host.events.lock().unwrap().len();

    let updated = set_gain_impl(&d, track, -6.0);
    assert!((updated.tracks[0].gain_db - (-6.0)).abs() < 1e-9);
    assert!(host.events.lock().unwrap().len() > events_before);
}

#[test]
fn transport_play_without_device_returns_error_not_panic() {
    let (d, _host) = test_daemon();  // idle 音频宿主：没有设备
    let result = transport_play_impl(&d);
    // 允许成功也允许失败，但**绝不能 panic**；这条守住的是「无窗口进程里按播放不会崩」。
    let _ = result;
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib commands::tests 2>&1 | tail -5
```

- [ ] **Step 3: 按模板迁移本组 32 个命令，并拆出 `app_ready_impl`**

- [ ] **Step 4: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib commands::tests 2>&1 | tail -8
```

期望：5 passed。

- [ ] **Step 5: 桌面壳编译 + 提交**

```bash
cd src-tauri && cargo build 2>&1 | tail -10
cd .. && git add -A && git commit -m "refactor(daemon): 迁移 transport/mixer 命令，app_ready 拆出窗口职责"
```

---

### Task 7: `/rpc` 分发与 axum 服务

**Files:**
- Create: `crates/daemon/src/server/mod.rs`
- Create: `crates/daemon/src/server/rpc.rs`
- Modify: `crates/daemon/src/lib.rs`（`pub mod server;`）

**Interfaces:**
- Produces:
  - `server::ServerState { pub daemon: Arc<Daemon>, pub token: String }`
  - `server::router(state: Arc<ServerState>) -> axum::Router`
  - `server::rpc::dispatch(d: &Daemon, cmd: &str, args: Value) -> Result<Value, RpcError>`
  - `enum RpcError { Unknown(String), BadArgs(String), Failed(String), Unauthorized }`（`impl IntoResponse`：400 / 400 / 500 / 401）

- [ ] **Step 1: 写失败测试**（`crates/daemon/src/server/mod.rs` 末尾）

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{Config, Daemon};
    use crate::host::FakeHost;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn server() -> axum::Router {
        let state = Arc::new(crate::state::AppState::bootstrap(None, Vec::new()));
        let daemon = Daemon::new(state, Arc::new(FakeHost::default()), Config::default());
        router(Arc::new(ServerState { daemon, token: "t0ken".into() }))
    }

    async fn call(app: axum::Router, uri: &str, auth: Option<&str>, body: &str) -> (StatusCode, String) {
        let mut builder = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("authorization", format!("Bearer {auth}"));
        }
        let response = app.oneshot(builder.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn get_project_requires_token() {
        let (status, _) = call(server(), "/rpc/get_project", None, "{}").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_project_returns_project_json() {
        let (status, body) = call(server(), "/rpc/get_project", Some("t0ken"), "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"tracks\""));
    }

    #[tokio::test]
    async fn add_track_returns_updated_project() {
        let (status, body) = call(server(), "/rpc/add_track", Some("t0ken"), r#"{"name":"Lead"}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Lead"));
    }

    #[tokio::test]
    async fn unknown_command_is_400() {
        let (status, body) = call(server(), "/rpc/no_such_cmd", Some("t0ken"), "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("unknown command"));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib server::tests 2>&1 | tail -5
```

- [ ] **Step 3: 实现 `server/rpc.rs`**

```rust
//! 命令名 → 实现函数的唯一映射表。
//!
//! 每迁移一组命令，就在这里加一组分支；**没有分支的命令返回 400**，不会静默成功。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::Value;

use crate::daemon::Daemon;

pub enum RpcError {
    Unknown(String),
    BadArgs(String),
    Failed(String),
    Unauthorized,
}

impl IntoResponse for RpcError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            RpcError::Unknown(cmd) => (StatusCode::BAD_REQUEST, format!("unknown command: {cmd}")),
            RpcError::BadArgs(err) => (StatusCode::BAD_REQUEST, format!("bad args: {err}")),
            RpcError::Failed(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
            RpcError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".into()),
        };
        (status, axum::Json(serde_json::json!({ "error": message }))).into_response()
    }
}

#[derive(Deserialize)]
struct AddTrackArgs {
    name: String,
}

pub fn dispatch(d: &Daemon, cmd: &str, args: Value) -> Result<Value, RpcError> {
    match cmd {
        "get_project" => json(crate::commands::get_project_impl(d)),
        "add_track" => {
            let a: AddTrackArgs = serde_json::from_value(args).map_err(|e| RpcError::BadArgs(e.to_string()))?;
            json(crate::commands::add_track_impl(d, a.name))
        }
        // …按 Task 4/5/6 的名单逐条加分支
        other => Err(RpcError::Unknown(other.to_string())),
    }
}

fn json<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::Failed(e.to_string()))
}
```

- [ ] **Step 4: 实现 `server/mod.rs`**

```rust
//! HTTP 服务：`/rpc/<cmd>` + `/health`。
//!
//! axum 0.8 的路径参数语法是 `{name}`（不是 `:name`）。

pub mod rpc;

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::Value;

use crate::daemon::Daemon;
use self::rpc::RpcError;

pub struct ServerState {
    pub daemon: Arc<Daemon>,
    pub token: String,
}

pub fn router(state: Arc<ServerState>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc_handler))
        .route("/health", get(health))
        .with_state(state)
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

fn authorized(headers: &HeaderMap, query: &TokenQuery, token: &str) -> bool {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    // EventSource / WebSocket 带不了 header，所以 query 也是合法通道。
    bearer == Some(token) || query.token.as_deref() == Some(token)
}

async fn rpc_handler(
    State(state): State<Arc<ServerState>>,
    Path(cmd): Path<String>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&headers, &query, &state.token) {
        return RpcError::Unauthorized.into_response();
    }
    let args: Value = if body.is_empty() {
        Value::Object(Default::default())
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(err) => return RpcError::BadArgs(err.to_string()).into_response(),
        }
    };
    // 命令实现里有渲染、读盘、加锁：绝不放在 async 任务的 poll 里。
    let daemon = Arc::clone(&state.daemon);
    let result = tokio::task::spawn_blocking(move || rpc::dispatch(&daemon, &cmd, args)).await;
    match result {
        Ok(Ok(value)) => axum::Json(value).into_response(),
        Ok(Err(err)) => err.into_response(),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("rpc task failed: {err}")).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib server::tests 2>&1 | tail -10
```

期望：4 passed。

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "feat(daemon): /rpc 分发与 axum 服务（含 token 鉴权）"
```

---

### Task 8: `boot::bootstrap` 与 `ai-daw` 无窗口入口

**Files:**
- Create: `crates/daemon/src/boot.rs`
- Create: `crates/daemon/src/bin/ai-daw.rs`
- Modify: `crates/daemon/src/lib.rs`（`pub mod boot;`）
- Modify: `crates/daemon/Cargo.toml`（`clap`）

**Interfaces:**
- Produces:
  - `boot::HeadlessHost`（`Host` 实现，`data_dir` 可配、`ui()` 为 `None`）
  - `boot::bootstrap(host: Arc<dyn Host>, cfg: Config) -> Arc<Daemon>`
  - `boot::serve(daemon: Arc<Daemon>, token: String) -> (SocketAddr, impl Future<Output = ()>)`
  - `bin/ai-daw`：`ai-daw [--data-dir DIR] [--port 28430] [--lan] [--token TOKEN]`

- [ ] **Step 1: 写失败测试**

`crates/daemon/src/boot.rs` 末尾：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_builds_a_daemon_without_touching_tauri() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(HeadlessHost::new(dir.path()));
        let cfg = Config { data_dir: dir.path().to_path_buf(), lan: false, port: None };
        let daemon = bootstrap(host, cfg);
        assert!(daemon.state().engine.lock().unwrap().project().tracks.len() > 0);
        assert!(daemon.host().ui().is_none());
    }

    #[tokio::test]
    async fn serve_returns_bound_address_and_answers_health() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(HeadlessHost::new(dir.path()));
        let cfg = Config { data_dir: dir.path().to_path_buf(), lan: false, port: Some(0) };
        let daemon = bootstrap(host, cfg);
        let (addr, fut) = serve(daemon, "t".into()).await.unwrap();
        assert!(addr.port() > 0);
        tokio::spawn(fut);
        let body = reqwest::get(format!("http://{addr}/health")).await.unwrap().text().await.unwrap();
        assert!(body.contains("\"ok\""));
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib boot::tests 2>&1 | tail -5
```

- [ ] **Step 3: 实现 `boot.rs`**

```rust
//! 无窗口启动：会话、预设、插件目录、管理器，然后交给调用方决定听哪里。
//!
//! 桌面端的 `setup()` 与 `ai-daw` 都调这里；**桌面独有的部分**（建窗、原生插件窗口、
//! 更新器）留在外壳里，不进来。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use tokio::net::TcpListener;

use crate::daemon::{Config, Daemon};
use crate::host::{Host, UiHost};
use crate::state::AppState;

/// 无窗口进程的宿主：有数据目录、没有原生能力。
pub struct HeadlessHost {
    dir: PathBuf,
}

impl HeadlessHost {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self { dir: dir.as_ref().to_path_buf() }
    }
}

impl Host for HeadlessHost {
    fn emit_value(&self, _event: &'static str, _payload: Value) {
        // M3 接到 SSE 广播上；M1 先丢弃（命令本身仍然正确执行）。
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn ui(&self) -> Option<&dyn UiHost> {
        None
    }
}

/// 装配后端。**不含任何窗口/事件循环**，可以在任意线程上调用。
pub fn bootstrap(host: Arc<dyn Host>, cfg: Config) -> Arc<Daemon> {
    crate::paths::init_with_data_dir(cfg.data_dir.clone());
    let session = crate::session::read_session(&cfg.data_dir.join("session.json"));
    let catalog = Vec::new();
    let state = Arc::new(AppState::bootstrap(session, catalog));
    Daemon::new(state, host, cfg)
}

/// 起 HTTP 服务。返回实际绑定的地址与可被 `select!` 的前台 future。
pub async fn serve(daemon: Arc<Daemon>, token: String) -> std::io::Result<(SocketAddr, impl std::future::Future<Output = ()>)> {
    let cfg = daemon.config().clone();
    let port = cfg.port.unwrap_or(28430);
    let host = if cfg.lan { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let listener = TcpListener::bind((host, port)).await?;
    let addr = listener.local_addr()?;
    let app = crate::server::router(Arc::new(crate::server::ServerState { daemon, token }));
    Ok((addr, async move {
        let _ = axum::serve(listener, app).await;
    }))
}
```

`paths::init_with_data_dir` 是新增函数（现有 `paths.rs` 的路径解析都从 `app.path()` 取）。实现方式：在 `paths.rs` 里加一个 `static DATA_DIR: OnceLock<PathBuf>`，所有内部取目录的地方优先读它，读不到再走 `AppHandle`。M1 只要求 `data_dir()`/`config_dir()`/`models_dir()` 三个函数走这条覆盖，其余保持原样。

- [ ] **Step 4: 写 `bin/ai-daw.rs`**

```rust
//! 无窗口入口：`aidawd --port 28430 --lan`。
//!
//! 只跑后端 + HTTP 服务，不开窗口、不初始化 Tauri App。

use std::sync::Arc;

use ai_daw_daemon::boot::{bootstrap, serve, HeadlessHost};
use ai_daw_daemon::daemon::Config;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "ai-daw", version, about = "AI DAW 的无窗口后端")]
struct Args {
    /// 数据目录；默认用系统配置目录下的 com.aidaw.app。
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
    /// HTTP 端口。
    #[arg(long, default_value_t = 28430)]
    port: u16,
    /// 绑到 0.0.0.0（局域网可访问）。
    #[arg(long, default_value_t = false)]
    lan: bool,
    /// 访问令牌；不给就生成一个并打印。
    #[arg(long)]
    token: Option<String>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let data_dir = args.data_dir.unwrap_or_else(|| std::env::temp_dir().join("aidaw"));
    std::fs::create_dir_all(&data_dir)?;
    let token = args.token.unwrap_or_else(|| {
        use sha2::{Digest, Sha256};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let digest = Sha256::digest(nanos.to_string().as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    });
    let host = Arc::new(HeadlessHost::new(&data_dir));
    let cfg = Config { data_dir, lan: args.lan, port: Some(args.port) };
    let daemon = bootstrap(host, cfg);
    let (addr, fut) = serve(daemon, token.clone()).await?;
    println!("ai-daw 已启动：http://{addr}");
    println!("token: {token}");
    println!("试一下：curl -H 'Authorization: Bearer {token}' http://{addr}/rpc/get_project");
    fut.await;
    Ok(())
}
```

`crates/daemon/Cargo.toml` 的 `[dependencies]` 加 `clap = { version = "4", features = ["derive"] }`。

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib boot::tests 2>&1 | tail -8
```

- [ ] **Step 6: 手动冒烟**

```bash
cargo run -p ai-daw-daemon --bin aidawd -- --data-dir /tmp/aidaw-smoke --port 28430 --token devtoken &
sleep 3
curl -s http://127.0.0.1:28430/health                                    # 期望 {"status":"ok",...}
curl -s http://127.0.0.1:28430/rpc/get_project | head -c 120            # 期望 401 unauthorized
curl -s -H "Authorization: Bearer devtoken" http://127.0.0.1:28430/rpc/get_project | head -c 120
kill %1
```

期望：`/health` 返回 `{"status":"ok",...}`；不带 token 的 `/rpc` 返回 401。

- [ ] **Step 7: 提交**

```bash
git add -A && git commit -m "feat(daemon): boot::bootstrap 与 ai-daw 无窗口入口"
```

---

### Task 9: 桌面端同进程起 HTTP 服务

**Files:**
- Modify: `src-tauri/src/lib.rs`（`setup()` 末尾）
- Modify: `crates/daemon/src/paths.rs`（新增 `web_token`）

**Interfaces:**
- Consumes: `boot::serve`、`Daemon`（Task 3 已托管）
- Produces: `paths::web_token() -> String`；桌面进程在 `127.0.0.1:28430` 上提供 `/rpc`

- [ ] **Step 1: 写 `paths::web_token` 的失败测试**

`crates/daemon/src/paths.rs` 的测试模块里：

```rust
#[test]
fn web_token_is_stable_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    crate::paths::init_with_data_dir(dir.path().to_path_buf());
    let first = crate::paths::web_token();
    let second = crate::paths::web_token();
    assert_eq!(first, second, "同一份数据目录下 token 必须稳定");
    assert_eq!(first.len(), 64, "32 字节的十六进制");
    assert!(dir.path().join("web-token").exists());
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib paths::tests::web_token 2>&1 | tail -5
```

- [ ] **Step 3: 实现**

```rust
/// 浏览器客户端的访问令牌：读 `data_dir/web-token`，没有就生成并落盘。
///
/// M4 会换成 CSPRNG 并加轮换；这里先用系统时间 + sha256 拼出 32 字节，
/// 目的是"同一台机器上稳定、不可猜"，局域网内的常见威胁（懒人密码）已覆盖。
pub fn web_token() -> String {
    let path = data_dir().join("web-token");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let trimmed = text.trim();
        if trimmed.len() == 64 {
            return trimmed.to_string();
        }
    }
    use sha2::{Digest, Sha256};
    let seed = format!(
        "{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        data_dir().display()
    );
    let token: String = Sha256::digest(seed.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let _ = std::fs::write(&path, &token);
    token
}
```

- [ ] **Step 4: 在 `setup()` 末尾起服务**

```rust
{
    let daemon = std::sync::Arc::clone(&daemon);
    let token = ai_daw_daemon::paths::web_token();
    tauri::async_runtime::spawn(async move {
        match ai_daw_daemon::boot::serve(daemon, token).await {
            Ok((addr, fut)) => {
                ai_daw_daemon::logging::info(format!("[web] 监听 http://{addr}"));
                fut.await;
            }
            Err(err) => ai_daw_daemon::logging::info(format!("[web] 启动失败：{err}")),
        }
    });
}
```

注意 `serve` 读的是 `Config.port`；Task 3 里给的是 `Some(28430)`，所以这里绑的就是 28430。

- [ ] **Step 5: 编译、启动、冒烟**

```bash
cd src-tauri && cargo build 2>&1 | tail -20
cd .. && npx tauri dev   # 手动确认：窗口出来、能播放、能改一条轨道
```

另开一个终端：

```bash
TOKEN=$(cat "$HOME/Library/Application Support/com.aidaw.app/web-token")
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:28430/rpc/get_project | head -c 200
curl -s -H "Authorization: Bearer $TOKEN" -H "content-type: application/json" \
     -d '{"name":"FromCurl"}' http://127.0.0.1:28430/rpc/add_track | head -c 200
```

期望：第一条返回与桌面窗口里一致的工程 JSON；第二条返回多了一条 `FromCurl` 轨道的工程，**并且桌面窗口里也出现了这条轨道**（同一份状态）。

- [ ] **Step 6: 提交**

```bash
git add -A && git commit -m "feat(shell): 桌面端同进程起 HTTP 服务，token 落盘"
```

---

### Task 10: 收尾 —— 回归、文档、已知缺口

**Files:**
- Modify: `docs/superpowers/specs/2026-11-13-web-client-design.md`（§5.2 标记完成项）
- Create: `crates/daemon/README.md`

- [ ] **Step 1: 全量测试**

```bash
cargo test -p ai-daw-daemon 2>&1 | tail -15
cargo test --workspace 2>&1 | tail -15
```

期望：全绿（引擎与各 crate 的既有测试不能退化）。

- [ ] **Step 2: 写 `crates/daemon/README.md`**

内容必须包含：这个 crate 是什么、`Host`/`UiHost` 的契约、如何加一个新命令（`_impl` + 薄壳 + `dispatch` 分支三处）、`ai-daw` 怎么跑、M2/M3 还要接什么（`/events`、`/audio`）。

- [ ] **Step 3: 在 spec 的 §5.2 打勾**

把已迁移的六组标成"已完成（M1）"，并在 §12 的 M1 行补一句实际耗时与偏差。

- [ ] **Step 4: 提交**

```bash
git add -A && git commit -m "docs(daemon): M1 收尾：README 与 spec 进度"
```

---

## M1 之后的边界（各自独立计划）

| 里程碑 | 范围 | 为什么分开 |
| --- | --- | --- |
| M2 | `WebTap` + `/audio` WS + 浏览器播放器 + 时钟同步 | 只碰音频线程与前端，与命令迁移无交叉 |
| M3 | `/events` SSE + 剩余 5 组命令迁移 + 前端双 Channel + 文件/导出 | 纯机械，量大但风险低 |
| M4 | token 加固、多客户端、降级矩阵、打包 | 收口工作，依赖前三步的结果 |
