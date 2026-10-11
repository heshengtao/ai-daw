use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use clack_host::prelude::*;
use ai_daw_plugin_api::{Plugin, PluginKind};
use serde::{Deserialize, Serialize};
use vst3_host::Vst3Host;

use crate::clap::ClapHostedPlugin;
use crate::editor::EditorHandle;
use crate::error::HostError;
use crate::vst3::Vst3HostedPlugin;

/// 扫描到的一个插件条目。
///
/// `id` 是 `"<format>:<unique_id>"`，跨格式唯一；工程文件里持久化的就是它。
#[derive(Debug, Clone)]
pub struct ScannedPlugin {
    /// 全局唯一 id（`vst3:<uid>` / `clap:<plugin_id>`）。
    pub id: String,
    /// 展示名。
    pub name: String,
    /// 厂商。
    pub vendor: String,
    /// 乐器 / 效果。
    pub kind: PluginKind,
    /// 格式标识：`"vst3"` / `"clap"`。
    pub format: &'static str,
    /// 是否接受 MIDI。
    pub accepts_midi: bool,
    /// 是否自带原生编辑器（CLAP 扫描阶段不可靠，加载后才准）。
    pub has_editor: bool,
    /// 插件包路径。
    pub path: PathBuf,
}

/// 可持久化的插件目录条目（不含任何不可序列化的句柄）。
///
/// 扫描很慢，所以结果缓存成它、存到磁盘；重开工程时按 `id` + `path` 重新加载。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPlugin {
    /// 全局唯一 id（`vst3:<uid>` / `clap:<plugin_id>`）。
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: PluginKind,
    /// 格式标识：`"vst3"` / `"clap"`。
    pub format: String,
    pub accepts_midi: bool,
    pub has_editor: bool,
    pub path: PathBuf,
}

impl ScannedPlugin {
    /// 转成可持久化的目录条目。
    pub fn catalog(&self) -> CatalogPlugin {
        CatalogPlugin {
            id: self.id.clone(),
            name: self.name.clone(),
            vendor: self.vendor.clone(),
            kind: self.kind,
            format: self.format.to_string(),
            accepts_midi: self.accepts_midi,
            has_editor: self.has_editor,
            path: self.path.clone(),
        }
    }

    /// 加载插件实例（已适配成引擎的 [`Plugin`]）。
    ///
    /// 会 dlopen 插件包，**不可**在音频线程的实时回调里调用（`Player` 的构造不在回调内，安全）。
    /// CLAP 走 [`crate::dispatch::on_main_thread`]：它的 `init` 与参数枚举只能在主线程调。
    pub fn load(&self) -> Result<Box<dyn Plugin>, HostError> {
        match self.format {
            "vst3" => {
                let plugin = vst3_host::simple::load_plugin(&self.path)
                    .map_err(|e| HostError::Load(e.to_string()))?;
                Ok(Box::new(Vst3HostedPlugin::from_loaded(self.id.clone(), plugin)))
            }
            "clap" => {
                let plugin_id = clap_plugin_id(&self.id)?;
                // 走统一的加载入口：它负责把 init / 参数枚举派发到主线程。
                let (plugin, _) = load_clap_plugin(&self.id, &self.path, &plugin_id)?;
                Ok(Box::new(plugin))
            }
            other => Err(HostError::Load(format!("未知插件格式：{other}"))),
        }
    }
}

/// 在**当前线程**上加载一个 CLAP 实例（含编辑器句柄）。
///
/// **必须在主线程调用**：`from_loaded` 内部的 `init` / 参数枚举都是主线程专属。
fn load_clap_here(
    id: &str,
    path: &Path,
    plugin_id: &CStr,
) -> Result<(ClapHostedPlugin, crate::clap::ClapEditorHandle), HostError> {
    // SAFETY: 加载外部动态库本身不安全；所有 CLAP 宿主都要承担这个风险。
    let entry = unsafe { PluginEntry::load(path) }.map_err(|e| HostError::Load(e.to_string()))?;
    let hosted = ClapHostedPlugin::from_loaded(id.to_string(), &entry, plugin_id)?;
    let handle = hosted.editor_handle();
    Ok((hosted, handle))
}

