//! `landesk-ui` — LanDesk GUI 服务层（M8，对齐 `docs/architecture.md` §3 `ui/` + PRD FR-08/09/11）。
//!
//! 形态 B（Tauri 前端）的**可离线验证的服务层**：把 `landesk-core` 的会话/重连/配置
//! 能力归一成纯逻辑 + 可序列化模型，供 Tauri 命令桥（`ui/src-tauri`）与 Vue 前端
//! （`ui/frontend`）对接。本 crate **不依赖 tauri 本体**（其传递依赖不在离线 vendor），
//! 只复用已在 vendor 的 `landesk-core`/`mdns`/`ice`/`quic-link`/`serde`。
//!
//! 模块：
//! - [`status`]：状态栏视图模型（FR-09.3，`CoreEvent` → 可序列化快照）；
//! - [`plan`]：连接路径决策链（FR-05/FR-11，内网→P2P→中继→NoPath）；
//! - [`devices`]：设备列表模型（FR-01.2/01.4，TTL 剪枝 + 排序行视图）；
//! - [`config_store`]：配置页持久化（FR-08，默认路径 + 首次生成模板）；
//! - [`reconnect_view`]：重连指示器（FR-05.4，接 M7 `ReconnectScheduler`）。

pub mod config_store;
pub mod devices;
pub mod local_info;
pub mod logs;
pub mod plan;
pub mod reconnect_view;
pub mod status;

pub use config_store::{ConfigStore, StoreError, default_config_path};
pub use devices::{device_rows, DeviceRow, DeviceTable, now_secs};
pub use local_info::{
    device_code_from, gen_temp_password, local_info_of, LocalInfo,
};
pub use logs::{LevelFilter, LogLine, LogRing, DEFAULT_LOG_CAPACITY};
pub use plan::{decide_path, PathDecision, PathDecisionView};
pub use reconnect_view::{ReconnectSchedule, ReconnectView, schedule_of, view_of};
pub use status::{StatusBarModel, conn_mode_label, encoder_label, state_label};

// ---------- 命令内核便捷 re-export（Tauri 壳只依赖 `landesk-ui` 即可取到全部业务类型）----------
pub use ice::HoleResult;
pub use landesk_core::config::{Config, ConfigError};
pub use landesk_core::events::CoreEvent;
pub use landesk_core::reconnect::{BackoffPolicy, ReconnectScheduler};
pub use landesk_core::SessionState;
pub use mdns::OnlineDevice;
pub use mdns::Listener;
pub use quic_link::ConnMode;

