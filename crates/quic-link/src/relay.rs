//! QUIC 中继透传（FR-11 兜底，ADR-A3「仅字节透传、不解密、不存会话状态」）。
//!
//! P2P 打洞失败时，两端各自拨号到本中继；中继按信令下发的 **ticket**
//! （`SignalingMsg::Relay{to,ep,ticket}`，protocol.md §3.1）把**恰好两条** QUIC
//! 连接撮合成一对，再把它们之间的数据双向盲搬：
//!
//! - **控制流**（reliable）：两端各自开一条 uni 控制流写 GCM 密文（lp 帧已 E2E
//!   加密），中继 [`pump_control_pair`] 把 `src` 的流原样写进 `dst` 的流（不解析、
//!   不解密）；
//! - **视频 Datagram**：QUIC datagram 是 per-connection 的，远端发的不会自动到对端，
//!   须中继 [`forward_video`] `src.read_datagram → dst.send_datagram` 盲搬。
//!
//! ticket 撮合键的传递：客户端拨号后在**首条单向流**写 ticket（[`write_ticket`]），
//! 服务端 [`read_ticket`] 读回，[`RelayMatch`] 据此把同一 ticket 的两端配对。
//!
//! 纯逻辑（ticket 生成/解析 + 撮合表）可离线单测；真实 QUIC 透传走 127.0.0.1 环回
//! （`tests/relay.rs`）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use quinn::{Connection, Endpoint, RecvStream, SendStream, ServerConfig, TransportConfig};
use rand::RngCore;

use crate::error::QuicLinkError;
use crate::tls::{self, SelfSigned};

/// ticket 前导在单向流上的字节布局：`MAGIC(4) ‖ ticket(utf8, ≤31B) ‖ 0x00`。
/// 首 4 字节魔数 `RLY\x01` 把「ticket 流」与「控制流」区分开（避免误读）。
pub const RELAY_STREAM_MAGIC: [u8; 4] = [0x52, 0x4c, 0x59, 0x01];
/// ticket 载荷上限（总前导 ≤ 36B）。
pub const RELAY_TICKET_CAP: usize = 31;

/// 中继透传错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RelayError {
    /// ticket 超过 [`RELAY_TICKET_CAP`]（信令侧应截断或拒绝）。
    #[error("relay ticket too long: {0} > max({RELAY_TICKET_CAP})")]
    TicketTooLong(usize),
    /// 读 ticket 前导失败（魔数不符 / 流提前结束 / 对端未出示）。
    #[error("relay ticket stream not readable")]
    TicketUnreadable,
    /// 同一 ticket 出现第三条连接（协议侧不应发生，防御性拒绝）。
    #[error("relay ticket already has two endpoints")]
    TicketFull,
}

