//! `capture` — 屏幕采集与脏矩形检测（FR-03，对齐 `docs/architecture.md` §2 `crates/capture`）。
//!
//! M1 阶段目标：先跑通画面采集的**逻辑层**（trait + 脏矩形 + 显示器事件），
//! 平台原生实现（Graphics Capture / PipeWire / CGDisplayStream）按 feature 门控，
//! 后续里程碑补全。本模块保证无平台依赖即可单测。

pub mod dirty;
pub mod monitors;

use std::fmt;

use thiserror::Error;

/// 采集层错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CaptureError {
    #[error("no display available")]
    NoDisplay,
    #[error("display not found: index {0}")]
    DisplayNotFound(u8),
    #[error("unsupported resolution: {w}x{h}")]
    UnsupportedResolution { w: u32, h: u32 },
    #[error("platform backend unavailable: {0}")]
    BackendUnavailable(String),
}

/// 一帧原始画面（平台采集产出，供 swscale 转 NV12）。
#[derive(Debug, Clone)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    /// 像素数据，格式由 `PixelFormat` 决定（M1 默认 BGRA，平台原生输出）。
    pub data: Vec<u8>,
    pub format: PixelFormat,
}

impl RawFrame {
    pub fn new_bgra(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0u8; width as usize * height as usize * 4],
            format: PixelFormat::Bgra,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// 像素格式（M1 仅 BGRA；后续扩展 RGB24/ARGB）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra,
}

impl fmt::Display for PixelFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PixelFormat::Bgra => write!(f, "BGRA"),
        }
    }
}

/// 显示器（对齐 FR-03.3 多显示器切换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub index: u8,
    pub width: u32,
    pub height: u32,
    /// 显示器名哈希（协议 DisplayEntry.name_hash 用）。
    pub name_hash: u16,
}

/// 显示器事件（FR-03.4，触发编码器重建）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEvent {
    /// 显示器插入。
    Added(Monitor),
    /// 显示器拔出（index 失效）。
    Removed(u8),
    /// 分辨率/刷新率变更。
    ResChanged(Monitor),
}

/// 屏幕采集抽象（平台实现：`win.rs` Graphics Capture / `linux.rs` PipeWire / `macos.rs` CGDisplayStream）。
///
/// 上层（`landesk-core/orchestrator`）只见此 trait，不感知平台细节。
pub trait ScreenCapture: Send {
    /// 枚举当前显示器列表。
    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError>;

    /// 抓取指定显示器一帧。
    fn grab(&self, monitor_index: u8) -> Result<RawFrame, CaptureError>;

    /// 显示器事件（插拔/分辨率变更），供触发编码器重建。
    /// 平台实现为阻塞或 channel 订阅；M1 先以 `None`（无事件）占位。
    fn poll_event(&mut self) -> Option<DisplayEvent>;
}

/// 供测试/回环的内存采集实现（无平台依赖，M1 冒烟用）。
#[derive(Debug, Default)]
pub struct MemCapture {
    monitors: Vec<Monitor>,
    next_event: std::collections::VecDeque<DisplayEvent>,
}

impl MemCapture {
    pub fn with_monitor(mut self, m: Monitor) -> Self {
        self.monitors.push(m);
        self
    }

    pub fn push_event(mut self, e: DisplayEvent) -> Self {
        self.next_event.push_back(e);
        self
    }
}

impl ScreenCapture for MemCapture {
    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
        if self.monitors.is_empty() {
            return Err(CaptureError::NoDisplay);
        }
        Ok(self.monitors.clone())
    }

    fn grab(&self, monitor_index: u8) -> Result<RawFrame, CaptureError> {
        let m = self
            .monitors
            .iter()
            .find(|m| m.index == monitor_index)
            .copied()
            .ok_or(CaptureError::DisplayNotFound(monitor_index))?;
        Ok(RawFrame::new_bgra(m.width, m.height))
    }

    fn poll_event(&mut self) -> Option<DisplayEvent> {
        self.next_event.pop_front()
    }
}
