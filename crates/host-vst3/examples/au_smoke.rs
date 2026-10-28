//! AU 后端冒烟测试（macOS）：**必须在进程主线程上跑**，所以做成 example 而不是 `#[test]`。
//!
//! 为什么不能用单测：`AUAudioUnit` 的 `setFullState` / `requestViewController...` 都会把工作投到
//! **主队列**；单测进程的主线程被测试框架占着、不会派发主队列，那份工作会拖到进程退出、踩到已经
//! 析构的实例，直接段错误。example 的 `main` 就是进程主线程，没有这个问题。
//!
//! 用 Apple 自带的 AU 验证（本机没装第三方 AU 也能跑）：
//! `cargo run -p ai-daw-host-vst3 --example au_smoke`
//!
//! 覆盖：实例化、参数表、效果器直通渲染、状态保存、音源（DLSMusicDevice）收到 MIDI 后发声。

#[cfg(target_os = "macos")]
fn main() {
    use ai_daw_host_vst3::AuHostedPlugin;
    use ai_daw_plugin_api::{MidiEvent as HostEvent, Plugin, ProcessContext, TimedMidiEvent};

    // 告诉宿主「当前就是主线程」，让需要主线程的工作就地执行。
    ai_daw_host_vst3::mark_main_thread();

    // ---- 扫描：目录里的 .component + 注册表里的 AUv3 ----
    let scanned = ai_daw_host_vst3::scan();
    let au: Vec<_> = scanned.iter().filter(|p| p.format == "au").collect();
    let registry_only = au.iter().filter(|p| p.path.as_os_str().is_empty()).count();
    println!(
        "[扫描] 共 {} 个插件，其中 AU {} 个（只在注册表里的 / AUv3：{} 个）",
        scanned.len(),
        au.len(),
        registry_only
    );

    let ctx = ProcessContext {
        sample_rate: 48_000.0,
        block_size: 128,
        tempo: 120.0,
        playing: true,
    };
    let mut failures = 0usize;
    let mut check = |ok: bool, what: &str| {
        println!("  {} {}", if ok { "✓" } else { "✗" }, what);
        if !ok {
            failures += 1;
        }
    };

    // ---- 效果器：AUPeakLimiter ----
    println!("[AUPeakLimiter] 效果器");
    match AuHostedPlugin::from_loaded("au:aufx:lmtr:appl".into()) {
        Err(err) => {
            check(false, &format!("加载失败：{err}"));
        }
        Ok(mut limiter) => {
            limiter.prepare(48_000.0, ctx.block_size);
            println!(
                "  参数 {} 个，名字 {}",
                limiter.params().len(),
                limiter.descriptor().name
            );
            check(!limiter.params().is_empty(), "读到参数表");

            let mut left = vec![0.5f32; ctx.block_size];
            let mut right = vec![-0.5f32; ctx.block_size];
            limiter.process(&ctx, &mut [&mut left, &mut right], &[]);
            let peak = left
                .iter()
                .chain(right.iter())
                .fold(0f32, |m, v| m.max(v.abs()));
            check(peak > 0.01, &format!("直通渲染出声（峰值 {peak:.4}）"));

            let state = limiter.save_state();
            check(
                !state.is_empty(),
                &format!("保存状态（{} 字节）", state.len()),
            );
            match limiter.load_state(&state) {
                Ok(()) => check(true, "状态回写成功（主线程）"),
                Err(err) => check(false, &format!("状态回写失败：{err}")),
            }

            match limiter.editor_handle().open("AUPeakLimiter") {
                Ok(window) => {
                    check(true, "打开原生界面");
                    // 让 AppKit 真正跑一次建窗 / CA 提交，再关——更接近应用里的顺序。
                    drain_main_queue();
                    window.close();
                    // 关窗后不能崩：`releasedWhenClosed` 已关、关闭动画已禁。
                    drain_main_queue();
                    check(true, "关闭原生界面");

                    // 关掉之后再开一次：AU 通常只肯给一次视图控制器，宿主必须自己缓存它。
                    match limiter.editor_handle().open("AUPeakLimiter（再开）") {
                        Ok(window) => {
                            check(true, "关闭后能再次打开原生界面");
                            window.close();
                        }
                        Err(err) => check(false, &format!("关闭后无法再次打开：{err}")),
                    }
                }
                Err(err) => println!("  － 没有自带界面（宿主用参数面板）：{err}"),
            }
        }
    }

    // ---- 音源：DLSMusicDevice ----
    println!("[DLSMusicDevice] 音源");
    match AuHostedPlugin::from_loaded("au:aumu:dls:appl".into()) {
        Err(err) => {
            check(false, &format!("加载失败：{err}"));
        }
        Ok(mut synth) => {
            synth.prepare(48_000.0, ctx.block_size);
            println!("  参数 {} 个", synth.params().len());

            let mut out_l = vec![0.0f32; ctx.block_size];
            let mut out_r = vec![0.0f32; ctx.block_size];
            let mut peak = 0f32;
            for block in 0..8 {
                for ch in [&mut out_l, &mut out_r] {
                    ch.fill(0.0);
                }
                let events = if block == 0 {
                    vec![TimedMidiEvent {
                        frame: 0,
                        channel: 0,
                        event: HostEvent::NoteOn {
                            note: 60,
                            velocity: 100,
                        },
                    }]
                } else {
                    Vec::new()
                };
                synth.process(&ctx, &mut [&mut out_l, &mut out_r], &events);
                if out_l.iter().chain(out_r.iter()).any(|v| !v.is_finite()) {
                    check(false, "输出出现非有限值");
                    break;
                }
                peak = peak.max(
                    out_l
                        .iter()
                        .chain(out_r.iter())
                        .fold(0f32, |m, v| m.max(v.abs())),
                );
            }
            check(
                peak > 0.001,
                &format!("收到 NoteOn 后出声（峰值 {peak:.4}）"),
            );
        }
    }

    // 让主队列上剩下的异步工作（状态落地等）在退出前跑完。
    drain_main_queue();

    if failures == 0 {
        println!("\n全部通过");
    } else {
        println!("\n{failures} 项失败");
        std::process::exit(1);
    }
}

/// 泵一小会儿主 run loop，把主队列上的异步工作跑完。
#[cfg(target_os = "macos")]
fn drain_main_queue() {
    use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
    for _ in 0..20 {
        let date = NSDate::dateWithTimeIntervalSinceNow(0.005);
        // SAFETY: 只在主线程上跑主 run loop。
        unsafe { NSRunLoop::mainRunLoop().runMode_beforeDate(NSDefaultRunLoopMode, &date) };
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("AU（Audio Units）只在 macOS 上受支持，此 example 无内容。");
}
