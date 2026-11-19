//! RFC 5389 STUN 绑定（binding）编解码（`docs/protocol.md` §3.2，自实现，无第三方 STUN 依赖）。
//!
//! 仅支持 Binding 类型（MVP 无 TURN 凭据；中继为自建 QUIC，非 TURN）。提供：
//! - [`StunBindingRequest`]：构造绑定请求（20B 头，magic cookie + 96-bit 事务 ID）；
//! - [`StunResponse`]：构造绑定成功响应（回显事务 ID + `XOR-MAPPED-ADDRESS`）；
//! - [`decode_mapped_address`]：从响应解析映射地址（`MAPPED-ADDRESS` / `XOR-MAPPED-ADDRESS`），
//!   供打洞侧识别 NAT 反射候选（SRFLX）。
//!
//! 纯字节逻辑、零平台依赖，可离线单测（合成响应喂入，校验解析）。

use rand::rngs::OsRng;
use rand::RngCore;
use tokio::net::UdpSocket;

/// RFC 5389 magic cookie。
pub const MAGIC_COOKIE: u32 = 0x21_12_A4_42;

/// 绑定请求类型（§3.2.1）。
pub const MSG_BINDING_REQUEST: u16 = 0x0001;
/// 绑定成功响应类型。
pub const MSG_BINDING_SUCCESS: u16 = 0x0101;

/// 属性：MAPPED-ADDRESS。
pub const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
/// 属性：XOR-MAPPED-ADDRESS（IPv4 场景推荐，地址/端口与 magic cookie 异或）。
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// 固定头部 20B：type(2) + len(2) + cookie(4) + txn(12)。
pub const HEADER_LEN: usize = 20;

/// 绑定请求（无属性，20B）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StunBindingRequest {
    /// 96-bit 事务 ID（随机，用于把响应匹配回请求）。
    pub txn_id: [u8; 12],
}

impl StunBindingRequest {
    /// 生成随机事务 ID 的绑定请求。
    pub fn random() -> Self {
        let mut t = [0u8; 12];
        OsRng.fill_bytes(&mut t);
        Self { txn_id: t }
    }

    /// 固定事务 ID（测试 / 可复现打洞）。
    pub fn with_txn_id(txn_id: [u8; 12]) -> Self {
        Self { txn_id }
    }

    /// 编码 20B 请求包。
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0..2].copy_from_slice(&MSG_BINDING_REQUEST.to_be_bytes());
        out[2..4].copy_from_slice(&0u16.to_be_bytes()); // 无属性 → len=0
        out[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());
        out[8..20].copy_from_slice(&self.txn_id);
        out
    }
}

/// 绑定成功响应。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StunResponse {
    pub txn_id: [u8; 12],
    /// NAT 映射后的源地址（对端打洞拨号目标）。
    pub mapped_addr: std::net::SocketAddr,
}

impl StunResponse {
    /// 构造成功响应（`XOR-MAPPED-ADDRESS`，IPv4）。`mapped_addr` 应为 IPv4。
    pub fn new_success(txn_id: [u8; 12], mapped_addr: std::net::SocketAddr) -> Option<Self> {
        // 仅支持 IPv4（MVP；IPv6 走 MAPPED-ADDRESS 变体，预留）。
        if !mapped_addr.ip().is_ipv4() {
            return None;
        }
        Some(Self {
            txn_id,
            mapped_addr,
        })
    }

