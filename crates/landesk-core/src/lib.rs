//! `landesk-core` — 会话编排层（无 GUI，纯 tokio，对齐 `docs/architecture.md` §3/§4）。
//!
//! M1 骨架：会话状态机 + 配置模型 + 对外事件通道。M4~M7 补全采集→编码→QUIC
//! 流水线（orchestrator）、控制流事件循环、重连调度。

pub mod session;
pub mod config;
pub mod events;
