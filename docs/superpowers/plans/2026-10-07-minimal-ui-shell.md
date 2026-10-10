# AI DAW 最小桌面外壳实现计划（Plan 4a）

> **状态（2026-10-08）**：✅ **已落地，但实现栈与本文不同**——最终采用 **Vue 3 + Pinia + Vue Router + Tailwind（Material You token 适配自 Exameow）**，而非本文所写的 React 18 + `@m3e/react`；文件布局亦不同（`src/main.ts`、`*.vue`、`src/api/`、`src/stores/`、`src/composables/`）。功能范围（窗口、工程操作、离线渲染试听、导出 WAV）均已实现。
> 「原生实时传输」属 Plan 4b，**✅ 已完成**（见 `../ROADMAP.md` §5 Stage A / A2）。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一个能启动的桌面应用：Tauri 窗口 + MD3 Expressive 界面（顶栏 / 传输条 / 轨道列表 / 活动列表），能操作工程（改速度、加轨道、载入示例）、**渲染试听**、**导出 WAV**。

**Architecture:** Tauri v2 后端进程内持有 `Engine`（`Mutex<Engine>` 托管状态），前端 React + `@m3e/react` 通过 `invoke` 调命令、通过 `listen` 收事件流。**试听采用"离线渲染 → 前端播放"**：后端把工程渲染成 WAV 字节返回，前端用 Blob URL 播放。理由：`cpal::Stream` 在部分平台是 `!Send`，无法直接放进 Tauri 托管状态；用离线渲染可以绕开线程架构问题，让 4a 尽快"看得见、听得到"。**原生实时传输（走 `AudioHost` + 专用音频线程）留到 4b**。

**Tech Stack:** Tauri v2、Vite、React 18 + TypeScript、`@m3e/react`（Material 3 Expressive Web Components 的 React 绑定）、`@tauri-apps/api` v2。

## Global Constraints

- 网络：本机代理在 **7892** 端口。npm/cargo 操作前先 `export HTTPS_PROXY=http://127.0.0.1:7892 HTTP_PROXY=http://127.0.0.1:7892`；cargo 另加 `export CARGO_HTTP_PROXY=http://127.0.0.1:7892`。
- `src-tauri/` **不并入根 Cargo workspace**：其 `Cargo.toml` 内自带空的 `[workspace]` 表（避免 `tauri::generate_context!()` 因前端 `dist/` 未构建而让 `cargo test --workspace` 失败）。
- 因此 `cargo test --workspace` 仍只覆盖 `crates/*`，必须保持全绿、无 warning；Tauri 应用通过 `npm run tauri dev` 验证。
- 后端 Rust：`edition = "2021"`、`license = "Apache-2.0"`、`#![forbid(unsafe_code)]`（`src-tauri` 同）。
- 前端 TypeScript 严格模式；组件与函数加 `///`/JSDoc 注释（沿用"接口可暴露"原则）。
- 应用标识 `identifier` 用 `com.aidaw.app`，`productName` 用 `AI DAW`。
- 每个任务末尾：`cargo test --workspace` 全绿；涉及前端时 `npm run build`（tsc + vite build）通过。
- 提交信息前缀 `feat:` / `fix:` / `chore:`，结尾固定带：
  `Co-authored-by: CommandCodeBot <noreply@commandcode.ai>`

## 文件结构