    /// 编码：20B 头 + XOR-MAPPED-ADDRESS 属性（4B 头 + 9B 值）。
    ///
    /// 值布局（IPv4，RFC 5389 §6.3.4）：reserved(2) | family=1(1) | port(2) | addr(4)。
    /// 消息按 4B 对齐，尾部补 3 个 0。
    pub fn to_bytes(&self) -> Option<Vec<u8>> {
        let ip4 = match self.mapped_addr.ip() {
            std::net::IpAddr::V4(v4) => v4,
            _ => return None,
        };
        let port = self.mapped_addr.port();
        let attr_len: u16 = 9;

        // XOR-MAPPED：port 与 (magic_cookie >> 16) 异或；addr 与 magic_cookie 异或。
        let xor_port = port ^ ((MAGIC_COOKIE >> 16) as u16);
        let xor_addr = u32::from(ip4) ^ MAGIC_COOKIE;

        let mut out = Vec::with_capacity(HEADER_LEN + 13 + 3);
        out.extend_from_slice(&MSG_BINDING_SUCCESS.to_be_bytes());
        out.extend_from_slice(&attr_len.to_be_bytes());
        out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        out.extend_from_slice(&self.txn_id);
        // 属性：type(2) + len(2) + value(9) → 13B
        out.extend_from_slice(&ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
        out.extend_from_slice(&attr_len.to_be_bytes());
        out.extend_from_slice(&[0u8, 0u8]); // reserved
        out.push(0x01); // family = IPv4
        out.extend_from_slice(&xor_port.to_be_bytes());
        out.extend_from_slice(&xor_addr.to_be_bytes());
        out.extend_from_slice(&[0u8; 3]); // 4B 对齐填充
        Some(out)
    }
}

/// 从响应字节解析映射地址（`XOR-MAPPED-ADDRESS` 优先，回退 `MAPPED-ADDRESS`）。
///
/// 返回 `(IpAddr, port)`；无匹配属性返回 `None`。
pub fn decode_mapped_address(msg: &[u8]) -> Option<std::net::SocketAddr> {
    if msg.len() < HEADER_LEN {
        return None;
    }
    // 校验 magic cookie（§3.2 抗报文注入）。
    let cookie = u32::from_be_bytes([msg[4], msg[5], msg[6], msg[7]]);
    if cookie != MAGIC_COOKIE {
        return None;
    }
    let mut off = HEADER_LEN;
    while off + 4 <= msg.len() {
        let attr_type = u16::from_be_bytes([msg[off], msg[off + 1]]);
        let attr_len = u16::from_be_bytes([msg[off + 2], msg[off + 3]]) as usize;
        let val_off = off + 4;
        if val_off + attr_len > msg.len() {
            break;
        }
        let val = &msg[val_off..val_off + attr_len];
        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Some((ip, port)) = parse_xor_mapped(val, MAGIC_COOKIE) {
                    return Some(std::net::SocketAddr::new(ip, port));
                }
            }
            ATTR_MAPPED_ADDRESS => {
                if let Some((ip, port)) = parse_plain_mapped(val) {
                    return Some(std::net::SocketAddr::new(ip, port));
                }
            }
            _ => {}
        }
        // 属性值按 4B 对齐（RFC 5389 §4.8）：跳过本属性（4B 头 + align_up(len,4)）。
        off = val_off + ((attr_len + 3) & !3);
    }
    None
}

/// 解析 XOR-MAPPED-ADDRESS 值（9B：res(2)+family(1)+port(2)+addr(4)，RFC 5389 §6.3.4）。
fn parse_xor_mapped(val: &[u8], cookie: u32) -> Option<(std::net::IpAddr, u16)> {
    if val.len() != 9 || val[2] != 0x01 {
        return None;
    }
    let xor_port = u16::from_be_bytes([val[3], val[4]]);
    let port = xor_port ^ ((cookie >> 16) as u16);
    let xor_addr = u32::from_be_bytes([val[5], val[6], val[7], val[8]]);
    let ip4 = xor_addr ^ cookie;
    Some((std::net::IpAddr::V4(std::net::Ipv4Addr::from(ip4)), port))
}

/// 解析 MAPPED-ADDRESS 值（9B，非异或；§6.1.4）。
fn parse_plain_mapped(val: &[u8]) -> Option<(std::net::IpAddr, u16)> {
    if val.len() != 9 || val[2] != 0x01 {
        return None;
    }
    let port = u16::from_be_bytes([val[3], val[4]]);
    let ip4 = u32::from_be_bytes([val[5], val[6], val[7], val[8]]);
    Some((std::net::IpAddr::V4(std::net::Ipv4Addr::from(ip4)), port))
}

/// 异步 STUN 反射器（§3.2：信令服务可内嵌，UDP 反射绑定请求）。
///
/// 读一个绑定请求 → 用「请求来源地址」（NAT 后的映射地址）构造成功响应回发。
/// 环回测试可在 127.0.0.1 起，客户端源端口即「映射地址」。
pub struct StunServer {
    socket: UdpSocket,
}

impl std::fmt::Debug for StunServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StunServer")
            .field("local_addr", &self.local_addr())
            .finish()
    }
}

