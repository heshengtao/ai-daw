//! 用户预设：每个插件一份「参数快照」，用来快速切音色。
//!
//! 存在应用配置目录的 `presets.json` 里，**跨工程**可用：预设是「这个插件怎么调」的知识，
//! 不属于某一首歌。保存的是参数值本身（而不是插件二进制状态），所以：
//! - 内置合成器与外部插件走同一条路；
//! - 插件后来新增 / 删除参数都不会让旧预设读不出来（缺的参数用插件默认值）。
//!
//! 写入是「整文件读-改-写」：预设量级是几十到几百条，不值得为它引入增量写。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 一条用户预设。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    /// 归属的插件 id（`builtin.synth` / `vst3:…` / `clap:…`）。
    pub plugin_id: String,
    /// 保存时的参数快照（参数 id → 值）。
    pub params: HashMap<String, f32>,
    /// 创建时间（Unix 毫秒），用于稳定排序与生成 id。
    #[serde(default)]
    pub created_at: u64,
}

/// 预设仓库：文件 + 内存镜像。
///
/// `path` 为 `None` 时只活在内存里（拿不到配置目录时的兜底：功能可用，重启丢失）。
pub struct PresetStore {
    path: Option<PathBuf>,
    presets: Vec<Preset>,
}

impl PresetStore {
    /// 从文件加载预设；文件不存在或损坏都当作「还没有预设」。
    pub fn load(path: Option<PathBuf>) -> Self {
        let presets = path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Vec<Preset>>(&text).ok())
            .unwrap_or_default();
        PresetStore { path, presets }
    }

    /// 全部预设（按创建时间升序）。
    pub fn all(&self) -> Vec<Preset> {
        let mut out = self.presets.clone();
        out.sort_by_key(|p| p.created_at);
        out
    }

    /// 新增一条预设并落盘。
    pub fn create(
        &mut self,
        name: &str,
        plugin_id: &str,
        params: HashMap<String, f32>,
    ) -> Result<Preset, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("预设名字不能为空".into());
        }
        let created_at = now_millis();
        let preset = Preset {
            id: unique_id(&self.presets, created_at),
            name: name.to_string(),
            plugin_id: plugin_id.to_string(),
            params,
            created_at,
        };
        self.presets.push(preset.clone());
        self.flush()?;
        Ok(preset)
    }

    /// 覆盖一条已存在的预设（改名 / 更新参数）。
    pub fn update(
        &mut self,
        id: &str,
        name: &str,
        params: HashMap<String, f32>,
    ) -> Result<Preset, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("预设名字不能为空".into());
        }
        let preset = self
            .presets
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| format!("预设不存在：{id}"))?;
        preset.name = name.to_string();
        preset.params = params;
        let updated = preset.clone();
        self.flush()?;
        Ok(updated)
    }

    /// 删除一条预设。
    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let before = self.presets.len();
        self.presets.retain(|p| p.id != id);
        if self.presets.len() == before {
            return Err(format!("预设不存在：{id}"));
        }
        self.flush()
    }

    /// 写回文件（内存模式下什么也不做）。
    fn flush(&self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建预设目录失败：{e}"))?;
        }
        let text =
            serde_json::to_string_pretty(&self.presets).map_err(|e| format!("序列化预设失败：{e}"))?;
        fs::write(path, text).map_err(|e| format!("写入预设失败：{e}"))
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 生成不与现有预设冲突的 id。
fn unique_id(existing: &[Preset], seed: u64) -> String {
    let mut candidate = format!("p{seed}");
    let mut n = 1;
    while existing.iter().any(|p| p.id == candidate) {
        candidate = format!("p{seed}-{n}");
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(pairs: &[(&str, f32)]) -> HashMap<String, f32> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn roundtrips_through_the_file() {
        let dir = std::env::temp_dir().join(format!("ai-daw-presets-{}", now_millis()));
        let path = dir.join("presets.json");

        let mut store = PresetStore::load(Some(path.clone()));
        let saved = store
            .create("亮主音", "builtin.synth", params(&[("attack", 0.01), ("release", 0.2)]))
            .unwrap();

        // 重新加载：内容必须原样回来。
        let reloaded = PresetStore::load(Some(path));
        let all = reloaded.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], saved);
        assert_eq!(all[0].params["release"], 0.2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_empty_names_and_deletes_by_id() {
        let mut store = PresetStore::load(None);
        assert!(store.create("   ", "builtin.synth", HashMap::new()).is_err());

        let saved = store.create("a", "builtin.synth", HashMap::new()).unwrap();
        assert_eq!(store.all().len(), 1);
        store.remove(&saved.id).unwrap();
        assert!(store.all().is_empty());
        // 再删一次必须报错，而不是静默成功。
        assert!(store.remove(&saved.id).is_err());
    }
}
