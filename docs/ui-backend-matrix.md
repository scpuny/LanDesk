# LanDesk 前端 ↔ Rust 后端对接对照矩阵

> 生成：2026-11-20 · 基线 commit `55ecfab`（前端 Vue3+TS 重构后）+ dev `e631079`
> 范围：`ui/frontend`（4 页面 + 数据层）× `crates/ui`（landesk-ui 内核）× `apps/ui-server`（REST）× `ui/src-tauri`（Tauri 命令壳）
> 结论先行：**命令面"已实现但未接通"——5 个后端命令在 Tauri / REST 两侧都有，但双宿主的 live 路径当前都走不到后端（详见 §5），另有 6 处字段级错位。**

## 1. 前端功能清单（逐页面 / 逐操作 / 逐查询）

### 1.1 页面与导航（4 主视图 + 壳）
| 页面 | 标识 | 入口 | 后端依赖 |
|---|---|---|---|
| 我的设备 | `devices` | LeftRail / Topbar | 设备列表 + 本机信息 |
| 远程会话 | `session` | 设备卡点击 / 快速连接 | 会话建立/操作（**当前纯前端演示**） |
| 日志中心 | `logs` | LeftRail | 日志查询（**当前 mock**） |
| 设置 | `settings` | LeftRail / 各页跳转 | 配置读写 |
| 壳：顶栏 | — | App.vue | 主题（纯前端 localStorage） |
| 壳：左轨 | LeftRail | App.vue | 设备计数 / 本机卡 |

### 1.2 桥接层 5 个 API（`src/lib/landesk.ts`，双轨：Tauri `invoke` / 浏览器 `fetch`）
| 前端方法 | Tauri 命令 | REST 路径 | 用途 |
|---|---|---|---|
| `configLoadOrInit()` | `config_load_or_init` | `GET /api/config` | 读/首生成配置（FR-08） |
| `configSave(cfg)` | `config_save` | `POST /api/config` | 保存配置（FR-08） |
| `deviceList(devs, now)` | `device_list` | `GET /api/devices` | 设备列表（FR-01.2/01.4） |
| `reconnectView(used)` | `reconnect_view` | `POST /api/reconnect` | 重连指示器（FR-05.4） |
| `choosePath(inner, hole, relay)` | `choose_path` | `POST /api/choose-path` | 连接路径决策（FR-05/11） |

### 1.3 逐页面操作明细
**设备页（DeviceList.vue）**
| 操作 | 触发 | 数据源 | 后端状态 |
|---|---|---|---|
| 加载设备 `onMounted(loadDevices)` | 挂载 | `landesk.deviceList` | ⚠️ 见 §5-2 |
| 刷新重扫 `loadDevices()` | 刷新/空态按钮 | 同上 | ⚠️ 同上 |
| 名称/地址搜索过滤 `filtered` | 输入框 | 纯前端 | ✅ 本地计算 |
| 设备卡点击 / 连接按钮 `connect(d)` | 点击 | `ui.startSession` | ❌ 纯前端，无建会话 |
| 快速连接码 `quickConnect` | 回车/按钮 | `ui.startSession` | ❌ 无信令查 ID |
| 一次性密码 | 按钮 | `toast` | ❌ 纯提示 |
| 复制设备码/临时密码 | 按钮 | `navigator.clipboard` | ✅ 浏览器能力 |
| 临时密码刷新 `refreshTempPassword` | 刷新按钮 | 前端随机数 | ❌ 无后端下发 |
| 本机设备码/信令/中继/编码卡 `local` | 渲染 | `useLanDesk.local` | ❌ 纯 mock，无 API |
| 最近连接 chips | 点击 | 前端写 quickCode | ❌ 无持久化 |

**会话页（Session.vue）—— 全演示态**
| 操作 | 后端状态 |
|---|---|
| 鉴权中/运行/重连/鉴权失败/无路径 5 态切换 `cycleState` | ❌ 纯前端 `setState` |
| 重新验证 `verify`（临时密码） | ❌ 前端直接置 running，无鉴权命令 |
| 立即重试 `retryNow` | ❌ 无重连调度命令 |
| 断开 `disconnect`（二次确认） | ❌ 无会话断开命令 |
| 画面工具栏：剪贴板/键鼠/截图/显示器切换/全屏 | ❌ 全部 `toast`，无数据面命令 |
| 状态栏 7 指标（encoder/latency/loss/bitrate/reset/…） | ❌ 前端写死 `metrics`，无 `apply_status` 推送 |

**日志页（Logs.vue）**
| 操作 | 后端状态 |
|---|---|
| 级别筛选（ALL/INFO/WARN/ERROR/DEBUG）| ❌ 前端过滤 mock `logs` |
| 关键字过滤 | ❌ 前端过滤 |
| 导出 log | ❌ `toast('已导出')`，无下载 API |
| 清空 | ❌ 前端置空，无后端 |

