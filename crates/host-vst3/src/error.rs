use thiserror::Error;

/// 宿主操作错误。
#[derive(Debug, Error)]
pub enum HostError {
    /// 扫描失败（目录不可读等）。
    #[error("插件扫描失败：{0}")]
    Scan(String),
    /// 加载插件失败（找不到、dlopen 失败等）。
    #[error("插件加载失败：{0}")]
    Load(String),
    /// 插件没有编辑器。
    #[error("该插件没有原生界面")]
    NoEditor,
    /// 当前平台无法承载插件原生界面（如 Linux 上 baseview 后端不可用）。
    #[error("当前平台不支持插件原生界面")]
    EditorUnsupported,
    /// 插件的界面只支持「嵌入宿主窗口」，而宿主没给它窗口。
    #[error("该插件的界面需要宿主提供一个原生窗口")]
    EditorNeedsHostWindow,
    /// 编辑器操作失败。
    #[error("插件界面错误：{0}")]
    Editor(String),
}