- `src-tauri/Cargo.toml`、`src-tauri/build.rs`、`src-tauri/tauri.conf.json`、`src-tauri/capabilities/default.json`、`src-tauri/icons/` — Tauri 应用（由官方脚手架生成后改名）。
- `src-tauri/src/main.rs` — 入口（调用 `lib::run`）。
- `src-tauri/src/lib.rs` — `run()`，注册状态与命令。
- `src-tauri/src/state.rs` — `AppState { engine: Mutex<Engine> }`。
- `src-tauri/src/commands.rs` — 全部 `#[tauri::command]`。
- `src/` — 前端：`main.tsx`、`App.tsx`、`lib/api.ts`、`lib/types.ts`、`components/AppShell.tsx`、`components/TransportBar.tsx`、`components/TrackList.tsx`、`components/ActivityFeed.tsx`、`theme.css`。
- `index.html`、`package.json`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`。
- 根 `.gitignore` 追加 `dist/`、`src-tauri/target/`、`node_modules/`（后两项可能已存在）。

---

### Task 1: Tauri v2 + React/TS 脚手架并入仓库，窗口能起来

**Files:**
- Create: `src-tauri/**`、`src/**`、`index.html`、`package.json`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`（由脚手架生成后调整）
- Modify: `.gitignore`

**Interfaces:**
- Consumes: 无。
- Produces: 可运行的 Tauri 应用骨架（默认模板界面），`npm run tauri dev` 能打开窗口。

- [ ] **Step 1: 用官方脚手架生成模板（临时目录）**

```bash
export HTTPS_PROXY=http://127.0.0.1:7892 HTTP_PROXY=http://127.0.0.1:7892 CARGO_HTTP_PROXY=http://127.0.0.1:7892
cd "$(mktemp -d)" && npm create tauri-app@latest ai-daw-ui -- --template react-ts
```

可选：若 npm 的额外 `--` 形式不生效，改用 `npx create-tauri-app@latest ai-daw-ui --template react-ts`。

- [ ] **Step 2: 把模板并入仓库根**

把生成目录里的这些内容复制到仓库根（`/Users/ailm32442/Documents/github/ai-daw`）：`src/`、`src-tauri/`、`index.html`、`package.json`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`（若模板有 `public/` 也一并复制）。

然后修改：
- `src-tauri/Cargo.toml`：把 `name` 改为 `ai-daw-app`；在文件末尾追加

```toml
[workspace]
```
（空 workspace 表，令其脱离根 workspace。）
- `src-tauri/tauri.conf.json`：`productName` → `"AI DAW"`，`identifier` → `"com.aidaw.app"`。
- `package.json`：把 `"name"` 改为 `"ai-daw"`。
- 根 `.gitignore` 追加：

```
node_modules/
dist/
src-tauri/target/
src-tauri/gen/
```

- [ ] **Step 3: 安装依赖并确认窗口能打开**

```bash
export HTTPS_PROXY=http://127.0.0.1:7892 HTTP_PROXY=http://127.0.0.1:7892 CARGO_HTTP_PROXY=http://127.0.0.1:7892
npm install
npm run tauri dev
```
Expected: 编译通过并弹出一个显示模板页面的原生窗口。确认后关掉窗口（Ctrl-C 结束 dev）。

- [ ] **Step 4: 确认核心 workspace 未被影响并提交**

Run: `cargo test --workspace`
Expected: PASS（仍只构建 `crates/*`；`src-tauri` 已脱离 workspace，不参与）。

```bash
git add -A
git commit -F - <<'EOF'
feat: scaffold Tauri v2 + React shell as src-tauri

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 2: 引擎状态、命令层与事件广播

**Files:**
- Modify: `crates/engine/src/event.rs`（给 `Actor` / `Event` 加 `serde` 派生）、`crates/engine/src/lib.rs`（如需要）
- Create: `src-tauri/src/state.rs`、`src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`、`src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `ai_daw_engine::{bus::Engine, command::Command, event::{Actor, Event}, model::{Project, Note}}`。
- Produces: `AppState`；Tauri 命令 `get_project`、`add_track`、`set_tempo`、`load_demo`、`clear_project`；事件名常量 `EVENT_ENGINE`（`"engine://event"`）。

- [ ] **Step 1: 给引擎事件加 serde 派生（核心 crate，含测试）**

`crates/engine/src/event.rs`：给 `Actor` 与 `Event` 的 `#[derive(...)]` 加上 `serde::Serialize, serde::Deserialize`（`TrackId`/`ClipId`/`Note` 已有 serde）。在测试模块追加：

```rust
    #[test]
    fn event_roundtrips_through_json() {
        let e = Event::TrackAdded { actor: Actor::Agent("claude".into()), id: crate::model::TrackId(3), name: "Bass".into() };
        let json = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
```

Run: `cargo test -p ai-daw-engine event::`
Expected: PASS。

- [ ] **Step 2: 加依赖并写状态**

`src-tauri/Cargo.toml` 的 `[dependencies]` 增加：

```toml
ai-daw-engine = { path = "../crates/engine" }
serde_json = "1"
```

`src-tauri/src/state.rs`：

```rust
use std::sync::Mutex;

use ai_daw_engine::bus::Engine;

/// 应用共享状态：进程内持有唯一的引擎实例。
pub struct AppState {
    pub engine: Mutex<Engine>,
}

impl AppState {
    pub fn new(name: &str) -> Self {
        AppState { engine: Mutex::new(Engine::new(name)) }
    }
}

/// 引擎事件广播的前端事件名。
pub const EVENT_ENGINE: &str = "engine://event";
```

`src-tauri/src/commands.rs`：

```rust
use tauri::{Emitter, State};

use ai_daw_engine::bus::Engine;
use ai_daw_engine::command::Command;
use ai_daw_engine::event::{Actor, Event};
use ai_daw_engine::model::{Note, Project};

use crate::state::{AppState, EVENT_ENGINE};

/// 读取当前工程快照。
#[tauri::command]
pub fn get_project(state: State<'_, AppState>) -> Project {
    state.engine.lock().unwrap().project().clone()
}

fn run(app: &tauri::AppHandle, state: &State<'_, AppState>, command: Command) -> Project {
    let mut engine = state.engine.lock().unwrap();
    let events: Vec<Event> = engine.execute(command, Actor::Human);
    let project = engine.project().clone();
    drop(engine);
    for event in events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    project
}

/// 新增一条轨道。
#[tauri::command]
pub fn add_track(app: tauri::AppHandle, state: State<'_, AppState>, name: String) -> Project {
    run(&app, &state, Command::AddTrack { name })
}

/// 设置速度（BPM）。
#[tauri::command]
pub fn set_tempo(app: tauri::AppHandle, state: State<'_, AppState>, bpm: f64) -> Project {
    run(&app, &state, Command::SetTempo(bpm))
}

/// 清空为新工程。
#[tauri::command]
pub fn clear_project(state: State<'_, AppState>) -> Project {
    *state.engine.lock().unwrap() = Engine::new("untitled");
    state.engine.lock().unwrap().project().clone()
}

/// 载入内置示例（C 大调琶音），让界面立刻有内容可看可听。
#[tauri::command]
pub fn load_demo(app: tauri::AppHandle, state: State<'_, AppState>) -> Project {
    *state.engine.lock().unwrap() = Engine::new("demo");
    {
        let mut engine = state.engine.lock().unwrap();
        let events = engine.execute(Command::AddTrack { name: "Lead".into() }, Actor::Human);
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!(),
        };
        engine.execute(Command::SetTrackInstrument { track, plugin_id: "builtin.synth".into() }, Actor::Human);
        let events = engine.execute(Command::AddClip { track, start: 0.0, length: 4.0 }, Actor::Human);
        let clip = match events[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!(),
        };
        for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 60, 64].into_iter().enumerate() {
            engine.execute(
                Command::AddNote {
                    track,
                    clip,
                    note: Note { pitch, start: i as f64 * 0.5, duration: 0.5, velocity: 100 },
                },
                Actor::Human,
            );
        }
    }
    state.engine.lock().unwrap().project().clone()
}
```

注意：`clear_project` 与 `load_demo` 各只锁一次，不要在同一作用域重复 `lock()`。`clear_project` 用一次 `let mut engine = state.engine.lock().unwrap();` 覆盖为新引擎后返回 `engine.project().clone()`；`load_demo` 同样用一次局部 `let mut engine = state.engine.lock().unwrap();` 完成"重置 + 建示例轨道/片段/音符"，末尾返回 `engine.project().clone()`（上面给出的 `load_demo` 代码里那两次 `lock()` 请合并为一次）。

`src-tauri/src/lib.rs`：

```rust
mod commands;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::new("untitled"))
        .invoke_handler(tauri::generate_handler![
            commands::get_project,
            commands::add_track,
            commands::set_tempo,
            commands::clear_project,
            commands::load_demo,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ai-daw");
}
```

- [ ] **Step 3: 编译验证**

Run: `cd src-tauri && cargo check`（先 `export CARGO_HTTP_PROXY=...`）
Expected: 通过，无 warning。

Run: `cargo test --workspace`（仓库根）
Expected: PASS（含 Step 1 的新测试）。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add engine state, command layer and event broadcast

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 3: 前端 API 层与 MD3 外壳（顶栏 + 传输条）

**Files:**
- Create: `src/lib/types.ts`、`src/lib/api.ts`、`src/components/AppShell.tsx`、`src/components/TransportBar.tsx`、`src/theme.css`
- Modify: `src/App.tsx`、`src/main.tsx`、`package.json`（加 `@m3e/react`、`@m3e/web`、`@m3e/icons`）

**Interfaces:**
- Consumes: Task 2 的命令与事件名。
- Produces: `api.ts` 的类型化封装：`getProject`、`addTrack`、`setTempo`、`clearProject`、`loadDemo`；`Project`/`Note`/`Track` 的 TS 类型。

- [ ] **Step 1: 装 m3e 依赖**

```bash
export HTTPS_PROXY=http://127.0.0.1:7892 HTTP_PROXY=http://127.0.0.1:7892
npm install @m3e/react @m3e/web @m3e/icons lit @lit/react
```

（`@m3e/react` v2.9.x 的 peerDependencies 是 `react >=18`、`@m3e/web ^2.9`、`lit ^3.3`、`@lit/react ^1.0`；脚手架给的是 React 19，满足要求。装完可直接读 `node_modules/@m3e/react/dist/src/<component>/index.d.ts` 确认各子路径的确切导出名——比猜名字可靠。）

- [ ] **Step 2: 类型与 API 封装**

`src/lib/types.ts`：

```ts
/** 与引擎 `model` 对应的前端视图类型。 */
export interface Note { pitch: number; start: number; duration: number; velocity: number }
export interface Clip { id: number; start: number; length: number; notes: Note[] }
export interface MixerChannel { gain_db: number; pan: number; mute: boolean; solo: boolean }
export interface PluginInstance { plugin_id: string; params: Record<string, number>; state: number[] }
export interface Track {
  id: number;
  name: string;
  instrument: PluginInstance | null;
  clips: Clip[];
  mixer: MixerChannel;
}
export interface Transport { tempo: number; time_sig_num: number; time_sig_den: number }
export interface Project { name: string; transport: Transport; tracks: Track[]; master: MixerChannel }
```

> 注意：`TrackId`/`ClipId` 在 Rust 侧是 `u64` 元组结构体，serde 默认序列化为**裸数字**，所以 TS 侧写 `number`。若实际序列化形状不同（例如变成 `{"0":1}`），以 `get_project` 的真实返回为准校正类型。

`src/lib/api.ts`：

```ts
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Project } from "./types";

