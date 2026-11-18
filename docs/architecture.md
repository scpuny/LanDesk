# LanDesk 技术架构文档

> 配套文档：`docs/PRD.md`（V0.1 MVP）、`docs/protocol.md`（内部协议规范）
> 版本：V0.1 | 状态：草稿
> 技术栈：Rust + Tauri + 裁剪 libav(H.264) + Quinn(QUIC) + ICE/STUN + mDNS
> 目标平台：Windows 10/11、Linux(Debian/Ubuntu)、macOS、OK3568(aarch64)

---

## 1. 架构总览

LanDesk 采用 **Cargo Workspace 多 crate** 组织，按依赖方向分层，编译产物有两种形态：

```
                    ┌─────────────────────────────────────────┐
                    │  Tauri 应用 (landesk-ui)                │  形态A：Lan Studio 内置模块 / 独立 GUI
                    │  - 设备列表 / 连接管理 / 预览窗口        │
                    │  - 配置面板 / 状态栏                     │
                    └──────────────────┬──────────────────────┘
                                       │ tauri 命令桥（invoke）
┌──────────────────────────────────────┴──────────────────────────────────────┐
│                     核心运行层 (landesk-core, 无 GUI 依赖)                  │
│                                                                            │
│  ┌──────────┐  ┌──────────┐  ┌──────────────┐  ┌──────────┐  ┌─────────┐ │
│  │ 发现     │  │ 会话管理  │  │ 输入注入     │  │ 编码器   │  │ 剪贴板  │ │
│  │ mdns/信令│  │ auth/    │  │ 键盘/鼠标    │  │ libav    │  │ 双向同步│ │
│  │ ICE/STUN│  │ 重连     │  │              │  │ 降级链   │  │         │ │
│  └──────────┘  └──────────┘  └──────────────┘  └──────────┘  └─────────┘ │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │
┌──────────────────────────────────────┴──────────────────────────────────────┐
│                         基础层（纯逻辑，可单测，无平台绑定）                 │
│   lp(协议编码/解密)  quic-link(QUIC双通道)  avcodec-wrap(libav封装)        │
│   capture(采集trait+平台实现)  ice(ICE/STUN)  crypto(密钥派生)              │
└─────────────────────────────────────────────────────────────────────────────┘
```

关键原则：

1. **平台隔离**：所有平台 API（屏幕采集、键鼠注入、显示器事件）藏在 `capture`/`input` crate 的 feature-gated 实现后，上层只见 trait；
2. **核心无 GUI 依赖**：`landesk-core` 是纯 tokio 库，CLI 与 Tauri 两个壳共用，保证"独立单文件二进制"目标；
3. **协议独立 crate**（`lp`）：加解密、分片、抖动缓冲纯函数化，单测可全量覆盖（M2 前先落地）；
4. **依赖单向**：ui → core → 基础层，禁止反向引用；基础层之间允许（ice 依赖 crypto、capture 依赖 avcodec-wrap 的像素类型）。

---

## 2. 目录结构