/// 加载一个 CLAP 实例（含编辑器句柄），负责把主线程专属的初始化派发到主线程。
///
/// 所有 CLAP 加载入口都必须走这里，别直接调 [`ClapHostedPlugin::from_loaded`]。
/// 主线程已退出时退回就地加载：线程不理想，但插件仍能用，不会整条加载不出来。
pub fn load_clap_plugin(
    id: &str,
    path: &Path,
    plugin_id: &CStr,
) -> Result<(ClapHostedPlugin, crate::clap::ClapEditorHandle), HostError> {
    let (id, path, plugin_id) = (id.to_string(), path.to_path_buf(), plugin_id.to_owned());
    let (dispatch_id, dispatch_path, dispatch_plugin_id) =
        (id.clone(), path.clone(), plugin_id.clone());
    match crate::dispatch::on_main_thread(move || {
        load_clap_here(&dispatch_id, &dispatch_path, &dispatch_plugin_id)
    }) {
        Ok(result) => result,
        Err(_) => load_clap_here(&id, &path, &plugin_id),
    }
}

/// 扫描本机全部 VST3 与 CLAP 插件。
///
/// 会遍历标准安装目录并**实例化**每个插件来读元数据，可能耗时数秒——**不可**在音频线程调用。
/// 单个插件出错会被跳过，不影响整体结果。
pub fn scan() -> Vec<ScannedPlugin> {
    scan_with(&[], &[])
}

/// 同 [`scan`]，但额外扫描给定的目录。
///
/// 宿主用它把「插件管理器安装/导入的库」也纳入扫描：那些插件由应用自己管在应用数据目录里，
/// 不在系统标准目录下，所以必须显式传进来。
pub fn scan_with(vst3_extra: &[PathBuf], clap_extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    let mut out = scan_vst3(vst3_extra);
    out.extend(scan_clap(clap_extra));
    out
}

/// 扫描并产出可持久化的目录。
pub fn scan_catalog() -> Vec<CatalogPlugin> {
    scan_catalog_with(&[], &[])
}

/// 同 [`scan_catalog`]，但额外扫描给定的目录（宿主传入插件管理器的托管目录）。
pub fn scan_catalog_with(vst3_extra: &[PathBuf], clap_extra: &[PathBuf]) -> Vec<CatalogPlugin> {
    scan_with(vst3_extra, clap_extra)
        .iter()
        .map(ScannedPlugin::catalog)
        .collect()
}

/// 按目录条目加载插件（已适配成引擎的 [`Plugin`]）。
pub fn load_catalog_plugin(entry: &CatalogPlugin) -> Result<Box<dyn Plugin>, HostError> {
    load_catalog_plugin_with_editor(entry).map(|(plugin, _)| plugin)
}

/// 同 [`load_catalog_plugin`]，另外把编辑器句柄带出来。
///
/// 宿主据此把「轨道 → 编辑器句柄」登记下来，界面才能为该轨道打开原生窗口。
pub fn load_catalog_plugin_with_editor(
    entry: &CatalogPlugin,
) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    match entry.format.as_str() {
        "vst3" => {
            let plugin = vst3_host::simple::load_plugin(&entry.path)
                .map_err(|e| HostError::Load(e.to_string()))?;
            let hosted = Vst3HostedPlugin::from_loaded(entry.id.clone(), plugin);
            let handle = hosted.editor_handle();
            Ok((Box::new(hosted), Some(EditorHandle::Vst3(handle))))
        }
        "clap" => {
            let plugin_id = clap_plugin_id(&entry.id)?;
            let (hosted, handle) = load_clap_plugin(&entry.id, &entry.path, &plugin_id)?;
            Ok((Box::new(hosted), Some(EditorHandle::Clap(handle))))
        }
        other => Err(HostError::Load(format!("未知插件格式：{other}"))),
    }
}

// ---------------------------------------------------------------------------
// VST3
// ---------------------------------------------------------------------------

