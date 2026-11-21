# 灵感页：生成结果可选导入 + 提示词引导 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让灵感页生成的歌只留在灵感页（卡片上点按钮才进工程），并把风格输入框改成英文示范 + 预设风格芯片 + 纯音乐开关。

**Architecture:** 导入动作在后端——`ComposeArgs` 加 `place_in_project` 开关，`ai_yue_compose` 摆轨那段按开关走；新增 `ai_yue_import_audio` 命令负责「稍后导入」。前端只负责传 `false`、在卡片上给按钮、并且「已导入」状态从工程里是否已有该音频路径**派生**（不新增持久化字段）。提示词部分纯前端：占位符改英文、芯片把完整英文 tag 串插进输入框。

**Tech Stack:** Rust（`ai-daw-daemon`，tauri command + engine `Command`）、Vue 3 `<script setup>` + TypeScript + Pinia、MD3 原语组件（`m3-chip` / `Md3Segmented`）。

## Global Constraints

- 设计文档：`docs/superpowers/specs/2026-11-20-inspire-optional-import-design.md`（冲突时以它为准）。
- `place_in_project` 缺省必须是 `true`；AI 面板（`AiYuePage.vue`）与 MCP 一个字都不改。
- 不新增前端持久化字段，不改 record JSON 结构，不做数据迁移。
- 所有新文案同时写进 `src/i18n/zh.ts` 与 `src/i18n/en.ts`，键名以 `inspire.` 开头，中英各一条，不许只写一边。
- 注释与文案沿用现有风格：中文、解释「为什么」而不是「做了什么」。
- 不引入任何新依赖。
- **提交纪律：工作区有大量与本任务无关的未提交改动。每次提交只 `git add` 本任务列出的文件，禁止 `git add -A` / `git add .` / `git commit -a`。**
- 后端校验：`cargo test -p ai-daw-daemon --lib --no-default-features`（CI 用的就是这条；省略 `--no-default-features` 会启用 `desktop` 特性，导致 `server::tests` / `extensions::tests` 里 9 个与本次无关的用例失败）。前端类型校验：`npx vue-tsc --noEmit -p tsconfig.json`（完成后统一跑一次 `npm run build`）。

---

## File Structure

| 文件 | 职责 |
|---|---|
| `crates/daemon/src/yue_engine.rs` | 生成参数、摆轨、导入命令、单测（已有 2360 行，本次只做点状修改，不拆分） |
| `src-tauri/src/lib.rs` | tauri command 注册表（新增一行） |
| `src/api/types.ts` | `YueComposeArgs.place_in_project` |
| `src/api/tauri.ts` | `yueImportAudio` 调用封装 |
| `src/api/mock.ts` | 预览模式桩 |
| `src/stores/inspire.ts` | `placedOf` 派生、`importToProject`、`styleOf` |
| `src/components/workspace/inspire/styles.ts` | **新建**：风格预设数据（纯数据，无逻辑） |
| `src/components/workspace/inspire/InspireComposer.vue` | 占位符提示、芯片行、纯音乐开关 |
| `src/components/workspace/inspire/InspireRow.vue` | 卡片上的导入按钮 |
| `src/components/workspace/inspire/InspireLibrary.vue` | 透传 `import` 事件 + toast |
| `src/i18n/zh.ts` / `src/i18n/en.ts` | 文案 |

---

### Task 1: 后端 `place_in_project` 开关

**Files:**
- Modify: `crates/daemon/src/yue_engine.rs:903-907`（`ComposeArgs` 字段）、`:1742-1821`（摆轨段）、测试模块（`mod tests`，约 2100 行起）

**Interfaces:**
- Produces: `ComposeArgs { pub place_in_project: Option<bool> }`，`None` 等价于「摆进工程」。
- Consumes: 无（后续 Task 3 的前端字段名必须叫 `place_in_project`，snake_case 原样透传）。

- [ ] **Step 1: 写失败测试**

在 `crates/daemon/src/yue_engine.rs` 的 `mod tests` 里、`semantic_prefix_is_forwarded_as_a_file` 那个测试之后插入：