**设置页（Settings.vue）**
| 操作 | 后端状态 |
|---|---|
| 读配置 `loadConfig` | ✅ `configLoadOrInit` |
| 保存 `save` | ✅ `configSave` |
| 重置 `reset`（二次确认）| ✅ 回到 `loadConfig` |
| 设备名/密码/预设/脏矩形/mDNS/信令/中继/STUN/日志级 各表单 | ⚠️ 字段见 §6-Config |
| "跳转日志中心带 encoder 关键字" | ⚠️ 前端无导航参数传递 |

**壳（主题/通知/最近连接）**
| 能力 | 后端状态 |
|---|---|
| 明暗 pref（light/dark/system）+ 7 palette | ✅ 纯前端 localStorage + CSS 变量，无需后端 |
| 通知铃铛 / 头像 | ❌ 无 |

## 2. Rust 后端命令面清单

### 2.1 内核 `crates/ui`（landesk-ui，纯逻辑可单测，17 单测）
| 模型/函数 | 出参形状 | 覆盖 FR |
|---|---|---|
| `device_rows(&mut Vec<OnlineDevice>, now) -> Vec<DeviceRow>` | `{id,name,instance,addr,res,enc,relay,last_seen}` | FR-01.2/01.4（TTL 剪枝+排序） |
| `decide_path(inner, &HoleResult, relay) -> PathDecision` | `Use(ConnMode) | NoPath` | FR-05/11 |
| `view_of(&ReconnectScheduler) -> Option<ReconnectView>` | `{attempt,max_attempts,wait_secs,exhausted,label}` | FR-05.4 |
| `ConfigStore::load_or_init / save` | `Config`（landesk-core） | FR-08 |
| `StatusBarModel::apply(&CoreEvent)` | `{state,conn_mode,encoder,latency_ms,loss_pct,bitrate_kbps,reset_events}` | FR-09.3 |

### 2.2 `apps/ui-server`（REST，浏览器宿主，复用内核 + mdns Listener）
| 路由 | 方法 | 状态 |
|---|---|---|
| `/health` | GET | ✅ |
| `/api/devices` | GET | ✅ 真接 mdns 在线表 + `device_rows` |
| `/api/config` | GET / POST | ✅ `ConfigStore` |
| `/api/choose-path` | POST | ✅ `decide_path` |
| `/api/reconnect` | POST | ✅ `view_of` |

> 说明：ui-server 侧 5 端点**实现完整且真接 mdns**，是"已对接"的。

### 2.3 `ui/src-tauri`（Tauri 命令壳，`#[tauri::command]`）
| 命令 | 入参 | 出参 | 状态 |
|---|---|---|---|
| `config_load_or_init` | — | `Result<Config, StoreError>` | ✅ |
| `config_save` | `config` | `Result<(), StoreError>` | ✅ |
| `device_list` | `devices: Vec<OnlineDevice>, now` | `Vec<DeviceRow>` | ⚠️ 入参由前端传（见 §5-3） |
| `reconnect_view` | `used_attempts` | `Option<ReconnectView>` | ⚠️ 字段错位（§6） |
| `choose_path` | `inner, hole: HoleResult, relay` | `PathDecision` | ⚠️ hole 类型（§6） |

## 3. 对接总矩阵（前端操作 → 后端命令 → 状态）

状态图例：✅ 已对接可用 / ⚠️ 已实现但字段或路径错位 / ❌ 前端有、后端无 / ⛔ 双宿主 live 路径不可达

| 前端功能 | 对应后端 | 状态 | 证据 |
|---|---|---|---|
| 设置读/存配置 | `config_load_or_init`/`config_save` + `/api/config` | ⚠️ | 命令在，但 live 路径不通（§5）+ Config 字段错位（§6-Config） |
| 设备列表 | `device_list` + `/api/devices` | ⚠️ | ui-server 真接 mdns；Tauri 壳入参传空（§5-3） |
| 连接路径决策 | `choose_path` + `/api/choose-path` | ⚠️ | 前端 `hole: boolean` vs 后端 `HoleResult` 枚举（§6） |
| 重连指示 | `reconnect_view` + `/api/reconnect` | ⚠️ | 字段名错位（§6） |
| 会话建立/鉴权/断开/重连 | — | ❌ | 后端无会话命令（`landesk-core` 有编排但**未暴露给 UI 层**） |
| 画面/键鼠/剪贴板/截图/显示器 | — | ❌ | 数据面命令完全未接 UI（见 real_dataface 仅测试层） |
| 日志查询/导出/清空 | — | ❌ | 无日志 API |
| 本机信息（设备码/临时密码/能力） | — | ❌ | 无本机信息 API |
| 主题/通知/最近连接持久化 | — | ✅ / ❌ / ❌ | 主题纯前端；其余无 |

