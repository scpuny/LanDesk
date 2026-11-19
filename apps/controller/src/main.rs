//! 控制端 CLI（形态A独立单文件二进制，ADR-A6）。
//!
//! M1 占位：参数解析骨架。M4~M8 补全连接、预览窗口调用与键鼠输入。

use clap::Parser;

/// LanDesk 控制端。
#[derive(Parser, Debug)]
#[command(name = "controller", about = "LanDesk 控制端（远程访问的主机）")]
struct Cli {
    /// 目标设备ID（外网模式）或设备名（内网）。
    #[arg(long)]
    target: Option<String>,
    /// 信令地址（wss://host:port，外网模式）。
    #[arg(long)]
    signaling: Option<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    tracing_subscriber::fmt().init();
    tracing::info!(target: "controller", ?cli, "控制端骨架启动（M4 接入会话编排）");
}
