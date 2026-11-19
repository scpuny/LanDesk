//! M5「双向剪贴板 + 注入全链路」环回集成测试（对齐 `docs/architecture.md` §3 landesk-core、
//! `protocol.md` §1.3 剪贴板分段 / §1.4 键码、`lp` FR-06/FR-07）。
//!
//! 在 ECDH 升级后的 [`OneClickSession`] 控制流上验证：
//! 1. 剪贴板 C→S：[`ClipboardFlow::send_text`] 发 `ClipStart → ClipChunk* → ClipEnd`，
//!    受控端 `receive_clipboard` 重组 + SHA-256 校验还原文本；
//! 2. 键盘 S→C：`KeyDown` / `KeyUp` 帧在控制流上往返（锁定 `Key` 拆分为
//!    `KeyDown`/`KeyUp` 的 wire 修正，见 `lp::msg`）。
//!
//! 并发模型与 M4 环回一致：双端点各带 UDP drive，`multi_thread` + `try_join!`。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use landesk_core::{
    ClipboardFlow, CoreError, Controlled, Controller, OneClickSession, SessionState,
};
use lp::keycodes::Lkc;
use lp::msg::{ControlMsg, KeyEvent};
use mdns::{DeviceAd, OnlineDevice};
use quic_link::QuicClient;

#[tokio::test(flavor = "multi_thread")]
async fn m5_clipboard_c2s() {
    let (mut c_session, mut s_session) = common::establish_session().await;

    assert_eq!(c_session.state(), SessionState::Running);
    assert_eq!(s_session.state(), SessionState::Running);

    let mut c_flow = ClipboardFlow::new();
    let mut s_flow = ClipboardFlow::new();
    let text = "hello landesk m5 剪贴板 📋";

    let (_, got) = tokio::try_join!(
        c_flow.send_text(&mut c_session, text),
        landesk_core::receive_clipboard(&mut s_flow, &mut s_session),
    )
    .expect("clipboard c2s");
    assert_eq!(got, text, "受控端重组 + SHA 校验后还原文本一致");

    // 双向：再发一段（验证 flow 状态机可复用）
    let (_, got2) = tokio::try_join!(
        c_flow.send_text(&mut c_session, "second payload"),
        landesk_core::receive_clipboard(&mut s_flow, &mut s_session),
    )
    .expect("clipboard c2s again");
    assert_eq!(got2, "second payload");

    c_session.close(0, b"done");
    s_session.close(0, b"done");
}

#[tokio::test(flavor = "multi_thread")]
async fn m5_key_roundtrip_s2c() {
    let (mut c_session, mut s_session) = common::establish_session().await;

    let key = Lkc::A.as_u32();
    let (_, (kd, ku)) = tokio::try_join!(
        async {
            s_session
                .send_control(&ControlMsg::KeyDown(KeyEvent {
                    key,
                    mods: 0,
                }))
                .await?;
            s_session
                .send_control(&ControlMsg::KeyUp(KeyEvent {
                    key,
                    mods: 0,
                }))
                .await?;
            Ok::<_, CoreError>(())
        },
        async {
            let kd = c_session.recv_control().await?;
            let ku = c_session.recv_control().await?;
            Ok::<_, CoreError>((kd, ku))
        },
    )
    .expect("key s2c");

    match kd {
        ControlMsg::KeyDown(k) => {
            assert_eq!(k.key, key, "KeyDown 载荷往返一致");
        }
        other => panic!("期望 KeyDown，实际 {other:?}"),
    }
    match ku {
        ControlMsg::KeyUp(k) => {
            assert_eq!(k.key, key, "KeyUp 载荷往返一致（锁定 wire 修正）");
        }
        other => panic!("期望 KeyUp，实际 {other:?}"),
    }

    c_session.close(0, b"done");
    s_session.close(0, b"done");
}

// 复用 M4 的受控/控制端 + 自签证书环回建立逻辑。
mod common {
    use super::*;
    use landesk_core::SessionState;

    const DEV_ID: &str = "8f3a1b2c9d0e";
    const PASSWORD: &str = "s3cret";

    fn device(port: u16) -> OnlineDevice {
        OnlineDevice {
            ad: DeviceAd {
                id: DEV_ID.into(),
                name: "devboard".into(),
                enc: "openh264".into(),
                res: "1920x1080".into(),
                ver: 1,
                relay: false,
            },
            ip: "127.0.0.1".into(),
            port,
            last_seen: 0,
        }
    }

    pub async fn establish_session() -> (OneClickSession, OneClickSession) {
        let listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let controlled =
            Controlled::new(listen, &["localhost"], DEV_ID.into()).expect("controlled bind");
        let port = controlled.server().local_addr().expect("listen addr").port();

        let client = QuicClient::new(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            controlled.server().tls(),
        )
        .expect("client init");
        let controller = Controller::new(None, client);
        let dev = device(port);

        let (c_session, s_session) =
            tokio::try_join!(controller.connect(&dev, PASSWORD, "localhost"), controlled.wait(PASSWORD))
                .expect("connect + wait (ECDH 会话升级)");
        assert_eq!(c_session.state(), SessionState::Running);
        assert_eq!(s_session.state(), SessionState::Running);
        (c_session, s_session)
    }
}
