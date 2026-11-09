//! 用户自定义的外部插件扫描目录。
//!
//! 系统标准目录（VST3 / CLAP）由扫描器自己覆盖；这里只存**额外**要扫的文件夹。
//! 主流 DAW 都允许挂多个插件目录（库散落在不同盘上），所以存成有序列表、可增删。
//!
//! 文件落在应用配置目录的 `plugin-scan-dirs.json`，每轮扫描开始时由
//! [`crate::manager::rescan_with_managed`] 读取，所以增删目录不需要重启。

use std::fs;
use std::path::{Path, PathBuf};

use tauri::AppHandle;

/// 配置文件里的结构（只存字符串，平台切换 / 目录迁移时也不会解析失败）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct ScanDirsFile {
    dirs: Vec<String>,
}

/// 界面上的一个扫描目录。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanDirInfo {
    pub path: String,
    /// 目录当前是否存在。不存在的条目**保留**，否则用户没法在界面上把它删掉。
    pub exists: bool,
}

fn config_file(app: &AppHandle) -> Option<PathBuf> {
    crate::paths::config_dir(app)
        .ok()
        .map(|dir| dir.join("plugin-scan-dirs.json"))
}

/// 读自定义目录（去空、去重；文件缺失 / 损坏时返回空列表）。
pub fn load(app: &AppHandle) -> Vec<PathBuf> {
    let Some(file) = config_file(app) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<ScanDirsFile>(&text) else {
        eprintln!("插件扫描目录配置无法解析，按空列表处理");
        return Vec::new();
    };

    let mut out: Vec<PathBuf> = Vec::new();
    for raw in parsed.dirs {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let path = PathBuf::from(text);
        if !out.iter().any(|existing| same_path(existing, &path)) {
            out.push(path);
        }
    }
    out
}

fn save(app: &AppHandle, dirs: &[PathBuf]) -> Result<(), String> {
    let file = config_file(app).ok_or_else(|| "拿不到应用配置目录".to_string())?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let data = ScanDirsFile {
        dirs: dirs
            .iter()
            .map(|dir| dir.to_string_lossy().into_owned())
            .collect(),
    };
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    // 先写临时文件再替换：中途崩掉也不会留下半截 JSON。
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| format!("写入扫描目录失败：{e}"))?;
    // Windows 的 rename 不会覆盖已存在的目标，先删旧的。
    let _ = fs::remove_file(&file);
    fs::rename(&tmp, &file).map_err(|e| format!("保存扫描目录失败：{e}"))?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(not(windows))]
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    a == b
}

fn info(dirs: &[PathBuf]) -> Vec<ScanDirInfo> {
    dirs.iter()
        .map(|dir| ScanDirInfo {
            path: dir.to_string_lossy().into_owned(),
            exists: dir.is_dir(),
        })
        .collect()
}

/// 当前自定义扫描目录（界面打开设置页时拉一次）。
#[tauri::command]
pub fn plugin_scan_dirs(app: AppHandle) -> Vec<ScanDirInfo> {
    info(&load(&app))
}

/// 覆盖保存自定义扫描目录，并返回保存后的列表（已规范化）。
///
/// 只接受绝对路径：相对路径会随当前工作目录变化，扫描结果不可预期。
/// 不要求目录存在：网络盘暂时没挂上、插拔硬盘时不能连「删掉这条」都做不了。
#[tauri::command]
pub fn plugin_set_scan_dirs(app: AppHandle, dirs: Vec<String>) -> Result<Vec<ScanDirInfo>, String> {
    let mut next: Vec<PathBuf> = Vec::new();
    for raw in dirs {
        let text = raw.trim();
        if text.is_empty() {
            continue;
        }
        let path = PathBuf::from(text);
        if !path.is_absolute() {
            return Err(format!("不是绝对路径：{text}"));
        }
        if !next.iter().any(|existing| same_path(existing, &path)) {
            next.push(path);
        }
    }
    save(&app, &next)?;
    Ok(info(&next))
}
