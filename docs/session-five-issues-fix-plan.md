# 会话五问题修复落地方案（顶栏 / 画面 / 重连 / 防火墙 / 跨平台黑屏）

> 目标：修复用户 v0.5.5 实测的五个运行期问题。用户决策（2026-11-23）：先出方案、确认后再写代码；画面问题聚焦「红块/条纹是合成测试图，要看真实桌面」。
>
> 现状版本：main `3a24524`（v0.5.5）。本方案基于该版本，工作区干净。

## 0. 五问题一览与结论

| # | 用户现象 | 根因 | 修复层面 |
|---|---|---|---|
| 1 | macOS 会话窗左侧红绿灯把右侧工具顶出边界 | `isMac` 异步判定 + 顶栏 flex 无收缩策略 | 前端 CSS + 平台判定同步化 |
| 2 | Windows 连 macOS 画面是「灰条纹 + 红方块」测试图 | **macOS 被控端采屏失败悄悄降级为合成源**，用户看的是 `SynthCapture` 测试图，不是桌面 | 被控端采屏降级检测 + 明示提示 |
| 3 | 关闭会话窗后再连接，打开窗体显示「选择设备」白屏 | 重开时 `session_launch` 为空 / 目标丢失，`SessionApp` 未拿到 target | 后端保留上次目标 + 前端守卫 |
| 4 | macOS 防火墙打开后，Windows 连不上 macOS | QUIC 受控端监听 `0.0.0.0:52000`，被 macOS 应用防火墙拦入站（无授权提示/例外引导） | 防火墙检测 + 用户引导 / 端口策略 |
| 5 | macOS 连 Windows 一片黑 | Windows GDI `BitBlt` 在锁屏/无交互会话下返回 Ok(全黑帧)，被控端无黑帧检测→把黑帧编码发出；控制端解出黑画面（与问题 2 同源但表现不同：macOS 未授权降级合成图，Windows 抓黑帧不降级） | 同问题 2 层 1：黑帧检测 + 降级/提示 |

关键判断（问题 2）：状态栏的 `18ms / 丢包 0% / 4.2Mbps / 重置 ×0` **是前端演示默认值，不是真实码流**。后端定义了 `EncoderChanged/LatencyMs/LossPct/BitrateKbps/EncoderReset` 事件（`crates/ui/src/events.rs`），但 `SessionManager` 只 apply 了 `StateChanged/ConnModeChanged`（`manager.rs:247-248`），这些指标从未写入状态。真实有效数据只有 `8fps / 0KB/s`——8 帧/秒要 round 成 0KB/s，需每帧 NALU < 64 字节，真实桌面 H.264 不可能这么小，**只有近乎纯色的合成/黑帧才会这样**。第二张截图的红方块条纹底正是 `SynthCapture` 的固定测试图案（`crates/capture/src/lib.rs:228`「红色块水平移动 + 左上角烧录 seq」）。

证据链完整：传输链路（分片/重组/解码）本身是自洽的，问题在被控端画面源，不在 QUIC 或 WebCodecs。

---

## 1. 问题 1：macOS 顶栏布局

### 1.1 现状证据
- 会话窗在 macOS 用 `decorations:true + titleBarStyle:Overlay + hiddenTitle`（`ui/src-tauri/src/lib.rs:428-434`），保留原生红绿灯；Win/Linux 用 `decorations:false` 自绘三键。
- 前端靠 `isMac` 加 `.ld-sess-top--mac { padding-left: 80px }` 避让（`app.css:363`），`isMac` 来自 `useWindowControls()`。
- **缺陷 A（根因）**：`isMac` 由 `landesk.platform()` **异步**解析，ref 初值恒为 `'linux'`（`useWindowControls.ts:20`）。首帧按无红绿灯布局渲染，平台返回后补 80px 留白→布局跳动；若窗口恰好较窄，右侧工具首次即被挤出。
- **缺陷 B（放大）**：顶栏 `.ld-sess-top` 是 `display:flex; gap`，子项无 `min-width:0 / flex-shrink`，目标名变长（如 `DESKTOP-A4FPIPS` + `内网直连` + 分辨率）时，右侧 tools 无收缩空间被顶出边界。

