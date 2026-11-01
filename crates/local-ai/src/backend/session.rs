//! **统一的建会话入口**：所有 `ort` 模型都从这里建会话，于是「所有能加速的模型都支持加速」
//! 与「EP 注册失败自动回落 CPU」这两件事只有一处实现。
//!
//! 两层回落：
//! 1. **硬回落**（这里）：EP 注册失败 / 建会话整体失败 → 重试纯 CPU，并打印原因。
//!    这一定是环境问题（该 EP 不在本构建里、驱动缺失、Metal 不可用…），绝不让它拖垮推理。
//! 2. **软回落**（ORT 内建）：CoreML / DirectML 都是 **partitioning EP**——它们不支持的算子
//!    会留在 CPU 上、会话照常建成。所以「会话建成」≠「加速生效」，实测见 spec。
//!
//! **per-graph 排除**：有些图在某个 EP 下会**编译失败或直接崩**，只能按图处理。
//! 编译失败的（如 6 分轨在 CoreML 上）由 [`UNSUPPORTED`] 运行时学习后跳过；
//! 实测会**在推理中崩溃**的图必须事先写死排除（崩溃兜不住）。见 [`coreml_allowed`]。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use ort::session::Session;

use super::kind::BackendKind;

/// CoreML 编译产物缓存目录（宿主可覆盖；缺省落在系统临时目录）。
///
/// **必须有**：CoreML EP 默认**每次建会话都把模型重新编译一遍**（官方文档明说「caching is not
/// enabled by default → the model will be compiled and saved to disk on each instantiation」）。
/// 本仓库的模型层是「一次推理建一次会话」，不缓存的话每次调用都要重编译——实测 LocalVQE
/// 一次 36 s，完全不可接受。给了缓存目录之后第二次起就只读缓存。
static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 宿主设定编译产物缓存目录（宿主启动时调一次，通常放应用配置目录下的 `runtime/coreml/`）。
pub fn set_coreml_cache_dir(dir: PathBuf) {
    let _ = CACHE_DIR.set(dir);
}

/// 按图分桶的缓存目录：`<base>/<role>`。
///
/// 分桶是有意的——EP 自己无法保证「结构相同的两张图」不互相复用编译产物（q4/q8 这类只换
/// 量化方式的图尤其可疑），按角色隔离能把误命中的面缩到最小。
#[cfg(target_os = "macos")]
fn coreml_cache_dir(role: GraphRole) -> PathBuf {
    let base = CACHE_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::temp_dir().join("ai-daw-coreml"));
    base.join(role.cache_key())
}

/// 一个 ONNX 图在流水线里的**角色**。用来做 per-graph 的后端策略（排除表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRole {
    /// Basic Pitch（音频 → MIDI）。
    BasicPitch,
    /// LocalVQE（降噪 / 去混响）。
    LocalVqe,
    /// HTDemucs 4 分轨。
    Demucs,
    /// HTDemucs 6 分轨。
    Demucs6s,
    /// SoulX-Singer 歌声合成（9 张图共用，见 [`crate::singing`]）。
    Svs,
    /// SoulX-Singer-SVC 音色替换（7 张图共用，见 [`crate::svc`]）。
    Svc,
}

impl GraphRole {
    /// 展示名（给日志与「加载 xx 权重失败」用）。
    pub fn label(self) -> &'static str {
        match self {
            GraphRole::BasicPitch => "Basic Pitch",
            GraphRole::LocalVqe => "LocalVQE",
            GraphRole::Demucs => "音源分离",
            GraphRole::Demucs6s => "6 分轨",
            GraphRole::Svs => "歌声合成",
            GraphRole::Svc => "音色替换",
        }
    }

    /// 稳定的 ASCII 键（给目录名用；`label()` 是中文，不能直接当路径）。
    pub fn cache_key(self) -> &'static str {
        match self {
            GraphRole::BasicPitch => "basic-pitch",
            GraphRole::LocalVqe => "localvqe",
            GraphRole::Demucs => "demucs",
            GraphRole::Demucs6s => "demucs6s",
            GraphRole::Svs => "svs",
            GraphRole::Svc => "svc",
        }
    }

    /// 在「已失败」位图里的位号（[`UNSUPPORTED`] 用）。
    fn bit(self) -> u32 {
        self as u32
    }
}

