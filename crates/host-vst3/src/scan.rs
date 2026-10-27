use std::ffi::{CStr, CString};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use ai_daw_plugin_api::{Plugin, PluginKind};
use clack_host::prelude::*;
use serde::{Deserialize, Serialize};
use vst3_host::{SafeDiscoverySkip, Vst3Host};

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
    /// 插件自述（VST3 moduleinfo 的 `Description` / CLAP descriptor 的 description）。
    pub description: Option<String>,
    /// 插件包路径。
    pub path: PathBuf,
}

/// 可持久化的插件目录条目（不含任何不可序列化的句柄）。
///
/// 扫描很慢，所以结果缓存成它、存到磁盘；重开工程时按 `id` + `path` 重新加载。
///
/// `PartialEq` 供宿主判断「这次扫描结果与已装目录是否一致」：一致就不必重建音频宿主。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// 插件自述；旧版目录 JSON 没有这个字段，读成 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
            description: self.description.clone(),
            path: self.path.clone(),
        }
    }

    /// 加载插件实例（已适配成引擎的 [`Plugin`]）。
    ///
    /// 会 dlopen 插件包，**不可**在音频线程的实时回调里调用（`Player` 的构造不在回调内，安全）。
    /// CLAP 走 [`crate::dispatch::on_main_thread`]：它的 `init` 与参数枚举只能在主线程调。
    ///
    /// VST3 始终在**进程内**加载；需要崩溃隔离的离线路径走
    /// [`load_catalog_plugin_with_editor_mode`]。
    pub fn load(&self) -> Result<Box<dyn Plugin>, HostError> {
        match self.format {
            "vst3" => {
                let plugin = load_vst3_plugin(&self.path, false)?;
                Ok(Box::new(Vst3HostedPlugin::from_loaded(
                    self.id.clone(),
                    plugin,
                )))
            }
            "clap" => {
                let plugin_id = clap_plugin_id(&self.id)?;
                // 走统一的加载入口：它负责把 init / 参数枚举派发到主线程。
                let (plugin, _) = load_clap_plugin(&self.id, &self.path, &plugin_id)?;
                Ok(Box::new(plugin))
            }
            "au" => crate::au::load(&self.id).map(|(plugin, _)| plugin),
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

/// 单个 VST3 元数据探针的超时。
///
/// 正常插件读元数据远用不到一秒；Waves / iZotope 这类在加载时会弹模态框把扫描线程卡死，
/// 超时后直接杀掉探针子进程，扫描继续往下走。
const VST3_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 扫描时被跳过的插件（崩溃 / 超时 / 探测失败）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedPlugin {
    /// 插件包路径。
    pub path: PathBuf,
    /// 跳过原因（给界面展示用，不要拿来程序判断）。
    pub reason: String,
}

/// 一次完整扫描的结果：成功条目 + 被跳过的插件 + 扫描器自身是否跑得起来。
#[derive(Debug, Default)]
pub struct ScanReport {
    /// 扫描成功的插件目录。
    pub plugins: Vec<CatalogPlugin>,
    /// 崩溃 / 超时 / 探测失败的插件。
    pub skipped: Vec<SkippedPlugin>,
    /// 扫描器整体没跑起来的原因（例如探针 sidecar 缺失）。
    ///
    /// `None` 表示扫描确实执行过；此时 [`Self::plugins`] 为空只代表本机没有插件。
    pub error: Option<String>,
}

/// 扫描本机全部 VST3 与 CLAP 插件。
///
/// VST3 的元数据在**独立探针子进程**里读（插件崩了只死子进程），所以本函数不会因为某个
/// 坏插件带走宿主；CLAP 只读插件描述符，不实例化。可能耗时较久——**不可**在音频线程调用。
pub fn scan() -> Vec<ScannedPlugin> {
    scan_with(&[], &[], &[])
}

/// 同 [`scan`]，但额外扫描给定的目录。
///
/// 宿主用它把「插件管理器安装/导入的库」也纳入扫描：那些插件由应用自己管在应用数据目录里，
/// 不在系统标准目录下，所以必须显式传进来。
///
/// `au_extra` 只在 macOS 上有意义（AU 的托管目录是用户级 Components 目录）。
pub fn scan_with(
    vst3_extra: &[PathBuf],
    clap_extra: &[PathBuf],
    au_extra: &[PathBuf],
) -> Vec<ScannedPlugin> {
    let mut out = scan_vst3(vst3_extra);
    out.extend(scan_clap(clap_extra));
    out.extend(crate::au::scan(au_extra));
    out
}

/// 扫描并产出可持久化的目录。
pub fn scan_catalog() -> Vec<CatalogPlugin> {
    scan_catalog_with(&[], &[], &[])
}

/// 同 [`scan_catalog`]，但额外扫描给定的目录（宿主传入插件管理器的托管目录）。
pub fn scan_catalog_with(
    vst3_extra: &[PathBuf],
    clap_extra: &[PathBuf],
    au_extra: &[PathBuf],
) -> Vec<CatalogPlugin> {
    scan_report_with(vst3_extra, clap_extra, au_extra).plugins
}

/// 同 [`scan_catalog_with`]，但把被跳过的插件与扫描器错误一并带出来。
///
/// 宿主用 `skipped` 在插件面板里提示「哪些插件没能扫描」，用 `error` 区分
/// 「确实没有插件」与「扫描根本没跑起来」（探针缺失等）。
pub fn scan_report_with(
    vst3_extra: &[PathBuf],
    clap_extra: &[PathBuf],
    au_extra: &[PathBuf],
) -> ScanReport {
    let vst3 = scan_vst3_report(vst3_extra);
    let clap = scan_clap_report(clap_extra);
    let mut plugins: Vec<CatalogPlugin> = vst3.plugins.iter().map(ScannedPlugin::catalog).collect();
    plugins.extend(clap.plugins.iter().map(ScannedPlugin::catalog));
    plugins.extend(crate::au::scan(au_extra).iter().map(ScannedPlugin::catalog));
    let mut skipped = vst3.skipped;
    skipped.extend(clap.skipped);
    ScanReport {
        plugins,
        skipped,
        error: vst3.error,
    }
}

/// VST3 扫描的内部结果（还没转成可持久化的 [`CatalogPlugin`]）。
struct Vst3ScanReport {
    plugins: Vec<ScannedPlugin>,
    skipped: Vec<SkippedPlugin>,
    error: Option<String>,
}

/// 按目录条目加载插件（已适配成引擎的 [`Plugin`]）。
pub fn load_catalog_plugin(entry: &CatalogPlugin) -> Result<Box<dyn Plugin>, HostError> {
    load_catalog_plugin_with_editor(entry).map(|(plugin, _)| plugin)
}

/// 按目录条目加载插件（已适配成引擎的 [`Plugin`]），并保证 **VST3 在它自己的插件线程上实例化**。
///
/// 应用里所有「要一个能用 / 能开界面的实例」的路径都必须走这里，别直接调
/// [`load_catalog_plugin_with_editor_mode`]：VST3 的 JUCE 静态状态会记住**第一次碰到这个 DLL 的
/// 线程**，换一条线程再建实例或开界面，插件就会去等一条早就不跑消息的线程（表现为加载 / 开界面
/// 永久卡住）。同一个模块永远落在同一条线程上，这条约束才算成立。
pub fn load_catalog_plugin_on_host(
    entry: &CatalogPlugin,
) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    match entry.format.as_str() {
        "vst3" => {
            let entry = entry.clone();
            let module = entry.path.clone();
            crate::plugin_thread::run_on(&module, crate::plugin_thread::LOAD_TIMEOUT, move || {
                load_catalog_plugin_with_editor_mode(&entry, false)
            })
            .and_then(|result| result)
        }
        // CLAP 的 init / 参数枚举必须主线程（`load_clap_plugin` 内部会派发）；AU 同理。
        _ => load_catalog_plugin_with_editor_mode(entry, false),
    }
}

/// 加载一个插件、现场读一点东西（参数表 / 描述符），**读完就地释放**。
///
/// 关键在「就地」：实例只在它自己的插件线程上创建与销毁，绝不跨线程搬运。界面里的参数面板、
/// MCP 的 `describe_plugin` 都走这条路——它们以前直接在 tokio 工作线程上实例化 VST3，正是
/// 「先点一下插件、之后界面就打不开」的根因。
pub fn inspect_catalog_plugin<R, F>(entry: &CatalogPlugin, inspect: F) -> Result<R, HostError>
where
    R: Send + 'static,
    F: FnOnce(&mut dyn Plugin) -> R + Send + 'static,
{
    match entry.format.as_str() {
        "vst3" => {
            let entry = entry.clone();
            let module = entry.path.clone();
            crate::plugin_thread::run_on(&module, crate::plugin_thread::LOAD_TIMEOUT, move || {
                let (mut plugin, _) = load_catalog_plugin_with_editor_mode(&entry, false)?;
                Ok(inspect(&mut *plugin))
            })
            .and_then(|result| result)
        }
        _ => {
            // CLAP / AU：整段派到主线程，连实例的释放也在主线程上。
            let entry = entry.clone();
            crate::dispatch::on_main_thread(move || {
                let (mut plugin, _) = load_catalog_plugin_with_editor_mode(&entry, false)?;
                Ok(inspect(&mut *plugin))
            })
            .and_then(|result| result)
        }
    }
}

/// 同 [`load_catalog_plugin`]，另外把编辑器句柄带出来。
///
/// 宿主据此把「轨道 → 编辑器句柄」登记下来，界面才能为该轨道打开原生窗口。
pub fn load_catalog_plugin_with_editor(
    entry: &CatalogPlugin,
) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    load_catalog_plugin_with_editor_mode(entry, false)
}

