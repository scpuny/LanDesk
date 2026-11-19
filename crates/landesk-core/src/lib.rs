//! `landesk-core` — 会话编排层（无 GUI，纯 tokio，对齐 `docs/architecture.md` §3/§4）。
//!
//! M1 骨架：会话状态机 + 配置模型 + 对外事件通道。M4~M7 补全采集→编码→QUIC
//! 流水线（orchestrator）、控制流事件循环、重连调度。
//!
//! M4「双机内网一键连接」落地 [`bootstrap`]（共享密码引导密钥）与 [`oneclick`]
//! （mdns 发现/广播 → QUIC 双通道 → bootstrap GCM 握手的端到端编排，产出
//! [`OneClickSession`]，即「键鼠可用」最小闭环）。

pub mod session;
pub mod config;
pub mod events;
pub mod bootstrap;
pub mod error;
pub mod oneclick;

pub use bootstrap::{derive as derive_bootstrap, Bootstrap};
pub use error::CoreError;
pub use oneclick::{Controlled, Controller, OneClickSession};
pub use session::SessionState;
