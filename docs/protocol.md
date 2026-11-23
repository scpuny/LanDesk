# LanDesk 内部协议规范

> 配套文档：`docs/PRD.md`（V0.1 MVP）
> 版本：V0.1 | 状态：草稿
> 适用范围：LanDesk 控制通道、视频通道、信令通道、mDNS 发现的四类报文定义

---

## 0. 通用约定

| 项 | 约定 |
|---|---|
| 字节序 | 全部 **小端 (LE)**，与目标平台（x86_64/aarch64）天然一致，减少转换成本 |
| 版本字段 | 每条报文带 `u8 ver`，MVP 固定 `1`；不兼容变更升 `major` |
| 编码 | 文本一律 UTF-8；长度前缀单位字节 |
| 长度前缀 | 流式报文 `u16`（≤65535）；超大载荷（剪贴板）分段，单段 ≤ 64KB |
| 加密 | 所有应用层报文（控制流 + 视频 Datagram）载荷经 **AES-256-GCM** 加密，禁止裸流（NFR-03） |
| 完整性 | GCM tag 12 字节附加在密文末尾，随报文传输 |

### 0.1 密钥派生

```
master_key(32B) = HKDF-SHA256(
    ikm  = X25519(shared_secret, client_pub, server_priv),
    salt = client_nonce(32) || server_nonce(32),
    info = b"landesk-v1"
)
enc_key(32B) = SHA256(master_key || b"enc")   // 会话对称加密密钥
```

- 会话密钥 ECDH（X25519）协商一次，整段会话复用 `enc_key`；重连（RECONNECT）重新走握手换新密钥。
- GCM 96-bit nonce 构造：`nonce = prefix(4B, 会话随机前缀) || seq(8B, LE)`（合计恰好 96-bit，AES-GCM 固定要求）
  - 控制流 seq = 该流单调递增报文计数（u64，LE）
  - 视频 Datagram seq = 帧序号（u32 左扩展 4B 零 + u32 分片号）
  - 发送端保证同 nonce 永不复用；接收端检测到 nonce 重复直接断会话（安全告警日志）。

### 0.2 密码校验（不传输明文）

- 受控端将预共享密码以 `PBKDF2-SHA256(100_000 次, salt=device_id||"landesk")` 存 32B 派生值，**不存明文**（NFR-03）。
- 握手时：
  1. 控制端发 `client_nonce(32)` + 自身 X25519 公钥 + `proof = SHA256(PBKDF2(password, client_nonce||server_nonce, 100_000))`
  2. 受控端用本地存的密码派生值验证（一次性密码由受控端生成并持有，控制端输入后即校验销毁）
  3. 双向 nonce + proof 互验，通过后双方独立派生 `master_key`，再发 `AUTH_RESULT{ok}`
- 校验失败发 `AUTH_RESULT{code}` 后关闭 QUIC 连接，明确原因（FR-02.4）。

---

## 1. 控制报文（QUIC 可靠流，Stream A）

### 1.1 帧结构

```
+------+----------+----------+----------------+------------------+
| ver 1B | type 1B | len 2B   | payload len B  | gcm_tag 12B      |
+------+----------+----------+----------------+------------------+
payload 整体（含 ver/type/len 头）为 AES-GCM 明文，tag 校验整个帧
```

- `len` = payload 字节数（不含头与 tag）。
- 单帧上限 64KB；超出则分段（剪贴板见 1.3）。

### 1.2 报文类型表（type 值）

| type | 名称 | 方向 | 说明 |
|---|---|---|---|
| 0x01 | AUTH_REQUEST | C→S | 握手请求（见 0.2） |
| 0x02 | AUTH_RESULT | 双向 | 握手结果 / 失败原因 |
| 0x10 | MOUSE_MOVE | C→S | 坐标 + 按键态 |
| 0x11 | MOUSE_BUTTON | C→S | 按下/释放 |
| 0x12 | MOUSE_WHEEL | C→S | 滚轮/横轮 |
| 0x20 | KEY_DOWN | C→S | 按键按下 |
| 0x21 | KEY_UP | C→S | 按键释放 |
| 0x30 | CLIPBOARD_START | 双向 | 剪贴板分段开始（含总长） |
| 0x31 | CLIPBOARD_CHUNK | 双向 | 剪贴板分段数据 |
| 0x32 | CLIPBOARD_END | 双向 | 剪贴板结束（含校验） |
| 0x40 | ENCODE_PARAMS | C→S | 码率/I帧间隔/预设调整（FR-04.5） |
| 0x41 | ENCODE_EVENT | S→C | 编码器重置/降级通知（FR-04.6） |
| 0x42 | DISPLAY_INFO | S→C | 显示器列表/当前分辨率 |
| 0x43 | DISPLAY_SWITCH | C→S | 切换目标显示器 |
| 0x50 | PING | 双向 | 延迟测量（带 u64 时间戳） |
| 0x51 | PONG | 双向 | 回显时间戳 |
| 0x60 | SESSION_CLOSE | 双向 | 主动结束（带 reason） |
| 0x61 | RECONNECT | 双向 | 请求重握手（FR-05.4） |
| 0x70 | HEARTBEAT | S→C | 周期性 1B 心跳，保活/探活 |

