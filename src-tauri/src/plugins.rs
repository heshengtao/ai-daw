//! 插件目录 → 注册表的装配。
//!
//! 扫描很慢，所以目录在启动时后台扫描一次、并缓存到磁盘；音频线程按 id 懒加载插件。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ai_daw_engine::model::Project;
use ai_daw_engine::registry::{missing_plugin, PluginRegistry, PluginSlot};
use ai_daw_host_vst3::{load_catalog_plugin_with_editor_mode, CatalogPlugin};

use crate::audio::EditorSlot;

/// 离线渲染期间收集到的问题（目前只有外部插件加载失败）。
///
/// 渲染不能因为一个坏插件就整首失败：工厂退回静音占位，把原因记在这里，调用方（MCP /
/// 导出 / 预览）再决定怎么呈现。
pub type LoadWarnings = Arc<Mutex<Vec<String>>>;

/// 依据扫描目录构建注册表：内置合成器 + 外部插件工厂 + 缺失占位。
///
/// 外部插件工厂在加载失败时退回静音占位，绝不让一条坏轨拖垮整首曲子；
/// 创建实例时顺带把「(轨道, 槽位) → 编辑器句柄」登记进 `editor_handles`，供界面打开原生窗口。
///
/// 实时路径用这个版本：VST3 在进程内加载，处理器不承担跨进程 IPC。
pub fn build_registry(
    catalog: &[CatalogPlugin],
    project: &Project,
    editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>>,
) -> PluginRegistry {
    build_registry_with(catalog, project, editor_handles, false, None)
}

/// 离线渲染用的注册表：VST3 走 helper 子进程加载，崩溃只杀掉 helper。
///
/// 为什么只对离线路径开启：隔离路径每个音频块都要一次 JSON + base64 的管道往返，实时回调
/// 付不起这个开销。离线渲染（预览 / 导出 / 分析）没有硬实时预算，换来的是「插件崩了应用
/// 还在」。
pub fn build_registry_for_offline_render(
    catalog: &[CatalogPlugin],
    project: &Project,
    editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>>,
    warnings: LoadWarnings,
) -> PluginRegistry {
    build_registry_with(catalog, project, editor_handles, true, Some(warnings))
}

fn build_registry_with(
    catalog: &[CatalogPlugin],
    project: &Project,
    editor_handles: Arc<Mutex<HashMap<(u64, PluginSlot), EditorSlot>>>,
    isolate_vst3: bool,
    warnings: Option<LoadWarnings>,
) -> PluginRegistry {
    let mut registry = PluginRegistry::with_builtins();

    for entry in catalog {
        let id = entry.id.clone();
        let fallback_id = id.clone();
        let display_name = entry.name.clone();
        let entry = entry.clone();
        let plugin_id = id.clone();
        let handles = Arc::clone(&editor_handles);
        let warnings = warnings.clone();
        registry.register(&id, move |track, slot| {
            match load_catalog_plugin_with_editor_mode(&entry, isolate_vst3) {
                Ok((plugin, Some(handle))) => {
                    handles.lock().unwrap().insert(
                        (track.0, slot),
                        EditorSlot {
                            handle,
                            plugin_id: plugin_id.clone(),
                        },
                    );
                    plugin
                }
                Ok((plugin, None)) => plugin,
                Err(err) => {
                    // 一个坏插件只让**这条轨道**静音；把原因留给调用方（MCP 会写进分析结果）。
                    if let Some(warnings) = &warnings {
                        warnings.lock().unwrap().push(format!(
                            "插件「{display_name}」（{fallback_id}）加载失败，已按静音处理：{err}"
                        ));
                    }
                    missing_plugin(&fallback_id)
                }
            }
        });
    }

    // 工程引用了、但本机目录里没有的插件 → 静音占位（否则该轨会被静默跳过）。
    let known: HashSet<String> = registry.ids().into_iter().collect();
    for id in referenced_plugin_ids(project) {
        if !known.contains(&id) {
            registry.register_missing(&id);
        }
    }

    registry
}

