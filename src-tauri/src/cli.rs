//! `aidaw` CLI 的**挂载 / 卸载**：让用户在终端里随处可敲 `aidaw`。
//!
//! **不自动做任何事**：启动时不碰 PATH、不弹密码框。挂载是设置页里的一个**显式开关**——
//! 用户点它的时候才知道「这次要输密码是为了把命令行挂上去」。不打开也完全能用，
//! 只是调用要给全路径（`…/AI DAW.app/Contents/MacOS/aidaw`）。
//!
//! 二进制由 Tauri 作为 sidecar 打进安装包（`bundle.externalBin`），落在**主程序旁边**；
//! 这里负责把它接进 PATH：
//!
//! - **macOS**：软链 `/usr/local/bin/aidaw` → 包内二进制。需要一次管理员授权（Ollama 同款）。
//! - **Windows**：把安装目录追加进**用户** PATH（`HKCU\Environment`），不需要提权。
//! - **Linux（deb / rpm）**：sidecar 本就在 `/usr/bin`，PATH 上已经有了 → 无需动作。
//! - **Linux（AppImage）**：二进制路径每次挂载都变，不能软链 → 不自动安装。
//!
//! 软链 / PATH 条目都指向**包内**文件，所以 App 升级时命令自动跟着更新，不存在版本漂移。
//! 状态直接**按固定路径 / 注册表**判断，不读 PATH：从 Finder 启动的 GUI 进程只拿到最小 PATH，
//! 看不到用户 shell 里的 PATH。因为开关是显式的，这里**不需要**任何配置文件的记忆——
//! 真相就在文件系统 / 注册表里。

use std::path::{Path, PathBuf};

use serde::Serialize;

/// 命令名，也是 sidecar 去掉三元组后缀后的文件名。
const COMMAND: &str = "aidaw";

/// **发现文件**名：包内 `aidaw` 的绝对路径。与 `mcp-url` 并列放在应用配置目录里。
///
/// 给**外部 AI harness** 用的：按 bundle id（`com.aidaw.app`）推导出配置目录、读这个文件，
/// 就能拿到全路径——**不需要搜索**（不依赖 Spotlight / 注册表 / PATH），也不需要用户开挂载开关。
pub const PATH_FILE: &str = "cli-path";

/// macOS / Linux 上放软链的目录。
#[cfg(unix)]
const LINK_DIR: &str = "/usr/local/bin";

/// Linux 系统包把二进制放这儿，天然在 PATH 上。
#[cfg(unix)]
const SYSTEM_BIN_DIR: &str = "/usr/bin";

// ---------------------------------------------------------------------------
// 状态
// ---------------------------------------------------------------------------

/// 安装状态。前端据此决定开关是开还是关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliState {
    /// 已挂载，且指向包内二进制。
    Installed,
    /// 没挂载，可以挂。
    Missing,
    /// 同名命令已存在，但**不是我们**挂的 —— 打开开关会覆盖它。
    Conflict,
    /// 系统包管理器已经放好了（Linux deb / rpm），无需动作。
    System,
    /// 这个环境挂不了（开发构建没有真包；AppImage 路径不稳定）。
    Unsupported,
}

/// 挂不了时的原因（前端据此本地化文案）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliHint {
    /// 开发构建：不是真正的安装包。
    DevBuild,
    /// AppImage：二进制在临时挂载点里，路径每次都变。
    AppImage,
    /// 这个平台需要 root，只能给出手工命令。
    ///
    /// 只在 Linux 上构造（macOS 没有这个状态），但它是**发给前端的契约**，故保留。
    #[allow(dead_code)]
    NeedsManual,
}