/// 「CoreML 建不起来」的图（**运行时学习**）：某张图第一次注册/编译失败后就记下来，后续
/// 直接走 CPU，不再白试一次。
///
/// 为什么不写死一张名单：哪些图能编译取决于 ONNX Runtime / CoreML 版本，写死的名单会过时。
/// 实测（2026-10-16，M4）**6 分轨**（`htdemucs_6s.onnx`）在 CoreML 上编译报
/// `Espresso … generic_general_slice: Invalid values 2048 in begin_ids` —— 每次都要白等十几秒。
/// 学习之后第二次起直接 CPU。
static UNSUPPORTED: AtomicU32 = AtomicU32::new(0);

fn marked_unsupported(role: GraphRole) -> bool {
    UNSUPPORTED.load(Ordering::Relaxed) & (1 << role.bit()) != 0
}

fn mark_unsupported(role: GraphRole) {
    UNSUPPORTED.fetch_or(1 << role.bit(), Ordering::Relaxed);
}

/// 这张图在 CoreML 下是否**允许**走加速。
///
/// 排除来源：
/// 1. **实测建不起来的**（运行时学习，见 [`UNSUPPORTED`]）：如 6 分轨每试必编译失败——
///    第一次失败后就记下来，后续直接 CPU，不再白等一次编译。
/// 2. 将来若有「建会话成功、但推理中崩溃」的图，必须在这里**写死**排除：
///    崩溃发生在推理中，进程级的「建会话失败回落」兜不住它。
pub fn coreml_allowed(role: GraphRole) -> bool {
    !marked_unsupported(role)
}

/// 这张图在 CUDA 下是否**允许**走加速（与 CoreML 共用同一张「学到的失败」表）。
pub fn cuda_allowed(role: GraphRole) -> bool {
    !marked_unsupported(role)
}
///
/// 建会话在 CoreML 下**不是免费的**（要编译模型），而本仓库的模型层是「一次推理建一次会话」
/// ——想知道某个模型慢在哪，先看这个。
/// 计时开关（`AIDAW_BACKEND_TIMING=1`）：把每次建会话的耗时打到 stderr。
///
/// 建会话在 CoreML 下**不是免费的**（要编译模型），而本仓库的模型层是「一次推理建一次会话」
/// ——想知道某个模型慢在哪，先看这个。
fn timing_on() -> bool {
    std::env::var_os("AIDAW_BACKEND_TIMING").is_some()
}

/// 本次任务里「建在加速器上 / 回落 CPU」的会话数。
///
/// 用户看到的「GPU 利用率没涨」几乎总是这个比值的答案：图全建在 CPU 上。
/// 把计数交给任务收尾打进日志与摘要，省得靠猜。
static ON_ACCELERATOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static ON_CPU: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 最近一次回落的原因（图名 + 错误）。
static LAST_FALLBACK: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// 最近一次「为什么回落 CPU」的一句话（含图名与原因）。
///
/// 「GPU 利用率不涨」时，这句话就是答案；它会出现在任务摘要里，也写进 `backend.log`。
pub fn last_fallback() -> Option<String> {
    LAST_FALLBACK.lock().ok().and_then(|slot| slot.clone())
}

/// 取走并清零计数（一次任务结束时调用一次）。
pub fn take_session_stats() -> (usize, usize) {
    use std::sync::atomic::Ordering;
    let stats = (
        ON_ACCELERATOR.swap(0, Ordering::Relaxed),
        ON_CPU.swap(0, Ordering::Relaxed),
    );
    if let Ok(mut slot) = LAST_FALLBACK.lock() {
        *slot = None;
    }
    stats
}

/// 建会话（唯一的入口）。
///
/// 先确保运行时已加载（`load-dynamic`：dylib 不在就**返回错误**，绝不让 `ort` 走它自己的
/// 默认路径 panic）；再按当前后端注册 EP，**失败回落 CPU**。
pub fn build(path: &Path, role: GraphRole) -> Result<Session, String> {
    // 运行时是进程级一次性加载：没装就明确报错，别让模型层用 panic 表达。
    crate::backend::runtime::ensure_loaded()?;
    let kind = super::backend();
    let started = std::time::Instant::now();
    let result = build_for(kind, path, role);
    if timing_on() {
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr(),
            "  [timing] {:<12} {:<8} 建会话 {:>9.2?} → {}",
            role.label(),
            kind.label(),
            started.elapsed(),
            if result.is_ok() { "ok" } else { "失败" }
        );
    }
    result
}

