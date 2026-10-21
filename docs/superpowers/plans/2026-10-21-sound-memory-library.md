# 音色记忆库 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让用户把「乐器 + 整条效果链 + 增益/声像」存成跨工程的音色记忆，AI 通过 MCP 保存/检索/应用，用户在前端音色库面板查看、编辑元数据、删除。

**Architecture:** 新增独立于 `presets.json` 的 `sound-memories.json` 与 `SoundMemoryStore`。保存时后端直接从轨道抓快照（模型只传 track id）；应用时走一次 `execute_all` 事务（换乐器 → 写参数 → 重建效果链 → 写增益/声像），插件二进制状态在事务后 best-effort 补写。MCP 新增 6 个工具，前端新增右侧「音色库」面板，并通过 `engine://sound-memories` 事件自动刷新。

**Tech Stack:** Rust（Tauri 2、serde、base64）、rmcp/schemars、Vue 3 + Pinia + TypeScript、Heroicons。

## Global Constraints

- 存储文件固定在应用配置目录：`sound-memories.json`；拿不到目录时退化为内存模式（重启丢失）。
- **默认绝不自动保存**：只有用户明确要求，或 AI 询问后用户确认，才写库。
- 保存快照由**后端从轨道读取**；MCP 的 `save_sound_memory` 不接受 `params` / `effects`。
- 应用一条记忆 = **一个撤销步**；效果链整体替换，不叠加。
- 快照只含：乐器、效果链、`gain_db` / `pan`；**不含**发送、自动化、片段、静音/独奏、轨道名。
- 外部插件 `state` 存 base64；应用后补写，不进入撤销/重做重放（引擎现状）。
- 所有新增界面文案同时补 `src/i18n/zh.ts` 与 `src/i18n/en.ts`，不在组件里硬编码中文。
- 不新增前端保存入口；保存只由 AI 触发。
- 新依赖版本：`base64 = "0.22"`。

---

### Task 1: 音色记忆数据模型与存储

**Files:**
- Modify: `src-tauri/Cargo.toml`（`[dependencies]` 末尾加 `base64`）
- Create: `src-tauri/src/sound_memories.rs`
- Modify: `src-tauri/src/lib.rs:3-20`（模块声明加 `mod sound_memories;`）

**Interfaces:**
- Produces:
  - `SoundMemory { id, name, description, tags, role, instrument, effects, mixer, source_project, created_at, updated_at }`
  - `SoundMemorySummary { id, name, description, tags, role, instrument_plugin_id, effect_plugin_ids, mixer, source_project, created_at, updated_at }`
  - `PluginSnapshot { plugin_id, params: BTreeMap<String, f32>, state: String }`
  - `SoundMixer { gain_db, pan }`
  - `MemoryMeta { name, description, tags, role }`
  - `SoundSnapshot { instrument, effects, mixer, source_project, role }`
  - `SoundMemoryStore::{load, all, summaries, get, create, replace, update_metadata, remove}`
  - `PluginSnapshot::{from_instance, state_bytes}`
  - `SoundMemory::summary()`

- [ ] **Step 1: 加 base64 依赖并创建带失败测试的模块骨架**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 里加：

```toml
base64 = "0.22"
```

在 `src-tauri/src/lib.rs` 的模块列表里加：

```rust
mod sound_memories;
```

创建 `src-tauri/src/sound_memories.rs`，先写类型、方法签名（`todo!()`）和测试：

```rust
//! 音色记忆库：把一条轨道的「乐器 + 效果链 + 增益/声像」存成跨工程快照。
//!
//! 与 `presets.rs` 的分工：用户预设只存单个插件的参数，用来快速切音色；音色记忆存整条
//! 音色链（含效果器顺序与二进制状态），用来在别的工程里原样复现。存在应用配置目录的
//! `sound-memories.json`，整文件读改写。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundMixer {
    pub gain_db: f32,
    pub pan: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginSnapshot {
    pub plugin_id: String,
    #[serde(default)]
    pub params: BTreeMap<String, f32>,
    /// base64 编码的插件二进制状态；空字符串 = 没有可存的状态。
    #[serde(default)]
    pub state: String,
}

impl PluginSnapshot {
    pub fn from_instance(inst: &ai_daw_engine::model::PluginInstance) -> Self {
        Self {
            plugin_id: inst.plugin_id.clone(),
            params: inst
                .params
                .iter()
                .filter(|(_, value)| value.is_finite())
                .map(|(id, value)| (id.clone(), *value))
                .collect(),
            state: if inst.state.is_empty() {
                String::new()
            } else {
                BASE64.encode(&inst.state)
            },
        }
    }

    pub fn state_bytes(&self) -> Vec<u8> {
        if self.state.is_empty() {
            Vec::new()
        } else {
            BASE64.decode(self.state.as_bytes()).unwrap_or_default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MemoryMeta {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SoundSnapshot {
    pub instrument: Option<PluginSnapshot>,
    pub effects: Vec<PluginSnapshot>,
    pub mixer: SoundMixer,
    pub source_project: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoundMemory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub instrument: Option<PluginSnapshot>,
    #[serde(default)]
    pub effects: Vec<PluginSnapshot>,
    pub mixer: SoundMixer,
    #[serde(default)]
    pub source_project: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

impl SoundMemory {
    pub fn summary(&self) -> SoundMemorySummary {
        SoundMemorySummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            tags: self.tags.clone(),
            role: self.role.clone(),
            instrument_plugin_id: self.instrument.as_ref().map(|i| i.plugin_id.clone()),
            effect_plugin_ids: self.effects.iter().map(|e| e.plugin_id.clone()).collect(),
            mixer: self.mixer.clone(),
            source_project: self.source_project.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SoundMemorySummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub role: Option<String>,
    pub instrument_plugin_id: Option<String>,
    pub effect_plugin_ids: Vec<String>,
    pub mixer: SoundMixer,
    pub source_project: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct SoundMemoryStore {
    path: Option<PathBuf>,
    memories: Vec<SoundMemory>,
}

impl SoundMemoryStore {
    pub fn load(path: Option<PathBuf>) -> Self {
        let memories = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Vec<SoundMemory>>(&text).ok())
            .unwrap_or_default();
        SoundMemoryStore { path, memories }
    }

    pub fn all(&self) -> Vec<SoundMemory> {
        let mut out = self.memories.clone();
        out.sort_by_key(|m| m.created_at);
        out
    }

    pub fn summaries(&self) -> Vec<SoundMemorySummary> {
        self.all().iter().map(SoundMemory::summary).collect()
    }

    pub fn get(&self, id: &str) -> Option<SoundMemory> {
        self.memories.iter().find(|m| m.id == id).cloned()
    }

    pub fn create(&mut self, meta: MemoryMeta, snapshot: SoundSnapshot) -> Result<SoundMemory, String> {
        todo!()
    }

    pub fn replace(
        &mut self,
        id: &str,
        meta: MemoryMeta,
        snapshot: SoundSnapshot,
    ) -> Result<SoundMemory, String> {
        todo!()
    }

    pub fn update_metadata(
        &mut self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        tags: Option<Vec<String>>,
        role: Option<&str>,
    ) -> Result<SoundMemory, String> {
        todo!()
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        todo!()
    }

    fn flush(&self) -> Result<(), String> {
        todo!()
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .filter(|t| seen.insert(t.to_lowercase()))
        .collect()
}

fn normalize_meta(meta: MemoryMeta) -> Result<MemoryMeta, String> {
    let name = meta.name.trim().to_string();
    if name.is_empty() {
        return Err("音色记忆的名字不能为空".into());
    }
    Ok(MemoryMeta {
        name,
        description: meta.description.trim().to_string(),
        tags: normalize_tags(meta.tags),
        role: meta
            .role
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty()),
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn unique_id(existing: &[SoundMemory], seed: u64) -> String {
    let mut candidate = format!("s{seed}");
    let mut n = 1;
    while existing.iter().any(|m| m.id == candidate) {
        candidate = format!("s{seed}-{n}");
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> SoundSnapshot {
        SoundSnapshot {
            instrument: Some(PluginSnapshot {
                plugin_id: "builtin.synth".into(),
                params: BTreeMap::from([("attack".into(), 1.2), ("cutoff".into(), 1800.0)]),
                state: String::new(),
            }),
            effects: vec![PluginSnapshot {
                plugin_id: "builtin.reverb".into(),
                params: BTreeMap::from([("mix".into(), 0.28)]),
                state: "AQID".into(),
            }],
            mixer: SoundMixer { gain_db: -6.0, pan: 0.0 },
            source_project: Some("Night Drive".into()),
            role: Some("pad".into()),
        }
    }

    fn meta(name: &str) -> MemoryMeta {
        MemoryMeta {
            name: name.into(),
            description: "温暖、宽".into(),
            tags: vec!["pad".into(), "Warm".into(), "warm".into(), " ".into()],
            role: Some("pad".into()),
        }
    }

    #[test]
    fn roundtrips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("ai-daw-sounds-{}", now_millis()));
        let path = dir.join("sound-memories.json");

        let mut store = SoundMemoryStore::load(Some(path.clone()));
        let saved = store.create(meta("温暖 Pad"), snapshot()).unwrap();
        assert_eq!(saved.tags, vec!["pad".to_string(), "Warm".to_string()]);
        assert_eq!(saved.instrument.as_ref().unwrap().params["attack"], 1.2);
        assert_eq!(saved.effects[0].state, "AQID");

        let reloaded = SoundMemoryStore::load(Some(path));
        assert_eq!(reloaded.all(), vec![saved.clone()]);
        assert_eq!(reloaded.get(&saved.id), Some(saved.clone()));
        let summary = &reloaded.summaries()[0];
        assert_eq!(summary.instrument_plugin_id.as_deref(), Some("builtin.synth"));
        assert_eq!(summary.effect_plugin_ids, vec!["builtin.reverb".to_string()]);
        assert_eq!(summary.mixer.gain_db, -6.0);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unique_ids_even_in_the_same_millisecond() {
        let mut store = SoundMemoryStore::load(None);
        let a = store.create(meta("a"), snapshot()).unwrap();
        let b = store.create(meta("b"), snapshot()).unwrap();
        let c = store.create(meta("c"), snapshot()).unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(b.id, c.id);
        assert_ne!(a.id, c.id);
    }

    #[test]
    fn replace_keeps_created_at_and_updates_content() {
        let mut store = SoundMemoryStore::load(None);
        let saved = store.create(meta("old"), snapshot()).unwrap();
        let mut next = snapshot();
        next.instrument = None;
        next.effects = Vec::new();
        next.mixer = SoundMixer { gain_db: -12.0, pan: 0.5 };

        let replaced = store
            .replace(&saved.id, meta("new"), next)
            .unwrap();
        assert_eq!(replaced.id, saved.id);
        assert_eq!(replaced.created_at, saved.created_at);
        assert_eq!(replaced.name, "new");
        assert!(replaced.instrument.is_none());
        assert!(replaced.effects.is_empty());
        assert_eq!(replaced.mixer.gain_db, -12.0);
    }

    #[test]
    fn metadata_update_leaves_the_snapshot_alone() {
        let mut store = SoundMemoryStore::load(None);
        let saved = store.create(meta("old"), snapshot()).unwrap();
        let updated = store
            .update_metadata(
                &saved.id,
                Some("new"),
                Some("新描述"),
                Some(vec!["bass".into(), " ".into()]),
                Some(""),
            )
            .unwrap();
        assert_eq!(updated.name, "new");
        assert_eq!(updated.description, "新描述");
        assert_eq!(updated.tags, vec!["bass".to_string()]);
        assert_eq!(updated.role, None);
        assert_eq!(updated.instrument, saved.instrument);
        assert_eq!(updated.effects, saved.effects);
        assert_eq!(updated.mixer, saved.mixer);
    }

    #[test]
    fn rejects_empty_names_and_missing_ids() {
        let mut store = SoundMemoryStore::load(None);
        assert!(store.create(meta("   "), snapshot()).is_err());
        assert!(store.replace("missing", meta("x"), snapshot()).is_err());
        assert!(store.update_metadata("missing", Some("x"), None, None, None).is_err());
        assert!(store.remove("missing").is_err());
    }
}
```