/// 生成一个随机 ticket（16B 熵 → 32 字节小写 hex，读起来像短 UUID）。
///
/// 真实 ticket 由信令服务端签发；本函数供测试 / 自签场景使用。
pub fn random_ticket() -> String {
    let mut buf = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    use std::fmt::Write as _;
    let mut s = String::with_capacity(32);
    for b in buf {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// 把 ticket 编码为可写入单向流的字节前导（`MAGIC ‖ ticket ‖ 0x00`）。
///
/// 超 [`RELAY_TICKET_CAP`] 返回 [`RelayError::TicketTooLong`]。
pub fn ticket_prefix_bytes(ticket: &str) -> Result<Vec<u8>, RelayError> {
    let b = ticket.as_bytes();
    if b.len() > RELAY_TICKET_CAP {
        return Err(RelayError::TicketTooLong(b.len()));
    }
    let mut out = Vec::with_capacity(4 + b.len() + 1);
    out.extend_from_slice(&RELAY_STREAM_MAGIC);
    out.extend_from_slice(b);
    out.push(0u8);
    Ok(out)
}

/// 在 `conn` 上开一条单向流写 ticket 前导并 `finish`（客户端拨号后第一件事）。
///
/// 连接已断 / 写失败归一为 [`RelayError::TicketUnreadable`]。
pub async fn write_ticket(conn: &Connection, ticket: &str) -> Result<(), RelayError> {
    let bytes = ticket_prefix_bytes(ticket)?;
    let send = conn
        .open_uni()
        .await
        .map_err(|_| RelayError::TicketUnreadable)?;
    let mut send = send;
    send.write_all(&bytes)
        .await
        .map_err(|_| RelayError::TicketUnreadable)?;
    send.finish().map_err(|_| RelayError::TicketUnreadable)?;
    Ok(())
}

/// 从 `conn` 接受的第一条单向流读回 ticket（魔数 + 终止符校验）。
///
/// 魔数不符 / 前导读不完整 → [`RelayError::TicketUnreadable`]。
pub async fn read_ticket(conn: &Connection) -> Result<String, RelayError> {
    let recv = conn
        .accept_uni()
        .await
        .map_err(|_| RelayError::TicketUnreadable)?;
    let mut recv = recv;
    let prefix = recv
        .read_to_end(4 + RELAY_TICKET_CAP + 1)
        .await
        .map_err(|_| RelayError::TicketUnreadable)?;
    if prefix.len() < 5 || prefix[..4] != RELAY_STREAM_MAGIC {
        return Err(RelayError::TicketUnreadable);
    }
    let payload = &prefix[4..];
    let end = payload.iter().position(|&b| b == 0u8).unwrap_or(payload.len());
    String::from_utf8(payload[..end].to_vec()).map_err(|_| RelayError::TicketUnreadable)
}

// ---------- 撮合表（ticket → 至多两条连接）----------

/// 一次 [`RelayMatch::add`] 的结果。
pub enum AddOutcome {
    /// 第一条连接，等对端。
    Waiting,
    /// 第二条连接，撮合成功，返回已配好的一对（可开泵）。
    Matched(Connection, Connection),
}

/// 中继撮合表：`ticket → 至多两条入向连接`。
///
/// `add` 在撮合成功时**移出**两条连接返回 [`AddOutcome::Matched`]（`Connection`
/// 虽可 `Clone`，但移出语义让「已撮合对」不再滞留表中）；同一 ticket 第三次到达
/// 报 [`RelayError::TicketFull`] 并清空该槽（防御性，避免内存泄漏）。
#[derive(Debug, Default)]
pub struct RelayMatch {
    table: HashMap<String, Vec<Connection>>,
}

impl RelayMatch {
    /// 登记一条入向连接（`ticket` 由 [`read_ticket`] 读出）。
    pub fn add(&mut self, ticket: &str, conn: Connection) -> Result<AddOutcome, RelayError> {
        let entry = self.table.entry(ticket.to_string()).or_default();
        if entry.len() >= 2 {
            entry.clear();
            return Err(RelayError::TicketFull);
        }
        entry.push(conn);
        if entry.len() == 2 {
            let a = entry.remove(0);
            let b = entry.remove(0);
            Ok(AddOutcome::Matched(a, b))
        } else {
            Ok(AddOutcome::Waiting)
        }
    }

    /// 当前在册的未撮合连接数（调试 / 日志）。
    pub fn pending(&self) -> usize {
        self.table.values().filter(|v| v.len() == 1).count()
    }
}

// ---------- 单向字节泵（控制流）----------

/// 单向字节泵：`from.read → to.write` 逐块盲搬，`from` EOF 即 `to.finish`。
///
/// 返回搬运字节数。任一侧连接丢失返回 [`QuicLinkError::ControlClosed`]。
/// 这是 ADR-A3「字节透传」的最小原语（不解密、不解析 lp 帧）。
pub async fn pump_stream(from: &mut RecvStream, to: &mut SendStream) -> Result<u64, QuicLinkError> {
    let mut buf = [0u8; 8192];
    let mut total = 0u64;
    loop {
        match from.read(&mut buf).await {
            Ok(Some(n)) => {
                to.write_all(&buf[..n])
                    .await
                    .map_err(|e| QuicLinkError::ControlWrite(e.to_string()))?;
                total += n as u64;
            }
            Ok(None) => break,
            Err(_) => return Err(QuicLinkError::ControlClosed),
        }
    }
    to.finish()
        .map_err(|_| QuicLinkError::ControlClosed)?;
    Ok(total)
}

/// 一条控制流泵：`src` 端上层开的一条 uni 控制流 → 中继 `src.accept_uni` 读 →
/// `dst.open_uni` 写 → `dst` 端上层 `accept_uni` 读。
///
/// 注意流序：每端开 2 条 uni（ticket 流 + 控制流）；本函数取的是**第 2 条**
/// （ticket 流须已先行被 [`read_ticket`] 消费）。`open_uni` 立即返回不阻塞，
/// 无跨泵死锁；阻塞点在「等 src 发控制帧」。
async fn pump_control_one(src: &Connection, dst: &Connection) -> Result<u64, QuicLinkError> {
    let mut recv = src
        .accept_uni()
        .await
        .map_err(|_| QuicLinkError::ControlClosed)?;
    let mut send = dst
        .open_uni()
        .await
        .map_err(|e| QuicLinkError::ControlOpen(e.to_string()))?;
    pump_stream(&mut recv, &mut send).await
}

/// 双向控制流泵（并发）：`a→b` 与 `b→a` 各一条 [`pump_control_one`]，`join!` 并发。
///
/// 返回 `(a→b 字节数, b→a 字节数)`。任一侧连接关闭（`ControlClosed`）→ 整体
/// `Err`（调用方据此停另一侧 / 关连接）。
pub async fn pump_control_pair(
    a: &Connection,
    b: &Connection,
) -> Result<(u64, u64), QuicLinkError> {
    let (ab, ba) = tokio::join!(pump_control_one(a, b), pump_control_one(b, a),);
    let ab = ab.map_err(|_| QuicLinkError::ControlClosed)?;
    let ba = ba.map_err(|_| QuicLinkError::ControlClosed)?;
    Ok((ab, ba))
}

// ---------- 视频 Datagram 盲搬 ----------

/// 把 `src` 的下一个视频 Datagram 原样转发给 `dst`（不解析、不解密）。
///
/// `src` 无数据（连接关闭）→ [`QuicLinkError::ControlClosed`]；`dst` 容量不足 /
/// 已关 → [`QuicLinkError::VideoDropped`]。返回搬走的明文包（供测试断言）。
pub async fn forward_video(src: &Connection, dst: &Connection) -> Result<Vec<u8>, QuicLinkError> {
    let dg = src
        .read_datagram()
        .await
        .map_err(|_| QuicLinkError::ControlClosed)?;
    dst.send_datagram(dg.clone())
        .map_err(|_| QuicLinkError::VideoDropped)?;
    Ok(dg.to_vec())
}

// ---------- 中继端点（薄壳 + 环回测试共用）----------

/// 中继 QUIC 端点：自签证书 + 入向 `accept`，返回**裸** [`Connection`]（不套 GCM——
/// 中继 ADR-A3 不解密，控制流 / Datagram 全是盲搬的字节，端点本身无需会话密钥）。
///
/// 与 [`crate::server::QuicServer`] 的区别：那边产出 `QuicConnection`（带会话 GCM），
/// 本端点只透传，故直接给裸连接 + 自签证书（客户端 [`crate::QuicClient`] 信任它拨号）。
pub struct RelayEndpoint {
    endpoint: Endpoint,
    tls: SelfSigned,
}

impl RelayEndpoint {
    /// 在 `listen` 地址监听（`port=0` 时 [`local_addr`] 取实际分配端口），
    /// 自签证书覆盖 `hostnames`。
    pub fn bind(listen: SocketAddr, hostnames: &[&str]) -> Result<Self, QuicLinkError> {
        let tls = tls::self_signed(hostnames).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        let mut server_config =
            ServerConfig::with_single_cert(vec![tls.cert_der.clone()], tls.key.clone_key())
                .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        server_config.transport_config(Arc::new(TransportConfig::default()));
        let endpoint =
            Endpoint::server(server_config, listen).map_err(|e| QuicLinkError::EndpointInit(e.to_string()))?;
        Ok(Self { endpoint, tls })
    }

    /// 实际监听地址（`port=0` 时取系统分配端口）。
    pub fn local_addr(&self) -> Result<SocketAddr, QuicLinkError> {
        self.endpoint
            .local_addr()
            .map_err(|e| QuicLinkError::EndpointInit(e.to_string()))
    }

    /// 自签证书（客户端信任根 / 环回测试用）。
    pub fn tls(&self) -> &SelfSigned {
        &self.tls
    }

    /// 接受一条入向连接（裸 [`Connection`]；中继侧随后 [`read_ticket`] + 撮合）。
    pub async fn accept(&self) -> Result<Connection, QuicLinkError> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or(QuicLinkError::AcceptFailed("endpoint closed".into()))?;
        let connecting = incoming.accept().map_err(|e| QuicLinkError::AcceptFailed(e.to_string()))?;
        let conn = connecting.await.map_err(|e| QuicLinkError::ConnectFailed(e.to_string()))?;
        Ok(conn)
    }

    /// 优雅关闭（不再接受新连接；已撮合对在 `close_pair` 里收尾）。
    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"relay close");
    }
}

