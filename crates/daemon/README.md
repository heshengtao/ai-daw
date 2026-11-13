# ai-daw-daemon

AI DAW 的后端 crate：工程模型、音频引擎、插件宿主、本地 AI、MCP 服务，以及给浏览器用的 HTTP `/rpc`。

它**不拥有窗口**。需要原生能力（对话框、插件编辑器窗口、主线程泵）时通过 `host::Host` 拿；
桌面端（`src-tauri`）与无窗口进程（`ai-daw`）各注入一份实现。于是同一套命令实现既服务
webview，也服务浏览器客户端。

```
crates/daemon/          ← 你在这里
├── src/host.rs         Host / UiHost trait、FakeHost、NoopHost
├── src/daemon.rs       Daemon 容器（Arc<AppState> + Host + Config）
├── src/boot.rs         无窗口启动：bootstrap / serve / web_token
├── src/server/         axum：/rpc/{cmd}、/health、/commands
├── src/bin/ai-daw.rs   无窗口入口
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
cargo run -p ai-daw-daemon --bin aidawd -- --data-dir /tmp/aidawd --port 28430 --token devtoken
curl -s -H "Authorization: Bearer devtoken" http://127.0.0.1:28430/rpc/get_project | head -c 200
curl -s -H "Authorization: Bearer devtoken" -H "content-type: application/json" \
     -d '{"name":"Lead"}' http://127.0.0.1:28430/rpc/add_track | head -c 200
```

- 默认只绑 `127.0.0.1`；`--lan` 才绑 `0.0.0.0`（容器里必须开）。
- token 来源优先级：`--token` → `AIDAW_TOKEN` 环境变量 → 数据目录里的 `web-token` → 新生成并落盘。
- 没有令牌时 `serve` 直接 panic：局域网内的裸 `/rpc` 等于把整台机器的文件读写交出去。

## 浏览器直接用

```bash
npm run build                                    # 产出 dist/
cargo run -p ai-daw-daemon --bin aidawd -- --token dev --port 28430 --web-dir dist
# 打开 http://127.0.0.1:28430/#token=dev
```

前端只有一处改动：`src/api/tauri.ts` 的 `invoke` / `listen` 换成 `src/api/ipc.ts` 的同名实现，
在浏览器里自动走 `POST /rpc/<cmd>` 与 `GET /events`（SSE）。命令名、参数名、事件通道名
三者与桌面端完全一致，所以 237 个调用点一行没改。

## Docker

```bash
docker build -t ai-daw .
docker run --rm -p 28430:28430 -v ai-daw-data:/data ai-daw
# 或
docker compose up --build
```

首次启动的日志里有 token；数据（会话、预设、音色记忆库、token）都在 `/data` 卷里。

容器里没有声卡：`/health` 会显示 `"audio": false`，日志里有 ALSA 的探测报错，这是正常的
——`/rpc` 的工程编辑、撤销、导入导出都不依赖输出设备。

> 已实测：在 Debian 13 + webkit2gtk-4.1 环境下 `cargo build --release -p ai-daw-daemon --bin aidawd`
> 通过，容器内 `ai-daw` 能起服务、`/health` 返回 ok、`/rpc/add_track` 能改工程。
> 本机 Docker Hub 不可达，所以仓库里的 `Dockerfile`（`rust:1-bookworm` / `debian:bookworm-slim`）
> 未在本地跑通完整镜像构建，包清单与运行期依赖已按 `ldd` 实测结果对齐。

> **体积（实测）**：`ai-daw:latest` = **201 MB**（二进制 20 MB + Debian slim 底座 +
> libasound2 / libssl3 / libxcb1 / libx11-6 / ca-certificates）。之前带 WebKit 的版本是
> 1.07 GB —— 现在 `ldd` 里**没有任何 webkit / gtk**。
>
> 关键在 `--no-default-features`：本 crate 的 `desktop` 特性（默认开）才会启用 `tauri`，
> 关掉之后 `tauri` 根本不进依赖图，WebKit/GTK 那 612 MB 也就不会被链进来。
>
> 无头构建里 `crate::tauri_api` 是一组空壳类型（只求编译，无头进程从不构造
> `AppHandle`）；桌面构建走 `pub use tauri::*`，行为完全一致。这是过渡脚手架：
> M3 把命令层搬回 `src-tauri` 后，这层空壳和所有 `AppHandle` 参数一起删掉。

## CI / Docker Hub 发布

`.github/workflows/docker.yml` 在 `main` 与 `v*` tag 上构建并推送镜像：

| 触发 | 产出标签 |
| --- | --- |
| push `main` | `latest`、`sha-<短 sha>` |
| push `v1.7.0` | **`1.7.0`**、**`1.7`**、`latest`、`sha-<短 sha>` |
| 手动触发 | `latest`、`sha-<短 sha>` |

构建按 **digest** 推送，仓库里不会出现 `main-amd64` 这类中间标签；两个架构的 digest
最后一个 job 合成多架构 manifest。

镜像名是 `<DOCKERHUB_USERNAME>/ai-daw`，需要在仓库里配两个 Secret：

- `DOCKERHUB_USERNAME`：Docker Hub 账号
- `DOCKERHUB_TOKEN`：Docker Hub → Account Settings → Personal access tokens

架构：amd64 用 `ubuntu-latest`、arm64 用 `ubuntu-24.04-arm` 各建一次（不走 QEMU），
最后由 `imagetools create` 合成多架构 manifest。

`.github/workflows/ci.yml` 是 PR 门禁：跑 daemon 的无 Tauri 单测 + 前端类型检查与打包。

## 现状（M1）

| 项 | 状态 |
| --- | --- |
| 后端整体迁入本 crate | ✅ 桌面壳只剩窗口 / 原生窗口 / 更新器 |
| `Host` / `UiHost` 与三份实现 | ✅ |
| `run_all` 走 Host 广播（无窗口也能编辑） | ✅ |
| 无窗口启动 + `/rpc` + 鉴权 | ✅ 31 条命令（工程 / 轨道 / 片段 / 音符 / 混音 / 传输 / 历史） |
| 无 Tauri 单测 | ✅ 228 条（含 `/rpc` 的鉴权、坏参数、未知命令、事件广播） |
| `/rpc` 暴露 96 个命令名（显式 43 + `Command` 兜底 53 个变体入口） | ✅ |
| `GET /events`（SSE）、`WS /audio`（PCM） | ✅ |
| 静态资源托管（浏览器直接开） | ✅ |
| 令牌加固（系统 CSPRNG） | ✅ |
| 浏览器端录音 / Web MIDI | ⬜ 需要 TLS（secure context） |
| AI 生成 / 模型下载 / 插件管理 / 歌声音色 | ⬜ 桌面专属（要本地路径、下载与原生窗口） |
