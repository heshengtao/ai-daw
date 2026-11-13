# Web 客户端（本机引擎 + 浏览器 UI）设计

- 日期：2026-11-13
- 状态：设计定稿，待实现（M1–M4 见 §12）
- 目标形态：**局域网内多设备用浏览器打开同一台机器的 DAW 会话**，实时监听（PCM 回传）+ 完整编辑
- 不在本设计范围：公网部署、多租户、纯浏览器（WASM）独立版

## 1. 问题与目标

AI DAW 现在是 Tauri 桌面应用：界面在 webview 里，工程模型、音频引擎、插件宿主全在 Rust 进程内。
用户想要的是：

> 同一条工程，在平板上打开就能听、能改；在第二台机器上也能打开同一份工程。

目标：

1. **一份后端**：桌面版与浏览器版跑同一套命令实现，不允许出现"两个后端、行为分叉"。
2. **实时监听**：浏览器里播放时能听到实时混音，端到端延迟目标 40–80 ms（WiFi）。
3. **完整编辑**：工程、轨道、片段、音符、混音、插件参数、导出，浏览器里都能做。
4. **渐进迁移**：每一步落地后桌面版都必须还能正常工作、可发布。
5. **明确降级**：浏览器拿不到的能力（原生插件窗口、系统对话框、录音、自动更新）显式降级并告知用户，不静默失败。

非目标（本阶段明确不做）：

- 公网访问、账号体系、多人协作（一个后端只有一份工程，多个客户端看到的是**同一个** session）。
- 纯浏览器（无本机后端）的 WASM 版：`cpal`、VST3/CLAP 宿主、`ort`、sidecar 二进制都无法进浏览器，那是另一条长线。
- 浏览器端录音与 Web MIDI：需要 secure context，留到开启 TLS 之后（§6.5）。
- 增量同步协议：`get_project()` 仍是整份工程下发，不做补丁流。

## 2. 为什么不是"把前端编成网页"

| 事实 | 证据 |
| --- | --- |
| 全部业务状态在 Rust 端，前端 237 处 `invoke` 调用 | `src/api/tauri.ts` |
| 后端 **239 个 Tauri 命令**，**489 处 `AppHandle`** 引用横跨 25 个文件 | `src-tauri/src/commands.rs`、`mcp.rs` |
| 音频 100% 原生（`cpal`），前端整个 `src/` 里没有一处 `AudioContext` | `crates/engine/Cargo.toml`、`crates/engine/src/realtime.rs` |
| 插件宿主会开**原生窗口**，依赖 dlopen 与 sidecar 进程 | `crates/host-vst3`、`src-tauri/binaries/` |
| 浏览器分支（`mock.ts`）只是内存假数据，不是真引擎 | `src/api/mock.ts:626` |

结论：前端可以编成静态站点，但它背后必须有一个真实的引擎进程。所以本设计不是"移植"，而是
**把后端从 Tauri 里解耦出来，让它能被两种前端驱动**。

## 3. 落地路径的选择

| 路径 | 做法 | 判断 |
| --- | --- | --- |
| ① 进程内网桥 | 在 `src-tauri` 里再起一个 axum，转发到现有命令 | 一周可验证，但后端永远绑着 Tauri，浏览器版必须依赖桌面 App 开着 |
| ② headless daemon（**采纳**） | 抽出 `crates/daemon`，命令实现只依赖 `Host` trait；桌面 App 与 `ai-daw` 都是它的外壳 | 终态干净，一条后端两种前端 |
| ③ 先①后② | 先网桥验证推流，再按域迁移 | 把风险最高的事串行化，不采纳 |

采纳 ② 的理由：这个方案里唯一**不确定**的只有"WiFi 下 50 ms 能不能听"，但重构本身是确定的机械工作量。
选择一次性做对，避免中间态长期存在。

## 4. 进程与部署拓扑

```
ai-daw（库 + 二进制）                    桌面 App（Tauri）
┌────────────────────────────┐          ┌────────────────────────────┐
│ Engine / AudioEngine       │          │ 同一份 daemon（进程内嵌）   │
│ 插件宿主 / MCP             │          │ + webview 窗口              │
│ HTTP /rpc   WS /audio      │          │ + 原生对话框 / 插件窗口      │
│ SSE /events  静态托管 dist │          └────────────────────────────┘
└────────────────────────────┘
      ▲                 ▲
      │ http            │ http
  浏览器（平板）      浏览器（第二台机器）
```