/// 汇报给界面的安装状态。
#[derive(Debug, Clone, Serialize)]
pub struct CliStatus {
    pub state: CliState,
    /// 命令名。
    pub command: String,
    /// 已挂载时：软链（macOS）或 PATH 条目（Windows）指向的位置。
    pub target: Option<String>,
    /// 包内 `aidaw` 的位置；`None` = 这个构建里没有 sidecar。
    pub bundled: Option<String>,
    /// 冲突时：现有那份 `aidaw` 在哪。
    pub conflict: Option<String>,
    /// 挂不了时的原因。
    pub hint: Option<CliHint>,
    /// 手工安装命令（可直接复制到终端）。
    pub manual_command: Option<String>,
    /// 不挂也能用：包内二进制的**全路径**，可直接调用。
    pub full_path: Option<String>,
}

// ---------------------------------------------------------------------------
// 要改 PATH 的动作（全由设置页的开关触发）
// ---------------------------------------------------------------------------

/// 读取 `aidaw` 的挂载状态。
pub fn status() -> CliStatus {
    classify(&probe())
}

/// 把 `aidaw` 接进 PATH（macOS 会弹一次管理员授权）。
///
/// 只会被设置页的开关调用，所以「覆盖已存在的同名命令」是用户**显式**意图。
pub fn install() -> Result<CliStatus, String> {
    let probe = probe();
    let Some(bundled) = probe.bundled.clone() else {
        return Err("这个构建里没有内置 aidaw（开发构建？）".to_owned());
    };
    if let Some(reason) = probe.unsupported {
        return Err(format!("此环境不支持挂载：{reason:?}"));
    }

    match install_command(&bundled, &probe) {
        Ok(()) => {}
        Err(InstallError::Declined) => return Err("已取消管理员授权".to_owned()),
        Err(InstallError::Failed(message)) => return Err(message),
    }

    // 装完回读校验：别假装成功。
    let after = status();
    if after.state != CliState::Installed {
        return Err("挂载似乎没有生效".to_owned());
    }
    Ok(after)
}

/// 卸载（只动我们自己挂的那份）。
pub fn uninstall() -> Result<CliStatus, String> {
    if status().state != CliState::Installed {
        // 不是我们挂的（或本来就没有）→ 不动别人的东西。
        return Ok(status());
    }
    let probe = probe();
    match uninstall_command(&probe) {
        Ok(()) => Ok(status()),
        Err(InstallError::Declined) => Err("已取消管理员授权".to_owned()),
        Err(InstallError::Failed(message)) => Err(message),
    }
}

// ---------------------------------------------------------------------------
// 发现文件：把「我在哪」告诉外部工具
// ---------------------------------------------------------------------------

/// 把 `bundled` 写进 `dir/cli-path`；`None`（没有 sidecar 的构建）什么都不做。
fn write_path_file(dir: &Path, bundled: Option<&Path>) -> std::io::Result<()> {
    let Some(bundled) = bundled else { return Ok(()) };
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(PATH_FILE), bundled.display().to_string())
}

/// 启动时调用：登记包内 `aidaw` 的绝对路径，供外部 AI harness 直接读取。
///
/// **每次启动都重写**——用户可能把 `.app` 拖到别处，写一次就不准了。
/// 与挂载开关无关：这只是「告诉别人我在哪」，不改 PATH、不要密码。
pub fn publish_path(dir: &Path) {
    if let Err(err) = write_path_file(dir, bundled_binary().as_deref()) {
        eprintln!("[cli] 写入发现文件 {PATH_FILE} 失败：{err}");
    }
}

// ---------------------------------------------------------------------------
// 探测与状态判定
// ---------------------------------------------------------------------------

/// 一次环境探测的原始事实（纯数据，便于单测）。
#[derive(Debug, Clone)]
struct Probe {
    /// 包内 `aidaw`。
    bundled: Option<PathBuf>,
    /// 我们会写的位置（Unix = `/usr/local/bin/aidaw`；Windows = 安装目录）。
    link: PathBuf,
    /// 该位置当前指向哪里（不存在 = `None`）。
    link_target: Option<PathBuf>,
    /// 该位置是否存在（断链的软链也算存在）。
    link_exists: bool,
    /// 系统 PATH 上已经有了（Linux 系统包）。
    system_present: bool,
    /// 环境层面的「挂不了」原因。
    unsupported: Option<CliHint>,
    /// 手工安装命令。
    manual_command: Option<String>,
}

