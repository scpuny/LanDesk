//! mDNS 监听器（FR-01，控制端发现局域网受控端）。
//!
//! 持 [`mdns_sd::ServiceDaemon`] + 浏览事件流，把 `ServiceResolved` 事件
//! 解码为 [`OnlineDevice`]，维护在线设备表并按 TTL 剪枝（FR-01.4）。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tracing::{debug, warn};

use crate::error::MdnsError;
use crate::txt;
use crate::{OnlineDevice, SERVICE_DOMAIN};

/// 当前 unix 时间（秒）。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 局域网监听器。
///
/// `poll_next()` 非阻塞地取走当前已到达的事件并刷新在线表；`discover_within`
/// 在时间窗内轮询收集设备（FR-01「一键发现」）。TTL 剪枝由 [`prune`](Self::prune)
/// 显式驱动（FR-01.4：45s 未刷新即移除）。
pub struct Listener {
    daemon: ServiceDaemon,
    events: mdns_sd::Receiver<ServiceEvent>,
    /// 在线设备表（按实例名去重，刷新 `last_seen`）。
    devices: Vec<OnlineDevice>,
}

impl Listener {
    /// 新建监听器并发起对 [`SERVICE_DOMAIN`] 的浏览。
    pub fn start() -> Result<Self, MdnsError> {
        let daemon = ServiceDaemon::new()?;
        let events = daemon.browse(SERVICE_DOMAIN)?;
        debug!("mdns 开始浏览 {SERVICE_DOMAIN}");
        Ok(Self {
            daemon,
            events,
            devices: Vec::new(),
        })
    }

    /// 非阻塞取走当前已到达的一个事件（若没有则返回 `None`），刷新在线表。
    ///
    /// 驱动语义：
    /// - `Some(ServiceResolved)`：解码 TXT → 设备，按实例名 upsert + 刷新 `last_seen`；
    /// - `Some(ServiceRemoved)`：从在线表移除；
    /// - `None`：暂无新事件（调用方可 sleep 后重试）。
    pub fn poll_next(&mut self) -> Option<ServiceEvent> {
        let ev = self.events.try_recv().ok()?;
        match &ev {
            ServiceEvent::ServiceResolved(info) => self.upsert_resolved(info),
            ServiceEvent::ServiceRemoved(_ty, fullname) => self.remove(fullname),
            _ => {}
        }
        Some(ev)
    }

    /// 在 `window` 内轮询收集设备（FR-01 一键发现）。每 `tick` 取一次事件，
    /// 到窗口结束剪枝并返回当前在线表快照。
    pub async fn discover_within(&mut self, window: Duration) -> Result<Vec<OnlineDevice>, MdnsError> {
        let tick = Duration::from_millis(50);
        let deadline = SystemTime::now() + window;
        loop {
            self.poll_next();
            if SystemTime::now() >= deadline {
                break;
            }
            tokio::time::sleep(tick).await;
        }
        self.prune();
        Ok(self.snapshot())
    }

    fn upsert_resolved(&mut self, info: &mdns_sd::ServiceInfo) {
        let txt_iter = info
            .get_properties()
            .iter()
            .map(|p| (p.key(), p.val_str()));
        let ad = match txt::decode(txt_iter) {
            Ok(ad) => ad,
            Err(e) => {
                warn!(?e, instance = info.get_fullname(), "TXT 解码失败，忽略该设备");
                return;
            }
        };
        let ip = info
            .get_addresses()
            .iter()
            .next()
            .map(|ip| ip.to_string())
            .unwrap_or_default();
        let port = info.get_port();
        let now = now_secs();

        if let Some(dev) = self
            .devices
            .iter_mut()
            .find(|d| d.ad.instance_name() == ad.instance_name())
        {
            dev.ad = ad;
            dev.ip = ip;
            dev.port = port;
            dev.last_seen = now;
        } else {
            debug!(instance = %ad.instance_name(), %ip, port, "mdns 发现新设备");
            self.devices.push(OnlineDevice {
                ad,
                ip,
                port,
                last_seen: now,
            });
        }
    }

    fn remove(&mut self, fullname: &str) {
        // fullname = "<instance>.<ty_domain>"，实例名 = 去掉 ty_domain 后缀。
        let instance = fullname
            .strip_suffix(SERVICE_DOMAIN)
            .unwrap_or(fullname)
            .to_string();
        let before = self.devices.len();
        self.devices.retain(|d| d.ad.instance_name() != instance);
        if self.devices.len() != before {
            debug!(instance = %instance, "mdns 设备下线");
        }
    }

    /// 按 TTL 剪枝（FR-01.4：超过 [`crate::TTL_SECS`] 未刷新即移除），返回移除数量。
    pub fn prune(&mut self) -> usize {
        crate::prune_stale(&mut self.devices, now_secs())
    }

    /// 当前在线设备快照（克隆）。
    pub fn snapshot(&self) -> Vec<OnlineDevice> {
        self.devices.clone()
    }

    /// 停止浏览 + 关闭守护。
    pub fn stop(&self) {
        let _ = self.daemon.stop_browse(SERVICE_DOMAIN);
        let _ = self.daemon.shutdown();
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop();
    }
}