```rust
    /// 「摆不摆进工程」是个可选开关：缺省必须还是「摆」，否则 AI 面板与 MCP 会突然少一轨。
    #[test]
    fn place_in_project_defaults_to_true() {
        let args: ComposeArgs = serde_json::from_str(r#"{"style":"pop"}"#).unwrap();
        assert_eq!(args.place_in_project, None);
        assert!(args.place_in_project.unwrap_or(true));

        let args: ComposeArgs =
            serde_json::from_str(r#"{"style":"pop","place_in_project":false}"#).unwrap();
        assert_eq!(args.place_in_project, Some(false));
    }
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cargo test -p ai-daw-daemon --lib --no-default-features place_in_project_defaults_to_true
```

Expected: 编译错误 `no field place_in_project on type ComposeArgs`。

- [ ] **Step 3: 加字段**

在 `ComposeArgs` 里、`after_track` 字段之后（`/// 新轨道的名字。` 那行之前）插入：

```rust
    /// 生成完是否把音频摆进工程（默认 `true`）。
    ///
    /// 灵感界面传 `false`：先把歌留在灵感页，用户点「导入到工程」才进编排。
    /// AI 面板与 MCP 不传这个字段，行为不变。
    pub place_in_project: Option<bool>,
```

- [ ] **Step 4: 摆轨段按开关执行**

改三处，不做整段重缩进：

**4a.** 在 `let is_extension = extension.is_some();` 之后加一行：

```rust
        // 摆进工程是可选的：灵感界面只想先听歌，不想每次都在编排页多一条轨。
        // 命令照样组出来（省得这段分叉），只是不发给引擎。
        let place = args.place_in_project.unwrap_or(true);
```

**4b.** 续写分支里那行 `appended_to = Some(track_name);` 改成：

```rust
            // 只有真的摆进去了，才算「接在某条轨道后面」。
            appended_to = place.then_some(track_name);
```

**4c.** 原来的 `crate::commands::run_agent_all(app, commands);` 包一层：

```rust
        if place {
            crate::commands::run_agent_all(app, commands);
        }
```

`audio_path = Some(wav_text);` 与 `seconds = Some(render_secs as f32);` 保持在 `if` 之外——文件要落盘，界面也需要时长。

- [ ] **Step 5: 跑测试确认通过**

```bash
cargo test -p ai-daw-daemon --lib --no-default-features place_in_project_defaults_to_true
cargo test -p ai-daw-daemon --lib --no-default-features
```

Expected: 新测试 PASS，其余测试全绿（无 `warning: unused variable`）。

- [ ] **Step 6: 提交**

```bash
git add crates/daemon/src/yue_engine.rs
git commit -m "feat(yue): 生成结果是否摆进工程改成可选开关"
```

---

### Task 2: 后端 `ai_yue_import_audio`

**Files:**
- Modify: `crates/daemon/src/yue_engine.rs:203`（紧接 `ai_yue_import_abc` 之后新增函数）
- Modify: `src-tauri/src/lib.rs:812`（注册表里 `ai_yue_import_abc` 下一行）

**Interfaces:**
- Consumes: `AppState.assets.load(&str) -> Result<Decoded, String>`（`decoded.sample_rate: f64`、`decoded.frames() -> usize`）、`engine.project().transport.tempo`、`EngineCommand::AddAudioTrack { name, start, length, source, at }`、`AudioSource { path, offset_frames, frames, rate, pitch_semitones }`、`crate::commands::run_agent_all(&AppHandle, Vec<Command>)`。
- Produces: tauri 命令 `ai_yue_import_audio(app, path: String, name: Option<String>) -> Result<String, String>`，返回**最终使用的轨道名**。Task 3 的 `yueImportAudio(path, name?)` 必须与这个名字逐字对应。

- [ ] **Step 1: 实现命令**

在 `ai_yue_import_abc` 函数结束（约 203 行 `}`）之后插入：

