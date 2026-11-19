//! mDNS 广播器（FR-01，受控端向局域网宣告自己）。
//!
//! 持 [`mdns_sd::ServiceDaemon`] + 已注册的 [`ServiceInfo`]，把 [`DeviceAd`]
//! 编码为 TXT 记录并周期重新宣告（mDNS 广播无持久会话，TTL 由守护自动维护，
//! 重连/换 IP 时 `register` 再宣告即可）。

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tracing::{debug, warn};

use crate::error::MdnsError;
use crate::txt;
use crate::{DeviceAd, SERVICE_DOMAIN};

/// 局域网广播器。
pub struct Advertiser {
    daemon: ServiceDaemon,
    /// 已注册的完整实例名（`my_name._landesk._tcp.local.`），供 `unregister` 用。
    fullname: String,
}

impl Advertiser {
    /// 新建广播器并注册本设备（`ip=""` 时由 `enable_addr_auto` 自动填全部网卡）。
    ///
    /// - `device`：要宣告的设备信息；
    /// - `port`：本端 QUIC 监听端口（通常 [`DEFAULT_PORT`]）；
    /// - `host`：主机名（`.local.` 后缀，可空串）；
    pub fn start(device: &DeviceAd, port: u16, host: &str) -> Result<Self, MdnsError> {
        let daemon = ServiceDaemon::new()?;

        let properties = txt::encode(device);
        let instance = device.instance_name();
        // `AsIpAddrs for ()` 允许空地址集 + addr_auto，由库自动填所有网卡 IP。
        let info = ServiceInfo::new(
            SERVICE_DOMAIN,
            &instance,
            host,
            (),
            port,
            Some(properties),
        )
        .map_err(MdnsError::from)?
        .enable_addr_auto();

        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(MdnsError::from)?;

        debug!(instance = %instance, port, "mdns 已注册 _landesk 服务");
        Ok(Self { daemon, fullname })
    }

    /// 重新宣告（IP 变化 / 设备信息更新时调用，无需先 unregister）。
    pub fn reannounce(&self, device: &DeviceAd, port: u16, host: &str) -> Result<(), MdnsError> {
        let properties = txt::encode(device);
        let instance = device.instance_name();
        let info = ServiceInfo::new(
            SERVICE_DOMAIN,
            &instance,
            host,
            (),
            port,
            Some(properties),
        )
        .map_err(MdnsError::from)?
        .enable_addr_auto();
        self.daemon.register(info).map_err(MdnsError::from)?;
        debug!("mdns 重新宣告服务");
        Ok(())
    }

    /// 注销服务（优雅下线，发 mDNS Goodbye）。
    pub fn stop(&self) {
        if let Ok(rx) = self.daemon.unregister(&self.fullname) {
            // 短暂等待注销完成（不阻塞主逻辑）。
            if let Ok(status) = rx.recv() {
                match status {
                    mdns_sd::UnregisterStatus::OK => debug!("mdns 注销完成"),
                    mdns_sd::UnregisterStatus::NotFound => warn!("mdns 注销：服务未找到（可能已注销）"),
                }
            }
        }
    }
}

impl Drop for Advertiser {
    fn drop(&mut self) {
        self.stop();
    }
}
