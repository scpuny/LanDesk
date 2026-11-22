//! GUI 兼作受控端（提醒4 / 完整远程控制，期一）：按设置页访问密码起 QUIC 服务端，
//! 接受控制端连接 → 握手 → 起视频发流泵 + 控制帧接收循环。
//!
//! 以前受控端只有独立 CLI（`apps/controlled`），GUI 只起 mDNS Listener + Advertiser。
//! 本模块把受控能力接进 GUI / ui-server：装了 LanDesk 的两端，A 在设备列表点 B、
//! 输 B 设置页访问密码，即可连上 B 的会话（B 发画面、收键鼠）。
//!
//! ## 密码门禁（用户决策：空密码不启动受控服务）
//!
//! `Config.password = Some(p)` 且非空 → 在 52000 端口起 `Controlled`；
//! `None` / 空串 → 不启动，GUI 保持纯控制端行为（安全默认）。
//! 密码派生遵循 `bootstrap` + `auth` 协议：控制端用 `dev.ad.id`（mDNS 广播的
//! 设备码）派生 bootstrap key 与 proof，故本端 `Controlled.device_id` **必须**等于
//! 广播的 `device_code_from(device_name)`，两端才能握手成功。
//!
//! ## 并发模型
//!
//! 每条会话拆两路：视频发流用 [`VideoDatagramTx`]（克隆的发送句柄，不占会话
//! `&mut`），`OneClickSession` 本体留给控制帧接收循环——两路互不争用（期四接
//! 真实键鼠注入时仍成立）。

use landesk_core::config::Config;
use landesk_core::error::CoreError;
use landesk_core::oneclick::{Controlled, OneClickSession};
use landesk_core::video_pump::{PumpConfig, VideoPump};
use mdns::DEFAULT_PORT;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::device_code_from;

/// 帧率（真编码 + 真采屏按显示器分辨率出帧；占位构建用下方中小分辨率兜底，
/// 因占位编码不压缩，全屏帧会产生数 MB 数据导致 QUIC 分片丢包）。
#[cfg(not(feature = "openh264"))]
const VIDEO_W: u32 = 640;
#[cfg(not(feature = "openh264"))]
const VIDEO_H: u32 = 360;
const VIDEO_FPS: u32 = 15;

/// GUI 受控服务状态（供设置页展示「受控服务：已启用/未启用」）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlledStatus {
    /// 是否正在监听（设置了非空访问密码）。
    pub serving: bool,
    /// 监听端口（受控服务固定 52000，与 mDNS 广播一致）。
    pub port: u16,
    /// 当前设备码（受控端握手身份，必须与控制端发现到的 ad.id 一致）。
    pub device_code: String,
}

/// GUI 受控端宿主：持有 QUIC 服务端 + accept 任务，保活到进程结束。
///
/// `Drop` 时 abort accept 任务 → 内部 `Controlled` 释放 → QUIC 端点关闭，
/// 对端自然掉线（优雅下线）。
pub struct ControlledHost {
    device_code: String,
    serving: bool,
    /// accept 循环任务句柄（保活 + Drop 时 abort）。
    task: Option<JoinHandle<()>>,
}

impl ControlledHost {
    /// 按配置启动受控服务；须在 tokio runtime 内调用（QUIC 端点要注册 UDP 驱动）。
    ///
    /// - `password = None` / 空串 → 不启动（`is_serving() = false`），GUI 纯控制端；
    /// - 端口被占用 / 证书生成失败 → 返回 `Err`，由调用方降级为纯控制端并告警，
    ///   不影响发起连接。
    pub fn start(cfg: &Config) -> Result<Self, CoreError> {
        let device_code = device_code_from(&cfg.device_name);
        let Some(password) = cfg.password.as_deref().filter(|p| !p.trim().is_empty()) else {
            return Ok(Self {
                device_code,
                serving: false,
                task: None,
            });
        };
        let password = password.to_string();

        // 监听 0.0.0.0:DEFAULT_PORT（52000）——与 mDNS 广播端口一致，
        // 控制端发现到的 ad.port 即拨号目标。证书 SNI 兼容 "localhost" 环回测试。
        let listen = format!("0.0.0.0:{DEFAULT_PORT}")
            .parse()
            .map_err(|_| CoreError::Unreachable("bad controlled listen addr".into()))?;
        let controlled = Controlled::new(listen, &["localhost"], device_code.clone())?;
        let local_port = controlled
            .server()
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(DEFAULT_PORT);
        debug!(%device_code, port = local_port, "GUI 受控端 QUIC 服务端已启动");

        let task = tokio::spawn(accept_loop(controlled, password, device_code.clone()));
        Ok(Self {
            device_code,
            serving: true,
            task: Some(task),
        })
    }

    /// 是否正在提供受控服务（设置了非空访问密码且服务端绑定成功）。
    pub fn is_serving(&self) -> bool {
        self.serving
    }

