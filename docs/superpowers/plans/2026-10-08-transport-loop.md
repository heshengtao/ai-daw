# 循环播放按钮 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 底栏加一个「循环」按钮；开启后播放到工程结尾**无缝**回到起点继续播放，实时与离线试听两种模式行为一致。

**Architecture:** 循环是**运行时传输状态**，不进工程。`Player` 记住开关（`TransportCommand::SetLoop`），但保持「不自动停止」的契约；「到结尾回到起点」由**实时宿主回调**执行（现在那里是「到结尾就暂停」）。开关存在 `SharedTransport` 里，宿主因编辑工程而重建时自动延续；`TransportSnapshot` 把它透出给界面。

**Tech Stack:** Rust（`crates/engine`）、Tauri 2、Vue 3 + Pinia + TypeScript。

## Global Constraints

- Rust 任务的验证 = `cargo test -p ai-daw-engine --features realtime`（`src-tauri` 是独立 workspace，构建用 `cargo build --manifest-path src-tauri/Cargo.toml`）。
- 前端**没有测试框架**：验证 = `npm run build` 通过 + `npm run dev` 手动核对。
- 循环开关**不写入工程、不进入撤销历史、不持久化**。
- 循环范围 = 整首工程内容（第 0 拍 → `end_frame`），**不做** in/out 区间。
- `Player` 的「不自动停止 / 不自动回卷」契约不可破坏（离线渲染依赖它：`render()` 直接循环调用 `process_block`）。
- 注释与文案用中文；i18n 文案中英各一份。
- 提交信息格式：`feat(transport): …`。

---

## File Structure

- `crates/engine/src/transport.rs` — `TransportCommand::SetLoop`。
- `crates/engine/src/player.rs` — `looping` 字段 + `is_looping()` + 命令处理 + 测试。
- `crates/engine/src/realtime.rs` — 结尾策略、`SharedTransport.looping`、重建时初始化、可测的动作函数。
- `src-tauri/src/audio.rs` — `TransportSnapshot.loop`。
- `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs` — `transport_set_loop` 命令与注册。
- `src/api/types.ts`、`src/api/tauri.ts`、`src/api/mock.ts` — 类型与命令。
- `src/stores/transport.ts` — `loop` 状态与 `setLoop`。
- `src/components/layout/BottomBar.vue` — 循环按钮。
- `src/i18n/zh.ts`、`src/i18n/en.ts` — 文案。

---

### Task 1: `Player` 记住循环开关

**Files:**
- Modify: `crates/engine/src/transport.rs`
- Modify: `crates/engine/src/player.rs`

**Interfaces:**
- Produces:
  - `TransportCommand::SetLoop(bool)`
  - `Player::is_looping(&self) -> bool`
  - `Player` 新增私有字段 `looping: bool`（构造时 `false`）。

- [ ] **Step 1: 加命令变体**

`crates/engine/src/transport.rs` 的 `TransportCommand` 末尾（`SetParam` 之后）加：

```rust
    /// 开关循环播放：播到工程结尾无缝回到起点继续。属于运行时传输状态，不写入工程。
    SetLoop(bool),
```

- [ ] **Step 2: 写失败测试**

`crates/engine/src/player.rs` 的 `mod tests` 里加：

```rust
    /// 循环开关本身不改变播放状态与位置，只是被记下来。
    #[test]
    fn set_loop_toggles_the_flag_without_touching_playback() {
        let mut player = Player::new(schedule(), &PluginRegistry::with_builtins(), 48_000.0, 512);
        assert!(!player.is_looping(), "默认不循环");
        player.apply_command(TransportCommand::SetLoop(true));
        assert!(player.is_looping());
        assert!(!player.is_playing(), "开关不该启动播放");
        assert_eq!(player.position_frames(), 0, "开关不该移动位置");
        player.apply_command(TransportCommand::SetLoop(false));
        assert!(!player.is_looping());
    }
```

- [ ] **Step 3: 运行确认失败**

Run: `cargo test -p ai-daw-engine set_loop_toggles_the_flag_without_touching_playback`
Expected: 编译失败（`is_looping` 不存在 / `SetLoop` 不存在）

- [ ] **Step 4: 实现**

`Player` 结构体加字段（放在 `playing: bool,` 之后）：

```rust
    /// 循环播放开关。`Player` 自己不改位置；「到结尾回到起点」由宿主执行。
    looping: bool,
```

`Player::new` 的结构体字面量里加 `looping: false,`。

