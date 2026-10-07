use thiserror::Error;

/// 插件操作错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PluginError {
    #[error("plugin state is too short: expected at least {expected} bytes, got {got}")]
    StateTooShort { expected: usize, got: usize },
    #[error("invalid plugin state: {0}")]
    InvalidState(String),
}