fn scan_vst3(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    let mut builder = Vst3Host::builder().scan_default_paths();
    for dir in extra {
        builder = builder.add_scan_path(dir);
    }
    let Ok(mut host) = builder.build() else {
        return Vec::new();
    };
    let Ok(infos) = host.discover_plugins() else {
        return Vec::new();
    };
    infos
        .into_iter()
        .map(|i| ScannedPlugin {
            id: format!("vst3:{}", i.uid),
            name: i.name.clone(),
            vendor: i.vendor.clone(),
            kind: if i.category.contains("Instrument") {
                PluginKind::Instrument
            } else {
                PluginKind::Effect
            },
            format: "vst3",
            accepts_midi: i.has_midi_input,
            has_editor: i.has_gui,
            path: i.path.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// CLAP
// ---------------------------------------------------------------------------

/// CLAP 的标准安装目录。
fn clap_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Audio/Plug-Ins/CLAP"));
        dirs.push(home.join(".clap"));
    }
    #[cfg(target_os = "macos")]
    dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/clap"));
        dirs.push(PathBuf::from("/usr/local/lib/clap"));
    }
    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from(r"C:\Program Files\Common Files\CLAP"));
        dirs.push(PathBuf::from(r"C:\Program Files (x86)\Common Files\CLAP"));
    }
    dirs
}

fn scan_clap(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    let mut out = Vec::new();
    let mut dirs = clap_dirs();
    for dir in extra {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("clap") {
                continue;
            }
            out.extend(read_clap_bundle(&path));
        }
    }
    out
}

/// 读出一个 .clap 文件里导出的全部插件。
///
/// 整段都派发到主线程：判断有没有界面要实例化一次（`has_gui`），而 CLAP 的 `init` 与
/// `is_api_supported` 都是主线程专属。
fn read_clap_bundle(path: &Path) -> Vec<ScannedPlugin> {
    let owned = path.to_path_buf();
    match crate::dispatch::on_main_thread(move || read_clap_bundle_here(&owned)) {
        Ok(out) => out,
        // 主线程已退出：就地读（那时已经没有更合规的选择了）。
        Err(_) => read_clap_bundle_here(path),
    }
}

/// [`read_clap_bundle`] 的实现。**必须在主线程上跑。**
fn read_clap_bundle_here(path: &Path) -> Vec<ScannedPlugin> {
    // SAFETY: 加载外部动态库本身不安全；所有 CLAP 宿主都要承担这个风险。
    let Ok(entry) = (unsafe { PluginEntry::load(path) }) else {
        return Vec::new();
    };
    let Some(factory) = entry.get_plugin_factory() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for descriptor in factory.plugin_descriptors() {
        let Some(id) = descriptor.id() else { continue };
        let plugin_id = id.to_string_lossy().into_owned();
        out.push(ScannedPlugin {
            id: format!("clap:{plugin_id}"),
            name: descriptor
                .name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| plugin_id.clone()),
            vendor: descriptor
                .vendor()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or_default(),
            kind: if descriptor.features().any(|f| f.to_bytes() == b"instrument") {
                PluginKind::Instrument
            } else {
                PluginKind::Effect
            },
            format: "clap",
            accepts_midi: true,
            // GUI 支持要实例化一次才知道（与 VST3 的扫描口径一致）。
            has_editor: crate::clap::has_gui(&entry, id),
            path: path.to_path_buf(),
        });
    }
    out
}

/// 从 `clap:<plugin_id>` 里取出插件 id。
fn clap_plugin_id(id: &str) -> Result<CString, HostError> {
    let raw = id.strip_prefix("clap:").unwrap_or(id);
    CString::new(raw).map_err(|e| HostError::Load(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_returns_well_formed_entries() {
        // 结果取决于本机装了什么插件（CI 上可能为空），这里只要求扫描不 panic、id 格式正确。
        for p in scan() {
            assert!(
                p.id.starts_with(&format!("{}:", p.format)),
                "插件 id 必须以 `<format>:` 开头：{}",
                p.id
            );
            assert!(!p.name.is_empty());
        }
    }
}