/// 同 [`load_catalog_plugin_with_editor`]，但可以选择把 VST3 放进 helper 子进程。
///
/// `isolate_vst3` 只用于**离线渲染**（预览 / 导出 / 分析）：隔离路径每个音频块都要走一次
/// JSON + base64 的管道往返，实时回调付不起这个开销，也不该在回调里等子进程。
/// CLAP / AU 目前没有隔离实现，仍按原路径在进程内加载。
pub fn load_catalog_plugin_with_editor_mode(
    entry: &CatalogPlugin,
    isolate_vst3: bool,
) -> Result<(Box<dyn Plugin>, Option<EditorHandle>), HostError> {
    match entry.format.as_str() {
        "vst3" => {
            let plugin = load_vst3_plugin(&entry.path, isolate_vst3)?;
            let hosted = Vst3HostedPlugin::from_loaded(entry.id.clone(), plugin);
            let handle = hosted.editor_handle();
            Ok((Box::new(hosted), Some(EditorHandle::Vst3(handle))))
        }
        "clap" => {
            let plugin_id = clap_plugin_id(&entry.id)?;
            let (hosted, handle) = load_clap_plugin(&entry.id, &entry.path, &plugin_id)?;
            Ok((Box::new(hosted), Some(EditorHandle::Clap(handle))))
        }
        "au" => crate::au::load(&entry.id),
        other => Err(HostError::Load(format!("未知插件格式：{other}"))),
    }
}