/// 包内 `aidaw` 的位置 = **主程序旁边**的同名文件（Tauri sidecar 就是这么放的）。
fn bundled_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { format!("{COMMAND}.exe") } else { COMMAND.to_owned() };
    let candidate = exe.parent()?.join(name);
    candidate.is_file().then_some(candidate)
}

/// 主程序所在目录（Windows 上要把它加进用户 PATH）。
#[cfg(windows)]
fn install_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// 判据（按序）：
///
/// 1. **macOS 的 `.app` 包优先**：真正的 App（含本机 `tauri build` 的产物，
///    它落在 `target/release/bundle/macos/AI DAW.app/`）一定在
///    `….app/Contents/MacOS/<exe>` 里 → **不是**开发构建。
///    少了这一条，本机打出来的包会被误判成开发构建，开关就永远是灰的。
/// 2. 其余情况看路径里**最后一个 `target` 之后**有没有 `debug` / `release` 段 → cargo 产物目录。
///
/// 为什么必须判断：Tauri **也会**把 sidecar 复制进 `target/debug`、`target/release`，
/// 所以「旁边有没有 aidaw」不足以区分开发构建与已安装的 App；把用户的全局命令指到
/// 临时产物上是不该发生的事。
///
/// 按 `/` 与 `\` 手工切分，好让 Windows 的路径在别的平台上也能被这条判据正确评估。
fn looks_like_dev_build(exe: &Path) -> bool {
    if exe.to_string_lossy().contains(".app/Contents/MacOS/") {
        return false;
    }
    let text = exe.to_string_lossy();
    let names: Vec<&str> = text.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    match names.iter().rposition(|name| *name == "target") {
        Some(index) => names[index + 1..].iter().any(|name| *name == "debug" || *name == "release"),
        None => false,
    }
}

/// 拿不到自身路径时保守当作开发构建：宁可不挂，也不要误改用户环境。
fn is_dev_build() -> bool {
    std::env::current_exe().map(|exe| looks_like_dev_build(&exe)).unwrap_or(true)
}

/// 状态判定：**纯函数**（只依赖探测到的事实），便于单测。
fn classify(probe: &Probe) -> CliStatus {
    let bundled = probe.bundled.as_ref().map(|p| p.display().to_string());
    let mut status = CliStatus {
        state: CliState::Missing,
        command: COMMAND.to_owned(),
        target: None,
        bundled: bundled.clone(),
        conflict: None,
        hint: None,
        manual_command: probe.manual_command.clone(),
        full_path: bundled,
    };

    if let Some(hint) = probe.unsupported {
        status.state = CliState::Unsupported;
        status.hint = Some(hint);
        return status;
    }
    if probe.link_exists {
        if probe.bundled.as_ref().is_some_and(|bundled| path_points_at(probe, bundled)) {
            status.state = CliState::Installed;
            status.target = Some(
                probe.link_target.clone().unwrap_or_else(|| probe.link.clone()).display().to_string(),
            );
        } else {
            status.state = CliState::Conflict;
            status.conflict = Some(
                probe.link_target.clone().unwrap_or_else(|| probe.link.clone()).display().to_string(),
            );
        }
        return status;
    }
    if probe.system_present {
        status.state = CliState::System;
        status.target = status.bundled.clone();
        return status;
    }
    if probe.bundled.is_none() {
        status.state = CliState::Unsupported;
        status.hint = Some(CliHint::DevBuild);
    }
    status
}

/// `probe` 描述的位置是否正指着包内那份二进制。
fn path_points_at(probe: &Probe, bundled: &Path) -> bool {
    match &probe.link_target {
        Some(target) => same_file(target, bundled),
        None => false,
    }
}

