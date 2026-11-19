//! QUIC 中继服务（FR-11，独立二进制，ADR-A3 仅字节透传、不解密不存会话状态）。
//!
//! M1 占位。M7 接入 quinn 中继：P2P 打洞失败时按信令 ticket 撮合两端 QUIC 连接，
//! 原始字节透传（会话密钥 E2E，中继不可见画面）。

fn main() {
    tracing_subscriber::fmt().init();
    tracing::info!(target: "relay-server", "中继服务骨架启动（M7 接入 QUIC 透传）");
}
