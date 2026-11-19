//! `clipboard` — 双向剪贴板同步（FR-07，对齐 `docs/protocol.md` §1.3 剪贴板分段协议）。
//!
//! 传输分段由 `lp::clipboard` 完成（ClipStart/Chunk/End）。本 crate 负责**回环保护**：
//! 本端写入剪贴板后 400ms 内不再触发对端同步，避免双向震荡。

/// 回环抑制窗（毫秒）。本端写入后 N ms 内忽略自身监听到的变化。
pub const BACKRING_MS: u64 = 400;

/// 剪贴板同步器：记录本端最近一次写入时间戳，过滤自身引起的回环。
#[derive(Debug, Default)]
pub struct ClipSyncer {
    /// 本端最近一次 apply 对端数据的时间（epoch ms）。
    last_apply: u64,
}

impl ClipSyncer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 本端用户操作剪贴板（非对端下发）：允许同步。
    pub fn local_change(&mut self, now_ms: u64) -> bool {
        // 本地变化总是允许同步出去（无论是否在回环窗内），
        // 因为这是用户真实操作。
        self.last_apply = 0;
        let _ = now_ms;
        true
    }

    /// 收到对端剪贴板数据，写入本端。返回是否写入。
    pub fn apply_remote(&mut self, now_ms: u64) -> bool {
        self.last_apply = now_ms;
        true
    }

    /// 本端监听到剪贴板变化时，判断是否应同步给对端：
    /// 若处于 apply_remote 的回环窗内 → 抑制（return false）。
    pub fn should_sync(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_apply) >= BACKRING_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backring_suppresses_echo() {
        let mut s = ClipSyncer::new();
        s.apply_remote(1000);
        // 1000~1400ms 内抑制
        assert!(!s.should_sync(1200));
        assert!(!s.should_sync(1399));
        // 1400ms 后放行
        assert!(s.should_sync(1400));
    }

    #[test]
    fn local_change_clears_window() {
        let mut s = ClipSyncer::new();
        s.apply_remote(1000);
        s.local_change(1100);
        assert!(s.should_sync(1100), "本地变化应立即可同步");
    }
}
