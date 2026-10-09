//! 与真实插件的集成测试。
//!
//! 依赖本机装了 KR-106（VST3 + CLAP）；没装时测试直接跳过（因此 CI 上没有插件也能通过）。
//!
//! 场景（两种格式各跑一遍）：扫描 → 加载 → 用引擎的 `Plugin` trait 渲染一个 NoteOn →
//! 断言有输出。这正是当初 truce-rack 失败的用例：VST3 静音、CLAP 段错误。
//!
//! 另外两条测试专门盯 CLAP 的**线程契约**（KR-106 会真的去核对，违规时会报警甚至卡死）：
//! `init` / 参数枚举 / `activate` / 状态存取必须在主线程，`start_processing` / `reset` /
//! `process` 必须在音频线程。所以这里造了一条「假主线程」当宿主的主线程。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, OnceLock};

use daw_party_host_vst3::MainThreadDispatch;
use daw_party_plugin_api::{MidiEvent, ProcessContext, TimedMidiEvent};

/// 假主线程上执行过的任务数。用来断言「这件事确实被派到主线程去了」。
static MAIN_THREAD_TASKS: AtomicUsize = AtomicUsize::new(0);

/// 测试用的「假主线程」。
///
/// 整个测试进程只起一条（`Once`），所有 CLAP 测试共用：它先在自己身上登记
/// `mark_main_thread()`，然后一直处理派发过来的活。
///
/// 有了它，跑测试时才不会看到一串 `... on wrong thread!` 警告——那正是真实应用里
/// 「开插件界面就卡死」的同一类违规。
fn fake_main_thread() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    static FAKE: OnceLock<Arc<FakeMain>> = OnceLock::new();

    INSTALLED.call_once(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("fake-main-thread".into())
            .spawn(move || {
                daw_party_host_vst3::mark_main_thread();
                let _ = ready_tx.send(());
                while let Ok(task) = rx.recv() {
                    task();
                    MAIN_THREAD_TASKS.fetch_add(1, Ordering::SeqCst);
                }
            })
            .expect("起假主线程");
        // 等它登记完 `mark_main_thread()`，否则断言会随机失败。
        ready_rx.recv().expect("假主线程必须完成登记");

        let fake = Arc::new(FakeMain { tx });
        daw_party_host_vst3::set_main_thread_dispatch(Arc::clone(&fake) as Arc<dyn MainThreadDispatch>);
        let _ = FAKE.set(fake);
    });
}

/// 把任务排队给假主线程并等它跑完。
struct FakeMain {
    tx: mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>,
}

impl MainThreadDispatch for FakeMain {
    fn dispatch(&self, task: Box<dyn FnOnce() + Send + 'static>) -> bool {
        let (done_tx, done_rx) = mpsc::channel();
        let wrapped = Box::new(move || {
            task();
            let _ = done_tx.send(());
        });
        if self.tx.send(wrapped).is_err() {
            return false;
        }
        done_rx.recv().is_ok()
    }
}

/// 按格式找一个 KR-106。
fn find(format: &str) -> Option<daw_party_host_vst3::ScannedPlugin> {
    daw_party_host_vst3::scan()
        .into_iter()
        .find(|p| p.name.contains("KR-106") && p.format == format)
}

/// 从 `clap:<id>` 形式的条目 id 里取出插件 id。
fn plugin_id_of(scanned: &daw_party_host_vst3::ScannedPlugin) -> std::ffi::CString {
    std::ffi::CString::new(
        scanned
            .id
            .strip_prefix("clap:")
            .expect("CLAP 条目 id 形如 clap:<id>"),
    )
    .unwrap()
}

