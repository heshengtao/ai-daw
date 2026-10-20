//! 历史工程：工程被替换前留下的一份工作副本。
//!
//! 每当当前工程即将被「新建 / 打开文件 / 从历史恢复」替换掉时，先把工作副本与保存基线
//! 记一份到应用配置目录的 `history/`。界面据此展示缩略图卡片，点一下就能回到之前的工程。
//!
//! 索引（`index.json`）只存展示用的轻量元信息（轨道 / 片段布局），完整工程放在
//! `<id>.json` 里——列历史时不必把几十份工程全塞进 IPC 载荷。

use std::path::{Path, PathBuf};

use ai_daw_engine::model::Project;
use serde::{Deserialize, Serialize};

/// 最多保留多少份历史（超出的从最旧开始丢弃）。
pub const MAX_ENTRIES: usize = 30;

/// 缩略图里的一段片段（拍）。只留画图需要的字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryClip {
    pub start: f64,
    pub length: f64,
    pub audio: bool,
}

/// 缩略图里的一条轨道。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryTrack {
    pub name: String,
    pub clips: Vec<HistoryClip>,
}

/// 历史列表的一项：展示用元信息 + 恢复时定位快照的 id。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub name: String,
    /// 记录时间（Unix 毫秒）。
    pub saved_at: i64,
    /// 当时对应的工程文件路径（未保存过为 `None`）。
    pub path: Option<String>,
    /// 当时是否有未保存改动。
    pub dirty: bool,
    pub tempo: f64,
    /// 最后一个片段的结束位置（拍）。
    pub end_beat: f64,
    pub track_count: usize,
    pub clip_count: usize,
    pub note_count: usize,
    pub tracks: Vec<HistoryTrack>,
    /// 工程内容指纹：连续记录同一份工程时用它去重。
    #[serde(default)]
    pub fingerprint: String,
}

/// 一份完整的历史快照：工作副本 + 保存基线。
///
/// 保存基线一并留下，恢复后「未保存」标记才和当时一致（不会把带未保存改动的工程
/// 误标成已保存，也不会反过来）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistorySnapshot {
    pub project: Project,
    pub saved: Project,
}

/// 读取索引；文件缺失或损坏时返回空列表（历史功能坏了不该拖垮启动）。
pub fn load_index(dir: &Path) -> Vec<HistoryEntry> {
    let text = match std::fs::read_to_string(dir.join("index.json")) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_index(dir: &Path, entries: &[HistoryEntry]) -> Result<(), String> {
    write_json(&dir.join("index.json"), entries)
}

/// 原子写 JSON：先写临时文件再改名，避免中途崩溃留下半截文件。
fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn fingerprint(project: &Project) -> String {
    // 工程不大（音符也就几十 KB），序列化一次比读回上一份快照便宜。
    serde_json::to_string(project).unwrap_or_default()
}

/// 「刚铺好的空工程」：没路径、没未保存改动、也没有任何片段。
///
/// 这种状态记进历史只会变成一堆空白卡片（连点几次「新建工程」就满了），所以直接跳过。
fn is_pristine(project: &Project, saved: &Project, path: &Option<String>) -> bool {
    path.is_none()
        && project == saved
        && project.tracks.iter().all(|t| t.clips.is_empty())
        && project.harmony.is_empty()
        && project.sections.is_empty()
}

fn make_entry(
    id: String,
    project: &Project,
    saved: &Project,
    path: Option<String>,
    saved_at: i64,
) -> HistoryEntry {
    let tracks: Vec<HistoryTrack> = project
        .tracks
        .iter()
        .map(|track| HistoryTrack {
            name: track.name.clone(),
            clips: track
                .clips
                .iter()
                .map(|clip| HistoryClip {
                    start: clip.start,
                    length: clip.length,
                    audio: clip.audio.is_some(),
                })
                .collect(),
        })
        .collect();
    let clip_count = tracks.iter().map(|t| t.clips.len()).sum();
    let note_count = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.notes.len())
        .sum();
    let end_beat = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.start + c.length)
        .fold(0.0_f64, f64::max);
    HistoryEntry {
        id,
        name: project.name.clone(),
        saved_at,
        path,
        dirty: project != saved,
        tempo: project.transport.tempo,
        end_beat,
        track_count: tracks.len(),
        clip_count,
        note_count,
        tracks,
        fingerprint: fingerprint(project),
    }
}

/// 生成一个目录里唯一的 id。
fn unique_id(dir: &Path, saved_at: i64) -> String {
    let base = saved_at.to_string();
    if !dir.join(format!("{base}.json")).exists() {
        return base;
    }
    let mut n = 1;
    loop {
        let id = format!("{base}-{n}");
        if !dir.join(format!("{id}.json")).exists() {
            return id;
        }
        n += 1;
    }
}

