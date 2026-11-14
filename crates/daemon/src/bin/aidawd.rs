//! 无窗口入口：`aidawd --port 28430 --lan`。
//!
//! 只跑后端 + HTTP 服务，不开窗口、不初始化 Tauri App。容器里跑的就是它。

use std::sync::Arc;

use ai_daw_daemon::boot::{bootstrap, serve, HeadlessHost};
use ai_daw_daemon::daemon::Config;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "aidawd", version, about = "AI DAW 的无窗口后端")]
struct Args {
    /// 数据目录；默认用系统临时目录下的 ai-daw-headless。
    #[arg(long)]
    data_dir: Option<std::path::PathBuf>,
    /// 确实要绑到 0.0.0.0 才开：容器里必须开，本机调试别开。
    #[arg(long, default_value_t = false)]
    lan: bool,
    /// HTTP 端口。
    #[arg(long, default_value_t = 28430)]
    port: u16,
    /// 前端构建产物目录（`dist/`）。给了就能直接用浏览器打开 `http://<host>:<port>/`。
    /// 也可以走环境变量 `AIDAW_WEB_DIR`（容器里更顺手）。
    #[arg(long)]
    web_dir: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    ai_daw_daemon::mark_boot();
    let data_dir = args
        .data_dir
        .unwrap_or_else(|| std::env::temp_dir().join("ai-daw-headless"));
    std::fs::create_dir_all(&data_dir)?;

    let host = Arc::new(HeadlessHost::new(&data_dir));
    let cfg = Config {
        data_dir: data_dir.clone(),
        lan: args.lan,
        port: Some(args.port),
        web_dir: args.web_dir.or_else(|| {
            std::env::var_os("AIDAW_WEB_DIR").map(std::path::PathBuf::from)
        }),
    };
    let daemon = bootstrap(host, cfg);
    let (addr, fut) = serve(daemon).await?;

    println!("aidawd 已启动：http://{addr}");
    println!("数据目录：{}", data_dir.display());
    println!("打开 http://{addr}/ 即可使用（无登录、无令牌）");
    fut.await;
    Ok(())
}