fn exercise(format: &str) {
    // 先装派发器再扫描：扫描阶段也会实例化插件问它有没有界面（`init` / `is_api_supported`
    // 都是主线程专属）。
    fake_main_thread();
    let Some(scanned) = find(format) else {
        eprintln!("本机未安装 KR-106 的 {format} 版本，跳过。");
        return;
    };

    let mut plugin = scanned.load().unwrap_or_else(|e| panic!("加载 {format} 失败：{e}"));

    let d = plugin.descriptor();
    assert_eq!(
        d.kind,
        daw_party_plugin_api::PluginKind::Instrument,
        "{format} 的 KR-106 是音源"
    );
    assert_eq!(d.io.outputs, 2, "引擎是立体声输出");
    assert!(d.capabilities.has_editor, "{format} 的 KR-106 自带原生界面");

    plugin.prepare(48_000.0, 512);
    let params = plugin.params();
    assert!(!params.is_empty(), "{format} 应能读到插件参数");

    // 「恢复默认参数」就是把这批 `default` 整组写回，所以后端**必须报真实默认值**。
    // VST3 适配器曾把它们硬写成 0.0：点一下就等于把所有参数清零，乐器当场哑掉
    // （CLAP 后端一直报的是真实值，于是只有 VST3 中招）。
    assert!(
        params.iter().any(|p| p.default != 0.0),
        "{format} 的参数默认值不应当全是 0——那说明后端没把插件的默认值传下来"
    );

    let ctx = ProcessContext { sample_rate: 48_000.0, block_size: 512, tempo: 120.0, playing: true };
    let events = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 60, velocity: 100 } }];

    let mut peak = 0.0f32;
    let mut left = vec![0.0f32; 512];
    let mut right = vec![0.0f32; 512];
    for _ in 0..50 {
        plugin.process(&ctx, &mut [&mut left, &mut right], &events);
        for s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }

    assert!(peak > 0.01, "{format} 宿主必须让音源出声（峰值 {peak}）");

    // 「暂停 / 停止」＝宿主对乐器调 `reset()`，之后必须**照样出声**。
    //
    // 注意这条断言抓不到完整的 bug：真正出事的是「**界面开着**时 reset」——那需要开原生窗口，
    // 只能在主线程上做，测试进程给不了。那一路由 `examples/editor_audio.rs` 覆盖
    // （它照搬应用的主线程泵界面 + 独立音频线程的结构）。这条断言守住的是「reset 本身不许弄哑乐器」。
    let off = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOff { note: 60 } }];
    plugin.process(&ctx, &mut [&mut left, &mut right], &off);
    plugin.process(&ctx, &mut [&mut left, &mut right], &[]);
    plugin.reset();

    let mut after = 0.0f32;
    let mut sent = false;
    for _ in 0..50 {
        let evs: &[TimedMidiEvent] = if sent { &[] } else { &events };
        plugin.process(&ctx, &mut [&mut left, &mut right], evs);
        sent = true;
        for s in left.iter().chain(right.iter()) {
            after = after.max(s.abs());
        }
    }
    assert!(
        after > 0.01,
        "{format} 在 reset()（暂停/停止）之后必须还能出声（峰值 {after}）"
    );

    assert!(!plugin.save_state().is_empty(), "{format} 应能保存插件状态");
}

#[test]
fn hosts_a_real_vst3_instrument_and_renders_audio() {
    exercise("vst3");
}

#[test]
fn hosts_a_real_clap_instrument_and_renders_audio() {
    exercise("clap");
}

