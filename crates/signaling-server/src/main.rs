//! 轻信令服务（FR-10，独立二进制）。
//!
//! M1 占位：服务骨架。M6 接入 axum + WebSocket，内存 `HashMap<device_id, conn>` 存在线设备
//! （FR-10.2 无数据库），只交换 ICE 候选 / 设备ID，零画面流量（FR-10.2）。

fn main() {
    tracing_subscriber::fmt().init();
    tracing::info!("signaling-server 骨架启动（M6 接入 WebSocket 撮合）");
    // M6：axum Router + /ws 端点，内存在线表，reg/req/peer/ice/relay 撮合。
}
