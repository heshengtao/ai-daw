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

/// 人声参考素材：歌声合成用的「音色」。
///
/// 和乐器快照存在**同一个音色库**里（只是 tags 里多一个「人声」，`voice` 非空），
/// 这样侧边栏的音色库就能统一浏览 / 改名 / 删除 / 按标签筛选，不需要第二套库。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceAsset {
    /// WAV 的绝对路径（单声道 24 kHz，存在 `<配置目录>/sound-memory-audio/`）。
    pub path: String,
    /// 时长（秒）。
    pub seconds: f64,
    /// 采样率（固定 24000）。
    pub sample_rate: u32,
    /// 帧数。
    pub frames: u64,
    /// 这段参考取自原音频的第几秒（自动裁剪的起点；0 = 从头）。
    #[serde(default)]
    pub offset_seconds: f64,
}

/// 人声参考的采样率（与歌声合成模型一致）。
pub const VOICE_RATE: u32 = 24_000;
/// 自动打上的标签：侧边栏按它筛选「人声」。
pub const VOICE_TAG: &str = "vocal";

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
    /// 人声参考素材；`Some` 时这条记忆是「人声音色」，没有乐器 / 效果链。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceAsset>,
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
            voice: self.voice.clone(),
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
    /// 人声参考（`Some` = 这条是人声音色）。
    pub voice: Option<VoiceAsset>,
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

    pub fn create(
        &mut self,
        meta: MemoryMeta,
        snapshot: SoundSnapshot,
    ) -> Result<SoundMemory, String> {
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
            voice: None,
            created_at: now,
            updated_at: now,
        };
        self.memories.push(memory.clone());
        self.flush()?;
        Ok(memory)
    }

    /// 存一条**人声音色**：写 WAV + 一条带 `voice` 的音色记忆（自动加「人声」标签）。
    ///
    /// `samples` 必须是单声道 24 kHz；入库前做峰值归一（与乐器链的存法一样，跨工程听起来
    /// 电平一致）。WAV 落在 `sound-memory-audio/<id>.wav`。
    pub fn create_voice(
        &mut self,
        meta: MemoryMeta,
        samples: &[f32],
        offset_seconds: f64,
    ) -> Result<SoundMemory, String> {
        if samples.is_empty() {
            return Err("参考音频是空的".into());
        }
        let mut meta = normalize_meta(meta)?;
        if !meta.tags.iter().any(|t| t.eq_ignore_ascii_case(VOICE_TAG)) {
            meta.tags.push(VOICE_TAG.to_string());
        }
        let now = now_millis();
        let id = unique_id(&self.memories, now);

        let dir = self.audio_dir().ok_or("拿不到音色库目录")?;
        fs::create_dir_all(&dir).map_err(|e| format!("创建人声目录失败：{e}"))?;
        let path = dir.join(format!("{id}.wav"));

        let peak = samples.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let gain = if peak > 0.0 { 0.891 / peak } else { 1.0 };
        let max_frames = (VOICE_RATE as usize) * 600;
        let mut pcm: Vec<f32> = samples.iter().take(max_frames).map(|v| v * gain).collect();
        if pcm.is_empty() {
            return Err("参考音频太短".into());
        }
        let mut buffer = ai_daw_engine::audio::AudioBuffer::silence(1, pcm.len());
        buffer.data_mut().copy_from_slice(&pcm);
        ai_daw_engine::wav::write_wav(&path, &buffer, VOICE_RATE)
            .map_err(|e| format!("写人声音色失败：{e}"))?;
        pcm.clear();

        let memory = SoundMemory {
            id,
            name: meta.name,
            description: meta.description,
            tags: meta.tags,
            role: meta.role,
            instrument: None,
            effects: Vec::new(),
            mixer: SoundMixer {
                gain_db: 0.0,
                pan: 0.0,
            },
            source_project: None,
            voice: Some(VoiceAsset {
                path: path.to_string_lossy().into_owned(),
                seconds: (buffer.frames() as f64) / VOICE_RATE as f64,
                sample_rate: VOICE_RATE,
                frames: buffer.frames() as u64,
                offset_seconds: offset_seconds.max(0.0),
            }),
            created_at: now,
            updated_at: now,
        };
        self.memories.push(memory.clone());
        if let Err(err) = self.flush() {
            let _ = fs::remove_file(&path);
            self.memories.retain(|m| m.id != memory.id);
            return Err(err);
        }
        Ok(memory)
    }

    /// 把**过长的人声参考**裁成 10 秒（自动挑一段最像人声的）。
    ///
    /// 早期版本允许把整首歌存进音色库（例如 4 分 49 秒）；那既占空间，又让 prompt 落进
    /// 前奏/间奏。启动时做一次就地迁移：解码 → 选段 → 覆写同一个 WAV → 更新时长 / 起点。
    /// 已经 ≤ `max_seconds` 的条目不动。返回迁移了几条。
    pub fn trim_long_voices(&mut self, max_seconds: f64) -> usize {
        let mut changed = 0usize;
        for memory in &mut self.memories {
            let Some(voice) = &mut memory.voice else {
                continue;
            };
            if voice.seconds <= max_seconds {
                continue;
            }
            let path = PathBuf::from(&voice.path);
            let Ok(audio) = ai_daw_asset::decode(&path) else {
                continue;
            };
            let mono = if audio.channels == 1 {
                audio.samples.to_vec()
            } else {
                audio
                    .samples
                    .chunks_exact(audio.channels)
                    .map(|frame| frame.iter().sum::<f32>() / audio.channels as f32)
                    .collect()
            };
            let rate = if audio.sample_rate > 0.0 {
                audio.sample_rate
            } else {
                VOICE_RATE as f64
            };
            let (picked, choice) =
                ai_daw_local_ai::singing::reference::trim_to_best(&mono, rate, 10.0);
            if picked.is_empty() {
                continue;
            }
            let mut buffer = ai_daw_engine::audio::AudioBuffer::silence(1, picked.len());
            buffer.data_mut().copy_from_slice(&picked);
            if ai_daw_engine::wav::write_wav(&path, &buffer, rate.round().max(1.0) as u32).is_err()
            {
                continue;
            }
            voice.seconds = picked.len() as f64 / rate;
            voice.sample_rate = rate.round().max(1.0) as u32;
            voice.frames = picked.len() as u64;
            voice.offset_seconds = choice.start_sec;
            changed += 1;
        }
        if changed > 0 {
            let _ = self.flush();
        }
        changed
    }

    /// 人声 WAV 的存放目录（与 `sound-memories.json` 同级）。
    fn audio_dir(&self) -> Option<PathBuf> {
        let path = self.path.as_ref()?;
        Some(path.parent()?.join("sound-memory-audio"))
    }

    /// 人声参考的音频路径（`None` = 不是人声条目）。
    pub fn voice_path(&self, id: &str) -> Option<PathBuf> {
        self.get(id)
            .and_then(|m| m.voice)
            .map(|v| PathBuf::from(v.path))
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
        // 人声条目没有乐器 / 效果链；用轨道快照「覆盖」它只会造出一条四不像。
        if memory.voice.is_some() {
            return Err(
                "这是一条人声音色，不能用轨道的乐器快照覆盖；请改名 / 改标签，或者删掉重存。"
                    .into(),
            );
        }
        memory.name = meta.name;
        memory.description = meta.description;
        memory.tags = meta.tags;
        memory.role = meta.role;
        memory.instrument = snapshot.instrument;
        memory.effects = snapshot.effects;
        memory.mixer = snapshot.mixer;
        memory.source_project = snapshot.source_project;
        // 乐器链的「覆盖」不动人声字段；人声条目走 `update_metadata`。
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
        let voice = self
            .memories
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.voice.clone());
        self.memories.retain(|m| m.id != id);
        if self.memories.len() == before {
            return Err(format!("音色记忆不存在：{id}"));
        }
        if let Some(voice) = voice {
            let _ = fs::remove_file(voice.path);
        }
        self.flush()
    }

    fn flush(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
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
    fn voice_memory_roundtrips_and_deletes_its_audio() {
        let dir = std::env::temp_dir().join(format!("aidaw-voice-mem-{}", now_millis()));
        let path = dir.join("sound-memories.json");
        let mut store = SoundMemoryStore::load(Some(path));
        let samples: Vec<f32> = (0..VOICE_RATE as usize)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect();
        let memory = store
            .create_voice(
                MemoryMeta {
                    name: "测试人声".into(),
                    description: "清亮".into(),
                    tags: vec!["女声".into()],
                    role: None,
                },
                &samples,
                3.5,
            )
            .expect("存人声失败");

        // 自动补「人声」标签，并且带 voice 素材与来源偏移。
        assert!(memory.tags.iter().any(|t| t == VOICE_TAG));
        let voice = memory.voice.clone().expect("voice 素材");
        assert!(std::path::Path::new(&voice.path).is_file());
        assert!((voice.seconds - 1.0).abs() < 0.01);
        assert_eq!(voice.sample_rate, VOICE_RATE);
        assert!((voice.offset_seconds - 3.5).abs() < 0.01);

        // 重新加载：元数据与音频都在。
        let mut reopened = SoundMemoryStore::load(Some(dir.join("sound-memories.json")));
        let summaries = reopened.summaries();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].voice.is_some());

        // 删除：连音频一起删。
        reopened.remove(&memory.id).expect("删除失败");
        assert!(!std::path::Path::new(&voice.path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_voice_references_are_trimmed_to_ten_seconds() {
        let dir = std::env::temp_dir().join(format!("aidaw-voice-trim-{}", now_millis()));
        let path = dir.join("sound-memories.json");
        let mut store = SoundMemoryStore::load(Some(path));
        // 2 秒静音 + 20 秒 220 Hz 音：应裁到有声段，且只剩 10 秒。
        let mut samples = vec![0.0f32; (VOICE_RATE as usize) * 2];
        let tone: Vec<f32> = (0..(VOICE_RATE as usize) * 20)
            .map(|i| {
                (std::f64::consts::TAU * 220.0 * i as f64 / VOICE_RATE as f64).sin() as f32 * 0.5
            })
            .collect();
        samples.extend(tone);
        let memory = store
            .create_voice(
                MemoryMeta {
                    name: "整首歌".into(),
                    description: String::new(),
                    tags: Vec::new(),
                    role: None,
                },
                &samples,
                0.0,
            )
            .expect("存人声失败");
        assert!(memory.voice.as_ref().unwrap().seconds > 20.0);

        let changed = store.trim_long_voices(12.0);
        assert_eq!(changed, 1);
        let voice = store
            .get(&memory.id)
            .and_then(|m| m.voice.clone())
            .expect("voice 仍在");
        assert!(
            (voice.seconds - 10.0).abs() < 0.2,
            "应裁成 10 秒：{}",
            voice.seconds
        );
        assert!(voice.offset_seconds > 1.0, "应跳过头部的静音");
        let after = ai_daw_asset::decode(&voice.path).unwrap();
        assert!(after.frames() < (VOICE_RATE as usize) * 11);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert_eq!(
            summary.instrument_plugin_id.as_deref(),
            Some("builtin.synth")
        );
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
