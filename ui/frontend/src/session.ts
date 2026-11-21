import { createApp } from 'vue'
import SessionApp from './views/SessionApp.vue'

// 会话独立 OS 窗体（label "session"；macOS 原生交通灯，Win/Linux 自绘三键）。
// 与主窗 index.html 同构但只挂 SessionApp（沉浸会话视图 + 窗口控制）。
// 主题系统 CSS 加载顺序与 main.ts 一致：原语+语义 → 视觉风格覆盖 → reset/窗口 → UI 原语 → 业务布局
import './styles/tokens.css'
import './styles/palettes.css'
import './styles/base.css'
import './styles/ui.css'
import './styles/app.css'

createApp(SessionApp).mount('#app')
