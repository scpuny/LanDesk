//! mDNS 广播接线（FR-01.1）：把本机 [`Config`] 翻译成 [`DeviceAd`] 并启动 [`Advertiser`]。
//!
//! `mdns` crate 里的 `Advertiser` 早已实现但从未接入任何应用入口——控制端 GUI /
//! `ui-server` 只启动了 `Listener`（只听不喊），结果局域网内没有任何一端在宣告
//! 自己，两台 GUI 互相发现不了。本模块提供统一接线点，供 GUI 形态共用：
//! - Tauri GUI（`ui/src-tauri`）：按本机配置广播，让另一台 GUI 能发现；
//! - `ui-server`（旁路 Web）：同上。
//!
//! 广播器 [`Advertiser`] 必须**保活**到进程结束（其 `Drop` 会发 mDNS goodbye），
//! 调用方把它存进长生命周期状态或局部变量即可。
//!
//! 注意：GUI 广播用于「本机在线可见」（设备列表能看到对方）；真正的受控连接
//! 由受控端 CLI（`controlled`）提供 QUIC 服务端，其广播真实监听端口属后续接线。

use landesk_core::config::Config;
use mdns::{Advertiser, DeviceAd, MdnsError, DEFAULT_PORT};

use crate::device_code_from;

/// 缺省设备名（配置为空时的兜底，正常由 `ConfigStore::load_or_init` 回填 hostname）。
const FALLBACK_NAME: &str = "landesk";

/// 由配置构造本机的广播设备描述（TXT 记录内容，对齐 protocol.md §4）。
///
/// - `id` 取设备名派生的稳定 8 位 hex 设备码（与控制端快速连接码一致）；
/// - `enc`/`res` 保守填通用值（GUI 侧未接编码能力探测，不谎报硬件特性）；
/// - `relay` 跟配置开关。
pub fn device_ad_of(config: &Config) -> DeviceAd {
    let name = if config.device_name.trim().is_empty() {
        FALLBACK_NAME.to_string()
    } else {
        config.device_name.clone()
    };
    DeviceAd {
        id: device_code_from(&name),
        name,
        enc: "openh264".to_string(),
        res: "1920x1080".to_string(),
        ver: 1,
        relay: config.relay_enabled,
    }
}

/// 按配置启动 mDNS 广播器（服务 `_landesk._tcp.local.`，QUIC 端口默认 52000）。
///
/// 调用方负责保活返回的 [`Advertiser`]；一旦 drop 即发 goodbye 下线。
pub fn start_advertiser(config: &Config, port: u16) -> Result<Advertiser, MdnsError> {
    let ad = device_ad_of(config);
    Advertiser::start(&ad, port, "")
}

/// 按配置启动广播器，端口取协议默认 [`DEFAULT_PORT`]（GUI / ui-server 用）。
pub fn start_advertiser_default(config: &Config) -> Result<Advertiser, MdnsError> {
    start_advertiser(config, DEFAULT_PORT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_ad_uses_stable_code_and_config() {
        let mut cfg = Config::default();
        cfg.device_name = "devboard".into();
        cfg.relay_enabled = true;
        let ad = device_ad_of(&cfg);
        assert_eq!(ad.name, "devboard");
        assert_eq!(ad.id, device_code_from("devboard"));
        assert_eq!(ad.id.len(), 8);
        assert!(ad.relay);
    }

    #[test]
    fn empty_name_falls_back() {
        let cfg = Config::default();
        let ad = device_ad_of(&cfg);
        assert_eq!(ad.name, FALLBACK_NAME);
        assert_eq!(ad.id, device_code_from(FALLBACK_NAME));
    }
}
