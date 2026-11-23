//! 采屏内容有效性校验（问题 2/5 根因：采屏 API 返回 Ok ≠ 画面有效）。
//!
//! 平台采屏在「无屏幕录制权限 / 锁屏 / 无交互桌面」下常见两类假成功：
//! - macOS `CGDisplayCreateImageForRect` 返回全黑位图（而非 Err）；
//! - Windows GDI `BitBlt` 在安全桌面下返回 TRUE 但像素全黑。
//!
//! 这类黑帧被当正常画面编码发出，控制端只见纯黑或（构造失败降级后的）合成测试图。
//! 本模块提供无 OS 依赖的纯函数，让 `VideoPump` 逐帧判定「近纯色/全黑」，连续 N 帧
//! 即触发降级为合成源并上报 degraded 状态，避免用户误以为真桌面黑屏。

use crate::{PixelFormat, RawFrame};

/// 采样步长（像素）。全屏 1920×1080 约取 5 万点，单次判定 < 1ms，不影响 30fps 发流。
const SAMPLE_STEP_PX: usize = 37;
/// 颜色量化位移：把 8bit 通道压到 5bit（÷8），容忍采集噪声与压缩前的微小色差。
const COLOR_QUANT_SHIFT: u32 = 3;
/// 有效画面至少需要的最小颜色桶数。纯黑/纯白/纯色屏只有 1 个桶。
/// 取 2 而非 3：避免把「白底黑字」类低色数真实桌面误判为黑屏（宁可漏判稀有双色异常帧，
/// 不可误伤有权限的正常桌面导致错误降级）。
const MIN_DISTINCT_BUCKETS: usize = 2;

/// 判定一帧是否为「无效画面」（全黑 / 纯色 / 数据异常）。
///
/// 判定法：对 BGRA 数据按固定步长采样，把 RGB 各通道量化到 5bit 后装入 15bit 桶，
/// 统计不同颜色桶数量。真实桌面必有文字/图标/渐变 → 远超阈值；纯色屏（含全黑）仅 1 桶。
/// 非 BGRA / 空帧一律视为无效，交由调用方降级。
pub fn is_blank_frame(frame: &RawFrame) -> bool {
    if frame.is_empty() || frame.width == 0 || frame.height == 0 {
        return true;
    }
    if frame.format != PixelFormat::Bgra {
        // 目前仅支持 BGRA；未知格式不做误判，交给调用方按 Err 处理更稳妥——
        // 但为防未知格式黑帧漏过，这里保守判为无效（D1 阶段仅有 BGRA 实现）。
        return true;
    }
    let bpp = 4usize;
    if frame.data.len() < frame.width as usize * frame.height as usize * bpp {
        return true;
    }

    let total_px = frame.width as usize * frame.height as usize;
    let step = SAMPLE_STEP_PX.max(1);
    let mut buckets = std::collections::HashSet::new();
    let mut sampled = 0usize;
    let mut idx = 0usize;
    while idx < total_px {
        let base = idx * bpp;
        let b = frame.data[base] as u32;
        let g = frame.data[base + 1] as u32;
        let r = frame.data[base + 2] as u32;
        let key = ((r >> COLOR_QUANT_SHIFT) << 10)
            | ((g >> COLOR_QUANT_SHIFT) << 5)
            | (b >> COLOR_QUANT_SHIFT);
        buckets.insert(key);
        sampled += 1;
        // 样本已足够覆盖整屏色彩分布时提前退出（避免大屏逐点扫描）。
        if sampled >= 4096 && buckets.len() >= MIN_DISTINCT_BUCKETS {
            break;
        }
        idx += step;
    }
    buckets.len() < MIN_DISTINCT_BUCKETS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawFrame;

    fn bgra_frame(w: u32, h: u32, paint: impl Fn(&mut [u8])) -> RawFrame {
        let mut f = RawFrame::new_bgra(w, h);
        paint(&mut f.data);
        f
    }

    #[test]
    fn blank_all_black() {
        let f = RawFrame::new_bgra(64, 64); // 全 0 = 全黑
        assert!(is_blank_frame(&f));
    }

    #[test]
    fn blank_all_white() {
        let f = bgra_frame(64, 64, |d| {
            for px in d.as_chunks_mut::<4>().0 {
                px[0] = 255; px[1] = 255; px[2] = 255; px[3] = 255;
            }
        });
        assert!(is_blank_frame(&f));
    }

    #[test]
    fn blank_single_solid_color() {
        let f = bgra_frame(64, 64, |d| {
            for px in d.as_chunks_mut::<4>().0 {
                px[0] = 10; px[1] = 20; px[2] = 200; px[3] = 255; // 纯蓝
            }
        });
        assert!(is_blank_frame(&f));
    }

    #[test]
    fn valid_two_tone_desktop_like() {
        // 交替两种颜色（模拟桌面文字/背景）→ 至少 2 桶；加第三种确保过阈值。
        let f = bgra_frame(256, 256, |d| {
            for (i, px) in d.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let v = (i % 3) as u8 * 80;
                px[0] = v; px[1] = v / 2; px[2] = 255 - v; px[3] = 255;
            }
        });
        assert!(!is_blank_frame(&f));
    }

    #[test]
    fn valid_gradient() {
        let f = bgra_frame(256, 256, |d| {
            for (i, px) in d.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = (i % 256) as u8;
                px[0] = x; px[1] = x; px[2] = x; px[3] = 255;
            }
        });
        assert!(!is_blank_frame(&f));
    }

    #[test]
    fn blank_empty_and_short_data() {
        let mut f = RawFrame::new_bgra(64, 64);
        f.data.clear();
        assert!(is_blank_frame(&f));
    }
}
