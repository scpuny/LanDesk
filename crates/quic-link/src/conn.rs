//! 双通道连接句柄（FR-05）。
//!
//! 一条 [`quinn::Connection`] 上同时承载：
//! - **可靠控制流**（reliable bidirectional stream）：跑 `lp` 控制帧
//!   （鉴权 / 输入 / 剪贴板 / 编码器控制 / 心跳），有序、不丢；
//! - **不可靠视频 Datagram**（QUIC datagram）：跑 H.264 NALU 分片，
//!   低时延、可容忍丢包（丢 P 帧分片由受控端请求强制 IDR 兜底，protocol.md §2.2）。
//!
//! 控制流采用「2B 长度前缀 + GCM 密文」分帧；视频 Datagram 为单包明文
//! （加解密在 `landesk-core` 套会话 GCM，QUIC 层保持与密码学解耦）。

use quinn::{Connection, ReadExactError, ReadError, RecvStream, SendStream};

use crate::error::QuicLinkError;
use crate::gcm::LpGcm;

/// 双通道连接。
///
/// 持有 quinn 连接与控制流 GCM 会话；视频 Datagram 明文层由
/// `lp::datagram::VideoDgram` 提供。
pub struct QuicConnection {
    conn: Connection,
    gcm: LpGcm,
}

impl QuicConnection {
    /// 内部构造（`server` / `client` 模块调用）。
    pub(crate) fn new(conn: Connection, gcm: LpGcm) -> Self {
        Self { conn, gcm }
    }

    /// 底层 quinn 连接（供 `close_reason` / 统计等高级操作）。
    pub fn raw(&self) -> &Connection {
        &self.conn
    }

    /// 本连接的控制流 GCM 会话（供上层读取 `prefix` 参与握手交换）。
    pub fn gcm(&self) -> &LpGcm {
        &self.gcm
    }

    /// 远端地址。
    pub fn remote_addr(&self) -> std::net::SocketAddr {
        self.conn.remote_address()
    }

    /// 本地 IP（连接未确定时可能为 `None`）。
    pub fn local_ip(&self) -> Option<std::net::IpAddr> {
        self.conn.local_ip()
    }

    // ---------- 可靠控制流 ----------

    /// 打开一条可靠双向控制流，返回 `(send, recv)`。
    ///
    /// 调用方约定：写侧先 `write_control_cipher` 再 `finish`；读侧用
    /// `read_control_cipher` 逐帧读取（2B 长度前缀 + GCM 密文含 tag）。
    pub async fn open_control_stream(&self) -> Result<(SendStream, RecvStream), QuicLinkError> {
        let (send, recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|e| QuicLinkError::ControlOpen(e.to_string()))?;
        Ok((send, recv))
    }

    /// 接受对端打开的可靠双向控制流（与对端 `open_control_stream` 配对），
    /// 返回 `(send, recv)`。用于服务端接收客户端发起的控制流。
    pub async fn accept_control_stream(&self) -> Result<(SendStream, RecvStream), QuicLinkError> {
        let (send, recv) = self
            .conn
            .accept_bi()
            .await
            .map_err(|e| QuicLinkError::ControlOpen(e.to_string()))?;
        Ok((send, recv))
    }

    /// 在控制流上写一条**已加密**控制帧：`len(2B LE) ‖ gcm_cipher(含 tag)`。
    ///
    /// `cipher` 由 `lp::wire::ControlFrame::encrypt` 产出（含 12B tag）。
    pub async fn write_control_cipher(
        &self,
        send: &mut SendStream,
        cipher: &[u8],
    ) -> Result<(), QuicLinkError> {
        let len = cipher.len() as u16;
        send.write_all(&len.to_le_bytes())
            .await
            .map_err(|e| QuicLinkError::ControlWrite(e.to_string()))?;
        send.write_all(cipher)
            .await
            .map_err(|e| QuicLinkError::ControlWrite(e.to_string()))?;
        Ok(())
    }

    /// 从控制流读一条**已加密**控制帧，返回 2B 长度前缀之后的 GCM 密文（含 tag）。
    ///
    /// 流正常 EOF / 对端 reset / 连接丢失时返回 [`QuicLinkError::ControlClosed`]。
    pub async fn read_control_cipher(
        &self,
        recv: &mut RecvStream,
    ) -> Result<Vec<u8>, QuicLinkError> {
        let mut len_buf = [0u8; 2];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(Self::read_exact_err)?;
        let len = u16::from_le_bytes(len_buf) as usize;
        let mut cipher = vec![0u8; len];
        recv.read_exact(&mut cipher)
            .await
            .map_err(Self::read_exact_err)?;
        Ok(cipher)
    }

