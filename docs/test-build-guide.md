# LanDesk 双机验证测试包指南

> 目的：用**测试包**（非正式 Release）在两台真实机器上验证画面与控制，避免"未验证就升正式版"。
> 测试包由 `.github/workflows/test-build.yml` 产出，版本号形如 `0.5.0-test.42`，
> 只上传 Actions 制品（artifact），**不建 Release、不打 tag、不改任何分支**。

---

## 0. 适用人群与前置条件

- 需要两台机器（物理机或 VM 均可，须同一局域网、能互相 mDNS 发现）：
  - **A 机：被控端**（跑 LANDesk GUI，开启"被控服务"，输入访问密码）
  - **B 机：控制端**（跑 LANDesk GUI，发现 A 机 → 连接 → 看画面/操作键鼠）
- 两台机器**都安装测试包**（同一 run 的对应平台制品）。
- 一个能访问 GitHub Actions 的账号（下载制品用）。

---

## 1. 构建测试包（约 2 分钟操作，CI 编译较慢）

> **为什么不是 Actions 页面点按钮？** GitHub 限制 `workflow_dispatch`（手动触发）
> 只响应**默认分支（main）**上的工作流文件。当前工作流在 `feature/diagnostic-verification`
> 分支、尚未合入 main，API/UI 都触发不到。因此用一个**专用触发分支** `test-build/**`：
> push 事件读取的是被推送分支自己的工作流定义，不受默认分支限制。
> 触发脚本把当前 HEAD 推到 `test-build/latest`（纯触发分支，可覆盖），**不碰 dev/main/tag**。

```bash
# 在要验证的分支上（当前是 feature/diagnostic-verification）
./scripts/trigger-test-build.sh              # 触发，输出 run 链接
./scripts/trigger-test-build.sh --wait       # 触发并等到结束（最多 40 分钟）
```

> 触发脚本只会 push `test-build/**` 分支，用的是 `git push -f origin HEAD:refs/heads/test-build/latest`
> （**不带 -u**）——早期版本带 `-u` 会把开发分支 upstream 改成触发分支，之后普通 `git push` 就推错地方，已修复。
>
> 工作流里的版本注入脚本对 `tauri.conf.json` 的读写都显式 `encoding='utf-8'`——Windows 上 Python
> 默认 cp1252，读含中文权限说明的配置会 `UnicodeDecodeError`（首个 run 实测踩到并已修复）。

1. 脚本会打印该 run 的 Actions 链接；也可打开
   `https://github.com/scpuny/LanDesk/actions/workflows/test-build.yml?query=branch%3Atest-build/latest`。
   （合入 main 后，也可直接在 Actions 页手动 Run workflow。）
2. 等待 run 完成（三个 job：macos / windows / linux）。**GUI 打包是 best-effort**：
   - 关键看每个 job 末尾 `GUI artifacts:` / `GUI NSIS:` / `GUI deb:` 的日志行——
     有文件名才是真出了安装包；空了就是 GUI 编译失败，只出了 CLI，**不要拿 CLI 当 GUI 用**。
4. run 页面底部 **Artifacts** 下载：
   - `test-macos-<N>`：含 `landesk-0.5.0-test.N-gui-macos-arm64.dmg`（Apple 芯片）或 `x86_64`（Intel），
     以及 `landesk-*.dmg` 对应 `-app.zip`。**按 A/B 机芯片选对架构**（Apple Silicon 用 arm64，Intel Mac 用 x86_64）。
   - `test-windows-<N>`：`landesk-0.5.0-test.N-gui-windows-x86_64-setup.exe`。
   - `test-linux-<N>`：`*.deb`（可选）。
   - 每个包附 `README-测试包.txt`，记录来源分支/commit，便于追溯。
   - 制品 **14 天过期**，过期重新跑工作流即可。

> 版本号 `test.N` 中的 N 与该 run 的序号一致；界面"关于"页与诊断包显示的也是这个版本，便于确认装的是测试包而非旧正式版。

---

## 2. 安装（测试包未签名，注意系统拦截）