```
LanDesk/
├── Cargo.toml                      # workspace 根：members + workspace.dependencies 统一版本
├── rust-toolchain.toml             # 锁定 stable 1.8x；aarch64 交叉用 rust-std 组件
├── .cargo/config.toml              # target/aarch64-unknown-linux-gnu 交叉链接器(Cargo 层，OK3568 工具链)
├── cross.toml                     # cross 容器配置（aarch64 编译用，避免手装 SDK）
│
├── crates/
│   ├── lp/                        # ★ 协议库（最先落地，M1~M2 基础）
│   │   ├── src/
│   │   │   ├── lib.rs             # 公开 API 门面
│   │   │   ├── wire.rs            # 帧编码/解码（LE、gcm_tag 校验）
│   │   │   ├── msg.rs             # ControlMsg 枚举 + 各载荷结构体（对齐 protocol.md §1.3）
│   │   │   ├── datagram.rs        # VideoDgram：分片/重组/needs_keyframe_request
│   │   │   ├── jitter.rs          # 抖动缓冲（环形 8 帧/200ms，乱序丢弃）
│   │   │   ├── clipboard.rs       # 剪贴板分段/重组 + 1MB 上限
│   │   │   ├── keycodes.rs        # LKC 统一键码表 + 平台映射函数声明
│   │   │   └── error.rs           # LpError：解码失败/GCM tag 错/nonce 重放
│   │   └── tests/
│   │       ├── frame_roundtrip.rs # 帧加解密往返
│   │       ├── dgram_reassembly.rs# NALU 分片重组 + 乱序
│   │       └── nonce_replay.rs    # nonce 重放检测
│   │
│   ├── crypto/                    # 密钥与鉴权原语
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── ecdh.rs            # X25519 密钥对/共享密钥（x25519-dalek）
│   │   │   ├── kdf.rs             # HKDF-SHA256 → master_key → enc_key（protocol.md §0.1）
│   │   │   ├── gcm.rs             # AES-256-GCM 封装 + nonce(rand8||seq) 生成
│   │   │   └── password.rs        # PBKDF2(100k) 派生值存取 + OTP 一次性密码
│   │   └── tests/password.rs
│   │
│   ├── avcodec-wrap/             # libav 统一编码器封装（核心稳定模块，FR-04）
│   │   ├── build.rs             # 探测 vendored 库路径（LAVF_ROOT 环境变量）
│   │   ├── src/
│   │   │   ├── lib.rs            # pub trait VideoEncoder { open/send_frame/recv_nalu/close }
│   │   │   ├── factory.rs       # ★ 降级链：按平台 caps 依次尝试，失败自动下一档
│   │   │   ├── qsv.rs           # h264_qsv（Windows/Linux）
│   │   │   ├── nvenc.rs         # h264_nvenc（NVENC）
│   │   │   ├── amf.rs           # h264_amf（Windows）
│   │   │   ├── v4l2m2m.rs      # h264_v4l2m2m（Linux PC / OK3568）
│   │   │   ├── videotoolbox.rs  # h264_videotoolbox（macOS）
│   │   │   ├── openh264.rs      # OpenH264 软编兜底（独立库，不走 libav）
│   │   │   ├── swscale.rs       # RGB→NV12 色彩转换（FR-04.4）
│   │   │   └── reset.rs         # 异常 close+reopen + 状态机（FR-04.6）
│   │   └── tests/encoder_chain.rs # 模拟驱动失败验证降级顺序
│   │
│   ├── capture/                   # 屏幕采集（FR-03）
│   │   ├── src/
│   │   │   ├── lib.rs            # trait ScreenCapture + DirtyRect 检测
│   │   │   ├── dirty.rs          # 脏矩形对比（可开关降级全帧，风险表#3）
│   │   │   ├── monitors.rs       # 多显示器枚举/切换/热插拔事件（FR-03.3/03.4）
│   │   │   └── impl/
│   │   │       ├── win.rs        # Graphics Capture (WGC, feature = "windows")
│   │   │       ├── linux.rs      # PipeWire (feature = "linux")
│   │   │       └── macos.rs      # CGDisplayStream (feature = "macos")
│   │   └── tests/dirty.rs
│   │
│   ├── input/                    # 键鼠注入（受控端，FR-06）
│   │   ├── src/
│   │   │   ├── lib.rs            # trait InputInjector + LKC → 平台码映射
│   │   │   ├── win.rs            # SendInput
│   │   │   ├── linux.rs         # /dev/uinput + evdev
│   │   │   └── macos.rs         # CGEvent
│   │   └── tests/
│   │
│   ├── quic-link/               # QUIC 双通道（FR-05）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── endpoint.rs       # Quinn endpoint 封装（0.0.0.0 双栈，UDP 52000）
│   │   │   ├── streams.rs        # Stream A 可靠流：控制报文 编解码 + 心跳/PING
│   │   │   ├── video.rs          # Stream B Datagram：发送窗口 + 丢包策略
│   │   │   ├── relay.rs          # 中继模式：字节透传代理（信令 ticket 出示）
│   │   │   └── stats.rs          # RTT/丢包/带宽估计（喂给编码器动态码率，FR-04.5）
│   │   └── tests/loopback.rs     # 本机回环双通道联调（M3）
│   │
│   ├── ice/                     # ICE/STUN 打洞（FR-05.2）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── candidate.rs      # 候选收集：host/srflx/prflx + 简化字段序列化
│   │   │   ├── stun.rs           # RFC 5389 绑定请求（自实现，无第三方 STUN 依赖）
│   │   │   └── hole.rs         # 打洞调度：并发连接尝试 + 对称 NAT 探测
│   │   └── tests/
│   │
│   ├── signaling/               # 轻信令客户端（FR-10）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── ws.rs             # tokio-tungstenite wss 连接 + 重连
│   │   │   ├── message.rs        # reg/req/peer/ice/relay JSON 消息（protocol.md §3）
│   │   │   └── register.rs       # 上线注册/离线注销 + TTL 心跳
│   │   └── tests/
│   │
│   ├── mdns/                    # 局域网发现（FR-01，复用 Lan Studio 的 mdns-sd 经验）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── publisher.rs      # 广播自身 TXT（id/name/enc/res/ver），TTL 45s
│   │   │   └── browser.rs       # 监听 _landesk._tcp + TTL 过期移除（FR-01.4）
│   │   └── tests/
│   │
│   ├── clipboard/               # 双向剪贴板同步（FR-07）
│   │   ├── src/
│   │   │   ├── lib.rs            # watch(本端变化→发对端) + apply(对端数据→写剪贴板)
│   │   │   └── backring.rs       # 回环保护：本端写入不触发再次同步（400ms 抑制窗）
│   │   └── tests/
│   │
│   ├── landesk-core/            # ★ 会话编排层（无 GUI，tokio）
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── session.rs        # 会话状态机：Idle→Auth→Running→Reconnecting→Closed
│   │   │   ├── orchestrator.rs  # 采集→脏矩形→编码→QUIC 流水线 + 背压
│   │   │   ├── control.rs        # 控制流事件循环（键盘/鼠标/剪贴板/参数）
│   │   │   ├── reconnect.rs      # 指数退避重连（1/2/4/8s ×5，FR-05.4）
│   │   │   ├── events.rs         # 对外事件通道：连接方式/编码器/丢包/延迟（喂 UI 状态栏）
│   │   │   ├── config.rs         # 配置模型（密码/信令地址/STUN/编码预设/日志级别）
│   │   │   └── role.rs           # Controlled / Controller 双角色入口（同二进制）
│   │   └── tests/session_sm.rs
│   │
│   └── signaling-server/       # 可选组件：轻信令服务（FR-10，独立二进制）
│       ├── src/main.rs          # 单文件部署：axum + ws，内存 HashMap 存在线设备
│       └── README.md
│
├── apps/
│   ├── controlled/              # 形态A壳：受控端 CLI/服务（单文件二进制）
│   │   └── src/main.rs          # clap：--role controlled|controller、--config 路径
│   ├── controller/             # 形态A壳：控制端 CLI
│   │   └── src/main.rs
│   └── relay-server/           # 可选组件：QUIC 中继（FR-11，独立二进制）
│       └── src/main.rs
│
├── ui/                          # 形态B：Tauri 前端（Lan Studio 模块 / 独立 GUI）
│   ├── src-tauri/
│   │   ├── tauri.conf.json     # bundle 配置（NSIS/deb/dmg/AppImage）
│   │   ├── Cargo.toml          # 依赖 landesk-core，feature 平台采集
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── commands/        # tauri 命令桥：connect/disconnect/switch_display/…
│   │       └── state.rs        # 共享 core 事件通道
│   └── frontend/               # Vue3 + Arco Design（与团队技术栈一致）
│       └── src/
│           ├── views/
│           │   ├── DeviceList.vue    # 局域网设备列表（FR-01.2）
│           │   ├── Session.vue       # 预览窗口（FR-09）：缩放/全屏/状态栏
│           │   ├── Settings.vue      # 受控端配置面板（FR-08）
│           │   └── Logs.vue          # 日志/编码器日志查看（FR-08.4）
│           ├── components/StatusBar.vue # 延迟/编码类型/连接方式/丢包率（FR-09.3）
│           └── lib/landesk.js        # 桥接封装
│
├── scripts/
│   ├── build-libav/
│   │   ├── build-x86_64.sh       # 裁剪 ffmpeg：avcodec/avutil/swscale + qsv/nvenc/amf/v4l2m2m
│   │   ├── build-aarch64-ok3568.sh # v4l2m2m + 软编；OK3568 交叉工具链
│   │   └── build-openh264.sh     # OpenH264 独立编译（x86_64/aarch64）
│   ├── cross-ok3568/
│   │   └── Dockerfile            # aarch64 交叉编译镜像（含 sysroot 指向裁剪 libav）
│   ├── release/
│   │   ├── build-all.sh          # 四平台产物 + 签名配置
│   │   └── stable-72h.sh        # NFR-01 稳定性压测脚本（长跑 + 句柄/内存采样）
│   └── test-e2e/
│       └── scenarios.md          # 内网/P2P/中继/异常场景 手测用例清单（验收标准 §9 对应）
│
├── docs/
│   ├── PRD.md
│   ├── protocol.md
│   ├── architecture.md           # 本文档
│   └── ops/
│       ├── signaling-deploy.md   # 信令/中继 部署说明（单文件二进制）
│       └── ok3568-build.md       # OK3568 交叉编译步骤
│
└── .github/workflows/
    ├── ci.yml                   # 三平台矩阵 lint/clippy/test + libav 缓存
    └── release.yml              # tag 触发四平台打包
```

