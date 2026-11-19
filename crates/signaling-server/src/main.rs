//! 轻信令服务（FR-10，独立二进制）。
//!
//! M1 占位：服务骨架。M6 接入 axum + WebSocket，内存 `HashMap<device_id, conn>` 存在线设备
//! （FR-10.2 无数据库），只交换 ICE 候选 / 设备ID，零画面流量（FR-10.2）。

fn main() {
    tracing_subscriber::fmt().init();
    tracing::info!("signaling-server 骨架启动（M6 接入 WebSocket 撮合）");

    // TODO(M3 尾巴 → M6): 信令服务仍是骨架，尚未接入 axum + WebSocket。
    // 原因：
    //   1. 信令撮合依赖 ICE 候选生成（`ice` crate），而 ICE/STUN 打洞属 M6 范围，
    //      现在实现撮合逻辑没有可联调的对端，只能写死骨架；
    //   2. 数据面（QUIC 双通道）已在 M3 落地，信令只需交换"对方 QUIC 端点 +
    //      ICE 候选"，撮合逻辑与数据面解耦，可推迟到 M6 与 ICE 一起交付；
    //   3. 中继兜底（`relay` 消息 + apps/relay-server）属 M7，进一步说明
    //      信令服务的完整形态要等 M6/M7 才收敛。
    // M6 实施清单：axum Router + /ws 端点；内存在线表 HashMap<device_id, Sink>；
    //             reg/req/peer/ice/relay 五类消息撮合；心跳 + 断线清理。
    // M6：axum Router + /ws 端点，内存在线表，reg/req/peer/ice/relay 撮合。
}