    /// 当前设备码（受控端身份）。
    pub fn device_code(&self) -> &str {
        &self.device_code
    }

    /// 供设置页/本机信息展示的状态快照。
    pub fn status(&self) -> ControlledStatus {
        ControlledStatus {
            serving: self.serving,
            port: DEFAULT_PORT,
            device_code: self.device_code.clone(),
        }
    }
}

impl Drop for ControlledHost {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

/// 接受循环：逐条接受控制端连接（每条握手独立校验密码），每条会话起独立任务处理，
/// 接受循环回到 `wait` 继续等下一条（支持多控制端并发）。
async fn accept_loop(controlled: Controlled, password: String, device_code: String) {
    loop {
        match controlled.wait(&password).await {
            Ok(session) => {
                let dc = device_code.clone();
                tokio::spawn(async move {
                    serve_session(session, dc).await;
                });
            }
            Err(e) => {
                // 一次握手失败（密码错 / 对端异常断开）不应终止服务，继续等下一条。
                warn!(error = %e, "受控端会话握手失败，继续等待新连接");
                // 避免异常连接导致的忙循环（如 endpoint 已关闭）。
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

/// 单条会话：起视频发流泵（独立 tx 句柄）+ 控制帧接收循环，两者并发。
async fn serve_session(mut session: OneClickSession, device_code: String) {
    let remote = session.remote_addr();
    debug!(%remote, %device_code, "受控端会话建立");

    // 视频发流：用克隆的 VideoDatagramTx，独立任务按 fps 出帧。
    let tx = session.video_tx();
    let video_task: JoinHandle<()> = tokio::spawn(async move {
        let cfg = PumpConfig {
            kind: 9,
            fps: VIDEO_FPS,
        };
        #[cfg(feature = "openh264")]
        let pump_result = VideoPump::new_platform(cfg);
        #[cfg(not(feature = "openh264"))]
        let pump_result = VideoPump::new(VIDEO_W, VIDEO_H, cfg);
        let mut pump = match pump_result {
            Ok(p) => p,
            Err(e) => {
                warn!(error = %e, "受控端视频泵 open 失败，停发流");
                return;
            }
        };
        let interval = pump.interval();
        loop {
            let vf = match pump.next_frame() {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "受控端采集/编码失败，停发流");
                    return;
                }
            };
            match pump.send_tx(&tx, &vf) {
                Ok(n) => debug!(fragments = n, "受控端发出视频分片"),
                Err(e) => {
                    warn!(error = %e, "受控端 send_video 失败，停发流（对端关闭）");
                    return;
                }
            }
            if let Some(d) = interval {
                tokio::time::sleep(d).await;
            }
        }
    });

    // 控制帧接收：session 本体独占 &mut，逐帧把键鼠意图注入本机（FR-06）。
    // 注入器构造失败（macOS 未授权辅助功能 / Linux 无 uinput 权限）→ 降级为只收不注入，
    // 不影响视频发流与会话存活。
    let injector = match input::platform_injector() {
        Ok(i) => Some(i),
        Err(e) => {
            warn!(error = %e, "平台键鼠注入器不可用，受控端降级为只发画面");
            None
        }
    };
    let (screen_w, screen_h) = input::primary_screen_size();
    loop {
        match session.recv_control().await {
            Ok(msg) => {
                if let Some(ref inj) = injector {
                    match landesk_core::apply_input(inj.as_ref(), &msg, screen_w, screen_h) {
                        Ok(landesk_core::DispatchResult::Injected) => {
                            debug!(?msg, "受控端注入输入帧");
                        }
                        Ok(skip) => {
                            debug!(?skip, "受控端跳过未知输入帧");
                        }
                        Err(e) => {
                            warn!(error = %e, "受控端输入注入失败");
                        }
                    }
                } else {
                    debug!(?msg, "受控端收到控制帧（注入器不可用，未注入）");
                }
            }
            Err(e) => {
                debug!(error = %e, "受控端控制流结束");
                break;
            }
        }
    }

    video_task.abort();
    session.close(0, b"controlled session end");
    debug!(%remote, "受控端会话结束");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_password_does_not_serve() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: None,
            ..Default::default()
        };
        // start 在无 runtime 时也不会走到 tokio::spawn（空密码提前返回），可离线测。
        let host = ControlledHost::start(&cfg).expect("empty password start");
        assert!(!host.is_serving(), "空密码不启动受控服务");
        assert_eq!(host.device_code(), device_code_from("devboard"));
        let st = host.status();
        assert!(!st.serving);
        assert_eq!(st.port, DEFAULT_PORT);
    }

    #[test]
    fn blank_password_does_not_serve() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: Some("   ".into()),
            ..Default::default()
        };
        let host = ControlledHost::start(&cfg).expect("blank password start");
        assert!(!host.is_serving(), "纯空白密码视为未设置");
    }
}
