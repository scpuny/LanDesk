//! 设备列表模型（FR-01.2 / FR-01.4，对齐 `docs/architecture.md` §3 `mdns` + `ui/frontend/views/DeviceList.vue`）。
//!
//! 控制端 mDNS 监听得到 [`mdns::OnlineDevice`] 列表，本模型把它归一成**可序列化的
//! 设备行**（喂 `DeviceList.vue`），并做 TTL 剪枝 + 排序（FR-01.4「离线自动移除」）。
//! 纯逻辑可离线单测。

use mdns::{prune_stale, OnlineDevice};
use serde::Serialize;

/// 当前 unix 时间（秒），设备表 TTL 剪枝用（宿主驱动 / 出参时传 `now`）。
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 设备列表行（序列化喂前端）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceRow {
    /// 设备ID（12 位 hex）。
    pub id: String,
    /// 设备名。
    pub name: String,
    /// 实例名（`name@id后6位`）。
    pub instance: String,
    /// QUIC 端点 `ip:port`（拨号目标）。
    pub addr: String,
    /// 主分辨率。
    pub res: String,
    /// 编码器能力 csv（`qsv,openh264`）。
    pub enc: String,
    /// 是否已注册自建中继。
    pub relay: bool,
    /// 最后刷新（epoch 秒）。
    pub last_seen: u64,
}

/// 从在线设备列表构建排序后的行视图（FR-01.4：先 TTL 剪枝，再按名排序）。
///
/// 入参为 `&mut Vec`（剪枝需改长度）；剪枝后就地保留有效设备，再排序返回行视图。
pub fn device_rows(devices: &mut Vec<OnlineDevice>, now: u64) -> Vec<DeviceRow> {
    let _removed = prune_stale(devices, now);
    devices.sort_by(|a, b| a.ad.name.cmp(&b.ad.name).then(a.ad.id.cmp(&b.ad.id)));
    devices
        .iter()
        .map(|d| DeviceRow {
            id: d.ad.id.clone(),
            name: d.ad.name.clone(),
            instance: d.ad.instance_name(),
            addr: format!("{}:{}", d.ip, d.port),
            res: d.ad.res.clone(),
            enc: d.ad.enc.clone(),
            relay: d.ad.relay,
            last_seen: d.last_seen,
        })
        .collect()
}

/// 监听器无关的设备在线表（P1.1「设备列表下沉」）：纯 `Vec<OnlineDevice>` + TTL 剪枝 + 行视图。
///
/// 与 `mdns::Listener` 解耦（`Send + Sync`，可 `Arc<Mutex<DeviceTable>>` 共享进宿主后台驱动），
/// 故**可离线单测**；宿主（ui-server / Tauri 壳）各自持 `Listener`，驱动后用
/// [`apply_snapshot`] 把监听器内部在线表同步进来，再经 [`rows`] 出剪枝+排序行。
/// 这样「设备列表」逻辑归一到内核，两个宿主共用（对齐 `docs/ui-backend-matrix.md` §5.3 / §7.3）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceTable {
    devices: Vec<OnlineDevice>,
}

impl DeviceTable {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 用监听器在线表快照**整体替换**本表（监听器已做去重 / upsert / 刷新 `last_seen`，
    /// 宿主每轮驱动后同步一次即可，是「镜像」语义）。
    pub fn apply_snapshot(&mut self, snapshot: Vec<OnlineDevice>) {
        self.devices = snapshot;
    }

    /// 出当前在线行（FR-01.4：先 TTL 剪枝、再按名排序）。入参 `now` 为当前 epoch 秒。
    pub fn rows(&mut self, now: u64) -> Vec<DeviceRow> {
        device_rows(&mut self.devices, now)
    }

    /// 当前设备数（剪枝前，供设备计数 / 角标）。
    pub fn len(&self) -> usize {
        self.devices.len()
    }

    /// 是否无设备。
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns::DeviceAd;

    fn ad(id: &str, name: &str, relay: bool) -> DeviceAd {
        DeviceAd {
            id: id.into(),
            name: name.into(),
            enc: "qsv,openh264".into(),
            res: "1920x1080".into(),
            ver: 1,
            relay,
        }
    }

    fn dev(id: &str, name: &str, ip: &str, last_seen: u64, relay: bool) -> OnlineDevice {
        OnlineDevice {
            ad: ad(id, name, relay),
            ip: ip.into(),
            port: 52000,
            last_seen,
        }
    }

    #[test]
    fn rows_sorted_by_name_and_addr() {
        let mut ds = vec![
            dev("b", "zzz-box", "10.0.0.2", 100, false),
            dev("a", "aaa-mac", "10.0.0.1", 100, true),
        ];
        let rows = device_rows(&mut ds, 100);
        assert_eq!(rows[0].name, "aaa-mac");
        assert_eq!(rows[1].name, "zzz-box");
        assert_eq!(rows[0].addr, "10.0.0.1:52000");
        assert!(rows[0].relay, "中继标志随设备带出");
    }

    #[test]
    fn stale_devices_pruned() {
        let mut ds = vec![
            dev("fresh", "fresh", "10.0.0.1", 90, false),
            dev("old", "old", "10.0.0.2", 1, false),
        ];
        // now=100：old 的 last_seen=1 → 已过期 45s+，剪枝。
        let rows = device_rows(&mut ds, 100);
        assert_eq!(rows.len(), 1, "过期设备被移除（FR-01.4）");
        assert_eq!(rows[0].name, "fresh");
    }

    #[test]
    fn row_is_serializable() {
        let mut ds = vec![dev("a", "mac", "10.0.0.9", 50, false)];
        let rows = device_rows(&mut ds, 50);
        let v = serde_json::to_value(&rows).expect("serialize");
        assert_eq!(v[0]["name"], "mac");
        assert_eq!(v[0]["addr"], "10.0.0.9:52000");
    }

    #[test]
    fn device_table_mirrors_snapshot_prunes_and_sorts() {
        // P1.1：宿主把监听器在线表快照镜像进来，再经 rows 出剪枝+排序行。
        let mut t = DeviceTable::new();
        assert!(t.is_empty());
        t.apply_snapshot(vec![
            dev("b", "zzz", "10.0.0.2", 100, false),
            dev("a", "aaa", "10.0.0.1", 100, true),
            dev("old", "old", "10.0.0.3", 1, false), // now=100 时已过期
        ]);
        assert_eq!(t.len(), 3, "快照前 3 台");
        let rows = t.rows(100);
        assert_eq!(rows.len(), 2, "过期设备被 TTL 剪枝（FR-01.4）");
        assert_eq!(rows[0].name, "aaa", "按名排序（aaa 在 zzz 前）");
        assert!(rows[0].relay);
        // 二次 apply_snapshot 整体替换语义。
        t.apply_snapshot(vec![dev("c", "ccc", "10.0.0.7", 100, false)]);
        assert_eq!(t.len(), 1, "快照整体替换（镜像）");
        assert_eq!(t.rows(100)[0].name, "ccc");
    }
}
