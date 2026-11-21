//! 错误类型：每种失败都要能落到「扩展管理器」里的一句人话。

use std::fmt;

/// 扩展子系统的错误。
#[derive(Debug)]
pub enum ExtError {
    /// 文件读写失败（路径 + 原因）。
    Io(String),
    /// JSON / 清单解析失败。
    Json(String),
    /// 包格式不合规（缺 `aidaw.manifest`、apiVersion 不对、入口不存在…）。
    InvalidPackage(String),
    /// 当前应用版本不满足包的 `engines.aidaw`。
    Incompatible(String),
    /// 注册冲突（工具名 / 面板 id 已被占用）。
    Conflict(String),
    /// 宿主线程装载或执行失败（JS 异常、超时、崩溃）。
    Host(String),
    /// 市场索引不合法（带 JSON 路径）。
    InvalidIndex(String),
    /// 网络或 registry 失败。
    Network(String),
    /// 安装流水线失败（下载 / 校验 / 解压 / 落盘）。
    Install(String),
    /// 状态文件版本不认识。
    State(String),
}

impl ExtError {
    /// 给界面/模型看的稳定错误码（与日志文案解耦）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::InvalidPackage(_) => "invalid-package",
            Self::Incompatible(_) => "incompatible",
            Self::Conflict(_) => "conflict",
            Self::Host(_) => "host",
            Self::InvalidIndex(_) => "invalid-index",
            Self::Network(_) => "network",
            Self::Install(_) => "install",
            Self::State(_) => "state",
        }
    }
}

impl fmt::Display for ExtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "文件操作失败：{msg}"),
            Self::Json(msg) => write!(f, "JSON 解析失败：{msg}"),
            Self::InvalidPackage(msg) => write!(f, "扩展包不合规：{msg}"),
            Self::Incompatible(msg) => write!(f, "版本不兼容：{msg}"),
            Self::Conflict(msg) => write!(f, "注册冲突：{msg}"),
            Self::Host(msg) => write!(f, "扩展运行失败：{msg}"),
            Self::InvalidIndex(msg) => write!(f, "市场索引不合法：{msg}"),
            Self::Network(msg) => write!(f, "网络请求失败：{msg}"),
            Self::Install(msg) => write!(f, "安装失败：{msg}"),
            Self::State(msg) => write!(f, "扩展状态文件不可用：{msg}"),
        }
    }
}

impl std::error::Error for ExtError {}

impl From<std::io::Error> for ExtError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for ExtError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err.to_string())
    }
}

/// 扩展子系统的结果。
pub type ExtResult<T> = Result<T, ExtError>;

/// 把错误带上上下文（路径 / 包名）。
pub trait Context<T> {
    /// 失败时把 `what` 拼到错误信息前面。
    fn context(self, what: impl Into<String>) -> ExtResult<T>;
}

impl<T> Context<T> for ExtResult<T> {
    fn context(self, what: impl Into<String>) -> ExtResult<T> {
        self.map_err(|err| {
            let what = what.into();
            match err {
                ExtError::Io(msg) => ExtError::Io(format!("{what}: {msg}")),
                ExtError::Json(msg) => ExtError::Json(format!("{what}: {msg}")),
                ExtError::InvalidPackage(msg) => ExtError::InvalidPackage(format!("{what}: {msg}")),
                ExtError::Incompatible(msg) => ExtError::Incompatible(format!("{what}: {msg}")),
                ExtError::Conflict(msg) => ExtError::Conflict(format!("{what}: {msg}")),
                ExtError::Host(msg) => ExtError::Host(format!("{what}: {msg}")),
                ExtError::InvalidIndex(msg) => ExtError::InvalidIndex(format!("{what}: {msg}")),
                ExtError::Network(msg) => ExtError::Network(format!("{what}: {msg}")),
                ExtError::Install(msg) => ExtError::Install(format!("{what}: {msg}")),
                ExtError::State(msg) => ExtError::State(format!("{what}: {msg}")),
            }
        })
    }
}
