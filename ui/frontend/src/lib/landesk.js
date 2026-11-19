// LanDesk 前端 ↔ 后端桥接封装（对齐 ui/src-tauri 的 Tauri 命令 + apps/ui-server 的 REST）。
//
// 双轨降级：
// - Tauri 宿主（有 window.__TAURI__）：invoke 命令（ui/src-tauri/src/lib.rs 的 #[tauri::command]）；
// - 浏览器（apps/ui-server 托管，无 Tauri）：REST fetch（/api/*，对应 landesk-ui 命令内核）。
// 出参为 landesk-ui 的可序列化模型（JSON 直进组件）。
//
// REST 端点（apps/ui-server）：
//   GET  /api/devices       -> DeviceRow[]
//   GET  /api/config        -> Config
//   POST /api/config        <- Config  -> 204
//   POST /api/choose-path   <- {innerFound,hole,relayEnabled} -> {use:{connMode}|nopath}
//   POST /api/reconnect     <- {usedAttempts} -> ReconnectView | 204

const IS_TAURI = () =>
  typeof window !== "undefined" && window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;

/** Tauri invoke 包装。 */
async function tauriInvoke(cmd, args) {
  return window.__TAURI__.core.invoke(cmd, args);
}

/** REST 包装：GET/POST JSON，非 2xx（除 204）抛错。 */
async function rest(path, { method = "GET", body } = {}) {
  const res = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  if (res.status === 204) return null; // 耗尽 / 保存成功
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    let msg = `${res.status} ${res.statusText}`;
    try {
      const j = JSON.parse(text);
      msg = j.error ? `${res.status}: ${j.error}` : text;
    } catch {
      if (text) msg = text;
    }
    throw new Error(msg);
  }
  const ct = res.headers.get("content-type") || "";
  return ct.includes("application/json") ? res.json() : res.text();
}

export const landesk = {
  /** 配置页：读或首次生成默认模板（FR-08）。 */
  configLoadOrInit: () =>
    IS_TAURI() ? tauriInvoke("config_load_or_init") : rest("/api/config"),

  /** 配置页：保存（FR-08）。`config` 为 landesk-core `Config` 的 JSON。 */
  configSave: (config) =>
    IS_TAURI() ? tauriInvoke("config_save", { config }) : rest("/api/config", { method: "POST", body: config }),

  /**
   * 设备列表（FR-01.2/01.4）。
   * Tauri 宿主：入参在线设备 + epoch 秒（宿主侧 mdns 监听喂）；
   * REST（ui-server）：直接 GET /api/devices（服务端自持 mdns 监听 + TTL 剪枝 + 排序）。
   */
  deviceList: (devices, nowEpochSecs) =>
    IS_TAURI()
      ? tauriInvoke("device_list", { devices, nowEpochSecs })
      : rest("/api/devices"),

  /** 重连指示器（FR-05.4）：入参已用尝试数，出参 ReconnectView | null。 */
  reconnectView: (usedAttempts) =>
    IS_TAURI()
      ? tauriInvoke("reconnect_view", { usedAttempts })
      : rest("/api/reconnect", { method: "POST", body: { usedAttempts } }),

  /** 连接路径决策（FR-05/11）：内网→P2P→中继→NoPath。 */
  choosePath: (innerFound, hole, relayEnabled) =>
    IS_TAURI()
      ? tauriInvoke("choose_path", { innerFound, hole, relayEnabled })
      : rest("/api/choose-path", {
          method: "POST",
          body: { innerFound, hole, relayEnabled },
        }),
};

export default landesk;
