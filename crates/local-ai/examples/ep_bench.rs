//! **执行后端对比基准**：同一份输入，分别在 CPU 与 CoreML 下跑各个真实模型，打印耗时。
//!
//! 用法（两次运行做对比）：
//! ```text
//! M="$HOME/Library/Application Support/com.aidaw.app/models"
//! AIDAW_MODELS="$M" AIDAW_BACKEND=cpu    AIDAW_BACKEND_TIMING=1 \
//!   cargo run --release -p ai-daw-local-ai --example ep_bench
//! AIDAW_MODELS="$M" AIDAW_BACKEND=coreml AIDAW_BACKEND_TIMING=1 \
//!   cargo run --release -p ai-daw-local-ai --example ep_bench
//! ```
//!
//! 注意：本仓库的模型层是「**一次推理建一次会话**」，所以「单次调用总耗时」里含建会话时间；
//! `AIDAW_BACKEND_TIMING=1` 会把建会话单独打出来，两者相减即为纯推理时间。

use std::time::Instant;

use ai_daw_local_ai::backend;
use ai_daw_local_ai::models;

/// 造一段立体声测试信号：三个正弦 + 一点噪声（避免退化成常量）。
fn tone(seconds: f64, rate: f64) -> Vec<f32> {
    let frames = (seconds * rate) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let mut noise: u32 = 0x1234_5678;
    for i in 0..frames {
        let t = i as f64 / rate;
        let l = (0.4 * (2.0 * std::f64::consts::PI * 220.0 * t).sin()
            + 0.25 * (2.0 * std::f64::consts::PI * 440.0 * t).sin())
            as f32;
        let r = (0.4 * (2.0 * std::f64::consts::PI * 330.0 * t).sin()
            + 0.25 * (2.0 * std::f64::consts::PI * 660.0 * t).sin()) as f32;
        noise = noise.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = ((noise >> 8) as f32 / 8_388_608.0 - 1.0) * 0.02;
        out.push(l + n);
        out.push(r + n);
    }
    out
}

/// 跑一段，打印耗时；`Err` 也照打（对比时重要的是不崩、以及失败原因）。
fn timed<T>(label: &str, run: impl FnOnce() -> Result<T, String>) -> Option<T> {
    let started = Instant::now();
    match run() {
        Ok(value) => {
            println!("  {:.<26} 总 {:>9.2?}", format!("{label} 单次调用"), started.elapsed());
            Some(value)
        }
        Err(e) => {
            println!("  {:.<26} 失败：{e}", label);
            None
        }
    }
}

fn main() {
    let kind = backend::init();
    println!("生效后端：{}", kind.label());
    println!();

    let rate = 44_100.0;
    let audio = tone(3.0, rate);

    if models::basic_pitch::model_path().is_some() {
        timed("Basic Pitch（3s）", || {
            models::basic_pitch::transcribe(&audio, 2, rate, &|_| true).map(|n| n.len())
        });
    }
    if models::localvqe::model_path().is_some() {
        timed("LocalVQE（3s）", || {
            models::localvqe::enhance(&audio, 2, rate, &|_| true).map(|v| v.len())
        });
    }
    if models::demucs::model_path().is_some() {
        timed("HTDemucs 4 分轨（3s）", || {
            models::demucs::separate(&audio, 2, rate, &|_| true).map(|s| s.stems.len())
        });
    }
    if models::demucs6s::model_path().is_some() {
        timed("HTDemucs 6 分轨（3s）", || {
            models::demucs6s::separate(&audio, 2, rate, &|_| true).map(|s| s.stems.len())
        });
    }
}
