//! Windows 真实采屏：GDI BitBlt + GetDIBits（降级自 WGC，windows-sys 0.61.2 vendor 已含）。
//!
//! 用全屏虚拟屏（SM_XVIRTUALSCREEN 等）抓主显示器，转 32bpp BGRA。GDI 无需窗口
//! 前台权限，比 WGC 更稳（WGC 需 Graphics Capture 权限 + COM 复杂）；多显示器
//! 场景先覆盖主屏，后续可按 MONITORENUMPROC 扩展。

use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HBITMAP, SRCCOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
};

use crate::{CaptureError, DisplayEvent, Monitor, PixelFormat, RawFrame, ScreenCapture};

/// Windows GDI 采屏器。
pub struct GdiCapture {
    monitors: Vec<Monitor>,
}

impl GdiCapture {
    pub fn new() -> Result<Self, CaptureError> {
        let monitors = platform_monitors()?;
        Ok(Self { monitors })
    }
}

impl ScreenCapture for GdiCapture {
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
        let (w, h) = (m.width as i32, m.height as i32);
        unsafe {
            // 屏幕 DC（只读）→ 兼容内存 DC + 兼容位图 → BitBlt 拷贝 → GetDIBits 取 BGRA。
            let screen_dc: HDC = GetDC(core::ptr::null_mut());
            if screen_dc.is_null() {
                return Err(CaptureError::BackendUnavailable("GetDC failed".into()));
            }
            let mem_dc = CreateCompatibleDC(screen_dc);
            let bitmap: HBITMAP = CreateCompatibleBitmap(screen_dc, w, h);
            let old = SelectObject(mem_dc, bitmap);
            let ok = BitBlt(mem_dc, 0, 0, w, h, screen_dc, 0, 0, SRCCOPY);
            if ok == 0 {
                SelectObject(mem_dc, old);
                DeleteObject(bitmap);
                DeleteDC(mem_dc);
                ReleaseDC(core::ptr::null_mut(), screen_dc);
                return Err(CaptureError::BackendUnavailable("BitBlt failed".into()));
            }

            let mut frame = RawFrame::new_bgra(m.width, m.height);
            let mut bmi: BITMAPINFO = core::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 负高度 = 自上而下（top-down）行序
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            };
            let got = GetDIBits(
                mem_dc,
                bitmap,
                0,
                h as u32,
                frame.data.as_mut_ptr() as *mut core::ffi::c_void,
                &mut bmi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem_dc, old);
            DeleteObject(bitmap);
            DeleteDC(mem_dc);
            ReleaseDC(core::ptr::null_mut(), screen_dc);

            if got == 0 {
                return Err(CaptureError::BackendUnavailable("GetDIBits failed".into()));
            }
            // GDI 32bpp 输出为 BGRA（B,G,R,A 顺序），与 RawFrame::format 一致。
            frame.format = PixelFormat::Bgra;
            Ok(frame)
        }
    }

    fn poll_event(&mut self) -> Option<DisplayEvent> {
        None
    }
}

#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: *mut core::ffi::c_void) -> HDC;
    fn ReleaseDC(hwnd: *mut core::ffi::c_void, hdc: HDC) -> i32;
}

/// 平台显示器枚举（主屏 + 虚拟屏尺寸）。
pub fn platform_monitors() -> Result<Vec<Monitor>, CaptureError> {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        if w <= 0 || h <= 0 {
            return Err(CaptureError::NoDisplay);
        }
        let _virtual = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        Ok(vec![Monitor {
            index: 0,
            width: w as u32,
            height: h as u32,
            name_hash: 0,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitmapinfo_header_size() {
        assert_eq!(core::mem::size_of::<BITMAPINFOHEADER>(), 40);
    }
}
