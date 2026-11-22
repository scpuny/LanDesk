//! Linux 真实采屏：X11 `XGetImage`（extern "C" FFI 直链 libX11）→ BGRA。
//!
//! 适用 X11 会话（多数桌面）；Wayland 会话需 PipeWire（后续）。无 DISPLAY /
//! 无 X11 库时构造失败，由调用方降级为合成源（不阻断会话）。
//! CI 编译需 `libx11-dev`（windows/linux 平台实现按 target_os 门控）。

use std::ffi::{c_char, c_int, c_ulong, c_void};

use crate::{CaptureError, DisplayEvent, Monitor, PixelFormat, RawFrame, ScreenCapture};

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(display: *mut c_void) -> c_int;
    fn XDefaultScreen(display: *mut c_void) -> c_int;
    fn XRootWindow(display: *mut c_void, screen: c_int) -> c_ulong;
    fn XDisplayWidth(display: *mut c_void, screen: c_int) -> c_int;
    fn XDisplayHeight(display: *mut c_void, screen: c_int) -> c_int;
    fn XGetImage(
        display: *mut c_void,
        drawable: c_ulong,
        x: c_int,
        y: c_int,
        width: u32,
        height: u32,
        plane_mask: c_ulong,
        format: c_int,
    ) -> *mut XImage;
    fn XDestroyImage(image: *mut XImage);
}

/// XImage 结构（Xlib.h，取到 bits_per_pixel 为止的完整前缀）。
#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
}

const Z_PIXMAP: c_int = 2;
const ALL_PLANES: c_ulong = 0xFFFF_FFFF;

/// X11 采屏器：持 Display 句柄（不可跨线程 Send，故用原生指针 + unsafe impl Send 单线程用）。
pub struct X11Capture {
    display: *mut c_void,
    monitors: Vec<Monitor>,
}

// X11 Display 在单线程内使用（VideoPump 独占 grab）；标记 Send 以便 move 进 tokio 任务。
unsafe impl Send for X11Capture {}

impl X11Capture {
    pub fn new() -> Result<Self, CaptureError> {
        let display = unsafe { XOpenDisplay(core::ptr::null()) };
        if display.is_null() {
            return Err(CaptureError::BackendUnavailable(
                "XOpenDisplay failed (no DISPLAY / Wayland session?)".into(),
            ));
        }
        unsafe {
            let screen = XDefaultScreen(display);
            let w = XDisplayWidth(display, screen);
            let h = XDisplayHeight(display, screen);
            if w <= 0 || h <= 0 {
                XCloseDisplay(display);
                return Err(CaptureError::NoDisplay);
            }
            Ok(Self {
                display,
                monitors: vec![Monitor {
                    index: 0,
                    width: w as u32,
                    height: h as u32,
                    name_hash: 0,
                }],
            })
        }
    }
}

impl Drop for X11Capture {
    fn drop(&mut self) {
        unsafe {
            XCloseDisplay(self.display);
        }
    }
}

impl ScreenCapture for X11Capture {
    fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
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
            let root = XRootWindow(self.display, XDefaultScreen(self.display));
            let image = XGetImage(
                self.display,
                root,
                0,
                0,
                m.width,
                m.height,
                ALL_PLANES,
                Z_PIXMAP,
            );
            if image.is_null() {
                return Err(CaptureError::BackendUnavailable("XGetImage failed".into()));
            }
            let bits = (*image).bits_per_pixel;
            let stride = (*image).bytes_per_line as usize;
            let mut frame = RawFrame::new_bgra(m.width, m.height);
            let need = m.width as usize * 4;
            if (bits == 24 || bits == 32) && stride >= need && !(*image).data.is_null() {
                let src = (*image).data as *const u8;
                for y in 0..m.height as usize {
                    let from = src.add(y * stride);
                    let to = frame.data.as_mut_ptr().add(y * need);
                    if bits == 32 {
                        core::ptr::copy_nonoverlapping(from, to, need);
                    } else {
                        // 24bpp：逐像素 3B RGB → 4B BGRA。
                        for x in 0..m.width as usize {
                            let p = from.add(x * 3);
                            let d = to.add(x * 4);
                            *d = *p.add(2);
                            *d.add(1) = *p.add(1);
                            *d.add(2) = *p;
                            *d.add(3) = 0xFF;
                        }
                    }
                }
            } else {
                frame.data.clear();
            }
            XDestroyImage(image);
            if frame.data.is_empty() {
                return Err(CaptureError::BackendUnavailable(
                    "XImage layout unsupported (need 24/32bpp ZPixmap)".into(),
                ));
            }
            // X11 小端 ZPixmap 32bpp 内存序为 B,G,R,A（与 BGRA 一致）。
            frame.format = PixelFormat::Bgra;
            Ok(frame)
        }
    }

    fn poll_event(&mut self) -> Option<DisplayEvent> {
        None
    }
}

/// 平台显示器枚举。
pub fn platform_monitors() -> Result<Vec<Monitor>, CaptureError> {
    let cap = X11Capture::new()?;
    Ok(cap.monitors.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ximage_layout_constants() {
        assert_eq!(Z_PIXMAP, 2);
        // 仅常量校验，不碰 X11 FFI（headless CI 可跑）。
        assert_eq!(std::mem::size_of::<c_int>(), 4);
    }
}
