import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// LanDesk 前端 dev server（离线/本地开发）。
// 产物 dist/ 供 Tauri 壳（ui/src-tauri）打包时引用（frontendDist）。
export default defineConfig({
  plugins: [vue()],
  server: {
    host: "127.0.0.1",
    port: 5173,
  },
  build: {
    outDir: "dist",
    // dist/ 已 gitignore，不入库
  },
});
