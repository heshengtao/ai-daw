//! **数据目录**：默认在系统给的 `%APPDATA%\com.aidaw.app`，但可以整体搬到别的盘。
//!
//! 为什么需要这一层：模型 / 运行时 / 插件 / AI 产物动辄十几 GB，系统盘很快被吃满。
//! 这里把「数据放哪」从 Tauri 的固定路径里抽出来，解析顺序是：
//!
//! 1. 命令行 `--data-dir <path>`（便携模式 / 自动化）
//! 2. 环境变量 `AIDAW_DATA_DIR`
//! 3. 设置页写的指针文件 `<默认配置目录>/data-dir.json`（`path` 字段）
//! 4. 安装器写的注册表 `HKCU\Software\AI DAW\DataDir`
//! 5. 默认目录（Tauri 的 `app_data_dir` / `app_config_dir` / `app_cache_dir`）
//!
//! 指针文件必须留在**默认**配置目录里：它是启动时唯一能稳定找到的锚点（数据目录本身
//! 可能已经搬走）。安装器与设置页都写这里，二者不冲突——设置页写的优先级更高。
//!
//! 迁移是「复制 → 写指针 → 提示重启」，不在进程运行中删除旧文件：插件 DLL、正在写的
//! 日志、会话缓存都还开着，删了只会留下一个半坏的应用。重启后设置页再提供「清理旧目录」。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 指针文件名（放在默认配置目录）。
const POINTER: &str = "data-dir.json";
/// Windows 安装器写数据目录用的注册表键。
#[cfg(windows)]
const REG_PATH: &str = r"Software\AI DAW";
/// 数据目录迁移进度事件。
pub const EVENT_PROGRESS: &str = "app://data-dir";

/// 命令行显式指定的目录（进程内只解析一次）。
static CLI_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();
/// **本进程实际生效**的数据目录（启动时解析一次后冻结）。
///
/// 迁移只写指针、不改变这里的值：运行中的进程继续用旧目录，重启后新进程才按指针切到
/// 新目录。否则迁移完的一瞬间「日志还写在旧目录、模型却去新目录找」，状态会撕成两半。
static EFFECTIVE: OnceLock<Option<(PathBuf, &'static str)>> = OnceLock::new();

/// 迁移 / 清理进度。
#[derive(Debug, Clone, Serialize)]
pub struct DataDirProgress {
    /// `scan` / `copy` / `write` / `done`。
    pub phase: &'static str,
    /// 已处理字节。
    pub done: u64,
    /// 总字节（`scan` 阶段前为 0，界面走不确定态）。
    pub total: u64,
    /// 已复制文件数。
    pub files: u64,
    /// 补充说明（失败原因等）。
    pub message: Option<String>,
}

/// 当前数据目录概况（设置页读）。
#[derive(Debug, Clone, Serialize)]
pub struct DataDirInfo {
    /// 当前生效的数据目录（绝对路径）。
    pub current: String,
    /// 系统默认数据目录（用于「恢复默认」与清理旧目录）。
    pub default: String,
    /// 当前缓存目录。
    pub cache: String,
    /// 来源：`cli` / `env` / `settings` / `installer` / `default`。
    pub source: String,
    /// 数据目录已占用字节数。
    pub size_bytes: u64,
    /// 是否由环境变量 / 命令行指定（便携模式，设置页改不了）。
    pub portable: bool,
    /// 上一次迁移留下的旧目录（重启后可一键清理）。
    pub previous: Option<String>,
    /// 指针里的新目录（与 `current` 不同 = 已迁移、等重启）。
    pub next: Option<String>,
    /// 是否需要重启才能切到 `next`。
    pub restart_required: bool,
}

/// 指针文件内容。也兼容纯文本（一行路径）——手写或旧版本都能读。
#[derive(Debug, Default, Serialize, Deserialize)]
struct Pointer {
    path: String,
    #[serde(default)]
    previous: Option<String>,
    /// 历史数据根目录（用于给**数据目录之外**的工程文件做旧路径重映射）。
    #[serde(default)]
    legacy: Vec<String>,
}

fn non_empty(path: PathBuf) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    if text.trim().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// 解析命令行 `--data-dir <path>` / `--data-dir=<path>`。启动最早期调用一次。
pub fn init_from_args() {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = if let Some(rest) = arg.strip_prefix("--data-dir=") {
            Some(rest.to_string())
        } else if arg == "--data-dir" {
            args.next()
        } else {
            None
        };
        if let Some(value) = value.and_then(|v| non_empty(PathBuf::from(v))) {
            let _ = CLI_OVERRIDE.set(value);
            return;
        }
    }
}