`apply_command` 的 `match` 里加一支（用 `_ => {}` 之外的位置均可）：

```rust
            TransportCommand::SetLoop(on) => self.looping = on,
```

在 `is_playing` 方法附近加：

```rust
    /// 当前是否开启循环播放。
    pub fn is_looping(&self) -> bool {
        self.looping
    }
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p ai-daw-engine --features realtime`
Expected: PASS（注意 `process_block`/`render` 的既有测试必须仍通过——`Player` 不再新增任何自动回卷）

- [ ] **Step 6: 提交**

```bash
git add crates/engine/src/transport.rs crates/engine/src/player.rs
git commit -m "feat(transport): Player remembers the loop toggle"
```

---

### Task 2: 实时宿主按开关决定「回卷 / 暂停」

**Files:**
- Modify: `crates/engine/src/realtime.rs`

**Interfaces:**
- Consumes: Task 1 的 `Player::is_looping()` / `TransportCommand::SetLoop`。
- Produces:
  - `SharedTransport::is_looping(&self) -> bool`（含私有 `Arc<AtomicBool>`）
  - `end_of_project_action(looping: bool) -> TransportCommand`（`pub(crate)`，可测）

- [ ] **Step 1: 写失败测试（可测的决策函数）**

`crates/engine/src/realtime.rs` 的 `mod tests` 里加：

```rust
    /// 到达工程结尾的动作：开循环回到起点，关循环暂停。
    #[test]
    fn end_of_project_action_wraps_or_pauses() {
        assert_eq!(end_of_project_action(true), TransportCommand::Seek { frame: 0 });
        assert_eq!(end_of_project_action(false), TransportCommand::Pause);
    }

    /// 共享状态的循环开关初始为关，且克隆共享同一份。
    #[test]
    fn shared_transport_loop_flag() {
        let shared = SharedTransport::new();
        assert!(!shared.is_looping());
        shared.looping.store(true, Ordering::Relaxed);
        assert!(shared.clone().is_looping());
    }
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test -p ai-daw-engine end_of_project_action_wraps_or_pauses`
Expected: 编译失败（函数/方法不存在）

- [ ] **Step 3: 加共享字段与访问器**

`SharedTransport` 结构体加字段：

```rust
    looping: Arc<AtomicBool>,
```

加访问器（放在 `is_playing` 之后）：

```rust
    /// 音频线程最近一次报告的循环开关。
    pub fn is_looping(&self) -> bool {
        self.looping.load(Ordering::Relaxed)
    }
```

（`#[derive(Default)]` 已能覆盖 `Arc<AtomicBool>`；`new()` 走 `Self::default()`，无需改。）

- [ ] **Step 4: 加决策函数**

在 `write_interleaved` 附近加：

```rust
/// 到达工程结尾时的动作：开循环 → 无缝回到起点继续；否则 → 暂停（现状）。
pub(crate) fn end_of_project_action(looping: bool) -> TransportCommand {
    if looping {
        TransportCommand::Seek { frame: 0 }
    } else {
        TransportCommand::Pause
    }
}
```

- [ ] **Step 5: 接进宿主**

在 `AudioHost::start_with` 里：

1. 读出初始开关并同步到播放器（在 `let mut player = Player::new(...)` 之后）：

```rust
        player.apply_command(TransportCommand::SetLoop(shared.is_looping()));
```

2. 加一个回写原子量（在 `let playing_cb = Arc::clone(&shared.playing);` 之后）：

```rust
        let looping_cb = Arc::clone(&shared.looping);
```

3. 把回调里现有的结尾策略整段替换为：

```rust
                    // 宿主策略：播到工程结尾。开循环 → 无缝回起点继续；否则暂停。
                    if player.end_frame() > 0 && player.position_frames() >= player.end_frame() {
                        player.apply_command(end_of_project_action(player.is_looping()));
                    }
```

4. 在回调里、`playing_cb.store(...)` 之后加：

```rust
                    looping_cb.store(player.is_looping(), Ordering::Relaxed);
```

- [ ] **Step 6: 运行测试**

Run: `cargo test -p ai-daw-engine --features realtime`
Expected: PASS（含 `start_returns_result_without_panicking` 在无设备环境仍返回 Result）

- [ ] **Step 7: 提交**

```bash
git add crates/engine/src/realtime.rs
git commit -m "feat(transport): loop wraps seamlessly at project end"
```

---

### Task 3: 快照透出 + Tauri 命令