### 1.2 改动点
**前端 `ui/frontend/src/composables/useWindowControls.ts`**
- 平台判定同步化：在模块加载时同步读一次宿主平台。Tauri 桌面轨用编译期常量注入（在 `vite.config` define `__PLATFORM__`）或壳启动时随首帧下发；最小改动方案——用 `navigator.userAgent` 同步判定 macOS（`Macintosh`），桌面 Tauri 的 WebView UA 同样含该标记，浏览器预览亦准确；`landesk.platform()` 异步结果仅作二次校正。

**前端 `ui/frontend/src/styles/app.css`**
- `.ld-sess-top` 增强溢出收缩：中间 `.ld-sess-target { flex: 1 1 auto; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap }`，`.ld-sess-tools { flex: none }`，`.ld-sess-state { flex: none }`。
- macOS 留白从固定 80px 改为 token 变量 `--ld-titlebar-traffic-w: 80px`，`.ld-sess-top--mac { padding-left: var(--ld-titlebar-traffic-w) }`，便于统一调整。
- 加窄窗 media query（窗口可缩到 720px）：≤900px 时隐藏分辨率标签或缩短目标名。

### 1.3 验证
- 本地 `npm run typecheck` + `vite build`；
- 三平台窗口各开会话窗截图核对：macOS 红绿灯不与「运行中」重叠、右侧「断开连接」完整可见；Win/Linux 自绘三键不被挤出；窗口缩到最小宽 720px 不溢出。

---

## 2. 问题 2：画面是合成测试图（核心）

### 2.1 现状证据
- `VideoPump::new_platform()`（`video_pump.rs:100-133`）：`platform_capture()` / `monitors()` 任一返回 `Err` 即 `fallback()` 用 `SynthCapture`。
- `VideoPump::next_frame()`（`video_pump.rs:174-190`）：采屏 `grab()` 仅在返回 `Err` 时降级合成源；**无「画面内容为黑/纯色」的校验**。
- 平台捕获器的权限失败表现：
  - macOS `CGDisplayCapture::new()`（`platform_macos.rs:53+`）：未授「屏幕录制」时 `CGDisplayCreateImageForRect` 返回 NULL 或全黑。构造期探测若只查显示器枚举而未逐像素校验，**全黑图会被当成成功**（`platform_macos.rs` 顶部注释明确「返回全黑或 NULL」）。
  - Windows `GdiCapture::grab()`（`platform_windows.rs:37-95`）：`BitBlt` 成功但锁屏/安全桌面/无交互会话下像素全黑，`grab()` 返回 `Ok(全黑帧)`，不触发任何降级。
- `SynthCapture`（`capture/src/lib.rs:196-273`）：产红方块 + 灰条纹 + seq 测试图。用户看到的正是它。
- **没有任何信号把「已降级合成源」告知控制端**：`VideoFrameMsg`（`video.rs:57`）只有 seq/kind/flags/nalu/is_idr，无画面源字段；状态栏指标是演示默认值。故用户完全不知道自己看的是测试图。

### 2.2 修复策略（三层，按优先级）

#### 层 1：采屏端识别「黑/无效帧」并降级 + 上报状态
**`crates/capture/src/lib.rs`**
- 新增 `FrameQuality` 判定工具：`fn is_blank_frame(frame: &RawFrame) -> bool`——采样 Y 通道（BGRA 每 4 字节取 1），计算像素方差 / 非背景色占比；连续 N 帧（如 15 帧 ≈ 0.5s）近纯色视为无效。
- 对 `SynthCapture` 之外的真实采屏器统一启用（Windows GDI、macOS CGDisplay、Linux X11）。

**`crates/landesk-core/src/video_pump.rs`**
- `next_frame()` 采到帧后先跑 `is_blank_frame` 判定：
  - 连续黑帧达阈值 → 切换到 `SynthCapture`（保证会话不中断），并置 `self.degraded = true`、记录 `degraded_reason`（`NoPermission` / `BlankFrame` / `NoDisplay`）。
  - 降级后每 5s 重试一次真实采屏；恢复成功则切回并清 degraded。
- `VideoPump` 增 `is_degraded() -> bool` 与 `degraded_reason()` 供宿主读取。

#### 层 2：把「降级」明示给控制端用户
**后端事件**（`crates/ui/src/events.rs` 增 `CaptureDegraded { reason }`，`manager.rs` apply 进 `SessionStatusView` 新字段 `captureDegraded`）——因为受控端与控制端是两个独立进程，降级状态必须经 QUIC 控制通道传过来。

