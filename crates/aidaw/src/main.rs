//! `aidaw` —— AI DAW 的 MCP 薄客户端。
//!
//! 软件开着就把命令转发给它的 MCP HTTP 端点；没开就先把它启动起来。命令面 = MCP 工具面：
//! 加一个 MCP 工具 = 自动多一条可调用的 CLI 命令（按名转发，无需逐工具写代码）。

mod client;
mod discovery;

use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use serde_json::Value;

use client::McpClient;

/// `aidaw start` 等待 MCP 就绪的上限。
const START_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Parser)]
#[command(
    name = "aidaw",
    version,
    about = "AI DAW 的 MCP 薄客户端：软件开着就转发命令，没开就先启动。"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 列出软件当前暴露的全部 MCP 工具。
    Tools,
    /// 调用一个 MCP 工具（参数为 JSON 对象）。
    Call {
        /// 工具名（用 `aidaw tools` 查看）。
        tool: String,
        /// 工具参数，JSON 对象，例如 '{"bpm":120}'。
        #[arg(long, value_name = "JSON", default_value = "{}")]
        json: String,
    },
    /// 报告软件是否在运行、MCP 地址是否可达。
    Status,
    /// 未运行则启动软件，等到 MCP 就绪再返回。
    Start,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("aidaw: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    match cli.command {
        Command::Tools => {
            let (client, _) = connect_any()?;
            for tool in client.list_tools()? {
                let description = one_line(&tool.description);
                if description.is_empty() {
                    println!("{}", tool.name);
                } else {
                    println!("{} — {description}", tool.name);
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Call { tool, json } => {
            let arguments = parse_arguments(&json)?;
            let (client, _) = connect_any()?;
            let outcome = client.call_tool(&tool, arguments)?;
            if outcome.is_error {
                eprintln!("{}", outcome.text);
                return Ok(ExitCode::FAILURE);
            }
            println!("{}", outcome.text);
            Ok(ExitCode::SUCCESS)
        }
        Command::Status => match probe_any() {
            Some(url) => {
                println!("运行中 {url}");
                Ok(ExitCode::SUCCESS)
            }
            None => {
                println!("未运行");
                Ok(ExitCode::FAILURE)
            }
        },
        Command::Start => {
            if let Some(url) = probe_any() {
                println!("已在运行 {url}");
                return Ok(ExitCode::SUCCESS);
            }
            launch()?;
            let url = wait_ready()?;
            println!("已启动 {url}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// 找到第一个可达的地址并完成握手（软件没开时给出可操作的报错）。
fn connect_any() -> Result<(McpClient, String), String> {
    let candidates = discovery::candidates();
    for url in &candidates {
        if McpClient::probe(url) {
            return Ok((McpClient::connect(url)?, url.clone()));
        }
    }
    Err(format!(
        "未找到正在运行的 AI DAW（已尝试：{}）。先执行 `aidaw start`。",
        candidates.join(", ")
    ))
}

/// 探测哪个候选地址开着（短超时）。
fn probe_any() -> Option<String> {
    discovery::candidates()
        .into_iter()
        .find(|url| McpClient::probe(url))
}

/// 轮询直到 MCP 就绪（每轮重读发现文件——软件起来后会写下**实际**端口）。
fn wait_ready() -> Result<String, String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(url) = probe_any() {
            return Ok(url);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "等待 MCP 就绪超时（{} 秒），软件可能仍在启动或未启用 MCP。",
                START_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// 解析 `--json` 参数：必须是 JSON 对象（MCP 的 `arguments`）。
fn parse_arguments(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| format!("--json 不是合法 JSON：{e}"))?;
    if !value.is_object() {
        return Err("--json 必须是一个 JSON 对象，例如 '{\"bpm\":120}'".to_owned());
    }
    Ok(value)
}

/// 启动软件：macOS 用 `open -a`；其它平台用 `AIDAW_APP_PATH` 指定的可执行文件。
///
/// 两者都可用环境变量 `AIDAW_APP_PATH` 覆盖（dev 构建 / 自定义安装位置）。
#[cfg(target_os = "macos")]
fn launch() -> Result<(), String> {
    let app = std::env::var("AIDAW_APP_PATH").unwrap_or_else(|_| discovery::APP_NAME.to_owned());
    let status = std::process::Command::new("open")
        .args(["-a", &app])
        .status()
        .map_err(|e| format!("无法启动软件（执行 open 失败）：{e}"))?;
    if !status.success() {
        return Err(format!(
            "无法启动软件：`open -a {app}` 退出码 {status}。可用 AIDAW_APP_PATH 指定应用路径。"
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn launch() -> Result<(), String> {
    let path = std::env::var("AIDAW_APP_PATH")
        .map_err(|_| "本平台需要设置 AIDAW_APP_PATH 指向 AI DAW 可执行文件。".to_owned())?;
    std::process::Command::new(&path)
        .spawn()
        .map_err(|e| format!("无法启动软件（{path}）：{e}"))?;
    Ok(())
}

/// 压成一行（工具描述里可能带换行）。
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