```rust
/// 把一条已经落盘的生成音频摆进工程（灵感页卡片的「导入到工程」）。
///
/// 生成时如果带了 `place_in_project = false`，音频只落盘、不进工程；这个命令补上那一步。
/// 重复调用就是再建一条轨——同一首歌放到两个位置是合理需求，不做去重。
#[cfg_attr(feature = "desktop", tauri::command)]
pub fn ai_yue_import_audio(
    app: crate::tauri_api::AppHandle,
    path: String,
    name: Option<String>,
) -> Result<String, String> {
    let state = app.state::<Arc<AppState>>();
    let decoded = state.assets.load(&path)?;
    let sample_rate = decoded.sample_rate.max(1.0);
    let seconds = decoded.frames() as f64 / sample_rate;

    // 工程按拍记时间：这里和 `ai_yue_compose` 用同一套换算，导进来才和生成时摆的位置一致。
    let tempo = {
        let engine = state.engine.lock().unwrap();
        engine.project().transport.tempo
    };
    let beats = seconds * tempo / 60.0;

    // 录出来的文件名是 `yue2-1795187559510.wav` 这种，直接当轨道名太难认；
    // 优先用界面上那首歌的标题，没给才退回文件名。
    let track_name = name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            std::path::Path::new(&path)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "导入音频".into())
        });

    let source = AudioSource {
        path,
        offset_frames: 0,
        frames: None,
        rate: 1.0,
        pitch_semitones: 0.0,
    };
    crate::commands::run_agent_all(
        &app,
        vec![EngineCommand::AddAudioTrack {
            name: track_name.clone(),
            start: 0.0,
            length: beats.max(1.0),
            source,
            at: None,
        }],
    );
    Ok(track_name)
}
```

- [ ] **Step 2: 编译**

```bash
cargo build -p ai-daw-daemon
```

Expected: 通过。若 `app.state::<Arc<AppState>>()` 报未导入 `Manager`，参照同文件 1703 行的写法（该 trait 已在文件里 `use` 进来）。

- [ ] **Step 3: 注册命令**

`src-tauri/src/lib.rs` 里 `ai_daw_daemon::yue_engine::ai_yue_import_abc,` 那行下面加一行：

```rust
            ai_daw_daemon::yue_engine::ai_yue_import_audio,
```

- [ ] **Step 4: 编译整个工作区**

```bash
cargo build -p ai-daw
cargo test -p ai-daw-daemon --lib --no-default-features
```

Expected: 均通过（若 `-p ai-daw` 不是正确的 crate 名，用 `cargo build --workspace`）。

- [ ] **Step 5: 提交**

```bash
git add crates/daemon/src/yue_engine.rs src-tauri/src/lib.rs
git commit -m "feat(yue): 新增把生成音频导入工程的命令"
```

---

### Task 3: 前端 API 层

**Files:**
- Modify: `src/api/types.ts:1329`（`YueComposeArgs` 里 `after_track` 之后）
- Modify: `src/api/tauri.ts:1497`（`yueImportAbc` 之后）
- Modify: `src/api/mock.ts:1780`（`yueImportAbc` 之后）

**Interfaces:**
- Produces: `YueComposeArgs.place_in_project?: boolean`；`yueImportAudio(path: string, name?: string): Promise<string>`；`mockApi.yueImportAudio`。
- Consumes: Task 1 的字段名、Task 2 的命令名（`ai_yue_import_audio`）与参数名（`path` / `name`）。

- [ ] **Step 1: 加类型字段**

`src/api/types.ts` 的 `YueComposeArgs` 里，`after_track` 之后插入：

```ts
  /** 生成完是否把音频摆进工程（默认 true；灵感界面传 false，稍后手动导入）。 */
  place_in_project?: boolean;
```

- [ ] **Step 2: 加 API 调用**

`src/api/tauri.ts` 的 `yueImportAbc` 函数之后插入：

```ts
/** 把生成好的音频导入工程，新建一条音频轨；返回最终使用的轨道名。 */
export function yueImportAudio(path: string, name?: string): Promise<string> {
  if (mockEnabled()) return Promise.resolve(mockApi.yueImportAudio(path, name));
  return invoke<string>("ai_yue_import_audio", { path, name });
}
```

- [ ] **Step 3: 加 mock 桩**

`src/api/mock.ts` 里 `yueImportAbc: (_path: string): string => "已把谱面导入工程（预览模式）",` 之后插入：

```ts
  yueImportAudio: (_path: string, name?: string): string => name?.trim() || "导入音频",
```

- [ ] **Step 4: 类型校验**

```bash
npx vue-tsc --noEmit -p tsconfig.json
```

Expected: 通过（此时还没人调用新函数，`noUnusedLocals` 不会报导出函数）。

- [ ] **Step 5: 提交**

```bash
git add src/api/types.ts src/api/tauri.ts src/api/mock.ts
git commit -m "feat(api): 加 place_in_project 与 yueImportAudio"
```

---

### Task 4: 灵感页导入按钮（store 派生状态 + 卡片 + 库）

