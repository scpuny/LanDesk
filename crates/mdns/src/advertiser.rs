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
    /// 注册参数快照，供周期 `heartbeat()` 重新宣告（对端据此刷新 last_seen）。
    device: DeviceAd,
    port: u16,
    host: String,
}

/// 生成合法的 mDNS hostname（SRV 记录 target，必须非空且符合 DNS label 规则）。
///
/// 传空 host 会产生空 SRV target：对端 browse 只能收到 PTR（`ServiceAdd`），
/// 无法继续解析 SRV/TXT/A → `ServiceResolved` 事件不触发 → 在线设备列表永远 0 台。
/// v0.3.0 正是因 `Advertiser::start(..., "")` 踩此坑：系统 `dns-sd` 能看到 Add
/// 但 `dns-sd -L` 解析失败。
///
/// 兜底策略：`landesk-<device.id>.local.`——`id` 是 hex 串，天然满足 DNS label
/// 规则且跨机唯一；A/AAAA 记录由 `enable_addr_auto` 自动填本机全部网卡 IP，
/// 因此无需真实机器名（也规避了中文/空格机器名造成非法 label 的风险）。
pub(crate) fn effective_host(device: &DeviceAd, host: &str) -> String {
    let h = host.trim();
    if h.is_empty() {
        format!("landesk-{}.local.", device.id)
    } else if h.ends_with(".local.") {
        h.to_string()
    } else {
        format!("{}.local.", h.trim_end_matches('.'))
    }
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
        // host 必须非空：空 SRV target 会让对端只收到 PTR Add 而无法 resolve。
        let host = effective_host(device, host);
        // `AsIpAddrs for ()` 允许空地址集 + addr_auto，由库自动填所有网卡 IP。
        let info = ServiceInfo::new(SERVICE_DOMAIN, &instance, &host, (), port, Some(properties))
            .map_err(MdnsError::from)?
            .enable_addr_auto();

        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(MdnsError::from)?;

        debug!(instance = %instance, %host, port, "mdns 已注册 _landesk 服务");
        Ok(Self {
            daemon,
            fullname,
            device: device.clone(),
            port,
            host,
        })
    }

    /// 周期心跳：重新宣告本服务，强制对端 listener 收到 `ServiceResolved` 刷新 last_seen。
    ///
    /// mdns-sd 自身有 cache-refresh，但在组播抑制 / 跨网段 / 省电网卡环境可能丢刷新包；
    /// 由广播端主动重播兜底，避免对端 TTL 过期后把本机从列表移除（"刷新几次设备消失"）。
    pub fn heartbeat(&self) -> Result<(), MdnsError> {
        self.reannounce(&self.device, self.port, &self.host)
    }

    /// 重新宣告（IP 变化 / 设备信息更新时调用，无需先 unregister）。
    pub fn reannounce(&self, device: &DeviceAd, port: u16, host: &str) -> Result<(), MdnsError> {
        let properties = txt::encode(device);
        let instance = device.instance_name();
        let host = effective_host(device, host);
        let info = ServiceInfo::new(SERVICE_DOMAIN, &instance, &host, (), port, Some(properties))
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
                    mdns_sd::UnregisterStatus::NotFound => {
                        warn!("mdns 注销：服务未找到（可能已注销）")
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ad() -> DeviceAd {
        DeviceAd {
            id: "8f3a1b2c".into(),
            name: "devboard".into(),
            enc: "openh264".into(),
            res: "1920x1080".into(),
            ver: 1,
            relay: false,
        }
    }

    /// 空 host（v0.3.0 的 bug）必须兜底成合法非空 hostname，否则 SRV target 为空，
    /// 对端 `ServiceResolved` 不触发 → 在线设备列表永远 0 台。
    #[test]
    fn empty_host_gets_valid_fallback() {
        let host = effective_host(&ad(), "");
        assert_eq!(host, "landesk-8f3a1b2c.local.");
        assert!(!host.is_empty());
        assert!(host.ends_with(".local."));
        // 每个 label 必须是合法 DNS 字符（hex + 连字符）
        assert!(host
            .trim_end_matches('.')
            .split('.')
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')));
    }

    #[test]
    fn explicit_host_is_normalized() {
        assert_eq!(effective_host(&ad(), "myhost"), "myhost.local.");
        assert_eq!(effective_host(&ad(), "myhost.local."), "myhost.local.");
        assert_eq!(effective_host(&ad(), " myhost. "), "myhost.local.");
    }
}
