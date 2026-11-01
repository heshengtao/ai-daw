//! 手动诊断：**本机装好的那一份 ONNX Runtime 到底能不能加载**。
//!
//! `load-dynamic` 下 `ort` 自己也会找库：先看 `ORT_DYLIB_PATH`，没有就按**裸文件名**
//! `onnxruntime.dll` 去系统目录里搜——Windows 自带一份 1.17.1（`C:\WINDOWS\system32`），
//! 版本不达标时 `ort` 会在 `setup_api()` 里 `expect` 直接 panic。所以「运行时有没有真的
//! 被我们加载」必须能单独验证，而不是等到某次推理才炸。
//!
//! 跑法（把 `AIDAW_TEST_RUNTIME` 指到 `%APPDATA%\com.aidaw.app\runtime\ort\cuda` 这类目录）：
//!
//! ```text
//! $env:AIDAW_TEST_RUNTIME="$env:APPDATA\com.aidaw.app\runtime\ort\cuda"
//! cargo test -p ai-daw-local-ai --test runtime_probe -- --nocapture
//! ```
//!
//! 不设这个变量时测试直接跳过（CI 上没有这些几百 MB 的 dylib）。
#[test]
fn loads_installed_runtime() {
    let Some(dir) = std::env::var_os("AIDAW_TEST_RUNTIME") else {
        eprintln!("跳过：没设 AIDAW_TEST_RUNTIME");
        return;
    };
    std::env::set_var("AIDAW_ORT_RUNTIME", &dir);

    // 常见坑：系统目录里那份 1.17.1。把它指出来，省得排查时怀疑错方向。
    if let Some(found) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(ai_daw_local_ai::backend::runtime::library_name()))
            .find(|candidate| candidate.is_file())
    }) {
        eprintln!("注意：PATH 里还有一份 {}", found.display());
    }

    let loaded = ai_daw_local_ai::backend::runtime::ensure_loaded()
        .unwrap_or_else(|err| panic!("加载失败：{err}"));
    assert!(loaded.is_file(), "返回的路径不存在：{}", loaded.display());
    eprintln!("已加载 {}", loaded.display());

    // 加载之后再碰 `ort` 的 API 不应该 panic（这正是 SVC / 歌声合成每次建会话走的路）。
    let builder = ort::session::Session::builder().expect("Session::builder");
    drop(builder);
    eprintln!("`Session::builder()` 正常");

    // CUDA EP 到底能不能用：`false` 就说明「显存不涨」是因为一路回落到 CPU 了。
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        use ort::ep::ExecutionProvider as _;
        let cuda = ort::ep::CUDA::default().is_available();
        eprintln!("CUDA EP：{cuda:?}");
    }
}