**Files:**
- Modify: `src/stores/inspire.ts:17`（import）、`:250-260`（`hueOf` 上方新增 helper）、`:582-588`（return 块）
- Modify: `src/components/workspace/inspire/InspireRow.vue:9-31`（import/props/emit）、`:86-100`（模板按钮）、样式区
- Modify: `src/components/workspace/inspire/InspireLibrary.vue:12-14`（import）、`:131-140`（传参）、脚本区（handler）
- Modify: `src/i18n/zh.ts` / `src/i18n/en.ts`

**Interfaces:**
- Consumes: `yueImportAudio(path, name?)`（Task 3）、`project.project.tracks[].clips[].audio.path`。
- Produces: store 暴露 `placedOf(item: InspireItem): boolean`、`importingId: Ref<string | null>`、`importToProject(item): Promise<string | null>`（失败返回 `null` 并把错误写进 `error`）。

- [ ] **Step 1: store 里抽出「在工程里吗」**

`src/stores/inspire.ts` 第 17 行的 import 改成：

```ts
import { onYueJobEvent, readImportFile, yueImportAudio } from "@/api/tauri";
```

在 `hueOf` 上方插入两个函数，并把 `hueOf` 内部那段查询换成 `trackIndexOf`：

```ts
  /**
   * 这在工程里的第几条轨道（没有就是 -1）。
   *
   * 列表的轨道配色、以及「导入到工程」按钮的状态都靠它——真相在工程里，
   * 不在记录里：轨道被删掉，按钮就该自己变回「未导入」。
   */
  function trackIndexOf(item: InspireItem): number {
    const tracks = project.project?.tracks ?? [];
    return tracks.findIndex((track) =>
      track.clips.some((clip) => clip.audio?.path === item.path),
    );
  }

  /** 这首歌是否已经在工程里（决定卡片按钮显示「导入」还是「已导入」）。 */
  function placedOf(item: InspireItem): boolean {
    return trackIndexOf(item) >= 0;
  }
```

`hueOf` 里原来的：

```ts
    const tracks = project.project?.tracks ?? [];
    const index = tracks.findIndex((track) =>
      track.clips.some((clip) => clip.audio?.path === item.path),
    );
```

整段替换为：

```ts
    const index = trackIndexOf(item);
```

- [ ] **Step 2: store 里加导入动作**

在 `clearError()` 之前插入：

```ts
  /** 正在导入的那一条（按钮转圈用）。 */
  const importingId = ref<string | null>(null);

  /**
   * 把这条记录导入工程（新建一条音频轨）。
   *
   * 失败时把错误写进 `error` 并返回 `null`——卡片状态不动，用户还能再点。
   */
  async function importToProject(item: InspireItem): Promise<string | null> {
    if (importingId.value) return null;
    importingId.value = item.id;
    try {
      return await yueImportAudio(item.path, item.title);
    } catch (err) {
      error.value = String(err);
      return null;
    } finally {
      importingId.value = null;
    }
  }
```

`return { ... }` 里在 `hueOf,` 之后加 `placedOf,`，在 `// 播放` 那一段之前插入：

```ts
    // 导入工程
    placedOf,
    importingId,
    importToProject,
```

（`placedOf` 已在上方列出，不要重复写两次——最终 return 里 `placedOf,` 只出现一次。）

- [ ] **Step 3: 卡片加按钮**

`InspireRow.vue`：

1. 第 10 行的图标 import 加上 `ArrowDownTrayIcon`：

```ts
import { ArrowDownTrayIcon, ArrowPathIcon, PlayIcon, TrashIcon } from "@heroicons/vue/24/outline";
```

2. props / emit 换成：

```ts
const props = defineProps<{
  item: InspireItem;
  /** 轨道色相（与编排页一致）。 */
  hue: number;
  /** 这首歌是否已经在工程里（决定按钮文案与高亮）。 */
  placed?: boolean;
  /** 正在导入这一条。 */
  busy?: boolean;
}>();

const emit = defineEmits<{
  (e: "toggle"): void;
  (e: "seek", ratio: number): void;
  (e: "remove"): void;
  (e: "import"): void;
}>();
```

3. 模板里把原来那个 `<Tooltip ...>…remove 按钮…</Tooltip>` 整块换成（顺序：导入在前、删除在后）：