链路：`ControlledHost` 的采屏泵检测降级 → 经控制流发 `CaptureDegraded` 帧（`ControlMsg` 增变体）→ 控制端 `SessionManager` 收帧 → 写入 `SessionStatusView.captureDegraded` → 前端状态快照读取。

> 约束：新增 `ControlMsg` 变体需协议兼容（旧端忽略未知变体）。若嫌协议改动大，可用现有 `EncoderReset` 事件复用为通用告警（但语义不清），本方案选前者。

**前端 `Session.vue`**：`watch(liveSession)` 里读 `captureDegraded`，在画面上叠一条醒目 banner：
「对方屏幕未能采集（可能未授予屏幕录制权限），当前显示测试画面。请在被控端系统设置中给 LanDesk 授权后重试。」
并在状态栏把假的码率/延迟指标改为「—」或在真实事件接通前显示为「实测值待接入」，避免误导。

#### 层 3：状态栏指标接真（顺带修复误导）
**`crates/ui/src/manager.rs`**：apply `EncoderChanged/LatencyMs/LossPct/BitrateKbps/EncoderReset` 事件到状态字段（受控端需经控制通道发这些指标，同样走层 2 的通道）。若本次范围不含受控端指标上报，则**前端在 live 态不显示演示默认值**（`?? state.metrics.xxx` 改为仅在非 live 演示态显示），live 态显示「—」。

### 2.3 权限引导（治本，需产品确认）
- macOS：首次启动受控服务时检测 `CGWindowListCreateImage` 是否可用；不可用则弹引导，打开系统设置 → 隐私与安全性 → 屏幕录制。
- Windows：检测会话是否交互（如 `GetProcessWindowStation` / WTS 查询），非交互会话明确提示「当前为非交互桌面，无法采集屏幕」。
- 此项涉及系统设置跳转与权限检测，工作量较大；**本方案先做层 1+2（检测 + 明示），层 3 权限引导列为后续**。

### 2.4 验证
- 单测：`is_blank_frame` 对纯黑/纯色/合成图/真实图各给正确判定（用 `capture` crate 的 RawFrame 构造，无 OS 依赖）。
- 集成：无屏幕录制权限的 macOS 起受控 → 控制端应显示降级 banner，而非让用户误以为是真桌面。
- 正常授权环境：采真桌面 → 无 banner、画面为真实内容、8fps 不再伴随 0KB/s。

---

## 3. 问题 3：关闭后重连显示「选择设备」

### 3.1 现状证据
- 关闭：`session_window_close_confirmed`（`ui/src-tauri/src/lib.rs:484`）只置 `session_allow_close=true` 并关窗，**未清 `session_launch`**。
- 重开：主窗 `launchSession(target)`（`useSessionWindow.ts:52`）→ `landesk.sessionWindowOpen(target, path)` → 后端 `session_window_open`（`lib.rs:406`）写 `session_launch=Some(payload)` → 因窗已关，走 `WebviewWindowBuilder` 新建 → 前端 `SessionApp.onMounted` 读 `readSessionLaunch()` → `startSession(target, 'await-auth')`。这条路径 target 应能带到。
- **缺陷**：`Session.vue` 的 `watch(liveSession)` 有 idle 空快照守卫（`Session.vue:506-507`），重开时后端 `status_view()` 是 idle+null target，守卫 return——**保留了前端 target**。看似正确。
- 实际复现显示「选择设备」白屏（`Session.vue:724` idle 且无 target 的分支），说明 `state.sessionTarget` 为 null。可能路径：
  - (a) 重连是从会话窗内部「去选择设备」→ 主窗未重新 `launchSession`，而是复用旧 session 窗（已关）失败；
  - (b) `session_launch` 在关窗时被 Tauri 销毁窗口的某清理逻辑置空（当前代码未见，但需复核运行态）；
  - (c) `SessionApp.onMounted` 的 `readSessionLaunch()` 与 `Session.vue.onMounted` 的 `loadSessionStatus()` 竞态：后者拿 idle+null，前者尚未 resolve，`startSession` 后又被 `loadSessionStatus` 的 `apply` 覆盖——但守卫应挡住；除非 `snap.target` 非 null 而 `state.sessionTarget` 被置为别的空值。