### macOS（被控端 A 机为例）
1. 双击 dmg 挂载，把 `LanDesk.app` 拖到"应用程序"。
2. **首次打开**：右键 `LanDesk.app` → 打开 → 弹窗再点"打开"（或 系统设置 → 隐私与安全性 → "仍要打开"）。
3. **关键**：启动后开启"被控服务"、设置访问密码时，macOS 会弹
   **"LanDesk 请求屏幕录制权限"**——**必须允许，然后完全退出并重启应用**。
   - 这一步正是 `NSScreenCaptureUsageDescription` 修复的验证点：**旧版根本不会弹这个窗**。
   - 若没弹：系统设置 → 隐私与安全性 → 屏幕录制 → 找到 LanDesk 勾选 → 重启应用。
4. **防火墙**：系统设置 → 网络 → 防火墙，若开启，须允许 LanDesk 入站（否则对端发现得到但连不上）。

### Windows（控制端 B 机为例，或被控端亦可）
1. 运行 `*_setup.exe`；SmartScreen 拦截时点"更多信息 → 仍要运行"。
2. 首次启动允许防火墙专用网络入站（弹窗勾选）。

---

## 3. 自动验证（推荐先跑，省去肉眼猜）

在一台**已装好 CLI** 的机器（可用测试包 zip 里的 `controller`/`controller.exe`，或仓库 `cargo build -p controller`）上，把 CLI 放到 PATH 后：

```bash
# 3.1 确认识别到对端（A 机须已开启被控服务）
controller device-list --wait-secs 5
# 期望输出 JSON 数组，含 A 机设备 id / 名称 / ip / 端口；空数组则 mDNS 不通，先查同网段与防火墙。

# 3.2 探测画面是否真实可用（核心验证）
export LAN_DESK_PW='A机设置的访问密码'
controller probe \
  --device '<上一步拿到的设备ID或设备名>' \
  --password "$LAN_DESK_PW" \
  --duration-secs 8
```

`probe` 输出 JSON 判据（详见 `apps/controller/src/main.rs` 的 `verdict_for`）：

| verdict | reason | 含义 | 对应问题 |
|---|---|---|---|
| PASS | ok | 收到 IDR 关键帧、无降级标记 → **画面确实在推流** | ✅ 问题 2/5 通过 |
| FAIL | capture_no_permission | 对端无屏幕录制权限（降级码 0） | ❌ 问题 2：去 A 机授权屏幕录制 |
| FAIL | capture_blank_frame | 对端采到全黑帧（降级码 1） | ❌ 问题 5：A 机锁屏/登录态，解锁到桌面 |
| FAIL | no_video_frames | 完全没有视频包 | ❌ 查连接/防火墙/被控服务 |
| FAIL | no_keyframe | 只有 SPS/PPS，无 IDR | ❌ 编码器未出关键帧 |

> `probe` 只读 QUIC 数据报头（kind/flags），**不解码 H.264**，所以无显示器也能跑。
> 密码走 `--password` 会留在 shell 历史/进程列表——用完清历史，或在本机临时改由环境变量包装（当前 CLI 尚未支持 `--password-env`）。

---

## 4. 人工回归清单（五问题 + 控制体验）

自动 PASS 只证明"有真实画面在推流"，**不能替代肉眼与手感**。逐项打勾：

| # | 场景 | 判定标准 | 关联修复 |
|---|---|---|---|
| 1 | macOS 控制端开会话窗 | 红绿灯未把右侧顶栏挤出边界，窗口标题/按钮完整可见 | `useWindowControls.ts` 平台判定同步 |
| 2 | B 机连 A 机看画面 | 显示**真实桌面**（非红块测试图）；切窗口/滚动有变化 | 屏幕录制权限 + 采屏降级 banner |
| 3 | 关闭会话窗后重新连接 | 直接回到连接流程，**不是"选择设备"空页** | `SessionApp.vue` recentTargets 回填 |
| 4 | A 机(macOS) 开防火墙，B 机(Windows) 连接 | 能连上；连不上时错误文案含"防火墙/放行 UDP 52000"提示 | 连接错误文案 + 设置页引导 |
| 5 | A 机(macOS) 连 B 机(Windows)，B 机为登录桌面态 | 画面非全黑；B 机锁屏时 A 机看到明确的"采屏降级"提示而非黑屏 | 连续黑帧检测 + 降级带内上报 |
| 6 | 键鼠往返 | B 机移动鼠标/打字，A 机桌面响应；延迟可接受 | QUIC 双通道 |
| 7 | 画面指标真实性 | 状态栏帧率/码率为实测值或"—"，**不显示假 60fps** | 去假 fps |