```vue
        <div class="actions">
          <Tooltip :text="placed ? i18n.t('inspire.imported') : i18n.t('inspire.import')" align="end">
            <button
              class="import"
              :class="{ on: placed }"
              type="button"
              :disabled="busy"
              :aria-label="placed ? i18n.t('inspire.imported') : i18n.t('inspire.import')"
              @click="emit('import')"
            >
              <ArrowPathIcon v-if="busy" class="icon spin" />
              <ArrowDownTrayIcon v-else class="icon" />
            </button>
          </Tooltip>
          <Tooltip :text="i18n.t('inspire.removeHint')" align="end">
            <button class="remove" type="button" :aria-label="i18n.t('inspire.remove')" @click="emit('remove')">
              <TrashIcon class="icon" />
            </button>
          </Tooltip>
        </div>
```

4. 样式区给 `.remove` 之后补：

```css
/* 导入与删除并排：导入常显（它是这张卡的主要动作），删除仍然 hover 才出现。 */
.actions {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: auto;
}
.actions .remove {
  margin-left: 0;
}
.import {
  flex: 0 0 auto;
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-muted));
  cursor: pointer;
  transition: background-color var(--md-duration-short-2) var(--md-ease-standard),
    color var(--md-duration-short-2) var(--md-ease-standard);
}
.import:hover:not(:disabled) {
  background-color: rgb(var(--md-primary) / 0.12);
  color: rgb(var(--md-primary));
}
.import.on {
  color: rgb(var(--md-primary));
}
.import:disabled {
  cursor: default;
  opacity: 0.6;
}
.import:focus-visible {
  outline: 2px solid rgb(var(--md-primary));
  outline-offset: 1px;
}
```

- [ ] **Step 4: 库里接上事件**

`InspireLibrary.vue`：

1. import 段加 `useLayoutStore`：

```ts
import { useLayoutStore } from "@/stores/layout";
```

2. 脚本里加：

```ts
const layout = useLayoutStore();

/** 导入工程：成功才提示轨道名，失败由 store 写进 error 横幅。 */
async function importItem(item: InspireItem): Promise<void> {
  const name = await inspire.importToProject(item);
  if (name) layout.notify(i18n.t("inspire.importedToast", { name }));
}
```

3. `<InspireRow …>` 上补三个绑定：

```vue
          :placed="inspire.placedOf(item)"
          :busy="inspire.importingId === item.id"
          @import="importItem(item)"
```

- [ ] **Step 5: 文案**

`src/i18n/zh.ts` 在 `"inspire.style"` 附近加：

```ts
  "inspire.import": "导入到工程",
  "inspire.imported": "已导入 · 再导入一次",
  "inspire.importedToast": "已导入为轨道「{name}」",
```

`src/i18n/en.ts` 同位置加：

```ts
  "inspire.import": "Add to project",
  "inspire.imported": "Added · add again",
  "inspire.importedToast": "Added as track \"{name}\"",
```

- [ ] **Step 6: 类型校验**

```bash
npx vue-tsc --noEmit -p tsconfig.json
```

Expected: 通过。

- [ ] **Step 7: 提交**

```bash
git add src/stores/inspire.ts src/components/workspace/inspire/InspireRow.vue src/components/workspace/inspire/InspireLibrary.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(inspire): 卡片加「导入到工程」，状态从工程派生"
```

---

### Task 5: 预设风格芯片 + 英文占位符

**Files:**
- Create: `src/components/workspace/inspire/styles.ts`
- Modify: `src/components/workspace/inspire/InspireComposer.vue:15-20`（import）、`:60-62`（`SECTION_TAGS` 旁）、`:292-300`（`insertTag` 之后）、`:356-369`（风格输入框）、`:591-601`（`.tags` 样式旁）
- Modify: `src/i18n/zh.ts:2564` / `src/i18n/en.ts:2567`

**Interfaces:**
- Produces: `STYLE_PRESETS: StylePreset[]`（`{ id: string; label: string; tags: string }`，`label` 是 i18n key），供 Task 6 之外的所有后续改动复用。
- Consumes: 无。

- [ ] **Step 1: 建预设数据文件**

新建 `src/components/workspace/inspire/styles.ts`：

