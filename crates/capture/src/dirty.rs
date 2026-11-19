//! 脏矩形检测（FR-03.2，风险表#3 可开关降级全帧）。
//!
//! 策略：逐块（默认 16×16 宏块）对比当前帧与上一帧像素，
//! 仅当块内存在差异时标记该块为脏；静止桌面下几乎不产生编码数据（NFR-02）。
//! 提供 `enabled` 开关，关闭时退化为全帧模式（整帧视为脏）。

use crate::RawFrame;

/// 脏矩形（像素坐标系，含边）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirtyRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl DirtyRect {
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
}

/// 宏块尺寸（16×16，H.264 最小编码块对齐）。
pub const BLOCK: u32 = 16;

/// 脏矩形检测器：持有上一帧，产出当前帧的脏块列表。
#[derive(Debug, Default)]
pub struct DirtyRectDetector {
    enabled: bool,
    prev: Option<PrevFrame>,
}

#[derive(Debug)]
struct PrevFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl DirtyRectDetector {
    /// `enabled = false` 时退化为全帧模式（风险表#3 预案）。
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            prev: None,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 对比当前帧与上一帧，返回脏块对应的矩形列表。
    /// 首帧（无上一帧）或尺寸变化时返回全帧脏块。
    pub fn detect(&mut self, frame: &RawFrame) -> Vec<DirtyRect> {
        if !self.enabled {
            // 全帧模式
            return vec![DirtyRect::new(0, 0, frame.width, frame.height)];
        }

        // 尺寸变化 / 首帧 → 全帧
        let Some(prev) = self.prev.as_ref() else {
            self.update_prev(frame);
            return vec![DirtyRect::new(0, 0, frame.width, frame.height)];
        };
        if prev.width != frame.width || prev.height != frame.height {
            self.update_prev(frame);
            return vec![DirtyRect::new(0, 0, frame.width, frame.height)];
        }

        let mut dirty = Vec::new();
        let px = prev.data.len();
        let cur = &frame.data;
        debug_assert_eq!(px, cur.len(), "同尺寸帧字节数必须一致");

        let w = frame.width as usize;
        let h = frame.height as usize;
        let stride = w * 4;

        for by in (0..h).step_by(BLOCK as usize) {
            for bx in (0..w).step_by(BLOCK as usize) {
                let x0 = bx;
                let y0 = by;
                let x1 = (bx + BLOCK as usize).min(w);
                let y1 = (by + BLOCK as usize).min(h);
                if block_dirty(prev, cur, x0, y0, x1, y1, stride) {
                    dirty.push(DirtyRect::new(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32));
                }
            }
        }
        self.update_prev(frame);
        dirty
    }

    /// 重置（分辨率变更 / 显示器切换 / 编码器重建后调用）。
    pub fn reset(&mut self) {
        self.prev = None;
    }

    fn update_prev(&mut self, frame: &RawFrame) {
        self.prev = Some(PrevFrame {
            width: frame.width,
            height: frame.height,
            data: frame.data.clone(),
        });
    }
}

/// 单块像素对比（BGRA，stride = 宽*4）。
fn block_dirty(prev: &PrevFrame, cur: &[u8], x0: usize, y0: usize, x1: usize, y1: usize, stride: usize) -> bool {
    for y in y0..y1 {
        let base = y * stride;
        for x in x0..x1 {
            let i = base + x * 4;
            if prev.data[i..i + 4] != cur[i..i + 4] {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_4x4() -> RawFrame {
        // 填充全 0
        RawFrame::new_bgra(4, 4)
    }

    fn frame_4x4_v2(prev: RawFrame) -> RawFrame {
        let mut f = prev;
        // 改动第 (0,0) 像素
        f.data[0] = 0xFF;
        f
    }

    #[test]
    fn first_frame_is_full_dirty() {
        let mut det = DirtyRectDetector::new(true);
        let f = frame_4x4();
        let d = det.detect(&f);
        assert_eq!(d, vec![DirtyRect::new(0, 0, 4, 4)]);
    }

    #[test]
    fn identical_frames_no_dirty() {
        let mut det = DirtyRectDetector::new(true);
        let f = frame_4x4();
        let _ = det.detect(&f); // 首帧全脏
        let d = det.detect(&f);
        assert!(d.is_empty(), "静止帧应无脏块");
    }

    #[test]
    fn changed_pixel_flags_block() {
        let mut det = DirtyRectDetector::new(true);
        let f1 = frame_4x4();
        let _ = det.detect(&f1);
        let f2 = frame_4x4_v2(f1.clone());
        let d = det.detect(&f2);
        // 4x4 帧小于一个 16x16 块，整帧归为一个块
        assert_eq!(d.len(), 1);
        assert_eq!(d[0], DirtyRect::new(0, 0, 4, 4));
    }

    #[test]
    fn disabled_mode_always_full() {
        let mut det = DirtyRectDetector::new(false);
        let f = frame_4x4();
        for _ in 0..3 {
            let d = det.detect(&f);
            assert_eq!(d, vec![DirtyRect::new(0, 0, 4, 4)]);
        }
    }

    #[test]
    fn resolution_change_triggers_full() {
        let mut det = DirtyRectDetector::new(true);
        let f1 = RawFrame::new_bgra(4, 4);
        let _ = det.detect(&f1);
        let f2 = RawFrame::new_bgra(8, 8);
        let d = det.detect(&f2);
        assert_eq!(d, vec![DirtyRect::new(0, 0, 8, 8)]);
    }
}
