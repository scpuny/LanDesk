import { createApp } from 'vue'
import App from './App.vue'

// 主题系统 CSS 加载顺序：原语+语义 → 视觉风格覆盖 → reset/窗口 → UI 原语 → 业务布局
import './styles/tokens.css'
import './styles/palettes.css'
import './styles/base.css'
import './styles/ui.css'
import './styles/app.css'

createApp(App).mount('#app')
