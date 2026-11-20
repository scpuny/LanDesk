//! 显示器管理与切换（FR-03.3 / FR-03.4，对齐 `docs/architecture.md` §2 `capture/monitors.rs`）。
//!
//! M1 阶段提供无平台依赖的**逻辑层**：显示器列表状态 + 切换选择 + 事件归约，
//! 平台实现（枚举真实显示器）通过 [`MonitorSource`] trait 注入。

use std::collections::VecDeque;

use crate::{CaptureError, DisplayEvent, Monitor};

/// 显示器事件源（平台实现注入；M1 以内存队列模拟）。
pub trait MonitorSource: Send {
    /// 当前显示器列表。
    fn list(&self) -> Result<Vec<Monitor>, CaptureError>;

    /// 轮询一个显示器事件（插拔 / 分辨率变更）。M1 内存实现。
    fn poll(&mut self) -> Option<DisplayEvent>;
}

/// 基于内存队列的显示器事件源（测试 / M1 冒烟）。
#[derive(Debug, Default)]
pub struct MemoryMonitors {
    monitors: Vec<Monitor>,
    events: VecDeque<DisplayEvent>,
}

impl MemoryMonitors {
    pub fn with(monitors: Vec<Monitor>) -> Self {
        Self { monitors, events: VecDeque::new() }
    }

    pub fn push_event(&mut self, e: DisplayEvent) {
        self.events.push_back(e);
    }

    /// 应用事件到本地列表（模拟平台真实状态变化）。
    pub fn apply(&mut self, e: &DisplayEvent) {
        match e {
            DisplayEvent::Added(m) => self.monitors.push(*m),
            DisplayEvent::Removed(i) => {
                self.monitors.retain(|m| &m.index != i);
            }
            DisplayEvent::ResChanged(m) => {
                if let Some(slot) = self.monitors.iter_mut().find(|x| x.index == m.index) {
                    *slot = *m;
                }
            }
        }
    }
}

impl MonitorSource for MemoryMonitors {
    fn list(&self) -> Result<Vec<Monitor>, CaptureError> {
        if self.monitors.is_empty() {
            return Err(CaptureError::NoDisplay);
        }
        Ok(self.monitors.clone())
    }

    fn poll(&mut self) -> Option<DisplayEvent> {
        self.events.pop_front()
    }
}

/// 显示器选择器：维护"当前选中显示器"，切换时触发事件。
#[derive(Debug, Default)]
pub struct DisplaySelector {
    current: Option<u8>,
}

impl DisplaySelector {
    /// 初始化到列表中第一个。
    pub fn init(&mut self, monitors: &[Monitor]) {
        self.current = monitors.first().map(|m| m.index);
    }

    /// 切到指定 index；成功返回 `true`，找不到返回 `false`。
    pub fn switch(&mut self, target: u8, monitors: &[Monitor]) -> bool {
        if monitors.iter().any(|m| m.index == target) {
            self.current = Some(target);
            true
        } else {
            false
        }
    }

    pub fn current(&self) -> Option<u8> {
        self.current
    }

    /// 显示器被移除时，若当前选中失效则回退到剩余第一个。
    pub fn on_removed(&mut self, removed: u8, monitors: &[Monitor]) -> bool {
        let changed = self.current == Some(removed);
        if changed {
            self.current = monitors.first().map(|m| m.index);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(i: u8, w: u32, h: u32) -> Monitor {
        Monitor { index: i, width: w, height: h, name_hash: i as u16 * 7 }
    }

    #[test]
    fn switch_and_fallback() {
        let mons = vec![m(0, 1920, 1080), m(1, 2560, 1440)];
        let mut sel = DisplaySelector::default();
        sel.init(&mons);
        assert_eq!(sel.current(), Some(0));
        assert!(sel.switch(1, &mons));
        assert_eq!(sel.current(), Some(1));
        assert!(sel.on_removed(1, &mons));
        assert_eq!(sel.current(), Some(0), "移除当前选中应回退到剩余第一个");
    }

    #[test]
    fn invalid_switch_rejected() {
        let mons = vec![m(0, 1920, 1080)];
        let mut sel = DisplaySelector::default();
        sel.init(&mons);
        assert!(!sel.switch(9, &mons));
        assert_eq!(sel.current(), Some(0));
    }

    #[test]
    fn memory_monitor_events() {
        let mut src = MemoryMonitors::with(vec![m(0, 1920, 1080)]);
        let added = m(1, 2560, 1440);
        src.push_event(DisplayEvent::Added(added));
        let ev = src.poll();
        assert_eq!(ev, Some(DisplayEvent::Added(added)));
    }
}