## 4. 汇总统计
- 前端 5 个桥接 API：后端 5 命令**全部已实现**（ui-server 真接、Tauri 壳完整）。
- **但双宿主 live 路径当前都走不到后端**（§5），故实际运行仍是 mock。
- 前端有而后端无：会话/数据面/日志/本机信息 共 **4 类 12+ 操作**（会话页 + 日志页 + 本机卡）。
- 字段级错位：6 处（§6）。

## 5. 关键断点（为什么 live 走不到后端）
1. **浏览器宿主：`window.__LANDesk_live__` 无人设置**。`useLanDesk` 的 `isLive = !!(window.__TAURI__?.core) || !!window.__LANDesk_live__`，但 `index.html` 无注入、ui-server 托管的 `dist/` 也没带该标记 → 浏览器下 `isLive` 恒 false → 5 个 REST 端点**全部不发起**，恒显 mock。
2. **Tauri 宿主：`tauri.conf.json` 未开 `withGlobalTauri`**。Tauri 2.x 默认不注入 `window.__TAURI__`（官方用 `@tauri-apps/api`）。前端 `isTauri()` 判 `window.__TAURI__?.core?.invoke` → 恒 false → 落到 REST 轨 → 在 Tauri WebView 里 `fetch('/api/...')` 无后端 → 5 个 Tauri 命令**也调不到**。
3. **Tauri 设备列表永远空**：`device_list` 的 `Vec<OnlineDevice>` 入参由前端传，而前端只传 `[]`；Tauri 壳**没有** mdns Listener / 在线表（那是 ui-server 独有）→ 出参恒 `[]`。
4. 结论：要让真实链路跑通，需 (a) Tauri 开 `withGlobalTauri` 或改用 `@tauri-apps/api`，(b) 浏览器宿主给 `index.html` 注入 `__LANDesk_live__=true`，(c) 把 mdns 在线表下沉到 Tauri 壳或让 `device_list` 无入参直接读壳内状态。

## 6. 字段级错位（即便 live 通了也会反序列化失败/丢字段）
1. **`choosePath` 的 `hole`**：前端传 `boolean`，后端要 `HoleResult`（`Direct|NeedRelay|NoPath` 枚举）。前端丢了三态信息。
2. **`ReconnectView` 字段名**：前端 `{attempt,max,mode}`，后端 `{attempt,max_attempts,wait_secs,exhausted,label}` → `max≠max_attempts`、前端独有 `mode`、后端独有 3 字段。
3. **`Config.preset`**：Rust `EncodePreset` 是 `#[repr(u8)] + Serialize`，serde 默认序列化成**字符串** `"Smooth"|"Balanced"|"Hd"`，前端期望数字 `0|1|2`。
4. **`Config.relay_url`**：前端有（`turn://relay.lan:3479`），Rust `Config` **无此字段**（只有 `relay_enabled: bool`）→ 保存时该字段丢失。
5. **`Config.password`**：Rust 是 `Option<String>`（可空=仅临时密码），前端是 `string`（mock `'••••••'`）→ 空/掩码语义不一致。
6. **`DeviceRow`**：后端出 `enc: String`（csv），前端 `DeviceRow.enc: string[]`；前端 `online/kind` 由本地推导（后端无这两字段，`kind` 恒回退 `monitor`）。

## 7. 建议（按投入/收益排序）
1. **P0 接通 live**：`tauri.conf.json` 加 `"security": {"withGlobalTauri": true}`（或前端改 `@tauri-apps/api`）；`index.html` 注入 `__LANDesk_live__`。让 5 个已实现命令真正可达。
2. **P0 修字段错位**：`choosePath` 传 `HoleResult` 字符串；`ReconnectView` 统一字段名；`EncodePreset` 改 `#[serde(untagged)]` 或前端改读字符串；`Config` 加 `relay_url`。
3. **P1 设备列表下沉**：Tauri 壳加 mdns 在线表，`device_list` 改无入参读壳状态（对齐 ui-server）。
4. **P1 暴露会话/数据面命令**：把 `landesk-core` 的会话建立/鉴权/重连、以及数据面（画面/键鼠/剪贴板）包成 UI 命令，替换会话页的演示态。
5. **P2 日志 + 本机信息 API**：接 `tracing` 环形缓冲查询/导出；本机设备码/能力/临时密码 API。
6. **P2 前端 mock 清理**：live 可用后移除 `mockDevices/mockConfig/mockLogs`，避免误导。

## 附：文件定位
- 前端：`ui/frontend/src/{views,composables,lib,types,components}`
- 内核：`crates/ui/src/{devices,plan,reconnect_view,status,config_store}.rs`
- REST：`apps/ui-server/src/main.rs`
- Tauri：`ui/src-tauri/src/{lib,commands}.rs` + `tauri.conf.json`
- 会话/数据面（未接 UI）：`crates/landesk-core/src/{session,auth,reconnect,events}.rs`、`crates/{capture,avcodec-wrap,input,quic-link}`
