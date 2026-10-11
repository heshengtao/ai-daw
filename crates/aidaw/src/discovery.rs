//! 找到「正在运行的 AI DAW」的 MCP 端点。
//!
//! 优先读软件写下的**运行时发现文件**（`mcp-url`）——端口被占用而退回临时端口时，
//! 只有它记着实际地址；读不到再退回默认端口。路径规则与 Tauri 的 `app_config_dir()` 一致
//! （identifier = `com.aidaw.app`）。

use std::path::PathBuf;

/// MCP 默认监听端口；与 `src-tauri/src/mcp.rs` 的 `DEFAULT_MCP_PORT` 保持一致。
pub const DEFAULT_MCP_PORT: u16 = 50562;

/// Tauri 应用标识符（配置目录以它命名）。
const APP_IDENTIFIER: &str = "com.aidaw.app";

/// 应用名（macOS 上 `open -a` 用它启动）。
pub const APP_NAME: &str = "AI DAW";

/// 应用配置目录：与 Tauri 的 `app_config_dir()` 同规则。
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(APP_IDENTIFIER)
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join(APP_IDENTIFIER))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join(APP_IDENTIFIER))
    }
}

/// 运行时发现文件（`mcp-url`）的位置。
pub fn url_file() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("mcp-url"))
}

/// 读发现文件里记的实际地址；缺失或空白时返回 `None`。
pub fn read_url_file() -> Option<String> {
    let text = std::fs::read_to_string(url_file()?).ok()?;
    let url = text.trim();
    (!url.is_empty()).then(|| url.to_owned())
}

/// 默认地址（固定端口）。
pub fn default_url() -> String {
    format!("http://127.0.0.1:{DEFAULT_MCP_PORT}/mcp")
}

/// 依序尝试的候选地址：环境变量覆盖 → 发现文件 → 默认端口（去重）。
///
/// 顺序即优先级：`AIDAW_MCP_URL` 便于测试与非常规部署；发现文件最可信；默认端口兜底。
pub fn candidates() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |url: String| {
        if !url.is_empty() && !out.contains(&url) {
            out.push(url);
        }
    };
    if let Ok(url) = std::env::var("AIDAW_MCP_URL") {
        push(url.trim().to_owned());
    }
    if let Some(url) = read_url_file() {
        push(url);
    }
    push(default_url());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_url_uses_the_standard_port() {
        assert_eq!(DEFAULT_MCP_PORT, 50562);
        assert_eq!(default_url(), "http://127.0.0.1:50562/mcp");
    }

    #[test]
    fn candidates_always_end_with_the_default() {
        // 不依赖机器上有没有发现文件：默认端口永远是最后兜底。
        let candidates = candidates();
        assert_eq!(candidates.last().unwrap(), &default_url());
    }
}
