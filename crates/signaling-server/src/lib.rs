//! `signaling-server` — 轻信令服务（FR-10，M6 落地，对齐 `docs/architecture.md` §3 / `protocol.md` §3）。
//!
//! - 传输：axum `/ws`（WebSocket + JSON envelope），**只传信令，零画面流量**（FR-10.2）；
//! - 存储：内存 [`Registry`]（`HashMap<device_id, 项>`），无数据库（FR-10.2）；
//! - 撮合：`reg` 注册 → `req` 请求 → 双方各收一条 `peer`（含对方端点 + ICE 候选槽）；
//!   `ice` 增量候选转发目标；`relay` 兜底（M7 中继撮合，MVP 预留）；
//! - 断线清理：连接任务退出即从在线表摘除；
//! - 内嵌 STUN 反射器（§3.2）：[`run_stun`] 起 UDP 反射，避免第三方 STUN（NFR-03）。
//!
//! 撮合逻辑（[`match_action`]）从 socket 循环剥出，离线可单测；完整 WS 闭环用
//! `tokio-tungstenite` 环回联调（`tests/`）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::Router;
use futures_util::StreamExt;
use ice::stun::StunServer;
use signaling::SignalingMsg;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};

/// 在线设备注册项（内存，FR-10.2 无数据库）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceReg {
    pub name: String,
    pub caps: Vec<String>,
    pub res: String,
    pub relay: bool,
}

/// 在线连接句柄：mpsc 发送端（连接任务退出即 `drop`，从表摘除 = 断线清理）。
#[derive(Clone, Debug)]
pub struct PeerHandle {
    pub tx: mpsc::UnboundedSender<SignalingMsg>,
}

impl PeerHandle {
    /// 经共享句柄发一条消息（克隆发送端，避免 `&mut` 借用）。
    /// 通道满 / 对端已断返回 `false`，调用方静默忽略（断线由清理兜底）。
    pub fn send(&self, msg: &SignalingMsg) -> bool {
        self.tx.send(msg.clone()).is_ok()
    }
}

/// 内存在线表（`HashMap<device_id, (注册项, 连接句柄)>`）。
#[derive(Debug, Default)]
pub struct Registry {
    devices: HashMap<String, (DeviceReg, PeerHandle)>,
}

impl Registry {
    /// 注册 / 更新一个在线设备（重注册幂等；句柄用本连接的发送端）。
    pub fn upsert(&mut self, id: &str, reg: DeviceReg, handle: PeerHandle) {
        self.devices.insert(id.to_string(), (reg, handle));
    }

    /// 断线清理。
    pub fn remove(&mut self, id: &str) {
        self.devices.remove(id);
    }