// ---------------------------------------------------------------------------
// VST3
// ---------------------------------------------------------------------------

/// 加载一个 VST3 实例。
///
/// `isolate` 为 true 时用 `vst3-host` 的 helper 子进程托管：插件在 `dlopen` / 构造 /
/// 处理音频时崩溃只会杀掉 helper，宿主拿到 `Error::PluginCrashed`（加载阶段则直接返回
/// 错误），不会再带着整个应用一起退出。
///
/// helper 的查找由 `vst3-host` 负责：先看 `VST3_HOST_HELPER_PATH`，再看主程序旁边
/// （Tauri 的 `externalBin` 会把 sidecar 放在那里），最后是 cargo target 目录。
fn load_vst3_plugin(path: &Path, isolate: bool) -> Result<vst3_host::Plugin, HostError> {
    // 先钉住模块：插件会在自己的线程上留隐藏消息窗口，模块一卸载，那些窗口过程就成了
    // 指向已解除映射内存的野指针（`0xC0000005`，崩溃模块显示 `xxx.vst3_unloaded`）。
    // 见 `module_pin` 的说明。
    crate::module_pin::pin(path);

    let mut host = Vst3Host::builder()
        // 与 `vst3_host::simple::load_plugin` 的默认配置保持一致；真实采样率/块大小由
        // `Vst3HostedPlugin::prepare` 通过 `reconfigure` 再告诉插件。
        .sample_rate(44_100.0)
        .block_size(512)
        .input_channels(2)
        .output_channels(2)
        .with_process_isolation(isolate)
        .build()
        .map_err(|e| HostError::Load(e.to_string()))?;
    host.load_plugin(path)
        .map_err(|e| HostError::Load(e.to_string()))
}