硬约束：

1. **`crates/daemon` 是可嵌入的库**：`Daemon::start(Config) -> Handle`。桌面 App 的 `lib.rs` 调它，
   `ai-daw` 的 `main.rs` 也调它。后端只有一份实现。
2. **桌面能力通过 `UiHost` 注入**：只有 Tauri 外壳提供原生对话框、插件原生窗口、主线程泵。
3. **端口与绑定**：默认 `127.0.0.1:28430`（与 vite dev 的 28420 分开，也避开 MCP 的 50562）；
   `--lan` 才绑 `0.0.0.0`。
4. **静态资源由 daemon 托管**：`dist/` 直接挂在 `/`。前端用 hash 路由（`createWebHashHistory`），
   不需要服务端 fallback 规则。

## 5. 后端改造：`Host` trait

### 5.1 现状与目标形态

命令层现在的签名是 `fn(app: AppHandle, state: State<'_, AppState>, …)`，实际用到的 `AppHandle`
能力只有四类：

| 用途 | 次数 | 改造后 |
| --- | --- | --- |
| `app.state::<AppState>()` | 110 | 直接持有 `&AppState` |
| `app.emit(…)` | 40 | `host.emit(event, payload)` |
| `app.get_webview_window` / `webview_windows` | 6 | `host.ui()`，浏览器端为 `None` |
| `app.path()` / `app.config()` | 7 | `host.data_dir()` |

```rust
pub trait Host: Send + Sync + 'static {
    fn state(&self) -> &AppState;
    fn emit(&self, event: &'static str, payload: serde_json::Value);
    fn data_dir(&self) -> PathBuf;
    /// 桌面端才有：原生对话框、插件原生窗口、主线程泵。
    fn ui(&self) -> Option<&dyn UiHost>;
}

pub trait UiHost {
    fn pick_file(&self, req: PickRequest) -> Result<Option<PathBuf>, String>;
    fn open_plugin_editor(&self, track: TrackId, slot: PluginSlot) -> Result<(), String>;
    fn run_on_main_thread(&self, f: Box<dyn FnOnce() + Send>);
    fn webview_alive(&self) -> bool;
}
```

### 5.2 迁移分域（每步以"桌面版可发布"为硬门槛）

1. `project` / `track` / `clip` / `note` / `transport`
2. `mix` / `automation` / `meters`
3. `audio`（导入、导出、渲染）
4. `plugins` / `params` / `presets` / `sound-memories`
5. `ai` / `models` / `runtimes` / `manager`（网络下载类）
6. `mcp`（工具层同样只依赖 `Host`；现在有 152 处 `AppHandle`）

### 5.3 为什么值得做

`Host` 可以有 `FakeHost`：那 239 个命令**第一次能在没有 Tauri、没有声卡的情况下跑单测**。
这次重构的真实回报在这里，而不在 web。

## 6. 实时音频通道

### 6.1 tap 点

`crates/engine/src/realtime.rs` 的 `on_data` 里，`staging[..need]` 就是最终混音后的 f32 立体声。
紧邻的 `scope.push_interleaved` 已经是"音频线程无锁外送"的既有范例
（`crates/engine/src/scope.rs:32`：纯原子写、不分配、不加锁）。

做法：再挂一个 `rtrb` ring（引擎已是依赖，容量约 2 s）：

- **写端（音频线程）**：`on_data` 末尾 push `staging[..need]`，附带 `frame_index`（从播放起点累计的采样数）。
- **读端（`web-audio` 线程）**：按 10 ms 块 drain → 组帧 → 广播给所有订阅者。

### 6.2 帧格式

二进制帧，小端：

```
[epoch:u32][start_frame:u64][channels:u16][samples:f32 × channels × blockFrames]
```

- `epoch`：**流代数**。播放器重建（任何编辑）会让 `AudioHost` 重建，旧流的帧必须作废；
  客户端看到 epoch 变化就清空 jitter buffer 重新同步。
- `start_frame`：该块第一个采样对应的时间轴位置（工程采样率下的帧号）。客户端据此把
  "已消费采样数"换算成播放头位置，不依赖服务器时间戳。