- [ ] **Step 2: 运行测试，确认失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sound_memories
```

Expected: 编译通过，5 个测试失败（`todo!()` panic）。

- [ ] **Step 3: 实现 store 方法**

把 `create` / `replace` / `update_metadata` / `remove` / `flush` 替换为：

```rust
    pub fn create(&mut self, meta: MemoryMeta, snapshot: SoundSnapshot) -> Result<SoundMemory, String> {
        let meta = normalize_meta(meta)?;
        let now = now_millis();
        let memory = SoundMemory {
            id: unique_id(&self.memories, now),
            name: meta.name,
            description: meta.description,
            tags: meta.tags,
            role: meta.role,
            instrument: snapshot.instrument,
            effects: snapshot.effects,
            mixer: snapshot.mixer,
            source_project: snapshot.source_project,
            created_at: now,
            updated_at: now,
        };
        self.memories.push(memory.clone());
        self.flush()?;
        Ok(memory)
    }

    pub fn replace(
        &mut self,
        id: &str,
        meta: MemoryMeta,
        snapshot: SoundSnapshot,
    ) -> Result<SoundMemory, String> {
        let meta = normalize_meta(meta)?;
        let memory = self
            .memories
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("音色记忆不存在：{id}"))?;
        memory.name = meta.name;
        memory.description = meta.description;
        memory.tags = meta.tags;
        memory.role = meta.role;
        memory.instrument = snapshot.instrument;
        memory.effects = snapshot.effects;
        memory.mixer = snapshot.mixer;
        memory.source_project = snapshot.source_project;
        memory.updated_at = now_millis();
        let updated = memory.clone();
        self.flush()?;
        Ok(updated)
    }

    pub fn update_metadata(
        &mut self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        tags: Option<Vec<String>>,
        role: Option<&str>,
    ) -> Result<SoundMemory, String> {
        let memory = self
            .memories
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("音色记忆不存在：{id}"))?;
        if let Some(name) = name {
            let name = name.trim();
            if name.is_empty() {
                return Err("音色记忆的名字不能为空".into());
            }
            memory.name = name.to_string();
        }
        if let Some(description) = description {
            memory.description = description.trim().to_string();
        }
        if let Some(tags) = tags {
            memory.tags = normalize_tags(tags);
        }
        if let Some(role) = role {
            memory.role = if role.trim().is_empty() {
                None
            } else {
                Some(role.trim().to_string())
            };
        }
        memory.updated_at = now_millis();
        let updated = memory.clone();
        self.flush()?;
        Ok(updated)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let before = self.memories.len();
        self.memories.retain(|m| m.id != id);
        if self.memories.len() == before {
            return Err(format!("音色记忆不存在：{id}"));
        }
        self.flush()
    }

    fn flush(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建音色记忆目录失败：{e}"))?;
        }
        let text = serde_json::to_string_pretty(&self.memories)
            .map_err(|e| format!("序列化音色记忆失败：{e}"))?;
        fs::write(path, text).map_err(|e| format!("写入音色记忆失败：{e}"))
    }
```

- [ ] **Step 4: 运行测试，确认通过**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sound_memories
```

Expected: 5 passed。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/src/sound_memories.rs
git commit -F - <<'EOF'
feat: 音色记忆数据模型与存储

新增独立于用户预设的 sound-memories.json：保存乐器、有序效果链与
增益/声像快照，插件二进制状态用 base64 存储。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 2: 把记忆库装进 AppState 与启动流程

**Files:**
- Modify: `src-tauri/src/state.rs:14`（import）、`:39-69`（字段）、`:86-100`（构造）、`:122-125`（init 方法）、`:158-171`（事件常量）
- Modify: `src-tauri/src/lib.rs:227-232`（文件位置）、`:285-287`（setup）

**Interfaces:**
- Consumes: `SoundMemoryStore::load(Option<PathBuf>)`
- Produces:
  - `AppState.sound_memories: Mutex<SoundMemoryStore>`
  - `AppState::init_sound_memories(Option<PathBuf>)`
  - `state::EVENT_SOUND_MEMORIES = "engine://sound-memories"`

- [ ] **Step 1: 给 AppState 加字段与初始化**

`src-tauri/src/state.rs` import 区加：

```rust
use crate::sound_memories::SoundMemoryStore;
```

`AppState` 字段区（`presets` 后面）加：

```rust
    /// 音色记忆库（跨工程；文件位置由 `init_sound_memories` 装上）。
    pub sound_memories: Mutex<SoundMemoryStore>,
```

`AppState::new` 构造体里（`presets` 后面）加：

```rust
            sound_memories: Mutex::new(SoundMemoryStore::load(None)),
```

`init_presets` 后面加：

```rust
    /// 装上音色记忆文件（启动时由 `setup` 调用；拿不到配置目录时只活在内存里）。
    pub fn init_sound_memories(&self, path: Option<PathBuf>) {
        *self.sound_memories.lock().unwrap() = SoundMemoryStore::load(path);
    }
```

事件常量区（`EVENT_PLUGINS` 后面）加：

```rust
/// 音色记忆库变更的前端事件名。
pub const EVENT_SOUND_MEMORIES: &str = "engine://sound-memories";
```

- [ ] **Step 2: 在 lib.rs 里定位文件并初始化**

`preset_file` 后面加：

```rust
/// 音色记忆库文件的位置：应用配置目录下的 `sound-memories.json`。
fn sound_memory_file(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("sound-memories.json"))
}
```

`setup` 里 `state.init_presets(...)` 后面加：

```rust
            state.init_sound_memories(sound_memory_file(app.handle()));
```

- [ ] **Step 3: 编译检查**

Run:

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: 编译通过（此时字段/方法暂时未被命令使用，不会有 warning 失败）。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs
git commit -F - <<'EOF'
feat: 启动时加载音色记忆库

AppState 持有 SoundMemoryStore，并在应用配置目录读取
sound-memories.json；新增 engine://sound-memories 事件名。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 3: 从轨道抓快照 & 一次事务应用

**Files:**
- Modify: `src-tauri/src/commands.rs:109`（`capture_plugin_states` 改 `pub(crate)`）
- Modify: `src-tauri/src/commands.rs:1-26`（import）
- Modify: `src-tauri/src/commands.rs:597-668`（在 `apply_preset_*` 后面加音色记忆内核）
- Modify: `src-tauri/src/commands.rs:3220-3344`（测试模块加回归测试）

**Interfaces:**
- Consumes: `SoundMemory`, `PluginSnapshot`, `SoundSnapshot`, `MemoryMeta`, `SoundMixer` from Task 1
- Produces:
  - `commands::capture_plugin_states(&AppState)`（`pub(crate)`）
  - `commands::sound_snapshot_for(&AppState, TrackId) -> Result<SoundSnapshot, String>`
  - `commands::apply_sound_memory_inner(&AppState, TrackId, &SoundMemory, Actor) -> (Project, Vec<Event>)`
  - `commands::apply_sound_memory_agent_as(&AppHandle, Actor, TrackId, &SoundMemory) -> (Project, Vec<Event>)`
  - `commands::notify_sound_memories(&AppHandle)`

- [ ] **Step 1: 写失败测试**

在 `src-tauri/src/commands.rs` 的 `mod tests` 里加：

