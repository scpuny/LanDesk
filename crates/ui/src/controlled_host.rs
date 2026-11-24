//! GUI 兼作受控端（提醒4 / 完整远程控制，期一）：按设置页凭证起 QUIC 服务端，
//! 接受控制端连接 → 握手 → 起视频发流泵 + 控制帧接收循环。
//!
//! 以前受控端只有独立 CLI（`apps/controlled`），GUI 只起 mDNS Listener + Advertiser。
//! 本模块把受控能力接进 GUI / ui-server：装了 LanDesk 的两端，A 在设备列表点 B、
//! 输 B 的凭证（访问密码 或 屏幕上亮的临时密码），即可连上 B 的会话。
//!
//! ## 双凭证门禁（用户决策：访问密码 与 临时密码 都要，且是不同的授权通道）
//!
//! - **访问密码**（`Config.password`）：长期自用，设一次一直有效（永久）。
//! - **临时密码**（6 位配对码）：给短期放别人进来，**默认开 + 定时自动过期**
//!   （`Config.temp_password_ttl_secs` 到点换新码、旧码作废）。
//!
//! 起服门禁 = **访问密码非空 OR 临时通道开**（[`serving_for`]）。两者都关才纯控制端。
//! 每条连接用「访问密码 + 当前临时码」作候选表喂 [`landesk_core::Controlled::wait_any`]，
//! 控制端填任一对（`Controlled::wait_any` 多候选 GCM 逐枚试）即放行——故**没设访问密码
//! 也能靠默认开启的临时码被连**（根治「没填永久密码就只能当控制端」）。
//!
//! 密码派生遵循 `bootstrap` + `auth` 协议：控制端用 `dev.ad.id`（mDNS 广播的设备码）
//! 派生 bootstrap key 与 proof，故本端 `Controlled.device_id` **必须**等于广播的
//! `device_code_from(device_name)`，两端才能握手成功。
//!
//! ## 并发模型
//!
//! 两条后台任务 + 一个共享 [`AuthMaterial`]（双凭证材料，`Arc` 持有）：
//! - **临时码旋转任务**：按 `ttl_secs` 周期换新码（写 `material.temp_code`）；
//! - **accept 循环任务**：每条连接取 `material.candidates()`（[访问密码?, 当前临时码?]）
//!   喂 `wait_any`，命中任一即 `serve_session`；
//! - `AuthMaterial` 同时被 `local_info` 读 [`ControlledHost::current_temp_code`] 展示
//!   **当前有效**临时码（与门禁同源——所见即可连，不再展示一次性的假码）。
//!
//! 视频发流用克隆的 `VideoDatagramTx`（不占会话 `&mut`），`OneClickSession` 本体留给
//! 控制帧接收循环——两路互不争用（期四接真实键鼠注入时仍成立）。

use std::sync::Arc;

use landesk_core::config::Config;
use landesk_core::error::CoreError;
use landesk_core::oneclick::{Controlled, OneClickSession};
use landesk_core::video_pump::{PumpConfig, VideoPump};
use mdns::DEFAULT_PORT;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

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
    /// 是否正在监听（**访问密码非空 或 临时通道开**，见 [`serving_for`]）。
    pub serving: bool,
    /// 监听端口（受控服务固定 52000，与 mDNS 广播一致）。
    pub port: u16,
    /// 当前设备码（受控端握手身份，必须与控制端发现到的 ad.id 一致）。
    pub device_code: String,
    /// 临时密码通道是否开启（前端据此提示「临时码可用 / 仅访问密码」）。
    pub temp_code_enabled: bool,
}

/// 起服门禁判定：访问密码非空 **或** 临时通道开 → 需起 52000 受控服务。
///
/// 用户决策「双凭证都要」：永久密码（访问）自用、临时码放人，二者独立——任一有效即可被连。
/// 默认临时通道开（`Config::default().temp_password_enabled = true`），故装了 LanDesk 的机器
/// 即使没设访问密码也能被当前临时码连接（根治「没填永久密码就只能当控制端」+ nmap 52000 closed）。
fn serving_for(cfg: &Config) -> bool {
    let access_set = cfg.password.as_deref().is_some_and(|p| !p.trim().is_empty());
    access_set || cfg.temp_password_enabled
}