/// CLAP 的线程契约必须被遵守：`init` / 参数枚举 / `activate` 在主线程，`start_processing` /
/// `reset` 在音频线程。
///
/// 这条测试复刻的就是「KR-106 报警 + 卡死」的那套违规。断言方式是**按阶段检查状态**，而不是
/// 只看计数器：
/// - `prepare` 之后：已 `activate`（主线程）但**还没** `start_processing`；
/// - 在「音频线程」上跑过 `process` 之后：才进入处理状态，且该线程被认成音频线程；
/// - `reset` 之后：仍然 active——说明音频线程上没有发生非法的 deactivate/activate 往返。
///
/// 另外用假主线程的任务计数证明 `activate` 确实是被派到主线程执行的。
#[test]
fn clap_lifecycle_follows_thread_rules() {
    use daw_party_plugin_api::Plugin;

    fake_main_thread();
    let Some(scanned) = find("clap") else {
        eprintln!("本机未安装 KR-106 的 clap 版本，跳过。");
        return;
    };

    // 加载：`init` 与参数枚举都必须发生在那条假主线程上。
    let plugin_id = plugin_id_of(&scanned);
    let (mut clap, _handle) =
        daw_party_host_vst3::scan::load_clap_plugin(&scanned.id, &scanned.path, &plugin_id)
            .unwrap_or_else(|e| panic!("加载 clap 版 KR-106 失败：{e}"));
    assert!(
        MAIN_THREAD_TASKS.load(Ordering::SeqCst) > 0,
        "CLAP 的加载（init + 参数枚举）必须派发到主线程执行"
    );

    // 本线程当音频线程用。
    let before_prepare = MAIN_THREAD_TASKS.load(Ordering::SeqCst);
    clap.prepare(48_000.0, 512);
    assert!(
        MAIN_THREAD_TASKS.load(Ordering::SeqCst) > before_prepare,
        "activate 必须派发到主线程执行"
    );
    assert!(clap.is_active(), "prepare 之后实例应当已激活");
    assert!(
        !clap.is_processing(),
        "start_processing 是音频线程专属，不该在 prepare 里做"
    );
    assert!(
        !clap.audio_thread_is_current(),
        "本线程还没跑过 process，不该被认成音频线程"
    );

    // 在「音频线程」上跑一块：这里才允许 start_processing。
    let ctx = ProcessContext { sample_rate: 48_000.0, block_size: 512, tempo: 120.0, playing: true };
    let events = [TimedMidiEvent { frame: 0, event: MidiEvent::NoteOn { note: 60, velocity: 100 } }];
    let mut left = vec![0.0f32; 512];
    let mut right = vec![0.0f32; 512];
    let mut peak = 0.0f32;
    for _ in 0..50 {
        clap.process(&ctx, &mut [&mut left, &mut right], &events);
        for s in left.iter().chain(right.iter()) {
            peak = peak.max(s.abs());
        }
    }
    assert!(
        clap.is_processing(),
        "第一次 process 应当在音频线程上完成 start_processing"
    );
    assert!(
        clap.audio_thread_is_current(),
        "跑 process 的线程必须被认成音频线程"
    );
    assert!(peak > 0.01, "线程规则改过之后仍要出声（峰值 {peak}）");

    // reset 走 clap_plugin.reset()：音频线程合法，且**不应该**把实例 deactivate 掉。
    clap.reset();
    assert!(
        clap.is_active(),
        "reset 不该 deactivate 实例——旧实现会在音频线程上做非法的 deactivate/activate 往返"
    );
    assert!(clap.is_processing(), "reset 之后应当仍在处理状态");

    // 状态存取也是主线程专属：这里跑一次，KR-106 不该再抱怨线程。
    assert!(!clap.save_state().is_empty(), "应能保存插件状态");
}

/// 音频线程 id 必须取自**真正跑 `process` 的那条线程**。
///
/// 真实应用里「建实例的线程」（主线程）和「跑 process 的线程」（音频后端回调线程）不是同一条。
/// 如果宿主在建实例时就把当前线程当成音频线程，插件每次 `process` 都会被告知「你不在音频
/// 线程」，于是**每个音频块往 stderr 打一行日志**——实时线程被 I/O 堵死，应用卡死。
///
/// 单线程的测试抓不到这个 bug，所以这里刻意在另一条线程上跑 `process`。
#[test]
fn audio_thread_is_learned_from_the_processing_thread() {
    use daw_party_plugin_api::Plugin;

    fake_main_thread();
    let Some(scanned) = find("clap") else {
        eprintln!("本机未安装 KR-106 的 clap 版本，跳过。");
        return;
    };

    let plugin_id = plugin_id_of(&scanned);
    let (mut plugin, _handle) =
        daw_party_host_vst3::scan::load_clap_plugin(&scanned.id, &scanned.path, &plugin_id)
            .expect("构造 CLAP 实例");
    plugin.prepare(48_000.0, 512);

    // 在另一条线程上 process：那条线程才是音频线程。
    let (learned, plugin) = std::thread::spawn(move || {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            block_size: 256,
            tempo: 120.0,
            playing: true,
        };
        let mut l = [0.0f32; 256];
        let mut r = [0.0f32; 256];
        plugin.process(&ctx, &mut [&mut l, &mut r], &[]);
        (plugin.audio_thread_is_current(), plugin)
    })
    .join()
    .expect("工作线程不应 panic");

    assert!(learned, "跑 process 的那条线程必须被认成音频线程");
    assert!(
        !plugin.audio_thread_is_current(),
        "本线程（建实例的地方）不该被误认成音频线程"
    );
}