### 6.3 客户端播放

- 用 `AudioBufferSourceNode` 按 `nextStartTime` 连续排队，预排 3–5 块（30–50 ms）形成 jitter buffer。
- 欠载：静音并进入"重同步"，界面显示缓冲状态（不允许静默卡顿）。
- 过载（缓冲超过 200 ms）：丢弃最老的块，按同样方式重新对齐。

### 6.4 时钟与漂移

唯一权威是**采样计数**：

```
位置 = start_frame + 客户端已消费帧数
```

服务端 30 Hz 的 `engine://transport` 事件只用于**漂移校正**：偏差 > 2 帧时按 1 帧/块逐步拉回，
禁止硬跳（硬跳会听出咔哒）。漂移来源是两块晶振（10–50 ppm），5 分钟约几毫秒，这个控制器足够。

### 6.5 为什么是 HTTP + `AudioBufferSourceNode`，不是 HTTPS + `AudioWorklet`

| | 甲：HTTP + `AudioBufferSource`（**采纳**） | 乙：HTTPS + `AudioWorklet` |
| --- | --- | --- |
| 证书 | 不需要 | 自签证书，平板/iOS 要手动信任一次 |
| 延迟 | 45–60 ms | 40–55 ms |
| 录音 / Web MIDI | 不可用（非 secure context 没有 `getUserMedia`） | 可用 |
| 复杂度 | 低 | 中（worklet 消息通道 + 打包） |

`AudioWorklet` 只在 secure context 可用，而 `http://192.168.x.x` 不是 secure context —— 也就是说
选乙等于强制给局域网配 TLS。**甲先上**，`--tls` 作为可选项预留；协议（§6.2）与播放器接口不变，
将来打开 TLS 只是把 `AudioBufferSource` 换成 worklet 实现。

### 6.6 带宽与多客户端

48 kHz 立体声 f32 = 384 KB/s/客户端；编成 i16 则 192 KB/s/客户端。4 个客户端约 6 Mbps，WiFi 可接受。
**单播**到每个订阅者，不做组播、不做服务端混音。

### 6.7 已知风险

- **iOS Safari 会挂起 `AudioContext`**（后台、锁屏、来电）：必须处理 `visibilitychange` / `pagehide`，
  回来时 `resume()` 并重同步。这是浏览器端最常见的"突然没声"。
- **WiFi 抖动**：jitter buffer 欠载比延迟爆炸好，界面要有可见指示（类似电平表旁边的缓冲灯）。

## 7. 前端传输层

```ts
interface Channel {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  subscribe(event: string, cb: (payload: unknown) => void): () => void;
  audio(): AudioLink | null;   // 只有 http 实现有
}
```

- `tauriChannel`：现有 `invoke` / `listen`。
- `httpChannel`：`POST /rpc/<cmd>`（JSON）+ `EventSource /events` + `WebSocket /audio`。
- 改造面：`src/api/tauri.ts` 的 237 处 `invoke<T>("cmd", {…})` 换成 `channel.invoke<T>("cmd", {…})`。
  命令名与参数名本来就和 Rust 一一对应（文件头注释的既有约定），是**纯机械替换**，可脚本化。
- 事件：`listen()` 只出现在 `tauri.ts` 的 3 个 `onXxxEvent` 函数里，改 `subscribe` 是几行的事；
  `EVENT_*` 常量不动。
- `mock.ts` 从"散落 237 个 if"变成第三个 `Channel` 实现。
- 平台差异：保留 `isTauri()`，新增 `hasNativeUi`。`App.vue` / `TopBar.vue` / `WindowControls.vue` /
  `EffectWindowView.vue` 里的 `getCurrentWindow()` 在 web 下隐藏。
- 路由已是 `createWebHashHistory`（`src/router/index.ts`），静态托管不需要 rewrite 规则。

## 8. 鉴权、文件与会话

