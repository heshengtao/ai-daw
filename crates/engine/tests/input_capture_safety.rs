//! 输入采集回调（[`capture_block`]）的零分配门禁。
//!
//! 与 `realtime_safety.rs` 一样：`#[global_allocator]` 作用于整个测试二进制，故本文件
//! **只放这一个测试**，避免并行测试污染计数。
//!
//! 注意：**只有回调路径**要求在预热后零分配。消费侧的 `drain_into`（把采样搬进 `Vec`）
//! 会正常增长缓冲、允许分配，因此**不在测量窗口内**——若把它也计入，会误判。

// 采集器整块都在 `realtime` 特性下；不带该特性时本文件编译为空（`cargo test --workspace`
// 不启用它，只有 `--features realtime` 才跑这个门禁）。
#![cfg(feature = "realtime")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ai_daw_engine::recorder::capture_block;
use ai_daw_engine::rtrb::RingBuffer;
use ai_daw_engine::scope::ScopeBuffer;

/// 统计分配的全局分配器（仅本测试二进制生效）。
struct CountingAllocator;
/// 自上次 `store(0)` 以来的分配次数。
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

#[test]
fn input_capture_callback_does_not_allocate() {
    const FRAMES: usize = 512;
    // 环开大一点：测量窗口内即便不消费也不会立刻填满（填满也只是丢帧，同样不分配）。
    let (mut producer, _consumer) = RingBuffer::<f32>::new(1 << 18);
    let dropped = AtomicUsize::new(0);
    let scope = ScopeBuffer::new(8192);
    let active = AtomicBool::new(true);
    let interleaved: Vec<f32> = (0..FRAMES * 2).map(|i| (i as f32 * 0.01).sin()).collect();

    // 预热：跑通 ScopeBuffer / 环的首次写入路径。
    for _ in 0..8 {
        capture_block(&interleaved, 2, 2, |x| x, &mut producer, &dropped, &scope, &active);
    }

    ALLOCATIONS.store(0, Ordering::SeqCst);
    for _ in 0..10_000 {
        capture_block(&interleaved, 2, 2, |x| x, &mut producer, &dropped, &scope, &active);
    }
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);

    assert_eq!(allocations, 0, "输入回调路径不得分配（saw {allocations}）");
}
