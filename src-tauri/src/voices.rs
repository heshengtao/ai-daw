//! **人声音色**：歌声合成用的参考声线。
//!
//! 它**不是**另一套库：人声就是「音色」，和乐器 / 效果链一起存在**音色记忆库**
//! （[`crate::sound_memories`]）里，只是条目多了一个 `voice` 字段、标签里多一个「人声」。
//! 于是侧边栏的「音色库」能统一浏览、改名、删库、按标签筛选，不需要第二套管理界面。
//!
//! 这里只是给歌声合成用的**适配层**：把音色记忆里的 `voice` 条目转成前端好用的
//! `VoiceEntry`，以及把一条轨道 / 一个文件转成「单声道 24 kHz + 峰值归一」的参考 WAV。
//!
//! 存储：`<配置目录>/sound-memories.json`（元数据）+ `sound-memory-audio/<id>.wav`。

use serde::{Deserialize, Serialize};

use ai_daw_engine::model::TrackId;

use crate::state::AppState;

/// 歌声合成用的参考采样率（与模型一致）。
pub const VOICE_RATE: u32 = 24_000;

/// 一条人声音色（歌声合成下拉用的扁平视图）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceEntry {
    /// 音色记忆 id（`s<毫秒>`）。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 一句话描述。
    #[serde(default)]
    pub description: String,
    /// 标签（含自动加的「人声」）。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 时长（秒）。
    pub seconds: f64,
    /// 采样率（固定 24000）。
    pub sample_rate: u32,
    /// 帧数。
    pub frames: u64,
    /// 创建时间（Unix 毫秒）。
    pub created_at: u64,
    /// 这段参考取自原音频的第几秒（自动裁剪的起点）。
    pub offset_seconds: f64,
}

impl From<crate::sound_memories::SoundMemorySummary> for VoiceEntry {
    fn from(m: crate::sound_memories::SoundMemorySummary) -> Self {
        let voice = m.voice.unwrap_or(crate::sound_memories::VoiceAsset {
            path: String::new(),
            seconds: 0.0,
            sample_rate: VOICE_RATE,
            frames: 0,
            offset_seconds: 0.0,
        });
        VoiceEntry {
            id: m.id,
            name: m.name,
            description: m.description,
            tags: m.tags,
            seconds: voice.seconds,
            sample_rate: voice.sample_rate.max(1),
            frames: voice.frames,
            created_at: m.created_at,
            offset_seconds: voice.offset_seconds,
        }
    }
}

// ---------------------------------------------------------------------------
// 宿主命令
// ---------------------------------------------------------------------------

/// 保存人声音色的来源：工程内某条音频轨，或一个音频文件。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct VoiceSaveArgs {
    /// 展示名（缺省用轨道名 / 文件名）。
    pub name: Option<String>,
    /// 一句话描述。
    pub description: Option<String>,
    /// 标签（会自动补「人声」）。
    pub tags: Vec<String>,
    /// 工程内轨道 id（取它的第一段音频）。
    pub track: Option<u64>,
    /// 音频文件路径（与 `track` 二选一）。
    pub path: Option<String>,
}