fn scan_vst3(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    scan_vst3_report(extra).plugins
}

/// 扫描 VST3，并把「崩溃 / 超时 / 失败」的插件单独记下来。
///
/// 这里**绝不**在本进程里实例化插件：`vst3_host::Vst3Host::discover_plugins` 会逐个
/// `LoadLibrary` + `FreeLibrary`，坏插件在卸载阶段（WER 里的 `xxx.vst3_unloaded`、
/// `0xc0000005`）或加载时 `abort()` 会直接带走宿主。改用 `discover_plugins_safe`：
/// 每个插件起一个 `vst3-host-probe` 子进程，崩的只是子进程，宿主把它记成「跳过」。
fn scan_vst3_report(extra: &[PathBuf]) -> Vst3ScanReport {
    if let Err(error) = ensure_vst3_probe() {
        return Vst3ScanReport {
            plugins: Vec::new(),
            skipped: Vec::new(),
            error: Some(error),
        };
    }

    let mut builder = Vst3Host::builder()
        .scan_default_paths()
        .probe_timeout(VST3_PROBE_TIMEOUT);
    for dir in extra {
        builder = builder.add_scan_path(dir);
    }
    let host = match builder.build() {
        Ok(host) => host,
        Err(error) => {
            return Vst3ScanReport {
                plugins: Vec::new(),
                skipped: Vec::new(),
                error: Some(format!("初始化 VST3 扫描器失败：{error}")),
            }
        }
    };

    let report = host.discover_plugins_safe();
    if let Some(error) = report.error {
        return Vst3ScanReport {
            plugins: Vec::new(),
            skipped: Vec::new(),
            error: Some(error),
        };
    }

    // 探针文件在、但根本起不来（缺 DLL / 权限被拦）时，每个插件都会报「failed to spawn
    // probe」。这属于扫描器自身坏了，不能当成「这些插件都有问题」——尤其不能拿空结果
    // 覆盖宿主磁盘上的上一次成功扫描。
    let probe_cannot_spawn = report.plugins.is_empty()
        && !report.skipped.is_empty()
        && report.skipped.iter().all(|skip| {
            matches!(
                skip,
                SafeDiscoverySkip::Failed { detail, .. } if detail.contains("failed to spawn probe")
            )
        });
    if probe_cannot_spawn {
        return Vst3ScanReport {
            plugins: Vec::new(),
            skipped: Vec::new(),
            error: Some("扫描探针无法启动（vst3-host-probe 缺失依赖或被安全软件拦截）".into()),
        };
    }

    let skipped = report
        .skipped
        .into_iter()
        .map(|skip| SkippedPlugin {
            path: skip.path().to_path_buf(),
            reason: match skip {
                SafeDiscoverySkip::Crashed { detail, .. } => format!("探测时崩溃：{detail}"),
                SafeDiscoverySkip::TimedOut { .. } => "探测超时（插件可能弹出了阻塞对话框）".into(),
                SafeDiscoverySkip::Failed { detail, .. } => format!("探测失败：{detail}"),
            },
        })
        .collect();

    let plugins = report
        .plugins
        .into_iter()
        .map(|detailed| {
            let info = detailed.info;
            ScannedPlugin {
                id: format!("vst3:{}", info.uid),
                name: info.name.clone(),
                vendor: info.vendor.clone(),
                kind: if info.category.contains("Instrument") {
                    PluginKind::Instrument
                } else {
                    PluginKind::Effect
                },
                format: "vst3",
                accepts_midi: info.has_midi_input,
                has_editor: info.has_gui,
                description: vst3_native_description(&info.path),
                path: info.path,
            }
        })
        .collect();

    Vst3ScanReport {
        plugins,
        skipped,
        error: None,
    }
}

