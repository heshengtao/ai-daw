//! 插件管理器：浏览 / 下载 / 安装开放注册表（Open Audio Stack）里的插件，
//! 导入本地插件，更新与删除已装插件。
//!
//! **放在哪**：装进**应用数据目录**下的托管库（`plugins/vst3`、`plugins/clap`），
//! 而不是系统标准插件目录。理由：① 用户级可写，不需要管理员权限；② 卸载干净；
//! ③ 与用户自己装的插件互不干扰。宿主扫描时会把这两个目录一并扫进来
//! （见 [`crate::manager::rescan_with_managed`]）。
//!
//! **安全**：只下载并**校验 sha256**后解压；解压时防目录穿越；`installer` 类型的条目
//! **绝不自动执行**，只提供「在浏览器打开下载页」。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

use ai_daw_host_vst3::CatalogPlugin;

use crate::state::{AppState, EVENT_PLUGINS};

/// 注册表地址：**ai-daw 仓库里的快照**（打包时由 `tools/fetch-plugin-registry.mjs` 更新）。
///
/// 客户端不再每次直连上游 Open Audio Stack——上游只在打包时被请求一次。可用环境变量覆盖，
/// 便于测试 / 镜像。
const REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/heshengtao/ai-daw/main/src-tauri/resources/plugin-registry.json";
/// 随包快照：网络与本地缓存都不可用时的最后兜底（`include_str!` 保证一定有）。
const BUNDLED_REGISTRY: &str = include_str!("../resources/plugin-registry.json");
/// 注册表缓存有效期：6 小时。
const REGISTRY_TTL: Duration = Duration::from_secs(6 * 3600);
/// 拉取注册表 JSON 的整体超时。
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// 下载插件二进制的整体超时（大文件 + 慢链路，给足）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(1800);
/// 单个归档解压后的体积上限（防解压炸弹）。
const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// 同一个下载地址最多试几次（每次之间退避）。
const DOWNLOAD_ATTEMPTS: u32 = 3;

/// 插件管理器进度广播的通道名（前端据此画卡片下边缘的进度条）。
pub const EVENT_MANAGER: &str = "engine://manager";

/// 一次安装的进度快照。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManagerProgress {
    /// 正在处理的注册表 slug。
    pub slug: String,
    /// `downloading` / `extracting` / `installing` / `error`。
    pub phase: String,
    /// 已接收字节（仅 `downloading` 有意义）。
    pub received: u64,
    /// 总字节（0 = 未知）。
    pub total: u64,
    /// 失败原因（仅 `error` 有意义）。
    pub message: Option<String>,
}

/// 广播一次进度。`error` 之外的阶段 `message` 为 `None`。
fn emit_progress(
    app: &AppHandle,
    slug: &str,
    phase: &str,
    received: u64,
    total: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_MANAGER,
        ManagerProgress {
            slug: slug.to_string(),
            phase: phase.to_string(),
            received,
            total,
            message,
        },
    );
}

// ---------------------------------------------------------------------------
// 注册表原始结构（只取我们需要的字段，缺字段一律回退默认值）
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawRegistry {
    #[serde(default)]
    plugins: HashMap<String, RawPackage>,
}

#[derive(Deserialize)]
struct RawPackage {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    versions: HashMap<String, RawVersion>,
}

#[derive(Deserialize)]
struct RawVersion {
    #[serde(default)]
    name: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    license: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    verified: bool,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    systems: Vec<RawSystem>,
    #[serde(default)]
    architectures: Vec<String>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    url: String,
}

#[derive(Deserialize)]
struct RawSystem {
    #[serde(default, rename = "type")]
    kind: String,
}

// ---------------------------------------------------------------------------
// 送给前端的结构
// ---------------------------------------------------------------------------

/// 注册表里的一个可下载文件。
#[derive(Serialize, Clone)]
pub struct RegistryFile {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// `archive`（可一键安装）或 `installer`（只能打开下载页，绝不自动执行）。
    pub kind: String,
    /// 归档里包含的插件格式（我们只留 `vst3` / `clap` / `au`）。
    pub contains: Vec<String>,
    pub platforms: Vec<String>,
    pub archs: Vec<String>,
    /// 与当前系统 + 架构匹配，可直接安装。
    pub compatible: bool,
}

/// 注册表里的一个插件（取最新版本）。
#[derive(Serialize, Clone)]
pub struct RegistryPlugin {
    pub slug: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub license: String,
    /// `instrument` / `effect` / …
    pub kind: String,
    pub tags: Vec<String>,
    pub image: Option<String>,
    pub homepage: Option<String>,
    pub version: String,
    pub verified: bool,
    pub files: Vec<RegistryFile>,
}

/// 一次注册表查询的结果。
#[derive(Serialize)]
pub struct RegistryIndex {
    pub plugins: Vec<RegistryPlugin>,
    /// 当前系统（`mac` / `win` / `linux`）。
    pub platform: String,
    /// 当前架构（`arm64` / `x64` / …）。
    pub arch: String,
    /// 数据时间戳（Unix 秒）。
    pub fetched_at: i64,
    /// 是否来自缓存（未联网拉取）。
    pub from_cache: bool,
}

/// 一条「本管理器装过的插件」记录（存在托管库的 `manifest.json` 里）。
#[derive(Serialize, Deserialize, Clone)]
pub struct InstallRecord {
    /// 唯一键：注册表安装 = slug；本地导入 = `local:<文件名>`。
    pub key: String,
    /// 注册表 slug；本地导入为 `null`。
    #[serde(default)]
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// `registry` 或 `local`。
    pub source: String,
    /// 装进去的文件 / 目录的绝对路径。
    pub files: Vec<String>,
    pub installed_at: i64,
}

/// 用户**自己安装**的插件（扫描到但不在托管库里）——本管理器不删它们。
#[derive(Serialize)]
pub struct ForeignPlugin {
    /// 全局唯一 id（`vst3:<uid>` / `clap:<id>`）。
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub kind: ai_daw_plugin_api::PluginKind,
    /// 格式标识（`vst3` / `clap`）。
    pub format: String,
    /// 插件所在路径（目录或文件）。
    pub path: String,
}

/// 安装请求（来自前端的注册表条目）。
#[derive(Deserialize)]
pub struct InstallRequest {
    pub slug: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

// ---------------------------------------------------------------------------
// 托管目录
// ---------------------------------------------------------------------------

/// 管理器自己的目录布局。
///
/// VST3 / CLAP 装在应用数据目录下；**AU 例外**：AU 的 `.component` 只能被系统的
/// `AudioComponent` 从标准目录发现（见 [`au_managed_dir`]），放应用数据目录加载不了。
struct ManagedDirs {
    root: PathBuf,
    vst3: PathBuf,
    clap: PathBuf,
    /// AU 的托管目录（macOS 上是用户级 Components 目录；其它平台是占位路径）。
    au: PathBuf,
    downloads: PathBuf,
    staging: PathBuf,
}

impl ManagedDirs {
    fn resolve(app: &AppHandle) -> Result<Self, String> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("拿不到应用数据目录：{e}"))?
            .join("plugins");
        Ok(Self {
            vst3: root.join("vst3"),
            clap: root.join("clap"),
            au: au_managed_dir().unwrap_or_else(|| root.join("au")),
            downloads: root.join("downloads"),
            staging: root.join("staging"),
            root,
        })
    }

    fn ensure(&self) -> Result<(), String> {
        for dir in [
            &self.root,
            &self.vst3,
            &self.clap,
            &self.au,
            &self.downloads,
            &self.staging,
        ] {
            fs::create_dir_all(dir).map_err(|e| format!("创建目录失败 {dir:?}：{e}"))?;
        }
        Ok(())
    }

    fn dir_for(&self, kind: &str) -> &Path {
        match kind {
            "clap" => &self.clap,
            "au" => &self.au,
            _ => &self.vst3,
        }
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("manifest.json")
    }
}

/// AU 的托管目录：macOS 上必须是**系统会扫描的用户级 Components 目录**。
///
/// 与 VST3 / CLAP 不同，AU 的 `.component` **不能**由宿主从任意路径加载——它只能通过系统的
/// `AudioComponent` 注册表发现，而注册表只扫标准目录。所以「导入 AU」= 把它装到
/// `~/Library/Audio/Plug-Ins/Components`，随后系统与宿主都能看到它。其它平台没有 AU。
///
/// 删除 / 更新仍然照常：管理器是按 manifest 里记录的文件路径操作的，与目录无关。
fn au_managed_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Audio/Plug-Ins/Components"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// 宿主扫描时要额外扫描的目录（VST3、CLAP、AU 各一组）。
///
/// 拿不到应用数据目录时返回空（扫描照常，只是看不到托管的插件）。
pub fn managed_scan_dirs(app: &AppHandle) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>) {
    match ManagedDirs::resolve(app) {
        Ok(dirs) => (
            vec![dirs.vst3.clone()],
            vec![dirs.clap.clone()],
            vec![dirs.au.clone()],
        ),
        Err(_) => (Vec::new(), Vec::new(), Vec::new()),
    }
}

/// 扫描结果缓存的格式版本；字段或语义变了就加一（旧缓存直接丢弃）。
const CATALOG_CACHE_VERSION: u32 = 1;

/// 上一次成功扫描的插件目录（落盘缓存）。
#[derive(Serialize, Deserialize)]
struct CatalogCache {
    version: u32,
    plugins: Vec<CatalogPlugin>,
}

fn catalog_cache_file(app: &AppHandle) -> Option<PathBuf> {
    ManagedDirs::resolve(app)
        .ok()
        .map(|dirs| dirs.root.join("catalog-cache.json"))
}

/// 读上次成功扫描的缓存：路径已经不存在的条目会被丢掉。
///
/// 启动时先用它把外部插件装上，界面不必等整轮扫描（每个 VST3 都要起一个探针子进程）。
pub fn cached_catalog(app: &AppHandle) -> Vec<CatalogPlugin> {
    let Some(file) = catalog_cache_file(app) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(&file) else {
        return Vec::new();
    };
    let Ok(cache) = serde_json::from_str::<CatalogCache>(&text) else {
        return Vec::new();
    };
    if cache.version != CATALOG_CACHE_VERSION {
        return Vec::new();
    }
    cache
        .plugins
        .into_iter()
        .filter(|plugin| plugin.path.exists())
        .collect()
}

fn save_catalog_cache(app: &AppHandle, plugins: &[CatalogPlugin]) {
    let Some(file) = catalog_cache_file(app) else {
        return;
    };
    let cache = CatalogCache {
        version: CATALOG_CACHE_VERSION,
        plugins: plugins.to_vec(),
    };
    let Ok(json) = serde_json::to_string_pretty(&cache) else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = fs::create_dir_all(dir);
    }
    // 先写临时文件再替换：中途崩掉也不会留下半截 JSON。
    let tmp = file.with_extension("json.tmp");
    if fs::write(&tmp, json).is_err() {
        return;
    }
    // Windows 的 rename 不会覆盖已存在的目标，先删旧的。
    let _ = fs::remove_file(&file);
    let _ = fs::rename(&tmp, &file);
}