fn cli_override() -> Option<PathBuf> {
    CLI_OVERRIDE.get().cloned()
}

fn env_override() -> Option<PathBuf> {
    std::env::var_os("AIDAW_DATA_DIR").and_then(|v| non_empty(PathBuf::from(v)))
}

fn default_data(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

fn default_config(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok()
}

/// **系统默认**配置目录（不受数据目录覆盖影响）。
///
/// 给「供外部工具按 bundle id 查找」的发现文件用（`cli-path` / `mcp-url`）：这些文件必须
/// 留在别人推导得到的位置，不能跟着数据目录搬走。它们都只有几十字节。
pub fn default_config_dir(app: &AppHandle) -> Option<PathBuf> {
    default_config(app)
}

fn default_cache(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok()
}

fn read_pointer_from(dir: &Path) -> Option<Pointer> {
    let text = fs::read_to_string(dir.join(POINTER)).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(pointer) = serde_json::from_str::<Pointer>(trimmed) {
        return non_empty(PathBuf::from(pointer.path)).map(|path| Pointer {
            path: path.to_string_lossy().into_owned(),
            previous: pointer.previous,
            legacy: pointer.legacy,
        });
    }
    // 看起来是 JSON 却解析失败：别把整段 JSON 当成路径。
    if trimmed.starts_with('{') {
        return None;
    }
    // 老式 / 手写：文件里只有一行路径。
    non_empty(PathBuf::from(trimmed)).map(|path| Pointer {
        path: path.to_string_lossy().into_owned(),
        previous: None,
        legacy: Vec::new(),
    })
}

fn read_pointer(app: &AppHandle) -> Option<Pointer> {
    // 只从**默认**配置目录读：数据目录搬走之后，这里仍是稳定的锚点。
    default_config(app).and_then(|dir| read_pointer_from(&dir))
}

#[cfg(windows)]
fn registry_data_dir() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(REG_PATH).ok()?;
    let value: String = key.get_value("DataDir").ok()?;
    non_empty(PathBuf::from(value))
}

#[cfg(not(windows))]
fn registry_data_dir() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn write_registry(path: &Path) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(REG_PATH)
        .map_err(|e| format!("写注册表失败：{e}"))?;
    key.set_value("DataDir", &path.to_string_lossy().into_owned())
        .map_err(|e| format!("写注册表失败：{e}"))
}

#[cfg(not(windows))]
fn write_registry(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// 解析覆盖目录（按优先级）。只在进程第一次用到路径时跑一次。
fn resolve_override(app: &AppHandle) -> Option<(PathBuf, &'static str)> {
    if let Some(path) = cli_override() {
        return Some((path, "cli"));
    }
    if let Some(path) = env_override() {
        return Some((path, "env"));
    }
    if let Some(pointer) = read_pointer(app) {
        return Some((PathBuf::from(pointer.path), "settings"));
    }
    registry_data_dir().map(|path| (path, "installer"))
}

/// 本进程实际生效的目录（冻结）。
fn effective(app: &AppHandle) -> Option<&'static (PathBuf, &'static str)> {
    EFFECTIVE.get_or_init(|| resolve_override(app)).as_ref()
}

/// 旧数据目录 → 当前数据目录的前缀映射（重启后生效）。
///
/// 用户自己的工程文件（不在数据目录里）里存的音频路径仍是旧目录的绝对路径。迁移后
/// 不能要求用户逐个重新链接素材，所以在素材加载时做一次透明重映射：只有「旧前缀 +
/// 新目录里确实存在这个文件」才改写，其它路径原样返回。
static LEGACY_REMAP: OnceLock<Vec<(PathBuf, PathBuf)>> = OnceLock::new();

