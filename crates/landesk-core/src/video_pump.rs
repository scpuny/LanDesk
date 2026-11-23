//! 受控端视频数据面发流（P2② → 受控闭环，FR-03/FR-04）。
//!
//! 打通「采集 → 编码 → NALU 分片 → QUIC 视频 Datagram」发流泵：
//! - 采集：openh264 feature 下用 [`Self::new_platform`] 接平台真实采屏（失败降级合成源）；
//!   纯逻辑测试用 [`capture::SynthCapture`]（逐帧产真·变化像素，无 OS 依赖）；
//! - 编码：openh264 feature 下用 [`avcodec_wrap::Openh264Encoder`] 真 H.264（一帧一个
//!   Annex-B 访问单元，WebCodecs 可解）；默认关闭该 feature 时用占位后端供离线验证；
//! - NALU 经 [`lp::datagram::VideoDgram::fragment_nalu`] 切成 ≤MTU 分片，逐片发出。
//!
//! 本模块从 `apps/controlled` 下沉到 `landesk-core`，使 GUI（受控端）与 CLI 共用
//! 同一发流实现。[`VideoPump::send`] 兼容旧 CLI（`&OneClickSession`）；
//! [`VideoPump::send_tx`] 用独立 [`VideoDatagramTx`] 句柄，供 GUI 受控端把发流泵
//! `move` 进独立任务，与键鼠控制帧接收并发（不争用会话控制流 `&mut`）。

use std::time::Duration;

// 生产构建（openh264 feature）用真 H.264；纯逻辑测试退回占位后端。
#[cfg(feature = "openh264")]
use avcodec_wrap::Openh264Encoder as DefaultEncoder;
#[cfg(not(feature = "openh264"))]
use avcodec_wrap::PlaceholderEncoder as DefaultEncoder;
use avcodec_wrap::VideoEncoder;
use capture::{is_blank_frame, Monitor, RawFrame, ScreenCapture, SynthCapture};
use quic_link::VideoDatagramTx;

use crate::error::CoreError;
use crate::oneclick::OneClickSession;
use lp::datagram::{NaluKind, VideoDgram, FLAG_FIRST};

/// 发流参数（环回测试 / 生产默认值）。
#[derive(Debug, Clone, Copy)]
pub struct PumpConfig {
    /// 编码器 kind（喂 [`lp::msg::EncodeParams`] 标签；占位后端恒 Openh264=9）。
    pub kind: u8,
    /// 目标帧率（泵按 1/fps 节拍出帧；环回测试取小值加快）。
    pub fps: u32,
}

impl Default for PumpConfig {
    fn default() -> Self {
        Self { kind: 9, fps: 30 }
    }
}

/// 一帧待发 NALU（编码产物，已含 Annex-B 起始码前缀）。
#[derive(Debug, Clone)]
pub struct VideoFrame {
    /// NALU 类型（I / P / SPSPPS）。
    pub kind: u8,
    /// 裸 H.264 NALU 字节（含起始码）。
    pub nalu: Vec<u8>,
}

/// 采屏降级原因（问题 2/5：采屏 API 假成功 / 失败 → 合成源）。
/// 经 B3 的 `CaptureDegraded` 事件跨 QUIC 上报给控制端，文案按此区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    /// 平台采屏构造失败（macOS 未授予屏幕录制权限 / 无显示器 / headless）。
    NoPermission,
    /// 连续多帧近纯色（Windows 锁屏/非交互桌面的全黑帧）。
    BlankFrame,
}
impl DegradedReason {
    /// 稳定标识（跨 QUIC 控制通道传字符串，前端据此选文案）。
    pub fn as_str(self) -> &'static str {
        match self {
            DegradedReason::NoPermission => "no_permission",
            DegradedReason::BlankFrame => "blank_frame",
        }
    }
}

/// 连续黑帧达此数即判定采屏无效并降级（15 帧 ≈ 0.5s@30fps / 1s@15fps）。
const BLANK_FRAMES_TO_DEGRADE: u32 = 15;
/// 降级后每隔多少帧重试一次真实采屏（约 5s@30fps / 7.5s@15fps）。
const RETRY_EVERY_FRAMES: u32 = 150;

