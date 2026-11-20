//! TXT 记录编解码（对齐 `docs/protocol.md` §4）。
//!
//! mDNS TXT 为 UTF-8 `k=v` 键值对。本模块提供 [`DeviceAd`] 与
//! `HashMap<String, String>`（mdns-sd 的 TXT 载体）之间的双向转换，
//! 以及必填键校验，纯逻辑、可单测（ADR-A1 精神：协议层可独立测试）。

use std::collections::HashMap;

use crate::error::MdnsError;
use crate::DeviceAd;

/// 必填 TXT 键（protocol.md §4 表格）。
pub const KEY_ID: &str = "id";
pub const KEY_NAME: &str = "name";
pub const KEY_ENC: &str = "enc";
pub const KEY_RES: &str = "res";
pub const KEY_VER: &str = "ver";
pub const KEY_RELAY: &str = "relay";

/// `DeviceAd` → TXT `HashMap`（广播侧，`DeviceAd` 的所有字段进 TXT）。
pub fn encode(ad: &DeviceAd) -> HashMap<String, String> {
    let mut m = HashMap::with_capacity(6);
    m.insert(KEY_ID.into(), ad.id.clone());
    m.insert(KEY_NAME.into(), ad.name.clone());
    m.insert(KEY_ENC.into(), ad.enc.clone());
    m.insert(KEY_RES.into(), ad.res.clone());
    m.insert(KEY_VER.into(), ad.ver.to_string());
    m.insert(KEY_RELAY.into(), if ad.relay { "1".into() } else { "0".into() });
    m
}

/// TXT 键值对 → [`DeviceAd`]（监听侧）。
///
/// 校验必填键缺失即报错；缺失项按 protocol.md §4 语义（必填键必须存在）。
/// 接受任意 `(&str, &str)` 迭代器：广播侧可传 `HashMap`，监听侧可传
/// `TxtProperties::iter()`（`key()`/`val_str()` 均返 `&str`）。
pub fn decode<'a>(
    txt: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<DeviceAd, MdnsError> {
    let m: HashMap<_, _> = txt.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();

    let req = |k: &str| -> Result<String, MdnsError> {
        m.get(k).cloned().ok_or(MdnsError::MissingTxtKey(k.into()))
    };

    let relay = m
        .get(KEY_RELAY)
        .map(|v| v == "1")
        .unwrap_or(false); // relay 非必填，缺省 false

    Ok(DeviceAd {
        id: req(KEY_ID)?,
        name: req(KEY_NAME)?,
        enc: req(KEY_ENC)?,
        res: req(KEY_RES)?,
        ver: req(KEY_VER)?.parse().unwrap_or(1),
        relay,
    })
}

/// 实例名 = `设备名@id后6位`（复用 `DeviceAd::instance_name`，mDNS 实例名规范）。
pub fn instance_name(ad: &DeviceAd) -> String {
    ad.instance_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceAd;

    fn ad() -> DeviceAd {
        DeviceAd {
            id: "8f3a1b2c9d0e".into(),
            name: "devboard".into(),
            enc: "qsv,openh264".into(),
            res: "1920x1080".into(),
            ver: 1,
            relay: true,
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let a = ad();
        let txt = encode(&a);
        let back = decode(txt.iter().map(|(k, v)| (k.as_str(), v.as_str()))).expect("decode");
        assert_eq!(back, a);
    }

    #[test]
    fn missing_required_key_errors() {
        let mut txt = encode(&ad());
        txt.remove("id");
        let iter = txt.iter().map(|(k, v)| (k.as_str(), v.as_str()));
        assert!(matches!(
            decode(iter).unwrap_err(),
            MdnsError::MissingTxtKey(k) if k == "id"
        ));
    }

    #[test]
    fn missing_relay_defaults_false() {
        let mut txt = encode(&ad());
        txt.remove("relay");
        let back = decode(txt.iter().map(|(k, v)| (k.as_str(), v.as_str()))).expect("decode without relay");
        assert!(!back.relay, "relay 缺省应为 false");
        assert_eq!(back.id, "8f3a1b2c9d0e");
    }

    #[test]
    fn relay_flag_encoding() {
        let mut a = ad();
        a.relay = true;
        let txt = encode(&a);
        assert_eq!(txt.get("relay").map(String::as_str), Some("1"));
        a.relay = false;
        let txt = encode(&a);
        assert_eq!(txt.get("relay").map(String::as_str), Some("0"));
    }
}