fn build_for(kind: BackendKind, path: &Path, role: GraphRole) -> Result<Session, String> {
    use std::sync::atomic::Ordering;
    let accelerate = match kind {
        BackendKind::Cpu => false,
        BackendKind::CoreMl => coreml_allowed(role),
        BackendKind::DirectMl => true,
        // CUDA 还要额外确认「EP 库注册成功」：没注册时 ORT 会静默把会话建在 CPU 上，
        // 只按 `cuda_allowed` 判断会把 CPU 会话当成加速器（以前就是这样误报的）。
        // 同时校验大版本：选了 13 但进程里注册的还是 12 的 provider 时，本次会话
        // 不该假装在跑 13（重启后才生效，设置页会提示）。
        BackendKind::Cuda12 => cuda_allowed(role) && crate::backend::runtime::cuda_ep_ready_for(12),
        BackendKind::Cuda13 => cuda_allowed(role) && crate::backend::runtime::cuda_ep_ready_for(13),
    };
    if !accelerate {
        ON_CPU.fetch_add(1, Ordering::Relaxed);
        log_build(path, role, kind, "CPU（这个角色已标记为该后端不可用）");
        return build_cpu(path, role);
    }
    // 两步分开处理，因为「EP 注册不上」和「权重读不进来」不是一回事：
    // 前者是环境问题（驱动 / cuDNN / provider），值得记住、后续直接 CPU；
    // 后者是这张图 / 这次调用的毛病（路径不对、文件损坏、显存不够），**不该**把整个
    // 角色的 CUDA 永久关掉——以前一有这种错误就 `mark_unsupported`，表现就是
    // 「第一次失败之后，后续全部静默走 CPU，GPU 利用率一直为 0」。
    let builder = match Session::builder() {
        Ok(builder) => builder,
        Err(e) => {
            let msg = crate::models::err("初始化 ONNX Runtime 失败", e);
            return Err(msg);
        }
    };
    let builder = match kind {
        BackendKind::CoreMl => register_coreml(builder, role),
        BackendKind::DirectMl => register_directml(builder),
        BackendKind::Cuda12 | BackendKind::Cuda13 => register_cuda(builder),
        BackendKind::Cpu => Ok(builder),
    };
    let mut builder = match builder {
        Ok(builder) => builder,
        Err(e) => {
            // **只有 CoreML 值得记**：它失败一次要白等十几秒编译。
            // CUDA 不记：注册失败的代价只有几十毫秒，而「记下来」会让一次偶然失败
            // （显存被占、驱动正在重置、provider 一时没找到）把**整个进程**的这个角色
            // 永久钉在 CPU 上——界面看不出任何异常，只是 GPU 利用率永远为 0。
            if matches!(kind, BackendKind::CoreMl) {
                mark_unsupported(role);
            }
            ON_CPU.fetch_add(1, Ordering::Relaxed);
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let line = format!(
                "[backend] {file}（{}）：{} 不可用（{e}），这张图回落 CPU EP（本任务后续同类图也走 CPU）",
                role.label(),
                kind.label()
            );
            remember_fallback(&file, &e);
            crate::backend::logfile::note(&line);
            return build_cpu(path, role);
        }
    };
    match builder.commit_from_file(path) {
        Ok(session) => {
            ON_ACCELERATOR.fetch_add(1, Ordering::Relaxed);
            log_build(path, role, kind, "加速器");
            Ok(session)
        }
        Err(e) => {
            // 权重加载失败：记下原因（摘要里能看到），但**不**禁用这个角色的加速。
            ON_CPU.fetch_add(1, Ordering::Relaxed);
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let text = format!("加载{}权重失败：{e}", role.label());
            crate::backend::logfile::note(&format!(
                "[backend] {file}（{}）：{} 上加载失败（{text}），这张图回落 CPU EP",
                role.label(),
                kind.label()
            ));
            remember_fallback(&file, &text);
            build_cpu(path, role)
        }
    }
}

/// 每建一张会话就写一行（`backend.log`）：图名、角色、落在哪。
///
/// 「GPU 利用率怎么不涨」这类问题，以前只能靠猜：成功不吭声、失败只吭一声且被刷掉。
/// 现在一次任务 7 张图就是 7 行，谁在哪、为什么，一眼能看出来。
fn log_build(path: &Path, role: GraphRole, kind: BackendKind, where_: &str) {
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    crate::backend::logfile::note(&format!(
        "[backend] 建会话 {file}（{}）→ {where_}（后端 {}）",
        role.label(),
        kind.label()
    ));
}

/// 记住「第一张没上加速器的图 + 原因」，任务摘要里会带上它。
fn remember_fallback(file: &str, reason: &str) {
    if let Ok(mut slot) = LAST_FALLBACK.lock() {
        if slot.is_none() {
            *slot = Some(format!("{file}：{reason}"));
        }
    }
}

