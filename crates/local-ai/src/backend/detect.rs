//! **硬件探测**：给「推荐哪个后端」提供依据。
//!
//! 设计上把**解析**与 **IO** 分开：`vendor_of_adapter` / `parse_nvidia_smi` /
//! `parse_windows_adapters` / `parse_macos_chip` 都是纯函数，喂字符串就能单测；
//! [`detect`] 才真正去调外部命令。**探测失败一律退化成「无 GPU」**（→ 推荐 CPU），
//! 绝不因为拿不到型号就在启动路径上出错。
//!
//! **启动路径只用 [`platform_only`]**——它不碰任何子进程。完整探测走 [`detect_cached`]
//! （带缓存），并由 [`warm_in_background`] 在后台预热；Windows 上那是 1–3 秒的开销。

/// GPU 厂商（够用来选后端即可，不追求穷举）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    /// Apple（M 系列 / Apple GPU）—— CoreML 的地盘。
    Apple,
    /// NVIDIA —— CUDA 的地盘。
    Nvidia,
    /// AMD —— 只能走 DirectML（Windows）。
    Amd,
    /// Intel —— 只能走 DirectML（Windows）。
    Intel,
    /// 认得出是 GPU、但厂商未知。
    Other,
}

impl GpuVendor {
    pub fn as_str(self) -> &'static str {
        match self {
            GpuVendor::Apple => "apple",
            GpuVendor::Nvidia => "nvidia",
            GpuVendor::Amd => "amd",
            GpuVendor::Intel => "intel",
            GpuVendor::Other => "other",
        }
    }
}

/// 本机硬件画像。字段都是「尽力而为」：拿不到就是 `None`。
#[derive(Debug, Clone)]
pub struct HardwareProfile {
    /// `std::env::consts::OS`（`macos` / `windows` / `linux`）。
    pub os: &'static str,
    /// `std::env::consts::ARCH`（`aarch64` / `x86_64` …）。
    pub arch: &'static str,
    /// 主显卡厂商。
    pub gpu: Option<GpuVendor>,
    /// 主显卡型号（展示用）。
    pub gpu_name: Option<String>,
    /// NVIDIA 驱动版本（有才填）。
    pub driver: Option<String>,
}

impl HardwareProfile {
    /// 探测不到 GPU 的画像（也用探测失败时的兜底）。
    pub fn headless(os: &'static str, arch: &'static str) -> Self {
        HardwareProfile {
            os,
            arch,
            gpu: None,
            gpu_name: None,
            driver: None,
        }
    }
}

/// 从一段显卡型号里判厂商。
pub fn vendor_of_adapter(name: &str) -> Option<GpuVendor> {
    let n = name.to_ascii_lowercase();
    if n.contains("nvidia")
        || n.contains("geforce")
        || n.contains("rtx")
        || n.contains("gtx")
        || n.contains("quadro")
        || n.contains("tesla")
        || n.contains("cuda")
    {
        Some(GpuVendor::Nvidia)
    } else if n.contains("apple")
        || n.contains("m1")
        || n.contains("m2")
        || n.contains("m3")
        || n.contains("m4")
    {
        Some(GpuVendor::Apple)
    } else if n.contains("amd") || n.contains("radeon") || n.contains("advanced micro devices") {
        Some(GpuVendor::Amd)
    } else if n.contains("intel")
        || n.contains("iris")
        || n.contains("uhd graphics")
        || n.contains("arc ")
    {
        Some(GpuVendor::Intel)
    } else if n.contains("gpu") || n.contains("graphics") || n.contains("display") {
        Some(GpuVendor::Other)
    } else {
        None
    }
}

/// macOS：`sysctl -n machdep.cpu.brand_string` / `system_profiler SPDisplaysDataType` 里取芯片名。
///
/// `sysctl` 的输出就是一行（如 `Apple M4`）；`system_profiler` 是 `Chipset Model: Apple M4`。
pub fn parse_macos_chip(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("Chipset Model") {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
            continue;
        }
        // `sysctl` 那种裸的一行。
        return Some(line.to_string());
    }
    None
}

/// Linux：解析 `nvidia-smi --query-gpu=name,driver_version --format=csv,noheader,nounits`。
///
/// 形如 `NVIDIA GeForce RTX 4090, 550.54.14`（每行一张卡，取第一张）。
pub fn parse_nvidia_smi(text: &str) -> Option<(String, Option<String>)> {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split(',').map(|s| s.trim());
        let name = parts.next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let driver = parts
            .next()
            .filter(|d| !d.is_empty() && *d != "[N/A]")
            .map(|d| d.to_string());
        return Some((name, driver));
    }
    None
}

/// Windows：解析 `wmic path win32_VideoController get name`（或 PowerShell）的适配器列表。
///
/// 去掉表头 `Name` 与空行，其余每行一个适配器型号。
pub fn parse_windows_adapters(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .filter(|l| !l.eq_ignore_ascii_case("Name"))
        .map(|l| l.to_string())
        .collect()
}