impl StunServer {
    /// 绑定 UDP 反射器（`addr` 端口 0 = 系统分配）。
    pub async fn bind(addr: std::net::SocketAddr) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(addr).await?;
        Ok(Self { socket })
    }

    /// 本端监听地址（取环回端口喂客户端拨号）。
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.socket.local_addr().expect("bound")
    }

    /// 读一个数据报：若是合法绑定请求（类型 + magic cookie 校验），
    /// 用请求来源地址回发成功响应，返回响应字节（测试可再 `decode_mapped_address`）；
    /// 非绑定请求返回 `None`（不外发）。
    pub async fn serve_one(&self) -> std::io::Result<Option<Vec<u8>>> {
        let mut buf = [0u8; 512];
        let (n, from) = self.socket.recv_from(&mut buf).await?;
        if n < HEADER_LEN {
            return Ok(None);
        }
        let msg_type = u16::from_be_bytes([buf[0], buf[1]]);
        if msg_type != MSG_BINDING_REQUEST {
            return Ok(None);
        }
        let cookie = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if cookie != MAGIC_COOKIE {
            return Ok(None);
        }
        let txn: [u8; 12] = buf[8..20].try_into().unwrap();
        // 用请求来源（NAT 映射地址）作为 mapped address；非 IPv4 时回退请求内 SOURCE-ADDRESS。
        let source_addr = if from.ip().is_ipv4() {
            from
        } else {
            decode_mapped_address(&buf[..n]).unwrap_or(from)
        };
        let Some(resp) = StunResponse::new_success(txn, source_addr) else {
            return Ok(None);
        };
        let bytes = resp.to_bytes().expect("source_addr 已校验为 IPv4");
        self.socket.send_to(&bytes, from).await?;
        Ok(Some(bytes))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_header_fields() {
        let req = StunBindingRequest::with_txn_id([0xAA; 12]);
        let b = req.to_bytes();
        assert_eq!(&b[0..2], &MSG_BINDING_REQUEST.to_be_bytes());
        assert_eq!(&b[2..4], &0u16.to_be_bytes(), "无属性 → len=0");
        assert_eq!(u32::from_be_bytes([b[4], b[5], b[6], b[7]]), MAGIC_COOKIE);
        assert_eq!(&b[8..20], &[0xAA; 12], "txn id 透传");
    }

    #[test]
    fn response_roundtrip_xor_mapped() {
        let sa = std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 2, 3, 4)), 4433);
        let resp = StunResponse::new_success([0x11; 12], sa).unwrap();
        let bytes = resp.to_bytes().unwrap();
        let got = decode_mapped_address(&bytes).expect("mapped addr parsed");
        assert_eq!(got, sa, "XOR-MAPPED-ADDRESS 往返还原源地址");
    }

    #[test]
    fn decode_rejects_bad_cookie() {
        let mut req = StunBindingRequest::with_txn_id([0; 12]).to_bytes().to_vec();
        req[4] = 0xFF; // 破坏 magic cookie
        assert!(decode_mapped_address(&req).is_none(), "cookie 不符即拒（§3.2 抗注入）");
    }

    #[test]
    fn mapped_address_not_ipv4_returns_none() {
        let v6 = std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 1);
        assert!(StunResponse::new_success([0; 12], v6).is_none(), "MVP 仅 IPv4");
    }

    /// 环回 STUN 绑定往返：反射器回发响应，客户端解析映射地址（= 本端源地址）。
    #[tokio::test]
    async fn stun_binding_roundtrip_loopback() {
        let server = StunServer::bind(std::net::SocketAddr::from((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .await
        .expect("stun bind");
        let server_addr = server.local_addr();

        // 客户端：起一个源 UDP socket（源端口即「映射地址」），发绑定请求。
        let client = tokio::net::UdpSocket::bind(std::net::SocketAddr::from((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .await
        .expect("client bind");
        let client_src = client.local_addr().expect("client local");

        let req = StunBindingRequest::random();
        client.send_to(&req.to_bytes(), server_addr).await.expect("send req");

        // 反射器回发响应；客户端读一次 + 解析。
        server.serve_one().await.unwrap().expect("stun response");
        let mut rbuf = [0u8; 512];
        let n = client.recv_from(&mut rbuf).await.unwrap().0;
        let client_resp = rbuf[..n].to_vec();
        let got = decode_mapped_address(&client_resp).expect("mapped addr from response");
        assert_eq!(got, client_src, "反射器回发的映射地址 = 客户端源地址（环回模拟 NAT）");
    }
}