/// GUI 受控端宿主：持有 QUIC 服务端 + 双凭证材料 + 两个后台任务（旋转码 / accept），
/// 保活到进程结束。`Drop` 时 abort 两任务 → 内部 `Controlled` 释放 → QUIC 端点关闭，
/// 对端自然掉线（优雅下线）。
pub struct ControlledHost {
    device_code: String,
    serving: bool,
    /// 双凭证材料（访问密码 + 当前临时码）：accept 循环取候选、本机信息读当前码同源。
    material: Arc<AuthMaterial>,
    /// accept 循环任务（保活 + Drop 时 abort）。
    task: Option<JoinHandle<()>>,
    /// 临时码旋转任务（保活 + Drop 时 abort）。
    rotate_task: Option<JoinHandle<()>>,
}

/// 双凭证材料（受控端握手门禁的候选来源）：
/// - `access`：访问密码（可选；非空才进候选表）；
/// - `temp_code`：当前有效临时码（临时通道开时存在，由旋转任务按 TTL 换新）；
///
/// 控制端填任一即过门禁（`Controlled::wait_any` 逐候选 GCM 试）。本机信息展示
/// [`ControlledHost::current_temp_code`] 与本结构同源——**所见即可连**。
pub struct AuthMaterial {
    access: Option<String>,
    temp_code: std::sync::RwLock<Option<String>>,
}

impl AuthMaterial {
    fn new(access: Option<String>, initial_temp: Option<String>) -> Self {
        Self {
            access,
            temp_code: std::sync::RwLock::new(initial_temp),
        }
    }

    /// 当前候选表（[访问密码?, 当前临时码?]，去空 + 去重）。
    pub fn candidates(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(2);
        if let Some(a) = self.access.clone().filter(|p| !p.trim().is_empty()) {
            out.push(a);
        }
        if let Ok(g) = self.temp_code.read() {
            if let Some(t) = g.clone().filter(|p| !p.trim().is_empty()) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
        }
        out
    }

    /// 换新临时码（旋转任务 / 手动刷新调用）。
    pub fn set_temp_code(&self, code: String) {
        if let Ok(mut g) = self.temp_code.write() {
            *g = Some(code);
        }
    }

    /// 当前有效临时码（本机信息展示用；通道关 = `None`）。
    pub fn current_temp_code(&self) -> Option<String> {
        self.temp_code.read().ok().and_then(|g| g.clone())
    }
}

impl ControlledHost {
    /// 按配置启动受控服务；须在 tokio runtime 内调用（QUIC 端点要注册 UDP 驱动）。
    ///
    /// - 起服条件 [`serving_for`]：访问密码非空 **或** 临时通道开（默认开）。
    ///   双凭证都关（`password` 空 且 `temp_password_enabled=false`）→ 不启动，纯控制端；
    /// - 临时通道开 → 生成初值临时码 + 起旋转任务（按 `temp_password_ttl_secs` 换新）；
    /// - 端口被占用 / 证书生成失败 → 返回 `Err`，由调用方降级为纯控制端并告警。
    pub fn start(cfg: &Config) -> Result<Self, CoreError> {
        let device_code = device_code_from(&cfg.device_name);
        let access = cfg
            .password
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(str::to_string);

        if !serving_for(cfg) {
            // 双凭证都关：纯控制端，不监听 52000。
            return Ok(Self {
                device_code,
                serving: false,
                material: Arc::new(AuthMaterial::new(access, None)),
                task: None,
                rotate_task: None,
            });
        }

        // 临时通道开 → 初值临时码 + 旋转任务；关 → 仅访问密码候选。
        let access_set = access.is_some(); // 取布尔后再 move 进 material
        let temp_enabled = cfg.temp_password_enabled;
        let initial_temp = if temp_enabled {
            Some(crate::gen_temp_password())
        } else {
            None
        };
        let material = Arc::new(AuthMaterial::new(access, initial_temp));

        // 监听 0.0.0.0:DEFAULT_PORT（52000）——与 mDNS 广播端口一致。
        let listen = format!("0.0.0.0:{DEFAULT_PORT}")
            .parse()
            .map_err(|_| CoreError::Unreachable("bad controlled listen addr".into()))?;
        let controlled = Controlled::new(listen, &["localhost"], device_code.clone())?;
        let local_port = controlled
            .server()
            .local_addr()
            .map(|a| a.port())
            .unwrap_or(DEFAULT_PORT);
        debug!(
            %device_code,
            port = local_port,
            access_set,
            temp_enabled,
            "GUI 受控端 QUIC 服务端已启动（双凭证门禁）"
        );

        // 旋转任务：按 TTL 周期换新临时码（通道关则不起）。
        let rotate_task = if temp_enabled {
            let mat = Arc::clone(&material);
            let ttl = std::time::Duration::from_secs(cfg.temp_password_ttl_secs.max(30) as u64);
            Some(tokio::spawn(rotate_temp_code(mat, ttl)))
        } else {
            None
        };

        let task = tokio::spawn(accept_loop(controlled, Arc::clone(&material), device_code.clone()));
        Ok(Self {
            device_code,
            serving: true,
            material,
            task: Some(task),
            rotate_task,
        })
    }