```ts
/**
 * 风格预设：点一下就把一整串英文 tag 插进提示词。
 *
 * 为什么要「整串」而不是一个词：YuE2 的 `style` 是英文 tag 分布，乐器与 BPM 是风格的
 * 主要锚点——只写「jazz」，模型会退回到自己的流行歌先验（实测生成出 Bb 调的标准流行歌）。
 * `label` 是 i18n key（芯片上显示中文/英文短名），`tags` 永远是英文，直接进请求体。
 */
export interface StylePreset {
  id: string;
  /** i18n key：`inspire.stylePreset.<id>`。 */
  label: string;
  /** 插进提示词的英文 tag 串（含乐器与 BPM）。 */
  tags: string;
}

export const STYLE_PRESETS: StylePreset[] = [
  {
    id: "jazz",
    label: "inspire.stylePreset.jazz",
    tags: "swing jazz, walking upright bass, brushed drums, comping piano, tenor saxophone, 96 BPM",
  },
  {
    id: "pop",
    label: "inspire.stylePreset.pop",
    tags: "contemporary pop, bright acoustic guitar, punchy drums, warm bass, catchy hook, 104 BPM",
  },
  {
    id: "rock",
    label: "inspire.stylePreset.rock",
    tags: "indie rock, overdriven electric guitar, live drums, driving bass, 128 BPM",
  },
  {
    id: "lofi",
    label: "inspire.stylePreset.lofi",
    tags: "lo-fi hip hop, dusty vinyl crackle, mellow Rhodes piano, soft boom-bap drums, 78 BPM",
  },
  {
    id: "citypop",
    label: "inspire.stylePreset.citypop",
    tags: "city pop, slap bass, chorus electric guitar, gated reverb drums, dreamy pads, 112 BPM",
  },
  {
    id: "edm",
    label: "inspire.stylePreset.edm",
    tags: "melodic house, four-on-the-floor kick, sidechained pads, plucky synth arp, 124 BPM",
  },
  {
    id: "hiphop",
    label: "inspire.stylePreset.hiphop",
    tags: "boom bap hip hop, sampled soul horns, dusty drums, deep sub bass, 88 BPM",
  },
  {
    id: "cnfolk",
    label: "inspire.stylePreset.cnfolk",
    tags: "Chinese folk, guzheng, dizi flute, erhu, modern arrangement, 90 BPM",
  },
  {
    id: "cinematic",
    label: "inspire.stylePreset.cinematic",
    tags: "cinematic orchestral, string swells, epic brass, timpani, choir pads, 72 BPM",
  },
  {
    id: "ambient",
    label: "inspire.stylePreset.ambient",
    tags: "ambient, warm analog pads, slow evolving textures, no drums, 60 BPM",
  },
];
```

- [ ] **Step 2: 文案**

`zh.ts` 把 `"inspire.stylePlaceholder"` 改成英文示范，并新增提示与十个芯片名：

```ts
  "inspire.stylePlaceholder": "English, warm female jazz vocal, upright bass, brushed drums, tenor saxophone, 96 BPM",
  "inspire.styleHint": "用英文 tag 描述最稳：开头写语言、末尾写 BPM，逗号分隔",
  "inspire.stylePreset.jazz": "爵士",
  "inspire.stylePreset.pop": "流行",
  "inspire.stylePreset.rock": "摇滚",
  "inspire.stylePreset.lofi": "Lo-fi",
  "inspire.stylePreset.citypop": "City Pop",
  "inspire.stylePreset.edm": "电子",
  "inspire.stylePreset.hiphop": "说唱",
  "inspire.stylePreset.cnfolk": "国风",
  "inspire.stylePreset.cinematic": "配乐",
  "inspire.stylePreset.ambient": "氛围",
```

`en.ts` 对应：

```ts
  "inspire.stylePlaceholder": "English, warm female jazz vocal, upright bass, brushed drums, tenor saxophone, 96 BPM",
  "inspire.styleHint": "English tags work best: language first, BPM last, comma separated",
  "inspire.stylePreset.jazz": "Jazz",
  "inspire.stylePreset.pop": "Pop",
  "inspire.stylePreset.rock": "Rock",
  "inspire.stylePreset.lofi": "Lo-fi",
  "inspire.stylePreset.citypop": "City Pop",
  "inspire.stylePreset.edm": "EDM",
  "inspire.stylePreset.hiphop": "Hip-hop",
  "inspire.stylePreset.cnfolk": "Chinese folk",
  "inspire.stylePreset.cinematic": "Cinematic",
  "inspire.stylePreset.ambient": "Ambient",
```

- [ ] **Step 3: 编辑器脚本**

`InspireComposer.vue` 的 import 段加：

```ts
import { STYLE_PRESETS } from "./styles";
```

