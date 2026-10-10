# `aidaw` CLI：MCP 薄客户端（Stage F 改向后的实现计划）

- 日期：2026-10-10
- 状态：⏳ **待开工**（本文档只描述方案；代码改动留待实现轮）
- 依据：`ROADMAP.md` §5 **Stage F**（本次改向）；`skills/README.md`。
- 取代：原 Stage F 的 F1（抽 `crates/runtime` 无 Tauri 运行时）+ F2（CLI 镜像全部命令）。
  原方案降级为 `ROADMAP.md` §5 Stage F 的 **F5（搁置）**。

## 1. 目标

让任意 AI harness 能用一条命令行入口操作**正在运行的** AI DAW，并在软件没开时把它启动起来。

- **软件开着** → `aidaw` 连它的 MCP HTTP 端点（`127.0.0.1`）执行命令。
- **软件没开** → `aidaw status` 报告不在；`aidaw start` 把它启动起来，等到 MCP 就绪再继续。
- **加一个 MCP 工具 = 自动多一条可调用的 CLI 命令**（按工具名转发，无需逐工具写 CLI 代码）。

## 2. 范围与非目标

**做：**

- 新增**根** workspace 的 `aidaw` bin（MCP 薄客户端）。
- 子命令 `tools` / `call` / `status` / `start`。
- 让 CLI 能**发现**软件的 MCP 实际地址（见 §4）。

**不做（本计划明确排除）：**

- **不链接 Tauri、不内置引擎**——`aidaw` 是客户端，不是引擎。
- **不做 headless / 无 GUI 引擎**（那是 F5，已搁置）。
- **不做 stdio 传输**——GUI 进程本就在跑 HTTP MCP；stdio 只在「CLI 自己当 MCP 服务器」时才需要。
- **不镜像命令面**（不做 `aidaw set-tempo` 这类逐命令子命令）。
- **不做 F4 打包**（`bundle.externalBin` / `bundle.resources`）。

## 3. `aidaw` crate

- 位置：**根** workspace（`Cargo.toml` 的 `members` 增加，如 `crates/aidaw`），`[[bin]] name = "aidaw"`。
- 依赖（许可需符合 `deny.toml`：MIT / Apache-2.0 …）：
  - `clap`（子命令解析）。
  - MCP 客户端：复用 `rmcp` 的 `transport-streamable-http-client`（与现有服务器同一协议栈），
    或等价的轻量 HTTP 客户端（`ureq` 已在 workspace 出现过）。**实现时二选一**——
    优先 rmcp（协议一致、`tools/list` 动态），若体积/复杂度不划算再退轻量 HTTP。
  - `serde_json`（透传 JSON 参数与结果）。
- **不依赖**：`tauri`、`ai-daw-engine`、`ai-daw-app` 等任何仓库内 crate。

## 4. 发现协议（F2 前置）

**问题**：MCP 默认端口 `50562`，但 `src-tauri/src/mcp.rs` 的 `start()` 在端口被占用时会
**退回系统临时端口**；此时固定端口找不到服务器。

**方案**：软件在启动（MCP 真正起来）后，把**实际**监听地址写入应用配置目录的运行时文件；
停止时清理。

- 目录：Tauri 的 `app_config_dir()`（`identifier = com.aidaw.app`）：
  - macOS：`~/Library/Application Support/com.aidaw.app/`
  - Linux：`~/.config/com.aidaw.app/`
  - Windows：`%APPDATA%\com.aidaw.app\`
- 文件：`mcp-url`（纯文本，内容形如 `http://127.0.0.1:50562/mcp`）。
  - 与既有 `mcp.json`（**配置**）分开：这是**运行时**状态，不进版本控制。
- 写入时机：`mcp.rs` 的 `start()` 在拿到 `addr` 之后写；`stop()` 时删除。
- CLI 读取顺序：`mcp-url` → 都不行则用默认 `http://127.0.0.1:50562/mcp` 兜底。

> **app 侧改动点（**下一轮**再做）**：仅 `src-tauri/src/mcp.rs` 的 `start()` / `stop()` 各加一处写/删。
> 是本次改向里**唯一**需要动源码的地方。

## 5. 子命令规格

| 子命令 | 行为 | 输出 |
| --- | --- | --- |
| `aidaw tools` | 连 MCP，转发 `tools/list` | 工具名 + 一行描述 |
| `aidaw call <tool> [--json '<args>']` | 转发 `tools/call`，`<args>` 为 JSON 对象 | MCP 返回的文本内容 |
| `aidaw status` | 探测发现文件 / 默认端口，尝试连接 | `运行中 <url>` 或 `未运行` |
| `aidaw start` | 未运行则启动（macOS `open -a "AI DAW"`；其余平台按需），轮询直到 MCP 就绪 | `已启动 <url>` 或超时报错 |

示例：

```
$ aidaw status
未运行
$ aidaw start
已启动 http://127.0.0.1:50562/mcp
$ aidaw call get_project | head
{ "name": "untitled", "tracks": [ ... ] }
$ aidaw call set_tempo --json '{"bpm":120}'
速度已设为 120 BPM
```

`aidaw start` 的平台启动方式（macOS 用 `open -a`；Linux/Windows 按打包产物路径）在实现时确定，
必要时允许用环境变量覆盖可执行文件路径。

## 6. 验证方案

1. **软件开着**：`aidaw status` → 运行中；`aidaw tools` 列出 33 个工具；
   `aidaw call get_project` 返回工程快照；`aidaw call set_tempo --json '{"bpm":120}'` 生效且可撤销。
2. **软件没开**：`aidaw status` → 未运行；`aidaw start` 启动并等到就绪；随后 `aidaw call …` 成功。
3. **端口被占用**：先占住 `50562`（另起进程监听），启动软件 → 软件退回临时端口并写 `mcp-url`；
   `aidaw status` / `aidaw call …` 仍能通过发现文件找到它。
4. **加工具自动可调**：临时给 `mcp.rs` 加一个 `#[tool]` → `aidaw tools` 立即列出、
   `aidaw call <新工具>` 可直接调用，**CLI 侧零改动**。

## 7. 与 skills 的衔接（随 F1 一起做）

`skills/README.md` 增加「CLI 入口」一节：

1. 先 `aidaw status`；
2. 没开就 `aidaw start`；
3. 用 `aidaw call <工具> --json '{…}'` 执行；
4. 需要工具清单时 `aidaw tools`。

`aidaw_call(op, args)` 的「总控执行」由 `aidaw call` 落地（与逐工具直接调用两种入口并存）。
