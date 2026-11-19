import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'

// LanDesk 前端构建配置（架构对齐 lanLink）。
// 产物 dist/ 供 Tauri 壳（ui/src-tauri）打包时引用；浏览器态可直接 npm run dev 预览。
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
  },
})
