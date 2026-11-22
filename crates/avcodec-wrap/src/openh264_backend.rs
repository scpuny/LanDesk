//! openh264 真 H.264 编码后端（feature `openh264`，FR-04）。
//!
//! 通过 C ABI 桥接层（`third_party/openh264/bridge.cpp`）调用 Cisco openh264 2.4.1
//! SVC 编码器：BGRA → I420（BT.601）→ ISVCEncoder → Annex-B H.264 NALU。
//! 编码产物是 WebCodecs `VideoDecoder` 可直接解码的真 H.264（SPS/PPS/IDR/P）。
//!
//! 构建：build.rs 在 feature 开启时把裁剪的 openh264 源码编译成静态库（纯 C++，
//! 无 nasm）；feature 关闭时本模块不编译、默认仍用 [`PlaceholderEncoder`]。

use std::ffi::c_void;
use std::os::raw::{c_int, c_ulonglong};

use crate::{EncoderError, VideoEncoder};

unsafe extern "C" {
    fn ld_oh264_create(w: c_int, h: c_int, fps: c_int, bitrate_bps: c_int) -> *mut c_void;
    fn ld_oh264_encode(
        enc: *mut c_void,
        i420: *const u8,
        w: c_int,
        h: c_int,
        ts: c_ulonglong,
        out: *mut u8,
        cap: c_int,
        frame_type: *mut c_int,
    ) -> c_int;
    fn ld_oh264_destroy(enc: *mut c_void);
}

const OPENH264: u8 = 9;
/// EVideoFrameType.videoFrameTypeIDR（openh264 codec_app_def.h）。
const VIDEO_FRAME_TYPE_IDR: c_int = 1;

/// openh264 真编码器。
pub struct Openh264Encoder {
    handle: *mut c_void,
    w: u32,
    h: u32,
    fps: u32,
    /// I420 中间缓冲（Y + U + V）。
    i420: Vec<u8>,
    /// 编码输出缓冲（Annex-B，复用避免每帧分配）。
    out: Vec<u8>,
    /// 待取出的 NALU 队列（一帧可能含 SPS/PPS/IDR 多条，按起始码切分成独立 NALU）。
    queue: std::collections::VecDeque<Vec<u8>>,
    /// 下一帧序号（时间戳 ms）。
    ts: u64,
    /// 最近一帧类型（供 kind 标签）。
    last_idr: bool,
}

// SAFETY: ISVCEncoder 句柄仅在单个 VideoPump 任务内串行使用——send_frame /
// recv_nalu / open / close 都由同一 task 顺序调用，不存在跨线程并发访问；
// 其余字段均为线程安全的 Vec/VecDeque/标量。故可安全跨线程转移所有权（非共享）。
unsafe impl Send for Openh264Encoder {}

impl Openh264Encoder {
    pub fn new() -> Self {
        Self {
            handle: std::ptr::null_mut(),
            w: 0,
            h: 0,
            fps: 0,
            i420: Vec::new(),
            out: Vec::new(),
            queue: std::collections::VecDeque::new(),
            ts: 0,
            last_idr: false,
        }
    }

    /// 按画质预设映射目标码率（bps）。kind 初值用于兜底。
    fn bitrate_for(w: u32, h: u32) -> c_int {
        // 约 2 Mbps @ 640×360，按像素数线性缩放，上限 8 Mbps。
        let px = (w as u64) * (h as u64);
        let bps = px * 3; // ~3 bit/pixel
        bps.clamp(200_000, 8_000_000) as c_int
    }
}

impl Default for Openh264Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Openh264Encoder {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { ld_oh264_destroy(self.handle) };
            self.handle = std::ptr::null_mut();
        }
    }
}

impl VideoEncoder for Openh264Encoder {
    fn kind(&self) -> u8 {
        OPENH264
    }

