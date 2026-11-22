//! M4 环回集成测试：本机 mDNS 广播/监听自检（FR-01）。
//!
//! 依赖真实组播（`224.0.0.251:5353`），CI 容器常不支持组播 → 用 `#[ignore]`
//! 默认不跑，避免 flake。本机验证：
//! ```sh
//! cargo test -p mdns -- --ignored
//! ```

use std::time::Duration;

use mdns::{Advertiser, DeviceAd, Listener};

fn test_ad() -> DeviceAd {
    DeviceAd {
        id: "0a1b2c3d4e5f".into(),
        name: "landesk-it".into(),
        enc: "openh264".into(),
        res: "1280x720".into(),
        ver: 1,
        relay: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "mDNS 组播需真实网卡；CI 容器常不支持。本机: cargo test -p mdns -- --ignored"]
async fn loopback_discovery() {
    let ad = test_ad();
    let expected = ad.instance_name();

    // 受控端广播（addr_auto 自动填本机关卡）。
    // 传空 host 走兜底路径（v0.3.0 bug 回归：空 SRV target 导致对端无法 resolve）。
    let advertiser = Advertiser::start(&ad, 52000, "").expect("advertiser");
    // 等注册 + 组播包生效。
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 控制端发现。
    let mut listener = Listener::start().expect("listener");
    let found = listener
        .discover_within(Duration::from_secs(3))
        .await
        .expect("discover");

    advertiser.stop();
    listener.stop();

    assert!(
        found.iter().any(|d| d.ad.instance_name() == expected),
        "3s 内未发现本机广播 {expected}，实际发现 = {found:?}"
    );
}
