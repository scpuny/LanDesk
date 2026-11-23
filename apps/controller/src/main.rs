//! 控制端 CLI（形态A独立单文件二进制，ADR-A6）。
//!
//! **现状：形态A 早期骨架，尚未接入会话编排。** 参数解析可用；连接 / 预览窗口 /
//! 键鼠输入（M4~M8）均未实现。控制端**主能力已由 Tauri GUI（`ui` crate +
//! `crates/ui::SessionManager`）承担**——GUI 完成 mDNS 发现、QUIC 会话、视频数据面、
//! 键鼠注入、剪贴板同步的端到端闭环。此 CLI 仅为形态A（无 GUI 的单文件二进制）保留
//! 参数入口占位；若需维持形态A，须把 GUI 的采集/编码/会话编排接进来（后续里程碑）。

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
    tracing::info!(target: "controller", ?cli, "控制端骨架启动（形态A 早期骨架；会话编排未接入，主能力见 Tauri GUI）");
}
