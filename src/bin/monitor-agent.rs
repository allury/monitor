use std::collections::VecDeque;
use std::env;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use monitor::agent::{self, AgentOptions};
use tracing_subscriber::EnvFilter;

const HELP: &str = r#"monitor-agent — Linux 只读探针

用法：
  monitor-agent --server <URL> [--token <密钥>] [--interval 2] [--disk /]
                [--telecom host:port] [--unicom host:port] [--mobile host:port]
  monitor-agent --version

密钥优先从 MONITOR_TOKEN 或 MONITOR_TOKEN_FILE 读取，避免出现在进程参数中。
三网地址可通过 MONITOR_PING_TELECOM、MONITOR_PING_UNICOM、
MONITOR_PING_MOBILE 覆盖，格式为 host:port。
"#;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("错误：{error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("monitor=info")),
        )
        .with_target(false)
        .compact()
        .init();

    let mut arguments: VecDeque<String> = env::args().skip(1).collect();
    if matches!(
        arguments.front().map(String::as_str),
        Some("--version" | "-V" | "version")
    ) {
        println!("monitor-agent {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments.is_empty()
        || matches!(
            arguments.front().map(String::as_str),
            Some("--help" | "-h" | "help")
        )
    {
        print!("{HELP}");
        return Ok(());
    }

    let server = required(&mut arguments, "--server")?;
    let token = option(&mut arguments, "--token")
        .or_else(|| env::var("MONITOR_TOKEN").ok())
        .or_else(|| {
            env::var("MONITOR_TOKEN_FILE")
                .ok()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .map(|value| value.trim().to_owned())
        })
        .context("请通过 MONITOR_TOKEN、MONITOR_TOKEN_FILE 或 --token 提供节点密钥")?;
    let seconds: u64 = option(&mut arguments, "--interval")
        .unwrap_or_else(|| "2".to_owned())
        .parse()
        .context("--interval 必须是秒数")?;
    let disk = option(&mut arguments, "--disk").unwrap_or_else(|| "/".to_owned());
    let telecom = option(&mut arguments, "--telecom")
        .unwrap_or_else(|| env_or("MONITOR_PING_TELECOM", "ha-ct-v4.ip.zstaticcdn.com:80"));
    let unicom = option(&mut arguments, "--unicom")
        .unwrap_or_else(|| env_or("MONITOR_PING_UNICOM", "ha-cu-v4.ip.zstaticcdn.com:80"));
    let mobile = option(&mut arguments, "--mobile")
        .unwrap_or_else(|| env_or("MONITOR_PING_MOBILE", "ha-cm-v4.ip.zstaticcdn.com:80"));
    ensure_empty(arguments)?;

    agent::run(AgentOptions {
        server,
        token,
        interval: Duration::from_secs(seconds),
        disk,
        telecom,
        unicom,
        mobile,
    })
    .await
}

fn option(arguments: &mut VecDeque<String>, name: &str) -> Option<String> {
    let position = arguments.iter().position(|item| item == name)?;
    arguments.remove(position);
    arguments.remove(position)
}

fn required(arguments: &mut VecDeque<String>, name: &str) -> Result<String> {
    option(arguments, name).with_context(|| format!("缺少参数 {name}"))
}

fn ensure_empty(arguments: VecDeque<String>) -> Result<()> {
    if let Some(argument) = arguments.front() {
        bail!("未知或缺少值的参数 {argument:?}");
    }
    Ok(())
}

fn env_or(name: &str, fallback: &str) -> String {
    env::var(name).unwrap_or_else(|_| fallback.to_owned())
}