    /// 是否正在提供受控服务（[`serving_for`] 命中且服务端绑定成功）。
    pub fn is_serving(&self) -> bool {
        self.serving
    }

    /// 当前设备码（受控端身份）。
    pub fn device_code(&self) -> &str {
        &self.device_code
    }

    /// 当前有效临时码（本机信息展示；与门禁同源——所见即可连；通道关 = `None`）。
    pub fn current_temp_code(&self) -> Option<String> {
        self.material.current_temp_code()
    }

    /// 手动刷新临时码（设置页「刷新」按钮；旋转任务到期也会自动换新）。
    pub fn rotate_temp_code_now(&self) {
        if self.material.current_temp_code().is_some() {
            self.material.set_temp_code(crate::gen_temp_password());
        }
    }

    /// 供设置页/本机信息展示的状态快照。
    pub fn status(&self, cfg: &Config) -> ControlledStatus {
        ControlledStatus {
            serving: self.serving,
            port: DEFAULT_PORT,
            device_code: self.device_code.clone(),
            temp_code_enabled: cfg.temp_password_enabled,
        }
    }
}

impl Drop for ControlledHost {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        if let Some(t) = self.rotate_task.take() {
            t.abort();
        }
    }
}

/// 临时码旋转任务：每 `ttl` 秒换新 6 位码（旧码即时作废，新码才进候选表）。
/// Drop（宿主下线）时任务被 abort，自然停止。
async fn rotate_temp_code(material: Arc<AuthMaterial>, ttl: std::time::Duration) {
    loop {
        tokio::time::sleep(ttl).await;
        let code = crate::gen_temp_password();
        material.set_temp_code(code.clone());
        debug!(code, "受控端临时码已轮换（旧码作废）");
    }
}

