# GUI 兼作受控端 · 完整远程控制落地方案

> 目标：任意两端装了 LanDesk GUI 后，A 在设备列表点 B → 输 B 设置页的访问密码 → 看到 B 的**真实桌面**并能用**真键鼠**控制。不再需要单独跑 `controlled` CLI。
> 用户决策（2026-11-22）：完整远程控制 / 真编码用 openh264 源码编译 / 三平台（macOS+Windows+Linux）一起做 / 密码复用设置页访问密码。

## 1. 现状与缺口

控制端（发起方）链路已完整：

```
Session.vue 输密码 → session_connect(deviceId,password)
  → DeviceTable.find_online(id) 解析 ip:port
  → SessionManager.connect → landesk_core::Controller::connect
  → QUIC 拨号 + bootstrap 密码隧道 + ECDH 升级 → OneClickSession
  → 视频：recv_video_datagram → JitterBuffer → WebCodecs(H.264) 解码 → canvas
  → 键鼠：ControlAction → ControlMsg(MouseMove/Button/Wheel/KeyDown/Up) → QUIC 发送
```

受控端（被控方）四处缺口：

| # | 缺口 | 现状证据 | 本方案动作 |
|---|---|---|---|
| ① | GUI 不起 QUIC 服务端 | `Controlled` 只在 `apps/controlled` CLI；GUI `run()` 只起 Listener + Advertiser | 新增 `ControlledHost` 状态，GUI/ui-server 启动时按 `Config.password` 起 52000 监听 |
| ② | 画面是合成源 | `VideoPump` 用 `SynthCapture`，产带帧号彩条 | 三平台真实 `ScreenCapture` 实现 + 注入 VideoPump |
| ③ | 编码是占位假 H.264 | `PlaceholderEncoder` 只加 `00 00 00 01` 前缀，WebCodecs 解不出 | 引入 openh264 真编码后端，替换占位 |
| ④ | 键鼠注入断链 | `InputInjector` 仅 trait 无实现；`apply_input` 无生产调用者；受控端无控制帧接收循环 | 三平台 `InputInjector` 实现 + 受控端收帧循环调用 `apply_input` |

## 2. 硬约束（决定了实现路径）

- **离线 vendor**：`.cargo/config.toml` 把 crates.io 替换为本地 `vendor/`（231 crate），CI 也走 vendor 不联网。新增 crate 必须：下载 `.crate` → 重跑 `vendor.py` → 提交 vendor 与 lock。
- **可用 bindings**：vendor 有 `core-foundation`/`core-foundation-sys`、`windows-sys`（含 0.61.2）、`libc`。**无** `core-graphics`/`core-video`/openh264-rs/任意编码器。
- **openh264**：crates.io 镜像（tuna）有 crate 0.9.x，但它是 C 源码绑定的封装 → 需要 C 编译器 + nasm，三平台 CI 工具链要补齐； macOS 可用 VideoToolbox 替代但同样缺 bindings。
- **屏幕录制权限**：macOS 需 TCC 屏幕录制授权、Windows WGC 需前台/窗口权限、Linux 多为 X11/PipeWire 会话。
- **编译验证**：本机 Tauri 壳无 vendor 依赖无法本地编译 GUI bundler，但 `cargo test/clippy -p landesk-ui -p capture -p input -p avcodec-wrap` 可离线跑；CI 负责 GUI 壳全平台编译。

## 3. 分期与数据流

按依赖顺序，四期各自可独立编译验证，最后拼成完整闭环。

### 期一：服务端接入 GUI + 密码门禁（可离线验证）

**数据流**：
```
GUI 启动 → load Config → 若 password=Some(p)：
  ControlledHost::start(device_code, p, res, fps)
    → Controlled::new(0.0.0.0:52000, ["localhost"], device_id)
    → tokio spawn 循环：controlled.wait(p) → 得 OneClickSession
        → 起 VideoPump 发流（此期仍合成源）
        → 起控制帧接收任务（期四填充）
    → Advertiser 广播 52000（已有，端口与服务端一致）
前端：Settings.vue 访问密码 → config_save → Config.password（已通，此前是死字段）
```

**数据结构**（新增 `crates/ui/src/controlled_host.rs`）：
```rust
pub struct ControlledHost {
    device_id: String,
    password: String,
    server: Option<Controlled>,   // 保活，Drop 时 shutdown
}
impl ControlledHost {
    pub fn start(cfg: &Config) -> Result<Self, CoreError>;  // password=None 时不启动
    pub fn is_serving(&self) -> bool;
}
```
- Tauri `AppState` 加 `controlled: Option<ControlledHost>` 保活；ui-server 同理用局部变量。
- 密码来源 `Config.password`（用户已选）。注意：当前明文落盘与 NFR-03 矛盾，本期不改（另案），但会在文档标注。

**验收**：两端 GUI 设置同一密码，A 连 B → 握手成功、B 端日志「会话建立」、A 看到合成画面（带帧号彩条）。证明连通闭环。

### 期四（先于二三做，因其纯逻辑可离线验证）：键鼠注入

**数据流**：
```
受控端 session（wait 返回的 OneClickSession）
  → tokio spawn：loop { session.recv_control() → ControlFrame }
  → ControlMsg::{MouseMove,MouseButton,MouseWheel,KeyDown,KeyUp}
  → landesk_core::apply_input(&injector, msg)   // 现成逻辑，此前无调用者
  → platform InputInjector 注入
```