/// 让 `vst3-host` 能找到扫描探针（`vst3-host-probe`）。
///
/// 上游查找探针时用 `exe_dir.join("vst3-host-probe")`——Windows 上不会自动补 `.exe`，
/// 而 Tauri 打包出来的 sidecar 叫 `vst3-host-probe.exe`。所以这里自己找一次，找到后用
/// `VST3_HOST_PROBE_PATH` 明确告诉上游；找不到就返回错误，让宿主把「扫描没跑起来」
/// 显示出来，而不是静默地报告「本机没有插件」。
pub fn ensure_vst3_probe() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("VST3_HOST_PROBE_PATH").map(PathBuf::from) {
        if configured.exists() {
            return Ok(configured);
        }
    }

    let exe = std::env::current_exe().map_err(|e| format!("拿不到当前可执行文件路径：{e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "当前可执行文件没有父目录".to_string())?;
    let name = probe_binary_name();

    // 打包后的 sidecar 就在主程序旁边。
    let direct = dir.join(name);
    if direct.exists() {
        return Ok(register_probe(direct));
    }

    // 开发 / 测试：二进制在 target/{debug,release} 里（test 在 deps 子目录）。
    for ancestor in dir.ancestors() {
        for profile in ["debug", "release"] {
            let candidate = ancestor.join("target").join(profile).join(name);
            if candidate.exists() {
                return Ok(register_probe(candidate));
            }
        }
        if ancestor.join("Cargo.toml").exists() {
            break;
        }
    }

    Err(format!(
        "找不到扫描探针 {name}（应在主程序旁边或 cargo target 里）。\
         已安装的插件无法扫描；请重新构建 sidecar（npm run build:vst3-helper）。"
    ))
}

fn register_probe(path: PathBuf) -> PathBuf {
    std::env::set_var("VST3_HOST_PROBE_PATH", &path);
    path
}

fn probe_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "vst3-host-probe.exe"
    } else {
        "vst3-host-probe"
    }
}