/// 对一对已撮合连接 [`a`,`b`] 驱动全部透传通道，直到任一侧连接关闭。
///
/// 三路并发：控制流双向泵（[`pump_control_pair`]）+ 视频 Datagram 双向全量转发
/// （`forward_all`）。`tokio::join!` 汇合——连接一关，三路都返回；返回本对已搬运的
/// `(a→b 控制字节, b→a 控制字节, a→b 视频包, b→a 视频包)`。
///
/// 这是 [`apps/relay-server`] 薄壳主循环与环回测试的核心原语。控制流因连接中途
/// 关闭而未收全时其字节计数记 0（视频 Datagram 计数仍有效——datagram 通道先于
/// 可靠流关闭是常见情况）。
pub async fn drive_matched(a: &Connection, b: &Connection) -> (u64, u64, u64, u64) {
    let video_ab = forward_all(a, b); // a 发的视频包 → b
    let video_ba = forward_all(b, a); // b 发的视频包 → a
    let ctrl = pump_control_pair(a, b);

    let (ctrl_res, vab, vba) = tokio::join!(ctrl, video_ab, video_ba);
    let (cab, cba) = match ctrl_res {
        Ok((ab, ba)) => (ab, ba),
        Err(_) => (0, 0), // 连接关 → 控制流未收全；视频计数保留
    };
    (cab, cba, vab, vba)
}

