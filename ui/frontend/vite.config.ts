import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath } from 'node:url'

// LanDesk 前端构建配置（架构对齐 lanLink）。
// 产物 dist/ 供 Tauri 壳（ui/src-tauri）打包时引用；浏览器态可直接 npm run dev 预览。
//
// 多页应用（MPA）：两个入口——
//   - index.html → src/main.ts   （主窗 label "main"：远程控制/我的设备/日志/设置）
//   - session.html → src/session.ts （会话独立 OS 窗 label "session"：完整 最小化/最大化/关闭 + 画面 + 键鼠）
// Tauri `frontendDist` 指向 dist/；主窗默认载 index.html，会话窗经 `WebviewWindowBuilder` 载 session.html。
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  base: './',
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
  },
  build: {
    target: 'es2020',
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL('./index.html', import.meta.url)),
        session: fileURLToPath(new URL('./session.html', import.meta.url)),
      },
    },
  },
})