在 `insertTag` 函数之后插入：

```ts
/**
 * 点芯片 = 切换这一段 tag：没有就追加、有就整段删掉。
 *
 * 选中态不用额外的 ref，直接看提示词里有没有这段子串——用户手改提示词时芯片状态
 * 不会和内容打架（删掉其中一个词，芯片自己就灭了）。
 */
function togglePreset(tags: string): void {
  const value = prompt.value;
  if (value.includes(tags)) {
    prompt.value = value
      .replace(tags, "")
      .replace(/(\s*,\s*){2,}/g, ", ")
      .replace(/^[\s,]+|[\s,]+$/g, "");
    return;
  }
  const trimmed = value.trim().replace(/[\s,]+$/, "");
  prompt.value = trimmed ? `${trimmed}, ${tags}` : tags;
}
```

- [ ] **Step 4: 模板**

风格输入框那个 `<label class="field">` 里，`<span class="field-label">` 与 `<textarea>` 之间插入芯片行；`</label>` 之前（textarea 之后）插入提示行：

```vue
        <div class="tags preset-tags">
          <button
            v-for="preset in STYLE_PRESETS"
            :key="preset.id"
            class="m3-chip tag"
            :class="{ on: prompt.includes(preset.tags) }"
            type="button"
            :aria-pressed="prompt.includes(preset.tags)"
            :disabled="inspire.crafting"
            @click="togglePreset(preset.tags)"
          >
            {{ i18n.t(preset.label) }}
          </button>
        </div>
```

```vue
        <p class="hint">{{ i18n.t("inspire.styleHint") }}</p>
```

- [ ] **Step 5: 样式**

在现有 `.tags { … }` 旁边补：

```css
/* 风格芯片：一行放不下就换行，横向不滚（面板只有 400 多 px）。 */
.preset-tags {
  margin-bottom: 2px;
}
```

（`.tag` 与 `.m3-chip.on` 都是现成的，不要重复定义。）

- [ ] **Step 6: 类型校验**

```bash
npx vue-tsc --noEmit -p tsconfig.json
```

Expected: 通过。

- [ ] **Step 7: 提交**

```bash
git add src/components/workspace/inspire/styles.ts src/components/workspace/inspire/InspireComposer.vue src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(inspire): 风格预设芯片 + 英文占位符引导"
```

---

### Task 6: 纯音乐 / 有人声开关

**Files:**
- Modify: `src/components/workspace/inspire/InspireComposer.vue:63-68`（`Draft` 接口）、`:84-90`（ref）、`:115-125`（`modeOptions` 之后）、`:158-162`（`canSubmit`）、`:176-195`（`submitLocal`）、`:395-420`（歌词区与参数区）、`:450-455`（底部提示）
- Modify: `src/stores/inspire.ts:499-515`（`craftLocal` 的 `style`）
- Modify: `src/i18n/zh.ts` / `src/i18n/en.ts`

**Interfaces:**
- Consumes: `CraftRequest.instrumental`（已存在，`src/stores/inspire.ts` 定义）。
- Produces: `styleOf(request: CraftRequest): string`（store 内部函数，不外露）。

- [ ] **Step 1: 文案**

`zh.ts`：新增两个键，并把 `inspire.instrumentalHint` 改写成「开关已开」的说法：

```ts
  "inspire.vocals": "有人声",
  "inspire.vocalsHint": "用歌词去唱；歌词留空则由模型写词",
  "inspire.instrumental": "纯音乐",
  "inspire.instrumentalHint": "纯音乐：不使用歌词",
```

`en.ts`：

```ts
  "inspire.vocals": "Vocals",
  "inspire.vocalsHint": "Sing the lyrics; leave them empty and the model writes them",
  "inspire.instrumental": "Instrumental",
  "inspire.instrumentalHint": "Instrumental: the lyrics are ignored",
```

- [ ] **Step 2: 草稿里存开关**

`InspireComposer.vue` 的 `Draft` 接口加一个字段（放在 `advanced` 之前）：

```ts
  instrumental: boolean;
```

在 `const seed = …` 之后加：

```ts
/** 纯音乐 / 有人声。默认有人声——与改动前「歌词留空 = 模型写词」的行为一致。 */
const instrumental = ref(draft.instrumental === true);
```

`draftTimer` 里持久化的对象加上 `instrumental: instrumental.value,`（与 `advanced` 并列）。