/// 读 VST3 包里的 `moduleinfo.json`，取插件自述。
///
/// 这个文件可能是**带尾随逗号的非法 JSON**（生成器很随意），所以不严格解析：
/// 找到第一个 `"Description"`，取它后面的字符串就够了。读不到就返回 `None`。
fn vst3_native_description(path: &Path) -> Option<String> {
    let file = path.join("Contents/Resources/moduleinfo.json");
    let text = std::fs::read_to_string(file).ok()?;
    let at = text.find("\"Description\"")?;
    let rest = &text[at + "\"Description\"".len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let start = rest.find('"')?;
    let rest = &rest[start + 1..];
    let end = rest.find('"')?;
    let value = rest[..end].trim();
    (!value.is_empty()).then(|| value.to_string())
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

/// CLAP 扫描探针在 stdout 上输出的协议前缀。
///
/// 插件自己也可能往 stdout 打印东西，所以不能把整行都当 JSON：只认带这个前缀的那一行。
pub const CLAP_PROBE_MARKER: &str = "AI_DAW_CLAP_SCAN:";

/// 单个 CLAP 探针的超时（理由同 VST3：卡住的插件不能拖住整轮扫描）。
const CLAP_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 子进程已经退出后，再给读取线程一点时间把已写出的结果送回来（有界）。
const PROBE_OUTPUT_GRACE: Duration = Duration::from_millis(250);

/// 扫描探针从 `.clap` 里读出的一个插件条目（JSON 协议，宿主与探针共用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClapScanEntry {
    /// CLAP 自己的插件 id（不含 `clap:` 前缀）。
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// 描述符 feature 里带 `instrument` 就是乐器。
    pub is_instrument: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// CLAP 扫描的内部结果。
#[derive(Default)]
struct ClapScanReport {
    plugins: Vec<ScannedPlugin>,
    skipped: Vec<SkippedPlugin>,
}

fn scan_clap(extra: &[PathBuf]) -> Vec<ScannedPlugin> {
    scan_clap_report(extra).plugins
}

/// 扫描 CLAP：每个 `.clap` 都交给 `clap-scan-probe` 子进程读描述符。
///
/// 和 VST3 一样，插件在加载/卸载（`dlclose` / `FreeLibrary` / bundle unload）阶段崩溃只会
/// 杀掉探针，宿主把它记进跳过名单。找不到探针时（开发 / 测试环境没构建 sidecar）退回进程内
/// 读取：功能不丢，但那一刻没有崩溃隔离。
fn scan_clap_report(extra: &[PathBuf]) -> ClapScanReport {
    let mut report = ClapScanReport::default();
    let probe = ensure_clap_probe();
    let mut warned = false;

    let mut dirs = clap_dirs();
    for dir in extra {
        if !dirs.contains(dir) {
            dirs.push(dir.clone());
        }
    }

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("clap") {
                continue;
            }
            match &probe {
                Some(probe) => match run_clap_probe(probe, &path, CLAP_PROBE_TIMEOUT) {
                    Ok(entries) => report.plugins.extend(
                        entries
                            .into_iter()
                            .map(|entry| scanned_from_clap(entry, &path)),
                    ),
                    Err(reason) => report.skipped.push(SkippedPlugin { path, reason }),
                },
                None => {
                    if !warned {
                        warned = true;
                        eprintln!(
                            "未找到 clap-scan-probe：CLAP 扫描退回进程内读取（没有崩溃隔离）"
                        );
                    }
                    report.plugins.extend(read_clap_bundle_in_process(&path));
                }
            }
        }
    }

    report
}

/// 把探针读出的条目转成扫描条目。
fn scanned_from_clap(entry: ClapScanEntry, path: &Path) -> ScannedPlugin {
    ScannedPlugin {
        id: format!("clap:{}", entry.id),
        name: entry.name,
        vendor: entry.vendor,
        kind: if entry.is_instrument {
            PluginKind::Instrument
        } else {
            PluginKind::Effect
        },
        format: "clap",
        accepts_midi: true,
        // CLAP 的描述符没有「有没有界面」这一位；扫描阶段又不该为了它实例化插件
        // （`has_gui` 要建实例，坏插件会因此把宿主带走）。先按「有界面」登记，
        // 真正加载时 `ClapHostedPlugin::from_loaded` 会再确认一次。
        has_editor: true,
        description: entry.description,
        path: path.to_path_buf(),
    }
}

/// 进程内读取一个 `.clap`（仅在探针不可用时兜底）。
fn read_clap_bundle_in_process(path: &Path) -> Vec<ScannedPlugin> {
    match read_clap_bundle_entries(path) {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| scanned_from_clap(entry, path))
            .collect(),
        Err(error) => {
            eprintln!("读取 CLAP 失败 {}：{error}", path.display());
            Vec::new()
        }
    }
}

/// 加载一个 `.clap` 并读出它的全部插件描述符。**不实例化插件。**
///
/// 这是兜底路径：`entry` 在函数结束时释放（可能触发插件的卸载代码）。正常扫描走
/// [`clap_bundle_probe_json`]，把这段风险隔离在子进程里。
fn read_clap_bundle_entries(path: &Path) -> Result<Vec<ClapScanEntry>, String> {
    // SAFETY: 加载外部动态库本身不安全；所有 CLAP 宿主都要承担这个风险。
    let entry = unsafe { PluginEntry::load(path) }.map_err(|e| e.to_string())?;
    read_clap_entries(&entry)
}

