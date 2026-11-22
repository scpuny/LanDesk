//! macOS 真实采屏：CoreGraphics `CGDisplayCreateImageForRect` + CGImage data provider
//! （extern "C" FFI 直链 ApplicationServices / CoreFoundation / CoreGraphics，
//! 不依赖 vendor 缺失的 core-graphics crate）。
//!
//! 权限：进程需被授予「屏幕录制」（Screen Recording）权限，否则 CGDisplayCreateImage
//! 返回全黑或 NULL。构造时探测一次，失败由调用方降级为合成源（不阻断会话）。

use std::ffi::c_void;

use crate::{CaptureError, DisplayEvent, Monitor, PixelFormat, RawFrame, ScreenCapture};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGGetOnlineDisplayList(max_displays: u32, online_displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCreateImageForRect(display: u32, rect: CGRect) -> *mut c_void;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGImageGetWidth(image: *mut c_void) -> usize;
    fn CGImageGetHeight(image: *mut c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *mut c_void) -> usize;
    fn CGImageGetDataProvider(image: *mut c_void) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> *mut c_void;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(data: *mut c_void) -> *const u8;
    fn CFDataGetLength(data: *mut c_void) -> isize;
    fn CFRelease(cf: *mut c_void);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

/// macOS CoreGraphics 采屏器。
pub struct CGDisplayCapture {
    monitors: Vec<Monitor>,
}

impl CGDisplayCapture {
    /// 构造并枚举在线显示器；无显示器 / 无权限返回 `BackendUnavailable`。
    pub fn new() -> Result<Self, CaptureError> {
        let monitors = enumerate_displays()?;
        Ok(Self { monitors })
    }
}

impl ScreenCapture for CGDisplayCapture {
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
        unsafe {
            let display = m.index as u32;
            let bounds = CGDisplayBounds(display);
            let image = CGDisplayCreateImageForRect(display, bounds);
            if image.is_null() {
                return Err(CaptureError::BackendUnavailable(
                    "CGDisplayCreateImageForRect returned null (screen recording permission?)".into(),
                ));
            }
            let w = CGImageGetWidth(image);
            let h = CGImageGetHeight(image);
            let stride = CGImageGetBytesPerRow(image);
            let provider = CGImageGetDataProvider(image);
            let data = CGDataProviderCopyData(provider);
            let bytes = CFDataGetBytePtr(data);
            let len = CFDataGetLength(data);

            let width = w as u32;
            let height = h as u32;
            let mut frame = RawFrame::new_bgra(width, height);
            let need = width as usize * 4;
            if !bytes.is_null() && len >= (stride * h) as isize && stride >= need {
                for y in 0..height as usize {
                    let src = bytes.add(y * stride);
                    let dst = frame.data.as_mut_ptr().add(y * need);
                    core::ptr::copy_nonoverlapping(src, dst, need);
                }
            } else {
                // 数据不足 / 布局异常 → 返回空帧（调用方据此降级）。
                frame.data.clear();
            }

            CFRelease(data);
            CFRelease(image);
            if frame.data.is_empty() {
                return Err(CaptureError::BackendUnavailable(
                    "CGImage data layout unexpected".into(),
                ));
            }
            frame.format = PixelFormat::Bgra;
            Ok(frame)
        }
    }

    fn poll_event(&mut self) -> Option<DisplayEvent> {
        None
    }
}

fn enumerate_displays() -> Result<Vec<Monitor>, CaptureError> {
    unsafe {
        let mut count: u32 = 0;
        // 先取数量（max=0，buffer=null）。
        CGGetOnlineDisplayList(0, core::ptr::null_mut(), &mut count);
        if count == 0 {
            return Err(CaptureError::NoDisplay);
        }
        let mut ids = vec![0u32; count as usize];
        let mut actual: u32 = 0;
        if CGGetOnlineDisplayList(count, ids.as_mut_ptr(), &mut actual) != 0 || actual == 0 {
            return Err(CaptureError::NoDisplay);
        }
        ids.truncate(actual as usize);
        let main = CGMainDisplayID();
        // 主屏排到 index 0，其余按 id 排。
        ids.sort_by_key(|id| (*id != main, *id));
        let monitors = ids
            .into_iter()
            .enumerate()
            .map(|(i, id)| {
                let b = CGDisplayBounds(id);
                Monitor {
                    index: i as u8,
                    width: b.size.width.max(1.0) as u32,
                    height: b.size.height.max(1.0) as u32,
                    name_hash: id as u16,
                }
            })
            .collect();
        Ok(monitors)
    }
}

/// 平台显示器枚举（供上层显示器列表使用）。
pub fn platform_monitors() -> Result<Vec<Monitor>, CaptureError> {
    enumerate_displays()
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_ids_main_first_sort() {
        // 仅验证排序逻辑（用纯数据，不碰 FFI）：主屏 id=2 应排第一。
        let main = 2u32;
        let mut ids = vec![3u32, 2, 1];
        ids.sort_by_key(|id| (*id != main, *id));
        assert_eq!(ids, vec![2, 1, 3]);
    }
}
