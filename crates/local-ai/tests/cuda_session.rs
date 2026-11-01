//! Manual diagnostic: **can CUDA actually build a session** (not just "the runtime loaded")?
//!
//! `runtime_probe` only proves the dylib loads and that the CUDA EP reports as available;
//! the usual failure is inside `session::build` (EP registration, unsupported operator,
//! provider DLL version mismatch). `build_for` silently falls back to CPU and only writes
//! one `[backend] ... fallback CPU EP` line to the log, so the UI looks fine while the GPU
//! stays at 0% and the CPU is pinned.
//!
//! Run it (point at a CUDA runtime directory + any installed graph):
//!
//! ```text
//! $env:AIDAW_TEST_RUNTIME="$env:APPDATA\com.aidaw.app\runtime\ort\cuda"
//! $env:AIDAW_TEST_MODEL="$env:APPDATA\com.aidaw.app\models\soulx-svc-fp16hq\cond.emb.onnx"
//! cargo test -p ai-daw-local-ai --test cuda_session -- --nocapture
//! ```
//!
//! `AIDAW_TEST_BACKEND` can be `directml`, `cpu`, `cuda12` (default) or `cuda13`.

#[test]
fn cuda_builds_session_on_worker_thread() {
    let (Some(runtime_dir), Some(model)) = (
        std::env::var_os("AIDAW_TEST_RUNTIME"),
        std::env::var_os("AIDAW_TEST_MODEL"),
    ) else {
        eprintln!("skipped: AIDAW_TEST_RUNTIME / AIDAW_TEST_MODEL not set");
        return;
    };
    std::env::set_var("AIDAW_ORT_RUNTIME", &runtime_dir);
    use ai_daw_local_ai::backend::{self, session::GraphRole, BackendKind};
    let kind = match std::env::var("AIDAW_TEST_BACKEND").as_deref() {
        Ok("directml") => BackendKind::DirectMl,
        Ok("cpu") => BackendKind::Cpu,
        Ok("cuda13") => BackendKind::Cuda13,
        _ => BackendKind::Cuda12,
    };
    backend::set_backend(kind);

    // The real app builds sessions on a worker thread (`ai-daw-svc`), so do the same here.
    let handle = std::thread::Builder::new()
        .name("ai-daw-svc".into())
        .spawn(move || {
            // Build one graph only; when a directory is given, use its first `.onnx`.
            let file = if std::path::Path::new(&model).is_dir() {
                let mut names: Vec<_> = std::fs::read_dir(&model)
                    .expect("read model directory")
                    .filter_map(|entry| entry.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|ext| ext == "onnx"))
                    .collect();
                names.sort();
                names.into_iter().next().expect("no .onnx in directory")
            } else {
                std::path::PathBuf::from(&model)
            };
            let session = backend::session::build(&file, GraphRole::Svc)
                .unwrap_or_else(|err| panic!("build session failed: {err}"));
            eprintln!(
                "worker thread built a session (inputs {})",
                session.inputs().len()
            );
            backend::session::take_session_stats()
        })
        .expect("spawn");
    let (gpu, cpu) = handle.join().expect("thread panicked");
    eprintln!("worker thread stats: GPU {gpu} / CPU {cpu}");
    assert_eq!(cpu, 0, "{cpu} graph(s) fell back to CPU");
}

#[test]
fn cuda_builds_session() {
    let (Some(runtime_dir), Some(model)) = (
        std::env::var_os("AIDAW_TEST_RUNTIME"),
        std::env::var_os("AIDAW_TEST_MODEL"),
    ) else {
        eprintln!("skipped: AIDAW_TEST_RUNTIME / AIDAW_TEST_MODEL not set");
        return;
    };
    std::env::set_var("AIDAW_ORT_RUNTIME", &runtime_dir);

    use ai_daw_local_ai::backend::{self, session::GraphRole, BackendKind};
    let kind = match std::env::var("AIDAW_TEST_BACKEND").as_deref() {
        Ok("directml") => BackendKind::DirectMl,
        Ok("cpu") => BackendKind::Cpu,
        Ok("cuda13") => BackendKind::Cuda13,
        _ => BackendKind::Cuda12,
    };
    backend::set_backend(kind);

    let path = std::path::Path::new(&model);
    // A directory means "build every graph in this tier", matching `Runner::load`.
    let files: Vec<std::path::PathBuf> = if path.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(path)
            .expect("read model directory")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "onnx"))
            .collect();
        names.sort();
        names
    } else {
        vec![path.to_path_buf()]
    };
    let mut sessions = Vec::new();
    for file in &files {
        let session = backend::session::build(file, GraphRole::Svc)
            .unwrap_or_else(|err| panic!("build session failed ({}): {err}", file.display()));
        eprintln!(
            "OK {} (inputs {} / outputs {})",
            file.file_name().unwrap_or_default().to_string_lossy(),
            session.inputs().len(),
            session.outputs().len()
        );
        sessions.push(session);
    }

    // Prove it with VRAM: while the sessions are alive, nvidia-smi should show the process
    // holding a few hundred MB.
    fn vram_used() -> String {
        std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.used", "--format=csv,noheader"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|text| text.trim().to_string())
            .unwrap_or_else(|| "?".into())
    }
    use ort::ep::ExecutionProvider as _;
    eprintln!(
        "CUDA available: {:?}, VRAM (after build): {}",
        ort::ep::CUDA::default().is_available(),
        vram_used()
    );

    let (gpu, cpu) = ai_daw_local_ai::backend::session::take_session_stats();
    eprintln!("session stats: GPU {gpu} / CPU {cpu}");
    assert_eq!(
        cpu, 0,
        "{cpu} graph(s) fell back to CPU -- GPU may still be idle"
    );
    drop(sessions);
}
