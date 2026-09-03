use std::collections::VecDeque;
use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use monitor::{db, server};
use tracing_subscriber::EnvFilter;

const HELP: &str = r#"monitor-server — 极简监控主控

用法：
  monitor-server [--listen 127.0.0.1:34331] [--db monitor.db]
  monitor-server admin reset [--db monitor.db]
  monitor-server node create --id <ID> --name <名称> [--db monitor.db]
  monitor-server node revoke --id <ID> [--db monitor.db]
  monitor-server node list [--db monitor.db]
  monitor-server --version
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
    match arguments.front().map(String::as_str) {
        Some("admin") => {
            arguments.pop_front();
            admin_command(arguments)
        }
        Some("node") => {
            arguments.pop_front();
            node_command(arguments)
        }
        Some("--version" | "-V" | "version") => {
            println!("monitor-server {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h" | "help") => {
            print!("{HELP}");
            Ok(())
        }
        _ => server_command(arguments).await,
    }
}

fn admin_command(mut arguments: VecDeque<String>) -> Result<()> {
    let action = arguments.pop_front().context("admin 后需要 reset")?;
    let database =
        PathBuf::from(option(&mut arguments, "--db").unwrap_or_else(|| "monitor.db".to_owned()));
    ensure_empty(arguments)?;
    match action.as_str() {
        "reset" => {
            let password = db::reset_admin(&database)?;
            println!("新管理员密钥（仅显示这一次）：{password}");
            println!("已有网页会话会在主控下次重启时全部失效。");
            Ok(())
        }
        other => bail!("未知 admin 命令 {other:?}；可用命令：reset"),
    }
}

async fn server_command(mut arguments: VecDeque<String>) -> Result<()> {
    let listen: SocketAddr = option(&mut arguments, "--listen")
        .unwrap_or_else(|| "127.0.0.1:34331".to_owned())
        .parse()
        .context("--listen 必须是 IP:端口")?;
    let database =
        PathBuf::from(option(&mut arguments, "--db").unwrap_or_else(|| "monitor.db".to_owned()));
    ensure_empty(arguments)?;
    server::run(server::ServerOptions { listen, database }).await
}

fn node_command(mut arguments: VecDeque<String>) -> Result<()> {
    let action = arguments
        .pop_front()
        .context("node 后需要 create、revoke 或 list")?;
    let database =
        PathBuf::from(option(&mut arguments, "--db").unwrap_or_else(|| "monitor.db".to_owned()));
    match action.as_str() {
        "create" => {
            let id = required(&mut arguments, "--id")?;
            let name = required(&mut arguments, "--name")?;
            ensure_empty(arguments)?;
            let token = db::create_node(&database, &id, &name)?;
            println!("节点已创建：{name} ({id})");
            println!("节点密钥（仅显示一次）：{token}");
            println!("如果主控正在运行，请重启主控以载入新节点。");
        }
        "revoke" => {
            let id = required(&mut arguments, "--id")?;
            ensure_empty(arguments)?;
            if db::revoke_node(&database, &id)? {
                println!("节点 {id} 已停用；重启主控后生效。");
            } else {
                bail!("节点 {id} 不存在");
            }
        }
        "list" => {
            ensure_empty(arguments)?;
            let nodes = db::list_nodes(&database)?;
            if nodes.is_empty() {
                println!("还没有节点。");
            } else {
                for (id, name, enabled, _) in nodes {
                    println!(
                        "{}\t{}\t{}",
                        id,
                        name,
                        if enabled { "启用" } else { "停用" }
                    );
                }
            }
        }
        other => bail!("未知 node 命令 {other:?}；可用命令：create、revoke、list"),
    }
    Ok(())
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