**Files:**
- Modify: `src-tauri/src/audio.rs`（`TransportSnapshot`、`snapshot()`）
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs`（`generate_handler!`）

**Interfaces:**
- Consumes: Task 2 的 `SharedTransport::is_looping()`、Task 1 的 `TransportCommand::SetLoop`。
- Produces: `transport_set_loop(state, on: bool) -> Result<(), String>`；JSON 快照字段 `loop: boolean`。

- [ ] **Step 1: 快照加字段**

`src-tauri/src/audio.rs` 的 `TransportSnapshot` 加：

```rust
    /// 是否开启循环播放。
    #[serde(rename = "loop")]
    pub loop_: bool,
```

`AudioEngine::snapshot` 里填：

```rust
            loop_: available && self.shared.is_looping(),
```

- [ ] **Step 2: 加命令**

`src-tauri/src/commands.rs` 的传输区（`transport_seek` 附近）加：

```rust
/// 开关循环播放：播到工程结尾无缝回到起点继续。
#[tauri::command]
pub fn transport_set_loop(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    state.audio.send(TransportCommand::SetLoop(on))
}
```

- [ ] **Step 3: 注册命令**

`src-tauri/src/lib.rs` 的 `generate_handler!` 里，在 `commands::transport_seek,` 之后加：

```rust
            commands::transport_set_loop,
```

- [ ] **Step 4: 构建**

Run: `cargo build --manifest-path src-tauri/Cargo.toml`
Expected: 成功

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/audio.rs src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(transport): expose loop state and setter over IPC"
```

---

### Task 4: 前端循环按钮

**Files:**
- Modify: `src/api/types.ts`
- Modify: `src/api/tauri.ts`
- Modify: `src/api/mock.ts`
- Modify: `src/stores/transport.ts`
- Modify: `src/components/layout/BottomBar.vue`
- Modify: `src/i18n/zh.ts`、`src/i18n/en.ts`

**Interfaces:**
- Consumes: Task 3 的 `transport_set_loop` 命令与快照字段。
- Produces: `useTransportStore().loop` / `setLoop(on)`；底栏循环按钮。

- [ ] **Step 1: 类型**

`src/api/types.ts` 的 `TransportSnapshot` 加：

```ts
  /** 是否开启循环播放。 */
  loop: boolean;
```

- [ ] **Step 2: API**

`src/api/tauri.ts` 加：

```ts
/** 开关循环播放。 */
export function transportSetLoop(on: boolean): Promise<void> {
  return mockEnabled() ? mockApi.transportSetLoop(on) : invoke("transport_set_loop", { on });
}
```

- [ ] **Step 3: mock 对齐**

`src/api/mock.ts`：

- `const transport: TransportSnapshot = { … }` 里加 `loop: false,`。
- `tick()` 里把「到达结尾即停」那段改为：开循环则回绕，否则维持现状：

```ts
  if (transport.playing) {
    transport.position_frames += Math.round(dt * transport.sample_rate);
    const end = projectEndFrames();
    if (end > 0 && transport.position_frames >= end) {
      if (transport.loop) {
        transport.position_frames %= end;
      } else {
        transport.position_frames = end;
        transport.playing = false;
      }
    }
  }
```

- 加一个方法（放在 `transportSeek` 附近）：

```ts
  transportSetLoop: (on: boolean): void => {
    transport.loop = on;
    emitTransport();
  },
```

- [ ] **Step 4: store**

`src/stores/transport.ts`：

- 加 `const loop = ref(false);`
- `applySnapshot` 里（在 `positionFrames` 之后）加：`if (snapshot.available) loop.value = snapshot.loop;`
- `startPreview` 里创建元素后、`await el.play()` 之前加 `el.loop = loop.value;`
- 加动作：

```ts
  /** 开关循环播放。 */
  async function setLoop(on: boolean): Promise<void> {
    loop.value = on;
    if (mode.value === "realtime") {
      try {
        await api.transportSetLoop(on);
      } catch {
        // 实时不可用时保持界面状态即可；下次快照会纠正。
      }
      return;
    }
    if (audio) audio.loop = on;
  }
```

- 在 store 的 return 里导出 `loop` 与 `setLoop`。

- [ ] **Step 5: 底栏按钮**

`src/components/layout/BottomBar.vue`：从 `@heroicons/vue/24/outline` 引入 `ArrowPathIcon`，在中间 `.core` 组的 Stop 按钮之后加：

