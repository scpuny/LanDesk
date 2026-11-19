//! 构建脚本：feature `tauri` 时跑 `tauri_build`（生成 context + 校验 tauri.conf.json）；
//! 默认（离线）不跑——避免拉 tauri-build 及其 GUI 依赖。

fn main() {
    #[cfg(feature = "tauri")]
    tauri_build::build();

    // feature 关：无操作（纯内核模式）。
}