---

## 5. 出问题怎么办（别再靠截图猜）

1. **当场**在出问题的那台机器 GUI → 日志中心 → 点"导出诊断包"（新功能，真写文件）：
   - 内含平台/版本/本机/受控/配置摘要（密码脱敏）+ 可疑事件 + 全部原始日志。
   - 采集降级跳变（0=无录屏权限 / 1=锁屏黑帧）、连接失败目标 IP:port 都已落日志。
2. 把诊断包发给开发，附上：测试包版本号（`0.5.0-test.N`）、A/B 机各自 OS 与芯片、当时在做什么。
3. **注意**：日志是内存环形缓冲（2000 条），**退出应用即丢**——务必当场导出，别重启后再找。

---

## 6. 验证通过后如何发布正式版（发布纪律）

只有上面 **§3 自动项全 PASS + §4 人工清单全打勾、无未决问题**，才允许进入正式发布：

1. 测试包分支合入 `dev`（PR 走 CI 全绿）。
2. 从最新 `dev` 切 `pre-release` 分支并推送。
3. CI 在 `pre-release` 上全绿。
4. 从 `pre-release` 打 `v0.x.y` tag → 触发 `release.yml` 正式发布。
5. Release 成功 → CI 自动合 tag 进 `main` + 删除 `pre-release`。

**任何回归项未过 → 不得 promote 正式版。** 手动跑一次 `test-build.yml` 的成本远低于又一个带 bug 的正式版。

---

## 7. 附：VBox VM 半自动编排（可选）

若用本机 VBox VM 代替物理机，仓库已有 `scripts/test-e2e/vbox-smoke.sh`：
- `./scripts/test-e2e/vbox-smoke.sh --no-start`：不启动 VM，验证脚本流程与诚实失败行为。
- 完整跑：脚本会 `VBoxManage startvm` → arp 探活 → 调 `controller device-list/probe` 自动判定 → 生成报告。
- 注意：脚本**不进 guest 内部**装包/启动服务，guest 内 GUI 操作仍是人工打勾项；报告含 JSON 证据与回归清单。

---

## 踩坑 3：CI "success" 是假绿灯 —— GUI 其实全失败（2026-11-24）

首个测试包 run 三平台步骤全绿、artifact 也产出了，但抓日志发现：

```
== GUI dmg/app x86_64
   Error `"tauri.conf.json"` error on `bundle > macOS > infoPlist`: {...} is not of types "null", "string"
GUI build x86_64 failed, skipping
```

**根因**：`bundle.macOS.infoPlist` 被写成了对象。Tauri 2 官方 schema 明确定义：
`infoPlist` 是 `string | null`，语义为 "Path to a Info.plist file to merge with the default Info.plist"，
且 Tauri 会自动读配置文件同目录的 `Info.plist`。写对象 → 配置校验失败 → `tauri build` 直接退出。

**更根本的问题**：三个 GUI 步骤都带 `continue-on-error: true`，且命令后跟 `|| echo "... skipping"`，
所以 tauri build 失败被吞成 step success。artifact 里只有 CLI，**没有能装的 dmg/exe/deb**——
如果照这个包去做双机验证，会完全无功而返。这正是"未经验证就升版"的同型问题：绿灯不代表可用。

**修复**：
- `tauri.conf.json`: `infoPlist` 改为字符串 `"Info.plist"`；
- 新增 `ui/src-tauri/Info.plist`（含 `NSScreenCaptureUsageDescription`），Tauri 自动合并进生成的 Info.plist；
- `test-build.yml`: macOS/Windows GUI 步骤去掉 `continue-on-error` 与 `|| skipping`，改为失败即红，
  并在末尾断言 `.dmg` / `.exe` 产物存在；Linux 保留 best-effort（双机主控是 macOS/Windows）。

**验证纪律**：测试包 run 必须同时满足 ①三 job success ②artifact 内含 GUI 安装包，才算通过。
只看 job 绿灯不算数——这是血的教训。

**仍未解决（需用户决策）**：`release.yml` 三平台 GUI 步骤同样是 `continue-on-error: true`，
且共用同一个 `tauri.conf.json`。虽然本次 plist 格式已修，但正式版仍可能在 GUI 打包失败时
"成功发布却无 GUI 包"。是否把正式版也改硬门禁，涉及发布策略变更，等用户决定。