/// 全量转发 `src` 的视频 Datagram 到 `dst`，直到 `src` 连接关闭（返回转发包数）。
///
/// 复用 [`forward_video`]（阻塞读 `src` 下一个 datagram）；`src` 关连接即
/// `ControlClosed` → 停，返回已累计包数。环回测试可直接用它驱动单方向视频通道。
pub async fn forward_all(src: &Connection, dst: &Connection) -> u64 {
    let mut n = 0u64;
    while forward_video(src, dst).await.is_ok() {
        n += 1;
    }
    n
}

// 说明：`drive_matched` 为 MVP 透传驱动器——控制流与视频 Datagram 并发盲搬，
// 任一侧关连接即停。完整 per-pair 统计日志（M7 收尾）接 tracing。


// ---------- 离线单测（纯逻辑：ticket / 撮合表）----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ticket_is_32_hex() {
        let t = random_ticket();
        assert_eq!(t.len(), 32, "16B → 32 位 hex");
        assert!(t.bytes().all(|c| c.is_ascii_hexdigit()));
        // 两次随机应不同（16B 熵，碰撞概率可忽略）。
        assert_ne!(t, random_ticket());
    }

    #[test]
    fn ticket_prefix_layout() {
        let bytes = ticket_prefix_bytes("abc").unwrap();
        assert_eq!(&bytes[..4], &RELAY_STREAM_MAGIC);
        assert_eq!(&bytes[4..7], b"abc");
        assert_eq!(bytes[7], 0u8, "末位终止符");
        assert_eq!(bytes.len(), 4 + 3 + 1);
    }

    #[test]
    fn ticket_too_long_rejected() {
        let long = "x".repeat(RELAY_TICKET_CAP + 1);
        assert_eq!(
            ticket_prefix_bytes(&long),
            Err(RelayError::TicketTooLong(long.len()))
        );
        // 恰好上限可通过。
        let max = "y".repeat(RELAY_TICKET_CAP);
        assert!(ticket_prefix_bytes(&max).is_ok());
    }

    #[test]
    fn match_default_empty() {
        // 空撮合表 pending=0（Waiting→Matched→Full 的状态机在 tests/relay.rs
        // 环回里覆盖，因 Connection 不可凭空构造）。
        let m = RelayMatch::default();
        assert_eq!(m.pending(), 0);
    }
}