---

## 3. 模块职责与依赖关系

| crate | 职责 | 直接依赖（内部） | 关键外部依赖 |
|---|---|---|---|
| `lp` | 帧编解码、NALU分片、抖动缓冲 | — | serde/bincode、aes-gcm、sha2 |
| `crypto` | ECDH/HKDF/GCM/PBKDF2/OTP | — | x25519-dalek、hkdf、pbkdf2、ring |
| `avcodec-wrap` | 编码器 trait + 降级链 + 重置 | —（仅像素类型自持） | 裁剪 libav（FFI）、openh264-sys |
| `capture` | 屏幕采集 + 脏矩形 + 显示器事件 | — | wgc-win / pipewire / CGDisplayStream（feature 门控） |
| `input` | 键鼠注入 + LKC 映射 | `lp`(keycodes) | SendInput / uinput / CGEvent（feature 门控） |
| `quic-link` | QUIC 双通道、中继、统计 | `lp`、`crypto` | quinn、tokio |
| `ice` | 候选收集/STUN/打洞 | `crypto`(报文签名) | 自实现 STUN，无第三方 |
| `signaling` | 信令客户端 | `ice` | tokio-tungstenite |
| `mdns` | 局域网发现 | — | mdns-sd |
| `clipboard` | 双向剪贴板 | `lp` | arboard |
| `landesk-core` | 会话编排（全部流水线） | 上述全部 | tokio、tracing |
| `signaling-server` | 信令服务（独立二进制） | `ice`(消息结构共享) | axum |
| `apps/*` / `ui/*` | 壳层 | `landesk-core` | clap / tauri |