```html
      <Tooltip :text="i18n.t('bottom.loop')" place="above" align="center">
        <button
          class="t-btn"
          :class="{ on: transport.loop }"
          :aria-pressed="transport.loop"
          :aria-label="i18n.t('bottom.loopLabel')"
          @click="transport.setLoop(!transport.loop)"
        >
          <ArrowPathIcon class="w-[17px] h-[17px]" />
        </button>
      </Tooltip>
```

在 `<style scoped>` 里给 `.t-btn.on` 加激活态：

```css
.t-btn.on {
  background-color: rgb(var(--md-secondary-container));
  color: rgb(var(--md-on-secondary-container));
}
```

- [ ] **Step 6: 文案**

`src/i18n/zh.ts`：`"bottom.loop": "循环", "bottom.loopLabel": "循环播放"`；
`src/i18n/en.ts`：`"bottom.loop": "Loop", "bottom.loopLabel": "Toggle loop playback"`。

- [ ] **Step 7: 类型检查 + 构建**

Run: `npm run build`
Expected: 成功

- [ ] **Step 8: 手动核对**

Run: `npm run dev`（浏览器预览走离线试听路径）：
Expected: 点循环按钮高亮；播放到结尾后自动从头继续（无停顿）。
再 Run: `npm run tauri dev`（桌面端实时路径）：同样能循环；循环中编辑工程（改个音符）后开关仍保持。

- [ ] **Step 9: 提交**

```bash
git add src/api/types.ts src/api/tauri.ts src/api/mock.ts src/stores/transport.ts src/components/layout/BottomBar.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(transport): loop button in the bottom bar"
```

---

## Self-Review

**Spec coverage（对照 `2026-10-08-transport-loop-design.md`）**
- 3.1 `TransportCommand::SetLoop` → Task 1。
- 3.2 `Player.looping` / `is_looping` → Task 1。
- 3.3 实时宿主回卷策略 → Task 2。
- 3.4 跨重建保留（`SharedTransport.looping` + 启动时初始化）→ Task 2 Step 3/5。
- 3.5 快照字段 → Task 3。
- 4 Tauri 命令 → Task 3。
- 5 前端（types/api/mock/store/按钮/i18n）→ Task 4。
- 预览模式 `audio.loop` → Task 4 Step 4。

**Placeholder 扫描**：无 TBD/TODO；代码均为具体实现。

**类型一致性**：Rust 侧字段名 `loop_` + serde rename `loop`（Task 3）与 TS 侧 `loop: boolean`（Task 4 Step 1）对应；`setLoop` 在 store（Task 4 Step 4）与按钮（Step 5）签名一致；`end_of_project_action` 在 Task 2 Step 4 定义、Step 5 使用。

**风险提示**：`Seek { frame: 0 }` 会重置插件，循环点的长音会被截断重起（预期取舍，避免挂音）。若日后要更「无缝」，再单独引入「不重置插件的回卷」。

---

## 执行记录（随进度更新）

### 全部完成（4/4）

| 任务 | 提交 | 内容 |
| --- | --- | --- |
| 1 | `d127b37` | `TransportCommand::SetLoop(bool)`；`Player.looping` + `is_looping()`；命令处理 |
| 2 | `c96fb59` | `SharedTransport.looping` + `is_looping()`；`end_of_project_action`；`start_with` 延续开关、回调按开关回卷/暂停 |
| 3 | `a72d47a` | `TransportSnapshot.loop`（serde rename）；`transport_set_loop` 命令并注册 |
| 4 | `56cc37a` | `loop` 类型/命令/mock（回绕）/store（预览 `audio.loop`）/底栏循环按钮 + i18n |

**验证**：`cargo test -p ai-daw-engine --features realtime` → 123 通过（含 `set_loop_toggles_the_flag_without_touching_playback`、`end_of_project_action_wraps_or_pauses`、`shared_transport_loop_flag`）；`cargo check --manifest-path src-tauri/Cargo.toml` 通过；`npm run build` 通过；词典中英 422 键一致。

**说明**：
- 循环开关**不写入工程、不进撤销历史、不持久化**（符合 Global Constraints）；编辑工程导致宿主重建时，`SharedTransport.looping` 会把它带回新 `Player`，界面因此不必重新订阅。
- 实时路径下的「循环中编辑后仍保持循环」由上面的重建延续保证；离线试听路径由 `<audio>.loop` 承担。两者都需要人工肉眼/真机确认（计划 Step 8），本次未做设备级验证。

