//! 「写歌智能体」的连接配置：OpenAI 兼容端点的地址、Key 与模型名。
//!
//! 落在应用配置目录的 `ai.json`，与 `mcp.json` / `store.json` 同一套惯例（明文 JSON，
//! 项目没有 keychain 集成）。文件缺失或损坏时回退到默认值，不让启动失败。

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// OpenAI 兼容端点的配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentConfig {
    /// 端点根地址，形如 `https://api.openai.com/v1`（也可填兼容网关）。
    pub base_url: String,
    /// API Key，走 `Authorization: Bearer`。
    pub api_key: String,
    /// 模型名。
    pub model: String,
}

impl AgentConfig {
    /// 是否已配置到「可以尝试调用」的程度。
    pub fn is_configured(&self) -> bool {
        !self.base_url.trim().is_empty()
            && !self.api_key.trim().is_empty()
            && !self.model.trim().is_empty()
    }
}

/// 配置的读写器（由 Tauri 管理）。
pub struct AgentConfigRuntime {
    inner: Mutex<AgentConfig>,
    /// 配置文件位置；拿不到配置目录时为 `None`（配置只活在内存里）。
    path: Option<PathBuf>,
}

impl AgentConfigRuntime {
    /// 从 `ai.json` 载入；缺失/损坏回退默认值。
    pub fn load(path: Option<PathBuf>) -> Self {
        let config = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<AgentConfig>(&text).ok())
            .unwrap_or_default();
        Self {
            inner: Mutex::new(config),
            path,
        }
    }

    /// 当前配置。
    pub fn get(&self) -> AgentConfig {
        self.inner.lock().unwrap().clone()
    }

    /// 更新配置并写盘，返回最新配置。
    pub fn set(&self, next: AgentConfig) -> AgentConfig {
        {
            let mut inner = self.inner.lock().unwrap();
            *inner = next.clone();
        }
        self.save(&next);
        next
    }

    fn save(&self, config: &AgentConfig) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(config) {
            Ok(json) => {
                if let Err(err) = std::fs::write(path, json) {
                    eprintln!("[agent] 写入配置失败：{err}");
                }
            }
            Err(err) => eprintln!("[agent] 序列化配置失败：{err}"),
        }
    }
}