### 3.2 修复策略（稳妥、不依赖精确复现）
1. **关闭时显式保留「上次目标」**：`AppState` 增 `last_session_launch: Mutex<Option<SessionLaunchPayload>>`，`session_window_open` 每次写入时同步更新它；`session_window_close_confirmed` / `window_close` **不清** `session_launch`（现状已如此），确保重开一定能读到。
2. **前端重连兜底**：`SessionApp.onMounted` 读到 `launch.target` 为 null 时，从 `recentTargets`（localStorage 持久化的最近连接，`useSessionWindow.ts:19`）取最新一条补上，调 `startSession(...,'await-auth')`；这样即便后端 payload 丢失，也回到上次设备的待鉴权页而非「选择设备」。
3. **消除竞态**：`SessionApp` 把 `startSession` 结果写入一个共享 ref，`Session.vue` 的 status watcher 只在 `state.sessionTarget` 已定时才 apply 后端 target，避免异步顺序覆盖。
4. **明确语义**：用户关闭会话窗后再从设备列表点连接，走全新 `launchSession`（target 明确），应直接进入待鉴权页——补全一条端到端测试覆盖「关窗→重开→target 保留」。

### 3.3 验证
- 集成测试：`session_window_open(Some(target))` → 模拟关窗 → 再 `session_window_target()` 返回同一 target。
- 前端单测：`launch.target=null` + `recentTargets` 有值 → `SessionApp` 回填最新目标、渲染待鉴权页。
- 手工：连一台设备→关闭窗口→从设备列表重新点连接→应显示「输入对方访问密码」页。

---

## 4. 问题 4：macOS 防火墙打开后 Windows 连不上

### 4.1 现象与根因
用户实测：macOS 被控端防火墙开启后，Windows 控制端无法连接。QUIC 是 **UDP 入站连接**，受控端监听 `0.0.0.0:52000`（`controlled_host.rs:177-181`），`Controlled::new` 内部经 `quinn::Endpoint::server` 绑定 UDP socket（`server.rs:40-47`）。macOS 应用防火墙（Application Firewall）默认拦截未签名/未授权应用的入站连接；Tauri 打包的 app 首次被入站时系统可能弹授权框，但：
- 用户可能点了「拒绝」或弹框被忽略；
- 即便防火墙放行 mDNS（UDP 5353 组播），QUIC 的 52000 入站仍可能被单独拦截——**发现得到设备但连不上**，与用户描述一致。

需区分两类防火墙：应用层防火墙（System Settings → Network → Firewall）与 `pf`/第三方。本方案聚焦前者（最常见）。

### 4.2 修复策略（可选组合，按侵入度递增）
1. **引导式（推荐先做，零代码风险）**：在设置页「被控服务」区加一段说明 + 检测按钮。检测到防火墙开启且 52000 不可达时，给出明确指引：「系统设置 → 网络 → 防火墙 → 选项 → 添加 LanDesk 并允许传入」。macOS 无公开 API 读应用防火墙状态（`socketfilterfw` 需 root），故用**自检代替状态读取**：受控端起服后用本机 mDNS 地址自连一次 52000，失败即判定可能防火墙拦截。
2. **端口策略**：若自连失败，提示可关闭「隐身模式」（防火墙「阻止所有传入」）或添加例外；不改代码强行开洞（无签名 app 无法绕过系统防火墙）。
3. **连接失败信息增强**：控制端 `connect` 超时时，若目标平台为 macOS，错误文案追加「请检查被控端 macOS 防火墙是否放行 LanDesk 的 52000 端口」，帮助用户自助排障。

### 4.3 验证
- macOS 开启应用防火墙 + 未授权：控制端连接失败并给出防火墙提示；按指引添加例外后连接成功。
- 单测：自连探测函数对「端口可达 / 不可达」两态返回正确。

---

## 5. 问题 5：macOS 连 Windows 一片黑（与问题 2 同源、表现不同）

### 5.1 现象与根因
用户实测：macOS 控制端连 Windows 被控端，画面一片黑（而非问题 2 的红块测试图）。两个方向对比：

| 方向 | 被控端采屏表现 | 画面结果 |
|---|---|---|
| Windows → macOS | macOS `CGDisplayCreateImageForRect` 无权限 → NULL 或全黑 → `platform_capture()` 构造失败 → 降级 `SynthCapture` | 红块条纹测试图（问题 2） |
| macOS → Windows | Windows GDI `BitBlt` 在锁屏/安全桌面/RDP 断开/无交互会话下**返回 Ok 但像素全黑**（`platform_windows.rs:37-95`：只检查 `BitBlt`/`GetDIBits` 返回值，`GetDIBits` 后直接 `Ok(frame)`，无像素内容校验） | 黑帧被正常编码发出→控制端解出纯黑（问题 5） |