/// 重新扫描插件（含托管目录），更新共享状态并广播给界面。
///
/// VST3 用探针子进程逐个探测：坏插件崩在子进程里，只会进「跳过名单」，不会带走应用。
pub fn rescan_with_managed(app: &AppHandle) -> Vec<CatalogPlugin> {
    let (vst3, clap, au) = managed_scan_dirs(app);
    let report = ai_daw_host_vst3::scan_report_with(&vst3, &clap, &au);
    let error = report.error;
    let skipped = report.skipped;
    let mut catalog = report.plugins;

    if let Some(error) = &error {
        eprintln!("插件扫描未完成：{error}");
    }
    for skip in &skipped {
        eprintln!("跳过插件 {}：{}", skip.path.display(), skip.reason);
    }

    if error.is_some() {
        // 扫描器临时跑不起来（例如升级后 sidecar 没就位）时，沿用上次缓存里的条目，
        // 别让「暂时扫不了」变成「插件全消失」。
        for cached in cached_catalog(app) {
            if !catalog.iter().any(|plugin| plugin.id == cached.id) {
                catalog.push(cached);
            }
        }
    } else {
        save_catalog_cache(app, &catalog);
    }

    let state = app.state::<AppState>();
    state.set_scan_report(skipped, error);
    state.install_catalog(catalog.clone());
    let _ = app.emit(EVENT_PLUGINS, &catalog);
    catalog
}

/// 注册表缓存目录（应用缓存目录下的 `plugin-manager`）。
fn registry_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("拿不到缓存目录：{e}"))?
        .join("plugin-manager"))
}

/// 后台预热注册表缓存（启动时调用，纯尽力而为）。
///
/// 注册表是 1.9 MB 的静态 JSON（gzip 后约 280 KB），在慢网络下第一次拉取要十几秒；
/// 启动时先在后台拉到磁盘缓存，用户点开「可下载」时通常就能直接命中缓存、秒开。
/// 缓存仍新鲜时 [`load_registry`] 直接返回，不会重复联网。失败静默（下次打开面板再试）。
pub fn warm_registry_cache(app: &AppHandle) {
    let Ok(dir) = registry_cache_dir(app) else {
        return;
    };
    let network = app.state::<StoreRuntime>().network();
    let sources = registry_sources(&network);
    let resolved = resolve_proxy(&network);
    let _ = load_registry(&dir, false, &sources, resolved.proxy.as_ref());
}

// ---------------------------------------------------------------------------
// 平台与兼容性
// ---------------------------------------------------------------------------

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "win"
    } else {
        "linux"
    }
}

fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "arm" => "arm32",
        "x86" => "x32",
        other => other,
    }
}

/// 该文件是否与当前系统 + 架构匹配。缺 `systems` / `architectures` 视为不限。
fn file_compatible(file: &RawFile, plat: &str, arch: &str) -> bool {
    let os_ok = file.systems.is_empty() || file.systems.iter().any(|s| s.kind == plat);
    let arch_ok = file.architectures.is_empty() || file.architectures.iter().any(|a| a == arch);
    os_ok && arch_ok
}

/// 把原始注册表归一成一份精简列表：只保留我们能加载（含 VST3 / CLAP / AU）的插件与文件。
fn normalize(raw: RawRegistry, plat: &str, arch: &str) -> Vec<RegistryPlugin> {
    let mut out = Vec::new();
    for (map_key, pkg) in raw.plugins {
        let slug = if pkg.slug.is_empty() {
            map_key
        } else {
            pkg.slug.clone()
        };
        // 取「最新版本」；缺失时若只有一版就用那一版。
        let version = if pkg.versions.contains_key(&pkg.version) {
            pkg.version.clone()
        } else if pkg.versions.len() == 1 {
            pkg.versions.keys().next().cloned().unwrap_or_default()
        } else {
            continue;
        };
        let Some(ver) = pkg.versions.get(&version) else {
            continue;
        };

        let files: Vec<RegistryFile> = ver
            .files
            .iter()
            .filter(|f| {
                !f.url.is_empty()
                    && f.contains
                        .iter()
                        .any(|c| c == "vst3" || c == "clap" || c == "au")
            })
            .map(|f| RegistryFile {
                url: f.url.clone(),
                sha256: f.sha256.clone(),
                size: f.size,
                kind: f.kind.clone(),
                contains: f
                    .contains
                    .iter()
                    .filter(|c| *c == "vst3" || *c == "clap" || *c == "au")
                    .cloned()
                    .collect(),
                platforms: f.systems.iter().map(|s| s.kind.clone()).collect(),
                archs: f.architectures.clone(),
                compatible: file_compatible(f, plat, arch),
            })
            .collect();
        if files.is_empty() {
            continue;
        }
        out.push(RegistryPlugin {
            slug,
            name: ver.name.clone(),
            author: ver.author.clone(),
            description: ver.description.clone(),
            license: ver.license.clone(),
            kind: ver.kind.clone(),
            tags: ver.tags.clone(),
            image: ver.image.clone(),
            homepage: ver.url.clone(),
            version,
            verified: ver.verified,
            files,
        });
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

// ---------------------------------------------------------------------------
// 可下载插件搜索（MCP：search_plugin_store）
// ---------------------------------------------------------------------------

/// 注册表搜索结果：一条可下载插件 + 本机状态 + 为什么命中。
#[derive(Serialize, Clone)]
pub struct RegistryMatch {
    pub slug: String,
    pub name: String,
    pub author: String,
    pub description: String,
    pub license: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub homepage: Option<String>,
    pub version: String,
    pub verified: bool,
    /// 本机是否已经装过这个 slug。
    pub installed: bool,
    /// 当前系统是否有可**一键安装**的归档（有 sha256 的 zip/tar 等）。
    pub installable: bool,
    /// 当前系统可用的格式（`vst3` / `clap` / `au`），去重。
    pub formats: Vec<String>,
    /// 首选归档的体积（字节）。
    pub size_bytes: u64,
    /// 匹配分数（只用于排序）。
    pub score: i64,
    /// 命中原因。
    pub why: String,
}

/// 搜索归一化：小写、只留字母数字（中文按字保留）。
fn search_norm(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 查询拆成词；整句都是中文时就是一个词。
fn search_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = query
        .split_whitespace()
        .map(search_norm)
        .filter(|term| !term.is_empty())
        .collect();
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        let whole = search_norm(query);
        if !whole.is_empty() {
            terms.push(whole);
        }
    }
    terms
}

/// 归档格式优先级：VST3 通用性最好，其次 CLAP，最后 AU。
fn format_rank(contains: &[String]) -> i32 {
    if contains.iter().any(|c| c == "vst3") {
        3
    } else if contains.iter().any(|c| c == "clap") {
        2
    } else if contains.iter().any(|c| c == "au") {
        1
    } else {
        0
    }
}

/// 选当前系统最合适的一个归档（只考虑可一键安装、且带 sha256 的）。
fn best_installable_file(plugin: &RegistryPlugin) -> Option<&RegistryFile> {
    plugin
        .files
        .iter()
        .filter(|file| file.compatible && file.kind != "installer" && !file.sha256.is_empty())
        .max_by_key(|file| (format_rank(&file.contains), file.size))
}

fn kind_matches(plugin_kind: &str, want: Option<&str>) -> bool {
    let Some(want) = want.map(str::trim).filter(|want| !want.is_empty()) else {
        return true;
    };
    match want.to_ascii_lowercase().as_str() {
        "all" | "全部" => true,
        "instrument" | "instruments" | "乐器" => plugin_kind.eq_ignore_ascii_case("instrument"),
        "effect" | "effects" | "效果" | "效果器" => plugin_kind.eq_ignore_ascii_case("effect"),
        other => plugin_kind.eq_ignore_ascii_case(other),
    }
}

/// 给一条注册表条目打分；没有任何词命中时返回 `None`。
fn score_plugin(plugin: &RegistryPlugin, terms: &[String]) -> Option<(i64, String)> {
    if terms.is_empty() {
        return Some((0, "按名字列出".into()));
    }
    let name = search_norm(&plugin.name);
    let slug = search_norm(&plugin.slug);
    let author = search_norm(&plugin.author);
    let description = search_norm(&plugin.description);
    let tags: Vec<String> = plugin.tags.iter().map(|tag| search_norm(tag)).collect();

    let mut score = 0i64;
    let mut why: Vec<String> = Vec::new();
    for term in terms {
        let hit = if name == *term {
            why.push(format!("名字精确命中「{term}」"));
            120
        } else if name.starts_with(term) {
            why.push(format!("名字以「{term}」开头"));
            90
        } else if name.contains(term) {
            why.push(format!("名字含「{term}」"));
            70
        } else if slug.contains(term) {
            why.push(format!("slug 含「{term}」"));
            55
        } else if tags.iter().any(|tag| tag == term || tag.contains(term)) {
            why.push(format!("标签命中「{term}」"));
            50
        } else if description.contains(term) {
            why.push(format!("描述提到「{term}」"));
            35
        } else if author.contains(term) {
            why.push(format!("作者命中「{term}」"));
            30
        } else {
            // 多个词时要求全部命中，避免「electric」把一堆不相关的鼓机带出来。
            return None;
        };
        score += hit;
    }
    if plugin
        .files
        .iter()
        .any(|file| file.compatible && file.kind != "installer")
    {
        score += 15;
    }
    why.dedup();
    Some((score, why.join("；")))
}

/// 纯函数版搜索（方便测试）：过滤 + 打分 + 排序 + 截断。
fn search_registry_entries(
    plugins: &[RegistryPlugin],
    query: &str,
    kind: Option<&str>,
    limit: usize,
    installed: &HashSet<String>,
) -> Vec<RegistryMatch> {
    let terms = search_terms(query);
    // query 全是标点、没有可用词时不要退化成「列出全部 563 个插件」。
    if terms.is_empty() {
        return Vec::new();
    }
    let limit = limit.clamp(1, 50);
    let mut matches: Vec<RegistryMatch> = plugins
        .iter()
        .filter(|plugin| kind_matches(&plugin.kind, kind))
        .filter_map(|plugin| {
            let (score, why) = score_plugin(plugin, &terms)?;
            let mut formats: Vec<String> = plugin
                .files
                .iter()
                .filter(|file| file.compatible)
                .flat_map(|file| file.contains.iter().cloned())
                .collect();
            formats.sort();
            formats.dedup();
            let installable = best_installable_file(plugin).is_some();
            Some(RegistryMatch {
                slug: plugin.slug.clone(),
                name: plugin.name.clone(),
                author: plugin.author.clone(),
                description: plugin.description.clone(),
                license: plugin.license.clone(),
                kind: plugin.kind.clone(),
                tags: plugin.tags.clone(),
                homepage: plugin.homepage.clone(),
                version: plugin.version.clone(),
                verified: plugin.verified,
                installed: installed.contains(&plugin.slug),
                installable,
                formats,
                size_bytes: best_installable_file(plugin)
                    .map(|file| file.size)
                    .unwrap_or(0),
                score,
                why,
            })
        })
        .collect();
    // 分数高的在前；同分时已安装的排后面（用户更关心「能装什么」）。
    matches.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.installed.cmp(&b.installed))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    matches.truncate(limit);
    matches
}

/// 读取注册表（联网 → 缓存 → 随包快照）并归一化成当前平台的条目。
fn load_registry_index(app: &AppHandle) -> Result<Vec<RegistryPlugin>, String> {
    let cache_dir = registry_cache_dir(app)?;
    let network = app.state::<StoreRuntime>().network();
    let sources = registry_sources(&network);
    let resolved = resolve_proxy(&network);
    let (raw, _) = load_registry(&cache_dir, false, &sources, resolved.proxy.as_ref())?;
    Ok(normalize(raw, platform(), arch()))
}

