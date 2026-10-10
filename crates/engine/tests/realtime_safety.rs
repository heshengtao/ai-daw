//! 实时安全不变量：音频回调路径（`Player::process_block` + `CommandReceiver::drain`）
//! 在预热之后不得发生任何堆分配。
//!
//! 本文件名下只有一个测试：`#[global_allocator]` 作用于整个测试二进制，若同二进制内
//! 还有其它并行测试，它们的分配会污染计数，导致误判。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ai_daw_engine::command_queue::command_queue;
use ai_daw_engine::model::{Clip, ClipId, Note, PluginInstance, Project, Track, TrackId};
use ai_daw_engine::player::Player;
use ai_daw_engine::registry::PluginRegistry;
use ai_daw_engine::schedule::ScheduledEvents;
use ai_daw_engine::transport::TransportCommand;
/// 统计分配的全局分配器（仅本测试二进制生效）。
struct CountingAllocator;

/// 自上次 `store(0)` 以来的分配次数（含 alloc / alloc_zeroed / realloc）。
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn schedule() -> ScheduledEvents {
    let mut project = Project::new("safety");
    project.transport.tempo = 120.0;
    project.tracks.push(Track {
        id: TrackId(1),
        name: "Lead".into(),
        instrument: Some(PluginInstance {
            plugin_id: "builtin.synth".into(),
            params: Default::default(),
            state: vec![],
        }),
        // 效果器链**也必须在零分配门禁的覆盖范围内**：内置延迟/混响都在音频回调里跑，
        // 参数给非默认值是为了让湿声路径真的执行（干声路径太容易「碰巧不分配」）。
        effects: vec![
            PluginInstance {
                plugin_id: ai_daw_engine::registry::BUILTIN_REVERB.into(),
                params: [("size".to_string(), 0.8), ("mix".to_string(), 0.5)].into_iter().collect(),
                state: vec![],
            },
            PluginInstance {
                plugin_id: ai_daw_engine::registry::BUILTIN_DELAY.into(),
                params: [("time".to_string(), 0.3), ("mix".to_string(), 0.4)].into_iter().collect(),
                state: vec![],
            },
        ],
        clips: vec![Clip {
            id: ClipId(1),
            start: 0.0,
            length: 4.0,
            notes: vec![Note { pitch: 69, start: 0.0, duration: 0.5, velocity: 100, volume: 127 }],
            audio: None,
        }],
        mixer: Default::default(),
        // 发送与自动化也要在门禁覆盖范围内：总线路由与逐块自动化求值都跑在音频回调里。
        sends: vec![ai_daw_engine::model::Send { target: TrackId(2), gain_db: -6.0 }],
        automation: vec![ai_daw_engine::model::AutomationLane {
            param: "gain_db".into(),
            points: vec![
                ai_daw_engine::model::AutomationPoint { beat: 0.0, value: 0.0 },
                ai_daw_engine::model::AutomationPoint { beat: 8.0, value: -12.0 },
            ],
        }],
    });
    // 一条总线轨道：接收上面那条轨道的发送。
    project.tracks.push(Track {
        id: TrackId(2),
        name: "Bus".into(),
        instrument: None,
        effects: vec![],
        clips: vec![],
        mixer: Default::default(),
        sends: vec![],
        automation: vec![],
    });
    ScheduledEvents::from_project(&project, 48_000.0)
}

#[test]
fn realtime_block_path_does_not_allocate() {
    const MAX_BLOCK: usize = 256;

    let registry = PluginRegistry::with_builtins();
    let mut player = Player::new(schedule(), &registry, 48_000.0, MAX_BLOCK);
    let (mut tx, mut rx) = command_queue(64);
    player.apply_command(TransportCommand::Play);

    // 混音命令与传输命令共用同一条实时路径：拖动推子时每秒会进来几十条，
    // 因此 `refresh_mix` 也必须在零分配的前提下工作。
    let mix = |mute: bool, solo: bool| TransportCommand::SetTrackMix {
        track: TrackId(1),
        gain_db: -3.0,
        pan: 0.25,
        mute,
        solo,
    };

    let mut out = vec![0.0f32; MAX_BLOCK * 2];

    // 预热：跑通所有可能一次性增长（resize / realloc）的路径，并让队列里有真实命令。
    for i in 0..8 {
        player.process_block(&mut out, MAX_BLOCK);
        for _ in 0..4 {
            let _ = tx.send(TransportCommand::Play);
            let _ = tx.send(mix(i % 2 == 0, false));
        }
        let _ = tx.send(TransportCommand::SetMasterGain { gain_db: -1.0 });
        player.drain_commands(&mut rx);
    }

    // 测量窗口。
    ALLOCATIONS.store(0, Ordering::SeqCst);
    for i in 0..10_000 {
        player.process_block(&mut out, MAX_BLOCK);
        let _ = tx.send(TransportCommand::Play);
        let _ = tx.send(mix(i % 2 == 0, i % 3 == 0));
        let _ = tx.send(TransportCommand::SetMasterGain { gain_db: -1.0 });
        player.drain_commands(&mut rx);
    }
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);

    assert_eq!(
        allocations, 0,
        "realtime audio path must not allocate (saw {allocations})"
    );

    // --- 第二阶段：`SetAutomation` 的 apply_command 也必须零分配 ---
    // 命令自身在测量前就构造好（它的 Vec/String 会分配，那是控制侧的事）。
    let command = TransportCommand::SetAutomation {
        track: TrackId(1),
        target: ai_daw_engine::schedule::LaneTarget::GainDb,
        points: vec![
            ai_daw_engine::model::AutomationPoint { beat: 0.0, value: -3.0 },
            ai_daw_engine::model::AutomationPoint { beat: 4.0, value: -6.0 },
        ],
    };
    ALLOCATIONS.store(0, Ordering::SeqCst);
    player.apply_command(command);
    player.process_block(&mut out, MAX_BLOCK);
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    assert_eq!(
        allocations, 0,
        "SetAutomation 必须在音频线程上零分配（saw {allocations}）"
    );
}