/// 记一份历史；返回新增的条目（被去重 / 跳过时返回 `None`）。
pub fn record(
    dir: &Path,
    project: &Project,
    saved: &Project,
    path: Option<String>,
) -> Result<Option<HistoryEntry>, String> {
    if is_pristine(project, saved, &path) {
        return Ok(None);
    }
    let mut entries = load_index(dir);
    let fp = fingerprint(project);
    // 同一份工程（内容 + 路径都相同）已经在历史里：把它挪到最前，不再存第二份。
    // 来回切换工程时历史不会堆成重复卡片。
    if let Some(pos) = entries
        .iter()
        .position(|e| e.fingerprint == fp && e.path == path)
    {
        if pos > 0 {
            let existing = entries.remove(pos);
            entries.insert(0, existing);
            write_index(dir, &entries)?;
        }
        return Ok(None);
    }

    let saved_at = now_ms();
    let id = unique_id(dir, saved_at);
    let snapshot = HistorySnapshot {
        project: project.clone(),
        saved: saved.clone(),
    };
    write_json(&dir.join(format!("{id}.json")), &snapshot)?;

    let entry = make_entry(id, project, saved, path, saved_at);
    entries.insert(0, entry.clone());
    while entries.len() > MAX_ENTRIES {
        if let Some(old) = entries.pop() {
            let _ = std::fs::remove_file(dir.join(format!("{}.json", old.id)));
        }
    }
    write_index(dir, &entries)?;
    Ok(Some(entry))
}

/// id 只可能是我们自己生成的数字 / 连字符，别让外部输入拼出路径穿越。
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// 取出并移除一份历史（恢复后它就是当前工程了，不该再留在列表里）。
pub fn take(dir: &Path, id: &str) -> Result<(HistorySnapshot, HistoryEntry), String> {
    if !valid_id(id) {
        return Err("无效的历史工程 id".into());
    }
    let file = dir.join(format!("{id}.json"));
    let text = std::fs::read_to_string(&file).map_err(|e| format!("读取历史工程失败：{e}"))?;
    let snapshot: HistorySnapshot =
        serde_json::from_str(&text).map_err(|e| format!("历史工程已损坏：{e}"))?;

    let entries = load_index(dir);
    let entry = entries
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or_else(|| "历史工程不存在".to_string())?;
    let rest: Vec<HistoryEntry> = entries.into_iter().filter(|e| e.id != id).collect();
    let _ = std::fs::remove_file(&file);
    write_index(dir, &rest)?;
    Ok((snapshot, entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ai-daw-history-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn skips_a_pristine_empty_project() {
        let dir = temp_dir("pristine");
        let project = Project::new("untitled");
        let saved = project.clone();
        assert!(record(&dir, &project, &saved, None).unwrap().is_none());
        assert!(load_index(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn records_and_restores_the_saved_baseline() {
        let dir = temp_dir("roundtrip");
        let saved = ai_daw_engine::bus::Engine::new_project("demo")
            .project()
            .clone();
        let mut project = saved.clone();
        project.tracks[0].name = "Lead".into();

        let entry = record(&dir, &project, &saved, Some("/tmp/demo.json".into()))
            .unwrap()
            .expect("应记下一条历史");
        assert_eq!(entry.name, "demo");
        assert!(entry.dirty, "工作副本与保存基线不同 → 未保存");
        assert_eq!(entry.track_count, project.tracks.len());

        let (snapshot, taken) = take(&dir, &entry.id).unwrap();
        assert_eq!(snapshot.project.tracks[0].name, "Lead");
        assert_eq!(snapshot.saved.tracks[0].name, saved.tracks[0].name);
        assert_eq!(taken.path.as_deref(), Some("/tmp/demo.json"));
        assert!(load_index(&dir).is_empty(), "取出后不再留在列表里");
        assert!(!dir.join(format!("{}.json", entry.id)).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dedupes_the_same_project_recorded_twice() {
        let dir = temp_dir("dedupe");
        let project = Project::new("demo");
        let saved = project.clone();
        let path = Some("/tmp/demo.json".to_string());
        assert!(record(&dir, &project, &saved, path.clone())
            .unwrap()
            .is_some());
        assert!(record(&dir, &project, &saved, path).unwrap().is_none());
        assert_eq!(load_index(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moves_a_revisited_project_to_the_front() {
        let dir = temp_dir("revisit");
        let a = ai_daw_engine::bus::Engine::new_project("a")
            .project()
            .clone();
        let b = ai_daw_engine::bus::Engine::new_project("b")
            .project()
            .clone();
        record(&dir, &a, &a, Some("/tmp/a.json".into())).unwrap();
        record(&dir, &b, &b, Some("/tmp/b.json".into())).unwrap();

        assert!(record(&dir, &a, &a, Some("/tmp/a.json".into()))
            .unwrap()
            .is_none());
        let entries = load_index(&dir);
        assert_eq!(entries.len(), 2, "同一份工程不该存成两张卡片");
        assert_eq!(entries[0].name, "a", "再次遇到时挪到最前");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prunes_beyond_the_cap() {
        let dir = temp_dir("prune");
        for i in 0..(MAX_ENTRIES + 3) {
            let project = Project::new(format!("song-{i}"));
            let saved = project.clone();
            record(&dir, &project, &saved, Some(format!("/tmp/{i}.json"))).unwrap();
        }
        let entries = load_index(&dir);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].name, format!("song-{}", MAX_ENTRIES + 2));
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(
            files,
            MAX_ENTRIES + 1,
            "快照文件也要一起清掉（+ index.json）"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_path_traversal_ids() {
        let dir = temp_dir("traversal");
        assert!(take(&dir, "../session").is_err());
        assert!(take(&dir, "1/2").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