/// 从已加载的 entry 里读描述符。不碰插件实例，所以与线程无关。
fn read_clap_entries(entry: &PluginEntry) -> Result<Vec<ClapScanEntry>, String> {
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| "插件没有导出 factory".to_string())?;

    let mut out = Vec::new();
    for descriptor in factory.plugin_descriptors() {
        let Some(id) = descriptor.id() else { continue };
        let plugin_id = id.to_string_lossy().into_owned();
        out.push(ClapScanEntry {
            name: descriptor
                .name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| plugin_id.clone()),
            vendor: descriptor
                .vendor()
                .map(|vendor| vendor.to_string_lossy().into_owned())
                .unwrap_or_default(),
            is_instrument: descriptor.features().any(|f| f.to_bytes() == b"instrument"),
            description: descriptor
                .description()
                .map(|value| value.to_string_lossy().into_owned())
                .filter(|value| !value.trim().is_empty()),
            id: plugin_id,
        });
    }
    Ok(out)
}

/// 供 `clap-scan-probe` 使用：读出条目并序列化成一行 JSON。
///
/// **故意不卸载模块**：有些插件的 `DllMain` / 静态析构在卸载阶段崩溃，那会发生在输出之前，
/// 把本来能用的插件误判成坏插件。探针进程随后立刻退出，这份泄漏由系统回收。
pub fn clap_bundle_probe_json(path: &Path) -> Result<String, String> {
    // SAFETY: 加载外部动态库本身不安全；所有 CLAP 宿主都要承担这个风险。
    let entry = unsafe { PluginEntry::load(path) }.map_err(|e| e.to_string())?;
    let entries = read_clap_entries(&entry)?;
    let json = serde_json::to_string(&entries).map_err(|e| e.to_string())?;
    std::mem::forget(entry);
    Ok(json)
}

/// 定位 `clap-scan-probe`（Windows 带 `.exe`，其它平台不带）。
///
/// 查找顺序：`AI_DAW_CLAP_PROBE_PATH` → 主程序旁边（Tauri sidecar）→ cargo
/// `target/{debug,release}`（`cargo run` / `cargo test`）。
fn ensure_clap_probe() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("AI_DAW_CLAP_PROBE_PATH").map(PathBuf::from) {
        if configured.exists() {
            return Some(configured);
        }
    }

    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = clap_probe_binary_name();

    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }

    for ancestor in dir.ancestors() {
        for profile in ["debug", "release"] {
            let candidate = ancestor.join("target").join(profile).join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if ancestor.join("Cargo.toml").exists() {
            break;
        }
    }

    None
}

fn clap_probe_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "clap-scan-probe.exe"
    } else {
        "clap-scan-probe"
    }
}

/// 子进程别再弹控制台窗口。
///
/// 打包后的宿主是 GUI 子系统、自己没有控制台；控制台子进程会被 Windows **另开一个黑窗**，
/// 扫描时每个插件闪一下。探针自己也声明了 GUI 子系统（见 `bin/clap-scan-probe.rs`），
/// 这里再兜一层：`AI_DAW_CLAP_PROBE_PATH` 指到别处（比如自己编译的调试版）时同样不闪。
fn hide_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：不分配控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