```rust
    /// 音色记忆：从轨道抓快照 → 应用到另一条轨道，乐器/效果链/增益声像逐值一致。
    #[test]
    fn sound_memory_snapshot_and_apply_roundtrip() {
        use std::collections::BTreeMap;

        let state = AppState::new("t");
        let track = {
            let engine = state.engine.lock().unwrap();
            engine.project().tracks[0].id
        };
        {
            let mut engine = state.engine.lock().unwrap();
            let events = engine.execute_all(
                vec![
                    Command::SetTrackInstrument {
                        track,
                        plugin_id: "builtin.synth".into(),
                    },
                    Command::SetParam {
                        track,
                        param: "attack".into(),
                        value: 0.25,
                    },
                    Command::AddEffect {
                        track,
                        plugin_id: "builtin.reverb".into(),
                    },
                    Command::SetEffectParam {
                        track,
                        index: 0,
                        param: "mix".into(),
                        value: 0.4,
                    },
                    Command::AddEffect {
                        track,
                        plugin_id: "builtin.delay".into(),
                    },
                    Command::SetGain {
                        track,
                        gain_db: -3.0,
                    },
                    Command::SetPan { track, pan: 0.2 },
                ],
                Actor::Human,
            );
            assert!(!events.is_empty());
        }

        let snapshot = sound_snapshot_for(&state, track).unwrap();
        assert_eq!(snapshot.instrument.as_ref().unwrap().plugin_id, "builtin.synth");
        assert_eq!(snapshot.instrument.as_ref().unwrap().params["attack"], 0.25);
        assert_eq!(snapshot.effects.len(), 2);
        assert_eq!(snapshot.effects[0].plugin_id, "builtin.reverb");
        assert_eq!(snapshot.effects[1].plugin_id, "builtin.delay");
        assert_eq!(snapshot.mixer.gain_db, -3.0);
        assert_eq!(snapshot.mixer.pan, 0.2);

        let memory = SoundMemory {
            id: "s1".into(),
            name: "test".into(),
            description: String::new(),
            tags: Vec::new(),
            role: None,
            instrument: Some(PluginSnapshot {
                plugin_id: "builtin.synth".into(),
                params: BTreeMap::from([("cutoff".into(), 900.0)]),
                state: String::new(),
            }),
            effects: vec![PluginSnapshot {
                plugin_id: "builtin.chorus".into(),
                params: BTreeMap::from([("mix".into(), 0.5)]),
                state: String::new(),
            }],
            mixer: SoundMixer {
                gain_db: -9.0,
                pan: -0.5,
            },
            source_project: None,
            created_at: 0,
            updated_at: 0,
        };

        let depth_before = state.engine.lock().unwrap().undo_depth();
        let (project, events) = apply_sound_memory_inner(&state, track, &memory, Actor::Human);
        assert!(!events.is_empty());
        assert_eq!(state.engine.lock().unwrap().undo_depth(), depth_before + 1);

        let t = project.tracks.iter().find(|t| t.id == track).unwrap();
        assert_eq!(t.instrument.as_ref().unwrap().plugin_id, "builtin.synth");
        assert_eq!(t.instrument.as_ref().unwrap().params["cutoff"], 900.0);
        assert_eq!(t.effects.len(), 1, "旧效果链必须被整体替换");
        assert_eq!(t.effects[0].plugin_id, "builtin.chorus");
        assert_eq!(t.effects[0].params["mix"], 0.5);
        assert_eq!(t.mixer.gain_db, -9.0);
        assert_eq!(t.mixer.pan, -0.5);
    }
```

