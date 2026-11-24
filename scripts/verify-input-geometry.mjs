// 画面几何 / 鼠标按键编码离线断言（无需浏览器 / Tauri）。
// 用法：ui/frontend 下 `npm run verify:input`。
import { readFileSync } from 'node:fs'
import assert from 'node:assert/strict'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const esbuild = require('../ui/frontend/node_modules/esbuild')
const source = readFileSync(new URL('../ui/frontend/src/lib/video-geometry.ts', import.meta.url), 'utf8')
const compiled = esbuild.transformSync(source, { loader: 'ts', format: 'esm', target: 'es2022' }).code
const { contentRect, normPoint, protocolButton, buttonBit } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
)

let passed = 0
function check(name, fn) {
  fn()
  passed++
  console.log(`  ✓ ${name}`)
}
const near = (a, b, tol = 1) => assert.ok(Math.abs(a - b) <= tol, `${a} 与 ${b} 相差超过 ${tol}`)

console.log('输入几何断言：')

check('自适应：宽盒子按高度留上下黑边', () => {
  const r = contentRect({ left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '自适应')
  near(r.width, 1000); near(r.height, 562.5); near(r.left, 0); near(r.top, 18.75)
})

check('自适应：高盒子按宽度留左右黑边', () => {
  const r = contentRect({ left: 0, top: 0, width: 900, height: 900 }, { width: 1920, height: 1080 }, '自适应')
  near(r.width, 900); near(r.height, 506.25); near(r.left, 0); near(r.top, 196.875)
})

check('原始尺寸：1:1 居中，超出盒子部分被裁切', () => {
  const r = contentRect({ left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '原始尺寸')
  near(r.width, 1920); near(r.height, 1080); near(r.left, -460); near(r.top, -240)
})

check('自适应：画面中心 → 65535 中点', () => {
  const p = normPoint(500, 300, { left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '自适应')
  assert.ok(p)
  near(p.x, 32768); near(p.y, 32768)
})

check('自适应：黑边点击不转发', () => {
  assert.equal(normPoint(500, 5, { left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '自适应'), null)
})

check('原始尺寸：盒子中心仍映射到画面中心（历史 bug 会偏移到角落）', () => {
  const p = normPoint(500, 300, { left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '原始尺寸')
  assert.ok(p)
  near(p.x, 32768); near(p.y, 32768)
})

check('原始尺寸：被裁掉的画面区域坐标仍连续', () => {
  const p = normPoint(0, 0, { left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '原始尺寸')
  assert.ok(p)
  near(p.x, 15701); near(p.y, 14563)
})

check('原始尺寸：画面小于盒子时黑边点击不转发', () => {
  assert.equal(normPoint(10, 10, { left: 0, top: 0, width: 2400, height: 1400 }, { width: 1920, height: 1080 }, '原始尺寸'), null)
})

check('归一化坐标可逆（与受控端 denormalize 对齐）', () => {
  const p = normPoint(750, 300, { left: 0, top: 0, width: 1000, height: 600 }, { width: 1920, height: 1080 }, '自适应')
  assert.ok(p)
  near(Math.floor((p.x * 1920) / 65535), 1440)
})

check('DOM 鼠标键码 → 协议码（右键 2→1，中键 1→2）', () => {
  assert.equal(protocolButton(0), 0)
  assert.equal(protocolButton(2), 1)
  assert.equal(protocolButton(1), 2)
  assert.equal(protocolButton(3), null)
})

check('DOM 鼠标键码 → buttons 位掩码', () => {
  assert.equal(buttonBit(0), 1)
  assert.equal(buttonBit(2), 2)
  assert.equal(buttonBit(1), 4)
  assert.equal(buttonBit(3), 0)
})

// 渲染回归护栏：canvas 尺寸设置绝不能出现在 VideoDecoder output 回调里。
// 设置 canvas.width/height 会清空后备存储并重置 2D context，WKWebView 下随后
// drawImage 静默失败 → 整屏全黑（v5 实测）。尺寸必须在模板静态声明、decode 之前同步。
check('渲染护栏：output 回调不写 canvas.width/height，canvas 内禀为 1920×1080', () => {
  const vue = readFileSync(new URL('../ui/frontend/src/views/Session.vue', import.meta.url), 'utf8')
  const outputBody = vue.slice(vue.indexOf('output: (frame: VideoFrame)'), vue.indexOf("frame.close()"))
  assert.ok(!/canvas\.width\s*=/.test(outputBody), 'output 回调里禁止写 canvas.width')
  assert.ok(!/canvas\.height\s*=/.test(outputBody), 'output 回调里禁止写 canvas.height')
  assert.ok(!/\bc\.width\s*=/.test(outputBody), 'output 回调里禁止写 c.width')
  assert.ok(!/\bc\.height\s*=/.test(outputBody), 'output 回调里禁止写 c.height')
  assert.match(vue, /width="1920"\s+height="1080"/, 'canvas 内禀应为 1920×1080')
})

console.log(`\n${passed}/${passed} 断言通过`)