/** 引擎事件广播名，与后端 `state::EVENT_ENGINE` 保持一致。 */
export const EVENT_ENGINE = "engine://event";

/** 读取当前工程。 */
export function getProject(): Promise<Project> {
  return invoke<Project>("get_project");
}
/** 新增轨道。 */
export function addTrack(name: string): Promise<Project> {
  return invoke<Project>("add_track", { name });
}
/** 设置速度（BPM）。 */
export function setTempo(bpm: number): Promise<Project> {
  return invoke<Project>("set_tempo", { bpm });
}
/** 清空工程。 */
export function clearProject(): Promise<Project> {
  return invoke<Project>("clear_project");
}
/** 载入示例工程。 */
export function loadDemo(): Promise<Project> {
  return invoke<Project>("load_demo");
}
/** 订阅引擎事件流。 */
export function onEngineEvent(handler: (event: unknown) => void): Promise<UnlistenFn> {
  return listen(EVENT_ENGINE, (e) => handler(e.payload));
}
```

- [ ] **Step 3: MD3 主题与外壳组件**

`src/theme.css`（M3 动态配色的最小可用 token 集，后续可换成 m3e 的动态色）：

```css
:root {
  --md-sys-color-surface: #141218;
  --md-sys-color-on-surface: #e6e0e9;
  --md-sys-color-surface-container: #211f26;
  --md-sys-color-primary: #d0bcff;
  --md-sys-color-on-primary: #381e72;
  --md-sys-shape-corner-large: 16px;
  --md-sys-typescale-title-medium-size: 16px;
}
body { margin: 0; background: var(--md-sys-color-surface); color: var(--md-sys-color-on-surface);
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
```

`src/components/AppShell.tsx`：顶栏（应用名 + 清空/示例按钮）+ `<main>` 插槽，`<TransportBar>` 与 `<TrackList>` 由 `App.tsx` 组合。按钮用 `@m3e/react/button` 的 `M3eButton`：

```tsx
import { M3eButton } from "@m3e/react/button";

export interface AppShellProps {
  onLoadDemo: () => void;
  onClear: () => void;
  children: React.ReactNode;
}

export function AppShell({ onLoadDemo, onClear, children }: AppShellProps) {
  return (
    <div className="app-shell">
      <header className="app-bar">
        <h1>AI DAW</h1>
        <div className="app-bar-actions">
          <M3eButton onClick={onLoadDemo}>Load demo</M3eButton>
          <M3eButton onClick={onClear}>New</M3eButton>
        </div>
      </header>
      <main>{children}</main>
    </div>
  );
}
```

> 若 `@m3e/react/button` 的导出名/子路径与实际不符，以 `matraic.github.io/m3e` 展示页为准校正（命名约定是 `@m3e/react/<name>` → `M3e<Name>`）。其他组件（slider/tabs 等）同样先到展示页确认再使用。

`src/components/TransportBar.tsx`：显示 `tempo`，提供播放/停止按钮占位（Task 5 接上行为）与一个数字输入改 tempo：

```tsx
import { M3eButton } from "@m3e/react/button";

export interface TransportBarProps {
  tempo: number;
  onTempoChange: (bpm: number) => void;
  onPlay: () => void;
  onStop: () => void;
  playing: boolean;
}

export function TransportBar({ tempo, onTempoChange, onPlay, onStop, playing }: TransportBarProps) {
  return (
    <section className="transport-bar">
      <M3eButton onClick={playing ? onStop : onPlay}>{playing ? "Stop" : "Play"}</M3eButton>
      <label>
        BPM
        <input
          type="number" min={20} max={300} value={tempo}
          onChange={(e) => onTempoChange(Number(e.target.value))}
        />
      </label>
    </section>
  );
}
```

`src/App.tsx`：持有 `project` 状态、挂载时 `getProject()`、订阅事件流、把命令结果写回状态。

- [ ] **Step 4: 构建并运行验证**

Run: `npm run build`（tsc + vite build 必须通过）
Run: `npm run tauri dev` → 窗口显示顶栏、传输条、BPM 输入；点 "Load demo" 后 `get_project` 返回示例轨道（此刻轨道还没渲染，Task 4 补）。

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add typed frontend api and MD3 app shell with transport bar

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 4: 轨道列表与活动列表

**Files:**
- Create: `src/components/TrackList.tsx`、`src/components/ActivityFeed.tsx`
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `types.ts` 的 `Project`/`Track`；`EVENT_ENGINE` 事件流。
- Produces: 展示轨道（名称、乐器、片段数、音符数）的列表 + "添加轨道"入口；右侧/下方"活动"列表显示引擎事件。

- [ ] **Step 1: 轨道列表**

`src/components/TrackList.tsx`：

```tsx
import { M3eButton } from "@m3e/react/button";
import type { Project } from "../lib/types";

export interface TrackListProps {
  project: Project;
  selectedTrackId: number | null;
  onSelect: (id: number) => void;
  onAddTrack: (name: string) => void;
}

export function TrackList({ project, selectedTrackId, onSelect, onAddTrack }: TrackListProps) {
  const add = () => onAddTrack(`Track ${project.tracks.length + 1}`);
  return (
    <section className="track-list">
      <header>
        <h2>Tracks</h2>
        <M3eButton onClick={add}>Add track</M3eButton>
      </header>
      {project.tracks.length === 0 ? (
        <p className="empty">No tracks yet — add one, or load the demo.</p>
      ) : (
        <ul>
          {project.tracks.map((t) => (
            <li
              key={t.id}
              className={t.id === selectedTrackId ? "selected" : ""}
              onClick={() => onSelect(t.id)}
            >
              <strong>{t.name}</strong>
              <span className="meta">
                {t.instrument ? t.instrument.plugin_id : "no instrument"} ·{" "}
                {t.clips.length} clip(s) ·{" "}
                {t.clips.reduce((n, c) => n + c.notes.length, 0)} note(s)
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
```

`src/components/ActivityFeed.tsx`：接收 `string[]` 行并倒序展示（这是"AI 操作可视化"的最小雏形）。

- [ ] **Step 2: 接到 App**

`App.tsx`：`selectedTrackId` 状态；`onEngineEvent` 把事件格式化成一行（例如 `TrackAdded{TrackId(1)}`）push 进 `activity` 数组；`onAddTrack` 调 `addTrack(name)` 并写回 `project`。

- [ ] **Step 3: 验证**

Run: `npm run build`
Run: `npm run tauri dev` → 载入示例后看到 1 条 "Lead" 轨道与 8 个音符计数；点 "Add track" 后列表新增且活动列表出现 `TrackAdded` 事件。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add track list and engine activity feed

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 5: 渲染试听（离线渲染 → 前端播放）

**Files:**
- Modify: `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`、`src/lib/api.ts`、`src/App.tsx`、`src/components/TransportBar.tsx`

**Interfaces:**
- Consumes: `ai_daw_engine::{registry::PluginRegistry, render::{render, RenderOptions}, audio::AudioBuffer, wav}`。
- Produces: Tauri 命令 `render_preview(sampleRate: u32) -> tauri::ipc::Response`（返回 WAV 字节）；前端 `renderPreview()` 返回 `Blob`。

- [ ] **Step 1: 后端渲染命令（返回 WAV 字节）**

在 `crates/engine/src/wav.rs` 增加一个写进内存的辅助（含测试），避免落盘：

```rust
/// 把交错缓冲编码为 16-bit PCM WAV 字节。
pub fn wav_bytes(buffer: &AudioBuffer, sample_rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: buffer.channels() as u16,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut out, spec).expect("cursor never fails");
        for &s in buffer.data() {
            writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).expect("cursor never fails");
        }
        writer.finalize().expect("cursor never fails");
    }
    out.into_inner()
}
```

测试：写一个 demo 工程，`wav_bytes` 结果以 `RIFF` 开头且长度 > 44（header）。

在 `src-tauri/src/commands.rs` 增加：

```rust
/// 离线渲染当前工程为 WAV 字节，供前端试听。
#[tauri::command]
pub fn render_preview(state: State<'_, AppState>, sample_rate: u32) -> tauri::ipc::Response {
    let project = state.engine.lock().unwrap().project().clone();
    let registry = PluginRegistry::with_builtins();
    let options = RenderOptions { sample_rate: sample_rate as f64, tail_seconds: 0.5 };
    let buffer = render(&project, &registry, &options);
    tauri::ipc::Response::new(wav_bytes(&buffer, sample_rate))
}
```

在 `lib.rs` 的 `generate_handler!` 里注册 `commands::render_preview`。

- [ ] **Step 2: 前端播放**

`src/lib/api.ts` 增加：

```ts
/** 渲染当前工程并返回可播放的 WAV Blob。 */
export async function renderPreview(sampleRate = 44100): Promise<Blob> {
  const bytes = await invoke<ArrayBuffer>("render_preview", { sampleRate });
  return new Blob([bytes], { type: "audio/wav" });
}
```

`App.tsx`：`playing` 状态；`onPlay` 调 `renderPreview()` → `URL.createObjectURL(blob)` → 用 `HTMLAudioElement` 播放；`onended` 与 `onStop` 清理并 `revokeObjectURL`。

- [ ] **Step 3: 验证**

Run: `cargo test --workspace`（wav_bytes 测试通过）
Run: `npm run build`
Run: `npm run tauri dev` → 载入示例，点 Play 能听到琶音，点 Stop 停止。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add offline render preview with in-app playback

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 6: 导出 WAV

**Files:**
- Modify: `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`、`src/lib/api.ts`、`src/App.tsx`、`src/components/AppShell.tsx`

**Interfaces:**
- Consumes: Task 5 的渲染路径与 `ai_daw_engine::wav::write_wav`。
- Produces: Tauri 命令 `export_wav(path: String, sampleRate: u32) -> Result<String, String>`。

- [ ] **Step 1: 后端导出命令**

```rust
/// 把当前工程导出为 WAV 文件，返回写入的绝对路径。
#[tauri::command]
pub fn export_wav(state: State<'_, AppState>, path: String, sample_rate: u32) -> Result<String, String> {
    let project = state.engine.lock().unwrap().project().clone();
    let registry = PluginRegistry::with_builtins();
    let options = RenderOptions { sample_rate: sample_rate as f64, tail_seconds: 0.5 };
    let buffer = render(&project, &registry, &options);
    ai_daw_engine::wav::write_wav(&path, &buffer, sample_rate).map_err(|e| e.to_string())?;
    Ok(path)
}
```

注册进 `generate_handler!`。

- [ ] **Step 2: 前端导出**

`api.ts` 增加 `exportWav(path, sampleRate = 44100): Promise<string>`。
`AppShell.tsx` 增加一个文本框（默认 `~/Documents/ai-daw-export.wav`，用 `@tauri-apps/api/path` 的 `documentDir()` 拼默认值）与 "Export WAV" 按钮；成功后把返回路径显示在活动列表里。

- [ ] **Step 3: 验证**

Run: `npm run build`；`npm run tauri dev` → 点 Export 后对应文件出现在磁盘，且能用系统播放器打开播放。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: add wav export command and UI

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 7: MD3 Expressive 打磨与空状态

**Files:**
- Modify: `src/theme.css`、`src/components/*.tsx`、`src/App.tsx`

**Interfaces:**
- Consumes: 前 6 个任务的组件。
- Produces: 视觉打磨后的外壳（布局、动效、空状态、选中态）。

- [ ] **Step 1: 布局与动效**

`theme.css`：使用 CSS Grid 排版（顶栏 / 传输条 / 两列：轨道列表 + 活动列表）；为列表项与按钮加入 M3 风格的过渡（`transition: background-color 120ms ease, transform 120ms ease`）、选中态与 `:active` 的轻微缩放、轨道列表的空状态居中提示。补齐 `app-bar`/`transport-bar`/`track-list`/`activity-feed` 的容器样式（`--md-sys-color-surface-container` 背景 + `--md-sys-shape-corner-large` 圆角 + 内边距）。

- [ ] **Step 2: 空状态与错误展示**

`App.tsx`：命令失败时把错误消息 push 进活动列表（而不是只 `console.error`）；工程无轨道时显示引导文案（"Load demo" 或 "Add track"）。

- [ ] **Step 3: 验证**

Run: `npm run build`；`npm run tauri dev` → 启动即有干净的深色 MD3 界面；空工程显示引导；载入示例后布局不跳变；按钮有 M3 动效。

- [ ] **Step 4: 提交**

```bash
git add -A
git commit -F - <<'EOF'
feat: polish MD3 expressive layout, motion and empty states

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## 自检

**Spec 覆盖（4a 范围）：**
- "前端可用的外壳" → Task 1（窗口）+ Task 3（MD3 外壳）+ Task 4（轨道/活动列表）。
- "工程操作" → Task 2（命令层）+ Task 4（加轨道）+ Task 3（改速度）。
- "试听" → Task 5（离线渲染 + 前端播放）。
- "导出" → Task 6。
- "AI 操作可视化"的最小雏形 → Task 4 的活动列表（订阅引擎事件流；完整可视化留 Plan 7）。
- 未覆盖（显式留到 4b）：原生实时传输（`AudioHost` + 专用音频线程）、时间线/钢琴卷帘/混音台、保存/打开工程文件、文件保存对话框插件。

**类型一致性：** 前端 `types.ts` 的 `Project/Track/Clip/Note` 必须与 `ai_daw_engine::model` 的 serde 形状一致（注意 `TrackId`/`ClipId` 是裸 `number`、字段名是 snake_case）；`EVENT_ENGINE` 常量前后端一致（`"engine://event"`）；命令名与 `generate_handler!` 注册名一致。

**已知取舍：**
- 试听是"渲染后播放"，不是走原生引擎的实时传输——按下 Play 到出声之间有一段渲染延迟；这是 4a 为了绕开 `cpal::Stream` 的 `!Send` 与线程架构问题而做的取舍，**4b 用专用音频线程 + `AudioHost` 换成真正的实时传输**。
- 导出用路径文本框而非系统保存对话框（避免 `tauri-plugin-dialog` 的权限配置）；4b 换成对话框。
- `mutex` 直接 `lock().unwrap()`：命令都在 Tauri 主线程/异步任务里短临界区，够用；后续若出现并发压力再换更细的锁策略。
