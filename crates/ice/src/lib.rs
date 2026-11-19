//! `ice` — ICE/STUN NAT 打洞（FR-05.2，对齐 `docs/protocol.md` §3）。
//!
//! M6 落地：RFC 5389 绑定请求/响应编解码（[`stun`]，自实现无第三方 STUN 依赖）
//! + 候选类型/打洞结果枚举。并发打洞调度接入应用层。

pub mod stun;

/// ICE 候选类型（对齐 protocol.md §3.2 `cand.t` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    Host,
    Srflx,
    Prflx,
    Relay,
}

impl CandidateType {
    /// 信令 wire 字符串（`cand.t`，protocol.md §3.1）。
    pub const fn as_wire(self) -> &'static str {
        match self {
            CandidateType::Host => "host",
            CandidateType::Srflx => "srflx",
            CandidateType::Prflx => "prflx",
            CandidateType::Relay => "relay",
        }
    }

    /// 从 wire 字符串解析；未知值返回 `None`（协议演进预留，§7 容错忽略）。
    pub fn from_wire(s: &str) -> Option<Self> {
        Some(match s {
            "host" => CandidateType::Host,
            "srflx" => CandidateType::Srflx,
            "prflx" => CandidateType::Prflx,
            "relay" => CandidateType::Relay,
            _ => return None,
        })
    }
}

/// 简化 ICE 候选（不引入完整 SDP，降低复杂度）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IceCandidate {
    pub ty: CandidateType,
    pub ip: String,
    pub port: u16,
    pub priority: u16,
    pub proto: String,
}

/// 打洞结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoleResult {
    /// P2P 直连成功。
    Direct,
    /// 需走中继。
    NeedRelay,
    /// 无可用路径。
    NoPath,
}