- [ ] **Step 3: 分段控件**

`modeOptions` 之后插入：

```ts
const vocalsValue = computed<string>({
  get: () => (instrumental.value ? "instrumental" : "vocals"),
  set: (value) => {
    instrumental.value = value === "instrumental";
  },
});

const vocalsOptions = computed<SegmentedOption[]>(() => [
  { value: "vocals", label: i18n.t("inspire.vocals"), title: i18n.t("inspire.vocalsHint") },
  {
    value: "instrumental",
    label: i18n.t("inspire.instrumental"),
    title: i18n.t("inspire.instrumentalHint"),
  },
]);
```

- [ ] **Step 4: 提交逻辑**

`canSubmit` 改成（纯音乐时歌词不可用，只剩描述能撑住提交）：

```ts
const canSubmit = computed(() => {
  if (inspire.crafting) return true;
  if (preparing.value) return false;
  const hasLyrics = !instrumental.value && Boolean(lyrics.value.trim());
  return Boolean(prompt.value.trim()) || hasLyrics;
});
```

`submitLocal` 里两处：

```ts
    lyrics: instrumental.value ? "" : lyrics.value,
    instrumental: instrumental.value,
```

（删掉原来的 `instrumental: !lyrics.value.trim()`。）

- [ ] **Step 5: store 里补 instrumental 标记**

`src/stores/inspire.ts` 的 `craftLocal` 里，`style: request.prompt.trim(),` 改成 `style: styleOf(request),`，并在 `craftLocal` 函数之前插入：

```ts
/**
 * 请求体里的风格串：纯音乐补一个 `instrumental`。
 *
 * 补在这里而不是提交时改 `prompt`，是为了让列表标题/描述保持用户原话，
 * 不被一个技术性后缀污染。
 */
function styleOf(request: CraftRequest): string {
  const style = request.prompt.trim();
  if (!request.instrumental) return style;
  if (!style) return "instrumental";
  return /\binstrumental\b/i.test(style) ? style : `${style}, instrumental`;
}
```

- [ ] **Step 6: 模板**

1. 歌词 `<textarea>` 的 `:disabled` 改成 `:disabled="inspire.crafting || instrumental"`。
2. 参数区（`<div v-if="advanced" class="params">`）之前插入开关：

```vue
      <div class="field">
        <span class="field-label">{{ i18n.t("inspire.voices") }}</span>
        <Md3Segmented v-model="vocalsValue" :options="vocalsOptions" :aria-label="i18n.t('inspire.voices')" />
      </div>
```

并在 `zh.ts`/`en.ts` 补这一个键：`"inspire.voices": "人声"` / `"inspire.voices": "Voice"`。

3. 底部那句提示（原来按 `isCloud` 分叉）换成按开关分叉：

```vue
      <p class="hint">{{ i18n.t(instrumental ? "inspire.instrumentalHint" : "inspire.lyricsAuto") }}</p>
```

- [ ] **Step 7: 类型校验**

```bash
npx vue-tsc --noEmit -p tsconfig.json
```

Expected: 通过。

- [ ] **Step 8: 提交**

```bash
git add src/components/workspace/inspire/InspireComposer.vue src/stores/inspire.ts src/i18n/zh.ts src/i18n/en.ts
git commit -m "feat(inspire): 纯音乐/有人声改成显式开关"
```

---

## 收尾验证（全部任务完成后）

- [ ] `cargo test -p ai-daw-daemon --lib --no-default-features`：全绿
- [ ] `npm run build`：`vue-tsc --noEmit` + `vite build` 都通过
- [ ] 手动跑一遍（引擎需已就绪，**不要为验证再加载新模型**）：
  1. 灵感页输入「jazz」，点「爵士」芯片 → 提示词变成完整英文 tag 串
  2. 生成 → 编排页**没有**新轨
  3. 回到灵感页点卡片上的导入按钮 → 编排页出现一条以歌名为名字的音频轨 → 按钮变「已导入 · 再导入一次」
  4. 再点一次 → 又一条轨
  5. 删掉这两条轨 → 按钮自动变回「导入到工程」
  6. 切「纯音乐」→ 歌词框禁用、提示词末尾带 `instrumental`；再点「爵士」→ 子串仍然匹配、芯片保持点亮
- [ ] 回归：AI 面板写歌仍然自动进工程（`AiYuePage` 没传 `place_in_project`）