| 项 | 决定 |
| --- | --- |
| token | 启动时生成 256-bit，落在 `data_dir/web-token`；`/rpc`、`/events`、`/audio` 全部校验。`EventSource` / `WebSocket` 带不了 header，token 走 query 参数 |
| 绑定 | 默认 `127.0.0.1`；`--lan` 才 `0.0.0.0`，启动时打印 `http://<ip>:<port>/#token=…` |
| Origin | HTTP 与 WS 握手都校验白名单，防 DNS rebinding / CSWSH（复用 `mcp.rs` 已有判断） |
| 危险操作 | 安装插件、扫描外部目录、迁移数据目录、删工程历史：web 端一律 403，提示"此操作需要桌面端" |
| 文件 | 工程仍在服务端磁盘。新增 `GET /fs/projects` **只列** `projectsDir` 下的工程，不做任意路径浏览；导出走 `GET /export/<job>` 直接下发文件 |
| 会话 | 多客户端共享同一个 `Engine`；后连的客户端靠 `get_project()` 拿全量快照。传输控制是全局的（谁都能按播放），不装多租户 |

## 9. 降级矩阵

| 能力 | 桌面 | 浏览器（甲方案） |
| --- | --- | --- |
| 实时监听 | 本机声卡 | WS PCM + jitter buffer |
| VST3 / CLAP 原生界面 | 原生窗口 | 参数表（现有"插件界面"标签）+ 不可编辑提示 |
| 系统文件对话框 | 原生 | 服务端工程列表 / 上传导入 |
| 录音、Web MIDI | `cpal` / `midir` | 禁用；开 `--tls` 后可启用 |
| 自动更新 | 有 | 无 |
| 插件扫描 / 安装 | 有 | 403 |

## 10. 错误处理与可观测性

- **断线重连**：`EventSource` 自动重连；重连成功后客户端重新 `get_project()` 全量对齐，
  并重建 transport 订阅 —— 不做事件回放（事件流不保证可重放）。
- **音频流中断**：设备切换、播放器重建都会递增 `epoch`；客户端丢弃旧 epoch 的帧并重同步。
- **命令失败**：沿用现有错误字符串协议；`/rpc` 用 HTTP 500 + `{ "error": "…" }`，前端统一走
  现有 catch 分支，不新增错误模型。
- **可观测性**：daemon 启动/绑定/鉴权失败/音频流欠载都写日志（沿用 `logging.rs` 的既有格式）；
  `/health` 返回版本、音频设备状态、订阅者数量。

## 11. 测试策略

| 层 | 测什么 |
| --- | --- |
| 单元 | `FakeHost` 下跑 239 个命令：工程结构、撤销、错误路径（现有引擎测试继续保留） |
| 集成 | daemon 起在临时端口，用 HTTP 客户端打 `/rpc`、`/events`、`/audio`；覆盖鉴权失败、错误 token、跨域拒绝、危险操作 403 |
| 音频一致性 | 同一工程离线渲染 vs tap 抓到的流，前 2 s 逐样本比对（差值 < -120 dB），证明加 tap 没改音质 |
| 时钟 | 假服务端注入抖动/丢包/epoch 变更，验证漂移控制器收敛、无爆音、断线后能重同步 |
| 手工 | iPad Safari + 笔记本 Chrome 同时连，播放中连续编辑 5 分钟：无爆音、无漂移、缓冲灯不闪红 |

## 12. 落地进度

M1 已完成（2026-11-13）：

- `src-tauri/src` 整体迁入 `crates/daemon`，桌面壳只剩 `run()` / 建窗 / 原生插件窗口 / 更新器。
- `Host` / `UiHost` 与三份实现（Tauri、Headless、Fake），`AppState` 持有 Host，
  所以**所有**命令的事件广播走同一条通道。
- `run_all` 拆成 `exec(app: Option<&AppHandle>, …)`：桌面带窗口、无窗口不带，
  两条路径共用撤销 / 事件 / 插件状态抢救。
- `boot::bootstrap` / `serve` / `web_token` + `ai-daw` 二进制 + Dockerfile / compose。
- `/rpc` 开放 31 条命令（工程 / 轨道 / 片段 / 音符 / 混音 / 传输 / 历史），
  带 Bearer 与 query token 校验、坏参数 400、未知命令 400。
- 测试：228 条无 Tauri 单测全部通过（含 `/rpc` 鉴权、坏参数、未知命令、事件广播）。

追加完成（同一轮）：

