# 循环播放按钮 — 设计文档

- 日期：2026-10-08
- 状态：**已完成（2026-10-10 校准）**（`SetLoop` / `SharedTransport.looping` / 底栏按钮 / i18n 均已落地；真机「无缝」仍待人工验证）
- 主题：底栏新增「循环」按钮；开启后播到工程结尾**无缝**回到起点继续播放

## 1. 背景

当前传输到工程结尾就**暂停**（`crates/engine/src/realtime.rs` 的音频回调里那段
「`position >= end_frame` → `Pause`」策略，注释写着「宿主策略：播放到工程结尾即暂停」）。
离线试听模式（`src/stores/transport.ts` 的 `<audio>`）到结尾则 `ended` → 停止并归零。

新手拿着一个 8 小节的 loop 想反复听，现在每次都要手动按播放。需要一个「循环」开关。

## 2. 目标与非目标

**目标**

- 底栏有一个**循环**按钮（MD3 图标按钮，有 `on` 状态）。
- 开启后：播到工程结尾**无缝**回到起点继续播放（中间没有静音间隙）。
- 实时播放与离线试听（浏览器预览）两种模式行为一致。

**非目标**

- 不做「循环区间 / in-out 点」——本期循环范围就是**整首工程内容**（第 0 拍 → 最后片段结束）。
- 循环开关**不写入工程、不持久化**（属于运行时传输状态，和播放位置同类）。
- 不做循环次数的预渲染/无缝拼接 DSP（真实音频的拼接点若有波形跳变本来就会有咔哒声，那是素材问题）。

## 3. 引擎改动

### 3.1 `TransportCommand`

`crates/engine/src/transport.rs` 新增：

```rust
/// 开关循环播放。属于运行时传输状态，不写入工程。
SetLoop(bool),
```

### 3.2 `Player`

- 新增字段 `looping: bool`（默认 `false`）与 `pub fn is_looping(&self) -> bool`。
- `apply_command` 处理 `TransportCommand::SetLoop(on) => self.looping = on`。
- **`Player` 自身仍然不会自动停止或自动回卷**（保持现有契约，离线渲染不受影响）；循环的「到点回到起点」由宿主执行（见 3.3）。

### 3.3 实时宿主策略（`realtime.rs`）

音频回调里把现有的结尾策略改成二选一：

```rust
if player.end_frame() > 0 && player.position_frames() >= player.end_frame() {
    if player.is_looping() {
        player.apply_command(TransportCommand::Seek { frame: 0 });   // 无缝回到起点，保持播放
    } else {
        player.apply_command(TransportCommand::Pause);               // 现状
    }
}
```

- `Seek` **不改变** `playing`，所以回到起点后会继续播；包裹发生在**块边界**，因此两遍之间没有静音间隙。
- `Seek` 会重置各插件（清空悬挂声部），循环点处不会留下卡住的音；代价是跨越循环点的长音会被截断并重新起音。这是可预期的取舍（好过挂音）。

### 3.4 循环状态跨重建保留（`SharedTransport` / `audio.rs`）

编辑工程会重建宿主（`AudioMessage::Load` → 重新 `start_host`），播放器随之重建。若开关只存在播放器里就会丢。做法：

- `SharedTransport` 增加 `looping: Arc<AtomicBool>`；回调里像 `position`/`playing` 一样，每块把 `player.is_looping()` 写回该原子量。
- `AudioHost::start_with` 创建 `Player` 后，用 `shared.is_looping()` 初始化它的 `looping`。
- 于是重建后开关状态自动延续。

### 3.5 状态快照

`src-tauri/src/audio.rs` 的 `TransportSnapshot` 增加 `pub loop_: bool`（JSON 名 `loop`）；`AudioEngine::snapshot()` 填入 `self.shared.is_looping()`。

## 4. Tauri 命令

- 新增 `transport_set_loop(state, on: bool) -> Result<(), String>` → `state.audio.send(TransportCommand::SetLoop(on))`，并在 `lib.rs` 的 `generate_handler!` 注册。

## 5. 前端改动

- `src/api/types.ts`：`TransportSnapshot` 增加 `loop: boolean`。
- `src/api/tauri.ts`：新增 `transportSetLoop(on)` → invoke `transport_set_loop`。
- `src/api/mock.ts`：`transport.loop` 加入快照；`transportSetLoop` 设置它；`tick()` 在到达工程结尾时若 `loop` 为真则把 `position_frames` 回绕到 0（而不是停止）。
- `src/stores/transport.ts`：
  - `loop` ref，随快照更新（`applySnapshot`）。
  - `setLoop(on)`：实时模式调用 `api.transportSetLoop`；预览模式设置 `audio.loop = on`（并让 `startPreview` 建元素时就带上该值）。
  - 预览模式的 `onended` 在 `loop` 为真时不再触发（`<audio>.loop` 本身就不会触发 `ended`）。
- `src/components/layout/BottomBar.vue`：在中间传输组里（Stop 右侧）加一个循环图标按钮（`ArrowPathIcon`），`on` 时用主色/toggle 样式；`aria-pressed` 反映状态；无内容（工程为空）时禁用。点击 → `transport.setLoop(!transport.loop)`。
- `src/i18n/zh.ts` / `en.ts`：加 `bottom.loop` / `bottom.loopLabel` / `bottom.loopOn` 等文案。

## 6. 验收标准

1. 底栏有循环按钮，开启后高亮。
2. 开启后播放到结尾会**立即**从起点继续，听不到静音间隙。
3. 关闭后行为与现在一致（到结尾暂停 / 归零）。
4. 循环开关在编辑工程（重建宿主）后仍保持。
5. 浏览器预览（离线试听）里同样能循环。

## 7. 验证方式

- Rust：`cargo test -p ai-daw-engine`
  - `Player`：`SetLoop(true)` 后 `is_looping()`；`Seek` 不改变 `playing`；`Player` 仍不自动停止（离线渲染回归不变）。
- 前端：`npm run build`（`vue-tsc`）通过；浏览器预览里点循环、听见（看得见播放头）回绕。
- 手动（桌面端）：有内容时开循环播放，确认无缝；编辑工程后开关仍在。

## 8. 风险

- **循环点的咔哒声**：真实音频在拼接点若波形不连续会有瞬态，这是素材/包络问题，不是本特性引入的；`Seek` 重置插件已尽量避免挂音。
- **边界对齐**：包卷发生在块边界（≤ `MAX_BLOCK` 帧 ≈ 43 ms），保证无静音；不做采样级精确拼接（非目标）。
- **与动态 BPM 的关系**：`end_frame` 由分段计时算出（见动态 BPM/节拍 spec），循环点因此自动跟随拍号/速度分段，无需额外处理。
