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

use std::collections::HashSet;
use std::net::IpAddr;

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

/// 从一组候选 IP 里挑一个「可作 QUIC 拨号目标」的地址（FR-01）。
///
/// `mdns-sd` 的 [`ServiceInfo::get_addresses`](mdns_sd::ServiceInfo::get_addresses) 返回
/// `&HashSet<IpAddr>`（**无序**，含全部网卡 v4+v6，可能把 `fe80::` 链路本地排在最前）。
/// 直接 `.iter().next()` 会随机取到链路本地/环回这类**无法当拨号目标**的地址
/// （`fe80::` 不可跨网段路由、且不带 scope-id 根本没法组 `SocketAddr`；`169.254::/16`
/// APIPA 是自举占位、多半不通）。
///
/// 本函数按「可拨号性」打分择优，保证局域网内「点到就连」：
/// - **0** 环回 / 未指定（`127.x`、`::1`、`0.0.0.0`、`::`）——永远不是目标；
/// - **1** 非环回 **IPv4**（`192.168.x`/`10.x`/`172.16-31.x` 等常规私网）——最优先；
/// - **2** 可拨号 **IPv6**（全局单播，排除 `fe80::` 链路本地、环回、未指定）——次优先；
/// - **3** 仅 APIPA（`169.254::/16`）兜底——最后才用（自举环境没有别的可用地址时）。
///
/// 全不可拨（或空集合）→ `None`（调用方落 `unwrap_or_default()`，保持旧语义不崩）。
///
/// 注：`Ipv6Addr::is_link_local` 在 stable 尚不可用，这里按 `fe80::/10` 前缀手写判定。
fn is_v6_link_local(a: &std::net::Ipv6Addr) -> bool {
    // fe80::/10：前 10 位固定为 1111 1110 1x → 掩码 0xffc0、比较值 0xfe80。
    (a.segments()[0] & 0xffc0) == 0xfe80
}

pub fn pick_dialable_ip(candidates: &HashSet<IpAddr>) -> Option<IpAddr> {
    // 打分（分数越小越优先）：
    //   0 = 环回 / 未指定（127.x、::1、0.0.0.0、::）—— 永不作拨号目标，直接过滤；
    //   1 = 非环回 IPv4（10/172.16/192.168 私网，或其它可拨单播）—— 最优先；
    //   2 = 可拨 IPv6（全局单播 / ULA）—— 次优先；
    //   3 = 仅 APIPA（169.254/16，自举占位）兜底；
    //   4 = fe80:: 链路本地（不可跨段路由、无 scope 不可拨）—— 最差。
    fn score(a: &IpAddr) -> u8 {
        match a {
            IpAddr::V4(v4) => {
                if v4.is_loopback() || v4.is_unspecified() {
                    0
                } else if v4.octets()[0] == 169 && v4.octets()[1] == 254 {
                    3 // APIPA
                } else {
                    1 // 常规私网 / 其它可拨 IPv4
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() || v6.is_unspecified() {
                    0
                } else if is_v6_link_local(v6) {
                    4 // fe80:: —— 不可拨
                } else {
                    2 // 全局单播 / ULA
                }
            }
        }
    }

    candidates
        .iter()
        .filter(|a| score(a) != 0)
        .min_by_key(|a| score(a))
        .copied()
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

    fn addrs(v4: &[&str], v6: &[&str]) -> HashSet<IpAddr> {
        let mut s = HashSet::new();
        for a in v4.iter().chain(v6.iter()) {
            s.insert(a.parse().unwrap());
        }
        s
    }

    #[test]
    fn pick_prefers_ipv4_over_ipv6() {
        // 常规私网 IPv4 优先于全局 IPv6。
        let c = addrs(&["192.168.5.249"], &["240e:461:2c9e::1"]);
        assert_eq!(pick_dialable_ip(&c).map(|a| a.to_string()), Some("192.168.5.249".into()));
    }

    #[test]
    fn pick_only_loopback_is_none() {
        // 只有环回 / 未指定（无 fe80、无 v4、无全局 v6）→ 全被过滤 → None。
        let c = addrs(&["127.0.0.1", "0.0.0.0"], &["::1", "::"]);
        assert_eq!(pick_dialable_ip(&c), None);
    }

    #[test]
    fn pick_falls_back_to_link_local_when_only_option() {
        // 只有 fe80 链路本地可用（无 v4 无全局 v6）→ 仍返回它作最后兜底（比"没有地址"
        // 强；能否真连上由网络层判定，至少不当 parse 错误）。
        let c = addrs(&[], &["fe80::5b1a:274c:cd6b:fe29"]);
        assert_eq!(
            pick_dialable_ip(&c).map(|a| a.to_string()),
            Some("fe80::5b1a:274c:cd6b:fe29".into())
        );
    }

    #[test]
    fn pick_prefers_ipv4_over_link_local() {
        // 真实场景复现：同时有 192.168.x 与 fe80:: → 必须选中 IPv4，不选链路本地。
        let c = addrs(&["192.168.5.249"], &["fe80::5b1a:274c:cd6b:fe29"]);
        assert_eq!(pick_dialable_ip(&c).map(|a| a.to_string()), Some("192.168.5.249".into()));
    }

    #[test]
    fn pick_global_ipv6_when_no_ipv4() {
        // 无 IPv4 但可拨全局单播 IPv6 + 环回 → 选全局 v6（排除环回）。
        let c = addrs(&["127.0.0.1"], &["240e:461:2c9e::1", "::1"]);
        assert_eq!(pick_dialable_ip(&c).map(|a| a.to_string()), Some("240e:461:2c9e::1".into()));
    }

    #[test]
    fn pick_apipa_last_resort() {
        // 只有 APIPA（169.254）→ 返回它（自举占位兜底）。
        let c = addrs(&["169.254.10.20"], &["fe80::abcd"]);
        assert_eq!(pick_dialable_ip(&c).map(|a| a.to_string()), Some("169.254.10.20".into()));
    }

    #[test]
    fn pick_empty_set_is_none() {
        let c: HashSet<IpAddr> = HashSet::new();
        assert_eq!(pick_dialable_ip(&c), None);
    }
}