**数据结构**：
```rust
// crates/input/src/lib.rs 现有 trait，三平台各实现
#[cfg(target_os="macos")] pub struct CGEventInjector;   // core-foundation + libc FFI
#[cfg(target_os="windows")] pub struct SendInputInjector; // windows-sys 0.61
#[cfg(target_os="linux")] pub struct UinputInjector;      // libc ioctl /dev/uinput
```
- 坐标归一化 `denormalize`（已有）；按键码 `is_supported_lkc`（已有）。
- `ControlMsg → InputAction` 映射复用 `landesk_core::input::apply_input`。

**验收**：A 连 B 后，A 画面区移动鼠标 → B 桌面光标移动；A 点击/按键 → B 响应。三平台分别测。

### 期三：openh264 真编码

**数据流**：
```
ScreenCapture::grab → RawFrame(RGBA/BGRA) → Openh264Encoder::encode → H.264 Annex-B NALU
  → VideoDgram::fragment_nalu → session.send_video → A 端 WebCodecs 解码成功
```

**数据结构**：
```rust
// crates/avcodec-wrap/src/backends.rs 新增（feature=openh264）
pub struct Openh264Encoder { /* ISVCEncoder* 句柄 + 参数 */ }
impl VideoEncoder for Openh264Encoder { open/send_frame/recv_nalu/close }
```
- `VideoPump` 的 `encoder: PlaceholderEncoder` 改为泛型/枚举后端，按 `Config.preset` + feature 选择。
- openh264 通过 `cargo:rustc-link-lib` 链接；三平台 CI 装 nasm + C 工具链（`apt install nasm`、`brew install nasm`、Windows 用预编 dll）。

**验收**：抓一帧真实画面 → 编码 → A 端 WebCodecs 能解码并显示（非彩条）。

### 期二：三平台真实采屏

**数据流**：
```
平台 ScreenCapture::grab → RawFrame → (期三)编码 → 发流
```

**数据结构**：
```rust
// crates/capture/src/ 下对应平台文件
#[cfg(target_os="macos")] CGDisplayCapture   // CGDisplayStreamCreate + IOSurface → BGRA
#[cfg(target_os="windows")] WgcCapture        // Windows.Graphics.Capture + Direct3D → BGRA
#[cfg(target_os="linux")] X11Capture / PipeWireCapture  // XGetImage 或 PipeWire frame → BGRA
```
- 统一 `RawFrame { data: Vec<u8>, width, height, format: PixelFormat }`（已有），新增平台实现填入真像素。
- `VideoPump` 的 `capture: SynthCapture` 改为按平台选真实采集器。

**验收**：A 连 B 后看到 B 的实时桌面（窗口移动、鼠标可见），非合成彩条。

## 4. 前端视图变更

- `Settings.vue`：访问密码输入框**已经存在**（`v-model=config.password`），本期让它真正生效（后端起服务端读取它）。补一行说明「设置密码后本机可被局域网内其他设备远程控制」。
- `Remote.vue` / `DeviceList.vue`：无需改动，连接流程不变。
- `Session.vue`：无需改动，WebCodecs 解码链路不变。
- 新增状态提示（可选）：设置页显示「受控服务：已启用/未启用（未设密码）」。

## 5. 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| openh264 C 编译三平台 | 期三最大风险，CI 可能挂 | 先在本机 macOS 编通；CI 每平台单独装 nasm；失败则用系统库 fallback |
| macOS 屏幕录制权限 | 采屏黑屏 | 首次启动检测并引导系统设置；TCC 提示 |
| Windows WGC 权限 | 采屏失败 | 降级 BitBlt/GDI 抓屏（windows-sys 足够） |
| Linux 无显示服务 | headless CI 采屏失败 | 平台实现仅在有会话时起，CI 跳过采屏测试 |
| 密码明文落盘 | 与 NFR-03 矛盾（已存在） | 本方案不扩大；另案改派生值持久化 |
| 端口占用 52000 | 服务端起不来 | start 失败降级为纯控制端，日志告警，不影响发起连接 |

## 6. 执行顺序与提交策略

1. 期一：服务端接入 + 密码门禁 → 本地 commit
2. 期四：键鼠注入三平台 → 本地 commit
3. 期三：openh264 编码 + vendor 变更 → 本地 commit
4. 期二：三平台采屏 → 本地 commit
5. 全量测试验证 → 按用户授权决定是否发版

每期结束跑 `cargo test` + `cargo clippy -p <crates> --all-targets -D warnings`；Tauri 壳全平台编译交给 CI。
所有提交默认**本地 commit，不推送**，等用户明确授权再走 Git Flow 发版。

## 7. 待确认的关键点

- 密码为空（`Config.password=None`）时：默认**不启动**受控服务端（安全，保持纯控制端行为）。是否认可？
- openh264 引入会改 vendor/CI（新增 C 编译链），是本方案最大改动面，是否认可？
- 三平台原生代码无法在本机全部实测（Windows/Linux 靠 CI 编译 + 用户侧验证），是否接受「编译通过 + macOS 实测 + 其余平台待用户验证」的交付标准？