/// 启动时解析一次旧前缀（`legacy` + 当前目录）。必须在 `EFFECTIVE` 冻结之后调用。
pub fn init_remap(app: &AppHandle) {
    LEGACY_REMAP.get_or_init(|| {
        let Some(pointer) = read_pointer(app) else {
            return Vec::new();
        };
        let Ok(current) = data_dir(app) else {
            return Vec::new();
        };
        let mut roots = pointer.legacy;
        if let Some(previous) = pointer.previous {
            roots.push(previous);
        }
        let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
        for root in roots {
            let old = PathBuf::from(root);
            if same_path(&old, &current) || out.iter().any(|(p, _)| same_path(p, &old)) {
                continue;
            }
            // 目录被删掉也留着映射：文件可能后来才被恢复，映射本身不花什么钱。
            out.push((old, current.clone()));
        }
        out
    });
}

/// 旧目录里的素材路径 → 新目录里的同一文件（不存在时返回 `None`）。
pub fn remap_legacy(path: &str) -> Option<PathBuf> {
    let maps = LEGACY_REMAP.get()?;
    let p = Path::new(path);
    for (from, to) in maps {
        if let Ok(rest) = p.strip_prefix(from) {
            let candidate = to.join(rest);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// 当前生效的数据目录。
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some((root, _)) = effective(app) {
        return Ok(root.clone());
    }
    default_data(app).ok_or_else(|| "拿不到应用数据目录".to_string())
}

/// 当前生效的配置目录（覆盖时与数据目录同根，保证模型 / 运行时 / 日志一起搬走）。
pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some((root, _)) = effective(app) {
        return Ok(root.clone());
    }
    default_config(app).ok_or_else(|| "拿不到应用配置目录".to_string())
}

/// 当前生效的缓存目录（覆盖时挂在数据目录下的 `cache/`）。
pub fn cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some((root, _)) = effective(app) {
        return Ok(root.join("cache"));
    }
    default_cache(app).ok_or_else(|| "拿不到应用缓存目录".to_string())
}

/// 递归统计目录大小（读不到的条目按 0 算）。
pub fn dir_size(path: &Path) -> u64 {
    dir_size_excluding(path, &[])
}

/// 同 [`dir_size`]，但跳过顶层指定名字的条目（迁移时不算日志 / 缓存）。
fn dir_size_excluding(path: &Path, skip: &[&str]) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if skip.iter().any(|s| *s == name) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            total += dir_size(&entry.path());
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// 当前数据目录概况（文件大小统计在后台线程做）。
pub fn info(app: &AppHandle) -> Result<DataDirInfo, String> {
    let current = data_dir(app)?;
    let default = default_data(app).ok_or_else(|| "拿不到默认数据目录".to_string())?;
    let source = effective(app).map(|(_, s)| *s).unwrap_or("default");
    let pointer = read_pointer(app);
    let previous = pointer.as_ref().and_then(|p| p.previous.clone());
    // 指针里的路径和本进程实际用的不同 = 上次迁移完还没重启。
    let next = pointer
        .as_ref()
        .map(|p| PathBuf::from(&p.path))
        .filter(|path| !same_path(path, &current));
    let restart_required = next.is_some();
    Ok(DataDirInfo {
        current: current.to_string_lossy().into_owned(),
        default: default.to_string_lossy().into_owned(),
        cache: cache_dir(app)?.to_string_lossy().into_owned(),
        source: source.to_string(),
        size_bytes: dir_size(&current),
        portable: matches!(source, "cli" | "env"),
        previous,
        next: next.map(|path| path.to_string_lossy().into_owned()),
        restart_required,
    })
}

fn lower(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    lower(a) == lower(b)
}

/// 去掉 Windows `canonicalize` 加上的 `\\?\` 前缀。
///
/// 这个前缀 Rust 自己能处理，但写进指针后会传给引擎子进程 / 插件宿主，别的程序未必认；
/// 界面上显示出来也很难看。只用于**存储和显示**，路径比较仍用 canonical 形式。
fn simplified(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

/// `child` 是否在 `parent` 之内（含相等）。
fn inside(child: &Path, parent: &Path) -> bool {
    let child = lower(child);
    let parent = lower(parent).trim_end_matches(['/', '\\']).to_string();
    child == parent
        || child
            .strip_prefix(&parent)
            .map(|rest| rest.starts_with('/') || rest.starts_with('\\'))
            .unwrap_or(false)
}

/// 复制进度（跨多个源目录累加）。
struct CopyProgress<'a> {
    app: &'a AppHandle,
    done: u64,
    total: u64,
    files: u64,
    last: Instant,
}