/// 本机已安装的注册表 slug（来自托管库的 `manifest.json`）。
fn installed_slugs(app: &AppHandle) -> HashSet<String> {
    let Ok(dirs) = ManagedDirs::resolve(app) else {
        return HashSet::new();
    };
    read_manifest(&dirs)
        .into_values()
        .filter_map(|record| record.slug)
        .collect()
}

/// 搜索**可下载**插件（不是本机已装的目录）。
pub fn search_registry(
    app: &AppHandle,
    query: &str,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<RegistryMatch>, String> {
    let plugins = load_registry_index(app)?;
    let installed = installed_slugs(app);
    Ok(search_registry_entries(
        &plugins, query, kind, limit, &installed,
    ))
}

/// 从注册表安装 / 更新一个插件（MCP：install_plugin）。
///
/// **会真的联网下载并解压可执行插件**。调用方必须先拿到用户的明确同意。
pub fn install_registry_plugin(app: &AppHandle, slug: &str) -> Result<String, String> {
    let plugins = load_registry_index(app)?;
    // 只认 slug，不按名字猜：同名插件不止一个，装错东西的代价太大。
    let plugin = plugins
        .iter()
        .find(|plugin| plugin.slug == slug)
        .ok_or_else(|| {
            format!("注册表里没有 slug「{slug}」。先用 search_plugin_store 查准确 slug。")
        })?;
    let file = best_installable_file(plugin).ok_or_else(|| {
        format!(
            "「{}」没有当前系统可一键安装的归档（可能只提供安装器或其它平台构建）。\
             请在浏览器打开下载页手动安装：{}",
            plugin.name,
            plugin.homepage.as_deref().unwrap_or("（无主页）")
        )
    })?;

    let request = InstallRequest {
        slug: plugin.slug.clone(),
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        url: file.url.clone(),
        sha256: file.sha256.clone(),
        size: file.size,
    };
    install_from_registry(app, &request)?;
    Ok(format!(
        "已安装「{}」{}（{}，格式 {}）。rescan_plugins 已自动完成，现在可用 list_plugins 看到它。",
        plugin.name,
        plugin.version,
        plugin.author,
        if file.contains.is_empty() {
            "未知".to_string()
        } else {
            file.contains.join("/")
        }
    ))
}

// ---------------------------------------------------------------------------
// 网络配置（镜像 / 代理）
// ---------------------------------------------------------------------------

/// 代理的取用方式。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    /// 直连，绝不走代理（哪怕系统开着）。
    Direct,
    /// **跟随系统代理**（默认）：先看环境变量，再问操作系统。
    #[default]
    System,
    /// 手动指定地址（`http://` / `https://` / `socks5://`）。
    Manual,
}

/// 模型下载源的**区域偏好**：决定先试国外源（HF / GitHub）还是国内源（魔搭）。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ModelSourcePreference {
    /// **自动探测连通性**（默认）：下载前快速探一下两边，谁先通/更快就排谁，失败再回退另一个。
    #[default]
    Auto,
    /// 优先国外源（HF / GitHub）。
    International,
    /// 优先国内源（魔搭）。
    Domestic,
}

/// **执行后端**偏好：决定用哪个 ONNX Runtime 执行后端跑本地模型。
///
/// 三个选项全部随包（零下载），**默认 CPU**。设置页只列出本机平台支持的那些
/// （如 Intel Mac 没有 CoreML、非 Windows 没有 DirectML）。
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BackendPreference {
    /// 纯 CPU（默认，全平台可用）。
    #[default]
    Cpu,
    /// Apple CoreML（macOS Apple Silicon）。
    #[serde(rename = "coreml")]
    CoreMl,
    /// Windows DirectML。
    #[serde(rename = "directml")]
    DirectMl,
}

impl BackendPreference {
    /// 映射到后端种类。
    pub fn kind(self) -> ai_daw_local_ai::backend::BackendKind {
        use ai_daw_local_ai::backend::BackendKind;
        match self {
            BackendPreference::Cpu => BackendKind::Cpu,
            BackendPreference::CoreMl => BackendKind::CoreMl,
            BackendPreference::DirectMl => BackendKind::DirectMl,
        }
    }
}

/// 插件源的网络配置（存在应用配置目录的 `store.json`）。
///
/// **不提供任何默认镜像/代理地址**：默认只有官方源与直连。国内的镜像与加速站五花八门、
/// 稳定性也各有差异，交给用户自己填更合适。
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StoreNetwork {
    /// 注册表镜像（按顺序尝试，**在官方源之前**）。留空 = 只用官方源。
    #[serde(default)]
    pub registry_mirrors: Vec<String>,
    /// 下载代理前缀：只重写 `github.com` / `githubusercontent.com` 的地址；
    /// 留空 = 直连。可含 `{url}` 占位符（如 `https://proxy.example/{url}`），
    /// 不含占位符则直接拼在前缀之后（如 `https://proxy.example/https://github.com/...`）。
    /// 无论走不走代理，下载都会**校验 sha256**——字节被改会被拒。
    ///
    /// 注意：这是**地址重写**，不是 HTTP 代理。真正走代理请用 [`StoreNetwork::proxy_mode`]。
    #[serde(default)]
    pub download_prefix: String,
    /// 代理取用方式。
    #[serde(default)]
    pub proxy_mode: ProxyMode,
    /// 手动模式的代理地址（`proxy_mode == Manual` 时才用）。
    #[serde(default)]
    pub proxy_url: String,
    /// **模型权重**的镜像 / 前置代理前缀（与插件下载共用同一套代理与镜像设置）。
    ///
    /// 每条是前缀模板：含 `{url}` 就替换，不含就直接拼在原地址前（如
    /// `https://mirror.example/` + 原 HF 地址）。留空 = 只用清单里的官方源。
    /// 无论走不走镜像，权重下载都会**校验 sha256**。**不给任何默认地址。**
    #[serde(default)]
    pub model_mirrors: Vec<String>,
    /// 模型下载源的**区域偏好**（自动探测 / 优先国外 / 优先国内）。见 [`ModelSourcePreference`]。
    #[serde(default)]
    pub model_source_preference: ModelSourcePreference,
    /// **执行后端**偏好（自动推荐 / 强制某一种）。见 [`BackendPreference`]。
    #[serde(default)]
    pub backend_preference: BackendPreference,
}

/// 运行时的网络配置（内存 + `store.json` 文件）。
pub struct StoreRuntime {
    network: Mutex<StoreNetwork>,
    file: Option<PathBuf>,
}

impl StoreRuntime {
    /// 从应用配置目录读入 `store.json`（没有或损坏就用默认 = 直连官方源）。
    pub fn load(app: &AppHandle) -> Self {
        let file = app
            .path()
            .app_config_dir()
            .ok()
            .map(|dir| dir.join("store.json"));
        let network = file
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<StoreNetwork>(&t).ok())
            .unwrap_or_default();
        Self {
            network: Mutex::new(network),
            file,
        }
    }

    /// 当前配置快照。
    pub fn network(&self) -> StoreNetwork {
        self.network.lock().unwrap().clone()
    }

    /// 写盘并更新内存（拿不到配置目录时只活在内存里）。
    pub fn set(&self, next: StoreNetwork) -> Result<(), String> {
        if let Some(file) = &self.file {
            if let Some(parent) = file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let text = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())?;
            fs::write(file, text).map_err(|e| e.to_string())?;
        }
        *self.network.lock().unwrap() = next;
        Ok(())
    }
}

/// 实际要依次尝试的注册表源：配置的镜像在前，官方源兜底。
///
/// 环境变量（`AIDAW_STORE_URLS` / `AIDAW_STORE_URL`）优先级最高，完全取代这份列表
/// —— 供测试 / 无界面场景使用。
fn registry_sources(config: &StoreNetwork) -> Vec<String> {
    if let Ok(list) = std::env::var("AIDAW_STORE_URLS") {
        let urls: Vec<String> = list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !urls.is_empty() {
            return urls;
        }
    }
    if let Ok(one) = std::env::var("AIDAW_STORE_URL") {
        if !one.trim().is_empty() {
            return vec![one.trim().to_string()];
        }
    }
    let mut sources = config.registry_mirrors.clone();
    if !sources.iter().any(|u| u == REGISTRY_URL) {
        sources.push(REGISTRY_URL.to_string());
    }
    sources
}

/// 下载地址是否需要走代理（只重写 GitHub 的地址）。
fn is_github_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
        || url.starts_with("http://github.com/")
        || url.contains("githubusercontent.com/")
}

/// 按配置重写下载地址；`prefix` 为空或非 GitHub 地址时原样返回。
fn rewrite_download_url(url: &str, prefix: &str) -> String {
    let prefix = prefix.trim();
    if prefix.is_empty() || !is_github_url(url) {
        return url.to_string();
    }
    if prefix.contains("{url}") {
        prefix.replace("{url}", url)
    } else {
        format!("{prefix}{url}")
    }
}

/// 下载地址的候选列表（按顺序尝试）。
///
/// 配了代理前缀就是 **先代理、再原始地址**——国内下载插件最容易出问题的两件事正是
/// 「代理站抖动/限速」和「GitHub 直连被墙」，两条路都留着才不会一条不通就整体失败。
/// 没配（或不适用）时只有原始地址。
pub(crate) fn download_candidates(url: &str, prefix: &str) -> Vec<String> {
    let rewritten = rewrite_download_url(url, prefix);
    if rewritten == url {
        vec![url.to_string()]
    } else {
        vec![rewritten, url.to_string()]
    }
}

/// 下载失败的两类原因——决定「重试同一个地址」还是「换个地址」。
#[derive(Debug)]
enum DownloadError {
    /// 传输层问题（连不上、被断开、5xx）：值得退避重试，也值得换源。
    Transient(String),
    /// 地址本身的问题（4xx、内容对不上）：同一地址重试没有意义，换源可以再试。
    Permanent(String),
}