/// 视频发流泵：把「采集 → 编码 → NALU」产出的一帧分片后逐片发出。
///
/// 会话视频 Datagram 用 `&self` 发送（quinn 共享 UDP 驱动），故泵不独占会话，
/// 可与其他控制帧并发。
pub struct VideoPump {
    cfg: PumpConfig,
    /// 画面源：优先平台真实采屏，失败降级合成源（无 OS 依赖）。
    capture: Box<dyn ScreenCapture>,
    /// 编码后端：openh264 feature 下为真 H.264（一帧 = 一个 Annex-B 访问单元），
    /// 否则为占位后端（一帧一条假 NALU，仅离线链路验证用）。
    encoder: DefaultEncoder,
    /// 实际采集分辨率（真实采屏取显示器尺寸，合成源取请求值）。
    width: u32,
    height: u32,
    /// 帧序号（喂 VideoDgram.seq，u16 循环空间）。
    seq: u16,
    /// 是否首帧（首帧强制 IDR + SPS/PPS，供对端解码器 flush + 重建参考帧）。
    first: bool,
    /// 当前活跃画面源是否为合成源（降级态）。
    degraded: bool,
    /// 降级原因（供 B3 上报控制端区分文案）。
    degraded_reason: Option<DegradedReason>,
    /// 是否源自平台真实采屏（仅此路径启用黑帧检测/降级；`new()` 合成源测试泵恒 false）。
    from_platform: bool,
    /// 连续判定为黑/纯色帧的计数，达阈值触发降级。
    blank_streak: u32,
    /// 降级后距上次重试真实采屏的帧数（周期重试，成功则恢复）。
    frames_since_retry: u32,
    /// 降级时保存的真实采屏器（active capture 换成合成源后，用它周期重试恢复）。
    real_capture: Option<Box<dyn ScreenCapture>>,
}

impl VideoPump {
    /// 当前是否处于降级态（画面来自合成源而非真实桌面）。
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }
    /// 降级原因（降级态下恒 Some）。
    pub fn degraded_reason(&self) -> Option<DegradedReason> {
        self.degraded_reason
    }
}

impl VideoPump {
    /// 在 `w×h` 分辨率上起泵（fps 见 [`PumpConfig`]），使用合成源。
    ///
    /// 占位编码阶段（期一/期二）不压缩，真实采屏的全屏帧会产生数 MB 数据、分片
    /// 超出 QUIC datagram 容量而丢包——故默认用轻量合成源验证链路。期三真编码
    /// 接入后由 [`Self::new_with_capture`] 切到平台真实采屏。
    pub fn new(w: u32, h: u32, cfg: PumpConfig) -> Result<Self, String> {
        let monitor = Monitor {
            index: 0,
            width: w,
            height: h,
            name_hash: 0,
        };
        Self::build(
            Box::new(SynthCapture::default().with_monitor(monitor)),
            w,
            h,
            cfg,
            false,
            false,
            None,
        )
    }

    /// 内部统一构造：`from_platform` 标记是否源自真实采屏（仅此路径启用黑帧检测/降级）。
    fn build(
        capture: Box<dyn ScreenCapture>,
        w: u32,
        h: u32,
        cfg: PumpConfig,
        from_platform: bool,
        degraded: bool,
        degraded_reason: Option<DegradedReason>,
    ) -> Result<Self, String> {
        let mut encoder = DefaultEncoder::new();
        // cfg.fps = 0 表示泵不节流（回环测试用），但编码器仍需有时基计算时间戳 /
        // IDR 周期：兜底 30fps，不影响 interval() 的"不节流"语义。
        let enc_fps = if cfg.fps == 0 { 30 } else { cfg.fps };
        encoder.open(w, h, enc_fps).map_err(|e| e.to_string())?;
        Ok(Self {
            cfg,
            capture,
            encoder,
            width: w,
            height: h,
            seq: 0,
            first: true,
            degraded,
            degraded_reason,
            from_platform,
            blank_streak: 0,
            frames_since_retry: 0,
            real_capture: None,
        })
    }

