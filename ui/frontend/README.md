# ui/frontend — LanDesk 前端（Vue3 + Arco Design，M8，形态 B）

对齐 `docs/architecture.md` §3 `ui/frontend`。Lan Studio 内置模块 / 独立 GUI。

> 工具链（node/pnpm/Arco/Tauri CLI）**不在离线 vendor**，故本目录不进主仓
> `cargo build/test --offline` 门；需本地装 Node + Tauri 工具链后构建（见 `ui/src-tauri`）。

## 视图（对齐架构文档）

| 文件 | 对应 FR |
|---|---|
| `views/DeviceList.vue` | FR-01.2 局域网设备列表（一键连接） |
| `views/Session.vue` | FR-09 预览窗口（缩放/全屏/状态栏） |
| `views/Settings.vue` | FR-08 受控端配置面板 |
| `views/Logs.vue` | FR-08.4 日志 / 编码器日志查看 |
| `components/StatusBar.vue` | FR-09.3 延迟/编码类型/连接方式/丢包率 |
| `lib/landesk.js` | Tauri 命令桥接封装 |

## 与 Rust 命令的对接（`lib/landesk.js`）

所有视图经 `lib/landesk.js` 调 Tauri `invoke`（命令定义见 `ui/src-tauri/src/lib.rs`）：

- `config_load_or_init` / `config_save` — 配置页读/写（FR-08）
- `device_list` — 设备列表（FR-01.2/01.4，出参含 TTL 剪枝 + 排序行）
- `reconnect_view` — 重连指示器（FR-05.4）
- `choose_path` — 连接路径决策（内网→P2P→中继，FR-05/11）

命令出参是 `landesk-ui` 的可序列化模型（`DeviceRow`/`StatusBarModel`/`ReconnectView` 等），
直接 JSON 进组件 `:data`。

## 构建

```
# 需 Node + 本地 Tauri 工具链（离线环境不可用）
cd ui/frontend && pnpm install && pnpm build   # 产物 dist/ 喂 tauri.conf.json frontendDist
```