    /// 控制流读错归一：任何读不到更多数据的情形均视为会话关闭。
    fn read_exact_err(e: ReadExactError) -> QuicLinkError {
        match e {
            ReadExactError::FinishedEarly(_) => QuicLinkError::ControlClosed,
            ReadExactError::ReadError(re) => match re {
                ReadError::Reset(_) | ReadError::ConnectionLost(_) => QuicLinkError::ControlClosed,
                // 流已 stop/finish/reset、或 0-RTT 被拒等——同样视为不可再读。
                ReadError::ClosedStream
                | ReadError::IllegalOrderedRead
                | ReadError::ZeroRttRejected => QuicLinkError::ControlClosed,
            },
        }
    }

    /// 非阻塞探测本连接是否已关闭（quinn [`quinn::Connection::close_reason`] 非 `None` 即已关）。
    ///
    /// 供上层（会话重连编排）判断「会话是否掉线」——不阻塞、不占控制流锁，可随时轮询。
    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }

    /// 关闭本连接（正常会话结束）。
    pub fn close(&self, code: u32, reason: &[u8]) {
        self.conn.close(code.into(), reason);
    }

    // ---------- 不可靠视频 Datagram ----------

    /// 发送一个视频 Datagram（**明文**，`lp::datagram::VideoDgram::encode_plain` 输出）。
    ///
    /// 超对端通告的 datagram 容量时返回 [`QuicLinkError::VideoDropped`]；
    /// 上层据此降级码率或改走分片 + 请求 IDR。
    pub fn send_video_datagram(&self, plain: &[u8]) -> Result<(), QuicLinkError> {
        self.conn
            .send_datagram(bytes::Bytes::copy_from_slice(plain))
            .map_err(|_| QuicLinkError::VideoDropped)
    }

    /// 接收下一个视频 Datagram（明文）；无数据时阻塞等待，连接关闭返回
    /// [`QuicLinkError::ControlClosed`]。
    pub async fn recv_video_datagram(&self) -> Result<Vec<u8>, QuicLinkError> {
        match self.conn.read_datagram().await {
            Ok(dg) => Ok(dg.to_vec()),
            Err(_) => Err(QuicLinkError::ControlClosed),
        }
    }

    /// 克隆出一个**可独立持有**的视频 Datagram 接收句柄（`Send + Sync + 'static`）。
    ///
    /// `quinn::Connection` 是 `Clone`（共享底层 UDP 驱动 + datagram 收发队列），
    /// 克隆出的接收者与原连接共用 `read_datagram()` 通道。故可 `move` 进后台
    /// `tokio::spawn` 的推流任务，**不持有** `OneClickSession` 的控制流 `&mut` 锁
    /// ——视频接收与控制帧收发互不阻塞（P2② 画面推流）。
    pub fn video_rx(&self) -> VideoDatagramRx {
        VideoDatagramRx {
            conn: self.conn.clone(),
        }
    }

    /// 克隆出一个**可独立持有**的视频 Datagram 发送句柄（`Send + Sync + 'static`）。
    ///
    /// 与 [`Self::video_rx`] 对称：受控端把发流泵 `move` 进独立 `tokio` 任务时，
    /// 只需 `send_datagram`（不碰会话控制流的 `&mut`）。这样视频发送与控制帧
    /// 接收（期四键鼠注入）可在同一会话上并发，互不争用 `OneClickSession`。
    pub fn video_tx(&self) -> VideoDatagramTx {
        VideoDatagramTx {
            conn: self.conn.clone(),
        }
    }
}

/// 视频 Datagram 接收句柄（[`QuicConnection`] 的 `&self` 克隆视图，可跨任务持有）。
///
/// 见 [`QuicConnection::video_rx`]；后台推流泵用它在独立任务里循环读视频 Datagram，
/// 与控制流锁解耦。
#[derive(Debug, Clone)]
pub struct VideoDatagramRx {
    conn: Connection,
}

impl VideoDatagramRx {
    /// 读下一个视频 Datagram（明文）；连接关闭返回 [`QuicLinkError::ControlClosed`]。
    pub async fn recv(&self) -> Result<Vec<u8>, QuicLinkError> {
        match self.conn.read_datagram().await {
            Ok(dg) => Ok(dg.to_vec()),
            Err(_) => Err(QuicLinkError::ControlClosed),
        }
    }
}

/// 视频 Datagram 发送句柄（[`QuicConnection`] 的 `&self` 克隆视图，可跨任务持有）。
///
/// 见 [`QuicConnection::video_tx`]；受控端发流泵用它在独立任务里循环发视频 Datagram，
/// 与控制帧接收并发（GUI 兼作受控端，画面 + 键鼠同时跑）。
#[derive(Debug, Clone)]
pub struct VideoDatagramTx {
    conn: Connection,
}

impl VideoDatagramTx {
    /// 发送一个视频 Datagram（明文）；超对端 datagram 容量返回
    /// [`QuicLinkError::VideoDropped`]。
    pub fn send(&self, plain: &[u8]) -> Result<(), QuicLinkError> {
        self.conn
            .send_datagram(bytes::Bytes::copy_from_slice(plain))
            .map_err(|_| QuicLinkError::VideoDropped)
    }
}
