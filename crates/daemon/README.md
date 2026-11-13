# ai-daw-daemon

AI DAW 的后端 crate：工程模型、音频引擎、插件宿主、本地 AI、MCP 服务，以及给浏览器用的 HTTP `/rpc`。

它**不拥有窗口**。需要原生能力（对话框、插件编辑器窗口、主线程泵）时通过 `host::Host` 拿；
桌面端（`src-tauri`）与无窗口进程（`aidawd`）各注入一份实现。于是同一套命令实现既服务
webview，也服务浏览器客户端。

```
crates/daemon/          ← 你在这里
├── src/host.rs         Host / UiHost trait、FakeHost、NoopHost
├── src/daemon.rs       Daemon 容器（Arc<AppState> + Host + Config）
├── src/boot.rs         无窗口启动：bootstrap / serve / web_token
├── src/server/         axum：/rpc/{cmd}、/health、/commands
├── src/bin/aidawd.rs   无窗口入口
└── src/ …              原有后端模块（commands / audio / state / mcp / manager …）
```

## 契约

```rust
pub trait Host: Send + Sync + 'static {
    fn emit_value(&self, event: &'static str, payload: Value); // 事件广播
    fn data_dir(&self) -> PathBuf;                             // 数据目录
    fn ui(&self) -> Option<&dyn UiHost>;                       // 没有原生能力时返回 None
}
```

- **桌面**：`src-tauri/src/tauri_host.rs` 把事件转给 webview，`ui()` 返回 `Some(self)`。
- **无窗口**：`boot::HeadlessHost`，`ui()` 返回 `None`，事件当前丢弃（M3 接 SSE）。
- **测试**：`host::FakeHost` 把所有事件记进 `events`，断言"有没有通知界面"。

`AppState` 持有 `Host`，所以**所有**命令实现走的是同一条广播通道——不会出现
"Tauri 里能收到、HTTP 客户端收不到"的分叉。

## 加一条新命令（三处）

1. 写实现（无 Tauri 参数）：

```rust
pub fn set_gain_impl(state: &AppState, track: u64, gain_db: f32) -> Project {
    crate::commands::run_one(state, Command::SetGain { track: TrackId(track), gain_db })
}
```

2. 加 Tauri 薄壳（紧挨着实现，名字不变，前端 `invoke` 契约不变）：

```rust
#[tauri::command]
pub fn set_gain(state: State<'_, Arc<AppState>>, track: u64, gain_db: f32) -> Project {
    set_gain_impl(&state, track, gain_db)
}
```

3. 在 `server/rpc.rs` 的 `dispatch` 加一个分支，并把名字加进 `COMMANDS`。

**没加第 3 步的命令在 HTTP 上会返回 400 `unknown command`**——明确失败，不会静默成功。

## 无窗口运行

```bash
cargo run -p ai-daw-daemon --bin aidawd -- --data-dir /tmp/aidaw --port 28430 --token devtoken
curl -s -H "Authorization: Bearer devtoken" http://127.0.0.1:28430/rpc/get_project | head -c 200
curl -s -H "Authorization: Bearer devtoken" -H "content-type: application/json" \
     -d '{"name":"Lead"}' http://127.0.0.1:28430/rpc/add_track | head -c 200
```

- 默认只绑 `127.0.0.1`；`--lan` 才绑 `0.0.0.0`（容器里必须开）。
- token 来源优先级：`--token` → `AIDAW_TOKEN` 环境变量 → 数据目录里的 `web-token` → 新生成并落盘。
- 没有令牌时 `serve` 直接 panic：局域网内的裸 `/rpc` 等于把整台机器的文件读写交出去。

## Docker

```bash
docker build -t aidawd .
docker run --rm -p 28430:28430 -v aidaw-data:/data aidawd
# 或
docker compose up --build
```

首次启动的日志里有 token；数据（会话、预设、音色记忆库、token）都在 `/data` 卷里。

容器里没有声卡：`/health` 会显示 `"audio": false`，日志里有 ALSA 的探测报错，这是正常的
——`/rpc` 的工程编辑、撤销、导入导出都不依赖输出设备。

> 已实测：在 Debian 13 + webkit2gtk-4.1 环境下 `cargo build --release -p ai-daw-daemon --bin aidawd`
> 通过，容器内 `aidawd` 能起服务、`/health` 返回 ok、`/rpc/add_track` 能改工程。
> 本机 Docker Hub 不可达，所以仓库里的 `Dockerfile`（`rust:1-bookworm` / `debian:bookworm-slim`）
> 未在本地跑通完整镜像构建，包清单与运行期依赖已按 `ldd` 实测结果对齐。

> 过渡期说明：本 crate 仍依赖 `tauri`（只为 `#[tauri::command]` 宏），所以镜像的构建层需要
> webkit2gtk 头文件、运行层需要它的运行库。等命令层与 Tauri 彻底解耦后，这两层都能瘦很多。

## 现状（M1）

| 项 | 状态 |
| --- | --- |
| 后端整体迁入本 crate | ✅ 桌面壳只剩窗口 / 原生窗口 / 更新器 |
| `Host` / `UiHost` 与三份实现 | ✅ |
| `run_all` 走 Host 广播（无窗口也能编辑） | ✅ |
| 无窗口启动 + `/rpc` + 鉴权 | ✅ 31 条命令（工程 / 轨道 / 片段 / 音符 / 混音 / 传输 / 历史） |
| 无 Tauri 单测 | ✅ 228 条（含 `/rpc` 的鉴权、坏参数、未知命令、事件广播） |
| 剩下 ~95 条命令的 `_impl` | ⬜ M3 |
| `GET /events`（SSE）、`WS /audio`（PCM） | ⬜ M2 / M3 |
| 静态资源托管（浏览器直接开） | ⬜ M3 |