/// 工程里被引用到的插件 id（乐器 + 效果链），去重升序。
pub fn referenced_plugin_ids(project: &Project) -> Vec<String> {
    let mut out = Vec::new();
    for track in &project.tracks {
        if let Some(instrument) = &track.instrument {
            out.push(instrument.plugin_id.clone());
        }
        for effect in &track.effects {
            out.push(effect.plugin_id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 工程引用的插件里，有没有**外部插件**（目录里的 VST3 / CLAP / AU）。
///
/// 用来决定离线渲染前要不要「等一会儿」（见 `commands::EXTERNAL_PLUGIN_SETTLE_SECONDS`）：
/// 外部插件（尤其是采样流式音源）在 `prepare` 之后还要异步装载素材，这期间只出静音，而离线
/// 渲染快得多，会把这段静音原样录成「开头几个小节没声音」。
///
/// 内置插件是纯 DSP、没有可装载的东西，所以内置插件工程**不等**——预览与 AI 侧的
/// `analyze_project` 会被反复调用，这个等待不能白付。
pub fn uses_external_plugins(project: &Project, catalog: &[CatalogPlugin]) -> bool {
    if catalog.is_empty() {
        return false;
    }
    let external: HashSet<&str> = catalog.iter().map(|entry| entry.id.as_str()).collect();
    referenced_plugin_ids(project)
        .iter()
        .any(|id| external.contains(id.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    use ai_daw_engine::bus::Engine;
    use ai_daw_engine::command::Command;
    use ai_daw_engine::event::{Actor, Event};
    use ai_daw_engine::model::Note;
    use ai_daw_engine::render::{render, RenderOptions};
    use ai_daw_host_vst3::MainThreadDispatch;

    /// 假主线程上执行过的任务数（证明 CLAP 的初始化确实被派到主线程了）。
    static MAIN_THREAD_TASKS: AtomicUsize = AtomicUsize::new(0);

    /// 假主线程：CLAP 的 `init` / 参数枚举 / `activate` 都必须落在它上面。
    struct FakeMainThread {
        tx: mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>,
    }

    impl MainThreadDispatch for FakeMainThread {
        fn dispatch(&self, task: Box<dyn FnOnce() + Send + 'static>) -> bool {
            let (done_tx, done_rx) = mpsc::channel();
            let wrapped = Box::new(move || {
                task();
                MAIN_THREAD_TASKS.fetch_add(1, Ordering::SeqCst);
                let _ = done_tx.send(());
            });
            if self.tx.send(wrapped).is_err() {
                return false;
            }
            done_rx.recv().is_ok()
        }
    }

    fn install_fake_main_thread() {
        static INSTALLED: std::sync::Once = std::sync::Once::new();
        INSTALLED.call_once(|| {
            let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
            let (ready_tx, ready_rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("fake-main-thread".into())
                .spawn(move || {
                    ai_daw_host_vst3::mark_main_thread();
                    let _ = ready_tx.send(());
                    while let Ok(task) = rx.recv() {
                        task();
                    }
                })
                .expect("起假主线程");
            ready_rx.recv().expect("假主线程必须完成登记");
            ai_daw_host_vst3::set_main_thread_dispatch(Arc::new(FakeMainThread { tx }));
        });
    }

    /// 端到端：把 KR-106 的 **CLAP** 版挂到一条轨道上，离线渲染整首曲子。
    ///
    /// 走的就是应用里真实那条路径（注册表 → 加载插件 → 激活 → 逐块处理），只是没有 GUI。
    /// 它同时验证两件事：线程规则改过之后引擎整链路仍然出声；CLAP 的初始化确实被派发到了
    /// 主线程（否则应用里就会是那个「越线程 + 卡死」的老样子）。
    #[test]
    fn renders_a_track_hosting_the_clap_kr106() {
        // 先装派发器再扫描：扫描阶段也会实例化插件问它有没有界面。
        install_fake_main_thread();
        let catalog = ai_daw_host_vst3::scan_catalog();
        let Some(kr106) = catalog
            .iter()
            .find(|p| p.format == "clap" && p.name.contains("KR-106"))
        else {
            eprintln!("本机未安装 KR-106 的 clap 版本，跳过。");
            return;
        };

        let mut engine = Engine::new("clap-e2e");
        let events = engine.execute(Command::AddTrack { name: "KR".into() }, Actor::Human);
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!("AddTrack 应当产出 TrackAdded"),
        };
        engine.execute(
            Command::SetTrackInstrument {
                track,
                plugin_id: kr106.id.clone(),
            },
            Actor::Human,
        );
        let events = engine.execute(
            Command::AddClip {
                track,
                start: 0.0,
                length: 2.0,
            },
            Actor::Human,
        );
        let clip = match events[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!("AddClip 应当产出 ClipAdded"),
        };
        engine.execute(
            Command::AddNote {
                track,
                clip,
                note: Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch: 60,
                    start: 0.0,
                    duration: 1.0,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: ai_daw_engine::model::PitchExpression::NONE,
                },
            },
            Actor::Human,
        );
        let project = engine.project().clone();

        let handles = Arc::new(Mutex::new(HashMap::new()));
        let registry = build_registry(&catalog, &project, Arc::clone(&handles));
        let options = RenderOptions {
            sample_rate: 44_100.0,
            tail_seconds: 0.5,
            settle_seconds: 0.0,
        };
        let buffer = render(&project, &registry, &options);

        assert!(
            MAIN_THREAD_TASKS.load(Ordering::SeqCst) > 0,
            "CLAP 的加载与激活必须派发到主线程执行"
        );
        assert!(
            buffer.peak() > 0.01,
            "挂 CLAP 插件的轨道必须出声（峰值 {}）",
            buffer.peak()
        );
    }

    /// 「离线渲染前要不要等一会儿」的判定：工程引用了**目录里的**外部插件才等。
    ///
    /// 这是「导出开头几个小节没声音」那个 bug 的修复开关（见 `commands::EXTERNAL_PLUGIN_SETTLE_SECONDS`），
    /// 判定一旦失效就会静默复发，所以钉住它：内置插件不等（预览 / `analyze_project` 会被反复调用，
    /// 不能白等）、缺失插件也不等（占位是透传的，没有东西在装载）。
    #[test]
    fn external_plugins_trigger_the_settle_wait() {
        use ai_daw_engine::model::{PluginInstance, Track, TrackId};
        use ai_daw_host_vst3::CatalogPlugin;
        use ai_daw_plugin_api::PluginKind;

        let catalog = vec![CatalogPlugin {
            id: "vst3:cool-synth".into(),
            name: "Cool Synth".into(),
            vendor: "ACME".into(),
            kind: PluginKind::Instrument,
            format: "vst3".into(),
            accepts_midi: true,
            has_editor: true,
            description: None,
            path: std::path::PathBuf::from("/tmp/CoolSynth.vst3"),
        }];

        let with_instrument = |plugin_id: &str| {
            let mut project = Project::new("p");
            project.tracks.push(Track {
                meta: None,
                id: TrackId(1),
                name: "T".into(),
                instrument: Some(PluginInstance {
                    plugin_id: plugin_id.into(),
                    params: Default::default(),
                    state: vec![],
                }),
                effects: vec![],
                clips: vec![],
                mixer: Default::default(),
                sends: vec![],
                automation: vec![],
            });
            project
        };

        assert!(
            uses_external_plugins(&with_instrument("vst3:cool-synth"), &catalog),
            "引用了目录里的外部插件 → 必须等（采样流式音源要时间装载）"
        );
        assert!(
            !uses_external_plugins(&with_instrument("builtin.synth.lead"), &catalog),
            "内置插件是纯 DSP → 不等"
        );
        assert!(
            !uses_external_plugins(&with_instrument("vst3:not-installed"), &catalog),
            "本机没有的插件只是占位 → 不等"
        );
        assert!(
            !uses_external_plugins(&with_instrument("vst3:cool-synth"), &[]),
            "目录还没扫出来（空目录）→ 不等：这一轮它只会是占位"
        );
    }

    /// 端到端：把 Apple 自带的 AU（DLSMusicDevice）挂到一条轨道上，离线渲染。
    ///
    /// 对应「在乐器/效果器里选 AU 插件直接闪退」那个问题：走的就是应用里真实那条路径
    /// （注册表 → 工厂 → `from_loaded` → `prepare` → `process`），并且 `process` 跑在**非主线程**上。
    #[test]
    fn renders_a_track_hosting_an_au() {
        install_fake_main_thread();
        let catalog = ai_daw_host_vst3::scan_catalog();
        let Some(entry) = catalog.iter().find(|p| p.id == "au:aumu:dls:appl") else {
            eprintln!("本机未安装 DLSMusicDevice，跳过。");
            return;
        };

        let mut engine = Engine::new("au-e2e");
        let events = engine.execute(Command::AddTrack { name: "AU".into() }, Actor::Human);
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!("AddTrack 应当产出 TrackAdded"),
        };
        engine.execute(
            Command::SetTrackInstrument {
                track,
                plugin_id: entry.id.clone(),
            },
            Actor::Human,
        );
        let events = engine.execute(
            Command::AddClip {
                track,
                start: 0.0,
                length: 2.0,
            },
            Actor::Human,
        );
        let clip = match events[0] {
            Event::ClipAdded { clip, .. } => clip,
            _ => unreachable!("AddClip 应当产出 ClipAdded"),
        };
        engine.execute(
            Command::AddNote {
                track,
                clip,
                note: Note {
                    id: 0,
                    channel: None,
                    pressure: None,
                    pitch: 60,
                    start: 0.0,
                    duration: 1.0,
                    velocity: 100,
                    volume: 127,
                    offset: 0.0,
                    legato: false,
                    expr: ai_daw_engine::model::PitchExpression::NONE,
                },
            },
            Actor::Human,
        );
        let project = engine.project().clone();

        let handles = Arc::new(Mutex::new(HashMap::new()));
        let registry = build_registry(&catalog, &project, Arc::clone(&handles));
        let options = RenderOptions {
            sample_rate: 44_100.0,
            tail_seconds: 0.5,
            settle_seconds: 0.0,
        };
        let buffer = render(&project, &registry, &options);

        assert!(
            buffer.peak() > 0.001,
            "挂 AU 插件的轨道必须出声（峰值 {}）",
            buffer.peak()
        );
    }

    /// 离线渲染注册表：VST3 走隔离加载，坏插件退回静音占位并把原因记录下来。
    ///
    /// 2026-10-20 的闪退就是 RdPiano 在 `dlopen` 后的构造函数里 `SIGBUS`。隔离之后同样的
    /// 崩溃只会杀掉 helper，工厂拿到错误、返回占位；这条测试钉住「错误不会向上冒成整个
    /// 渲染失败，也不会被静默吞掉」。
    #[test]
    fn offline_registry_reports_a_broken_vst3_and_falls_back_to_silence() {
        let id = "vst3:DEADBEEFDEADBEEFDEADBEEFDEADBEEF".to_string();
        let catalog = vec![CatalogPlugin {
            id: id.clone(),
            name: "Broken".into(),
            vendor: "test".into(),
            kind: ai_daw_plugin_api::PluginKind::Instrument,
            format: "vst3".into(),
            accepts_midi: true,
            has_editor: false,
            description: None,
            path: std::path::PathBuf::from("/nonexistent/Broken.vst3"),
        }];

        let mut engine = Engine::new("offline-isolation");
        let events = engine.execute(
            Command::AddTrack {
                name: "Broken".into(),
            },
            Actor::Human,
        );
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!("AddTrack 应当产出 TrackAdded"),
        };
        engine.execute(
            Command::SetTrackInstrument {
                track,
                plugin_id: id.clone(),
            },
            Actor::Human,
        );
        let project = engine.project().clone();

        let warnings: LoadWarnings = Arc::new(Mutex::new(Vec::new()));
        let handles = Arc::new(Mutex::new(HashMap::new()));
        let registry =
            build_registry_for_offline_render(&catalog, &project, handles, Arc::clone(&warnings));

        let plugin = registry
            .create(&id, track, PluginSlot::Instrument)
            .expect("目录里的插件应当有工厂");
        assert!(plugin.is_placeholder(), "加载失败必须退回静音占位");

        let warnings = warnings.lock().unwrap();
        assert!(
            warnings.iter().any(|w| w.contains(&id)),
            "必须记录加载失败（不是静默静音）：{warnings:?}"
        );
    }

    /// 手动端到端：用**本机会话里的真实工程**跑一次隔离离线渲染。
    ///
    /// 走的正是 2026-10-20 闪退那条路径（`analyze_project` → `render_offline` →
    /// `build_registry` → 加载 RdPiano）。默认 `#[ignore]`：它依赖本机的 session 与插件，
    /// CI 上没有意义。本机专门跑：
    ///
    /// ```text
    /// cd src-tauri
    /// cargo test --lib manual_offline_render_of_local_session -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "依赖本机会话与已安装插件"]
    fn manual_offline_render_of_local_session() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let app_dir =
            std::path::PathBuf::from(home).join("Library/Application Support/com.aidaw.app");
        let Some(session) = crate::session::read_session(&app_dir.join("session.json")) else {
            eprintln!("没有本机会话，跳过。");
            return;
        };

        // 测试进程旁边没有 Tauri 放好的 sidecar，显式指向构建脚本的产物。
        let binaries = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
        if let Ok(entries) = std::fs::read_dir(&binaries) {
            if let Some(helper) = entries.flatten().map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("vst3-host-helper-"))
            }) {
                std::env::set_var("VST3_HOST_HELPER_PATH", helper);
            }
        }

        let catalog = ai_daw_host_vst3::scan_catalog_with(
            &[app_dir.join("plugins/vst3")],
            &[app_dir.join("plugins/clap")],
            &[],
        );
        let warnings: LoadWarnings = Arc::new(Mutex::new(Vec::new()));
        let inputs = crate::commands::RenderInputs {
            project: session.project,
            catalog,
            editor_handles: Arc::new(Mutex::new(HashMap::new())),
            samples: HashMap::new(),
            warnings: Arc::clone(&warnings),
        };

        let buffer = crate::commands::render_offline(inputs, 48_000, 0.0);
        eprintln!(
            "隔离渲染完成：{} 帧，峰值 {:.6}",
            buffer.frames(),
            buffer.peak()
        );
        eprintln!("warnings: {:?}", warnings.lock().unwrap());
    }
}
