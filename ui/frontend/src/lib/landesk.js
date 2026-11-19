// LanDesk 前端 ↔ Tauri 命令桥接封装（对齐 ui/src-tauri/src/lib.rs 的命令定义）。
//
// 命令出参为 landesk-ui 的可序列化模型（JSON 直进组件）；入参字段名与 Rust 端
// `#[tauri::command]` 形参一致（camelCase，tauri 自动转换）。
//
// 浏览器/非 Tauri 环境（如直接打开 dev server）下 `window.__TAURI__` 不存在，
// 各方法降级为 reject 空结果，避免白屏——联调时以 Tauri 宿主为准。

/**
 * Tauri invoke 包装：`window.__TAURI__` 存在则真调，否则 reject（降级）。
 * @param {string} cmd 命令名
 * @param {object} [args] 入参（Rust 形参 camelCase）
 */
async function invoke(cmd, args) {
  const tauri = window.__TAURI__;
  if (!tauri || !tauri.core || !tauri.core.invoke) {
    throw new Error(`非 Tauri 环境，命令 ${cmd} 不可用（请以 Tauri 宿主运行）`);
  }
  return tauri.core.invoke(cmd, args);
}

export const landesk = {
  /** 配置页：读或首次生成默认模板（FR-08）。 */
  configLoadOrInit: () => invoke("config_load_or_init"),

  /** 配置页：保存（FR-08）。`config` 为 landesk-core `Config` 的 JSON。 */
  configSave: (config) => invoke("config_save", { config }),

  /** 设备列表（FR-01.2/01.4）：入参在线设备数组 + 当前 epoch 秒，出参 DeviceRow[]。 */
  deviceList: (devices, nowEpochSecs) =>
    invoke("device_list", { devices, nowEpochSecs }),

  /** 重连指示器（FR-05.4）：入参已用尝试数，出参 ReconnectView | null。 */
  reconnectView: (usedAttempts) =>
    invoke("reconnect_view", { usedAttempts }),

  /** 连接路径决策（FR-05/11）：内网→P2P→中继→NoPath。 */
  choosePath: (innerFound, hole, relayEnabled) =>
    invoke("choose_path", { innerFound, hole, relayEnabled }),
};

export default landesk;