- 事件总线 `bus.rs`：`Host` 实现与 SSE 共用一条 `broadcast`，webview 与浏览器收到同一份事件。
- `GET /events`（SSE）+ `GET /` 静态托管（`--web-dir` / `AIDAW_WEB_DIR`）。
- 前端只改一处：`src/api/tauri.ts` 的 `invoke`/`listen` 换成 `src/api/ipc.ts`，237 个调用点零改动。
- 镜像瘦身：`tauri` 改为可选特性（`desktop`，默认开），无头用
  `--no-default-features` 构建 —— `tauri` 不进依赖图，WebKit/GTK 那 612 MB 不再被链进来，
  镜像从 1.07 GB 降到 **201 MB**（实测，`ldd` 里已无 webkit / gtk）。无头构建里 `crate::tauri_api` 提供空壳类型（只求编译），
  桌面构建仍是真实 `tauri`，228 条测试与桌面壳构建均不受影响。

M2 也已完成（同一轮）：

- `crates/engine/src/tap.rs`：`AudioTap`（原子环形缓冲，音频线程无锁写入，容量约 2 秒）。
- `AudioEngine` 把 tap 装进 `RealtimeOptions`，并在重建时沿用；`/audio` 用 WebSocket
  按 ~10 ms 一块推 `[epoch][start_frame][frames][channels][f32…]`。
- 传输状态 ticker 从桌面壳搬进 daemon：浏览器通过 SSE 也能收到 `engine://transport`。
- 前端 `src/api/audio.ts`：`WebMonitor` 用 `AudioBufferSourceNode` 连续排播，带预滚
  （80 ms）、断流重锚（250 ms）、epoch 切换重置、iOS `visibilitychange` 恢复。
- 实测（无头进程 + Python WS 客户端）：播放 2 秒收到 96 256 帧、峰值 0.404、epoch 稳定。

收尾（M4 的一部分）：

- `/rpc` 现在暴露 **96 个命令名**：43 个显式分支（只读 / 传输 / 设备 / 历史 / 插件列表）
  + 53 个由 `Command` 枚举通用兜底覆盖的编辑命令。
- 令牌改用系统 CSPRNG（`getrandom`），并保留哈希兜底。
- 明确不跨端的功能（降级矩阵里已写）：AI 生成、模型下载、插件管理、录音、Web MIDI。

与设计的偏差（有意为之）：

1. **Host 放在 `AppState` 里**，而不是只由 `Daemon` 持有。命令层有 239 条命令，
   逐个改签名会把 M1 变成一次大爆炸；把 Host 注入状态后，`run_all` 一处改动就让
   所有命令都能无窗口执行。`Daemon` 仍是装配与配置的唯一入口。
2. **`/rpc` 先只开放 31 条命令**，其余返回 400 `unknown command`。M1 的出口标准是
   "浏览器能读工程、加轨道"，与其一次迁完 126 条再调，不如先打通链路。

## 13. 里程碑

| 阶段 | 内容 | 出口标准 |
| --- | --- | --- |
| M1（约 1 周） | `crates/daemon` 骨架 + `Host` trait + §5.2 第 1–2 组迁移 + `/rpc` + 桌面内嵌 | `curl` 能读工程、加轨道；桌面版无回归 |
| M2（约 1 周） | PCM tap + `/audio` + 浏览器播放 + 播放头同步 | 平板戴耳机听 5 分钟不漂、实测延迟 < 80 ms |
| M3（约 1 周） | SSE 事件 + 剩余域迁移 + 前端双 Channel + 文件/导出 | 浏览器里能完整编曲 |
| M4（约 1 周） | 鉴权、多客户端、降级矩阵、打包发布 | 两台设备同时可用，文档齐 |

## 14. 风险与未决问题

1. **迁移期双入口并存的回归风险** —— 用"每域迁移完桌面版必须能跑、可发布"的硬门槛压住。
2. **iOS Safari 的 AudioContext 挂起** —— 见 §6.7，必须在 M2 就实现并实测，不能留到 M4。
3. **WiFi 欠载** —— 界面必须有可见的缓冲状态；否则用户只会看到"卡了但不报错"。
4. **未决**：浏览器端的插件参数编辑是否足够（还是必须做 Electron 式的原生窗口截图流）。
   先按"参数表 + 提示"上线，用一轮真实使用再判断。
