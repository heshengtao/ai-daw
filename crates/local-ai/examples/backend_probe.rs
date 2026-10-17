//! **执行后端探针**：打印本机硬件画像与生效后端，并逐个模型建会话 + 计时。
//!
//! 用途：
//! 1. 确认「推荐后端」在本机是否符合预期（`AIDAW_BACKEND=cpu|coreml|directml|cuda` 可覆盖）；
//! 2. 逐个图确认加速真的**建得上**（EP 注册失败会打印回落原因）；
//! 3. 对同一模型比较 `AIDAW_BACKEND=cpu` 与默认的建会话耗时。
//!
//! ```text
//! AIDAW_MODELS="$HOME/Library/Application Support/com.aidaw.app/models" \
//!   cargo run --release -p ai-daw-local-ai --example backend_probe
//! ```

use std::time::Instant;

use ai_daw_local_ai::backend;
use ai_daw_local_ai::backend::session::GraphRole;
use ai_daw_local_ai::models;

fn main() {
    let profile = backend::detect::detect();
    println!("硬件：{} / {}", profile.os, profile.arch);
    println!(
        "GPU ：{}（厂商 {:?}）驱动 {:?}",
        profile.gpu_name.as_deref().unwrap_or("未检测到"),
        profile.gpu,
        profile.driver
    );
    println!("本机可用：{:?}", backend::catalog::supported(&profile));

    let kind = backend::init();
    println!("生效后端：{}", kind.label());
    if let Some(entry) = backend::catalog::by_kind(kind) {
        println!("说明：{}", entry.note);
    }
    println!();

    let mut graphs: Vec<(GraphRole, std::path::PathBuf)> = Vec::new();
    let mut push = |role: GraphRole, path: Option<std::path::PathBuf>| {
        if let Some(path) = path {
            graphs.push((role, path));
        }
    };
    push(GraphRole::BasicPitch, models::basic_pitch::model_path());
    push(GraphRole::LocalVqe, models::localvqe::model_path());
    push(GraphRole::Demucs, models::demucs::model_path());
    push(GraphRole::Demucs6s, models::demucs6s::model_path());

    if graphs.is_empty() {
        eprintln!("一个权重都没找到（用 AIDAW_MODELS 指定 models 目录）");
        std::process::exit(1);
    }

    let mut failed = 0;
    // `--io`：把每张图的输入 / 输出契约打出来（回答「这个导出带没带 KV cache」这类问题）。
    let show_io = std::env::args().any(|a| a == "--io");
    for (role, path) in &graphs {
        let mb = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        let started = Instant::now();
        match backend::session::build(path, *role) {
            Ok(session) => {
                println!("  ✓ {:<12}（{mb:>7.1} MB）建会话 {:.2?}", role.label(), started.elapsed());
                if show_io {
                    for input in session.inputs() {
                        println!("        ← {}  {:?}", input.name(), input.dtype());
                    }
                    for output in session.outputs() {
                        println!("        → {}  {:?}", output.name(), output.dtype());
                    }
                }
            }
            Err(e) => {
                failed += 1;
                println!("  ✗ {:<12}（{mb:>7.1} MB）{e}", role.label());
            }
        }
    }
    println!("\n{} 张图，{failed} 张失败", graphs.len());
    if failed > 0 {
        std::process::exit(1);
    }

    let availability = models::availability();
    if availability.any() {
        println!("\n已安装模型：");
        for (name, present) in [
            ("Basic Pitch", availability.basic_pitch.is_some()),
            ("LocalVQE", availability.localvqe.is_some()),
            ("HTDemucs", availability.htdemucs.is_some()),
            ("Beat This!", availability.beat_this.is_some()),
        ] {
            if present {
                println!("  · {name}");
            }
        }
    }
}
