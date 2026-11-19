//! `signaling` — 轻信令客户端（FR-10，对齐 `docs/protocol.md` §3）。
//!
//! M1 占位：信令消息类型 + 连接状态。M6 接入 tokio-tungstenite 与 STUN 反射。

use ice::{HoleResult, IceCandidate};
use serde::{Deserialize, Serialize};

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
        use ice::CandidateType::*;
        Self {
            t: match c.ty {
                Host => "host".into(),
                Srflx => "srflx".into(),
                Prflx => "prflx".into(),
                Relay => "relay".into(),
            },
            ip: c.ip,
            p: c.port,
            pr: c.priority,
            proto: c.proto,
        }
    }
}

/// 信令连接状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalState {
    Disconnected,
    Connecting,
    Online(HoleResult),
    Relay,
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
