//! `landesk-core` — 会话编排层（无 GUI，纯 tokio，对齐 `docs/architecture.md` §3/§4）。
//!
//! M1 骨架：会话状态机 + 配置模型 + 对外事件通道。M4~M7 补全采集→编码→QUIC
//! 流水线（orchestrator）、控制流事件循环、重连调度。
//!
//! M4「双机内网一键连接」落地 [`bootstrap`]（共享密码引导密钥）与 [`oneclick`]
//! （mdns 发现/广播 → QUIC 双通道 → bootstrap GCM 握手的端到端编排，产出
//! [`OneClickSession`]，即「键鼠可用」最小闭环）。
//!
//! M5「鉴权 + 双向剪贴板 + 注入全链路」在其上落地：[`auth`]（ECDH+HKDF 前向保密
//! 会话升级，M4 的 bootstrap 密钥退为认证隧道）、[`input`]（控制帧 → 受控端
//! 注入器全链路，FR-06）、[`clipboard_flow`]（双向剪贴板分段传输 + SHA 校验，
//! FR-07）。
//!
//! M7「中继兜底 + 异常恢复」落地 [`reconnect`]（QUIC 断连指数退避 1/2/4/8s×5，
//! 对齐 `architecture.md` §4.1 状态机 + §4.2 异常恢复矩阵；配合 `quic-link/relay`
//! 的字节透传与 `apps/relay-server`，构成 FR-11 中继兜底链路）。

pub mod session;
pub mod config;
pub mod events;
pub mod bootstrap;
pub mod auth;
pub mod error;
pub mod oneclick;
pub mod input;
pub mod clipboard_flow;
pub mod reconnect;

pub use auth::{ClientHandshake, ServerHandshake, SessionKeys, session_keys};
pub use bootstrap::{derive as derive_bootstrap, Bootstrap};
pub use clipboard_flow::{ClipboardFlow, receive_clipboard};
pub use error::CoreError;
pub use input::{apply_input, DispatchResult};
pub use oneclick::{Controlled, Controller, OneClickSession};
pub use reconnect::{BackoffPolicy, ReconnectScheduler};
pub use session::SessionState;