impl DownloadError {
    fn message(self) -> String {
        match self {
            DownloadError::Transient(m) | DownloadError::Permanent(m) => m,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// 拉取注册表用的 agent：连接级超时设短，遇到被墙 / 不通的源能**快速失败、切下一个**。
fn fetch_agent(proxy: Option<&ureq::Proxy>) -> ureq::Agent {
    tls_agent(
        proxy,
        Some(Duration::from_secs(5)),
        Some(Duration::from_secs(10)),
        Some(Duration::from_secs(20)),
        Some(FETCH_TIMEOUT),
    )
}

/// 下载插件二进制用的 agent：只保留整体超时（大文件、慢链路）。
fn download_agent(proxy: Option<&ureq::Proxy>) -> ureq::Agent {
    tls_agent(
        proxy,
        None,
        Some(Duration::from_secs(30)),
        None,
        Some(DOWNLOAD_TIMEOUT),
    )
}

/// 探测连通性用的 agent：**每一级超时都设得很短**，好让「被墙 / 不通」的源快速失败，
/// 不至于在选源阶段就把用户卡住（模型下载前用它给国内外源排序）。
pub(crate) fn probe_agent(proxy: Option<&ureq::Proxy>) -> ureq::Agent {
    tls_agent(
        proxy,
        Some(Duration::from_secs(3)),
        Some(Duration::from_secs(3)),
        Some(Duration::from_secs(4)),
        Some(Duration::from_secs(6)),
    )
}

/// 统一的 agent 构造。
///
/// **必须**显式选 native-tls：ureq 的 `TlsConfig` 默认 provider 是 Rustls，而本项目只编进了
/// `native-tls`（不引 rustls 那棵树）；不指定就会 panic：
/// 「uri scheme is https, provider is Rustls but feature is not enabled: rustls」。
///
/// 代理也**显式**设置（`Direct` 传 `None`）：不依赖 ureq 的隐式行为，
/// 「不走代理」就是真的不走。
fn tls_agent(
    proxy: Option<&ureq::Proxy>,
    resolve: Option<Duration>,
    connect: Option<Duration>,
    recv_response: Option<Duration>,
    global: Option<Duration>,
) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_resolve(resolve)
        .timeout_connect(connect)
        .timeout_recv_response(recv_response)
        .timeout_global(global)
        .http_status_as_error(false)
        .user_agent("AI-DAW-PluginManager/0.1")
        .proxy(proxy.cloned())
        .build()
        .into()
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 代理
// ---------------------------------------------------------------------------

/// 解析出来的代理 + 一句人类可读的来源说明（给日志与设置页的「测试」按钮）。
#[derive(Clone)]
pub struct ResolvedProxy {
    /// 真正要用的代理；`None` = 直连。
    pub proxy: Option<ureq::Proxy>,
    /// 来源说明，例如「系统代理（macOS 网络设置）http://127.0.0.1:7892」。
    pub source: String,
}

/// 按配置解析出代理。
///
/// 三种模式。注意 [`ProxyMode::System`] **不能只看环境变量**：macOS 上的「系统代理」
/// （Clash / Surge 那类）写在 SystemConfiguration 里，而 GUI 应用从 Dock 启动时
/// **不继承 shell 的环境变量**——这正是「明明开了代理，软件却直连」的原因。
pub fn resolve_proxy(config: &StoreNetwork) -> ResolvedProxy {
    match config.proxy_mode {
        ProxyMode::Direct => ResolvedProxy {
            proxy: None,
            source: "直连（不使用代理）".into(),
        },
        ProxyMode::Manual => {
            let url = config.proxy_url.trim();
            if url.is_empty() {
                return ResolvedProxy {
                    proxy: None,
                    source: "手动模式但地址为空 → 直连".into(),
                };
            }
            match ureq::Proxy::new(url) {
                Ok(proxy) => ResolvedProxy {
                    proxy: Some(proxy),
                    source: format!("手动代理 {url}"),
                },
                Err(e) => ResolvedProxy {
                    proxy: None,
                    source: format!("手动代理地址无效（{e}）→ 直连"),
                },
            }
        }
        ProxyMode::System => {
            if let Some(proxy) = ureq::Proxy::try_from_env() {
                return ResolvedProxy {
                    proxy: Some(proxy),
                    source: "系统代理（环境变量）".into(),
                };
            }
            match system_proxy_url() {
                Some(url) => match ureq::Proxy::new(&url) {
                    Ok(proxy) => ResolvedProxy {
                        proxy: Some(proxy),
                        source: format!("系统代理（操作系统设置）{url}"),
                    },
                    Err(e) => ResolvedProxy {
                        proxy: None,
                        source: format!("系统代理地址 {url} 无法解析（{e}）→ 直连"),
                    },
                },
                None => ResolvedProxy {
                    proxy: None,
                    source: "未检测到系统代理 → 直连".into(),
                },
            }
        }
    }
}

/// 问操作系统要代理地址（`http://host:port` / `socks5://host:port`）。
///
/// Linux 桌面环境各自为政（GNOME/KDE 设的是 gsettings，不是环境变量），这里先只认
/// 环境变量——那条路已经由 [`ureq::Proxy::try_from_env`] 覆盖，所以返回 `None`。
fn system_proxy_url() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos_system_proxy()
    }
    #[cfg(target_os = "windows")]
    {
        windows_system_proxy()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// 从 `scutil --proxy` 的 `Key : Value` 输出里取一个值。
#[cfg(target_os = "macos")]
fn scutil_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once(" : "))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
}

/// macOS：读「网络 → 代理」。优先 HTTPS，其次 HTTP，最后 SOCKS。
///
/// 三者都是 CONNECT 类代理，拿 HTTP 代理去访问 `https://` 目标是标准做法
/// （Clash / Surge 的 HTTP 端口本身就支持 CONNECT）。
#[cfg(target_os = "macos")]
fn macos_system_proxy() -> Option<String> {
    let out = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for (enable, host, port, scheme) in [
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5"),
    ] {
        if scutil_value(&text, enable).as_deref() != Some("1") {
            continue;
        }
        if let (Some(h), Some(p)) = (scutil_value(&text, host), scutil_value(&text, port)) {
            if !h.is_empty() && !p.is_empty() {
                return Some(format!("{scheme}://{h}:{p}"));
            }
        }
    }
    None
}

/// Windows：读注册表 `HKCU\...\Internet Settings` 的 `ProxyEnable` / `ProxyServer`。
///
/// `ProxyServer` 可能是 `127.0.0.1:7890`，也可能是 `http=...;https=...` 这种按协议分开的形式。
#[cfg(target_os = "windows")]
fn windows_system_proxy() -> Option<String> {
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let out = std::process::Command::new("reg")
        .args(["query", KEY])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let value_of = |name: &str| -> Option<String> {
        text.lines()
            .find(|l| l.trim_start().starts_with(name))
            .map(|l| l.split_whitespace().last().unwrap_or("").to_string())
    };
    let enabled = value_of("ProxyEnable")?;
    if enabled != "0x1" && enabled != "1" {
        return None;
    }
    let server = value_of("ProxyServer")?;
    let host = if let Some(rest) = server.split_once("https=").map(|(_, r)| r) {
        rest.split(';').next().unwrap_or(rest).to_string()
    } else if let Some(rest) = server.split_once("http=").map(|(_, r)| r) {
        rest.split(';').next().unwrap_or(rest).to_string()
    } else {
        server
    };
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    Some(if host.contains("://") {
        host.to_string()
    } else {
        format!("http://{host}")
    })
}

/// 从单个地址拉取并解析注册表。
fn fetch_registry_from(agent: &ureq::Agent, url: &str) -> Result<(RawRegistry, String), String> {
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("连接失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("返回 HTTP {}", resp.status()));
    }
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("读取失败：{e}"))?;
    let raw: RawRegistry = serde_json::from_str(&text).map_err(|e| format!("格式无法解析：{e}"))?;
    Ok((raw, text))
}

/// 拉取注册表 JSON（带磁盘缓存，多源依次尝试）。返回（原始数据，是否来自缓存）。
///
/// 顺序：新鲜的磁盘缓存 → 依次尝试各源（默认只有 ai-daw 快照）→ 旧缓存 → **随包快照**。
/// 随包快照保证「连不上 GitHub 且本地没缓存」时插件管理器仍然可用，而不是整块报错。
fn load_registry(
    cache_dir: &Path,
    force: bool,
    sources: &[String],
    proxy: Option<&ureq::Proxy>,
) -> Result<(RawRegistry, bool), String> {
    let file = cache_dir.join("registry.json");
    let cached = fs::read_to_string(&file).ok();

    if !force {
        let fresh = fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age < REGISTRY_TTL)
            .unwrap_or(false);
        if fresh {
            if let Some(text) = &cached {
                if let Ok(raw) = serde_json::from_str::<RawRegistry>(text) {
                    return Ok((raw, true));
                }
            }
        }
    }

    let agent = fetch_agent(proxy);
    let mut errors = Vec::new();
    for url in sources {
        match fetch_registry_from(&agent, url) {
            Ok((raw, text)) => {
                if fs::create_dir_all(cache_dir).is_ok() {
                    let _ = fs::write(&file, &text);
                }
                return Ok((raw, false));
            }
            Err(e) => errors.push(format!("{url} → {e}")),
        }
    }

    // 所有源都不通：有旧缓存就用旧缓存（过期也比没有强）。
    if let Some(text) = &cached {
        if let Ok(raw) = serde_json::from_str::<RawRegistry>(text) {
            return Ok((raw, true));
        }
    }
    // 连旧缓存都没有：退回随包快照。它是打包时从上游抓的，只是可能比线上旧一点。
    match serde_json::from_str::<RawRegistry>(BUNDLED_REGISTRY) {
        Ok(raw) => {
            eprintln!(
                "[manager] 注册表联网失败且没有本地缓存，改用随包快照：{}",
                errors.join("；")
            );
            Ok((raw, true))
        }
        Err(e) => Err(format!(
            "所有注册表源都失败了，随包快照也无法解析：\n{}\n随包快照：{e}",
            errors.join("\n")
        )),
    }
}

/// 单个下载进度回调：`(已接收字节, 总字节)`；总字节为 0 表示未知。
pub(crate) type DownloadProgress<'a> = &'a mut dyn FnMut(u64, u64);

/// 把候选地址逐个试一遍，每个地址最多试 [`DOWNLOAD_ATTEMPTS`] 次。
///
/// 失败信息里**带上地址**：国内最常见的就是「代理站挂了」和「GitHub 直连不通」，
/// 不写清是哪个地址，用户根本没法判断该改设置还是换时间再试。
pub(crate) fn download_verified(
    candidates: &[String],
    dest: &Path,
    expected_sha256: &str,
    expected_size: u64,
    proxy: Option<&ureq::Proxy>,
    cancel: Option<&AtomicBool>,
    progress: DownloadProgress<'_>,
) -> Result<(), String> {
    let mut last = String::new();
    for (index, url) in candidates.iter().enumerate() {
        for attempt in 1..=DOWNLOAD_ATTEMPTS {
            if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
                return Err("已取消".into());
            }
            match download_from(
                url,
                dest,
                expected_sha256,
                expected_size,
                proxy,
                cancel,
                &mut *progress,
            ) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    let permanent = matches!(err, DownloadError::Permanent(_));
                    last = format!("{url} → {}", err.message());
                    // 地址本身有问题就不必重试同一地址了，但还可以换下一个源。
                    if permanent {
                        break;
                    }
                    if attempt < DOWNLOAD_ATTEMPTS {
                        std::thread::sleep(Duration::from_millis(600 * u64::from(attempt)));
                    }
                }
            }
        }
        if index + 1 < candidates.len() {
            eprintln!("[manager] 换下一个下载源（上一个失败：{last}）");
        }
    }
    Err(last)
}