/// 列出人声音色（本质是音色记忆里带 `voice` 的条目，新存的在前）。
pub fn list(state: &AppState) -> Vec<VoiceEntry> {
    let mut out: Vec<VoiceEntry> = state
        .sound_memories
        .lock()
        .unwrap()
        .summaries()
        .into_iter()
        .filter(|m| m.voice.is_some())
        .map(VoiceEntry::from)
        .collect();
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// 把一条参考声线存进音色库（写 WAV + 一条带「人声」标签的音色记忆）。
pub fn save(
    app: &tauri::AppHandle,
    state: &AppState,
    args: VoiceSaveArgs,
) -> Result<VoiceEntry, String> {
    let (samples, fallback_name, offset_seconds) = read_source(app, state, &args)?;
    let name = args
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(fallback_name);
    let meta = crate::sound_memories::MemoryMeta {
        name,
        description: args.description.clone().unwrap_or_default(),
        tags: args.tags.clone(),
        role: Some("vocal".into()),
    };
    let memory = state
        .sound_memories
        .lock()
        .unwrap()
        .create_voice(meta, &samples, offset_seconds)?;
    Ok(VoiceEntry::from(memory.summary()))
}

/// 改名字 / 描述 / 标签（不动音频）。
pub fn update(
    state: &AppState,
    id: String,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<VoiceEntry, String> {
    let mut store = state.sound_memories.lock().unwrap();
    if store.get(&id).and_then(|m| m.voice).is_none() {
        return Err(format!("音色库里没有这条人声：{id}"));
    }
    let memory = store.update_metadata(&id, name.as_deref(), description.as_deref(), tags, None)?;
    Ok(VoiceEntry::from(memory.summary()))
}

/// 删一条人声（连 WAV 一起删）。
pub fn delete(state: &AppState, id: String) -> Result<(), String> {
    let mut store = state.sound_memories.lock().unwrap();
    if store.get(&id).and_then(|m| m.voice).is_none() {
        return Err(format!("音色库里没有这条人声：{id}"));
    }
    store.remove(&id)
}

/// 读取一条人声的 PCM（歌声合成用；单声道 24 kHz）。
pub(crate) fn samples_for(state: &AppState, id: &str) -> Result<(Vec<f32>, f64), String> {
    let path = state
        .sound_memories
        .lock()
        .unwrap()
        .voice_path(id)
        .ok_or_else(|| format!("音色库里没有这条人声：{id}"))?;
    let audio = ai_daw_asset::decode(&path).map_err(|e| format!("读取人声音色失败：{e}"))?;
    let mono = to_mono(&audio.samples, audio.channels);
    Ok((mono, audio.sample_rate))
}

/// 把来源读成「单声道 24 kHz」，并给一个缺省名字。
fn read_source(
    app: &tauri::AppHandle,
    state: &AppState,
    args: &VoiceSaveArgs,
) -> Result<(Vec<f32>, String, f64), String> {
    let _ = app;
    match (&args.track, &args.path) {
        (Some(track), _) => {
            let (path, name) = {
                let engine = state.engine.lock().unwrap();
                let project = engine.project();
                let t = project
                    .tracks
                    .iter()
                    .find(|t| t.id == TrackId(*track))
                    .ok_or("参考声线轨道不存在")?;
                let clip = t
                    .clips
                    .iter()
                    .find(|c| c.audio.is_some())
                    .ok_or("这条轨道上没有音频片段")?;
                (
                    clip.audio.clone().expect("已确认是音频片段").path,
                    t.name.clone(),
                )
            };
            let audio = state.assets.load(&path)?;
            let mono = to_mono(&audio.samples, audio.channels);
            let resampled =
                ai_daw_local_ai::resample::resample(&mono, audio.sample_rate, VOICE_RATE as f64);
            let (picked, choice) = ai_daw_local_ai::singing::reference::trim_to_best(
                &resampled,
                VOICE_RATE as f64,
                10.0,
            );
            Ok((picked, name, choice.start_sec))
        }
        (None, Some(path)) => {
            let audio = ai_daw_asset::decode(path).map_err(|e| format!("读取音频失败：{e}"))?;
            let mono = to_mono(&audio.samples, audio.channels);
            let resampled =
                ai_daw_local_ai::resample::resample(&mono, audio.sample_rate, VOICE_RATE as f64);
            let (picked, choice) = ai_daw_local_ai::singing::reference::trim_to_best(
                &resampled,
                VOICE_RATE as f64,
                10.0,
            );
            let name = std::path::Path::new(path)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "人声音色".into());
            Ok((picked, name, choice.start_sec))
        }
        (None, None) => Err("需要 track（工程内音频轨）或 path（音频文件）之一".into()),
    }
}

/// 交错多声道 → 单声道（平均）。
fn to_mono(samples: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// 列出人声音色库（新存的在前）。
#[tauri::command]
pub fn voice_list(state: tauri::State<'_, AppState>) -> Vec<VoiceEntry> {
    list(&state)
}

/// 把一条参考声线存进音色库（单声道 24 kHz，峰值归一，自动带「人声」标签）。
#[tauri::command]
pub fn voice_save(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    args: VoiceSaveArgs,
) -> Result<VoiceEntry, String> {
    save(&app, &state, args)
}

/// 改名字 / 描述 / 标签。
#[tauri::command]
pub fn voice_update(
    state: tauri::State<'_, AppState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<VoiceEntry, String> {
    update(&state, id, name, description, tags)
}

/// 删除一条人声音色（连 WAV）。
#[tauri::command]
pub fn voice_delete(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    delete(&state, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_downmix_averages_channels() {
        assert_eq!(to_mono(&[1.0, 0.0, 0.5, -0.5], 2), vec![0.5, 0.0]);
    }
}