    fn open(&mut self, width: u32, height: u32, fps: u32) -> Result<(), EncoderError> {
        if width == 0 || height == 0 || width & 1 != 0 || height & 1 != 0 {
            return Err(EncoderError::OpenFailed(
                "I420 requires positive even dimensions".into(),
            ));
        }
        if fps == 0 {
            return Err(EncoderError::OpenFailed("fps must be > 0".into()));
        }
        let handle = unsafe {
            ld_oh264_create(
                width as c_int,
                height as c_int,
                fps as c_int,
                Self::bitrate_for(width, height),
            )
        };
        if handle.is_null() {
            return Err(EncoderError::OpenFailed(
                "WelsCreateSVCEncoder/Initialize failed".into(),
            ));
        }
        self.handle = handle;
        self.w = width;
        self.h = height;
        self.fps = fps;
        self.i420 = vec![0u8; (width as usize) * (height as usize) * 3 / 2];
        // 压缩后单帧通常 < w*h/2（高码率上限 8Mbps），留足冗余。
        self.out = vec![0u8; (width as usize) * (height as usize) * 2 + 4096];
        self.queue.clear();
        self.ts = 0;
        self.last_idr = false;
        Ok(())
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), EncoderError> {
        if self.handle.is_null() {
            return Err(EncoderError::OpenFailed("encoder not opened".into()));
        }
        let w = self.w as usize;
        let h = self.h as usize;
        let need = w * h * 4;
        if frame.len() < need {
            return Err(EncoderError::EncodeFailed(format!(
                "frame too small: {} < {} (BGRA {}x{})",
                frame.len(),
                need,
                w,
                h
            )));
        }
        // BGRA → I420（BT.601 有限范围）。
        bgra_to_i420(&frame[..need], w, h, &mut self.i420);

        let mut ft: c_int = 0;
        let n = unsafe {
            ld_oh264_encode(
                self.handle,
                self.i420.as_ptr(),
                w as c_int,
                h as c_int,
                self.ts as c_ulonglong,
                self.out.as_mut_ptr(),
                self.out.len() as c_int,
                &mut ft,
            )
        };
        self.ts += 1000 / self.fps.max(1) as u64;
        if n <= 0 {
            return Err(EncoderError::EncodeFailed(format!(
                "openh264 EncodeFrame returned {n}"
            )));
        }
        self.last_idr = ft == VIDEO_FRAME_TYPE_IDR;
        // 一帧 = 一个 Annex-B 访问单元（IDR 帧含 SPS+PPS+IDR 三条 NALU，P 帧一条），
        // 整体入队：WebCodecs 以单个 EncodedVideoChunk 解一个访问单元，解码器按起始码
        // 自行切分 NALU。分片重组后仍是一整块 Annex-B，kind 标签随帧走（is_keyframe）。
        debug_assert!(
            !split_annex_b(&self.out[..n as usize]).is_empty(),
            "openh264 输出应含 Annex-B 起始码"
        );
        self.queue.push_back(self.out[..n as usize].to_vec());
        Ok(())
    }

    fn recv_nalu(&mut self) -> Option<Vec<u8>> {
        self.queue.pop_front()
    }

    fn is_keyframe(&self) -> bool {
        self.last_idr
    }

    fn close(&mut self) {
        if !self.handle.is_null() {
            unsafe { ld_oh264_destroy(self.handle) };
            self.handle = std::ptr::null_mut();
        }
        self.queue.clear();
        self.w = 0;
        self.h = 0;
    }
}

impl Openh264Encoder {
    /// 最近一帧是否为关键帧（IDR+SPS/PPS）。
    pub fn is_idr(&self) -> bool {
        self.last_idr
    }
}

