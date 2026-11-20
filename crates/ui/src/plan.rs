//! 连接路径决策链（FR-05 / FR-11，对齐 `docs/architecture.md` §3 降级顺序 + PRD §12 核心场景）。
//!
//! 控制端按「内网 → P2P 打洞 → 中继兜底」的**严格优先级**选路径；打洞失败且无中继
//! （用户关掉 FR-11.2）→ 拒绝连接（`NoPath`）。纯函数，可离线单测（`ui/frontend`
//! 与 `landesk-core` 编排共用同一决策逻辑，保证「打洞失败自动切中继」行为一致）。

use ice::HoleResult;
use quic_link::ConnMode;

/// 路径决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

/// 双宿主共享的路径决策**视图**（REST `/api/choose-path` 出参 + Tauri `choose_path`
/// 命令出参**同形**，P1.3 归一——见 `docs/ui-backend-matrix.md` §5.4：此前 Tauri 裸
/// `PathDecision` 出 `{Use:{Inner}}`（PascalCase 嵌套枚举）、REST 出 `{use:{conn_mode}}`，
/// 两宿主形状漂移，前端要写两套解析）。
///
/// 用**扁平结构体** + `camelCase` 对齐前端 `PathDecision`：
/// - 有路径 → `{"connMode":"p2p"}`（`conn_mode` 小写，对齐前端 `ConnMode`）；
/// - 无路径 → `{"nopath":true}`。
///
/// 两宿主都经 [`PathDecisionView::from`] 出它，字节一致；前端一套解析即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathDecisionView {
    /// 选定连接方式（`inner`/`p2p`/`relay`）；无路径时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conn_mode: Option<&'static str>,
    /// 是否无可用路径（打洞失败且中继不可用/关闭，FR-11.2）；`false` 时省略。
    #[serde(default, skip_serializing_if = "skip_false")]
    pub nopath: bool,
}

/// `skip_serializing_if`：`nopath` 为 `false` 时省略（保持「有路径」出参干净）。
fn skip_false(b: &bool) -> bool {
    !b
}

impl PathDecisionView {
    /// 由内核决策 [`PathDecision`] 归一为共享视图（两宿主统一走这里）。
    pub fn from(d: &PathDecision) -> Self {
        match d {
            PathDecision::Use(m) => PathDecisionView {
                conn_mode: Some(match m {
                    ConnMode::Inner => "inner",
                    ConnMode::P2p => "p2p",
                    ConnMode::Relay => "relay",
                }),
                nopath: false,
            },
            PathDecision::NoPath => PathDecisionView {
                conn_mode: None,
                nopath: true,
            },
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

    #[test]
    fn view_serializes_flat_camel_case_for_frontend() {
        // 双宿主共享视图：扁平 camelCase，conn_mode 小写（P1.3 归一）。
        let v = PathDecisionView::from(&decide_path(false, &HoleResult::Direct, false));
        let json = serde_json::to_value(v).expect("serialize");
        assert_eq!(json, serde_json::json!({ "connMode": "p2p" }), "有路径 → connMode 小写、无 nopath 键");

        let no = PathDecisionView::from(&decide_path(false, &HoleResult::NoPath, false));
        let json = serde_json::to_value(no).expect("serialize");
        assert_eq!(json, serde_json::json!({ "nopath": true }), "无路径 → nopath:true、无 connMode 键");
    }
}
