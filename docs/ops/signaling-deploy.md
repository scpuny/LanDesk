# 信令 / 中继服务部署说明

> 对应 `docs/PRD.md` FR-10 / FR-11。两个组件均为**独立单文件二进制**，可分机部署。

## 1. 轻信令服务（signaling-server）

### 职责边界（FR-10.2，务必守住）
- 只做三件事：**注册撮合、ICE 候选中继、中继 ticket 发放**；
- **零画面流量**（视频 QUIC 流量绝不过此服务）；
- **无数据库**：在线设备表存内存 `HashMap<device_id, PeerConn>`，进程重启即清空（MVP 可接受，重启后受控端重新注册）；
- 仅 WebSocket over TLS，对外暴露 1 个端口（建议 `:8443`）。

### 构建
```bash
# Linux x86_64
cargo build --release -p signaling-server
# 产物：target/release/signaling-server（单文件，无运行时依赖）
```

### 部署（systemd 示例）
```ini
# /etc/systemd/system/landesk-signal.service
[Unit]
Description=LanDesk lightweight signaling
After=network.target

[Service]
ExecStart=/opt/landesk/signaling-server --bind 0.0.0.0:8443 --cert /etc/landesk/fullchain.pem --key /etc/landesk/privkey.pem
Restart=always
# 无状态服务，内存设备表重启即丢，无需状态目录
StandardOutput=journal

[Install]
WantedBy=multi-user.target
```

### 与 STUN 的关系
- 信令可附带 `-stun` 参数内嵌一个 STUN 反射器（UDP `:4433`），用户把 `stun_url` 指向自建即可满足 NFR-03（无第三方 STUN）。
- 也可独立部署 `signaling-server -stun --bind 0.0.0.0:8443 --stun-bind 0.0.0.0:4433`。

## 2. 中继服务（relay-server）

### 职责边界（FR-11 / ADR-A3）
- **只做 QUIC 字节透传**：验证信令发放的 ticket 后，把 A↔B 的 QUIC 包原样搬运；
- **不解密、不解析协议、不存会话状态**——攻击面最小化；
- P2P 打洞成功时本服务流量为零（纯兜底路径）；
- 带宽与连接数需按预期并发预估，中继会消耗出口带宽。

### 构建与部署
```bash
cargo build --release -p relay-server
# relay-server --bind 0.0.0.0:8444 --cert ... --key ...
```

### 用户侧开关（FR-11.2）
- 受控端配置 `relay_enabled=false` 时，打洞失败直接报「无可用路径」，不连中继。
- 不部署中继服务的用户，信令侧 `relay` 消息不发放 ticket。

## 3. 网络与防火墙

| 服务 | 协议 | 端口（建议） | 说明 |
|---|---|---|---|
| 信令 | WSS | 8443/tcp | 需 TLS 证书；信令带宽极低 |
| STUN | UDP | 4433 | 可选内嵌 |
| 中继 | QUIC(UDP) | 8444/udp | 按 P2P 成功率决定是否部署 |
| 受控端 | QUIC(UDP) | 52000 | 内网直连/打洞目标端口 |

> QUIC 走 UDP，若目标网络防火墙封 UDP/QUIC（风险表#5），MVP 无 TCP 兜底（已列入演进预留）。