/// 从**一个**地址下载到 `dest`，边下边算 sha256，并校验大小与哈希。
///
/// 支持**断点续传**：本地已有残缺文件、且注册表给了总大小时，用 `Range` 从断点继续
/// （重试与换源都会走到这里，所以「下到 80% 被掐断」不必从头再来）。
fn download_from(
    url: &str,
    dest: &Path,
    expected_sha256: &str,
    expected_size: u64,
    proxy: Option<&ureq::Proxy>,
    cancel: Option<&AtomicBool>,
    progress: DownloadProgress<'_>,
) -> Result<(), DownloadError> {
    // 断点：只有「知道总大小」且「本地已有一段但不完整」时才续传。
    let mut have = 0u64;
    if expected_size > 0 {
        if let Ok(meta) = fs::metadata(dest) {
            if meta.len() > 0 && meta.len() < expected_size {
                have = meta.len();
            }
        }
    }
    let mut hasher = Sha256::new();
    if have > 0 {
        // 把已经落到本地的那一段重新喂给哈希（不能用上一轮的内存状态——可能是别的进程留下的）。
        let mut file = fs::File::open(dest)
            .map_err(|e| DownloadError::Transient(format!("读取断点文件失败：{e}")))?;
        let mut buf = vec![0u8; 128 * 1024];
        let mut read = 0u64;
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| DownloadError::Transient(format!("读取断点文件失败：{e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            read += n as u64;
        }
        have = read;
        progress(have, expected_size);
    }

    let mut request = download_agent(proxy).get(url);
    if have > 0 {
        request = request.header("Range", format!("bytes={have}-"));
    }
    let resp = request
        .call()
        .map_err(|e| DownloadError::Transient(e.to_string()))?;
    let status = resp.status();
    let mut received = have;
    if status.as_u16() == 206 {
        // 服务器接受了 Range，从断点继续。
    } else if status.is_success() {
        // 服务器忽略了 Range（返回 200）：只能从头来过。
        received = 0;
        have = 0;
        hasher = Sha256::new();
    } else if status.is_server_error() {
        return Err(DownloadError::Transient(format!(
            "HTTP {}",
            status.as_u16()
        )));
    } else {
        return Err(DownloadError::Permanent(format!(
            "HTTP {}",
            status.as_u16()
        )));
    }

    let mut file = if have > 0 {
        fs::OpenOptions::new()
            .append(true)
            .open(dest)
            .map_err(|e| DownloadError::Transient(format!("打开断点文件失败：{e}")))?
    } else {
        fs::File::create(dest)
            .map_err(|e| DownloadError::Permanent(format!("写入失败 {dest:?}：{e}")))?
    };

    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 128 * 1024];
    let mut last_report = Instant::now();
    loop {
        if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            return Err(DownloadError::Permanent("已取消".into()));
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| DownloadError::Transient(format!("下载中断：{e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        std::io::Write::write_all(&mut file, &buf[..n])
            .map_err(|e| DownloadError::Permanent(format!("写入失败：{e}")))?;
        received += n as u64;
        // 节流：128 KB 一块会有上万次回调，按时间上报即可。
        if last_report.elapsed() >= Duration::from_millis(120) {
            progress(received, expected_size);
            last_report = Instant::now();
        }
    }
    progress(received, expected_size);
    drop(file);

    // 大小 / 哈希对不上说明拿到的不是注册表里那个文件。同一个地址重试无意义（内容不会变），
    // 但换个源可能就好了——所以算 Permanent，交给外层换源；残缺文件必须删掉，否则下一轮
    // 会当成「断点」继续拼一个已经错了的文件。
    let cleanup = |msg: String| -> DownloadError {
        let _ = fs::remove_file(dest);
        DownloadError::Permanent(msg)
    };
    if expected_size > 0 && received != expected_size {
        return Err(cleanup(format!(
            "文件大小不符：注册表 {expected_size} 字节，实际下载 {received} 字节"
        )));
    }
    let digest = hex_lower(&hasher.finalize());
    if !expected_sha256.is_empty() && !digest.eq_ignore_ascii_case(expected_sha256) {
        return Err(cleanup(format!(
            "sha256 校验失败：注册表 {expected_sha256}，实际 {digest}"
        )));
    }
    Ok(())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

// ---------------------------------------------------------------------------
// 解压
// ---------------------------------------------------------------------------

/// URL 的小写扩展名：`zip` / `tar.gz` / `tgz` / `clap` / …（无扩展名返回空串）。
fn url_extension(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path).to_lowercase();
    if let Some(rest) = name.strip_suffix(".tar.gz") {
        let _ = rest;
        return "tar.gz".into();
    }
    match name.rsplit_once('.') {
        Some((_, ext)) => ext.to_string(),
        None => String::new(),
    }
}

fn url_file_name(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 把归档解到 `dest`；非归档（如裸 `.clap`）则直接把下载文件放进去。
fn extract_archive(archive: &Path, dest: &Path, url: &str) -> Result<(), String> {
    match url_extension(url).as_str() {
        "zip" => extract_zip(archive, dest),
        "tar.gz" | "tgz" => extract_tar_gz(archive, dest),
        _ => {
            let name = url_file_name(url).ok_or("无法从下载地址推断文件名")?;
            fs::copy(archive, dest.join(name)).map_err(|e| format!("复制失败：{e}"))?;
            Ok(())
        }
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("不是有效的 zip：{e}"))?;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // `enclosed_name` 会拒绝绝对路径与 `..`，防目录穿越。
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(rel);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        total += entry.size();
        if total > MAX_EXTRACTED_BYTES {
            return Err("归档解压后体积过大，已中止".into());
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut writer = fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut writer).map_err(|e| e.to_string())?;
        drop(writer);
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            set_mode(&out, mode);
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    let mut total: u64 = 0;
    for entry in tar
        .entries()
        .map_err(|e| format!("不是有效的 tar.gz：{e}"))?
    {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let rel = entry.path().map_err(|e| e.to_string())?.into_owned();
        // 双保险：自己先挡一次绝对路径 / `..`，`unpack_in` 内部也会再挡。
        if rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            continue;
        }
        total += entry.size();
        if total > MAX_EXTRACTED_BYTES {
            return Err("归档解压后体积过大，已中止".into());
        }
        entry.unpack_in(dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 当前平台上，这个 `.vst3` **目录**（bundle）是否是本机可加载的。
///
/// macOS 的 VST3 必须是 bundle，且内含 `Contents/MacOS/<二进制>`。Windows/Linux 的 bundle
/// 布局与此不同，判定保持宽松、交给对应平台。
#[cfg(target_os = "macos")]
fn vst3_bundle_ok(dir: &Path) -> bool {
    dir.join("Contents").join("MacOS").is_dir()
}

#[cfg(not(target_os = "macos"))]
fn vst3_bundle_ok(_dir: &Path) -> bool {
    true
}

/// 当前平台上，这个 `.vst3` **裸文件**是否是本机可加载的。
///
/// macOS 上 VST3 必须是 bundle，所以裸 `.vst3` 文件一定是别的平台（Windows 的 DLL、Linux 的
/// SO）——放进去也永远不会被扫描到，必须丢弃，否则它会与同名的 mac bundle 抢同一个落点。
#[cfg(target_os = "macos")]
fn vst3_file_ok(_file: &Path) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
fn vst3_file_ok(_file: &Path) -> bool {
    true
}

/// 递归找出 staging 里的插件：`.vst3`（bundle 目录或文件）与 `.clap` 文件。
///
/// **只收当前平台能加载的产物**：一个 release 常把 Windows 与 macOS 的构建打进同一个压缩包、
/// 且文件名相同（例如 `JE-201.vst3`）。若不加筛选，两者会被拍平到同一落点互相覆盖，最后留下
/// 的若是别的平台的二进制，宿主就永远扫不到它。
fn collect_plugin_paths(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    collect_into(root, &mut out);
    out
}

/// 当前平台上，这个 `.component` **目录**（AU bundle）是否是本机可加载的。
///
/// AU 只在 macOS 上存在，且必须是含 `Contents/Info.plist` 的 bundle。
#[cfg(target_os = "macos")]
fn au_bundle_ok(dir: &Path) -> bool {
    dir.join("Contents").join("Info.plist").is_file()
}

#[cfg(not(target_os = "macos"))]
fn au_bundle_ok(_dir: &Path) -> bool {
    false
}

fn collect_into(dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if path.is_dir() {
            match ext.as_str() {
                // VST3 bundle 本身就是插件，不再往下钻；只收当前平台能加载的那种。
                "vst3" => {
                    if vst3_bundle_ok(&path) {
                        out.push(("vst3".into(), path));
                    }
                }
                // AU bundle 同理（非 macOS 一律不收）。
                "component" => {
                    if au_bundle_ok(&path) {
                        out.push(("au".into(), path));
                    }
                }
                _ => collect_into(&path, out),
            }
            continue;
        }
        match ext.as_str() {
            "vst3" => {
                if vst3_file_ok(&path) {
                    out.push(("vst3".into(), path));
                }
            }
            "clap" => out.push(("clap".into(), path)),
            _ => {}
        }
    }
}

/// 规范化路径：不存在或解析失败时原样返回（用于宽松比对）。
fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// 装机后校验：确认刚放进去的文件真的被扫描到了。
///
/// 平台选错（例如这个 release 只提供了别的系统的构建）时，文件会静静地躺在托管库里、目录里
/// 却永远没有它。与其让用户对着「已安装但没有效果」发呆，不如明确报错。
fn verify_installed_loadable(
    installed: &[String],
    catalog: &[CatalogPlugin],
) -> Result<(), String> {
    if installed.is_empty() {
        return Ok(());
    }
    let scanned: Vec<PathBuf> = catalog.iter().map(|e| canon(&e.path)).collect();
    let loadable = installed.iter().any(|target| {
        let t = canon(Path::new(target));
        scanned
            .iter()
            .any(|s| s == &t || s.starts_with(&t) || t.starts_with(s))
    });
    if loadable {
        Ok(())
    } else {
        Err("插件已写入磁盘，但当前系统无法加载：这个 release 可能没有适配本平台的构建".into())
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777));
}

/// 保证插件二进制可执行：`.clap` 文件与 `.vst3` bundle 内的文件都设成 0755。
#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            make_executable(&entry.path());
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == "Contents")
            .unwrap_or(false)
        {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
        }
    } else {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}

// ---------------------------------------------------------------------------
// 文件操作
// ---------------------------------------------------------------------------

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// 移动文件 / 目录；跨设备时退回「复制 + 删除」。
fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_recursive(from, to)?;
    remove_path(from);
    Ok(())
}