/// BGRA（4B/像素，B,G,R,A）→ I420（Y 平面 + U/V 各 1/4），BT.601 有限范围。
fn bgra_to_i420(bgra: &[u8], w: usize, h: usize, out: &mut [u8]) {
    let y_plane = out.as_mut_ptr();
    let u_plane = unsafe { out.as_mut_ptr().add(w * h) };
    let v_plane = unsafe { out.as_mut_ptr().add(w * h + (w * h) / 4) };
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let b = bgra[i] as i32;
            let g = bgra[i + 1] as i32;
            let r = bgra[i + 2] as i32;
            // BT.601 limited-range RGB → YUV
            let yy = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            unsafe { *y_plane.add(y * w + x) = yy.clamp(0, 255) as u8 };
            if y % 2 == 0 && x % 2 == 0 {
                let cb = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                let cr = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
                let uv_idx = (y / 2) * (w / 2) + (x / 2);
                unsafe {
                    *u_plane.add(uv_idx) = cb.clamp(0, 255) as u8;
                    *v_plane.add(uv_idx) = cr.clamp(0, 255) as u8;
                }
            }
        }
    }
}

/// 按 H.264 Annex-B 起始码切分 NALU（保留起始码），供逐 NALU 分片发送。
fn split_annex_b(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            starts.push(i);
            i += 4;
        } else if i + 2 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 1
        {
            starts.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    for (k, &s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(data.len());
        if e > s {
            out.push(data[s..e].to_vec());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_annex_b_sps_pps_idr() {
        let data = [
            0, 0, 0, 1, 0x67, 0x42, // SPS
            0, 0, 0, 1, 0x68, 0xce, // PPS
            0, 0, 0, 1, 0x65, 0x88, // IDR
        ];
        let nalus = split_annex_b(&data);
        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0], vec![0, 0, 0, 1, 0x67, 0x42]);
        assert_eq!(nalus[2].last().copied(), Some(0x88));
    }

    /// 真编码闭环：BGRA → openh264 → Annex-B 访问单元（SPS/PPS/IDR）。
    /// 验证产物能被 WebCodecs `VideoDecoder`（avc1.42c01f，annexb）解码。
    #[test]
    fn encodes_real_annex_b_access_unit() {
        let w = 64usize;
        let h = 48usize;
        let mut enc = Openh264Encoder::new();
        enc.open(w as u32, h as u32, 15).expect("open");

        let mut bgra = vec![0u8; w * h * 4];
        for p in bgra.as_chunks_mut::<4>().0 {
            p[0] = 0;
            p[1] = 0;
            p[2] = 255; // 红
            p[3] = 255;
        }
        enc.send_frame(&bgra).expect("encode frame");
        let au = enc.recv_nalu().expect("access unit");
        assert!(au.starts_with(&[0, 0, 0, 1]), "应含 Annex-B 起始码");

        let hdr = |n: &[u8]| -> u8 {
            if n.starts_with(&[0, 0, 0, 1]) {
                n[4]
            } else if n.starts_with(&[0, 0, 1]) {
                n[3]
            } else {
                0
            }
        };
        let types: Vec<u8> = split_annex_b(&au).iter().map(|n| hdr(n) & 0x1f).collect();
        assert!(enc.is_keyframe(), "首帧应为 IDR");
        assert!(types.contains(&7), "应含 SPS(7): {types:02x?}");
        assert!(types.contains(&8), "应含 PPS(8): {types:02x?}");
        assert!(types.contains(&5), "应含 IDR(5): {types:02x?}");
    }

    #[test]
    fn bgra_to_i420_basic() {
        let w = 4;
        let h = 4;
        let mut bgra = vec![0u8; w * h * 4];
        for p in bgra.as_chunks_mut::<4>().0 {
            p[0] = 0;
            p[1] = 0;
            p[2] = 255; // 红色
            p[3] = 255;
        }
        let mut i420 = vec![0u8; w * h * 3 / 2];
        bgra_to_i420(&bgra, w, h, &mut i420);
        // 红色 Y 应显著 > U/V，U < 128 < V（红色 Cb<128, Cr>128）。
        assert!(i420[0] > 60, "红色亮度 {}", i420[0]);
        assert!(i420[w * h] < 128, "红色 Cb {}", i420[w * h]);
        assert!(i420[w * h + (w * h) / 4] > 128, "红色 Cr {}", i420[w * h + (w * h) / 4]);
    }
}
