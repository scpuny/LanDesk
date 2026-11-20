//! 本机信息视图（P2.5，矩阵 §7.5「本机信息 API」：喂设备页左栏「本机卡」）。
//!
//! 前端 `LocalInfo` 是**本机的**设备码 / 临时密码 / 能力 / 在线态，取值分两类：
//! - [`Config`] 能直接给的（`device_name` / `mdns_enabled` / `signaling_url` / `relay_enabled`）
//!   由建造器自动填；
//! - 宿主态（`device_code` / `temp_password` / `enc` 能力 / `online`）由宿主传入
//!   （本 crate 不探测系统，保持纯逻辑可离线单测）。
//!
//! 序列化按 `camelCase` 对齐前端 `types/index.ts` 的 `LocalInfo`。

use landesk_core::config::Config;
use serde::Serialize;

/// 本机信息（对齐前端 `LocalInfo`，设备页左栏「本机卡」渲染）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalInfo {
    /// 设备名（← `Config.device_name`）。
    pub name: String,
    /// 本机设备码（8 位大写 hex，[`device_code_from`] 派生；供复制/发起连接）。
    pub device_code: String,
    /// 临时密码（6 位数字，[`gen_temp_password`] 生成；支持一次性密码连接）。
    pub temp_password: String,
    /// mDNS 发现是否启用（← `Config.mdns_enabled`）。
    pub mdns: bool,
    /// 信令服务地址（← `Config.signaling_url`，未配置为空串）。
    pub signaling: String,
    /// 中继是否可用（← `Config.relay_enabled`）。
    pub relay: bool,
    /// 编码能力标签列表（宿主探测结果；本批未接数据面探测，宿主可传保守值）。
    pub enc: Vec<String>,
    /// 本机当前是否在线（可被连接；宿主态）。
    pub online: bool,
}

/// 由 [`Config`] + 宿主态字段建造 [`LocalInfo`]。
///
/// - `cfg`：受控端配置（提供 `device_name` / `mdns_enabled` / `signaling_url` / `relay_enabled`）；
/// - `device_code`：本机设备码（宿主管，通常 [`device_code_from`] 派生）；
/// - `temp_password`：当前临时密码（宿主管，通常 [`gen_temp_password`]）；
/// - `enc`：编码能力标签（宿主探测，未接数据面时传空或保守值）；
/// - `mdns_advertising`：本端是否正在广播 mDNS（宿主态，与配置开关取与）；
/// - `online`：本机是否在线可连（宿主态）。
pub fn local_info_of(
    cfg: &Config,
    device_code: &str,
    temp_password: &str,
    enc: Vec<String>,
    mdns_advertising: bool,
    online: bool,
) -> LocalInfo {
    LocalInfo {
        name: cfg.device_name.clone(),
        device_code: device_code.to_string(),
        temp_password: temp_password.to_string(),
        mdns: cfg.mdns_enabled && mdns_advertising,
        signaling: cfg.signaling_url.clone().unwrap_or_default(),
        relay: cfg.relay_enabled,
        enc,
        online,
    }
}

/// 生成一个 6 位数字临时密码（`100000..=999999`）。
///
/// 用系统时间纳秒 + 进程号混合取模——非加密随机（本 crate 不引 `rand`，守离线门），
/// 但作为**会话一次性连接口令**足够：真正的密钥安全由 ECDH+PBKDF2 证明派生保证
/// （`crypto::derive_password`），临时密码只是共享口令输入。
pub fn gen_temp_password() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    // 黄金比例常量混合，降低相邻两次取到同值的概率。
    let n = ((nanos ^ pid.wrapping_mul(0x9E37_79B9_7F4A_7C15)) % 900_000) + 100_000;
    n.to_string()
}

/// 由设备名（或主机名）派生**稳定的** 8 位大写 hex 设备码。
///
/// FNV-1a 64 取低 32 位（跨进程/跨重启稳定，区别于 `std` `DefaultHasher` 的随机化种子）。
/// 受控端尚未内置设备身份（`apps/controlled` 为 M1 骨架），此处以「设备名/主机名 → 稳定摘要」
/// 给出可复制、可发起连接的本机设备码，而非伪造网卡 MAC。
pub fn device_code_from(name: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08X}", h & 0xFFFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_info_fills_from_config_and_host() {
        let cfg = Config {
            device_name: "我的工作机".into(),
            signaling_url: Some("ws://lan:8443".into()),
            mdns_enabled: true,
            relay_enabled: true,
            ..Default::default()
        };
        let info = local_info_of(
            &cfg,
            "83FA217C",
            "629418",
            vec!["QSV".into(), "VAAPI".into()],
            true,
            true,
        );
        assert_eq!(info.name, "我的工作机");
        assert_eq!(info.device_code, "83FA217C");
        assert_eq!(info.temp_password, "629418");
        assert!(info.mdns, "配置开 + 广播中 → mdns 真");
        assert_eq!(info.signaling, "ws://lan:8443");
        assert!(info.relay);
        assert_eq!(info.enc, vec!["QSV".to_string(), "VAAPI".to_string()]);
        assert!(info.online);
    }

    #[test]
    fn local_info_signaling_empty_when_unset() {
        let cfg = Config::default(); // signaling_url = None
        let info = local_info_of(&cfg, "ABCD1234", "123456", vec![], false, true);
        assert_eq!(info.signaling, "", "未配置信令 → 空串");
        assert_eq!(info.relay, cfg.relay_enabled, "relay 跟配置");
    }

    #[test]
    fn local_info_mdns_and_advertise_gate() {
        let cfg = Config {
            mdns_enabled: true,
            ..Default::default()
        };
        // 配置开但未广播 → mdns 假
        let off = local_info_of(&cfg, "A", "1", vec![], false, true);
        assert!(!off.mdns);
        // 配置开且广播中 → mdns 真
        let on = local_info_of(&cfg, "A", "1", vec![], true, true);
        assert!(on.mdns);
    }

    #[test]
    fn temp_password_is_six_digits() {
        let p = gen_temp_password();
        assert_eq!(p.len(), 6, "6 位: {p}");
        assert!(p.chars().all(|c| c.is_ascii_digit()), "全数字: {p}");
        let n: u32 = p.parse().unwrap();
        assert!((100_000..=999_999).contains(&n), "落在 6 位区间: {n}");
    }

    #[test]
    fn device_code_is_stable_eight_upper_hex() {
        let a = device_code_from("ok3568");
        let b = device_code_from("ok3568");
        assert_eq!(a, b, "同输入同输出（稳定）");
        assert_eq!(a.len(), 8, "8 位: {a}");
        assert!(
            a.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)),
            "大写 hex（数字 + A-F）: {a}"
        );
        assert_ne!(device_code_from("ok3568"), device_code_from("devboard"), "不同输入不同码");
    }

    #[test]
    fn local_info_serializes_camel_case_for_frontend() {
        let cfg = Config::default();
        let info = local_info_of(&cfg, "83FA217C", "629418", vec!["QSV".into()], true, true);
        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["deviceCode"], "83FA217C", "camelCase 键");
        assert_eq!(json["tempPassword"], "629418");
        assert_eq!(json["mdns"], true);
        assert_eq!(json["signaling"], "", "空信令序列化空串");
        assert_eq!(json["enc"], serde_json::json!(["QSV"]));
    }
}