/// 两个路径是否指向同一个文件（能规范化就规范化，否则按字面比）。
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// macOS 能提权建软链，不存在「只能手工」的情况。
#[cfg(target_os = "macos")]
fn needs_manual(_bundled: &Option<PathBuf>, _system_present: bool) -> Option<CliHint> {
    None
}

/// Linux 的常态是 deb/rpm（此时 `system_present`）；剩下要软链的情况只能靠 root。
#[cfg(all(unix, not(target_os = "macos")))]
fn needs_manual(bundled: &Option<PathBuf>, system_present: bool) -> Option<CliHint> {
    (bundled.is_some() && !system_present).then_some(CliHint::NeedsManual)
}

#[cfg(unix)]
fn probe() -> Probe {
    let bundled = bundled_binary();
    let link = PathBuf::from(LINK_DIR).join(COMMAND);
    let link_exists = std::fs::symlink_metadata(&link).is_ok();
    let link_target = std::fs::read_link(&link).ok();
    let system_present = bundled.as_deref().and_then(Path::parent) == Some(Path::new(SYSTEM_BIN_DIR));
    // 开发构建不碰用户环境；AppImage 的二进制在临时挂载点，软链必然失效。
    let unsupported = if is_dev_build() {
        Some(CliHint::DevBuild)
    } else if std::env::var_os("APPIMAGE").is_some() {
        Some(CliHint::AppImage)
    } else {
        needs_manual(&bundled, system_present)
    };
    let manual_command = bundled.as_ref().map(|bundled| {
        format!(
            "sudo /bin/mkdir -p {dir} && sudo /bin/ln -sfn {src} {dir}/{COMMAND}",
            dir = LINK_DIR,
            src = sh_quote(&bundled.display().to_string()),
        )
    });
    Probe { bundled, link, link_target, link_exists, system_present, unsupported, manual_command }
}

#[cfg(windows)]
fn probe() -> Probe {
    let bundled = bundled_binary();
    let link = install_dir().unwrap_or_default();
    let present = user_path_contains(&link).unwrap_or(false);
    Probe {
        bundled,
        link_target: present.then(|| link.clone()),
        link,
        link_exists: present,
        system_present: false,
        unsupported: is_dev_build().then_some(CliHint::DevBuild),
        manual_command: None,
    }
}

// ---------------------------------------------------------------------------
// 安装 / 卸载
// ---------------------------------------------------------------------------

/// 安装失败：区分「用户取消授权」与真失败。
enum InstallError {
    Declined,
    Failed(String),
}