    /// 目标是否在线（撮合前置检查）。
    pub fn is_online(&self, id: &str) -> bool {
        self.devices.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.devices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// 取目标在线句柄（撮合 / 候选转发用）。
    pub fn handle(&self, id: &str) -> Option<&PeerHandle> {
        self.devices.get(id).map(|(_, h)| h)
    }
}

/// 撮合动作：处理一条**非 reg** 上行消息，返回「要发给**当前连接**」的下行消息。
///
/// - `req{id, from}`：`from`=请求方（= `me`），`id`=目标。目标在线 → 双方各发
///   `Peer`（`ice` 槽留空，真实候选经 `ice` 消息交换，FR：信令不做画面流量）；
/// - `ice{from, to, cand}`：候选转发给 `to`；
/// - `relay{to, ep, ticket}`：中继兜底转发给 `to`（M7 撮合）。
pub fn match_action(
    registry: &mut Registry,
    me: &str,
    msg: &SignalingMsg,
) -> Vec<SignalingMsg> {
    match msg {
        SignalingMsg::Req { id: tgt, from } => {
            if from == me && registry.is_online(tgt) {
                let for_me = SignalingMsg::Peer {
                    id: from.clone(),
                    peer: tgt.clone(),
                    ice: Vec::new(), // 候选经 ice 消息交换
                };
                let for_target = SignalingMsg::Peer {
                    id: tgt.clone(),
                    peer: from.clone(),
                    ice: Vec::new(),
                };
                if let Some(t) = registry.handle(tgt) {
                    t.send(&for_target);
                }
                return vec![for_me];
            }
            Vec::new()
        }
        SignalingMsg::Ice { from: _, to, cand } => {
            if let Some(t) = registry.handle(to) {
                t.send(&SignalingMsg::Ice {
                    from: me.to_string(),
                    to: to.clone(),
                    cand: cand.clone(),
                });
            }
            Vec::new()
        }
        SignalingMsg::Relay { to, ep, ticket } => {
            if let Some(t) = registry.handle(to) {
                t.send(&SignalingMsg::Relay {
                    to: to.clone(),
                    ep: ep.clone(),
                    ticket: ticket.clone(),
                });
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// 构造路由（`/ws` + `/health`）。
pub fn router(registry: Arc<Mutex<Registry>>) -> Router {
    use axum::routing::get;
    Router::new()
        .route(
            "/ws",
            get(move |ws: WebSocketUpgrade| {
                let registry = registry.clone();
                async move {
                    ws.on_upgrade(move |socket| async move {
                        run_session(registry, socket).await;
                    })
                }
            }),
        )
        .route("/health", get(|| async { "ok" }))
}

/// 一次信令会话：循环收发。`reg` 把本连接登记进在线表；此后 `req`/`ice`/`relay`
/// 经 [`match_action`] 撮合；来自其它连接的转发消息经 `out_rx` 下发。
async fn run_session(registry: Arc<Mutex<Registry>>, mut ws: WebSocket) {
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<SignalingMsg>();
    let mut my_id: Option<String> = None;

    // 把本连接句柄先登记到一个占位 ID，reg 到达后迁移到真实 ID。
    // 简化：reg 前不出现在在线表（撮合要求目标已 reg）。

    loop {
        tokio::select! {
            maybe = ws.next() => {
                match maybe {
                    Some(Ok(Message::Text(t))) => {
                        let Ok(msg) = serde_json::from_str::<SignalingMsg>(&t) else {
                            continue;
                        };
                        if let SignalingMsg::Reg { id, name, caps, res, relay } = &msg {
                            my_id = Some(id.clone());
                            registry
                                .lock()
                                .await
                                .upsert(
                                    id,
                                    DeviceReg {
                                        name: name.clone(),
                                        caps: caps.clone(),
                                        res: res.clone(),
                                        relay: *relay,
                                    },
                                    PeerHandle {
                                        tx: out_tx.clone(),
                                    },
                                );
                        }
                        let me = my_id.as_deref().unwrap_or("anon");
                        let outs = {
                            let mut reg = registry.lock().await;
                            match_action(&mut reg, me, &msg)
                        };
                        for o in outs {
                            let Ok(raw) = serde_json::to_string(&o) else {
                                continue;
                            };
                            if ws.send(Message::Text(raw)).await.is_err() {
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    _ => {}
                }
            }
            out = out_rx.recv() => {
                match out {
                    Some(m) => {
                        let Ok(raw) = serde_json::to_string(&m) else {
                            continue;
                        };
                        if ws.send(Message::Text(raw)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }

    // 断线清理：从在线表摘除（FR-10.2 内存在线表）。
    if let Some(id) = my_id {
        registry.lock().await.remove(&id);
    }
}

/// 内嵌 STUN 反射器（§3.2，避免第三方 STUN，NFR-03）：起 UDP 反射直到进程退出。
pub async fn run_stun(addr: SocketAddr) -> std::io::Result<()> {
    let server = StunServer::bind(addr).await?;
    tracing::info!(%addr, "STUN 反射器启动");
    loop {
        match server.serve_one().await {
            Ok(Some(_resp)) => tracing::debug!("STUN 绑定反射"),
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(%e, "STUN 反射器 IO 错误，退出");
                break;
            }
        }
    }
    Ok(())
}

/// 启动信令服务（`/ws` 端点）+ 可选内嵌 STUN（`stun` 传 `None` 不起）。
pub async fn serve(bind: SocketAddr, stun: Option<SocketAddr>) -> std::io::Result<()> {
    let registry = Arc::new(Mutex::new(Registry::default()));
    let app = router(registry.clone());
    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, "signaling-server 启动");
    if let Some(saddr) = stun {
        tokio::spawn(run_stun(saddr));
    }
    axum::serve(listener, app).await?;
    Ok(())
}

/// 测试 / 单测用：纯撮合表（不建 HTTP）。
#[cfg(test)]
pub mod testsupport {
    use super::*;

    /// 预置一个在线设备（给定一个可控的 mpsc 接收端，断言其收到的转发）。
    pub fn online(reg: &mut Registry, id: &str) -> mpsc::UnboundedReceiver<SignalingMsg> {
        let (tx, rx) = mpsc::unbounded_channel();
        reg.upsert(
            id,
            DeviceReg {
                name: id.into(),
                caps: vec!["openh264".into()],
                res: "1920x1080".into(),
                relay: false,
            },
            PeerHandle { tx },
        );
        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use signaling::IceCandidateWire;

    #[test]
    fn req_matches_when_target_online() {
        let mut reg = Registry::default();
        let target_rx = testsupport::online(&mut reg, "dev-1");
        let from = "cli-1";
        let req = SignalingMsg::Req {
            id: "dev-1".into(),
            from: from.into(),
        };
        let out = match_action(&mut reg, from, &req);
        assert_eq!(out.len(), 1, "请求方收到 1 条 Peer");
        match &out[0] {
            SignalingMsg::Peer { id, peer, .. } => {
                assert_eq!(id, from);
                assert_eq!(peer, "dev-1");
            }
            other => panic!("期望 Peer，实际 {other:?}"),
        }
        // 目标也收到一条 Peer（含请求方）。
        let mut rx = target_rx;
        let got = rx.try_recv().expect("目标收到 Peer");
        assert!(matches!(
            got,
            SignalingMsg::Peer {
                peer: ref p,
                ..
            } if p == from
        ));
    }

    #[test]
    fn req_noop_when_target_offline() {
        let mut reg = Registry::default();
        let req = SignalingMsg::Req {
            id: "ghost".into(),
            from: "cli-1".into(),
        };
        assert!(match_action(&mut reg, "cli-1", &req).is_empty(), "目标离线 → 无撮合");
    }

    #[test]
    fn ice_forwarded_to_target() {
        let mut reg = Registry::default();
        let mut target_rx = testsupport::online(&mut reg, "dev-1");
        let cand = IceCandidateWire {
            t: "srflx".into(),
            ip: "1.2.3.4".into(),
            p: 4433,
            pr: 100,
            proto: "udp".into(),
        };
        let ice = SignalingMsg::Ice {
            from: "x".into(),
            to: "dev-1".into(),
            cand: cand.clone(),
        };
        let out = match_action(&mut reg, "cli-1", &ice);
        assert!(out.is_empty(), "ice 不下发给请求方，只转发目标");
        let got = target_rx.try_recv().expect("目标收到 ice");
        assert_eq!(got, SignalingMsg::Ice { from: "cli-1".into(), to: "dev-1".into(), cand }, "转发时 from 改为本连接 me");
    }

    #[test]
    fn registry_upsert_remove() {
        let mut reg = Registry::default();
        let rx = testsupport::online(&mut reg, "a");
        let _ = rx;
        assert!(reg.is_online("a"));
        reg.remove("a");
        assert!(!reg.is_online("a"));
        assert!(reg.is_empty());
    }
}