### 1.3 载荷定义（Rust）

```rust
use byteorder::{BigEndian, LittleEndian}; // 实际全部 LE

// ---- 鉴权 ----
pub struct AuthRequest {
    pub device_id: [u8; 16],      // 受控端设备ID（128bit）
    pub server_nonce: [u8; 32],
    pub client_nonce: [u8; 32],
    pub client_x25519: [u8; 32],
    pub proof: [u8; 32],          // 密码证明（见 0.2）
    pub flags: u8,                // bit0: 一次性密码模式
}
pub struct AuthResult {
    pub code: u8,                 // 0=成功, 1=密码错误, 2=设备不存在, 3=nonce重放, 4=OTP已失效
    pub server_x25519: [u8; 32],  // 仅成功时有效
    pub server_nonce: [u8; 32],
}

// ---- 输入（FR-06）----
// 坐标映射：控制端发自己视口归一化坐标 [0,65535]，受控端乘自身分辨率还原（FR-06.3）
pub struct MouseMove {
    pub x: u16,          // 0..=65535 → x/65535 * screen_w
    pub y: u16,
    pub buttons: u16,   // bit0左 bit1右 bit2中
    pub mods: u8,       // bit0 Ctrl bit1 Shift bit2 Alt bit3 Meta（FR-06.2）
}
pub struct MouseButton { pub button: u8, pub down: u8 }
pub struct MouseWheel { pub dx: i16, pub dy: i16 } // 负=下/左

pub struct KeyEvent {
    pub key: u32,    // 平台无关键码表（见 §1.4）
    pub mods: u8,    // 同上
}

// ---- 剪贴板（FR-07，双向）----
pub struct ClipboardStart { pub total_len: u32, pub mime_flags: u8 } // 仅文本
pub struct ClipboardChunk { pub seq: u16, pub data: [u8; 32_768] } // 定长分段，末段短
pub struct ClipboardEnd { pub sha256: [u8; 32] }
// 单份剪贴板上限 1MB，超出丢弃并提示（FR-07.2）

// ---- 编码器控制（FR-04.5）----
pub struct EncodeParams {
    pub bitrate_kbps: u16,     // 0=维持
    pub keyframe_interval: u16,
    pub preset: u8,            // 0=流畅 1=平衡 2=高清（FR-08.5）
}
pub struct EncodeEvent {
    pub code: u8,              // 1=编码器已重置 2=降级到软编 3=分辨率变更重建
    pub active_encoder: u8,    // 枚举见下
}
pub const ENC_QSV: u8 = 1; pub const ENC_NVENC: u8 = 2;
pub const ENC_AMF: u8 = 3; pub const ENC_V4L2M2M: u8 = 4;
pub const ENC_VIDEOTOOLBOX: u8 = 5; pub const ENC_OPENH264: u8 = 9;

// ---- 显示器（FR-03.3）----
pub struct DisplayInfo {
    pub count: u8,
    pub current: u8,
    pub entries: [DisplayEntry; 8], // 每项 { index u8, w u16, h u16, name_hash u16 }
}
pub struct DisplaySwitch { pub index: u8 }

// ---- 会话 ----
pub struct Ping { pub ts: u64 }
pub struct SessionClose { pub reason: u8 } // 1=用户 2=密码失效 3=编码器不可恢复 4=网络
pub struct Reconnect { pub reason: u8 }    // 1=断线重连 2=编码器全量重置
```

### 1.4 键码映射

不直接透传平台 scancode，使用统一 **LanDesk Key Code (LKC)** 表（0–255），两端各自映射到 `GetAsyncKeyState`/`x11/evdev`/CGEvent。修饰键与 LKC 分开发送，保证 Ctrl+组合键在跨平台下语义一致。

---

## 2. 视频报文（QUIC Datagram，Stream B）

### 2.1 设计约束

