//! `mdns` — 局域网发现（FR-01，`_landesk._tcp`，对齐 `docs/protocol.md` §4）。
//!
//! M4 接入 mdns-sd：
//! - [`Advertiser`]：受控端广播自己的 [`DeviceAd`]（TXT 记录）；
//! - [`Listener`]：控制端浏览局域网、收集在线设备、按 TTL 剪枝；
//! - [`txt`]：`DeviceAd` ↔ TXT 键值对编解码（纯逻辑，可单测）。
//!
//! 协议常量：服务类型 [`SERVICE_TYPE`]、完整域 [`SERVICE_DOMAIN`]、默认端口
//! [`DEFAULT_PORT`]、TTL [`TTL_SECS`]。

pub mod advertiser;
pub mod error;
pub mod listener;
pub mod txt;

pub use advertiser::Advertiser;
pub use error::MdnsError;
pub use listener::Listener;
pub use txt::{encode, decode, instance_name};

/// 服务类型（protocol.md §4，用于 mdns-sd 浏览/注册的 `ty_domain` 前缀）。
pub const SERVICE_TYPE: &str = "_landesk._tcp";
/// 完整 mDNS 域（mdns-sd 要求带 `.local.` 后缀的完整域）。
pub const SERVICE_DOMAIN: &str = "_landesk._tcp.local.";
/// 默认 QUIC 监听端口（protocol.md §4，可配置）。
pub const DEFAULT_PORT: u16 = 52000;
/// 实例 TTL（秒，对齐 FR-01.4 过期移除）。
///
/// 45s 偏紧：mDNS cache-refresh 在部分网络（跨网段 / 组播抑制 / 省电网卡）会被丢弃，
/// 一次刷新丢失即误判离线。放宽到 180s，配合广播端周期 reannounce 心跳，
/// 「刷新几次设备就消失」不再因单次丢包发生。
pub const TTL_SECS: u32 = 180;

/// 广播的设备信息（TXT 记录内容，protocol.md §4 表格）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceAd {
    /// 设备ID（12 位 hex，16B 的 12 位缩写）。
    pub id: String,
    /// 设备名（≤32B）。
    pub name: String,
    /// 编码器能力 csv：`qsv,nvenc,openh264`。
    pub enc: String,
    /// 当前主分辨率 `WxH`。
    pub res: String,
    /// 协议版本。
    pub ver: u8,
    /// 是否已注册自建中继。
    pub relay: bool,
}

impl DeviceAd {
    /// 实例名 = `设备名@id后6位`（避免重名冲突，对齐 protocol.md §4）。
    pub fn instance_name(&self) -> String {
        format!("{}@{}", self.name, self.id.get(..6).unwrap_or(&self.id))
    }
}

/// 在线设备（控制端监听结果）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OnlineDevice {
    pub ad: DeviceAd,
    pub ip: String,
    pub port: u16,
    /// 最后刷新时间戳（epoch 秒），用于 TTL 判定。
    pub last_seen: u64,
}

impl OnlineDevice {
    /// 是否已过期（超过 [`TTL_SECS`] 未刷新）。
    pub fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.last_seen) > TTL_SECS as u64
    }
}

/// 按 TTL 移除过期设备（FR-01.4）。
pub fn prune_stale(devices: &mut Vec<OnlineDevice>, now: u64) -> usize {
    let before = devices.len();
    devices.retain(|d| !d.is_stale(now));
    before - devices.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad() -> DeviceAd {
        DeviceAd {
            id: "8f3a1b2c9d0e".into(),
            name: "devboard".into(),
            enc: "qsv,openh264".into(),
            res: "1920x1080".into(),
            ver: 1,
            relay: false,
        }
    }

    #[test]
    fn instance_name_uses_id_suffix() {
        assert_eq!(ad().instance_name(), "devboard@8f3a1b");
    }

    #[test]
    fn ttl_prune() {
        let d = OnlineDevice {
            ad: ad(),
            ip: "10.0.0.5".into(),
            port: DEFAULT_PORT,
            last_seen: 100,
        };
        let mut vec = vec![d];
        // 基于 TTL_SECS 计算，避免常量调整后断言失效。
        let within = 100 + TTL_SECS as u64 - 1; // 间隔 < TTL，保留
        let beyond = 100 + TTL_SECS as u64 + 1; // 间隔 > TTL，移除
        assert_eq!(prune_stale(&mut vec, within), 0);
        assert_eq!(prune_stale(&mut vec, beyond), 1);
        assert!(vec.is_empty());
    }
}
