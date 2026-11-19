//! 连接路径决策链（FR-05 / FR-11，对齐 `docs/architecture.md` §3 降级顺序 + PRD §12 核心场景）。
//!
//! 控制端按「内网 → P2P 打洞 → 中继兜底」的**严格优先级**选路径；打洞失败且无中继
//! （用户关掉 FR-11.2）→ 拒绝连接（`NoPath`）。纯函数，可离线单测（`ui/frontend`
//! 与 `landesk-core` 编排共用同一决策逻辑，保证「打洞失败自动切中继」行为一致）。

use ice::HoleResult;
use quic_link::ConnMode;

/// 路径决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathDecision {
    /// 用该路径连（`ConnMode` 喂状态栏 + QUIC 拨号）。
    Use(ConnMode),
    /// 无可用路径（打洞失败且中继不可用/关闭）——拒绝连接（FR-11.2）。
    NoPath,
}

/// 按降级链选路径：
/// 1. 内网发现（`inner`）→ [`ConnMode::Inner`]（延迟最低，优先）；
/// 2. P2P 打洞成功（`hole` = `Direct`）→ [`ConnMode::P2p`]；
/// 3. 中继可用（`relay_enabled`）→ [`ConnMode::Relay`]（打洞 `NeedRelay`/`NoPath` 都兜底）；
/// 4. 否则 [`PathDecision::NoPath`]。
///
/// `hole` 为本次 ICE 打洞结果；内网已通时 `hole` 不参与（直接 `Inner`）。
pub fn decide_path(inner: bool, hole: &HoleResult, relay_enabled: bool) -> PathDecision {
    if inner {
        return PathDecision::Use(ConnMode::Inner);
    }
    match hole {
        HoleResult::Direct => PathDecision::Use(ConnMode::P2p),
        // 需中继 / 无路径：只要中继开就兜底（FR-05.3 / FR-11.1）。
        HoleResult::NeedRelay | HoleResult::NoPath => {
            if relay_enabled {
                PathDecision::Use(ConnMode::Relay)
            } else {
                PathDecision::NoPath
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_beats_all() {
        // 内网发现在手 → 无论打洞/中继，走内网（最低延迟）。
        assert_eq!(decide_path(true, &HoleResult::Direct, true), PathDecision::Use(ConnMode::Inner));
        assert_eq!(decide_path(true, &HoleResult::NoPath, false), PathDecision::Use(ConnMode::Inner));
    }

    #[test]
    fn p2p_when_hole_direct() {
        assert_eq!(decide_path(false, &HoleResult::Direct, false), PathDecision::Use(ConnMode::P2p));
    }

    #[test]
    fn relay_when_enabled_regardless_of_hole() {
        assert_eq!(decide_path(false, &HoleResult::NeedRelay, true), PathDecision::Use(ConnMode::Relay));
        // 打洞彻底失败（NoPath）但中继开 → 仍走中继兜底（FR-11.1）。
        assert_eq!(decide_path(false, &HoleResult::NoPath, true), PathDecision::Use(ConnMode::Relay));
    }

    #[test]
    fn nopath_when_hole_fails_and_relay_off() {
        // 打洞失败 + 用户关掉中继（FR-11.2 可关）→ 拒绝连接。
        assert_eq!(decide_path(false, &HoleResult::NoPath, false), PathDecision::NoPath);
        assert_eq!(decide_path(false, &HoleResult::NeedRelay, false), PathDecision::NoPath);
    }
}