- 单 Datagram ≤ **1148B**（quinn 0.11 默认 `INITIAL_MTU=1200`；每包可发上限 =
  `current_mtu − 1rtt_overhead(≈22) − Datagram::SIZE_BOUND(9)` ≈ 1169，取 1148 留裕量，
  保证任何时刻单包都发得出、`send_datagram` 不报 `TooLarge`。MTU 探测收敛到 1450 时上限升至 ~1419）。
- H.264 裸 NALU 流，不用容器（FR-04.3）；SPS/PPS 在 I 帧前冗余发送，且每 5s 或重连后强制重发。
- 丢包策略：视频 Datagram 允许丢；**I 帧分片丢失 → 该 GOP 整段标记不可解，请求强制 I 帧**（控制流发 `EncodeParams{keyframe_now=1}` 语义复用 `bitrate_kbps=0xFFFF` 触发）。

### 2.2 报文结构

```
+----+----+-------+------+-------+--------+-------------+------------+
| mag| ver| seq  | kind | frag  | flags  | nalu_len   | nalu_data..|
+----+----+-------+------+-------+--------+-------------+------------+
 2B   1B  2B     1B    1B+   1B     2B(LE)     变长
```

| 字段 | 说明 |
|---|---|
| `mag` | 固定 `0x4C44`（"LD"），接收端识别非本协议包 |
| `ver` | 1 |
| `seq` | 帧序号 u16（65535 循环，配合 pts 消歧） |
| `kind` | NALU 类型：0=IDR+SPS/PPS，1=IDR，2=P 帧，3=SPS/PPS 单独重传 |
| `frag` | 高 7 bit=分片号，bit7=是否末片 |
| `flags` | bit0=首帧标志（解码器 flush 用），bit1=丢弃过期提示（>300ms 旧帧） |
| `nalu_len` | 本 Datagram 内 NALU 载荷长度（整 NALU 跨包时按 frag 拼接） |
| `nalu_data` | 实际 NAL 单元字节 |

- **整包（mag..gcm_tag）AES-GCM 加密**，nonce 按 0.1 的 Datagram 规则构造。
- 单 NALU 超过 1 包时按 frag 顺序切分，拼接缓冲超时 200ms 丢弃该 NALU。

### 2.3 接收端抖动缓冲（FR-05.5）

- 环形缓冲 8 帧 / 200ms 上限。
- `seq` 落后当前最大 seq 的帧直接丢弃（除非是 IDR）。
- 缓冲满时：丢最旧 P 帧；IDR 必保留。
- 丢包率统计 = 丢弃 NALU 数 / 期望 NALU 数（滑动 5s 窗口），上报状态栏（FR-09.3）并反馈 `ENCODE_PARAMS` 降码率。

---

## 3. 信令通道（外网，FR-10 / FR-11）

> 传输：`wss://` WebSocket over TLS；**只传信令，零画面流量**。
> 报文：envelope 为 JSON（可调试优先），字段紧凑。

### 3.1 消息类型

```jsonc
// C/S → 信令
{ "type": "reg",  "id": "dev-8f3a…", "name": "devboard-ok3568",
  "caps": ["qsv","openh264"], "res": "1920x1080", "relay": true }

// 控制端 → 信令
{ "type": "req",  "id": "dev-8f3a…", "from": "cli-77aa…" }

// 信令 → 双方（撮合后各自下发）
{ "type": "peer", "id": "dev-8f3a…", "peer": "cli-77aa…",
  "ice": [ /* 候选数组，见下 */ ] }

// ICE 候选增量交换（简化字段，FR：不引入完整 SDP）
{ "type": "ice", "from": "cli-77aa…", "to": "dev-8f3a…",
  "cand": { "t": "srflx", "ip": "1.2.3.4", "p": 4433, "pr": 100,
            "proto": "udp" } }

// P2P 探测超时后，信令撮合中继
{ "type": "relay", "to": "dev-8f3a…", "ep": "relay.example:4433",
  "ticket": "…32B base64(含中继会话密钥派生材料)…" }
```

| 字段 | 说明 |
|---|---|
| `cand.t` | host / srflx / prflx / relay（STUN 反射） |
| `cand.ip/p` | 候选端点；**信令侧只存内存**，无数据库（FR-10.2） |
| `relay.ticket` | 双方与中继 QUIC 连接时出示，中继验证后直接字节透传（中继不解密，会话密钥 E2E） |

### 3.2 STUN

- 使用标准 RFC 5389 绑定请求，属性仅支持 `CHANNEL`；无 TURN 凭据（MVP 中继为自建 QUIC 中继，非 TURN）。
- 信令服务可内嵌一个 STUN 反射器（同二进制 `-stun` 参数，UDP 4433），用户可指向自建，避免第三方 STUN（NFR-03）。