fn copy_recursive(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_dir() {
        fs::create_dir_all(to).map_err(|e| e.to_string())?;
        for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name();
            copy_recursive(&entry.path(), &to.join(name))?;
        }
        Ok(())
    } else {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::copy(from, to).map_err(|e| e.to_string())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 清单（manifest.json）
// ---------------------------------------------------------------------------

fn read_manifest(dirs: &ManagedDirs) -> HashMap<String, InstallRecord> {
    fs::read_to_string(dirs.manifest())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_manifest(dirs: &ManagedDirs, map: &HashMap<String, InstallRecord>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
    let tmp = dirs.root.join("manifest.json.tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dirs.manifest()).map_err(|e| e.to_string())
}

/// 去掉文件已经不在磁盘上的记录（用户手动删过等），必要时回写。
fn prune_manifest(dirs: &ManagedDirs, map: &mut HashMap<String, InstallRecord>) -> bool {
    let before = map.len();
    map.retain(|_, rec| rec.files.iter().any(|f| Path::new(f).exists()));
    if map.len() != before {
        let _ = write_manifest(dirs, map);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// 安装 / 导入 / 删除 / 打开
// ---------------------------------------------------------------------------

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 从注册表条目安装（也用于**更新**：先清掉旧文件，再装新的）。
fn install_from_registry(app: &AppHandle, req: &InstallRequest) -> Result<(), String> {
    if req.kind_is_installer() {
        return Err("这是安装器类型，请在浏览器里打开下载页手动安装".into());
    }
    let dirs = ManagedDirs::resolve(app)?;
    dirs.ensure()?;

    let token = if req.sha256.is_empty() {
        sanitize(&req.slug)
    } else {
        sanitize(&req.sha256)
    };
    let download = dirs.downloads.join(format!("{token}.download"));
    let staging = dirs.staging.join(&token);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let result = (|| -> Result<Vec<String>, String> {
        // 下载地址按配置重写（GitHub → 代理前缀），并且**把原始地址留作后备**；
        // **扩展名判定仍用原始地址**，因为归档类型（zip / tar.gz / 裸文件）由原始 URL 决定。
        let network = app.state::<StoreRuntime>().network();
        let candidates = download_candidates(&req.url, &network.download_prefix);
        let source_count = candidates.len();
        let resolved = resolve_proxy(&network);
        eprintln!("[manager] 下载 {}：{}", req.name, resolved.source);
        let mut progress = |received: u64, total: u64| {
            emit_progress(app, &req.slug, "downloading", received, total, None);
        };
        download_verified(
            &candidates,
            &download,
            &req.sha256,
            req.size,
            resolved.proxy.as_ref(),
            None,
            &mut progress,
        )
        .map_err(|e| {
            format!(
                "下载失败（{source_count} 个源 × 最多 {DOWNLOAD_ATTEMPTS} 次都试过了，{}）：\n{e}",
                resolved.source
            )
        })?;

        emit_progress(app, &req.slug, "extracting", 0, 0, None);
        extract_archive(&download, &staging, &req.url)?;
        let found = collect_plugin_paths(&staging);
        if found.is_empty() {
            return Err("归档里没有找到能在当前系统加载的 VST3 / CLAP / AU 插件（可能只提供了其它平台的构建）".into());
        }

        emit_progress(app, &req.slug, "installing", 0, 0, None);
        let mut manifest = read_manifest(&dirs);
        // 更新语义：先删掉这个键之前的文件，避免旧版本残留。
        if let Some(old) = manifest.get(&req.slug) {
            for file in &old.files {
                remove_path(Path::new(file));
            }
        }

        let mut installed = Vec::new();
        for (kind, path) in found {
            let target_dir = dirs.dir_for(&kind);
            fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;
            let name = path.file_name().ok_or("插件文件没有文件名")?.to_owned();
            let target = target_dir.join(name);
            remove_path(&target);
            move_path(&path, &target)?;
            make_executable(&target);
            installed.push(target.to_string_lossy().into_owned());
        }

        manifest.insert(
            req.slug.clone(),
            InstallRecord {
                key: req.slug.clone(),
                slug: Some(req.slug.clone()),
                name: req.name.clone(),
                version: req.version.clone(),
                source: "registry".into(),
                files: installed.clone(),
                installed_at: now_secs(),
            },
        );
        write_manifest(&dirs, &manifest)?;
        Ok(installed)
    })();

    let _ = fs::remove_file(&download);
    let _ = fs::remove_dir_all(&staging);
    let installed = result?;
    // 装机后校验：确认刚放进去的文件真的进了目录，装不上就明确报错（见 [verify_installed_loadable]）。
    let catalog = rescan_with_managed(app);
    verify_installed_loadable(&installed, &catalog)?;
    Ok(())
}

/// 导入本地插件（`.vst3` bundle / 文件、`.clap` 文件，或 macOS 的 `.component`）到托管库。
fn import_local(app: &AppHandle, path: &str) -> Result<(), String> {
    let dirs = ManagedDirs::resolve(app)?;
    dirs.ensure()?;

    let source = PathBuf::from(path);
    if !source.exists() {
        return Err(format!("找不到插件：{path}"));
    }
    let ext = source
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let kind = match ext.as_str() {
        "vst3" => "vst3",
        "clap" => "clap",
        // AU 只能装到 macOS 上系统会扫描的用户 Components 目录（见 `au_managed_dir`）。
        #[cfg(target_os = "macos")]
        "component" => "au",
        _ => return Err("只支持导入 .vst3 / .clap（macOS 上还有 .component）".into()),
    };
    let name = source
        .file_name()
        .ok_or("插件没有文件名")?
        .to_string_lossy()
        .into_owned();

    let target = dirs.dir_for(kind).join(&name);
    if target == source {
        return Err("这个插件已经在托管库里了".into());
    }
    remove_path(&target);
    copy_recursive(&source, &target)?;
    make_executable(&target);

    let key = format!("local:{}", name);
    let mut manifest = read_manifest(&dirs);
    manifest.insert(
        key.clone(),
        InstallRecord {
            key,
            slug: None,
            name: name.clone(),
            version: String::new(),
            source: "local".into(),
            files: vec![target.to_string_lossy().into_owned()],
            installed_at: now_secs(),
        },
    );
    write_manifest(&dirs, &manifest)?;
    rescan_with_managed(app);
    Ok(())
}

/// 删除一条托管记录（只删本管理器装的东西）。
fn remove_installed(app: &AppHandle, key: &str) -> Result<(), String> {
    let dirs = ManagedDirs::resolve(app)?;
    let mut manifest = read_manifest(&dirs);
    let Some(record) = manifest.remove(key) else {
        return Err("没有这条安装记录".into());
    };
    for file in &record.files {
        remove_path(Path::new(file));
    }
    write_manifest(&dirs, &manifest)?;
    rescan_with_managed(app);
    Ok(())
}

/// 在系统浏览器里打开一个 URL（**仅 http/https**，用于安装器类型的下载页）。
fn open_external(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只允许打开 http/https 链接".into());
    }
    #[cfg(target_os = "macos")]
    let (cmd, args): (&str, Vec<&str>) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (cmd, args): (&str, Vec<&str>) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (cmd, args): (&str, Vec<&str>) = ("xdg-open", vec![url]);

    std::process::Command::new(cmd)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开浏览器失败：{e}"))
}

impl InstallRequest {
    /// 这个请求是否是「安装器」类型——前端会先过滤，但后端再挡一道。
    fn kind_is_installer(&self) -> bool {
        // 只按 URL 扩展名判断：安装器扩展名一律拒绝自动安装。
        matches!(
            url_extension(&self.url).as_str(),
            "dmg" | "pkg" | "exe" | "msi" | "deb" | "rpm" | "appimage" | "flatpak"
        )
    }
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

/// 注册表列表（带缓存；`force` 强制联网刷新）。
#[tauri::command]
pub async fn plugin_manager_registry(app: AppHandle, force: bool) -> Result<RegistryIndex, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache_dir = registry_cache_dir(&app)?;
        // 源 = 配置的镜像（按顺序）+ 官方源兜底（环境变量优先级最高）。
        let network = app.state::<StoreRuntime>().network();
        let sources = registry_sources(&network);
        let resolved = resolve_proxy(&network);
        let (raw, from_cache) =
            load_registry(&cache_dir, force, &sources, resolved.proxy.as_ref())?;
        let plugins = normalize(raw, platform(), arch());
        Ok(RegistryIndex {
            plugins,
            platform: platform().to_string(),
            arch: arch().to_string(),
            fetched_at: now_secs(),
            from_cache,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 本管理器装过的插件（会清掉磁盘上已不存在的记录）。
#[tauri::command]
pub fn plugin_manager_installed(app: AppHandle) -> Result<Vec<InstallRecord>, String> {
    let dirs = ManagedDirs::resolve(&app)?;
    let mut manifest = read_manifest(&dirs);
    prune_manifest(&dirs, &mut manifest);
    let mut records: Vec<InstallRecord> = manifest.into_values().collect();
    records.sort_by_key(|r| r.name.to_lowercase());
    Ok(records)
}

/// 从注册表下载并安装（也是更新）。
#[tauri::command]
pub async fn plugin_manager_install(app: AppHandle, req: InstallRequest) -> Result<(), String> {
    let slug = req.slug.clone();
    let app_for_job = app.clone();
    let outcome =
        tauri::async_runtime::spawn_blocking(move || install_from_registry(&app_for_job, &req))
            .await
            .map_err(|e| e.to_string())?;
    // 失败时广播一次 `error`，前端好把那条进度条收掉并弹出原因。
    if let Err(message) = &outcome {
        emit_progress(&app, &slug, "error", 0, 0, Some(message.clone()));
    }
    outcome
}

/// 按当前网络配置把注册表里的原始地址解析成**实际会请求的地址**。
///
/// 界面上的「复制下载链接」用它——保证复制到的和真去下的是同一个 URL，
/// 而不是在前端重写一遍（两处规则迟早会不一致）。
#[tauri::command]
pub fn plugin_manager_resolve_url(runtime: tauri::State<'_, StoreRuntime>, url: String) -> String {
    rewrite_download_url(&url, &runtime.network().download_prefix)
}

/// 导入本地 `.vst3` / `.clap`（macOS 上还有 `.component`）。
#[tauri::command]
pub async fn plugin_manager_import(app: AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || import_local(&app, &path))
        .await
        .map_err(|e| e.to_string())?
}

/// 删除一条托管安装。
#[tauri::command]
pub fn plugin_manager_remove(app: AppHandle, key: String) -> Result<(), String> {
    remove_installed(&app, &key)
}

/// 在浏览器里打开下载页（安装器类型用）。
#[tauri::command]
pub fn plugin_manager_open(url: String) -> Result<(), String> {
    open_external(&url)
}

/// 用户**自己装**的插件：扫描到、但不在托管库里的那些。
///
/// 它们由系统标准目录 / 其他安装器管理，本管理器**只读**（不提供删除）。
#[tauri::command]
pub fn plugin_manager_foreign(app: AppHandle) -> Result<Vec<ForeignPlugin>, String> {
    let dirs = ManagedDirs::resolve(&app)?;
    // VST3 / CLAP 的托管目录是应用数据目录，位置即身份；AU 装在**共享的**用户 Components
    // 目录里，只能靠 manifest 记录区分「管理器装的」与「用户自己装的」。
    let installed: HashSet<PathBuf> = read_manifest(&dirs)
        .into_values()
        .flat_map(|record| record.files)
        .map(PathBuf::from)
        .collect();
    let catalog = app.state::<AppState>().catalog_snapshot();
    let mut out: Vec<ForeignPlugin> = catalog
        .into_iter()
        .filter(|p| !path_is_managed(&p.path, &dirs, &installed))
        .map(|p| ForeignPlugin {
            id: p.id,
            name: p.name,
            vendor: p.vendor,
            kind: p.kind,
            format: p.format,
            path: p.path.to_string_lossy().into_owned(),
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    Ok(out)
}

/// 该路径是不是「本管理器装在托管库里」的。
///
/// - VST3 / CLAP：落在应用数据目录的托管目录下即算（位置即身份）；
/// - AU：托管目录是共享的用户 Components 目录，所以另看 manifest 记录——
///   用户自己装在同一个目录里的 AU 仍算「外部（只读）」。
fn path_is_managed(path: &Path, dirs: &ManagedDirs, installed: &HashSet<PathBuf>) -> bool {
    path.starts_with(&dirs.vst3) || path.starts_with(&dirs.clap) || installed.contains(path)
}

/// 读取插件源的网络配置（镜像 / 下载代理）。
#[tauri::command]
pub fn plugin_manager_network(runtime: tauri::State<'_, StoreRuntime>) -> StoreNetwork {
    runtime.network()
}

/// 写入插件源的网络配置（落 `store.json`）。空白项会被丢弃。
#[tauri::command]
pub fn plugin_manager_set_network(
    runtime: tauri::State<'_, StoreRuntime>,
    registry_mirrors: Vec<String>,
    download_prefix: String,
    proxy_mode: ProxyMode,
    proxy_url: String,
    model_mirrors: Option<Vec<String>>,
    model_source_preference: Option<ModelSourcePreference>,
) -> Result<StoreNetwork, String> {
    let clean = |items: Vec<String>| -> Vec<String> {
        items
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let next = StoreNetwork {
        registry_mirrors: clean(registry_mirrors),
        download_prefix: download_prefix.trim().to_string(),
        proxy_mode,
        proxy_url: proxy_url.trim().to_string(),
        // 老前端不带这两个参数时就保留原值（追加字段不破坏既有调用）。
        model_mirrors: model_mirrors
            .map(clean)
            .unwrap_or_else(|| runtime.network().model_mirrors),
        model_source_preference: model_source_preference
            .unwrap_or_else(|| runtime.network().model_source_preference),
        // 执行后端偏好由 `backend_set_preference` 单独管，这里原样保留。
        backend_preference: runtime.network().backend_preference,
    };
    runtime.set(next.clone())?;
    Ok(next)
}

/// 只做解析、**不联网**，用来在设置页显示「当前会走哪个代理」。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyStatus {
    /// 解析出来的代理地址（不含账号密码）；`None` = 直连。
    pub proxy: Option<String>,
    /// 来源说明。
    pub source: String,
}

/// 当前配置会解析成什么代理（不发请求，设置页打开时就能显示）。
#[tauri::command]
pub fn plugin_manager_proxy_status(runtime: tauri::State<'_, StoreRuntime>) -> ProxyStatus {
    let resolved = resolve_proxy(&runtime.network());
    ProxyStatus {
        proxy: proxy_display(&resolved),
        source: resolved.source,
    }
}

/// 代理地址的可读形式（`http://127.0.0.1:7892`），**不带账号密码**。
fn proxy_display(resolved: &ResolvedProxy) -> Option<String> {
    resolved
        .proxy
        .as_ref()
        .map(|p| format!("{:?}://{}:{}", p.protocol(), p.host(), p.port()).to_lowercase())
}

/// 一次代理探测的结果（给设置页的「测试」按钮）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxyProbe {
    /// 解析出来的代理地址（不含账号密码）；`None` = 直连。
    pub proxy: Option<String>,
    /// 解析来源的一句话说明。
    pub source: String,
    /// 是否真的连上了。
    pub ok: bool,
    /// 成功时是耗时；失败时是原因。
    pub detail: String,
}

/// 用**给定**的代理设置实际发一次请求，验证它到底通不通。
///
/// 刻意接受参数而不是读已保存的配置：用户刚把模式改到「手动」、填上地址，点「测试」
/// 期待的是**测这个地址**，而不是测上一次保存的东西——同时也不该为了测试就悄悄写盘。
///
/// 这也是回答「我明明开了代理，为什么还是下不动」的唯一靠谱办法：它把
/// 「解析到了哪个代理」与「这个代理能不能用」分开报——解析不到说明模式/地址不对，
/// 解析到却连不上说明代理本身没在跑（代理软件退出后经常把系统代理设置留在原地）。
#[tauri::command]
pub async fn plugin_manager_probe_proxy(
    runtime: tauri::State<'_, StoreRuntime>,
    proxy_mode: ProxyMode,
    proxy_url: String,
) -> Result<ProxyProbe, String> {
    let mut config = runtime.network();
    config.proxy_mode = proxy_mode;
    config.proxy_url = proxy_url.trim().to_string();
    let resolved = resolve_proxy(&config);
    let display = proxy_display(&resolved);
    let source = resolved.source.clone();

    let started = Instant::now();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        fetch_registry_from(&fetch_agent(resolved.proxy.as_ref()), REGISTRY_URL)
    })
    .await
    .map_err(|e| e.to_string())?;

    let (ok, detail) = match outcome {
        Ok((_, text)) => (
            true,
            format!(
                "连接成功，{:.0} ms，收到 {} KB",
                started.elapsed().as_millis() as f64,
                text.len() / 1024
            ),
        ),
        Err(e) => (false, e),
    };
    Ok(ProxyProbe {
        proxy: display,
        source,
        ok,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_file(url: &str, contains: &[&str], systems: &[&str], archs: &[&str]) -> RawFile {
        RawFile {
            systems: systems
                .iter()
                .map(|k| RawSystem {
                    kind: k.to_string(),
                })
                .collect(),
            architectures: archs.iter().map(|a| a.to_string()).collect(),
            contains: contains.iter().map(|c| c.to_string()).collect(),
            kind: "archive".into(),
            size: 10,
            sha256: "abc".into(),
            url: url.into(),
        }
    }

    fn registry_with(files: Vec<RawFile>) -> RawRegistry {
        let mut versions = HashMap::new();
        versions.insert(
            "1.0.0".to_string(),
            RawVersion {
                name: "Test".into(),
                author: "A".into(),
                description: "d".into(),
                license: "mit".into(),
                kind: "effect".into(),
                tags: vec![],
                url: None,
                image: None,
                files,
                verified: true,
            },
        );
        let mut plugins = HashMap::new();
        plugins.insert(
            "org/test".to_string(),
            RawPackage {
                slug: "org/test".into(),
                version: "1.0.0".into(),
                versions,
            },
        );
        RawRegistry { plugins }
    }

    #[test]
    fn normalize_keeps_only_vst3_and_clap_files() {
        let raw = registry_with(vec![
            raw_file("https://x/a.zip", &["vst3"], &["mac"], &["arm64"]),
            raw_file("https://x/b.zip", &["lv2"], &["mac"], &["arm64"]),
            raw_file("https://x/c.tar.gz", &["clap", "lv2"], &["linux"], &["x64"]),
        ]);
        let out = normalize(raw, "mac", "arm64");
        assert_eq!(out.len(), 1, "只有一个插件");
        assert_eq!(out[0].files.len(), 2, "lv2 文件被过滤掉");
        assert!(out[0].files[0].compatible, "mac/arm64 匹配");
        assert!(!out[0].files[1].compatible, "linux/x64 不匹配");
    }

    #[test]
    fn normalize_drops_plugins_without_hostable_files() {
        let raw = registry_with(vec![raw_file(
            "https://x/a.zip",
            &["lv2"],
            &["mac"],
            &["arm64"],
        )]);
        assert!(normalize(raw, "mac", "arm64").is_empty());
    }

    fn registry_file(contains: &[&str], size: u64, compatible: bool, kind: &str) -> RegistryFile {
        RegistryFile {
            url: format!("https://x/{}.zip", contains.join("-")),
            sha256: "deadbeef".into(),
            size,
            kind: kind.into(),
            contains: contains.iter().map(|c| c.to_string()).collect(),
            platforms: vec!["mac".into()],
            archs: vec!["arm64".into()],
            compatible,
        }
    }

    fn registry_plugin(
        slug: &str,
        name: &str,
        author: &str,
        description: &str,
        tags: &[&str],
        kind: &str,
        files: Vec<RegistryFile>,
    ) -> RegistryPlugin {
        RegistryPlugin {
            slug: slug.into(),
            name: name.into(),
            author: author.into(),
            description: description.into(),
            license: "mit".into(),
            kind: kind.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            image: None,
            homepage: Some("https://example.test".into()),
            version: "1.0.0".into(),
            verified: true,
            files,
        }
    }

    /// 随包快照必须能被解析——它现在是客户端最后的兜底，坏了就等于没有。
    #[test]
    fn bundled_registry_is_parseable_and_non_empty() {
        let raw: RawRegistry = serde_json::from_str(BUNDLED_REGISTRY).expect("随包快照是合法 JSON");
        assert!(!raw.plugins.is_empty(), "随包快照里必须有插件");
    }

    /// 用真实快照走一遍搜索，确认打分链路对上游数据有效（不是只在小样本上能跑）。
    #[test]
    fn bundled_registry_search_finds_a_known_synth() {
        let raw: RawRegistry = serde_json::from_str(BUNDLED_REGISTRY).expect("随包快照是合法 JSON");
        let plugins = normalize(raw, platform(), arch());
        let hits = search_registry_entries(&plugins, "surge", None, 5, &HashSet::new());
        assert!(
            hits.iter()
                .any(|hit| hit.name.to_lowercase().contains("surge")),
            "搜索 surge 应当命中 Surge XT：{:?}",
            hits.iter().map(|hit| &hit.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn search_scores_name_tags_and_description() {
        let plugins = vec![
            registry_plugin(
                "tape-delay",
                "Tape Delay",
                "Acme",
                "Vintage tape echo",
                &["delay", "tape"],
                "effect",
                vec![registry_file(&["vst3"], 100, true, "archive")],
            ),
            registry_plugin(
                "string-ensemble",
                "String Ensemble",
                "Acme",
                "Warm orchestral strings",
                &["strings"],
                "instrument",
                vec![registry_file(&["vst3"], 200, true, "archive")],
            ),
        ];
        let installed: HashSet<String> = ["tape-delay".to_string()].into_iter().collect();

        let hits = search_registry_entries(&plugins, "tape delay", Some("effect"), 8, &installed);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].slug, "tape-delay");
        assert!(hits[0].installed);
        assert!(hits[0].installable);
        assert!(hits[0].why.contains("tape"), "{}", hits[0].why);

        let hits = search_registry_entries(&plugins, "warm strings", None, 8, &HashSet::new());
        assert_eq!(hits[0].slug, "string-ensemble");
    }

    /// 多词查询要求全部命中：`electric piano` 不该把 Electric Drums 带出来。
    #[test]
    fn search_requires_every_term_to_match() {
        let plugins = vec![registry_plugin(
            "electric-drums",
            "Electric Drums",
            "Acme",
            "Punchy drum machine",
            &["drums"],
            "instrument",
            vec![registry_file(&["vst3"], 100, true, "archive")],
        )];
        assert!(
            search_registry_entries(&plugins, "electric piano", None, 8, &HashSet::new())
                .is_empty()
        );
        assert_eq!(
            search_registry_entries(&plugins, "electric drums", None, 8, &HashSet::new()).len(),
            1
        );
    }

    /// 有多个归档时优先 VST3；安装器 / 无 sha256 的不参与一键安装。
    #[test]
    fn install_prefers_vst3_and_skips_installers() {
        let plugin = registry_plugin(
            "x",
            "X",
            "A",
            "d",
            &[],
            "effect",
            vec![
                registry_file(&["au"], 300, true, "archive"),
                registry_file(&["clap"], 200, true, "archive"),
                registry_file(&["vst3"], 100, true, "archive"),
                registry_file(&["vst3"], 999, true, "installer"),
            ],
        );
        let file = best_installable_file(&plugin).expect("有可安装归档");
        assert_eq!(file.contains, vec!["vst3".to_string()]);
        assert_eq!(file.size, 100, "安装器不参与自动安装");
    }

    /// AU 现在也是可托管的格式，注册表里只提供 AU 的文件也要留下。
    #[test]
    fn normalize_keeps_au_files_too() {
        let raw = registry_with(vec![raw_file(
            "https://x/a.zip",
            &["au"],
            &["mac"],
            &["arm64"],
        )]);
        let out = normalize(raw, "mac", "arm64");
        assert_eq!(out.len(), 1, "只提供 AU 的插件应当在列表里");
        assert_eq!(out[0].files[0].contains, vec!["au".to_string()]);
    }

    /// AU 装在**共享**的用户 Components 目录里：只有 manifest 记过的才算「已安装」，
    /// 用户自己放进去的仍算「外部（只读）」。
    #[test]
    fn au_in_the_shared_dir_is_managed_only_when_recorded() {
        let dirs = ManagedDirs {
            root: PathBuf::from("/tmp/root"),
            vst3: PathBuf::from("/tmp/root/vst3"),
            clap: PathBuf::from("/tmp/root/clap"),
            au: PathBuf::from("/Users/u/Library/Audio/Plug-Ins/Components"),
            downloads: PathBuf::from("/tmp/root/downloads"),
            staging: PathBuf::from("/tmp/root/staging"),
        };
        let ours = dirs.au.join("Ours.component");
        let user_installed = dirs.au.join("UserOne.component");
        let installed: HashSet<PathBuf> = [ours.clone()].into_iter().collect();

        assert!(
            path_is_managed(&ours, &dirs, &installed),
            "管理器导入的 AU 算已安装"
        );
        assert!(
            !path_is_managed(&user_installed, &dirs, &installed),
            "用户自己装在同一个目录里的 AU 仍是外部只读"
        );
        assert!(
            path_is_managed(&dirs.vst3.join("X.vst3"), &dirs, &installed),
            "vst3 仍按托管目录判定"
        );
    }

    /// macOS 上：`.component` 目录（含 `Contents/Info.plist`）应当被认出是 AU。
    #[test]
    #[cfg(target_os = "macos")]
    fn collect_finds_au_components() {
        let root = std::env::temp_dir().join(format!("ai-daw-pm-au-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // 合法的 AU bundle：含 Contents/Info.plist。
        fs::create_dir_all(root.join("Cool.component/Contents")).unwrap();
        fs::write(root.join("Cool.component/Contents/Info.plist"), b"plist").unwrap();
        // 缺 Info.plist 的目录不算插件。
        fs::create_dir_all(root.join("Junk.component/Contents")).unwrap();

        let found = collect_plugin_paths(&root);
        assert_eq!(found.len(), 1, "只应认出一个合法的 AU：{found:?}");
        assert_eq!(found[0].0, "au");
        assert!(found[0].1.to_string_lossy().contains("Cool.component"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn url_extension_handles_query_and_tar_gz() {
        assert_eq!(url_extension("https://x/Plugin.zip?download=1"), "zip");
        assert_eq!(url_extension("https://x/Plugin.tar.gz"), "tar.gz");
        assert_eq!(url_extension("https://x/Plugin.clap"), "clap");
        assert_eq!(url_extension("https://x/plain"), "");
    }

    #[test]
    fn installer_extensions_are_refused() {
        let req = InstallRequest {
            slug: "s".into(),
            name: "n".into(),
            version: "1".into(),
            url: "https://x/Plugin.dmg".into(),
            sha256: "a".into(),
            size: 1,
        };
        assert!(req.kind_is_installer());
    }

    #[test]
    fn collect_finds_bundles_and_clap_files() {
        let root = std::env::temp_dir().join(format!("ai-daw-pm-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // 一个真正的 VST3 bundle：macOS 要求含 `Contents/MacOS`，其它平台不校验。
        fs::create_dir_all(root.join("nested/Foo.vst3/Contents/MacOS")).unwrap();
        fs::write(root.join("nested/Foo.vst3/Contents/MacOS/Foo"), b"x").unwrap();
        fs::write(root.join("Bar.clap"), b"y").unwrap();
        fs::write(root.join("readme.txt"), b"z").unwrap();

        let found = collect_plugin_paths(&root);
        let kinds: Vec<&str> = found.iter().map(|(k, _)| k.as_str()).collect();
        assert!(kinds.contains(&"vst3"), "应找到 .vst3 bundle");
        assert!(kinds.contains(&"clap"), "应找到 .clap 文件");
        assert_eq!(found.len(), 2, "readme.txt 不算插件");
        let _ = fs::remove_dir_all(&root);
    }

    /// macOS 上：混装着 Windows 与 macOS 同名产物时，只应收下 macOS 的 bundle、丢弃裸 DLL。
    ///
    /// 这正是 RE201 踩的坑——同一个 zip 里 Windows 与 macOS 的 VST3 同名，裸 DLL 覆盖了 bundle。
    #[test]
    #[cfg(target_os = "macos")]
    fn collect_prefers_the_native_bundle_over_a_foreign_binary() {
        let root = std::env::temp_dir().join(format!("ai-daw-pm-native-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // macOS 的：bundle 目录，含 Contents/MacOS。
        fs::create_dir_all(root.join("mac/JE-201.vst3/Contents/MacOS")).unwrap();
        fs::write(
            root.join("mac/JE-201.vst3/Contents/MacOS/JE-201"),
            b"mach-o",
        )
        .unwrap();
        // Windows 的：同名裸文件（DLL）。
        fs::create_dir_all(root.join("win")).unwrap();
        fs::write(root.join("win/JE-201.vst3"), b"MZ").unwrap();

        let found = collect_plugin_paths(&root);
        assert_eq!(found.len(), 1, "只应收下 macOS 的 bundle");
        let (kind, path) = &found[0];
        assert_eq!(kind, "vst3");
        assert!(path.is_dir(), "收下的必须是 bundle 目录，而不是裸文件");
        assert!(
            path.to_string_lossy().contains("mac"),
            "收下的应当是 macOS 那份"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// 联网冒烟测试（默认忽略，避免 CI 无网失败）：确认 TLS provider 配好、能真的走通 HTTPS，
    /// 并打印归一化后的数量，便于排查「拉到了但列表为空」这类问题。
    ///
    /// 手动跑：
    /// `cargo test --lib manager::tests::fetches_registry_over_https -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn fetches_registry_over_https() {
        let start = std::time::Instant::now();
        let (raw, text) = fetch_registry_from(&fetch_agent(None), REGISTRY_URL)
            .expect("HTTPS 连接失败（TLS provider 或网络问题）");
        eprintln!("下载 {} 字节，用时 {:?}", text.len(), start.elapsed());

        eprintln!("原始 plugins：{}", raw.plugins.len());
        let plugins = normalize(raw, platform(), arch());
        let with_compatible = plugins
            .iter()
            .filter(|p| p.files.iter().any(|f| f.compatible))
            .count();
        eprintln!(
            "归一化后：{} 个插件，其中 {} 个有兼容本机（{} / {}）的文件",
            plugins.len(),
            with_compatible,
            platform(),
            arch()
        );
        assert!(!plugins.is_empty(), "归一化后不该为空");
    }

    #[test]
    fn download_proxy_only_rewrites_github_urls() {
        let gh = "https://github.com/x/y/releases/download/v1/P.zip";
        assert_eq!(rewrite_download_url(gh, ""), gh, "没配代理就不动");
        assert_eq!(
            rewrite_download_url(gh, "https://proxy.example/"),
            "https://proxy.example/https://github.com/x/y/releases/download/v1/P.zip",
            "无占位符 → 直接前缀"
        );
        assert_eq!(
            rewrite_download_url(gh, "https://proxy.example/fetch?u={url}"),
            "https://proxy.example/fetch?u=https://github.com/x/y/releases/download/v1/P.zip",
            "有 {{url}} → 替换"
        );
        let other = "https://ci.helio.fm/helio-3.17-x64.zip";
        assert_eq!(
            rewrite_download_url(other, "https://proxy.example/"),
            other,
            "非 GitHub 不动"
        );
    }

    /// 候选地址：配了代理就是「先代理、再原始」，没配就只有原始——代理挂了也能直连兜底。
    #[test]
    fn download_candidates_keep_the_original_as_a_fallback() {
        let gh = "https://github.com/x/y/releases/download/v1/P.zip";
        assert_eq!(download_candidates(gh, ""), vec![gh.to_string()]);
        assert_eq!(
            download_candidates(gh, "https://proxy.example/"),
            vec![
                "https://proxy.example/https://github.com/x/y/releases/download/v1/P.zip"
                    .to_string(),
                gh.to_string(),
            ],
            "代理在前，原始地址在后"
        );
        // 非 GitHub 地址即便配了代理也不该被重写，候选自然只有一个。
        let other = "https://ci.helio.fm/x.zip";
        assert_eq!(
            download_candidates(other, "https://proxy.example/"),
            vec![other.to_string()]
        );
    }

    /// macOS 的 `scutil --proxy` 输出解析（样本取自真实输出）。
    #[cfg(target_os = "macos")]
    #[test]
    fn parses_scutil_proxy_output() {
        let sample = "<dictionary> {\n  ExceptionsList : <array> {\n    0 : localhost\n  }\n  HTTPEnable : 1\n  HTTPPort : 7892\n  HTTPProxy : 127.0.0.1\n  HTTPSEnable : 1\n  HTTPSPort : 7892\n  HTTPSProxy : 127.0.0.1\n  SOCKSEnable : 0\n}\n";
        assert_eq!(scutil_value(sample, "HTTPEnable").as_deref(), Some("1"));
        assert_eq!(
            scutil_value(sample, "HTTPSProxy").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(scutil_value(sample, "HTTPSPort").as_deref(), Some("7892"));
        assert_eq!(scutil_value(sample, "SOCKSEnable").as_deref(), Some("0"));
        assert_eq!(scutil_value(sample, "不存在的键"), None);
        // `ExceptionsList` 里的数组项也是 `键 : 值` 行（键就是下标），所以按 "0" 能查到它。
        // 我们只查 `HTTPEnable` 这类命名键，撞不上；写在这里是为了别把它当 bug 再「修」一次。
        assert_eq!(scutil_value(sample, "0").as_deref(), Some("localhost"));
    }

    /// 三种模式的解析结果：直连绝不带代理，手动模式沿用给的地址。
    #[test]
    fn proxy_modes_resolve_as_expected() {
        let direct = resolve_proxy(&StoreNetwork {
            proxy_mode: ProxyMode::Direct,
            ..Default::default()
        });
        assert!(direct.proxy.is_none(), "直连模式不该产生代理");

        let manual = resolve_proxy(&StoreNetwork {
            proxy_mode: ProxyMode::Manual,
            proxy_url: "http://127.0.0.1:7892".into(),
            ..Default::default()
        });
        let proxy = manual.proxy.expect("手动模式应当解析出代理");
        assert_eq!(proxy.host(), "127.0.0.1");
        assert_eq!(proxy.port(), 7892);

        // 地址写错时不该炸，退回直连并说明原因。
        let broken = resolve_proxy(&StoreNetwork {
            proxy_mode: ProxyMode::Manual,
            proxy_url: "这不是地址".into(),
            ..Default::default()
        });
        assert!(broken.proxy.is_none());
        assert!(
            broken.source.contains("无效"),
            "应当说明原因：{}",
            broken.source
        );
    }

    /// 手动跑（需要本机真的开着代理）：
    /// `cargo test --lib manager::tests::follows_the_system_proxy -- --ignored --nocapture`
    ///
    /// 这是「明明开了代理，软件却直连」这个问题的端到端验证：它会打印解析到哪个代理，
    /// 然后**真的经它**去拉一次注册表。
    #[test]
    #[ignore]
    fn follows_the_system_proxy() {
        let config = StoreNetwork {
            proxy_mode: ProxyMode::System,
            ..Default::default()
        };
        let resolved = resolve_proxy(&config);
        eprintln!("解析结果：{}", resolved.source);
        assert!(
            resolved.proxy.is_some(),
            "没解析到系统代理——本机没开代理，或探测方式不对"
        );
        let start = std::time::Instant::now();
        let (raw, text) = fetch_registry_from(&fetch_agent(resolved.proxy.as_ref()), REGISTRY_URL)
            .expect(
            "经系统代理拉注册表失败——多半是代理软件没在跑（`scutil --proxy` 里那套设置会留在原地）",
        );
        eprintln!(
            "经代理拉取 {} 字节，用时 {:?}，plugins {}",
            text.len(),
            start.elapsed(),
            raw.plugins.len()
        );
        assert!(!raw.plugins.is_empty());
    }

    #[test]
    fn registry_sources_put_official_last_and_env_first() {
        let config = StoreNetwork {
            registry_mirrors: vec!["https://mirror.example/index.json".into()],
            download_prefix: String::new(),
            ..Default::default()
        };
        let sources = registry_sources(&config);
        assert_eq!(
            sources.first().unwrap(),
            "https://mirror.example/index.json"
        );
        assert_eq!(sources.last().unwrap(), REGISTRY_URL, "官方源兜底");
    }
}
