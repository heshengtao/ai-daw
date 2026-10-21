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
            mixer: SoundMixer {
                gain_db: -6.0,
                pan: 0.0,
            },
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
        assert_eq!(
            summary.effect_plugin_ids,
            vec!["builtin.reverb".to_string()]
        );
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
        next.mixer = SoundMixer {
            gain_db: -12.0,
            pan: 0.5,
        };

        let replaced = store.replace(&saved.id, meta("new"), next).unwrap();
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
        assert!(store
            .update_metadata("missing", Some("x"), None, None, None)
            .is_err());
        assert!(store.remove("missing").is_err());
    }
}
