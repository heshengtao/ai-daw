//! 扫描本机 VST3 / CLAP 插件并打印清单。
//!
//! 用法：`cargo run -p ai-daw-host-vst3 --example scan_demo`

use ai_daw_host_vst3::scan;

fn main() {
    let plugins = scan();
    println!("扫描到 {} 个插件：", plugins.len());
    for p in &plugins {
        println!(
            "  [{:5}] {:24} {:16} {:?}  {}",
            p.format,
            p.name,
            p.vendor,
            p.kind,
            p.path.display()
        );
    }
}