#[cfg(target_os = "macos")]
fn install_command(bundled: &Path, probe: &Probe) -> Result<(), InstallError> {
    let dir = probe
        .link
        .parent()
        .ok_or_else(|| InstallError::Failed("安装目录不合法".to_owned()))?;
    // `/usr/local/bin` 在 Apple Silicon 上常常不存在，所以先 mkdir。
    let shell = format!(
        "/bin/mkdir -p {dir} && /bin/ln -sfn {src} {link}",
        dir = sh_quote(&dir.display().to_string()),
        src = sh_quote(&bundled.display().to_string()),
        link = sh_quote(&probe.link.display().to_string()),
    );
    run_privileged(&shell)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn install_command(_bundled: &Path, _probe: &Probe) -> Result<(), InstallError> {
    // Linux 的常态是 deb/rpm（走 System 分支，根本到不了这里）；真到了需要软链的情况，
    // 提权得靠 pkexec，留给后续——先老实给出手工命令。
    Err(InstallError::Failed("需要 root 权限，请按提示的命令手工挂载".to_owned()))
}

#[cfg(windows)]
fn install_command(bundled: &Path, _probe: &Probe) -> Result<(), InstallError> {
    let dir = bundled
        .parent()
        .ok_or_else(|| InstallError::Failed("安装目录不合法".to_owned()))?
        .to_path_buf();
    add_to_user_path(&dir).map_err(InstallError::Failed)
}

#[cfg(not(any(unix, windows)))]
fn install_command(_bundled: &Path, _probe: &Probe) -> Result<(), InstallError> {
    Err(InstallError::Failed("不支持的平台".to_owned()))
}

#[cfg(target_os = "macos")]
fn uninstall_command(probe: &Probe) -> Result<(), InstallError> {
    let shell = format!("/bin/rm -f {link}", link = sh_quote(&probe.link.display().to_string()));
    run_privileged(&shell)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn uninstall_command(_probe: &Probe) -> Result<(), InstallError> {
    Err(InstallError::Failed("请用系统包管理器卸载".to_owned()))
}

#[cfg(windows)]
fn uninstall_command(probe: &Probe) -> Result<(), InstallError> {
    remove_from_user_path(&probe.link).map_err(InstallError::Failed)
}

#[cfg(not(any(unix, windows)))]
fn uninstall_command(_probe: &Probe) -> Result<(), InstallError> {
    Err(InstallError::Failed("不支持的平台".to_owned()))
}

/// 用 `osascript` 以管理员身份跑一段 shell（会弹系统授权框）。
#[cfg(target_os = "macos")]
fn run_privileged(shell_command: &str) -> Result<(), InstallError> {
    use std::process::Command;

    let script = osascript_script(shell_command);
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| InstallError::Failed(format!("无法调用 osascript：{e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("User canceled") || stderr.contains("-128") {
        return Err(InstallError::Declined);
    }
    Err(InstallError::Failed(format!("挂载失败：{}", stderr.trim())))
}

// ---------------------------------------------------------------------------
// 转义（纯函数，单测覆盖）
// ---------------------------------------------------------------------------

/// shell 单引号引用：`'` → `'\''`。
///
/// 路径里带空格（`/Applications/AI DAW.app/…`）或引号时都靠它。
fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// AppleScript 字符串字面量转义（反斜杠与双引号）。
fn applescript_escape(value: &str) -> String {
    value.replace('\\', r"\\").replace('"', "\\\"")
}

/// 组装 `osascript -e` 的脚本：把一段 shell 命令交给带管理员权限的 `do shell script`。
fn osascript_script(shell_command: &str) -> String {
    format!(
        "do shell script \"{}\" with administrator privileges",
        applescript_escape(shell_command)
    )
}

// ---------------------------------------------------------------------------
// Windows：用户 PATH
// ---------------------------------------------------------------------------

/// 读 `HKCU\Environment` 的 `Path`（连同它的类型，写回时保持原类型）。
#[cfg(windows)]
fn read_user_path() -> Result<(String, winreg::enums::RegType), String> {
    use winreg::enums::{HKEY_CURRENT_USER, RegType};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .map_err(|e| format!("打开注册表 Environment 失败：{e}"))?;
    match key.get_raw_value("Path") {
        Ok(raw) => {
            let units: Vec<u16> =
                raw.bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let text = String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned();
            Ok((text, raw.vtype))
        }
        // 没有就按 REG_EXPAND_SZ 新建（PATH 里常有 %USERPROFILE% 这类变量）。
        Err(_) => Ok((String::new(), RegType::REG_EXPAND_SZ)),
    }
}

/// 以原类型写回用户 PATH。
#[cfg(windows)]
fn write_user_path(value: &str, vtype: winreg::enums::RegType) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::{RegKey, RegValue};

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Environment")
        .map_err(|e| format!("打开注册表 Environment 失败：{e}"))?;
    let mut bytes: Vec<u8> = value.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    bytes.extend([0, 0]); // 结尾 NUL
    key.set_raw_value("Path", &RegValue { bytes: bytes.into(), vtype })
        .map_err(|e| format!("写入用户 PATH 失败：{e}"))
}

#[cfg(windows)]
fn user_path_contains(dir: &Path) -> Result<bool, String> {
    let (path, _) = read_user_path()?;
    let want = normalize_path(dir);
    Ok(path.split(';').any(|entry| normalize_path(Path::new(entry.trim())) == want))
}

#[cfg(windows)]
fn add_to_user_path(dir: &Path) -> Result<(), String> {
    let (path, vtype) = read_user_path()?;
    let want = normalize_path(dir);
    if path.split(';').any(|entry| normalize_path(Path::new(entry.trim())) == want) {
        return Ok(());
    }
    let joined = if path.trim().is_empty() {
        want
    } else {
        format!("{};{}", path.trim_end_matches(';'), want)
    };
    write_user_path(&joined, vtype)
}

#[cfg(windows)]
fn remove_from_user_path(dir: &Path) -> Result<(), String> {
    let (path, vtype) = read_user_path()?;
    let want = normalize_path(dir);
    let kept: Vec<String> = path
        .split(';')
        .filter(|entry| !entry.trim().is_empty())
        .filter(|entry| normalize_path(Path::new(entry.trim())) != want)
        .map(|entry| entry.trim().to_owned())
        .collect();
    write_user_path(&kept.join(";"), vtype)
}

/// Windows 路径比较用：去掉尾部分隔符、统一小写。
#[cfg(windows)]
fn normalize_path(path: &Path) -> String {
    path.display().to_string().trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// Tauri 命令
// ---------------------------------------------------------------------------

/// 读取 `aidaw` 的挂载状态。
#[tauri::command]
pub fn cli_status() -> CliStatus {
    status()
}

/// 把 `aidaw` 接进 PATH（macOS 会弹一次管理员授权）。由设置页的开关调用。
#[tauri::command]
pub fn install_cli() -> Result<CliStatus, String> {
    install()
}

/// 撤掉本应用挂的 `aidaw`。由设置页的开关调用。
#[tauri::command]
pub fn uninstall_cli() -> Result<CliStatus, String> {
    uninstall()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_with(bundled: Option<&str>, link_target: Option<&str>, exists: bool) -> Probe {
        Probe {
            bundled: bundled.map(PathBuf::from),
            link: PathBuf::from("/usr/local/bin/aidaw"),
            link_target: link_target.map(PathBuf::from),
            link_exists: exists,
            system_present: false,
            unsupported: None,
            manual_command: None,
        }
    }

    /// 临时目录：每个用例一个，避免互相踩。
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aidaw-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn publish_path_records_the_bundled_binary() {
        let dir = temp_dir("publish");
        let bundled = Path::new("/Applications/AI DAW.app/Contents/MacOS/aidaw");
        write_path_file(&dir, Some(bundled)).expect("应能写发现文件");
        let text = std::fs::read_to_string(dir.join(PATH_FILE)).expect("发现文件应存在");
        assert_eq!(text, bundled.display().to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publish_path_does_nothing_without_a_sidecar() {
        // 没有 sidecar（比如 `cargo test` 自己）时不该凭空造出一个文件。
        let dir = temp_dir("nosidecar");
        write_path_file(&dir, None).expect("无 sidecar 也该是 Ok");
        assert!(!dir.join(PATH_FILE).exists());
    }

    #[test]
    fn shell_quote_survives_spaces_and_quotes() {
        assert_eq!(sh_quote("/Applications/AI DAW.app/aidaw"), "'/Applications/AI DAW.app/aidaw'");
        // 单引号要被拆出来转义，否则整条命令会被截断。
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn applescript_escape_doubles_backslash_and_quote() {
        assert_eq!(applescript_escape(r#"/bin/ln -sfn "a""#), r#"/bin/ln -sfn \"a\""#);
        assert_eq!(applescript_escape(r"a\b"), r"a\\b");
    }

    #[test]
    fn osascript_script_wraps_the_shell_command() {
        let script = osascript_script("/bin/ln -sfn '/a' '/b'");
        assert_eq!(script, "do shell script \"/bin/ln -sfn '/a' '/b'\" with administrator privileges");
    }

    #[test]
    fn classify_reports_missing_when_nothing_is_there() {
        let status = classify(&probe_with(Some("/app/aidaw"), None, false));
        assert_eq!(status.state, CliState::Missing);
        assert_eq!(status.bundled.as_deref(), Some("/app/aidaw"));
        // 没挂也能用：全路径要能告诉用户。
        assert_eq!(status.full_path.as_deref(), Some("/app/aidaw"));
    }

    #[test]
    fn classify_reports_installed_when_link_points_at_bundled() {
        let probe = probe_with(Some("/app/aidaw"), Some("/app/aidaw"), true);
        assert_eq!(classify(&probe).state, CliState::Installed);
    }

    #[test]
    fn classify_reports_conflict_for_a_foreign_command() {
        let probe = probe_with(Some("/app/aidaw"), Some("/opt/other/aidaw"), true);
        let status = classify(&probe);
        assert_eq!(status.state, CliState::Conflict);
        assert_eq!(status.conflict.as_deref(), Some("/opt/other/aidaw"));
    }

    #[test]
    fn classify_reports_system_when_the_package_manager_installed_it() {
        let mut probe = probe_with(Some("/usr/bin/aidaw"), None, false);
        probe.system_present = true;
        assert_eq!(classify(&probe).state, CliState::System);
    }

    #[test]
    fn classify_reports_unsupported_without_a_sidecar() {
        let status = classify(&probe_with(None, None, false));
        assert_eq!(status.state, CliState::Unsupported);
        assert_eq!(status.hint, Some(CliHint::DevBuild));
    }

    #[test]
    fn dev_build_detection_covers_cargo_target_dirs() {
        // 开发构建：cargo 产物目录（含交叉编译的 <triple> 一层）。
        assert!(looks_like_dev_build(Path::new(
            "/Users/me/proj/src-tauri/target/debug/ai-daw-app"
        )));
        assert!(looks_like_dev_build(Path::new(
            "/Users/me/proj/src-tauri/target/release/ai-daw-app"
        )));
        assert!(looks_like_dev_build(Path::new(
            "/Users/me/proj/target/aarch64-apple-darwin/release/ai-daw-app"
        )));
        // Windows 的开发构建（在 macOS 上跑这条判据也要判对）。
        assert!(looks_like_dev_build(Path::new(
            r"C:\Users\me\proj\src-tauri\target\debug\ai-daw-app.exe"
        )));
    }

    #[test]
    fn real_installs_are_not_dev_builds() {
        // 本机 `tauri build` 的产物也在 target/ 下，但它在 .app 包里 → 必须当成「真安装」，
        // 否则本地打包出来开关永远是灰的。
        assert!(!looks_like_dev_build(Path::new(
            "/Users/me/proj/src-tauri/target/release/bundle/macos/AI DAW.app/Contents/MacOS/ai-daw-app"
        )));
        assert!(!looks_like_dev_build(Path::new(
            "/Applications/AI DAW.app/Contents/MacOS/ai-daw-app"
        )));
        assert!(!looks_like_dev_build(Path::new("/usr/bin/ai-daw-app")));
        assert!(!looks_like_dev_build(Path::new(r"C:\Program Files\AI DAW\ai-daw-app.exe")));
    }

    #[test]
    fn classify_reports_unsupported_for_appimage() {
        let mut probe = probe_with(Some("/tmp/.mount_x/usr/bin/aidaw"), None, false);
        probe.unsupported = Some(CliHint::AppImage);
        let status = classify(&probe);
        assert_eq!(status.state, CliState::Unsupported);
        assert_eq!(status.hint, Some(CliHint::AppImage));
    }

    #[test]
    fn classify_offers_a_manual_command_when_root_is_needed() {
        let mut probe = probe_with(Some("/opt/ai-daw/aidaw"), None, false);
        probe.unsupported = Some(CliHint::NeedsManual);
        probe.manual_command =
            Some("sudo ln -sfn '/opt/ai-daw/aidaw' '/usr/local/bin/aidaw'".to_owned());
        let status = classify(&probe);
        assert_eq!(status.state, CliState::Unsupported);
        assert_eq!(status.hint, Some(CliHint::NeedsManual));
        assert!(status.manual_command.is_some(), "手工命令要能复制给用户");
    }
}