/// 一串适配器里挑主显卡（优先独立显卡：NVIDIA > AMD > Intel > 其它）。
// `pick_primary` 只有 Windows 探测与单测用得上，其它平台上会「未使用」。
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn pick_primary(adapters: &[String]) -> Option<(GpuVendor, String)> {
    let ranked = |v: GpuVendor| match v {
        GpuVendor::Nvidia => 0,
        GpuVendor::Amd => 1,
        GpuVendor::Intel => 3,
        GpuVendor::Apple => 0,
        GpuVendor::Other => 4,
    };
    adapters
        .iter()
        .filter_map(|name| vendor_of_adapter(name).map(|v| (v, name.clone())))
        .min_by_key(|(v, _)| ranked(*v))
}

/// 探测本机硬件。任何一步失败都只是让对应字段为 `None`。
pub fn detect() -> HardwareProfile {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let mut profile = HardwareProfile::headless(os, arch);

    #[cfg(target_os = "macos")]
    {
        // Apple Silicon 上 `machdep.cpu.brand_string` 就是 `Apple M4` 这类芯片名；
        // Intel Mac 上是 `Intel(R) Core(TM) …`。拿不到就按架构兜底。
        if let Some(chip) = run("sysctl", &["-n", "machdep.cpu.brand_string"])
            .as_deref()
            .and_then(parse_macos_chip)
        {
            profile.gpu = vendor_of_adapter(&chip).or(Some(GpuVendor::Other));
            profile.gpu_name = Some(chip);
        } else if arch == "aarch64" {
            profile.gpu = Some(GpuVendor::Apple);
            profile.gpu_name = Some("Apple Silicon".into());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(text) = windows_adapters() {
            if let Some((vendor, name)) = pick_primary(&parse_windows_adapters(&text)) {
                profile.gpu = Some(vendor);
                profile.gpu_name = Some(name);
            }
        }
        if profile.gpu == Some(GpuVendor::Nvidia) {
            profile.driver = windows_nvidia_driver();
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(text) = run(
            "nvidia-smi",
            &[
                "--query-gpu=name,driver_version",
                "--format=csv,noheader,nounits",
            ],
        ) {
            if let Some((name, driver)) = parse_nvidia_smi(&text) {
                profile.gpu = vendor_of_adapter(&name).or(Some(GpuVendor::Nvidia));
                profile.gpu_name = Some(name);
                profile.driver = driver;
            }
        }
    }

    profile
}

/// 缓存的本机画像：第一次 [`detect_cached`] 才真的起子进程探测，之后直接复用。
///
/// Windows 上探测要跑一两个 `powershell -Command Get-CimInstance …`（实测每个 0.9–1.5 s），
/// **绝不能出现在启动路径上**——那会把首屏拖成几秒黑屏。
static PROFILE: std::sync::OnceLock<HardwareProfile> = std::sync::OnceLock::new();

/// 只含 OS / 架构的画像：**纯内存判断，不起任何子进程**。
///
/// 「这个后端在本机能不能用」只看 OS + 架构（见 [`super::catalog::BackendEntry::platform_ok`]），
/// 所以启动时校验用户偏好用这份就够了；GPU 型号纯属展示信息。
pub fn platform_only() -> HardwareProfile {
    HardwareProfile::headless(std::env::consts::OS, std::env::consts::ARCH)
}

/// 缓存版 [`detect`]：首次调用才真的探测（可能耗时 1–3 s），之后返回同一份结果。
///
/// 只应在**后台线程**上调用，或先用 [`warm_in_background`] 热上。
pub fn detect_cached() -> &'static HardwareProfile {
    PROFILE.get_or_init(detect)
}

/// 后台预热缓存：启动时调一次，等设置页真要显示显卡型号时通常已经算好了。
pub fn warm_in_background() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("ai-daw-hw-detect".into())
            .spawn(|| {
                let _ = detect_cached();
            });
    });
}

/// 跑一个外部命令并返回 stdout（非零退出 / 找不到命令 → `None`）。
///
/// Windows 上必须带 `CREATE_NO_WINDOW`：打包后的宿主是 GUI 子系统、自己没有控制台，
/// 控制台子进程（powershell / wmic）会被系统**另开一个黑窗**，一闪而过。
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：不分配控制台窗口。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let out = command.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Windows：取显卡适配器名列表。
///
/// **优先 PowerShell + CIM**：`wmic` 从 Windows 11 24H2 起已被移除，只认它会让有独显的
/// 机器也报「未检测到」。老系统上 PowerShell 被策略挡住时再回退 `wmic`。
#[cfg(target_os = "windows")]
fn windows_adapters() -> Option<String> {
    run(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ProgressPreference='SilentlyContinue'; \
             Get-CimInstance Win32_VideoController | Select-Object -ExpandProperty Name",
        ],
    )
    .filter(|text| !text.trim().is_empty())
    .or_else(|| {
        run("wmic", &["path", "win32_VideoController", "get", "name"])
            .filter(|text| !text.trim().is_empty())
    })
}

