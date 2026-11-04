//! 诊断用：列出本机输出设备、默认格式与 cpal 报告的 F32 支持区间。
//!
//! `cargo run -p ai-daw-engine --features realtime --example audio_probe`

use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    let default = host.default_output_device();
    let default_name = default.as_ref().and_then(|d| {
        d.description().ok().map(|desc| desc.name().to_owned())
    });
    println!("default = {default_name:?}");
    if let Some(device) = default.as_ref() {
        match device.default_output_config() {
            Ok(cfg) => println!(
                "  default config: {:?} {} ch @ {} Hz",
                cfg.sample_format(),
                cfg.channels(),
                cfg.sample_rate()
            ),
            Err(e) => println!("  default config error: {e}"),
        }
        match device.supported_output_configs() {
            Ok(ranges) => {
                for r in ranges {
                    println!(
                        "  range: {:?} {} ch {}..{} Hz",
                        r.sample_format(),
                        r.channels(),
                        r.min_sample_rate(),
                        r.max_sample_rate()
                    );
                }
            }
            Err(e) => println!("  supported configs error: {e}"),
        }
    }

    println!("\nall output devices:");
    if let Ok(devices) = host.output_devices() {
        for (i, device) in devices.enumerate() {
            let name = device
                .description()
                .ok()
                .map(|d| d.name().to_owned())
                .unwrap_or_default();
            let cfg = device
                .default_output_config()
                .map(|c| {
                    format!(
                        "{:?} {}ch@{}",
                        c.sample_format(),
                        c.channels(),
                        c.sample_rate()
                    )
                })
                .unwrap_or_else(|e| format!("ERR {e}"));
            println!("  [{i}] {name}  default={cfg}");
        }
    }
}