impl CopyProgress<'_> {
    fn file_done(&mut self, bytes: u64) {
        self.done += bytes;
        self.files += 1;
        if self.last.elapsed() >= Duration::from_millis(120) {
            self.last = Instant::now();
            emit(self.app, "copy", self.done, self.total, self.files, None);
        }
    }
}

fn copy_tree(
    src: &Path,
    dst: &Path,
    skip: &[&str],
    progress: &mut CopyProgress<'_>,
) -> Result<(), String> {
    let entries = fs::read_dir(src).map_err(|e| format!("读 {} 失败：{e}", src.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if skip.iter().any(|s| *s == name) {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // 符号链接 / 目录联接一律跳过：迁移不是备份，跟着链接跑可能绕回自己。
        if kind.is_symlink() {
            continue;
        }
        let target = dst.join(&name);
        if kind.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("建目录 {} 失败：{e}", target.display()))?;
            copy_tree(&path, &target, &[], progress)?;
        } else if let Ok(meta) = entry.metadata() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("建目录失败：{e}"))?;
            }
            // 正在写的日志 / 被占用的缓存可能复制失败：跳过它们，不值得让整次迁移失败。
            if fs::copy(&path, &target).is_ok() {
                progress.file_done(meta.len());
            }
        }
    }
    Ok(())
}