依赖方向（编译期强制，可用 cargo-workspace 拓扑 + `forbid` lint 检查）：

```
ui, apps ─→ landesk-core ─→ quic-link / ice / signaling / mdns / clipboard
                              │          │
                              └──→ lp ──┘
capture, avcodec-wrap, input, crypto, lp ：基础层，互不引用（lp 独立）
```

---

## 4. 运行时数据流（受控端视角）

```
屏幕采集(capture)
  │ RawFrame + DirtyRects（每帧 vs 上帧，可关闭）
  ▼
色彩转换(avcodec-wrap/swscale)
  │ NV12
  ▼
编码(avcodec-wrap/factory)
  │ 裸 NALU（I/P）← 码率反馈(quic-link/stats RTT/丢包，FR-04.5)
  ▼
分片(lp/datagram) + 加密(crypto/gcm)
  ▼
QUIC Datagram(Stream B) ──→ 对端
控制流(Stream A)：键鼠/剪贴板/参数/心跳（lp 帧 + GCM）
```

### 4.1 会话状态机（landesk-core/session）

```
Idle ──auth ok──▶ Running ──quic 断──▶ Reconnecting(退避×5)
  │                   │  ▲                  │ 成功
  │ 信令/ICE 失败     │  └──────────────────┘
  ▼                   ▼
AuthFailed / NoPath  Closed(reason)
Running 中事件：ENC_RESET(编码器重建,会话保持) / RESOLUTION_CHANGE(重建+SPS重发)
```

### 4.2 异常恢复矩阵（对齐 NFR-01）

| 触发 | 检测点 | 动作 | 会话影响 |
|---|---|---|---|
| 显卡休眠/驱动重置 | 编码器 send_frame 报错 | avcodec-wrap/reset：close+reopen 同档 | 保持，1~2s 花屏窗口 |
| reopen 失败 | factory 同档失败 3 次 | 降级下一档（最终 OpenH264）+ `ENCODE_EVENT{code=2}` | 保持 |
| 显示器插拔/分辨率变 | capture/monitors 事件 | 采集重启 + 编码器重建 + SPS/PPS `kind=3` 重发 | 保持 |
| QUIC 断连 | endpoint 事件 | Reconnecting 指数退避；重连后强制 IDR | 最多 5 次后 NoPath |

---

## 5. 配置与可观测性（FR-08 / NFR-06）

### 5.1 配置文件

