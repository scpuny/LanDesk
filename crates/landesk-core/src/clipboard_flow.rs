//! M5 双向剪贴板编排（FR-07，对齐 `docs/architecture.md` §3 landesk-core `control.rs` 的剪贴板分支）。
//!
//! 把 `lp::clipboard` 的分段 / 重组 + `clipboard::ClipSyncer` 的回环抑制，接到
//! [`OneClickSession`] 的控制流上：
//!
//! - 发送：[`ClipboardFlow::send_text`] —— `ClipStart → ClipChunk* → ClipEnd`；
//! - 接收：[`ClipboardFlow::recv_clipboard`] —— 驱动控制流读帧，喂
//!   [`lp::clipboard::ClipboardReassembler`]，校验 SHA-256 后还原文本。
//!
//! 回环保护（本端写入 → 400ms 内不再触发对端同步）由 `clipboard::ClipSyncer`
//! 承担；本模块只管「一次剪贴板内容」的传输与重组。

use lp::clipboard::{split_clipboard, ClipboardReassembler};
use lp::msg::{
    CLIP_TOTAL_CAP, ClipboardEnd, ClipboardStart, ControlMsg,
};

use crate::error::CoreError;
use crate::oneclick::OneClickSession;

/// 一次剪贴板传输的接收状态机。
pub struct ClipboardFlow {
    rx: ClipboardReassembler,
    /// 本次传输的总长（`ClipStart` 携带；末段校验用）。
    total_len: u32,
    /// 是否正在接收一次剪贴板传输。
    active: bool,
}

impl Default for ClipboardFlow {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardFlow {
    pub fn new() -> Self {
        Self {
            rx: ClipboardReassembler::default(),
            total_len: 0,
            active: false,
        }
    }

    /// 一次剪贴板是否正在接收中。
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// 发送一段文本剪贴板（≤1MB）：`ClipStart → ClipChunk* → ClipEnd`。
    ///
    /// 超出 [`CLIP_TOTAL_CAP`] 由 `split_clipboard` 报 `ClipboardTooLarge`。
    pub async fn send_text(&mut self, session: &mut OneClickSession, text: &str) -> Result<(), CoreError> {
        let bytes = text.as_bytes();
        if (bytes.len() as u32) > CLIP_TOTAL_CAP {
            return Err(CoreError::Frame(lp::LpError::ClipboardTooLarge(bytes.len())));
        }

        session
            .send_control(&ControlMsg::ClipStart(ClipboardStart {
                total_len: bytes.len() as u32,
                mime_flags: 0,
            }))
            .await?;

        for chunk in split_clipboard(bytes)? {
            session
                .send_control(&ControlMsg::ClipChunk(chunk))
                .await?;
        }

        let end = ClipboardEnd {
            sha256: ClipboardReassembler::digest_of(bytes),
        };
        session.send_control(&ControlMsg::ClipEnd(end)).await?;
        Ok(())
    }

    /// 读入一条剪贴板控制帧（`ClipStart` / `ClipChunk` / `ClipEnd`）。
    ///
    /// 返回 `Some(text)` 表示一次完整传输已收齐（SHA 校验通过）；`None` 表示
    /// 本次读到的不是剪贴板帧（调用方应把它派发给其它控制分支）。传输途中
    /// 的分段错误（乱序 / 超大 / 校验失败）向上抛 [`CoreError::Frame`]。
    pub async fn feed(
        &mut self,
        session: &mut OneClickSession,
    ) -> Result<Option<String>, CoreError> {
        let msg = session.recv_control().await?;
        match msg {
            ControlMsg::ClipStart(s) => {
                self.rx.on_start(&s)?;
                self.total_len = s.total_len;
                self.active = true;
                Ok(None)
            }
            ControlMsg::ClipChunk(c) => {
                self.rx.on_chunk(&c)?;
                Ok(None)
            }
            ControlMsg::ClipEnd(e) => {
                if !self.active {
                    return Err(CoreError::Frame(lp::LpError::Reassembly(
                        "clip end before start".into(),
                    )));
                }
                let text = self.rx.on_end(&e)?;
                self.active = false;
                self.total_len = 0;
                self.rx = ClipboardReassembler::default();
                Ok(Some(text))
            }
            _ => Ok(None),
        }
    }
}

/// 便捷：接收一次完整剪贴板传输（从 `ClipStart` 到 `ClipEnd`）。
///
/// 循环 `feed` 直到拿到 `Some`；途中非剪贴板帧（心跳等）会被静默跳过——
/// 生产事件循环应把「非剪贴板帧」按各自分支处理，本函数为测试 / 简化场景提供
/// 「阻塞取一次完整剪贴板」的便利封装。
pub async fn receive_clipboard(
    flow: &mut ClipboardFlow,
    session: &mut OneClickSession,
) -> Result<String, CoreError> {
    loop {
        if let Some(text) = flow.feed(session).await? {
            return Ok(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_flow_is_idle() {
        let f = ClipboardFlow::new();
        assert!(!f.is_active());
    }
}