/// 让探针读一个 `.clap`，带超时；插件崩溃 / 卡住只影响这个子进程。
fn run_clap_probe(
    probe: &Path,
    plugin: &Path,
    timeout: Duration,
) -> Result<Vec<ClapScanEntry>, String> {
    let mut command = Command::new(probe);
    command
        .arg(plugin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // stderr 只用来诊断；插件日志可能很吵，直接丢掉。
        .stderr(Stdio::null());
    hide_console(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("无法启动 CLAP 扫描探针：{e}"))?;

    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("探针没有 stdout 管道".into());
    };

    // 只认带协议前缀的那一行：插件自己打印的内容会被忽略。
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
                Ok(_) => {
                    if let Some(json) = line.trim_end().strip_prefix(CLAP_PROBE_MARKER) {
                        let _ = tx.send(Some(json.to_string()));
                        return;
                    }
                }
            }
        }
    });

    /// 距离 deadline 的剩余时间；至少给一点，避免子进程刚写完就因为超时把输出丢掉。
    fn remaining(deadline: Instant) -> Duration {
        deadline
            .saturating_duration_since(Instant::now())
            .max(PROBE_OUTPUT_GRACE)
    }

    let deadline = Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            // 已经拿到结果：插件就算之后崩溃 / 卡住也不影响这次扫描。
            Ok(Some(json)) => {
                let _ = child.kill();
                let _ = child.wait();
                return parse_clap_probe_output(&json);
            }
            Ok(None) | Err(TryRecvError::Disconnected) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("探针没有返回扫描结果".into());
            }
            Err(TryRecvError::Empty) => {}
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                // 子进程已退出：给它已经写出的结果一个很短的收集窗口。
                let line = rx.recv_timeout(remaining(deadline)).ok().flatten();
                return match line {
                    Some(json) => parse_clap_probe_output(&json),
                    None if status.success() => Err("探针没有返回扫描结果".into()),
                    None => Err(format!("探测时崩溃（{status}）")),
                };
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("探测超时（插件可能弹出了阻塞对话框）".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("等待探针失败：{error}"));
            }
        }
    }
}

fn parse_clap_probe_output(json: &str) -> Result<Vec<ClapScanEntry>, String> {
    serde_json::from_str(json).map_err(|e| format!("探针输出无法解析：{e}"))
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
        let plugins = scan();
        for p in &plugins {
            assert!(
                p.id.starts_with(&format!("{}:", p.format)),
                "插件 id 必须以 `<format>:` 开头：{}",
                p.id
            );
            assert!(!p.name.is_empty());
        }
        // 「目录扫描」与「AudioComponent 注册表枚举」必须按 id 去重：同一个 AU 不能出现两次。
        let mut ids: Vec<&str> = plugins.iter().map(|p| p.id.as_str()).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "扫描结果里有重复的插件 id");
    }

    #[test]
    fn a_broken_bundle_is_skipped_instead_of_failing_the_scan() {
        // 没有探针（没构建 sidecar）时这条回归测试没法跑。
        if ensure_vst3_probe().is_err() {
            eprintln!("未找到 vst3-host-probe，跳过。");
            return;
        }

        let root = std::env::temp_dir().join(format!("ai-daw-broken-vst3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let broken = root.join("DefinitelyNotAPlugin.vst3");
        std::fs::create_dir_all(&broken).expect("创建假插件包");

        let report = scan_report_with(&[root.clone()], &[], &[]);
        assert!(
            report.error.is_none(),
            "扫描器本身应当跑完：{:?}",
            report.error
        );
        assert!(
            !report.plugins.iter().any(|p| p.path.starts_with(&root)),
            "坏插件不该出现在成功列表里"
        );
        assert!(
            report
                .skipped
                .iter()
                .any(|skip| skip.path.starts_with(&root) && !skip.reason.is_empty()),
            "坏插件应当被记入跳过名单并带原因：{:?}",
            report.skipped
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_broken_clap_bundle_is_skipped_instead_of_failing_the_scan() {
        // 没有探针（没构建 sidecar）时这条回归测试没法跑。
        if ensure_clap_probe().is_none() {
            eprintln!("未找到 clap-scan-probe，跳过。");
            return;
        }

        let root = std::env::temp_dir().join(format!("ai-daw-broken-clap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建临时目录");
        let broken = root.join("DefinitelyNotAClap.clap");
        std::fs::write(&broken, b"not a clap plugin").expect("写入假插件");

        let report = scan_report_with(&[], &[root.clone()], &[]);
        assert!(
            report.error.is_none(),
            "扫描器本身应当跑完：{:?}",
            report.error
        );
        assert!(
            !report.plugins.iter().any(|p| p.path.starts_with(&root)),
            "坏插件不该出现在成功列表里"
        );
        assert!(
            report
                .skipped
                .iter()
                .any(|skip| skip.path.starts_with(&root) && !skip.reason.is_empty()),
            "坏插件应当被记入跳过名单并带原因：{:?}",
            report.skipped
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
