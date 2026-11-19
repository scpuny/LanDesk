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

/// 合成画面源（C / 数据面打通用，无 OS 依赖）：逐帧产出**真实变化**的画面。
///
/// M1 的 [`MemCapture`] 只返回空帧（`new_bgra`）；本实现让"采集"环节产真像素——
/// 一个随帧号水平移动的红色块 + 把帧号烧进左上角 4 字节，使连续两帧可区分。
/// 用于最薄数据面链路（采集→编码→QUIC→还原）的离线验证，不接真实截屏
/// （PipeWire/WGC/CGDisplayStream 平台实装留作后续，见 FR-03.1）。
///
/// 帧号推进用 [`std::cell::Cell`]（内禀可变），因 [`ScreenCapture::grab`] 取 `&self`。
#[derive(Debug, Default)]
pub struct SynthCapture {
    monitors: Vec<Monitor>,
    next_event: std::collections::VecDeque<DisplayEvent>,
    frame_seq: std::cell::Cell<u32>,
}

impl SynthCapture {
    pub fn with_monitor(mut self, m: Monitor) -> Self {
        self.monitors.push(m);
        self
    }

    pub fn push_event(mut self, e: DisplayEvent) -> Self {
        self.next_event.push_back(e);
        self
    }

    /// 已产出的帧序号（下一次 `grab` 将烧录该值）。
    pub fn frame_seq(&self) -> u32 {
        self.frame_seq.get()
    }
}

/// 用帧号 `seq` 渲染一帧合成 BGRA 画面：红色块水平移动 + 左上角烧录 seq。
fn render_synth_frame(width: u32, height: u32, seq: u32) -> RawFrame {
    let block_w = width.div_ceil(4);
    let block_h = height.div_ceil(2);
    // 每帧色块右移 8px，到右缘回绕；width 为 0 时取 1 防除零。
    let off = (seq * 8) % width.max(1);
    let mut data = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let i = ((y * width + x) as usize) * 4;
            // 左上角前 4 字节 = seq 的 little-endian（原样烧录，供测试读回；
            // 不额外写 alpha，避免污染第 4 字节）。
            if x == 0 && y == 0 {
                let b = seq.to_le_bytes();
                data[i] = b[0];
                data[i + 1] = b[1];
                data[i + 2] = b[2];
                data[i + 3] = b[3];
                continue;
            }
            let in_block = x >= off && x < off.saturating_add(block_w) && y < block_h;
            if in_block {
                // 红色块（B,G,R,A）
                data[i] = 0x00;
                data[i + 1] = 0x00;
                data[i + 2] = 0xFF;
                data[i + 3] = 0xFF;
            } else {
                // 灰背景，按行微变化（静止处也非全 0）
                let g = ((y % 4) * 0x20) as u8;
                data[i] = g;
                data[i + 1] = g;
                data[i + 2] = g;
                data[i + 3] = 0xFF;
            }
        }
    }
    RawFrame {
        width,
        height,
        data,
        format: PixelFormat::Bgra,
    }
}

impl ScreenCapture for SynthCapture {
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
        // 本帧用当前 seq 绘制，绘制后推进一位（Cell 内禀可变，`&self` 可写）。
        let seq = self.frame_seq.get();
        self.frame_seq.set(seq.wrapping_add(1));
        Ok(render_synth_frame(m.width, m.height, seq))
    }

    fn poll_event(&mut self) -> Option<DisplayEvent> {
        self.next_event.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_monitor() -> Monitor {
        Monitor {
            index: 0,
            width: 64,
            height: 32,
            name_hash: 0,
        }
    }

    #[test]
    fn synth_frame_is_nonempty_and_varies() {
        let cap = SynthCapture::default().with_monitor(one_monitor());
        let f0 = cap.grab(0).expect("grab f0");
        let f1 = cap.grab(0).expect("grab f1");
        assert_eq!(f0.width, 64);
        assert_eq!(f0.height, 32);
        assert!(!f0.is_empty(), "合成帧须非空（区别于 MemCapture 空帧）");
        assert_ne!(f0.data, f1.data, "连续两帧像素必须不同（画面在动）");
    }

    #[test]
    fn synth_burns_frame_seq() {
        let cap = SynthCapture::default().with_monitor(one_monitor());
        let f0 = cap.grab(0).expect("grab f0");
        // 左上角前 4 字节 = seq 的 little-endian：首帧 0。
        assert_eq!(
            u32::from_le_bytes([f0.data[0], f0.data[1], f0.data[2], f0.data[3]]),
            0,
            "首帧烧录 seq=0"
        );
        let f1 = cap.grab(0).expect("grab f1");
        assert_eq!(
            u32::from_le_bytes([f1.data[0], f1.data[1], f1.data[2], f1.data[3]]),
            1,
            "次帧烧录 seq=1"
        );
    }

    #[test]
    fn synth_missing_display_errors() {
        let cap = SynthCapture::default().with_monitor(one_monitor());
        assert!(matches!(cap.grab(9), Err(CaptureError::DisplayNotFound(9))));
        let empty = SynthCapture::default();
        assert!(matches!(empty.monitors(), Err(CaptureError::NoDisplay)));
    }
}
