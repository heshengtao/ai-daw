//! 验证「专用插件线程」这个设计：实例和界面都在同一条**非主线程**上。
//!
//! 这是专用线程方案的前提——如果它也卡，那方案不成立。
//!
//! ```text
//! cargo run -p ai-daw-host-vst3 --example editor_offthread
//! ```

use std::io::Write;
use std::time::{Duration, Instant};

fn say(message: impl AsRef<str>) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "{}", message.as_ref());
    let _ = err.flush();
}

fn main() {
    ai_daw_host_vst3::mark_main_thread();
    say(format!("主线程 = {:?}", std::thread::current().id()));

    let handle = std::thread::Builder::new()
        .name("plugin-host-probe".into())
        .spawn(|| {
            say(format!(
                "  插件线程 = {:?}（加载 + 开界面都在这里）",
                std::thread::current().id()
            ));
            let dir = std::env::args()
                .nth(1)
                .map(std::path::PathBuf::from)
                .or_else(|| {
                    // 默认看应用托管目录（插件管理器装进来的插件都在这里）。
                    std::env::var_os("APPDATA").map(|base| {
                        std::path::PathBuf::from(base).join("com.aidaw.app/plugins/vst3")
                    })
                })
                .unwrap_or_default();
            let catalog = ai_daw_host_vst3::scan_catalog_with(&[dir], &[], &[]);
            let Some(entry) = catalog.iter().find(|e| e.name.contains("RdPiano")) else {
                say("  找不到 RdPiano");
                return;
            };

            let load_at = Instant::now();
            let (_plugin, handle) = match ai_daw_host_vst3::load_catalog_plugin_with_editor(entry) {
                Ok((plugin, Some(handle))) => (plugin, handle),
                _ => {
                    say("  加载失败");
                    return;
                }
            };
            say(format!("  加载完成，耗时 {:?}", load_at.elapsed()));
            std::thread::sleep(Duration::from_millis(300));

            say("  → handle.open()（与加载同线程）");
            let open_at = Instant::now();
            match handle.open("RdPiano offthread", None, None) {
                Ok(window) => {
                    say(format!("  ← open() 返回 Ok，耗时 {:?}", open_at.elapsed()));
                    // 这条线程没有消息循环，只验证「能不能开出来」。
                    std::thread::sleep(Duration::from_secs(2));
                    window.close();
                    say("  已关闭界面");
                }
                Err(error) => say(format!("  ← open() 返回 Err({error})")),
            }
        })
        .expect("起插件线程");

    let _ = handle.join();
    say("done");
}