/// Windows：读 NVIDIA 驱动版本（`DriverVersion`）。
///
/// 通配条件用单引号，避免命令里再套一层双引号（PowerShell 的命令行转义容易翻车）。
#[cfg(target_os = "windows")]
fn windows_nvidia_driver() -> Option<String> {
    let text = run(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$ProgressPreference='SilentlyContinue'; \
             Get-CimInstance Win32_VideoController \
             | Where-Object { $_.Name -like '*NVIDIA*' } \
             | ForEach-Object { $_.DriverVersion }",
        ],
    )
    .filter(|text| !text.trim().is_empty())
    .or_else(|| {
        run(
            "wmic",
            &[
                "path",
                "win32_VideoController",
                "where",
                "name like '%NVIDIA%'",
                "get",
                "DriverVersion",
            ],
        )
        .filter(|text| !text.trim().is_empty())
    })?;
    text.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.eq_ignore_ascii_case("DriverVersion"))
        .map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendor_detection_covers_the_common_adapters() {
        assert_eq!(
            vendor_of_adapter("NVIDIA GeForce RTX 4090"),
            Some(GpuVendor::Nvidia)
        );
        assert_eq!(vendor_of_adapter("Apple M4"), Some(GpuVendor::Apple));
        assert_eq!(
            vendor_of_adapter("AMD Radeon RX 7900 XTX"),
            Some(GpuVendor::Amd)
        );
        assert_eq!(
            vendor_of_adapter("Intel(R) UHD Graphics 770"),
            Some(GpuVendor::Intel)
        );
        assert_eq!(
            vendor_of_adapter("Microsoft Basic Display Adapter"),
            Some(GpuVendor::Other)
        );
        assert_eq!(vendor_of_adapter("something unrecognizable"), None);
    }

    #[test]
    fn parses_macos_chip_from_sysctl_and_system_profiler() {
        // `sysctl -n machdep.cpu.brand_string`
        assert_eq!(parse_macos_chip("Apple M4\n"), Some("Apple M4".into()));
        // `system_profiler SPDisplaysDataType`
        let text = "Graphics/Displays:\n\n    Apple M4:\n\n      Chipset Model: Apple M4\n      Type: GPU\n";
        assert_eq!(parse_macos_chip(text), Some("Apple M4".into()));
        assert_eq!(parse_macos_chip("\n\n"), None);
    }

    #[test]
    fn parses_nvidia_smi_first_gpu_and_driver() {
        let (name, driver) = parse_nvidia_smi("NVIDIA GeForce RTX 4090, 550.54.14\n").unwrap();
        assert_eq!(name, "NVIDIA GeForce RTX 4090");
        assert_eq!(driver.as_deref(), Some("550.54.14"));
        // 驱动拿不到时也不能失败。
        let (name, driver) = parse_nvidia_smi("Tesla T4, [N/A]\n").unwrap();
        assert_eq!(name, "Tesla T4");
        assert_eq!(driver, None);
        assert!(parse_nvidia_smi("  \n").is_none());
    }

    #[test]
    fn parses_windows_adapter_list_without_the_header() {
        let text = "Name\nNVIDIA GeForce RTX 4070\nIntel(R) UHD Graphics 770\n\n";
        assert_eq!(
            parse_windows_adapters(text),
            vec!["NVIDIA GeForce RTX 4070", "Intel(R) UHD Graphics 770"]
        );
    }

    /// 多显卡机器上要挑独显（NVIDIA 优先于 Intel 核显）。
    #[test]
    fn picks_the_discrete_gpu_over_the_integrated_one() {
        let adapters = vec![
            "Intel(R) UHD Graphics 770".to_string(),
            "NVIDIA GeForce RTX 4070".to_string(),
        ];
        let (vendor, name) = pick_primary(&adapters).unwrap();
        assert_eq!(vendor, GpuVendor::Nvidia);
        assert_eq!(name, "NVIDIA GeForce RTX 4070");
    }

    /// PowerShell/CIM 的输出没有表头，且常混着虚拟显示适配器（远程桌面 / 投屏）。
    #[test]
    fn parses_headerless_cim_adapter_list() {
        let text =
            "Todesk Virtual Display Adapter\nNVIDIA GeForce RTX 4080\nLuminonCore IDDCX Adapter\n";
        assert_eq!(
            parse_windows_adapters(text),
            vec![
                "Todesk Virtual Display Adapter",
                "NVIDIA GeForce RTX 4080",
                "LuminonCore IDDCX Adapter",
            ]
        );
        // 虚拟适配器只能算 `Other`，独显永远排在它前面。
        let (vendor, name) = pick_primary(&parse_windows_adapters(text)).unwrap();
        assert_eq!(vendor, GpuVendor::Nvidia);
        assert_eq!(name, "NVIDIA GeForce RTX 4080");
    }

    /// 探测本身绝不能 panic —— 无论本机是什么，都得给出一个画像。
    #[test]
    fn detect_always_returns_a_profile() {
        let profile = detect();
        assert_eq!(profile.os, std::env::consts::OS);
        assert_eq!(profile.arch, std::env::consts::ARCH);
    }
}
