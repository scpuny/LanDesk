# LanDesk

局域网远程桌面工具（Remote Desktop over LAN）。Rust + Tauri + 裁剪 libav(H.264) + QUIC(Quinn) + ICE/STUN + mDNS 实现，目标是内网直连、外网 P2P 打洞、中继兜底的三段式连接，端到端加密（X25519 ECDH + AES-256-GCM），单文件二进制交付。

## 特性

- **三段连接路径**：内网 mDNS 发现直连 → 外网 ICE/STUN P2P 打洞 → 中继转发兜底
- **端到端加密**：X25519 ECDH 派生会话密钥，AES-256-GCM 加密全部控制流与视频流
- **低延迟视频**：QUIC 双通道（可靠控制流 + 不可靠视频 Datagram），H.264 编码，WebCodecs 解码渲染
- **完整远程控制**：键鼠注入、双向剪贴板同步、显示器切换、截图
- **自动重连**：掉线检测 + 指数退避重连（1/2/4/8/16s，最多 5 次），末帧保留
- **多形态交付**：Tauri GUI / 独立 CLI 二进制 / 浏览器 UI 服务（`ui-server`）

## 架构

Cargo Workspace 多 crate，按依赖方向单向分层：

```
ui (GUI 服务层, Tauri 壳 / ui-server 共用)
└─ landesk-core (会话编排: 状态机 + 采集→编码→QUIC 流水线 + 重连)
   ├─ mdns (局域网发现 _landesk._tcp)      ├─ ice (ICE/STUN NAT 打洞)
   ├─ quic-link (QUIC 双通道)             ├─ signaling (WS 信令客户端)
   ├─ avcodec-wrap (编码器 trait+降级链)  ├─ capture (屏幕采集)
   ├─ input (键鼠注入)                    ├─ clipboard (双向同步)
   └─ crypto (X25519/HKDF/AES-GCM/PBKDF2)
      └─ lp (协议帧编解码 / NALU 分片 / 抖动缓冲 — 纯逻辑，可全量单测)
```

关键原则：平台 API（采屏 / 键鼠注入 / 显示器事件）藏在 `capture` / `input` 的 feature-gated 实现后；`landesk-core` 是纯 tokio 库，无 GUI 依赖；`lp` 独立为纯函数协议库。

## 目录

```
crates/
  lp/               协议库：控制帧 / 视频 Datagram 编解码、NALU 分片、抖动缓冲
  crypto/           密钥与鉴权原语：X25519、HKDF、AES-256-GCM、PBKDF2
  avcodec-wrap/     统一编码器封装：trait + 降级链 + 异常重置（libav FFI 按 feature 门控）
  capture/          屏幕采集：trait + 脏矩形检测（平台实现 feature 门控）
  input/            键鼠注入：LKC 键码映射到 macOS CGEvent / Windows SendInput / Linux uinput
  quic-link/        QUIC 双通道：可靠控制流 + 不可靠视频 Datagram（Quinn 0.11）
  ice/              ICE/STUN NAT 打洞
  signaling/        轻信令客户端（tokio-tungstenite + JSON envelope）
  signaling-server/ 可选轻信令服务（axum + ws，内存存储）
  mdns/             局域网发现（mdns-sd，广播 / 监听 + TTL 剪枝）
  clipboard/        双向剪贴板同步
  landesk-core/     会话编排层（状态机 + 流水线 + 退避重连）
  ui/               GUI 服务层（状态栏映射 + 路径决策 + 配置持久化 + 设备列表 + SessionManager）
apps/
  controlled/       受控端（被远程访问的主机，QUIC 服务端 + mDNS 广播 + 视频发流）
  controller/       控制端（独立二进制骨架，主能力由 GUI 承担）
  relay-server/     可选 QUIC 中继（字节透传不解密）
  ui-server/        浏览器 UI 服务（axum 托管 Vue 前端 + REST，无需 Tauri CLI）
ui/
  frontend/         Vue 3 + Vite + TypeScript 前端（Tauri / 浏览器双宿主）
  src-tauri/        Tauri 壳（独立 [workspace]，离线 vendor）
docs/
  PRD.md  architecture.md  protocol.md  ui-backend-matrix.md  ui-redesign-prd.md  ops/
```

## 构建

本环境 cargo registry 解包会被系统 LSM 拦截（`futimens` EPERM），因此**统一走本地 `vendor/` 离线构建**（`.cargo/config.toml` 已把 `crates-io` 替换为 `vendor/` 目录源，`vendor/` 已入库）。所有 build/test 固定带 `--offline`。

> cargo 不在默认 PATH：`export PATH="$HOME/.cargo/bin:$PATH"`

### 离线门（提交前必须全绿）

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --offline          # 全仓单测
cargo clippy --offline --locked --all-targets -- -D warnings   # 全仓 0 warning
```

测试基线：主仓 210 通过 / 0 失败 / 2 忽略（`mdns` 环回需真网卡，`#[ignore]`）。

### 浏览器 UI 服务（形态 A 旁路，最快可跑）

无需 Tauri CLI，复用已 vendor 的 crate。

```bash
# 1. 先构建前端（产出 ui/frontend/dist，本二进制不依赖 Node 运行时）
cd ui/frontend && npm install --registry=https://registry.npmmirror.com && npm run build

# 2. 构建并启动 ui-server（默认 127.0.0.1:8080）
cd /path/to/LanDesk
export PATH="$HOME/.cargo/bin:$PATH"
cargo build -p ui-server --offline
./target/debug/ui-server --bind 127.0.0.1:8080
# 浏览器打开 http://127.0.0.1:8080
```

`ui-server` 暴露的 REST API（camelCase 与 Tauri invoke 约定一致）：

| 路由 | 说明 |
|------|------|
| `GET /api/devices` | 设备列表 |
| `GET/POST /api/config` | 配置读 / 写 |
| `GET /api/controlled-status` | 受控端状态 |
| `GET /api/local-info` | 本机信息（含当前临时码） |
| `POST /api/choose-path` | 连接路径决策 |
| `POST /api/reconnect` `GET /api/reconnect-schedule` | 重连计划 |
| `POST /api/session/connect` `/status` `/control` `/close` `/reconnect-now` | 会话命令面 |
| `GET /api/session/video` | 视频数据面（SSE 推帧） |
| `GET /api/logs` `POST /api/logs/clear` | 日志环形缓冲 |
| `GET /health` | 健康检查 |

### Tauri GUI 壳

`ui/src-tauri` 是独立 `[workspace]`（自带 `vendor/`，不入库），自成离线门：

```bash
cd ui/src-tauri
CARGO_NET_OFFLINE=true cargo build --features tauri
```

> 会话独立 OS 窗、自绘窗体等改动须在 `vite build` 之后重跑 `cargo build --features tauri`，才能烤进新 `dist`。

### 受控端 CLI

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo run -p controlled --offline   # 默认监听 0.0.0.0:52000
```

## 测试

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --offline                                   # 全仓
cargo test -p mdns -- --ignored                          # 真网卡环回（容器跳过）
cd ui/frontend && npm run typecheck                       # 前端 vue-tsc --noEmit
```

## 发布

`scripts/release/build-all.sh` 按平台跑 `cargo build --features tauri --release --locked`（含 OK3568 aarch64 交叉，见 `docs/ops/ok3568-build.md`）。

## 设计参考

根目录 `.gitignore` 已忽略散放的 `/*.png` 设计稿与误生成的 `/package-lock.json`；真实前端 lock 在 `ui/frontend/package-lock.json`（已跟踪）。

## License

MIT
