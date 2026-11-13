//! 「加载未响应」插件的隔离名单。
//!
//! # 为什么需要
//!
//! 有的插件（现场是 ACE Bridge）会在实例化或开界面时卡在一条**不可见**的模态框 / 死锁上，
//! 宿主只能靠超时把它放弃。问题是：只要工程里还挂着它，下次启动就会再卡一次——用户看到的是
//! 「用了这个插件之后，重启软件连别的插件都打不开了」。
//!
//! 所以把超时的模块路径记到这里：下次启动直接给它静音占位，**启动不再为它等待**；用户修好
//! 插件（或放行防火墙）之后点「重新扫描插件」即可清空名单、立刻重试。
//!
//! 只记「超时」，不记普通加载失败：参数不对、文件损坏这类错误本来就该每次照常报出来。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// 一条隔离记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// 插件模块路径（`.vst3` / `.clap` 文件或包目录）。
    pub path: PathBuf,
    /// 插件展示名（给人看）。
    pub plugin: String,
    /// 为什么被隔离。
    pub reason: String,
    /// 记录时间（本地时间，方便对照日志）。
    pub at: String,
}

/// 隔离名单（内存 + 磁盘）。
pub struct Quarantine {
    /// 落盘位置；拿不到配置目录时为 `None`（只在本次运行内有效）。
    file: Option<PathBuf>,
    /// 路径（规范字符串）→ 记录。
    entries: HashMap<String, Entry>,
}

impl Quarantine {
    /// 从磁盘读名单；文件不存在 / 损坏都当成空名单（不能因为一份坏文件起不来）。
    pub fn load(file: Option<PathBuf>) -> Self {
        let entries = file
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<Entry>>(&bytes).ok())
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| (key(&entry.path), entry))
                    .collect()
            })
            .unwrap_or_default();
        Self { file, entries }
    }

    /// 这个模块是否被隔离。
    pub fn get(&self, path: &Path) -> Option<&Entry> {
        self.entries.get(&key(path))
    }

    /// 记一条（同一路径重复记录只保留最新的一条），并立即落盘。
    pub fn note(&mut self, path: &Path, plugin: &str, reason: &str) {
        self.entries.insert(
            key(path),
            Entry {
                path: path.to_path_buf(),
                plugin: plugin.to_string(),
                reason: reason.to_string(),
                at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            },
        );
        self.save();
    }

    /// 加载成功了：把这条旧的隔离记录删掉。
    pub fn forget(&mut self, path: &Path) {
        if self.entries.remove(&key(path)).is_some() {
            self.save();
        }
    }

    /// 清空名单（用户显式重扫）。
    pub fn clear(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.save();
    }

    /// 当前名单（给插件面板展示）。
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self.entries.values().cloned().collect();
        entries.sort_by(|a, b| a.plugin.cmp(&b.plugin));
        entries
    }

    fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut entries: Vec<&Entry> = self.entries.values().collect();
        entries.sort_by(|a, b| a.plugin.cmp(&b.plugin));
        let Ok(bytes) = serde_json::to_vec_pretty(&entries) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // 先写临时文件再改名：中途退出也不会留下半份 JSON。
        let temp = file.with_extension("json.tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, file);
        }
    }
}

/// 查表用的键：能规范化就规范化，避免同一模块的两种写法被当成两个。
fn key(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

static STORE: OnceLock<Mutex<Quarantine>> = OnceLock::new();

/// 装上隔离名单（启动时调一次；没装时所有查询都当成「没被隔离」）。
pub fn init(file: Option<PathBuf>) {
    let _ = STORE.set(Mutex::new(Quarantine::load(file)));
}

fn store() -> Option<&'static Mutex<Quarantine>> {
    STORE.get()
}

/// 这个模块是否被隔离（没装名单时为 `None`）。
pub fn get(path: &Path) -> Option<Entry> {
    store()?.lock().ok()?.get(path).cloned()
}

/// 记一条隔离（失败也不影响主流程）。
pub fn note(path: &Path, plugin: &str, reason: &str) {
    if let Some(store) = store() {
        if let Ok(mut quarantine) = store.lock() {
            quarantine.note(path, plugin, reason);
        }
    }
}

/// 加载成功：清掉这条记录。
pub fn forget(path: &Path) {
    if let Some(store) = store() {
        if let Ok(mut quarantine) = store.lock() {
            quarantine.forget(path);
        }
    }
}

/// 清空名单。
pub fn clear() {
    if let Some(store) = store() {
        if let Ok(mut quarantine) = store.lock() {
            quarantine.clear();
        }
    }
}

/// 当前名单。
pub fn entries() -> Vec<Entry> {
    store()
        .and_then(|store| store.lock().ok().map(|quarantine| quarantine.entries()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 名单要能落盘、重读、按路径解除——这是「重启后不再为一个坏插件卡一次」的全部依据。
    #[test]
    fn quarantine_round_trips_and_forgets() {
        let dir = std::env::temp_dir().join(format!("aidaw-quarantine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("plugin-quarantine.json");
        let bad = Path::new("C:/plugins/Bad Bridge.vst3");

        let mut quarantine = Quarantine::load(Some(file.clone()));
        assert!(quarantine.get(bad).is_none(), "新名单里不该有它");

        quarantine.note(bad, "Bad Bridge", "插件没有响应");
        let remembered = quarantine.get(bad).expect("记下后应能查到");
        assert_eq!(remembered.plugin, "Bad Bridge");

        // 重新读盘：记录还在（重启后的行为）。
        let mut reloaded = Quarantine::load(Some(file.clone()));
        assert!(reloaded.get(bad).is_some(), "重启后仍应处于隔离状态");

        // 加载成功 → 解除；用户点重扫 → 整份清空。
        reloaded.forget(bad);
        assert!(
            Quarantine::load(Some(file.clone())).get(bad).is_none(),
            "加载成功应解除隔离并落盘"
        );

        let mut again = Quarantine::load(Some(file.clone()));
        again.note(bad, "Bad Bridge", "插件没有响应");
        again.clear();
        assert!(Quarantine::load(Some(file)).get(bad).is_none(), "重扫应清空名单");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
