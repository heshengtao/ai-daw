//! 插件目录 → 注册表的装配。
//!
//! 扫描很慢，所以目录在启动时后台扫描一次、并缓存到磁盘；音频线程按 id 懒加载插件。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use ai_daw_engine::model::Project;
use ai_daw_engine::registry::{missing_plugin, PluginRegistry};
use ai_daw_host_vst3::{load_catalog_plugin_with_editor, CatalogPlugin, EditorHandle};

/// 依据扫描目录构建注册表：内置合成器 + 外部插件工厂 + 缺失占位。
///
/// 外部插件工厂在加载失败时退回静音占位，绝不让一条坏轨拖垮整首曲子；
/// 创建实例时顺带把「轨道 → 编辑器句柄」登记进 `editor_handles`，供界面打开原生窗口。
pub fn build_registry(
    catalog: &[CatalogPlugin],
    project: &Project,
    editor_handles: Arc<Mutex<HashMap<u64, EditorHandle>>>,
) -> PluginRegistry {
    let mut registry = PluginRegistry::with_builtins();

    for entry in catalog {
        let id = entry.id.clone();
        let fallback_id = id.clone();
        let entry = entry.clone();
        let handles = Arc::clone(&editor_handles);
        registry.register(&id, move |track| {
            match load_catalog_plugin_with_editor(&entry) {
                Ok((plugin, Some(handle))) => {
                    handles.lock().unwrap().insert(track.0, handle);
                    plugin
                }
                Ok((plugin, None)) => plugin,
                Err(_) => missing_plugin(&fallback_id),
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
        let events = engine.execute(
            Command::AddTrack { name: "KR".into() },
            Actor::Human,
        );
        let track = match events[0] {
            Event::TrackAdded { id, .. } => id,
            _ => unreachable!("AddTrack 应当产出 TrackAdded"),
        };
        engine.execute(
            Command::SetTrackInstrument { track, plugin_id: kr106.id.clone() },
            Actor::Human,
        );
        let events = engine.execute(
            Command::AddClip { track, start: 0.0, length: 2.0 },
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
                    pitch: 60,
                    start: 0.0,
                    duration: 1.0,
                    velocity: 100,
                    volume: 127,
                },
            },
            Actor::Human,
        );
        let project = engine.project().clone();

        let handles = Arc::new(Mutex::new(HashMap::new()));
        let registry = build_registry(&catalog, &project, Arc::clone(&handles));
        let options = RenderOptions { sample_rate: 44_100.0, tail_seconds: 0.5 };
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
}
