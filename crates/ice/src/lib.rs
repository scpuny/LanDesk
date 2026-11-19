//! `ice` — ICE/STUN NAT 打洞（FR-05.2，对齐 `docs/protocol.md` §3）。
//!
//! M1 占位：候选类型 + 打洞结果枚举。M6 接入 RFC 5389 绑定请求与并发打洞调度。

/// ICE 候选类型（对齐 protocol.md §3.2 `cand.t` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateType {
    Host,
    Srflx,
    Prflx,
    Relay,
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