---

## 4. mDNS 发现协议（局域网，FR-01）

- 服务类型：`_landesk._tcp`，端口 `52000`（可配置）。
- TXT 记录（UTF-8，键值 `k=v`）：

| 键 | 值 | 必填 |
|---|---|---|
| `id` | 设备ID 16B → 12 位 hex | 是 |
| `name` | 设备名（≤32B） | 是 |
| `enc` | 编码器能力 csv：`qsv,nvenc,openh264` | 是 |
| `res` | `WxH` 当前主分辨率 | 是 |
| `ver` | 协议版本 `1` | 是 |
| `relay` | `1`/`0` 是否已注册自建中继 | 否 |

- 实例名 = `设备名@id后6位`，避免重名冲突；TTL 45s，控制端对 TTL 过期项自动移除（FR-01.4）。
- 控制端查询：PTR `_landesk._tcp.local.` + A/AAAA 解析实例名。
- 首次发现后直连 QUIC（UDP 52000），不走 HTTP。

---

## 5. 错误码与故障语义（FR-02.4 / NFR-01）

| 场景 | 行为 |
|---|---|
| 密码错误 | `AUTH_RESULT{code=1}` + 关连接，UI 明确提示 |
| P2P 打洞失败 | 信令 `relay` 流程；无中继时 `SESSION_CLOSE{reason=4}` 提示"无可用路径" |
| 编码器异常 | `ENCODE_EVENT{code=1}` 本地 close+reopen，**不断会话**；reopen 失败 → `code=2` 降级 OpenH264 并通知控制端 |
| 显示器插拔/分辨率变 | 采集层重启 + 编码器重建 + SPS/PPS 重发（`kind=3`），视频流 1s 内恢复 |
| QUIC 断连 | 指数退避 1s→2s→4s→8s，最多 5 次；期间保留剪贴板/状态 |

---

## 6. Rust 实现要点（骨架）

```rust
// crates/lp/（protocol 库）
pub mod wire {           // 帧编解码：serde-bins 风格手写 LE
    pub fn encode_control_frame<'a>(enc_key: &AesGcm, seq: u64,
        msg: &ControlMsg, out: &mut impl Write) -> Result<()>;
    pub fn decode_control_frame(dec_key: &AesGcm,
        buf: &[u8]) -> Result<(ControlMsg, u64)>;  // 校验 gcm_tag
}
#[repr(u8)]
pub enum MsgType { AuthReq = 0x01, AuthRes = 0x02, /* … */ }

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum ControlMsg {
    AuthReq(AuthRequest),
    AuthRes(AuthResult),
    Mouse(MouseMove),
    Key(KeyEvent),
    Clip(ClipMsg),
    Encode(EncodeParams),
    Display(DisplayInfo),
    PING(Ping),
    Close(SessionClose),
    Reconnect(Reconnect),
}

// crates/lp/datagram.rs
pub struct VideoDgram { pub magic: u16, pub ver: u8, pub seq: u16,
                        pub kind: u8, pub frag: u8, pub flags: u8,
                        pub nalu: Cow<'a, [u8]> }
impl VideoDgram {
    pub const MAX: usize = 1148;          // 单包上限（≤ quinn 默认可发 datagram 上限，见 §2.1）
    pub fn split(self, nalu: &[u8]) -> impl Iterator<Item = Vec<u8>>; // 分片
    pub fn needs_keyframe_request(&self) -> bool;
}
```

- 依赖：`x25519-dalek`、`aes-gcm`、`hkdf`、`sha2`、`pbkdf2`、`serde + bincode(LE)`、`quinn`、`mdns-sd`、`tokio-tungstenite`。
- 协议库 `lp` 独立 crate，单测覆盖：帧加解密往返、分片重组、抖动缓冲乱序、nonce 重放检测。

---

## 7. 演进预留（MVP 不做，占位不实现）

| 预留 | 说明 |
|---|---|
| `ver` major 升版 | 报文头已带 ver，向后兼容规则：接收端忽略未知 type 并 warn 日志 |
| 音频/文件传输 | 新增 Stream（QUIC 多流），type 段 0x80+ 预留给文件、0xA0+ 音频 |
| AV1/H.265 | `ENC_EVENT.active_encoder` 枚举扩展 + caps TXT 增 `av1`；NALU 头 kind 扩展 |
| TCP 备选通道 | QUIC 被防火墙拦截场景（风险表），信封格式可复用 |
| 组织/多账号 | 信令 `reg` 增 `org` 字段，MVP 忽略 |
