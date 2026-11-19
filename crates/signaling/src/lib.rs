//! `signaling` — 轻信令客户端（FR-10，对齐 `docs/protocol.md` §3）。
//!
//! M6 落地：信令消息类型 + 连接状态 + tokio-tungstenite 客户端（[`Client`]），
//! 完成「注册 → 请求 → 撮合下发对方端点 + ICE 候选」的最小联通闭环
//! （`docs/architecture.md` §3 landesk-core 信令分支）。

use ice::{HoleResult, IceCandidate};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite;

/// 信令消息（JSON envelope，protocol.md §3.1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum SignalingMsg {
    /// 受控端注册。
    #[serde(rename = "reg")]
    Reg {
        id: String,
        name: String,
        caps: Vec<String>,
        res: String,
        relay: bool,
    },
    /// 控制端请求连接。
    #[serde(rename = "req")]
    Req { id: String, from: String },
    /// 撮合后下发对方端点。
    #[serde(rename = "peer")]
    Peer { id: String, peer: String, ice: Vec<IceCandidateWire> },
    /// ICE 候选增量。
    #[serde(rename = "ice")]
    Ice { from: String, to: String, cand: IceCandidateWire },
    /// 中继兜底撮合。
    #[serde(rename = "relay")]
    Relay {
        to: String,
        ep: String,
        ticket: String,
    },
}

/// ICE 候选的 wire 形式（JSON 扁平字段，对齐 protocol.md §3.1 `cand`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IceCandidateWire {
    pub t: String,
    pub ip: String,
    pub p: u16,
    pub pr: u16,
    pub proto: String,
}

impl From<IceCandidate> for IceCandidateWire {
    fn from(c: IceCandidate) -> Self {
        Self {
            t: c.ty.as_wire().to_string(),
            ip: c.ip,
            p: c.port,
            pr: c.priority,
            proto: c.proto,
        }
    }
}

/// 反向：信令 wire 候选 → 协议候选（`cand.t` 未知值 → `None`，§7 容错忽略）。
impl From<IceCandidateWire> for IceCandidate {
    fn from(w: IceCandidateWire) -> Self {
        Self {
            ty: ice::CandidateType::from_wire(&w.t).unwrap_or(ice::CandidateType::Host),
            ip: w.ip,
            port: w.p,
            priority: w.pr,
            proto: w.proto,
        }
    }
}

/// 信令客户端错误（连接 / 序列化 / 对端关闭）。
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SignalingError {
    #[error("signaling connect failed: {0}")]
    Connect(String),
    #[error("signaling io: {0}")]
    Io(String),
    #[error("signaling protocol: {0}")]
    Protocol(String),
}

/// 信令连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalState {
    Disconnected,
    Connecting,
    Online(HoleResult),
    Relay,
}

/// 客户端上行消息（包装 [`SignalingMsg`]，标记方向）。
#[derive(Debug, Clone, PartialEq)]
pub enum ClientOut {
    /// 注册本设备。
    Reg(SignalingMsg),
    /// 请求连接目标。
    Req(SignalingMsg),
    /// 增量下发本端 ICE 候选。
    Ice(SignalingMsg),
}

impl ClientOut {
    /// 取出内层消息（供序列化 / 匹配）。
    pub fn into_msg(self) -> SignalingMsg {
        match self {
            ClientOut::Reg(m) | ClientOut::Req(m) | ClientOut::Ice(m) => m,
        }
    }
}

/// 客户端下行消息（服务端撮合结果，归一化后）。
#[derive(Debug, Clone, PartialEq)]
pub enum ClientIn {
    /// 撮合成功，下发对方端点 + 候选。
    Peer(SignalingMsg),
    /// 收到对端增量候选。
    Ice(SignalingMsg),
    /// 中继兜底撮合。
    Relay(SignalingMsg),
    /// 未识别 / 非下行类型（容错忽略，§7）。
    Unknown(SignalingMsg),
}

/// 轻信令客户端（FR-10，tokio-tungstenite + JSON envelope）。
///
/// 与信令服务一一对应的 WS 连接；`reg`/`req`/`ice` 上行，`peer`/`ice`/`relay` 下行。
/// 持 [`WsStream`]（同时实现 `Sink<Message>` 与 `Stream<Item = Result<Message>>`）。
pub struct Client {
    socket: WsStream,
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

impl Client {
    /// 建立 WS 连接（`ws://host:port/ws`）。
    pub async fn connect(url: &str) -> Result<Self, SignalingError> {
        let (ws_stream, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| SignalingError::Connect(e.to_string()))?;
        Ok(Self { socket: ws_stream })
    }

    /// 发送一条上行消息（序列化为 JSON 文本帧）。
    pub async fn send(&mut self, out: &ClientOut) -> Result<(), SignalingError> {
        use futures_util::SinkExt;
        let raw = serde_json::to_string(&out.clone().into_msg())
            .map_err(|e| SignalingError::Protocol(e.to_string()))?;
        self.socket
            .send(tungstenite::Message::text(raw))
            .await
            .map_err(|e| SignalingError::Protocol(e.to_string()))?;
        Ok(())
    }

    /// 读一条下行消息（阻塞到下一条；跳过 Ping/Pong）。
    pub async fn recv(&mut self) -> Result<ClientIn, SignalingError> {
        use futures_util::StreamExt;
        loop {
            let Some(frame) = self.socket.next().await else {
                return Err(SignalingError::Protocol("stream ended".into()));
            };
            let frame = frame.map_err(|e| SignalingError::Protocol(e.to_string()))?;
            match frame {
                tungstenite::Message::Text(t) => {
                    let msg: SignalingMsg =
                        serde_json::from_str(&t).map_err(|e| SignalingError::Protocol(e.to_string()))?;
                    return Ok(classify_incoming(&msg));
                }
                tungstenite::Message::Close(_) => {
                    return Err(SignalingError::Protocol("closed".into()))
                }
                _ => continue, // Ping/Pong 等跳过
            }
        }
    }

    /// 优雅关闭。
    pub async fn close(&mut self) {
        use futures_util::SinkExt;
        let _ = self.socket.send(tungstenite::Message::Close(None)).await;
    }
}

/// 把下行 [`SignalingMsg`] 归入 [`ClientIn`]（未知类型 → `Unknown`，容错忽略）。
fn classify_incoming(m: &SignalingMsg) -> ClientIn {
    match m {
        SignalingMsg::Peer { .. } => ClientIn::Peer(m.clone()),
        SignalingMsg::Ice { .. } => ClientIn::Ice(m.clone()),
        SignalingMsg::Relay { .. } => ClientIn::Relay(m.clone()),
        _ => ClientIn::Unknown(m.clone()),
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn reg_serialization() {
        let m = SignalingMsg::Reg {
            id: "dev-8f3a".into(),
            name: "devboard-ok3568".into(),
            caps: vec!["qsv".into(), "openh264".into()],
            res: "1920x1080".into(),
            relay: true,
        };
        let s = serde_json::to_string(&m).unwrap();
        assert!(s.contains("\"type\":\"reg\""));
        let back: SignalingMsg = serde_json::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn candidate_wire_roundtrip() {
        let c = IceCandidate {
            ty: ice::CandidateType::Srflx,
            ip: "1.2.3.4".into(),
            port: 4433,
            priority: 100,
            proto: "udp".into(),
        };
        let w: IceCandidateWire = c.clone().into();
        assert_eq!(w.t, "srflx");
        let s = serde_json::to_string(&w).unwrap();
        let back: IceCandidateWire = serde_json::from_str(&s).unwrap();
        assert_eq!(back, w);
    }
}