/// 纯 CPU EP 建会话（回落的目标，也是现状行为）。
fn build_cpu(path: &Path, role: GraphRole) -> Result<Session, String> {
    Session::builder()
        .map_err(|e| crate::models::err("初始化 ONNX Runtime 失败", e))?
        .commit_from_file(path)
        .map_err(|e| format!("加载{}权重失败：{e}", role.label()))
}

/// CoreML 的计算单元：默认 **CPUAndNeuralEngine**（对齐 Phase 0 的可用配置）。
///
/// `All`（ANE+GPU）下 GPU 那条路会连文本编码器都崩，所以默认避开 GPU；需要时用
/// `AIDAW_COREML_UNITS=all|ane|gpu|cpu` 覆盖（排障旋钮）。
#[cfg(target_os = "macos")]
fn coreml_units() -> ort::ep::coreml::ComputeUnits {
    use ort::ep::coreml::ComputeUnits;
    match std::env::var("AIDAW_COREML_UNITS").as_deref() {
        Ok("all") => ComputeUnits::All,
        Ok("gpu") => ComputeUnits::CPUAndGPU,
        Ok("cpu") => ComputeUnits::CPUOnly,
        _ => ComputeUnits::CPUAndNeuralEngine,
    }
}

#[cfg(target_os = "macos")]
fn register_coreml(
    builder: ort::session::builder::SessionBuilder,
    role: GraphRole,
) -> Result<ort::session::builder::SessionBuilder, String> {
    use ort::ep::CoreML;
    // 缓存目录必须给：否则每次建会话都把 CoreML 模型重编译一遍（LocalVQE 实测 36 s）。
    let ep = CoreML::default()
        .with_compute_units(coreml_units())
        .with_model_cache_dir(coreml_cache_dir(role).to_string_lossy().into_owned())
        .build();
    builder
        .with_execution_providers([ep])
        .map_err(|e| crate::models::err("注册 CoreML 执行后端失败", e))
}

#[cfg(not(target_os = "macos"))]
fn register_coreml(
    _builder: ort::session::builder::SessionBuilder,
    _role: GraphRole,
) -> Result<ort::session::builder::SessionBuilder, String> {
    Err("CoreML 仅 macOS 支持".into())
}

#[cfg(target_os = "windows")]
fn register_directml(
    builder: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, String> {
    use ort::ep::DirectML;
    builder
        .with_execution_providers([DirectML::default().build()])
        .map_err(|e| crate::models::err("注册 DirectML 执行后端失败", e))
}

#[cfg(not(target_os = "windows"))]
fn register_directml(
    _builder: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, String> {
    Err("DirectML 仅 Windows 支持".into())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn register_cuda(
    builder: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, String> {
    use ort::ep::CUDA;
    builder
        .with_execution_providers([CUDA::default().build()])
        .map_err(|e| crate::models::err("注册 CUDA 执行后端失败", e))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn register_cuda(
    _builder: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, String> {
    Err("CUDA 仅 Windows / Linux x86_64 支持".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `UNSUPPORTED` 是进程级静态量；下面两条用例都会改它，必须串行跑。
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn locked() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 默认所有角色都允许走 CoreML；被学习进「不支持」集合的才被排除。
    #[test]
    fn coreml_is_allowed_by_default() {
        let _guard = locked();
        // 这个静态量是进程级的：先把「学习到的失败」清空，用例才不依赖跑过哪些用例。
        UNSUPPORTED.store(0, Ordering::Relaxed);
        assert!(coreml_allowed(GraphRole::BasicPitch));
        assert!(coreml_allowed(GraphRole::LocalVqe));
        assert!(coreml_allowed(GraphRole::Demucs));
        assert!(coreml_allowed(GraphRole::Demucs6s));
    }

    #[test]
    fn every_role_has_a_label_and_cache_key() {
        for role in [
            GraphRole::BasicPitch,
            GraphRole::LocalVqe,
            GraphRole::Demucs,
            GraphRole::Demucs6s,
            GraphRole::Svs,
            GraphRole::Svc,
        ] {
            assert!(!role.label().is_empty());
            // cache_key 要能当目录名：纯 ASCII、无空格。
            assert!(role
                .cache_key()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-'));
        }
    }

    /// 运行时学习：某张图 CoreML 失败一次后，后续不再尝试（但仍不影响别的图）。
    #[test]
    fn unsupported_graphs_are_learned_once() {
        let _guard = locked();
        assert!(coreml_allowed(GraphRole::Demucs6s));
        mark_unsupported(GraphRole::Demucs6s);
        assert!(!coreml_allowed(GraphRole::Demucs6s));
        assert!(coreml_allowed(GraphRole::Demucs));
    }
}
