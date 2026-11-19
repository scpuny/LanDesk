//! 受控端 CLI / 服务（形态A独立单文件二进制，ADR-A6 双角色同二进制）。
//!
//! M1 占位：参数解析 + 配置加载骨架。M4~M8 补全会话编排与采集/编码/QUIC 流水线。

use clap::Parser;
use landesk_core::config::Config;

/// LanDesk 受控端。
#[derive(Parser, Debug)]
#[command(name = "controlled", about = "LanDesk 受控端（被远程访问的主机）")]
struct Cli {
    /// 配置文件路径（`~/.config/landesk/config.toml`）。
    #[arg(long)]
    config: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let _config = Config::default();
    let _ = cli.config;
    tracing_subscriber::fmt().init();
    tracing::info!(target: "controlled", "受控端骨架启动（M4 接入会话编排）");
}