/// 把复制到新目录里 JSON 状态文件中的旧绝对路径改成新路径。
///
/// 工程的音频引用、插件 manifest、catalog 缓存都存的是**绝对路径**；只复制文件不
/// 改这些字符串，迁移后工程会「找不到素材」、插件管理器会认为插件没装。这里统一处理
/// 目标目录下的 `.json`：JSON 文本里的反斜杠是 `\\`，所以两种写法都替换。
fn rebase_json_paths(root: &Path, from: &Path, to: &Path) -> Result<u64, String> {
    let from_plain = from
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string();
    let to_plain = to
        .to_string_lossy()
        .trim_end_matches(['/', '\\'])
        .to_string();
    if from_plain.is_empty() || to_plain.is_empty() || same_path(from, to) {
        return Ok(0);
    }
    let from_json = from_plain.replace('\\', "\\\\");
    let to_json = to_plain.replace('\\', "\\\\");
    let mut changed = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            // 工程快照一般 < 1 MB；超过 32 MB 的 JSON 不碰，避免误伤大文件。
            if meta.len() > 32 * 1024 * 1024 {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if !text.contains(&from_json) && !text.contains(&from_plain) {
                continue;
            }
            let next = text
                .replace(&from_json, &to_json)
                .replace(&from_plain, &to_plain);
            if next != text && fs::write(&path, next).is_ok() {
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// 扫描要复制的源目录（去重：Windows 上 data / config 经常是同一个目录）。
fn sources(app: &AppHandle) -> Result<Vec<(PathBuf, Vec<&'static str>)>, String> {
    let data = data_dir(app)?;
    let config = config_dir(app)?;
    let mut out = Vec::new();
    // 先配置、后数据：万一重名，数据目录（模型/插件）优先。
    if !same_path(&config, &data) {
        out.push((config, vec![POINTER]));
    }
    // 日志可重建、且可能正被本进程打开；缓存同理（会自己再生成）。
    out.push((data, vec![POINTER, "logs", "cache"]));
    Ok(out)
}

/// 把当前数据目录整体迁到 `target`（复制 + 写指针；旧目录留着重启后清理）。
pub fn migrate(app: &AppHandle, target: String) -> Result<DataDirInfo, String> {
    let target = PathBuf::from(target.trim());
    if target.as_os_str().is_empty() {
        return Err("请选择一个数据目录".into());
    }
    if !target.is_absolute() {
        return Err("数据目录必须是绝对路径".into());
    }
    let source = data_dir(app)?;
    fs::create_dir_all(&target).map_err(|e| format!("建目标目录失败：{e}"))?;
    // canonicalize 之后比较，避免 `D:\a\..\a` 这类写法绕过检查；比较用 canonical，
    // 落地（写指针 / 复制）用去掉 `\\?\` 的普通绝对路径。
    let target_canon = fs::canonicalize(&target).map_err(|e| format!("目标目录不可用：{e}"))?;
    let source_canon = fs::canonicalize(&source).unwrap_or_else(|_| source.clone());
    if same_path(&source_canon, &target_canon) {
        return Err("目标目录就是当前数据目录".into());
    }
    if inside(&target_canon, &source_canon) || inside(&source_canon, &target_canon) {
        return Err("目标目录不能是当前数据目录的父目录或子目录".into());
    }
    let target = simplified(target_canon);
    let mut existing = fs::read_dir(&target)
        .map_err(|e| format!("读目标目录失败：{e}"))?
        .flatten();
    if existing.next().is_some() {
        return Err("目标目录不是空的：请新建一个专用文件夹（避免覆盖已有文件）".into());
    }

    emit(app, "scan", 0, 0, 0, None);
    let roots = sources(app)?;
    let total: u64 = roots
        .iter()
        .map(|(path, skip)| dir_size_excluding(path, skip))
        .sum();
    emit(app, "copy", 0, total, 0, None);

    let started = Instant::now();
    let mut progress = CopyProgress {
        app,
        done: 0,
        total,
        files: 0,
        last: Instant::now(),
    };
    for (path, skip) in &roots {
        if path.is_dir() {
            copy_tree(path, &target, skip, &mut progress)?;
        }
    }
    let files = progress.files;
    let copied = progress.done;
    emit(app, "write", copied, total, files, None);
    let rebased = rebase_json_paths(&target, &source, &target)?;
    if rebased > 0 {
        crate::logging::info(format!(
            "[paths] 已重写 {rebased} 个 JSON 状态文件里的旧路径"
        ));
    }

    // 写指针：先写临时文件再改名，避免断电留下半个 JSON。
    let config = default_config(app).ok_or_else(|| "拿不到默认配置目录".to_string())?;
    fs::create_dir_all(&config).map_err(|e| format!("建配置目录失败：{e}"))?;
    let source_text = source.to_string_lossy().into_owned();
    let target_text = target.to_string_lossy().into_owned();
    // 保留历史根目录：数据目录之外的工程文件里可能还存着更早的绝对路径。
    let mut legacy = read_pointer(app).map(|p| p.legacy).unwrap_or_default();
    legacy.push(source_text.clone());
    // 目标自己不需要出现在 legacy 里。
    legacy.retain(|item| !same_path(Path::new(item), &target));
    legacy.sort();
    legacy.dedup_by(|a, b| lower(Path::new(a)) == lower(Path::new(b)));
    let pointer = Pointer {
        path: target_text,
        previous: Some(source_text),
        legacy,
    };
    let text = serde_json::to_string_pretty(&pointer).map_err(|e| e.to_string())?;
    let tmp = config.join(format!("{POINTER}.tmp"));
    fs::write(&tmp, text).map_err(|e| format!("写数据目录指针失败：{e}"))?;
    fs::rename(&tmp, config.join(POINTER)).map_err(|e| format!("保存数据目录指针失败：{e}"))?;
    if let Err(err) = write_registry(&target) {
        // 注册表只是安装器那条后备路径，写不上不影响指针文件生效。
        crate::logging::info(format!("[paths] {err}（忽略，指针文件已写）"));
    }
    crate::logging::info(format!(
        "[paths] 数据目录已迁移：{} → {}（{} 个文件，用时 {:.1}s）",
        source.display(),
        target.display(),
        files,
        started.elapsed().as_secs_f32()
    ));
    emit(app, "done", copied, total, files, None);
    info_after_migrate(app)
}

/// 迁移刚写完指针：此时 `info()` 已经会读新目录。
fn info_after_migrate(app: &AppHandle) -> Result<DataDirInfo, String> {
    info(app)
}

/// 清理上一次迁移留下的旧目录（重启后调用；失败条目跳过）。
pub fn cleanup_previous(app: &AppHandle) -> Result<Option<String>, String> {
    let Some(pointer) = read_pointer(app) else {
        return Ok(None);
    };
    let Some(previous) = pointer.previous.clone() else {
        return Ok(None);
    };
    let old = PathBuf::from(&previous);
    let current = data_dir(app)?;
    if same_path(&old, &current) || inside(&current, &old) {
        return Err("旧目录包含当前数据目录，已拒绝清理".into());
    }
    if !old.exists() {
        clear_previous(app)?;
        return Ok(Some(previous));
    }
    let keep_pointer = default_config(app)
        .map(|dir| same_path(&dir, &old))
        .unwrap_or(false);
    // 防呆：只删「看起来确实是本应用数据目录」的路径。指针文件是可写的本地状态，
    // 万一被改坏 / 指向用户自己的文件夹，不能把人家整个目录删掉。
    let looks_like_data = {
        const KNOWN: &[&str] = &[
            "models",
            "runtime",
            "local-ai",
            "plugins",
            "history",
            "recordings",
            "agent",
            "session.json",
            "store.json",
            "sound-memories.json",
            "presets.json",
            "ai.json",
        ];
        fs::read_dir(&old)
            .map(|entries| {
                entries.flatten().any(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    KNOWN.contains(&name.as_str())
                })
            })
            .unwrap_or(false)
    };
    if !looks_like_data && !keep_pointer {
        return Err(format!(
            "{} 不像 AI DAW 的数据目录，已拒绝清理",
            old.display()
        ));
    }
    let entries = fs::read_dir(&old).map_err(|e| format!("读旧目录失败：{e}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep_pointer && name == POINTER {
            continue;
        }
        let path = entry.path();
        let result = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        if let Err(err) = result {
            // Windows 上偶发占用（杀毒 / 索引）：跳过，下次再清。
            crate::logging::info(format!("[paths] 清理 {} 失败：{err}", path.display()));
        }
    }
    clear_previous(app)?;
    crate::logging::info(format!("[paths] 已清理旧数据目录：{previous}"));
    Ok(Some(previous))
}

/// 把指针里的 `previous` 清掉（保留 `path` 与 `legacy`）。
///
/// `legacy` 要留着：数据目录之外的工程文件仍可能引用更早的顶级目录，素材加载时还要
/// 靠它做重映射。UI 只看 `previous`，清理完就不会再显示「旧数据」。
fn clear_previous(app: &AppHandle) -> Result<(), String> {
    let Some(config) = default_config(app) else {
        return Ok(());
    };
    let Some(pointer) = read_pointer(app) else {
        return Ok(());
    };
    let text = serde_json::to_string_pretty(&Pointer {
        path: pointer.path,
        previous: None,
        legacy: pointer.legacy,
    })
    .map_err(|e| e.to_string())?;
    let tmp = config.join(format!("{POINTER}.tmp"));
    fs::write(&tmp, text).map_err(|e| format!("写数据目录指针失败：{e}"))?;
    fs::rename(&tmp, config.join(POINTER)).map_err(|e| format!("保存数据目录指针失败：{e}"))?;
    Ok(())
}

fn emit(
    app: &AppHandle,
    phase: &'static str,
    done: u64,
    total: u64,
    files: u64,
    message: Option<String>,
) {
    let _ = app.emit(
        EVENT_PROGRESS,
        DataDirProgress {
            phase,
            done,
            total,
            files,
            message,
        },
    );
}

/// 设置页读：当前数据目录、默认目录、占用大小、来源。
#[tauri::command]
pub async fn data_dir_info(app: AppHandle) -> Result<DataDirInfo, String> {
    tauri::async_runtime::spawn_blocking(move || info(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// 数据目录所在磁盘的剩余字节数。
///
/// **尽力而为**：拿不到（无权限、命令被策略禁用、非本地盘）就返回 `None`，由前端当作
/// 「不知道」放行。这里刻意不用 `unsafe` 直接调 `GetDiskFreeSpaceExW`——整个 crate 是
/// `#![forbid(unsafe_code)]`，一次 200–400 ms 的 PowerShell 探测换个心安。
#[tauri::command]
pub fn disk_free_bytes(app: AppHandle) -> Option<u64> {
    let dir = config_dir(&app).ok()?;
    free_bytes_at(&dir)
}

/// 某个路径所在卷的剩余字节（`df` / `Get-PSDrive` 的薄封装）。
fn free_bytes_at(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        // `C:\Users\x` → `C:\`：取最后一个 component 作为卷根。
        let root = path.ancestors().last()?;
        let text = root.to_string_lossy();
        let drive = text.trim_end_matches('\\').trim_end_matches(':');
        // UNC（`\\server\share`）没有盘符，`Get-PSDrive` 查不到——直接当作「不知道」。
        if drive.len() != 1 || !drive.chars().next()?.is_ascii_alphabetic() {
            return None;
        }
        let script = format!("(Get-PSDrive -Name {drive}).Free");
        let output = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }
    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("df")
            .args(["-k", &path.to_string_lossy()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        // 第二行：Filesystem 1024-blocks Used Available Capacity Mounted-on
        let line = text.lines().nth(1)?;
        let available: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
        Some(available.saturating_mul(1024))
    }
}

/// 设置页一键迁移：复制当前数据到 `target`，写指针，返回需要重启的新状态。
#[tauri::command]
pub async fn data_dir_migrate(app: AppHandle, target: String) -> Result<DataDirInfo, String> {
    tauri::async_runtime::spawn_blocking(move || migrate(&app, target))
        .await
        .map_err(|e| e.to_string())?
}

/// 重启后清理上一次迁移的旧目录（返回清掉的路径）。
#[tauri::command]
pub async fn data_dir_cleanup_previous(app: AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || cleanup_previous(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_pointer_is_accepted() {
        let dir = std::env::temp_dir().join(format!("aidaw-paths-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(POINTER), "D:\\AI DAW Data\n").unwrap();
        let pointer = read_pointer_from(&dir).expect("纯文本也要能读");
        assert_eq!(pointer.path, "D:\\AI DAW Data");
        assert!(pointer.previous.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_pointer_keeps_previous() {
        let dir =
            std::env::temp_dir().join(format!("aidaw-paths-json-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(POINTER),
            r#"{"path":"E:\\Data","previous":"C:\\Old"}"#,
        )
        .unwrap();
        let pointer = read_pointer_from(&dir).expect("JSON 指针");
        assert_eq!(pointer.path, "E:\\Data");
        assert_eq!(pointer.previous.as_deref(), Some("C:\\Old"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn nesting_checks_reject_parent_and_child() {
        let base = if cfg!(windows) {
            PathBuf::from(r"D:\AI DAW")
        } else {
            PathBuf::from("/ai-daw")
        };
        let child = base.join("models");
        assert!(inside(&child, &base));
        assert!(!inside(&base, &child));
        // 前缀相同但不是同一层：`D:\AI DAW2` 不算在里面。
        let sibling = base.with_file_name("AI DAW2");
        assert!(!inside(&sibling, &base));
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_prefix_is_stripped_before_storing() {
        assert_eq!(
            simplified(PathBuf::from(r"\\?\D:\AI DAW Data")),
            PathBuf::from(r"D:\AI DAW Data")
        );
        assert_eq!(
            simplified(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\server\share")
        );
    }

    #[test]
    fn json_state_paths_are_rebased_to_the_new_root() {
        let base = std::env::temp_dir().join(format!("aidaw-paths-rebase-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("nested")).unwrap();
        let from = PathBuf::from(r"C:\Users\me\AppData\Roaming\com.aidaw.app");
        let to = PathBuf::from(r"D:\AI DAW Data");
        // JSON 里的反斜杠是双写的；写成原始字符串更接近磁盘上的真实内容。
        fs::write(
            base.join("session.json"),
            r#"{"audio":{"path":"C:\\Users\\me\\AppData\\Roaming\\com.aidaw.app\\local-ai\\a.wav"}}"#,
        )
        .unwrap();
        fs::write(base.join("nested").join("plain.json"), "not a path").unwrap();
        let changed = rebase_json_paths(&base, &from, &to).unwrap();
        assert_eq!(changed, 1);
        let text = fs::read_to_string(base.join("session.json")).unwrap();
        assert!(text.contains(r"D:\\AI DAW Data\\local-ai\\a.wav"), "{text}");
        assert!(!text.contains("com.aidaw.app"));
        let _ = fs::remove_dir_all(&base);
    }
}
