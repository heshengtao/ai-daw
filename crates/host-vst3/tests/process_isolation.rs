//! 进程隔离的回归测试：用一个**已知会偶发崩溃**的真实插件验证「崩的是 helper，不是宿主」。
//!
//! 2026-10-20 的闪退（`ai-daw-app-2026-10-20-145023.ips`）是 `rdpiano_juce.vst3` 在
//! `RdPiano_juceAudioProcessor` 构造函数里 `SIGBUS`。这里反复用隔离模式加载它：
//!
//! - 插件若在加载时崩，`load_catalog_plugin_with_editor_mode(..., true)` 返回 `Err`；
//! - 插件若在处理时崩，适配器把该轨输出置零；
//! - **宿主进程无论如何都活着**——这就是本测试的断言。
//!
//! 没装插件 / 没构建 helper 时跳过（CI 上通常两者都没有）。

use std::path::{Path, PathBuf};

use ai_daw_host_vst3::{load_catalog_plugin_with_editor_mode, CatalogPlugin};
use ai_daw_plugin_api::{PluginKind, ProcessContext};

/// 构建脚本把 helper 放在 `src-tauri/binaries/vst3-host-helper-<triple>`。
fn helper_path() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/binaries");
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("vst3-host-helper-"))
        })
}

/// 测试插件：`AIDAW_TEST_VST3` 优先，否则用本机安装的 RdPiano。
fn crashy_plugin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AIDAW_TEST_VST3") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home).join(
        "Library/Application Support/com.aidaw.app/plugins/vst3/rdpiano_juce.vst3",
    );
    path.exists().then_some(path)
}

#[test]
fn isolated_load_survives_a_crashing_vst3() {
    let Some(helper) = helper_path() else {
        eprintln!("未构建 vst3-host-helper（先跑 tools/build-vst3-helper.sh），跳过。");
        return;
    };
    let Some(plugin_path) = crashy_plugin() else {
        eprintln!("未找到测试用 VST3（设置 AIDAW_TEST_VST3），跳过。");
        return;
    };
    // `vst3-host` 在 `PluginHostProcess::new` 里读这个变量；必须指向我们构建的 helper。
    std::env::set_var("VST3_HOST_HELPER_PATH", &helper);

    // 多跑几次：这个插件的崩溃是偶发的，单次可能刚好蒙混过关。
    let mut exercised = false;
    for i in 0..6 {
        let entry = CatalogPlugin {
            id: format!("vst3:isolation-test-{i}"),
            name: "Isolation Test".into(),
            vendor: "test".into(),
            kind: PluginKind::Instrument,
            format: "vst3".into(),
            accepts_midi: true,
            has_editor: false,
            path: plugin_path.clone(),
        };

        match load_catalog_plugin_with_editor_mode(&entry, true) {
            Ok((mut plugin, _)) => {
                exercised = true;
                plugin.prepare(48_000.0, 512);
                let ctx = ProcessContext {
                    sample_rate: 48_000.0,
                    block_size: 512,
                    tempo: 120.0,
                    playing: true,
                };
                let mut left = vec![0.0f32; 512];
                let mut right = vec![0.0f32; 512];
                // 插件内部崩了也只会让这一块全零；适配器不会把宿主带走。
                plugin.process(&ctx, &mut [&mut left, &mut right], &[]);
            }
            Err(err) => {
                exercised = true;
                let text = err.to_string();
                assert!(
                    !text.contains("Helper executable not found")
                        && !text.contains("helper unavailable"),
                    "隔离 helper 没跑起来，这个测试就没意义：{text}"
                );
                eprintln!("第 {i} 次隔离加载返回错误（宿主仍存活）：{text}");
            }
        }
    }

    assert!(exercised, "隔离路径一次都没跑起来");
}
