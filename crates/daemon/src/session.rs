//! 会话缓存：把「当前工程 + 保存基线 + 文件路径」写到应用配置目录。
//!
//! 用途是**下次打开还原上次状态**：应用退出前（或编辑过程中定期）把工作副本落盘，
//! 启动时读回来。它和「保存工程」是两回事——保存写的是用户选定的工程文件，缓存写的是
//! 应用自己的 `session.json`，即使从未保存过也照常有。
//!
//! 缓存里同时存工作副本与保存基线：这样带着未保存改动退出后，下次打开能如实显示「未保存」。

use std::path::{Path, PathBuf};

use ai_daw_engine::model::Project;
use serde::{Deserialize, Serialize};

/// 会话缓存文件的内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    /// 当前工程对应的文件路径；尚未落盘时为 `None`。
    #[serde(default)]
    pub path: Option<String>,
    /// 当前工程的身份键（历史卡片归并版本用）；旧缓存没有时留空，启动后重新生成。
    #[serde(default)]
    pub project_key: String,
    /// 工作副本（当前正在编辑的工程）。
    pub project: Project,
    /// 保存基线（最近一次保存/打开/新建时的工程）。
    pub saved: Project,
}

/// 读取会话缓存；文件缺失或损坏时返回 `None`（回退到默认新工程）。
pub fn read_session(path: &Path) -> Option<SessionFile> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 写入会话缓存（原子写：先写临时文件再改名，避免中途崩溃留下半截文件）。
pub fn write_session(path: &Path, session: &SessionFile) -> Result<(), String> {
    let json = serde_json::to_string(session).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("ai-daw-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");

        let project = Project::new("demo");
        let session = SessionFile {
            path: Some("/tmp/song.json".into()),
            project_key: "p-session".into(),
            project: project.clone(),
            saved: Project::new("older"),
        };
        write_session(&path, &session).unwrap();

        let back = read_session(&path).unwrap();
        assert_eq!(back.project, project);
        assert_eq!(back.path.as_deref(), Some("/tmp/song.json"));
        assert_eq!(back.saved.name, "older", "保存基线要一并保留");

        // 临时文件已被改名走，不留残渣。
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_corrupt_file_reads_as_none() {
        assert!(read_session(Path::new("/nonexistent/ai-daw/session.json")).is_none());
    }
}
