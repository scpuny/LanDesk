//! 编码器降级链（FR-04.2：按优先级尝试硬编，失败自动降级 OpenH264）。
//!
//! 各平台优先级链（对齐 `docs/architecture.md` §2 编码器表）：
//! - Windows：qsv → nvenc → amf → openh264
//! - Linux PC：qsv → nvenc → v4l2m2m → openh264
//! - OK3568：v4l2m2m → openh264
//! - macOS：videotoolbox → openh264
//!
//! 编码器 kind 值与 `lp::msg::EncoderKind` 一致。
//! M1/M2 阶段：`opener` 为纯判定（给定 kind 返回 open 是否成功），
//! 编码器实例的真实创建/FFI 在后续里程碑接入，本层保证降级顺序可单测。

use crate::{EncoderCaps, EncoderError};

const QSV: u8 = 1;
const NVENC: u8 = 2;
const AMF: u8 = 3;
const V4L2M2M: u8 = 4;
const VIDEOTOOLBOX: u8 = 5;
const OPENH264: u8 = 9;

/// 各平台优先级链。
pub fn caps_for(platform: Platform) -> EncoderCaps {
    let arr: [u8; 6];
    let count: u8 = match platform {
        Platform::Windows => {
            arr = [QSV, NVENC, AMF, OPENH264, 0, 0];
            4
        }
        Platform::LinuxPc => {
            arr = [QSV, NVENC, V4L2M2M, OPENH264, 0, 0];
            4
        }
        Platform::Ok3568 => {
            arr = [V4L2M2M, OPENH264, 0, 0, 0, 0];
            2
        }
        Platform::Macos => {
            arr = [VIDEOTOOLBOX, OPENH264, 0, 0, 0, 0];
            2
        }
    };
    EncoderCaps { priority: arr, count }
}

/// 平台标识（M1 编译期常量，后续由构建脚本注入）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    LinuxPc,
    Ok3568,
    Macos,
}

/// 编码器工厂：按 caps 优先级链依次尝试 open，首个成功即选定；
/// 全部失败报 [`EncoderError::OpenFailed`]（理论上 openh264 兜底不会全败）。
pub struct EncoderFactory {
    caps: EncoderCaps,
}

impl EncoderFactory {
    pub fn new(platform: Platform) -> Self {
        Self {
            caps: caps_for(platform),
        }
    }

    /// 依次尝试各编码器。`opener(kind) -> Ok` 表示该编码器可 open，
    /// 返回选定的 kind；全部 `Err` 时返回最后一个错误。
    pub fn open_first<F>(&self, mut opener: F) -> Result<u8, EncoderError>
    where
        F: FnMut(u8) -> Result<(), EncoderError>,
    {
        let mut last_err = EncoderError::OpenFailed("no encoder attempted".into());
        for kind in self.caps.iter() {
            match opener(kind) {
                Ok(()) => return Ok(kind),
                Err(e) => {
                    last_err = e;
                    tracing::warn!(kind, "encoder open failed, degrading");
                }
            }
        }
        Err(last_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_priority_chains() {
        let w = caps_for(Platform::Windows);
        let mut w_iter = w.iter();
        assert_eq!(w_iter.next(), Some(QSV));
        assert_eq!(w_iter.next(), Some(NVENC));
        assert_eq!(w_iter.next(), Some(AMF));
        assert_eq!(w_iter.next(), Some(OPENH264));
        assert_eq!(w_iter.next(), None);

        let o = caps_for(Platform::Ok3568);
        let mut o_iter = o.iter();
        assert_eq!(o_iter.next(), Some(V4L2M2M));
        assert_eq!(o_iter.next(), Some(OPENH264));
        assert_eq!(o_iter.next(), None);

        let m = caps_for(Platform::Macos);
        let mut m_iter = m.iter();
        assert_eq!(m_iter.next(), Some(VIDEOTOOLBOX));
        assert_eq!(m_iter.next(), Some(OPENH264));
        assert_eq!(m_iter.next(), None);
    }

    #[test]
    fn factory_selects_first_success() {
        let f = EncoderFactory::new(Platform::Windows);
        let kind = f.open_first(|_k| Ok(())).unwrap();
        assert_eq!(kind, QSV, "全部可 open 时选首位");
    }

    #[test]
    fn factory_degrades_on_failure() {
        let f = EncoderFactory::new(Platform::Windows);
        let kind = f
            .open_first(|k| {
                if k == OPENH264 {
                    Ok(())
                } else {
                    Err(EncoderError::OpenFailed("simulated driver failure".into()))
                }
            })
            .unwrap();
        assert_eq!(kind, OPENH264, "硬编全失败应降级软编兜底");
    }

    #[test]
    fn factory_all_fail_reports_open_failed() {
        let f = EncoderFactory::new(Platform::Ok3568);
        let res = f.open_first(|_| Err(EncoderError::OpenFailed("all down".into())));
        assert!(matches!(res, Err(EncoderError::OpenFailed(_))));
    }
}