- 路径：受控端 `~/.config/landesk/config.toml`（首次生成默认模板）；
- 内容：访问密码（仅存 PBKDF2 派生值 + 校验用 pepper 摘要，**不存明文**）、设备名、mDNS 开关、信令/STUN/中继地址、编码预设（流畅/平衡/高清 → 码率表：1.5M/4M/8M@1080p）、日志级别、脏矩形开关。
- CLI 参数可覆盖配置项（`--bitrate`、`--log-level`）。

### 5.2 日志

- `tracing` 分级 debug/info/warn/error；
- 结构化字段：`conn_mode(inner|p2p|relay)`、`encoder(qsv|nvenc|…)`、`bitrate`、`loss_pct`、`reset_events`；
- 编码器日志单独落 `landesk-encoder.log`（排硬编驱动问题，NFR-06）；
- UI Logs 页（FR-08.4）直接读文件流。

---

## 6. 平台构建策略

| 平台 | 采集 | 注入 | 硬编 | 备注 |
|---|---|---|---|---|
| Windows | Graphics Capture（需 Win10 1903+；降级 GDI） | SendInput | qsv→nvenc→amf→openh264 | NSIS 安装包 |
| Linux PC | PipeWire（Debian 12+/Ubuntu 22.04+；无则 V4L2 buffer 兜底） | /dev/uinput | qsv→nvenc→v4l2m2m→openh264 | deb + AppImage |
| macOS | CGDisplayStream | CGEvent | videotoolbox→openh264 | dmg，需屏幕录制权限引导 |
| OK3568 | 平台 V4L2/DRM | /dev/uinput | v4l2m2m→openh264 | aarch64 静态链接单文件；cross 交叉编译 |

交叉编译（OK3568）：`cross` 容器 + 裁剪 libav aarch64 产物（`LAVF_ROOT` 指向），静态链接 `mold`/`lld`，产物 ~15MB 量级（NFR-05）。

CI：三平台矩阵（ubuntu / windows / macos）`cargo clippy + test`，aarch64 仅编译检查（不跑测试，OK3568 真机 M9 阶段跑 `stable-72h.sh`）。

---

## 7. 里程碑到模块映射（对齐 PRD §8）

| 里程碑 | 涉及 crate | 验收点 |
|---|---|---|
| M1 | `capture`、`lp`(像素类型) | 本机采集出帧 + 脏矩形日志可见 |
| M2 | `avcodec-wrap`、`crypto` | 编码器降级链单测通过（模拟 qsv 缺失→openh264） |
| M3 | `quic-link`、`lp`(dgram) | 本地回环：编码→QUIC→解码渲染跑通 |
| M4 | `mdns`、`landesk-core` | 双机内网一键连接，键鼠可用 |
| M5 | `input`、`clipboard`、`crypto`(auth) | 鉴权 + 双向剪贴板 + 注入全链路 |
| M6 | `ice`、`signaling`、`signaling-server` | P2P 打洞 + 信令注册联通 |
| M7 | `quic-link/relay`、`apps/relay-server`、`landesk-core/reconnect` | 中继兜底 + 异常恢复矩阵全部场景 |
| M8 | `ui`、`landesk-core/events` | GUI 全流程 + 状态栏指标 |
| M9 | `scripts/*`、CI | 四平台产物 + OK3568 真机 + 72h 压测 |

---

## 8. 关键设计决策记录（ADR 摘要）

1. **A1 协议 crate 先行**：`lp` 无 tokio/平台依赖，纯函数可全测 —— 对应 M2"先单独单测编码器/协议"的思路，把风险前移；
2. **A2 信令用 WebSocket+JSON，数据面纯 QUIC**：信令低频（reg/ice/relay 每条 <1KB），可调试性优先；数据面 QUIC Datagram 天然支持不可靠视频 + 可靠控制流双语义；
3. **A3 中继只做字节透传**：不解析协议、不解密、不存会话状态 —— 攻击面最小，符合"NFR-03 自托管无第三方"；
4. **A4 OpenH264 独立于裁剪 libav**：libav 只保留硬编通路 + swscale，软编走独立 `openh264-sys`，降低裁剪面与交叉编译复杂度（风险表#4）；
5. **A5 GUI 用 Vue3+Arco 而非 Tauri 官方模板 React**：与团队既有栈（Hermes-EAM 前端）一致，降低维护面；
6. **A6 双角色同二进制**：`--role controlled|controller`，受控端与中继/信令仍是独立小二进制（FR-10/11 可独立开关）。