/// 接受循环：逐条接受控制端连接，每条用当前「访问密码 + 临时码」候选表喂 `wait_any`
/// （任一命中即放行），每条会话起独立任务处理；循环回到 `wait_any` 继续等下一条。
async fn accept_loop(controlled: Controlled, material: Arc<AuthMaterial>, device_code: String) {
    loop {
        // 每条连接取**当下**的候选表（临时码可能已旋转，访问密码稳定）——所见即门禁。
        let candidates = material.candidates();
        match controlled.wait_any(&candidates).await {
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
        // 连续发送失败容忍度：`send_video` 把「连接已死 / 对端未开 datagram / 瞬时限长」
        // 都归一为 `VideoDropped`。超限才停发流，避免一次瞬时抖动（MTU 未收敛窗口、
        // 路径 MTU 短暂低于分片长）就把整条画面流永久打黑。
        const MAX_CONSECUTIVE_SEND_FAILS: u32 = 30;
        let mut send_fails: u32 = 0;
        loop {
            let vf = match pump.next_frame() {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "受控端采集/编码失败，停发流");
                    return;
                }
            };
            match pump.send_tx(&tx, &vf) {
                Ok(n) => {
                    if send_fails > 0 {
                        debug!(recovered_after = send_fails, "受控端视频发送恢复");
                    }
                    send_fails = 0;
                    debug!(fragments = n, "受控端发出视频分片");
                }
                Err(e) => {
                    send_fails += 1;
                    warn!(error = %e, consecutive = send_fails, "受控端 send_video 失败");
                    if send_fails >= MAX_CONSECUTIVE_SEND_FAILS {
                        warn!(consecutive = send_fails, "受控端连续发流失败超限，停发流（连接失效 / 对端关闭）");
                        return;
                    }
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
    // 输入帧采样日志：前 12 帧 + 每 200 帧一条 INFO，用于双机排障时证明
    // 「控制端点击是否真的到达受控端、归一化坐标落在哪、注入是否成功」。
    // 全量帧仍走 DEBUG，不把日志 ring 冲爆。
    let mut input_count: u64 = 0;
    loop {
        match session.recv_control().await {
            Ok(msg) => {
                if let Some(ref inj) = injector {
                    input_count += 1;
                    let sampled = input_count <= 12 || input_count.is_multiple_of(200);
                    let applied = landesk_core::apply_input(inj.as_ref(), &msg, screen_w, screen_h);
                    if sampled {
                        info!(count = input_count, screen_w, screen_h, ?msg, result = ?applied,
                            "受控端输入帧采样");
                    }
                    match applied {
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

    // 双凭证都关（访问密码空 且 临时通道关）→ 纯控制端，不起 52000。
    // 双关时 `start` 提前返回（不 spawn / 不 bind），**无 tokio runtime 也能跑**（离线可测）。
    #[test]
    fn both_credentials_off_does_not_serve() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: None,
            temp_password_enabled: false,
            ..Default::default()
        };
        assert!(!serving_for(&cfg), "访问密码空 + 临时通道关 → 不起服");
        let host = ControlledHost::start(&cfg).expect("dual-off start");
        assert!(!host.is_serving(), "双凭证关 → 不监听 52000");
        assert_eq!(host.device_code(), device_code_from("devboard"));
    }

    // 空白访问密码 + 临时通道关 → 视同未设，双关不服务。
    #[test]
    fn blank_access_and_temp_off_does_not_serve() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: Some("   ".into()),
            temp_password_enabled: false,
            ..Default::default()
        };
        assert!(!serving_for(&cfg), "纯空白访问密码视为未设置");
        let host = ControlledHost::start(&cfg).expect("blank+dual-off start");
        assert!(!host.is_serving());
    }

    // 用户决策「双凭证都要 + 临时通道默认开」：访问密码为空、临时通道默认开 → 仍起服。
    // 这是根治「没填永久密码就只能当控制端」+ nmap 52000 closed 的核心判定（纯逻辑，不起 QUIC）。
    #[test]
    fn default_temp_channel_on_serves_without_access() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: None,
            ..Default::default()
        };
        assert!(cfg.temp_password_enabled, "临时通道默认开（Config::default）");
        assert!(serving_for(&cfg), "访问密码空 + 临时通道默认开 → 起服");
    }

    // 访问密码非空（无论临时通道开关）→ 起服。
    #[test]
    fn access_password_always_serves() {
        let cfg = Config {
            device_name: "devboard".into(),
            password: Some("s3cret".into()),
            temp_password_enabled: false,
            ..Default::default()
        };
        assert!(serving_for(&cfg), "访问密码非空 → 起服");
    }

    // 候选表组装：访问 + 当前临时码，去空去重。
    #[test]
    fn candidates_dedupe_and_drop_blank() {
        let m = AuthMaterial::new(Some("  ".into()), Some("123456".into()));
        // 访问密码纯空白被丢弃，只留临时码。
        assert_eq!(m.candidates(), vec!["123456".to_string()]);

        let m2 = AuthMaterial::new(Some("access".into()), Some("temp".into()));
        assert_eq!(m2.candidates(), vec!["access".to_string(), "temp".to_string()]);

        let m3 = AuthMaterial::new(Some("same".into()), Some("same".into()));
        assert_eq!(m3.candidates(), vec!["same".to_string()], "重复凭证去重");
    }
}