    /// 用平台真实采屏 + 主显示器分辨率起泵（openh264 feature 下 GUI 受控端入口）。
    ///
    /// 真编码把全屏帧压到几十 KB，分片量可控，故此时按显示器真实分辨率出帧。
    /// 平台采屏失败（无屏幕录制权限 / headless）时 [`Self::next_frame`] 自动降级
    /// 为同分辨率合成源，发流不中断（画面降级但会话存活）。
    #[cfg(feature = "openh264")]
    pub fn new_platform(cfg: PumpConfig) -> Result<Self, String> {
        // 平台采屏不可用（headless / 无屏幕录制权限 / 无显示器）时降级合成源，
        // 保证远程会话仍有画面——绝不因为采屏失败而停掉整条发流。
        // ★ 标记 degraded=NoPermission：B3 据此经 QUIC 上报控制端 banner，
        //   不再让用户把合成测试图误认为真桌面（问题 2 核心）。
        let fallback = || {
            let monitor = Monitor {
                index: 0,
                width: 1280,
                height: 720,
                name_hash: 0,
            };
            Self::build(
                Box::new(SynthCapture::default().with_monitor(monitor)),
                1280,
                720,
                cfg,
                true,
                true,
                Some(DegradedReason::NoPermission),
            )
        };
        let cap = match capture::platform_capture() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "平台真实采屏不可用，降级为合成源并标记 degraded");
                return fallback();
            }
        };
        let m = match cap.monitors() {
            Ok(ms) => ms.into_iter().next(),
            Err(e) => {
                tracing::warn!(error = %e, "平台显示器枚举失败，降级为合成源并标记 degraded");
                return fallback();
            }
        };
        let Some(m) = m else {
            tracing::warn!("平台未枚举到显示器，降级为合成源并标记 degraded");
            return fallback();
        };
        tracing::info!(
            w = m.width,
            h = m.height,
            "受控端启用平台真实采屏 + 真 H.264 编码"
        );
        Self::new_with_capture(cap, m.width, m.height, cfg)
    }

    /// 用指定采集器起泵（真实采屏接入点）。
    ///
    /// 采集器分辨率由调用方按 `monitors()` 传入（通常与 w×h 一致）；编码器同分辨率打开。
    pub fn new_with_capture(
        capture: Box<dyn ScreenCapture>,
        w: u32,
        h: u32,
        cfg: PumpConfig,
    ) -> Result<Self, String> {
        Self::build(capture, w, h, cfg, true, false, None)
    }

    /// 当前实际采集分辨率。
    pub fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// 造一个同分辨率合成源（降级时的替代画面源）。
    fn synth_capture(&self) -> Box<dyn ScreenCapture> {
        let monitor = Monitor {
            index: 0,
            width: self.width,
            height: self.height,
            name_hash: 0,
        };
        Box::new(SynthCapture::default().with_monitor(monitor))
    }

    /// 切到降级态：保存真实采屏器用于恢复，active 换合成源，标记原因供上报。
    fn degrade_to(&mut self, reason: DegradedReason) {
        if self.degraded {
            return;
        }
        tracing::warn!(reason = reason.as_str(), "采屏内容无效，降级为合成源并上报 degraded");
        let synth = self.synth_capture();
        self.real_capture = Some(std::mem::replace(&mut self.capture, synth));
        self.degraded = true;
        self.degraded_reason = Some(reason);
        self.blank_streak = 0;
        self.frames_since_retry = 0;
        // 切源后强制下一帧 IDR，让对端解码器用新画面重建参考帧。
        self.first = true;
    }

    /// 尝试恢复真实采屏： grab 一帧，若有效则切回并清 degraded。
    fn try_restore(&mut self) {
        let Some(real) = self.real_capture.as_ref() else { return };
        let probe = match real.grab(0) {
            Ok(f) => f,
            Err(_) => return,
        };
        if is_blank_frame(&probe) {
            return;
        }
        // 探测有效：把真实采屏器换回 active，清降级态。
        if let Some(real) = self.real_capture.take() {
            self.capture = real;
        }
        self.degraded = false;
        self.degraded_reason = None;
        self.blank_streak = 0;
        self.first = true; // 恢复真实画面后强制 IDR
        tracing::info!("真实采屏恢复，退出 degraded 态");
    }

    /// 产出一帧 NALU（采集一帧 → 真编码 → 取 NALU）。首帧标 IDR+SPS/PPS，其后标 P 帧。
    ///
    /// 采屏降级两类（问题 2/5）：
    /// - 构造期失败（macOS 无屏幕录制权限）→ `new_platform` 已标 degraded；
    /// - 运行期 grab 失败或连续黑帧（Windows 锁屏/非交互桌面）→ 此处降级。
    ///
    /// 降级后周期重试真实采屏，恢复即清 degraded（权限补授/解锁后自动回真画面）。
    pub fn next_frame(&mut self) -> Result<VideoFrame, String> {
        // 降级态：每隔 N 帧探测一次真实采屏，成功即恢复（不阻塞正常合成源出帧）。
        if self.degraded && self.from_platform {
            self.frames_since_retry += 1;
            if self.frames_since_retry >= RETRY_EVERY_FRAMES {
                self.frames_since_retry = 0;
                self.try_restore();
            }
        }

        let frame = match self.capture.grab(0) {
            Ok(f) => f,
            Err(e) => {
                if self.from_platform && !self.degraded {
                    tracing::warn!(error = %e, "真实采屏 grab 失败，降级为合成源");
                    self.degrade_to(DegradedReason::NoPermission);
                }
                self.capture.grab(0).map_err(|e| e.to_string())?
            }
        };

        // 黑帧检测：仅真实采屏、非降级态时统计（合成源是彩色测试图，天然非 blank）。
        if self.from_platform && !self.degraded {
            if is_blank_frame(&frame) {
                self.blank_streak += 1;
                if self.blank_streak >= BLANK_FRAMES_TO_DEGRADE {
                    tracing::warn!(streak = self.blank_streak, "连续黑帧达阈值，判定采屏无效");
                    self.degrade_to(DegradedReason::BlankFrame);
                    // 本帧改发合成源画面（避免把刚判定的黑帧编码发出）。
                    return self.encode_frame(self.capture.grab(0).map_err(|e| e.to_string())?);
                }
            } else {
                self.blank_streak = 0;
            }
        }

        self.encode_frame(frame)
    }

    /// 把一帧 RawFrame 编码成待发 NALU（首帧/切源后 IDR）。
    fn encode_frame(&mut self, frame: RawFrame) -> Result<VideoFrame, String> {
        self.encoder
            .send_frame(&frame.data)
            .map_err(|e| e.to_string())?;
        let nalu = self
            .encoder
            .recv_nalu()
            .ok_or_else(|| "no nalu after send_frame".to_string())?;
        // 首帧强制 IDR；其后由编码器上报真实 IDR（openh264 周期关键帧），
        // 保证对端 VideoDecoder 能正确 flush / 重建参考帧。
        let is_idr = self.first || self.encoder.is_keyframe();
        let kind = if is_idr {
            NaluKind::IdrWithSpsPps as u8
        } else {
            NaluKind::PFrame as u8
        };
        self.first = false;
        Ok(VideoFrame { kind, nalu })
    }

    /// 取帧节拍（1/fps；fps=0 不节流）。
    pub fn interval(&self) -> Option<Duration> {
        (self.cfg.fps > 0).then(|| Duration::from_millis(1000 / self.cfg.fps.max(1) as u64))
    }

    /// 把一帧 NALU 分片（首片置 FLAG_FIRST）。递增 seq，返回分片列表。
    fn fragment(&mut self, vf: &VideoFrame) -> Vec<VideoDgram> {
        self.seq = self.seq.wrapping_add(1);
        let flags = if vf.kind == NaluKind::IdrWithSpsPps as u8 {
            FLAG_FIRST
        } else {
            0
        };
        VideoDgram::fragment_nalu(&vf.nalu, self.seq, vf.kind, flags)
    }

    /// 分片后经 `OneClickSession::send_video` 发出（CLI / 回环测试兼容入口）。
    pub fn send(&mut self, session: &OneClickSession, vf: &VideoFrame) -> Result<u32, CoreError> {
        let frags = self.fragment(vf);
        let n = frags.len() as u32;
        for f in &frags {
            session.send_video(&f.encode_plain()?)?;
        }
        Ok(n)
    }

    /// 分片后经独立 [`VideoDatagramTx`] 发出（GUI 受控端发流泵入口）。
    ///
    /// 与 [`Self::send`] 等价，但只持有克隆出的视频发送句柄，不占会话控制流
    /// `&mut`，可与键鼠控制帧接收循环并发。
    pub fn send_tx(&mut self, tx: &VideoDatagramTx, vf: &VideoFrame) -> Result<u32, CoreError> {
        let frags = self.fragment(vf);
        let n = frags.len() as u32;
        for f in &frags {
            tx.send(&f.encode_plain()?)?;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capture::CaptureError;

    /// 模拟 Windows 锁屏：grab 成功但产全黑帧（API 假成功的典型）。
    struct BlankCapture {
        monitor: Monitor,
        /// 可切换为产出彩色帧，用于验证降级后恢复。
        good: std::sync::atomic::AtomicBool,
    }
    impl ScreenCapture for BlankCapture {
        fn monitors(&self) -> Result<Vec<Monitor>, CaptureError> {
            Ok(vec![self.monitor])
        }
        fn grab(&self, _i: u8) -> Result<RawFrame, CaptureError> {
            let mut f = RawFrame::new_bgra(self.monitor.width, self.monitor.height);
            if self.good.load(std::sync::atomic::Ordering::SeqCst) {
                // 三色条纹（非 blank）
                for (i, px) in f.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let v = (i % 3) as u8 * 90;
                    px[0] = v; px[1] = 255 - v; px[2] = 120; px[3] = 255;
                }
            }
            Ok(f) // 全黑也返回 Ok（假成功）
        }
        fn poll_event(&mut self) -> Option<capture::DisplayEvent> { None }
    }

    #[test]
    fn blank_frames_degrade_with_reason() {
        let monitor = Monitor { index: 0, width: 64, height: 64, name_hash: 0 };
        let cap = BlankCapture { monitor, good: std::sync::atomic::AtomicBool::new(false) };
        let mut p = VideoPump::new_with_capture(Box::new(cap), 64, 64, PumpConfig { kind: 9, fps: 0 })
            .expect("pump");
        assert!(!p.is_degraded(), "初始未降级");
        // 连续喂黑帧超过阈值 → 降级（仍能产出合成源帧，会话不中断）
        for _ in 0..(BLANK_FRAMES_TO_DEGRADE + 2) {
            let f = p.next_frame().expect("frame");
            assert!(!f.nalu.is_empty());
        }
        assert!(p.is_degraded(), "连续黑帧后应降级");
        assert_eq!(p.degraded_reason(), Some(DegradedReason::BlankFrame));
    }

    #[test]
    fn degraded_frame_is_not_blank() {
        // 降级后产出的是合成源彩色测试图，不应再被判为 blank（防二次误降级）。
        let monitor = Monitor { index: 0, width: 64, height: 64, name_hash: 0 };
        let cap = BlankCapture { monitor, good: std::sync::atomic::AtomicBool::new(false) };
        let mut p = VideoPump::new_with_capture(Box::new(cap), 64, 64, PumpConfig { kind: 9, fps: 0 })
            .expect("pump");
        for _ in 0..(BLANK_FRAMES_TO_DEGRADE + 1) { p.next_frame().expect("frame"); }
        let synth_frame = p.capture.grab(0).expect("synth grab");
        assert!(!is_blank_frame(&synth_frame), "合成源应为彩色，不再触发降级");
    }

    #[test]
    fn synthetic_pump_never_degrades() {
        // new() 合成源测试泵 from_platform=false，不应启用黑帧检测。
        let mut p = VideoPump::new(64, 64, PumpConfig { kind: 9, fps: 0 }).expect("pump");
        for _ in 0..20 { p.next_frame().expect("frame"); }
        assert!(!p.is_degraded(), "纯合成源泵不应降级");
    }

    #[test]
    fn first_frame_is_idr_then_p() {
        let mut p = VideoPump::new(32, 24, PumpConfig { kind: 9, fps: 0 }).expect("pump");
        assert_eq!(
            p.next_frame().expect("f0").kind,
            NaluKind::IdrWithSpsPps as u8
        );
        assert_eq!(p.next_frame().expect("f1").kind, NaluKind::PFrame as u8);
        assert_eq!(p.interval(), None, "fps=0 不节流");
    }

    /// 平台采屏入口在 headless / 无权限环境下必须降级而非失败（会话不能没画面）。
    #[cfg(feature = "openh264")]
    #[test]
    fn platform_pump_degrades_instead_of_failing() {
        let mut p = VideoPump::new_platform(PumpConfig { kind: 9, fps: 0 })
            .expect("平台采屏应降级为合成源，而不是 open 失败");
        let f = p.next_frame().expect("应能产出编码帧");
        assert!(!f.nalu.is_empty(), "编码帧不应为空");
        assert_eq!(f.kind, NaluKind::IdrWithSpsPps as u8, "首帧为 IDR");
    }
}