- [ ] **Step 2: 运行测试，确认失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sound_memory_snapshot_and_apply_roundtrip
```

Expected: 编译失败，提示 `sound_snapshot_for` / `apply_sound_memory_inner` / `SoundMemory` 未定义。

- [ ] **Step 3: 改 import 与 `capture_plugin_states` 可见性**

`commands.rs` 的 `use crate::presets::Preset;` 后面加：

```rust
use crate::sound_memories::{
    MemoryMeta, PluginSnapshot, SoundMemory, SoundMixer, SoundSnapshot,
};
```

把 `fn capture_plugin_states(state: &AppState)` 改成：

```rust
pub(crate) fn capture_plugin_states(state: &AppState) {
```

- [ ] **Step 4: 实现快照与内核**

在 `apply_preset_agent_as` 后面加：

```rust
/// 从一条轨道抓音色快照：乐器 + 有序效果链 + 增益/声像。
///
/// 抓之前先抢救外部插件编辑器里的二进制状态，确保存下来的是用户**听到的**那份。
pub(crate) fn sound_snapshot_for(
    state: &AppState,
    track: TrackId,
) -> Result<SoundSnapshot, String> {
    capture_plugin_states(state);
    let engine = state.engine.lock().unwrap();
    let project = engine.project();
    let t = project
        .tracks
        .iter()
        .find(|t| t.id == track)
        .ok_or_else(|| format!("轨道 {} 不存在", track.0))?;
    Ok(SoundSnapshot {
        instrument: t.instrument.as_ref().map(PluginSnapshot::from_instance),
        effects: t.effects.iter().map(PluginSnapshot::from_instance).collect(),
        mixer: SoundMixer {
            gain_db: t.mixer.gain_db,
            pan: t.mixer.pan,
        },
        source_project: Some(project.name.clone()),
        role: t.meta.as_ref().and_then(|m| m.role.clone()),
    })
}

/// 应用一条音色记忆的**内核**：一次事务写回乐器、整条效果链、增益/声像。
///
/// 效果链是**替换**而不是叠加。插件二进制状态在事务后补写：`set_*_state` 不产生事件，
/// 因此不进入撤销/重做重放——这是引擎现状，内置合成器/效果器只用参数，不受影响。
pub(crate) fn apply_sound_memory_inner(
    state: &AppState,
    track: TrackId,
    memory: &SoundMemory,
    actor: Actor,
) -> (Project, Vec<Event>) {
    capture_plugin_states(state);
    let mut engine = state.engine.lock().unwrap();
    let existing = engine
        .project()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .map_or(0, |t| t.effects.len());

    let mut commands: Vec<Command> = Vec::new();
    match &memory.instrument {
        Some(inst) => {
            commands.push(Command::SetTrackInstrument {
                track,
                plugin_id: inst.plugin_id.clone(),
            });
            for (param, value) in &inst.params {
                if value.is_finite() {
                    commands.push(Command::SetParam {
                        track,
                        param: param.clone(),
                        value: *value,
                    });
                }
            }
        }
        None => commands.push(Command::SetTrackInstrument {
            track,
            plugin_id: String::new(),
        }),
    }

    for _ in 0..existing {
        commands.push(Command::RemoveEffect { track, index: 0 });
    }
    for (index, fx) in memory.effects.iter().enumerate() {
        commands.push(Command::AddEffect {
            track,
            plugin_id: fx.plugin_id.clone(),
        });
        for (param, value) in &fx.params {
            if value.is_finite() {
                commands.push(Command::SetEffectParam {
                    track,
                    index,
                    param: param.clone(),
                    value: *value,
                });
            }
        }
    }

    if memory.mixer.gain_db.is_finite() {
        commands.push(Command::SetGain {
            track,
            gain_db: memory.mixer.gain_db,
        });
    }
    if memory.mixer.pan.is_finite() {
        commands.push(Command::SetPan {
            track,
            pan: memory.mixer.pan,
        });
    }

    let events = engine.execute_all(commands, actor);

    if let Some(inst) = &memory.instrument {
        let bytes = inst.state_bytes();
        if !bytes.is_empty() {
            engine.set_instrument_state(track, &inst.plugin_id, bytes);
        }
    }
    for (index, fx) in memory.effects.iter().enumerate() {
        let bytes = fx.state_bytes();
        if !bytes.is_empty() {
            engine.set_effect_state(track, index, bytes);
        }
    }

    (engine.project().clone(), events)
}

/// 应用一条音色记忆（代理入口）：执行 → 广播 → 同步音频线程。
pub(crate) fn apply_sound_memory_agent_as(
    app: &tauri::AppHandle,
    actor: Actor,
    track: TrackId,
    memory: &SoundMemory,
) -> (Project, Vec<Event>) {
    let state = app.state::<AppState>();
    let (project, events) = apply_sound_memory_inner(&state, track, memory, actor);
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    (project, events)
}

/// 音色记忆库变更后通知前端刷新。
pub(crate) fn notify_sound_memories(app: &tauri::AppHandle) {
    let _ = app.emit(crate::state::EVENT_SOUND_MEMORIES, ());
}
```

- [ ] **Step 5: 运行测试，确认通过**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib sound_memory_snapshot_and_apply_roundtrip
```

Expected: 1 passed。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands.rs
git commit -F - <<'EOF'
feat: 音色记忆快照与应用内核

后端直接从轨道抓乐器/效果链/增益声像；应用时一次事务重建，
效果链整体替换，插件状态在事务后 best-effort 补写。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 4: Tauri 命令与前端事件

**Files:**
- Modify: `src-tauri/src/commands.rs:3180-3218`（预设命令后面加 5 个命令）
- Modify: `src-tauri/src/lib.rs:437-441`（invoke_handler 注册）

**Interfaces:**
- Consumes: `commands::sound_snapshot_for`, `apply_sound_memory_inner`, `notify_sound_memories`
- Produces:
  - `list_sound_memories() -> Vec<SoundMemorySummary>`
  - `save_sound_memory(track, name, description?, tags?, id?) -> Result<SoundMemorySummary, String>`
  - `update_sound_memory(id, name?, description?, tags?, role?) -> Result<SoundMemorySummary, String>`
  - `delete_sound_memory(id) -> Result<(), String>`
  - `apply_sound_memory(id, track) -> Result<Project, String>`

- [ ] **Step 1: 加 Tauri 命令**

在 `delete_preset` 后面加：

```rust
// ---------------------------------------------------------------------------
// 音色记忆库（跨工程）
// ---------------------------------------------------------------------------

/// 全部音色记忆的摘要（不含参数/状态，界面列表用）。
#[tauri::command]
pub fn list_sound_memories(state: State<'_, AppState>) -> Vec<crate::sound_memories::SoundMemorySummary> {
    state.sound_memories.lock().unwrap().summaries()
}

/// 把当前轨道的音色链存成一条记忆；传 `id` 表示覆盖该条记忆的内容。
#[tauri::command]
pub fn save_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    track: u64,
    name: String,
    description: Option<String>,
    tags: Option<Vec<String>>,
    id: Option<String>,
) -> Result<crate::sound_memories::SoundMemorySummary, String> {
    let snapshot = sound_snapshot_for(&state, TrackId(track))?;
    let meta = MemoryMeta {
        name,
        description: description.unwrap_or_default(),
        tags: tags.unwrap_or_default(),
        role: snapshot.role.clone(),
    };
    let memory = {
        let mut store = state.sound_memories.lock().unwrap();
        match id {
            Some(id) => store.replace(&id, meta, snapshot)?,
            None => store.create(meta, snapshot)?,
        }
    };
    notify_sound_memories(&app);
    Ok(memory.summary())
}

/// 只修改一条记忆的元数据（名称/描述/标签/分工），不动声音快照。
#[tauri::command]
pub fn update_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    role: Option<String>,
) -> Result<crate::sound_memories::SoundMemorySummary, String> {
    let memory = state.sound_memories.lock().unwrap().update_metadata(
        &id,
        name.as_deref(),
        description.as_deref(),
        tags,
        role.as_deref(),
    )?;
    notify_sound_memories(&app);
    Ok(memory.summary())
}

/// 删除一条音色记忆。
#[tauri::command]
pub fn delete_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.sound_memories.lock().unwrap().remove(&id)?;
    notify_sound_memories(&app);
    Ok(())
}

/// 把一条音色记忆应用到轨道（一次事务：乐器 + 效果链 + 增益/声像）。
#[tauri::command]
pub fn apply_sound_memory(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    track: u64,
) -> Result<Project, String> {
    let memory = state
        .sound_memories
        .lock()
        .unwrap()
        .get(&id)
        .ok_or_else(|| format!("音色记忆不存在：{id}"))?;
    let (project, events) =
        apply_sound_memory_inner(&state, TrackId(track), &memory, Actor::Human);
    for event in &events {
        let _ = app.emit(EVENT_ENGINE, event);
    }
    state.audio.sync_events(&project, &events);
    Ok(project)
}
```

- [ ] **Step 2: 注册命令**

在 `lib.rs` 的 `invoke_handler` 里，`commands::delete_preset` 后面加：

```rust
            commands::list_sound_memories,
            commands::save_sound_memory,
            commands::update_sound_memory,
            commands::delete_sound_memory,
            commands::apply_sound_memory,
```

- [ ] **Step 3: 编译并跑现有测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Expected: 全部通过。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -F - <<'EOF'
feat: 音色记忆 Tauri 命令

列表/保存/元数据更新/删除/应用五个命令；写操作广播
engine://sound-memories，应用走一次事务并返回完整工程。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 5: MCP 工具、技能文档与 AI 行为规则

**Files:**
- Modify: `src-tauri/src/mcp.rs:591-618`（参数结构体）
- Modify: `src-tauri/src/mcp.rs:3536-3566`（工具实现）
- Modify: `src-tauri/src/mcp.rs:3665-3700`（工具定义）
- Modify: `src-tauri/src/mcp.rs:3804-3808`（dispatch）
- Modify: `src-tauri/src/mcp.rs:4401-4405`（ToolRouter 方法）
- Modify: `src-tauri/src/mcp.rs:4658-4758`（工具守卫测试）
- Modify: `src-tauri/src/mcp.rs:492-521`（HelpArgs 文档 + SKILL_DOCS）
- Modify: `src-tauri/src/mcp.rs:4986-4993`（技能域测试）
- Create: `skills/aidaw/references/sound-memory.md`
- Modify: `skills/aidaw/SKILL.md`
- Modify: `skills/aidaw/references/sound-design.md:34-40`
- Modify: `src-tauri/src/agent/mod.rs:49-58`（系统提示词）

**Interfaces:**
- Consumes: `commands::sound_snapshot_for`, `apply_sound_memory_agent_as`, `notify_sound_memories`, `crate::sound_memories::SoundMemoryStore`
- Produces: 6 个 MCP 工具 `list_sound_memories` / `get_sound_memory` / `save_sound_memory` / `update_sound_memory` / `delete_sound_memory` / `apply_sound_memory`
- Produces: `aidaw_help("sound-memory")`

- [ ] **Step 1: 加参数结构体与过滤函数（带测试）**

在 `mcp.rs` 顶部 import 区加：

```rust
use crate::sound_memories::MemoryMeta;
```

在 `PresetIdArgs` 后面加：

```rust
/// 列出音色记忆（只返回摘要）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ListSoundMemoriesArgs {
    /// 关键词：匹配名称、描述、标签、分工、乐器/效果器 plugin id。
    #[serde(default)]
    query: Option<String>,
    /// 精确匹配一个标签（大小写不敏感）。
    #[serde(default)]
    tag: Option<String>,
    /// 只看使用这个插件的记忆（乐器或效果链里出现即可）。
    #[serde(default)]
    plugin_id: Option<String>,
    /// 最多返回多少条；缺省 50，上限 200。
    #[serde(default)]
    limit: Option<usize>,
}

/// 读取一条音色记忆的完整快照。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SoundMemoryIdArgs {
    /// 记忆 id（用 list_sound_memories 查）。
    id: String,
}

/// 把当前轨道的音色链存成记忆。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct SaveSoundMemoryArgs {
    /// 源轨道 id（后端直接读它的乐器/效果链/增益声像）。
    track: u64,
    /// 记忆名称（不能为空）。
    name: String,
    /// 一句话描述（建议写听感/用途）。
    #[serde(default)]
    description: Option<String>,
    /// 标签（角色 / 风格 / 听感，2–5 个）。
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 传了就是覆盖这条已有记忆的内容；不传 = 新建。
    #[serde(default)]
    id: Option<String>,
}

/// 只改音色记忆的元数据。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct UpdateSoundMemoryArgs {
    /// 记忆 id。
    id: String,
    /// 新名称；省略 = 不改。
    #[serde(default)]
    name: Option<String>,
    /// 新描述；省略 = 不改。
    #[serde(default)]
    description: Option<String>,
    /// 新标签（整体替换）；省略 = 不改。
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// 新分工；省略 = 不改，空字符串 = 清除。
    #[serde(default)]
    role: Option<String>,
}

/// 把一条音色记忆应用到轨道。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub(crate) struct ApplySoundMemoryArgs {
    /// 记忆 id。
    id: String,
    /// 目标轨道 id。
    track: u64,
}
```

在 `tools` 模块里（`delete_preset` 后面）加纯过滤函数：

```rust
    /// 按 query / tag / plugin_id / limit 过滤摘要。抽成纯函数方便测试。
    pub(crate) fn filter_sound_memories(
        items: Vec<crate::sound_memories::SoundMemorySummary>,
        args: &ListSoundMemoriesArgs,
    ) -> Vec<crate::sound_memories::SoundMemorySummary> {
        let query = args.query.as_deref().unwrap_or("").trim().to_lowercase();
        let tag = args.tag.as_deref().unwrap_or("").trim().to_lowercase();
        let plugin_id = args.plugin_id.as_deref().unwrap_or("").trim().to_string();
        let limit = args.limit.unwrap_or(50).min(200);
        items
            .into_iter()
            .filter(|m| {
                if !plugin_id.is_empty()
                    && m.instrument_plugin_id.as_deref() != Some(plugin_id.as_str())
                    && !m.effect_plugin_ids.iter().any(|id| id == &plugin_id)
                {
                    return false;
                }
                if !tag.is_empty() && !m.tags.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                    return false;
                }
                if query.is_empty() {
                    return true;
                }
                let haystack = format!(
                    "{} {} {} {} {}",
                    m.name,
                    m.description,
                    m.tags.join(" "),
                    m.role.as_deref().unwrap_or(""),
                    m.instrument_plugin_id.as_deref().unwrap_or(""),
                )
                .to_lowercase();
                haystack.contains(&query)
            })
            .take(limit)
            .collect()
    }
```

- [ ] **Step 2: 加过滤测试**

在 `mcp.rs` 的 `mod tests` 里加：

```rust
    /// 音色记忆的检索：关键词、标签、插件都能组合过滤。
    #[test]
    fn sound_memory_filters_by_query_tag_and_plugin() {
        use crate::sound_memories::{SoundMemorySummary, SoundMixer};

        let summary = |id: &str, name: &str, tags: &[&str], inst: &str, effects: &[&str]| {
            SoundMemorySummary {
                id: id.into(),
                name: name.into(),
                description: format!("{name} description"),
                tags: tags.iter().map(|t| (*t).to_string()).collect(),
                role: Some("pad".into()),
                instrument_plugin_id: Some(inst.into()),
                effect_plugin_ids: effects.iter().map(|e| (*e).to_string()).collect(),
                mixer: SoundMixer { gain_db: -6.0, pan: 0.0 },
                source_project: None,
                created_at: 0,
                updated_at: 0,
            }
        };
        let items = vec![
            summary("s1", "warm pad", &["pad", "warm"], "builtin.synth", &["builtin.reverb"]),
            summary("s2", "acid bass", &["bass", "acid"], "builtin.synth", &["builtin.delay"]),
            summary("s3", "dry piano", &["keys"], "builtin.piano", &[]),
        ];

        let by_query = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: Some("WARM".into()),
                tag: None,
                plugin_id: None,
                limit: None,
            },
        );
        assert_eq!(by_query.len(), 1);
        assert_eq!(by_query[0].id, "s1");

        let by_tag = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: None,
                tag: Some("bass".into()),
                plugin_id: None,
                limit: None,
            },
        );
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].id, "s2");

        let by_plugin = tools::filter_sound_memories(
            items.clone(),
            &ListSoundMemoriesArgs {
                query: None,
                tag: None,
                plugin_id: Some("builtin.reverb".into()),
                limit: None,
            },
        );
        assert_eq!(by_plugin.len(), 1);
        assert_eq!(by_plugin[0].id, "s1");

        let limited = tools::filter_sound_memories(
            items,
            &ListSoundMemoriesArgs {
                query: None,
                tag: None,
                plugin_id: None,
                limit: Some(2),
            },
        );
        assert_eq!(limited.len(), 2);
    }
```

- [ ] **Step 3: 加工具实现**

在 `delete_preset` 后面加：

```rust
    // ---- 音色记忆库 ----

    pub(crate) fn list_sound_memories(
        app: &AppHandle,
        args: ListSoundMemoriesArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let all = state.sound_memories.lock().unwrap().summaries();
        let total = all.len();
        let memories = filter_sound_memories(all, &args);
        json_s(&serde_json::json!({
            "total": total,
            "returned": memories.len(),
            "memories": memories,
        }))
    }

    pub(crate) fn get_sound_memory(app: &AppHandle, args: SoundMemoryIdArgs) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state
            .sound_memories
            .lock()
            .unwrap()
            .get(&args.id)
            .ok_or_else(|| format!("音色记忆不存在：{}", args.id))?;
        json_s(&memory)
    }

    pub(crate) fn save_sound_memory(
        app: &AppHandle,
        args: SaveSoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let snapshot = commands::sound_snapshot_for(&state, TrackId(args.track))?;
        let meta = MemoryMeta {
            name: args.name,
            description: args.description.unwrap_or_default(),
            tags: args.tags.unwrap_or_default(),
            role: snapshot.role.clone(),
        };
        let memory = {
            let mut store = state.sound_memories.lock().unwrap();
            match args.id {
                Some(id) => store.replace(&id, meta, snapshot)?,
                None => store.create(meta, snapshot)?,
            }
        };
        commands::notify_sound_memories(app);
        json_s(&memory.summary())
    }

    pub(crate) fn update_sound_memory(
        app: &AppHandle,
        args: UpdateSoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state.sound_memories.lock().unwrap().update_metadata(
            &args.id,
            args.name.as_deref(),
            args.description.as_deref(),
            args.tags,
            args.role.as_deref(),
        )?;
        commands::notify_sound_memories(app);
        json_s(&memory.summary())
    }

    pub(crate) fn delete_sound_memory(
        app: &AppHandle,
        args: SoundMemoryIdArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        state.sound_memories.lock().unwrap().remove(&args.id)?;
        commands::notify_sound_memories(app);
        Ok(format!("音色记忆 {} 已删除", args.id))
    }

    pub(crate) fn apply_sound_memory(
        app: &AppHandle,
        actor: &Actor,
        args: ApplySoundMemoryArgs,
    ) -> Result<String, String> {
        let state = app.state::<AppState>();
        let memory = state
            .sound_memories
            .lock()
            .unwrap()
            .get(&args.id)
            .ok_or_else(|| format!("音色记忆不存在：{}", args.id))?;
        let track = TrackId(args.track);
        let (project, _events) =
            commands::apply_sound_memory_agent_as(app, actor.clone(), track, &memory);
        let t = project
            .tracks
            .iter()
            .find(|t| t.id == track)
            .ok_or_else(|| format!("轨道 {} 不存在", args.track))?;
        let instrument = t
            .instrument
            .as_ref()
            .map(|i| i.plugin_id.clone())
            .unwrap_or_else(|| "无".into());
        let chain = if t.effects.is_empty() {
            "空".into()
        } else {
            t.effects
                .iter()
                .map(|e| e.plugin_id.clone())
                .collect::<Vec<_>>()
                .join(" → ")
        };
        Ok(format!(
            "已把音色记忆「{}」应用到轨道 {}：乐器 {}，效果链 [{}]，增益 {:.1} dB，声像 {:.2}（一个撤销步）",
            memory.name, args.track, instrument, chain, t.mixer.gain_db, t.mixer.pan
        ))
    }
```

- [ ] **Step 4: 加工具定义与 dispatch**

在 `definitions()` 的 `delete_preset` 后面加：

```rust
            tool_def!("list_sound_memories", "列出用户存过的音色记忆（跨工程）。只返回摘要；`query` 搜名称/描述/标签/分工/插件 id，`tag` 精确匹配标签，`plugin_id` 过滤乐器或效果链。做同类音色前先查这里。", ListSoundMemoriesArgs),
            tool_def!("get_sound_memory", "读取一条音色记忆的完整快照（乐器/效果链参数与二进制状态、增益/声像）。", SoundMemoryIdArgs),
            tool_def!("save_sound_memory", "把**当前轨道**的音色链存成一条音色记忆（后端从轨道抓快照，不需要逐字传参数）。传 `id` 表示覆盖该条记忆的内容。**只有用户明确要求或确认后才调用**。", SaveSoundMemoryArgs),
            tool_def!("update_sound_memory", "只修改音色记忆的元数据（名称/描述/标签/分工），不动声音快照。", UpdateSoundMemoryArgs),
            tool_def!("delete_sound_memory", "删除一条音色记忆。**只在用户明确要求时调用**。", SoundMemoryIdArgs),
            tool_def!("apply_sound_memory", "把一条音色记忆应用到轨道：一次事务写入乐器、整条效果链、增益/声像（一个撤销步）。", ApplySoundMemoryArgs),
```

`dispatch` 的 `"delete_preset"` 后面加：

```rust
            "list_sound_memories" => list_sound_memories(app, parse_args(args)?),
            "get_sound_memory" => get_sound_memory(app, parse_args(args)?),
            "save_sound_memory" => save_sound_memory(app, parse_args(args)?),
            "update_sound_memory" => update_sound_memory(app, parse_args(args)?),
            "delete_sound_memory" => delete_sound_memory(app, parse_args(args)?),
            "apply_sound_memory" => apply_sound_memory(app, actor, parse_args(args)?),
```

- [ ] **Step 5: 加 ToolRouter 方法**

在 `delete_preset` 的 `#[tool]` 方法后面加：

```rust
    #[tool(description = "列出用户存过的音色记忆（跨工程）。只返回摘要；`query` 搜名称/描述/标签/分工/插件 id，`tag` 精确匹配标签，`plugin_id` 过滤乐器或效果链。做同类音色前先查这里。")]
    fn list_sound_memories(&self, Parameters(args): Parameters<ListSoundMemoriesArgs>) -> Result<String, ErrorData> {
        tools::list_sound_memories(&self.app, args).map_err(fail)
    }

    #[tool(description = "读取一条音色记忆的完整快照（乐器/效果链参数与二进制状态、增益/声像）。")]
    fn get_sound_memory(&self, Parameters(args): Parameters<SoundMemoryIdArgs>) -> Result<String, ErrorData> {
        tools::get_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "把**当前轨道**的音色链存成一条音色记忆（后端从轨道抓快照，不需要逐字传参数）。传 `id` 表示覆盖该条记忆的内容。**只有用户明确要求或确认后才调用**。")]
    async fn save_sound_memory(&self, Parameters(args): Parameters<SaveSoundMemoryArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        tauri::async_runtime::spawn_blocking(move || tools::save_sound_memory(&app, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }

    #[tool(description = "只修改音色记忆的元数据（名称/描述/标签/分工），不动声音快照。")]
    fn update_sound_memory(&self, Parameters(args): Parameters<UpdateSoundMemoryArgs>) -> Result<String, ErrorData> {
        tools::update_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "删除一条音色记忆。**只在用户明确要求时调用**。")]
    fn delete_sound_memory(&self, Parameters(args): Parameters<SoundMemoryIdArgs>) -> Result<String, ErrorData> {
        tools::delete_sound_memory(&self.app, args).map_err(fail)
    }

    #[tool(description = "把一条音色记忆应用到轨道：一次事务写入乐器、整条效果链、增益/声像（一个撤销步）。")]
    async fn apply_sound_memory(&self, Parameters(args): Parameters<ApplySoundMemoryArgs>) -> Result<String, ErrorData> {
        let app = self.app.clone();
        let actor = self.actor();
        tauri::async_runtime::spawn_blocking(move || tools::apply_sound_memory(&app, &actor, args))
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(fail)
    }
```

- [ ] **Step 6: 更新工具守卫测试**

`every_tool_is_registered` 的 `expected` 数组里，`"delete_preset"` 后面加：

```rust
            "list_sound_memories",
            "get_sound_memory",
            "save_sound_memory",
            "update_sound_memory",
            "delete_sound_memory",
            "apply_sound_memory",
```

- [ ] **Step 7: 注册技能文档**

`SKILL_DOCS` 数组里 `("workflow", ...)` 后面加：

```rust
    ("sound-memory", include_str!("../../skills/aidaw/references/sound-memory.md")),
```

`HelpArgs` 的注释里补上 `sound-memory`：

```rust
    /// 能力域：`project` / `track` / `clip` / `note` / `composition` / `mix` / `audio` / `transport` /
    /// `analysis` / `plugin` / `sound-design` / `sound-memory` / `theory` / `rhythm` / `genres` / `workflow`；留空返回索引。
```

`skill_docs_cover_every_domain` 的 expected 数组里 `"sound-design"` 后面加 `"sound-memory"`。

- [ ] **Step 8: 写技能文档**

创建 `skills/aidaw/references/sound-memory.md`：

```markdown
# 域：音色记忆库（sound-memory）

用户捏出满意的音色后，可以把**整条音色链**存进记忆库：乐器参数 + 有序效果链 +
轨道增益/声像。记忆跨工程；下次做同类音色先查这里，别从零重捏。

## 工具

| 工具 | 用途 |
| --- | --- |
| `list_sound_memories(query?, tag?, plugin_id?, limit?)` | 查摘要（id / 名称 / 描述 / 标签 / 乐器 / 效果链） |
| `get_sound_memory(id)` | 看完整快照（含参数与插件状态） |
| `save_sound_memory(track, name, description?, tags?, id?)` | 从**当前轨道**抓快照；传 `id` = 覆盖已有记忆的内容 |
| `update_sound_memory(id, name?, description?, tags?, role?)` | 只改元数据，不动声音 |
| `delete_sound_memory(id)` | 删除 |
| `apply_sound_memory(id, track)` | 应用到轨道（一个撤销步） |

`save_sound_memory` **不需要**传参数或效果链：后端直接读轨道，参数再多也不会漏。

## 铁律：默认不保存

- 只有用户**明确要求**（「记住这个音色」「存进音色库」）才保存。
- 用户表达满意（「这个音色真好」「就用这个」）时，可以问**一次**
  「要存进音色库吗？」；用户没确认就不存，也不要反复追问。
- 绝不自动删除 / 清理；`delete_sound_memory` 只响应用户明确要求。
- 保存时补一句中文 `description` 和 2–5 个 `tags`（角色 / 风格 / 听感，例如
  `pad`、`warm`、`wide`、`cinematic`），不要只写插件名。

## 典型用法

```text
# 用户说「把这个音色记住」
save_sound_memory(track=3, name="温暖电影感 Pad",
  description="慢起音、宽立体声，适合副歌铺底", tags=["pad","warm","wide"])

# 下次要同类音色：先查
list_sound_memories(query="温暖 pad")

# 应用到当前轨道
apply_sound_memory(id="s1760000000000", track=5)
```

## 应用语义

- 一次事务写入：乐器、整条效果链、`gain_db` / `pan`，只占**一个撤销步**。
- 效果链是**整体替换**，不是叠加。
- 不保存发送、自动化、片段、静音/独奏。
- 外部插件的二进制状态会尽量恢复；它不参与撤销/重做重放（引擎现有语义）。
- 快照里的插件在本机不存在时，应用后会得到一条「缺失插件」轨，和打开老工程一样。

## 查找建议

- 按听感 / 角色搜：`query="warm"`、`tag="pad"`。
- 按乐器或效果器筛：`plugin_id="builtin.synth"` / `plugin_id="builtin.reverb"`。
- 列表只给摘要；要参数细节再 `get_sound_memory(id)`。
```

- [ ] **Step 9: 更新 SKILL.md 与 sound-design.md**

`skills/aidaw/SKILL.md`：

1. frontmatter 的 `description` 里，`内置合成器音色设计` 后面加 `、**跨工程音色记忆库**`。
2. 能力域表格里加一行：

```markdown
| 音色记忆 | `references/sound-memory.md` | **跨工程保存 / 检索 / 应用整条音色链**；用户明确要求或确认后才保存，默认不自动存 |
```

3. 「约定与坑」里加：

```markdown
- **音色记忆默认不保存**：只有用户明确说「记住这个音色」，或你问过且用户确认后，才
  `save_sound_memory`。用户表达满意时可以问一次，别反复追问；**绝不自动删除**。
  用户要某类音色时，先 `list_sound_memories(query=...)` 查他存过的。
```

`skills/aidaw/references/sound-design.md` 的「一步到位的工作流」第 6 步后面加第 7 步：

```markdown
7. **满意后问用户要不要存进音色库**：只有用户明确要求或确认后，才
   `save_sound_memory(track, name, description, tags)`；默认绝不自动保存。
   下次做同类音色先 `list_sound_memories(query=...)`。
```

- [ ] **Step 10: 更新内置智能体系统提示词**

把 `src-tauri/src/agent/mod.rs::system_prompt` 的 `format!` 字符串改成：

```rust
    format!(
        "你是内嵌在 AI DAW 里的编曲助手，通过工具操纵当前工程（轨道、片段、音符、混音、\
生成、分析等）。先用 get_project 了解现状；需要细则时调用 aidaw_help 取对应能力域的文档。\
挑乐器或效果器前先调 list_plugins 读它的分类与音色描述（别凭插件名猜——`JC303` 其实是\
单音酸性贝斯），单个插件的参数表用 describe_plugin 查。\
所有改动都会进入撤销历史（署名 chat），用户可随时撤销，所以放手去做、必要时也可撤销/重做。\
音色记忆：默认绝不自动保存。只有用户明确要求（「记住这个音色」），或用户表达满意后\
你询问并得到确认，才调用 save_sound_memory；不要每次调参后都存。删除记忆必须用户明确\
要求。用户要某类音色时，先 list_sound_memories 查他存过的。\
回复用简洁的中文，不要复述工具的原始 JSON。\n\n--- 能力域索引 ---\n{index}"
    )
```

- [ ] **Step 11: 运行测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib mcp
```

Expected: 全部通过（工具守卫、技能域、过滤测试）。

- [ ] **Step 12: 提交**

```bash
git add src-tauri/src/mcp.rs src-tauri/src/agent/mod.rs skills/aidaw/SKILL.md skills/aidaw/references/sound-memory.md skills/aidaw/references/sound-design.md
git commit -F - <<'EOF'
feat: 音色记忆 MCP 工具与技能文档

新增 6 个 MCP 工具：列表/读取/保存/更新/删除/应用。保存由后端从
轨道抓快照；默认不自动保存，用户满意时只提醒一次。技能包新增
sound-memory 能力域，并写入内置智能体系统提示词。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 6: 前端类型、API 与 mock

**Files:**
- Modify: `src/api/types.ts:468-477`（`Preset` 后面加类型）
- Modify: `src/api/tauri.ts:10-65`（import）、`:67-70`（事件常量）、`:1202-1236`（预设 API 后面加）
- Modify: `src/api/mock.ts:9-66`（import）、`:646-663`（localStorage 辅助）、`:1184-1211`（预设 mock 后面加）

**Interfaces:**
- Consumes: backend JSON shapes from Tasks 4/5
- Produces:
  - `SoundMixer` / `PluginSnapshot` / `SoundMemorySummary` / `SoundMemory` types
  - `listSoundMemories`, `updateSoundMemory`, `deleteSoundMemory`, `applySoundMemory`
  - `EVENT_SOUND_MEMORIES`, `onSoundMemoriesEvent`
  - mock 对应实现

- [ ] **Step 1: 加前端类型**

`src/api/types.ts` 的 `Preset` 后面加：

```ts
/** 音色记忆的混音部分（不含发送）。 */
export interface SoundMixer {
  gain_db: number;
  pan: number;
}

/** 音色记忆里的一个插件快照。`state` 是 base64；空字符串 = 无二进制状态。 */
export interface PluginSnapshot {
  plugin_id: string;
  params: Record<string, number>;
  state: string;
}

/** 列表用的摘要（不含 params / state）。 */
export interface SoundMemorySummary {
  id: string;
  name: string;
  description: string;
  tags: string[];
  role: string | null;
  instrument_plugin_id: string | null;
  effect_plugin_ids: string[];
  mixer: SoundMixer;
  source_project: string | null;
  created_at: number;
  updated_at: number;
}

/** 一条完整的音色记忆。 */
export interface SoundMemory {
  id: string;
  name: string;
  description: string;
  tags: string[];
  role: string | null;
  instrument: PluginSnapshot | null;
  effects: PluginSnapshot[];
  mixer: SoundMixer;
  source_project: string | null;
  created_at: number;
  updated_at: number;
}
```

- [ ] **Step 2: 加 tauri API 与事件**

`src/api/tauri.ts` 的 import 类型列表里加 `SoundMemorySummary`。事件常量区加：

```ts
/** 音色记忆库变更广播通道名，与后端 `state::EVENT_SOUND_MEMORIES` 保持一致。 */
export const EVENT_SOUND_MEMORIES = "engine://sound-memories";
```

在 `deletePreset` 后面加：

```ts
// ---------------------------------------------------------------------------
// 音色记忆库（跨工程）
// ---------------------------------------------------------------------------

/** 全部音色记忆的摘要。 */
export function listSoundMemories(): Promise<SoundMemorySummary[]> {
  if (mockEnabled()) return Promise.resolve(mockApi.listSoundMemories());
  return invoke<SoundMemorySummary[]>("list_sound_memories");
}

/** 只修改元数据（名称 / 描述 / 标签 / 分工）。 */
export function updateSoundMemory(
  id: string,
  patch: { name?: string; description?: string; tags?: string[]; role?: string },
): Promise<SoundMemorySummary> {
  if (mockEnabled()) return Promise.resolve(mockApi.updateSoundMemory(id, patch));
  return invoke<SoundMemorySummary>("update_sound_memory", {
    id,
    name: patch.name ?? null,
    description: patch.description ?? null,
    tags: patch.tags ?? null,
    role: patch.role ?? null,
  });
}

/** 删除一条音色记忆。 */
export function deleteSoundMemory(id: string): Promise<void> {
  if (mockEnabled()) return Promise.resolve(mockApi.deleteSoundMemory(id));
  return invoke<void>("delete_sound_memory", { id });
}

/** 把一条音色记忆应用到轨道（一次事务）。 */
export function applySoundMemory(id: string, track: number): Promise<Project> {
  if (mockEnabled()) return Promise.resolve(mockApi.applySoundMemory(id, track));
  return invoke<Project>("apply_sound_memory", { id, track });
}

/** 订阅音色记忆库变更（AI 保存/删除后前端自动刷新）。 */
export function onSoundMemoriesEvent(handler: () => void): Promise<UnlistenFn> {
  if (mockEnabled()) return Promise.resolve(() => {});
  return listen(EVENT_SOUND_MEMORIES, () => handler());
}
```

- [ ] **Step 3: 加 mock 类型与 localStorage 辅助**

`src/api/mock.ts` 的 import 列表里加 `SoundMemory`, `SoundMemorySummary`。

在 `writePresets` 后面加：

```ts
/**
 * 音色记忆：浏览器预览里用 localStorage 顶替后端的 `sound-memories.json`。
 */
const SOUND_MEMORY_KEY = "ai-daw-sound-memories";

function readSoundMemories(): SoundMemory[] {
  try {
    return JSON.parse(localStorage.getItem(SOUND_MEMORY_KEY) ?? "[]") as SoundMemory[];
  } catch {
    return [];
  }
}

function writeSoundMemories(memories: SoundMemory[]): void {
  localStorage.setItem(SOUND_MEMORY_KEY, JSON.stringify(memories));
}

function toSoundSummary(memory: SoundMemory): SoundMemorySummary {
  return {
    id: memory.id,
    name: memory.name,
    description: memory.description,
    tags: memory.tags,
    role: memory.role,
    instrument_plugin_id: memory.instrument?.plugin_id ?? null,
    effect_plugin_ids: memory.effects.map((fx) => fx.plugin_id),
    mixer: memory.mixer,
    source_project: memory.source_project,
    created_at: memory.created_at,
    updated_at: memory.updated_at,
  };
}
```

- [ ] **Step 4: 加 mock 实现**

在 `deletePreset` 后面加：

```ts
  listSoundMemories: (): SoundMemorySummary[] => readSoundMemories().map(toSoundSummary),

  updateSoundMemory: (
    id: string,
    patch: { name?: string; description?: string; tags?: string[]; role?: string },
  ): SoundMemorySummary => {
    const all = readSoundMemories();
    const found = all.find((m) => m.id === id);
    if (!found) throw new Error(`音色记忆不存在：${id}`);
    if (patch.name !== undefined) {
      const name = patch.name.trim();
      if (!name) throw new Error("音色记忆的名字不能为空");
      found.name = name;
    }
    if (patch.description !== undefined) found.description = patch.description.trim();
    if (patch.tags !== undefined) found.tags = patch.tags.map((t) => t.trim()).filter(Boolean);
    if (patch.role !== undefined) found.role = patch.role.trim() || null;
    found.updated_at = Date.now();
    writeSoundMemories(all);
    return toSoundSummary(found);
  },

  deleteSoundMemory: (id: string): void => {
    const all = readSoundMemories();
    const next = all.filter((m) => m.id !== id);
    if (next.length === all.length) throw new Error(`音色记忆不存在：${id}`);
    writeSoundMemories(next);
  },

  applySoundMemory: (id: string, trackId: number): Project =>
    commit(() => {
      const memory = readSoundMemories().find((m) => m.id === id);
      if (!memory) throw new Error(`音色记忆不存在：${id}`);
      const t = track(trackId);
      if (!t) throw new Error(`轨道 ${trackId} 不存在`);
      t.instrument = memory.instrument ? { ...memory.instrument } : null;
      t.effects = memory.effects.map((fx) => ({ ...fx }));
      t.mixer.gain_db = memory.mixer.gain_db;
      t.mixer.pan = memory.mixer.pan;
    }),
```

- [ ] **Step 5: 类型检查**

Run:

```bash
pnpm build
```

Expected: 通过（此时 store/面板还没用这些函数，但类型和 mock 必须自洽）。

- [ ] **Step 6: 提交**

```bash
git add src/api/types.ts src/api/tauri.ts src/api/mock.ts
git commit -F - <<'EOF'
feat: 音色记忆前端类型与 API

新增 SoundMemory 类型、四个 Tauri 封装、变更事件订阅，以及浏览器
预览用的 localStorage mock。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 7: 音色库状态与右侧面板

**Files:**
- Modify: `src/stores/project.ts:118-125`（`setParams` 后面加 `applySoundMemory`）、`:527-595`（导出）
- Create: `src/stores/soundMemories.ts`
- Create: `src/components/workspace/panes/SoundLibraryPane.vue`
- Modify: `src/stores/layout.ts:10-41`（`PaneId` + `PANES`）
- Modify: `src/components/workspace/RightPanel.vue:6-39`（import + 图标）、`:62-68`（渲染）
- Modify: `src/i18n/zh.ts:482-487`、`src/i18n/en.ts:482-487`（面板与音色库文案）

**Interfaces:**
- Consumes: `api.listSoundMemories` / `updateSoundMemory` / `deleteSoundMemory` / `applySoundMemory`, `selection.track`
- Produces: `useSoundMemoriesStore`, `SoundLibraryPane.vue`, `PaneId = "sounds"`

- [ ] **Step 1: 给 project store 加应用方法**

`setParams` 后面加：

```ts
  /** 应用一条音色记忆（一次事务：乐器 + 效果链 + 增益/声像）。 */
  function applySoundMemory(id: string, track: number): Promise<void> {
    return edit(() => api.applySoundMemory(id, track));
  }
```

返回对象里 `setEffectParams` 后面加 `applySoundMemory,`。

- [ ] **Step 2: 创建音色库 store**

创建 `src/stores/soundMemories.ts`：

```ts
/**
 * 音色记忆库 store：列表、搜索、元数据编辑、删除。
 *
 * 保存只由 AI 触发（MCP），前端不提供保存入口；AI 保存后后端广播
 * `engine://sound-memories`，这里自动刷新。
 */
import { defineStore } from "pinia";
import { computed, ref } from "vue";

import {
  deleteSoundMemory,
  listSoundMemories,
  onSoundMemoriesEvent,
  updateSoundMemory,
} from "@/api/tauri";
import type { SoundMemorySummary } from "@/api/types";

export const useSoundMemoriesStore = defineStore("soundMemories", () => {
  const items = ref<SoundMemorySummary[]>([]);
  const loading = ref(false);
  const query = ref("");
  const activeTag = ref("");

  const tags = computed<string[]>(() => {
    const set = new Set<string>();
    for (const memory of items.value) {
      for (const tag of memory.tags) set.add(tag);
    }
    return [...set].sort();
  });

  const filtered = computed<SoundMemorySummary[]>(() => {
    const q = query.value.trim().toLowerCase();
    return items.value.filter((memory) => {
      if (activeTag.value && !memory.tags.includes(activeTag.value)) return false;
      if (!q) return true;
      const haystack = [
        memory.name,
        memory.description,
        memory.role ?? "",
        memory.instrument_plugin_id ?? "",
        ...memory.tags,
        ...memory.effect_plugin_ids,
      ]
        .join(" ")
        .toLowerCase();
      return haystack.includes(q);
    });
  });

  async function refresh(): Promise<void> {
    loading.value = true;
    try {
      items.value = await listSoundMemories();
    } finally {
      loading.value = false;
    }
  }

  async function updateMetadata(
    id: string,
    patch: { name?: string; description?: string; tags?: string[]; role?: string },
  ): Promise<void> {
    await updateSoundMemory(id, patch);
    await refresh();
  }

  async function remove(id: string): Promise<void> {
    await deleteSoundMemory(id);
    await refresh();
  }

  /** 订阅后端变更（返回取消订阅函数）。 */
  function watchChanges(): () => void {
    let stop: (() => void) | null = null;
    void onSoundMemoriesEvent(() => {
      void refresh();
    }).then((off) => {
      stop = off;
    });
    return () => stop?.();
  }

  return {
    items,
    loading,
    query,
    activeTag,
    tags,
    filtered,
    refresh,
    updateMetadata,
    remove,
    watchChanges,
  };
});
```

- [ ] **Step 3: 创建音色库面板**

创建 `src/components/workspace/panes/SoundLibraryPane.vue`：

```vue
<script setup lang="ts">
/**
 * 音色库：查看 AI 存下的整轨音色快照，应用到选中轨道，编辑元数据或删除。
 *
 * 保存入口不在这里——用户让 AI「记住这个音色」时才写入；本面板只做查看与清理。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  BookmarkSquareIcon,
  PencilSquareIcon,
  TrashIcon,
} from "@heroicons/vue/24/outline";

import { useSoundMemoriesStore } from "@/stores/soundMemories";
import { useProjectStore } from "@/stores/project";
import { useSelectionStore } from "@/stores/selection";
import { usePluginsStore } from "@/stores/plugins";
import { useLayoutStore } from "@/stores/layout";
import { useI18n } from "@/i18n";
import type { SoundMemorySummary } from "@/api/types";

const sounds = useSoundMemoriesStore();
const project = useProjectStore();
const selection = useSelectionStore();
const plugins = usePluginsStore();
const layout = useLayoutStore();
const { t } = useI18n();

const editingId = ref<string | null>(null);
const draftName = ref("");
const draftDescription = ref("");
const draftTags = ref("");
const confirmingId = ref<string | null>(null);
const busyId = ref<string | null>(null);
let stop: (() => void) | null = null;

const track = computed(() => selection.track);

function pluginName(id: string | null): string {
  return id ? plugins.name(id) : t("common.none");
}

function chainLabel(memory: SoundMemorySummary): string {
  return memory.effect_plugin_ids.length === 0
    ? t("sounds.noEffects")
    : memory.effect_plugin_ids.map((id) => plugins.name(id)).join(" → ");
}

function dateLabel(ms: number): string {
  return new Date(ms).toLocaleDateString();
}

async function apply(memory: SoundMemorySummary): Promise<void> {
  const target = track.value;
  if (!target) {
    layout.notify(t("sounds.selectTrackFirst"));
    return;
  }
  busyId.value = memory.id;
  try {
    await project.applySoundMemory(memory.id, target.id);
    layout.notify(t("sounds.applied", { name: memory.name }));
  } catch (err) {
    layout.notify(t("sounds.applyFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

function startEdit(memory: SoundMemorySummary): void {
  editingId.value = memory.id;
  draftName.value = memory.name;
  draftDescription.value = memory.description;
  draftTags.value = memory.tags.join(", ");
}

async function commitEdit(memory: SoundMemorySummary): Promise<void> {
  const name = draftName.value.trim();
  if (!name) return;
  busyId.value = memory.id;
  try {
    await sounds.updateMetadata(memory.id, {
      name,
      description: draftDescription.value.trim(),
      tags: draftTags.value
        .split(",")
        .map((tag) => tag.trim())
        .filter(Boolean),
    });
    editingId.value = null;
  } catch (err) {
    layout.notify(t("sounds.saveFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

async function remove(memory: SoundMemorySummary): Promise<void> {
  busyId.value = memory.id;
  try {
    await sounds.remove(memory.id);
    confirmingId.value = null;
  } catch (err) {
    layout.notify(t("sounds.deleteFailed", { error: String(err) }));
  } finally {
    busyId.value = null;
  }
}

onMounted(() => {
  void sounds.refresh();
  stop = sounds.watchChanges();
});
onUnmounted(() => stop?.());
</script>

<template>
  <section class="sound-library">
    <header class="head">
      <BookmarkSquareIcon class="head-icon" />
      <div class="head-text">
        <h2>{{ t("sounds.title") }}</h2>
        <p>{{ t("sounds.count", { count: sounds.filtered.length }) }}</p>
      </div>
    </header>

    <div class="toolbar">
      <input
        v-model="sounds.query"
        class="search md3-field"
        type="search"
        :placeholder="t('sounds.search')"
      />
    </div>

    <div v-if="sounds.tags.length" class="tags">
      <button
        class="chip"
        :class="{ active: sounds.activeTag === '' }"
        @click="sounds.activeTag = ''"
      >
        {{ t("sounds.tagAll") }}
      </button>
      <button
        v-for="tag in sounds.tags"
        :key="tag"
        class="chip"
        :class="{ active: sounds.activeTag === tag }"
        @click="sounds.activeTag = tag"
      >
        {{ tag }}
      </button>
    </div>

    <div class="list">
      <div v-if="sounds.loading" class="empty">{{ t("common.loading") }}</div>
      <div v-else-if="sounds.filtered.length === 0" class="empty">
        <p>{{ t("sounds.emptyTitle") }}</p>
        <p class="hint">{{ t("sounds.emptyHint") }}</p>
      </div>

      <article v-for="memory in sounds.filtered" :key="memory.id" class="card">
        <template v-if="editingId === memory.id">
          <input v-model="draftName" class="md3-field" :placeholder="t('sounds.name')" />
          <textarea
            v-model="draftDescription"
            class="md3-field"
            rows="2"
            :placeholder="t('sounds.description')"
          />
          <input v-model="draftTags" class="md3-field" :placeholder="t('sounds.tags')" />
          <div class="actions">
            <button
              class="btn primary"
              :disabled="!draftName.trim() || busyId === memory.id"
              @click="commitEdit(memory)"
            >
              {{ t("common.save") }}
            </button>
            <button class="btn" @click="editingId = null">{{ t("common.cancel") }}</button>
          </div>
        </template>

        <template v-else>
          <div class="card-head">
            <h3>{{ memory.name }}</h3>
            <span v-if="memory.role" class="role">{{ memory.role }}</span>
          </div>
          <p v-if="memory.description" class="desc">{{ memory.description }}</p>
          <p class="meta">{{ t("sounds.instrument") }}：{{ pluginName(memory.instrument_plugin_id) }}</p>
          <p class="meta">{{ t("sounds.effects") }}：{{ chainLabel(memory) }}</p>
          <p class="meta">
            {{ t("sounds.mixer", { gain: memory.mixer.gain_db.toFixed(1), pan: memory.mixer.pan.toFixed(2) }) }}
          </p>
          <div v-if="memory.tags.length" class="tag-row">
            <span v-for="tag in memory.tags" :key="tag" class="pill">{{ tag }}</span>
          </div>
          <p class="foot">
            <span>{{ t("sounds.savedAt", { date: dateLabel(memory.updated_at || memory.created_at) }) }}</span>
            <span v-if="memory.source_project">
              {{ t("sounds.sourceProject", { name: memory.source_project }) }}
            </span>
          </p>
          <div class="actions">
            <button
              class="btn primary"
              :disabled="!track || busyId === memory.id"
              :title="t('sounds.applyHint')"
              @click="apply(memory)"
            >
              {{ t("sounds.apply") }}
            </button>
            <button class="icon-btn" :title="t('sounds.edit')" @click="startEdit(memory)">
              <PencilSquareIcon />
            </button>
            <button
              v-if="confirmingId !== memory.id"
              class="icon-btn"
              :title="t('sounds.delete')"
              @click="confirmingId = memory.id"
            >
              <TrashIcon />
            </button>
            <button
              v-else
              class="btn danger"
              :disabled="busyId === memory.id"
              @click="remove(memory)"
            >
              {{ t("sounds.deleteConfirm") }}
            </button>
          </div>
        </template>
      </article>
    </div>
  </section>
</template>

<style scoped>
.sound-library {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.head {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 12px 6px;
}
.head-icon {
  width: 18px;
  height: 18px;
  color: rgb(var(--md-primary));
}
.head-text h2 {
  margin: 0;
  font-size: 13px;
  font-weight: 600;
}
.head-text p {
  margin: 1px 0 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.toolbar {
  padding: 0 12px 8px;
}
.search {
  width: 100%;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  padding: 0 12px 8px;
}
.chip,
.pill {
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 10px;
  padding: 2px 8px;
}
.chip {
  cursor: pointer;
}
.chip.active {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0 12px 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.empty {
  padding: 24px 8px;
  text-align: center;
  font-size: 12px;
  color: rgb(var(--md-on-surface-variant));
}
.empty .hint {
  margin-top: 6px;
  font-size: 11px;
  opacity: 0.8;
}
.card {
  border: 1px solid rgb(var(--md-outline-variant) / 0.6);
  border-radius: 10px;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.card-head {
  display: flex;
  align-items: center;
  gap: 6px;
}
.card-head h3 {
  margin: 0;
  font-size: 12px;
  font-weight: 600;
}
.role {
  font-size: 10px;
  color: rgb(var(--md-primary));
}
.desc {
  margin: 0;
  font-size: 11px;
  color: rgb(var(--md-on-surface-variant));
}
.meta {
  margin: 0;
  font-size: 10px;
  color: rgb(var(--md-on-surface-variant));
}
.tag-row {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.foot {
  margin: 2px 0 0;
  display: flex;
  justify-content: space-between;
  gap: 8px;
  font-size: 10px;
  color: rgb(var(--md-on-surface-variant));
}
.actions {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 4px;
}
.btn {
  height: 26px;
  padding: 0 10px;
  border: 1px solid rgb(var(--md-outline-variant) / 0.7);
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
}
.btn.primary {
  border-color: transparent;
  background-color: rgb(var(--md-primary));
  color: rgb(var(--md-on-primary));
}
.btn.danger {
  border-color: transparent;
  background-color: rgb(var(--md-error));
  color: rgb(var(--md-on-error));
}
.btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.icon-btn {
  width: 26px;
  height: 26px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 0;
  border-radius: 999px;
  background: transparent;
  color: rgb(var(--md-on-surface-variant));
  cursor: pointer;
}
.icon-btn svg {
  width: 14px;
  height: 14px;
}
.icon-btn:hover {
  background-color: rgb(var(--md-on-surface) / var(--md-state-hover-alpha));
  color: rgb(var(--md-on-surface));
}
</style>
```

- [ ] **Step 4: 注册面板**

`src/stores/layout.ts` 的 `PaneId` 加 `| "sounds"`，`PANES` 在 `manager` 后面加：

```ts
  { id: "sounds", labelKey: "panes.sounds", hintKey: "panes.soundsHint" },
```

`src/components/workspace/RightPanel.vue`：

- import 图标：在 `@heroicons/vue/24/outline` 里加 `BookmarkSquareIcon`
- import 面板：`import SoundLibraryPane from "./panes/SoundLibraryPane.vue";`
- `ICONS` 加 `sounds: BookmarkSquareIcon,`
- 模板里 `ManagerPane` 后面加：

```vue
      <SoundLibraryPane v-else-if="layout.activeTab === 'sounds'" />
```

- [ ] **Step 5: 补 i18n**

`src/i18n/zh.ts` 的 `panes.managerHint` 后面加：

```ts
  "panes.sounds": "音色库",
  "panes.soundsHint": "AI 与你保存的音色快照",
  // ---- 音色记忆库 ----
  "sounds.title": "音色库",
  "sounds.search": "搜索音色、标签、插件…",
  "sounds.count": "{count} 条",
  "sounds.tagAll": "全部标签",
  "sounds.emptyTitle": "还没有音色记忆",
  "sounds.emptyHint": "在对话里让 AI 捏好音色后说「记住这个音色」，它就会出现在这里。",
  "sounds.apply": "应用到选中轨道",
  "sounds.applyHint": "把乐器、效果链、增益/声像写入当前轨道（一个撤销步）",
  "sounds.selectTrackFirst": "先选中一条轨道",
  "sounds.applied": "已应用「{name}」，可撤销",
  "sounds.edit": "编辑",
  "sounds.delete": "删除",
  "sounds.deleteConfirm": "确认删除？",
  "sounds.name": "名称",
  "sounds.description": "描述",
  "sounds.tags": "标签（逗号分隔）",
  "sounds.instrument": "乐器",
  "sounds.effects": "效果链",
  "sounds.noEffects": "无",
  "sounds.mixer": "增益 {gain} dB · 声像 {pan}",
  "sounds.savedAt": "更新于 {date}",
  "sounds.sourceProject": "来自「{name}」",
  "sounds.saveFailed": "更新失败：{error}",
  "sounds.deleteFailed": "删除失败：{error}",
  "sounds.applyFailed": "应用失败：{error}",
```

`src/i18n/en.ts` 的 `panes.managerHint` 后面加：

```ts
  "panes.sounds": "Sounds",
  "panes.soundsHint": "Sound snapshots saved by you and the AI",
  // ---- Sound library ----
  "sounds.title": "Sound Library",
  "sounds.search": "Search sounds, tags, plugins…",
  "sounds.count": "{count}",
  "sounds.tagAll": "All tags",
  "sounds.emptyTitle": "No sound memories yet",
  "sounds.emptyHint": "Ask the AI to remember a sound after shaping it; it will show up here.",
  "sounds.apply": "Apply to selected track",
  "sounds.applyHint": "Write the instrument, effect chain and gain/pan to the current track (one undo step)",
  "sounds.selectTrackFirst": "Select a track first",
  "sounds.applied": "Applied “{name}”. Undo is available.",
  "sounds.edit": "Edit",
  "sounds.delete": "Delete",
  "sounds.deleteConfirm": "Confirm delete?",
  "sounds.name": "Name",
  "sounds.description": "Description",
  "sounds.tags": "Tags (comma separated)",
  "sounds.instrument": "Instrument",
  "sounds.effects": "Effect chain",
  "sounds.noEffects": "None",
  "sounds.mixer": "Gain {gain} dB · Pan {pan}",
  "sounds.savedAt": "Updated {date}",
  "sounds.sourceProject": "From “{name}”",
  "sounds.saveFailed": "Update failed: {error}",
  "sounds.deleteFailed": "Delete failed: {error}",
  "sounds.applyFailed": "Apply failed: {error}",
```

- [ ] **Step 6: 构建**

Run:

```bash
pnpm build
```

Expected: 通过（`vue-tsc` 无错误，vite 构建成功）。

- [ ] **Step 7: 提交**

```bash
git add src/stores/project.ts src/stores/soundMemories.ts src/components/workspace/panes/SoundLibraryPane.vue src/stores/layout.ts src/components/workspace/RightPanel.vue src/i18n/zh.ts src/i18n/en.ts
git commit -F - <<'EOF'
feat: 音色库面板

右侧新增「音色库」标签：搜索、标签筛选、应用到选中轨道、编辑
元数据、二次确认删除；AI 保存后通过事件自动刷新。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

### Task 8: 全量验证与收尾

**Files:**
- Modify: `docs/superpowers/specs/2026-10-21-sound-memory-library-design.md:4`（状态改为已实现）
- No code changes expected.

- [ ] **Step 1: 跑 Rust 全量测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Expected: 全部通过，无 `warning: unused`。

- [ ] **Step 2: 跑前端构建**

Run:

```bash
pnpm build
```

Expected: 通过。

- [ ] **Step 3: 手动验收（桌面端）**

1. 启动应用，让 AI 捏一个带 2 个效果器的音色，说「记住这个音色」。
2. 确认音色库面板出现该条，描述/标签/乐器/效果链正确。
3. 选中另一条轨道 → 应用该记忆 → 听感/参数/效果链/增益声像一致。
4. 撤销一次 → 乐器、效果链、增益声像全部恢复。
5. 编辑名称/描述/标签 → 刷新后仍在；删除 → 列表消失；重启应用不复活。
6. 对 AI 说「这个音色真好」但**不确认**保存 → 音色库不应新增。

- [ ] **Step 4: 更新设计文档状态**

把 `docs/superpowers/specs/2026-10-21-sound-memory-library-design.md` 的状态行改成：

```markdown
- 状态：✅ 已实现
```

- [ ] **Step 5: 提交**

```bash
git add docs/superpowers/specs/2026-10-21-sound-memory-library-design.md
git commit -F - <<'EOF'
docs: 音色记忆库标记为已实现

全量 Rust 测试与前端构建通过，手动验收覆盖保存/应用/撤销/删除。

Co-authored-by: CommandCodeBot <noreply@commandcode.ai>
EOF
```

---

## Self-Review

**Spec coverage**

- §4 数据模型 → Task 1
- §5.1 存储 / AppState / 启动加载 → Task 1、Task 2
- §5.2 保存快照 → Task 3（`sound_snapshot_for`）、Task 4（Tauri 命令）、Task 5（MCP）
- §5.3 应用快照 / 一个撤销步 → Task 3
- §5.4 Tauri 命令 → Task 4
- §6 MCP 6 个工具 / `aidaw_help` / 技能文档 → Task 5
- §7 前端数据层 / 面板 / i18n / 事件刷新 → Task 6、Task 7
- §8 AI 行为规则 → Task 5（技能文档 + 系统提示词）
- §9 测试 → Task 1、Task 3、Task 5、Task 8
- §10 边界 → 写入技能文档与设计文档，代码不实现发送/自动化

**Placeholder scan**

- 无 TBD / TODO；所有代码步骤都给了实际代码。
- `SoundLibraryPane.vue` 的样式是完整可编译的。

**Type consistency**

- `SoundMemorySummary` 在 Rust / TS / mock 中的字段名一致（`instrument_plugin_id`、`effect_plugin_ids`、`mixer`、`source_project`）。
- `PluginSnapshot.state` 在 Rust 是 base64 `String`，TS 是 `string`，mock 同形。
- `applySoundMemory` 在 project store 与 `api` 中的签名一致：`(id: string, track: number) -> Promise<Project>`。
- MCP 工具名与 `definitions()`、`dispatch()`、`ToolRouter`、守卫测试四处一致。