两者根因同一类：**被控端采屏结果缺乏内容有效性校验**，只是 macOS 走「构造失败→降级合成源」路径、Windows 走「grab 成功但全黑→当正常帧发送」路径，故表现不同。

### 5.2 修复策略
完全复用问题 2 层 1 的 `is_blank_frame` 检测：
- `VideoPump::next_frame()` 对 Windows GDI 返回的帧做黑帧判定；连续 N 帧纯黑 → 降级 `SynthCapture`（红块测试图）+ 标记 degraded（这样至少用户能从红块看出是测试图，而不是误以为真桌面黑屏）。
- 增加 Windows 侧「无交互桌面」检测：`WTSQuerySessionInformation` / `GetProcessWindowStation` 判断当前会话是否交互；非交互则直接在 `new_platform()` 阶段降级合成源并上报原因（`NonInteractiveSession`），避免逐帧黑帧检测的 0.5s 延迟。
- 层 2 的状态上报与 banner 同样覆盖问题 5，文案区分原因：macOS「屏幕录制权限未授予」/ Windows「当前为非交互桌面或已锁屏」。

### 5.3 验证
- 单测：`is_blank_frame` 对全黑 RawFrame 返回 true（Windows 路径核心）。
- 实机：Windows 锁屏状态下起受控 → 控制端看到红块测试图 + 降级 banner，而非纯黑。Windows 正常登录桌面 → 看到真实桌面、无 banner。

---

## 6. 实施分批与提交

| 批次 | 内容 | 提交 | 风险 |
|---|---|---|---|
| B1 | 问题 1（顶栏布局）+ 问题 3（重连兜底）：纯前端 + 少量后端状态 | `fix/session-topbar-reconnect` | 低 |
| B2 | 问题 2/5 层 1+3（采屏黑帧检测降级 + 状态栏假指标处理）：Rust 侧 + 前端展示，覆盖 macOS 与 Windows 两平台 | `fix/capture-degraded-detect` | 中（涉及采屏策略） |
| B3 | 问题 2/5 层 2（降级状态跨 QUIC 上报 + banner）：新增协议变体 | `fix/capture-degraded-signal` | 中高（协议兼容） |
| B4 | 问题 4（macOS 防火墙检测 + 引导 + 连接错误文案）：设置页说明 + 自检 + 前端文案 | `fix/macos-firewall-guidance` | 低 |

按用户偏好：每批**本地提交**，不推送、不建 PR、不发版，等用户明确指令再走 Git Flow。

> **实施结果（2026-11-23，分支 `fix/session-five-issues`，6 提交）**：
> - B3 层 2 实际未采用「新增 ControlMsg 变体」，改为**视频 datagram flags 带内上报**（`FLAG_CAPTURE_DEGRADED`/`FLAG_DEGRADED_REASON`）。原因：控制端 `SessionManager` 无 `recv_control` 循环，且 `send_control`/`recv_control` 均需 `&mut session`——新增接收循环会长期持锁阻塞其他命令，需重构会话拆半，风险高于收益。带内方案与画面天然同步、控制端视频泵任务独立不争锁、零协议兼容成本。
> - B4 未做自连探测（macOS 无公开 API 读应用防火墙状态，且本机自连成功无法证明外部可入站），改为设置页 + 连接失败页的**引导文案**。
> - 验证：全量 `cargo clippy --workspace --all-targets -D warnings`、`cargo test --workspace`、前端 `typecheck` + `vite build` 全通过。

## 7. 待用户确认的决策点

1. **降级提示方式**（问题 2/5）：是否接受「新增控制通道 `CaptureDegraded` 事件 + banner」（B3），还是先只做本地提示（受控端弹窗/日志）以缩小改动？
2. **状态栏假指标**：live 态是否直接隐藏 `18ms/4.2Mbps` 等演示默认值（改为「—」）？
3. **权限引导与防火墙**（问题 4 + 2.3）：本期做还是后续单独一期？macOS 应用防火墙无公开 API 读状态，方案采用「自连探测 + 文案引导」，是否接受？
4. **修复范围**：五个问题分四批全做，还是先做某几批（如先 B2 解决画面黑屏/测试图，其余后续）？
