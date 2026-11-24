#!/usr/bin/env node
/**
 * 诊断包生成器回归验证（零新依赖）。
 *
 * 前端没有测试框架，这里复用已有的 typescript 把纯函数模块转译成临时 .mjs 后断言。
 * 不碰 DOM、不碰宿主 API，验证的是 buildDiagnosticReport / diagnosticFilename 的输出契约：
 * 版本注入、密码脱敏、可疑事件筛选、全量日志、时间格式、文件名格式、空输入鲁棒性。
 *
 * 用法：node scripts/verify-diagnostics.mjs
 */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const ts = require('../ui/frontend/node_modules/typescript')
const root = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..')
const srcPath = path.join(root, 'ui/frontend/src/lib/diagnostics.ts')

const src = fs.readFileSync(srcPath, 'utf8')
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2020,
  },
  fileName: srcPath,
}).outputText

const tmp = path.join(os.tmpdir(), `landesk-diag-${process.pid}.mjs`)
fs.writeFileSync(tmp, js, 'utf8')

let pass = 0
function check(name, fn) {
  fn()
  pass += 1
  console.log(`  PASS  ${name}`)
}

try {
  const mod = await import(pathToFileURL(tmp).href)
  const { buildDiagnosticReport, diagnosticFilename } = mod

  const now = '2026-11-24 19:13:34'
  const logs = [
    { t: 1763987612345000, level: 'INFO', mod: 'session', msg: 'connected', kv: 'target=192.168.1.10:52000' },
    { t: 1763987612350000, level: 'WARN', mod: 'video_pump', msg: 'capture degraded', kv: 'reason=no_permission' },
    { t: 1763987612360000, level: 'ERROR', mod: 'quic', msg: 'timeout', kv: 'stage=handshake' },
  ]
  const input = {
    version: '0.5.6-test',
    platform: 'macos',
    userAgent: 'Mozilla/5.0 (Macintosh)',
    generatedAt: now,
    local: { name: 'MacBook-Pro', deviceCode: 'AB12CD34EF56', online: true, mdns: true, signaling: '', relay: false, enc: ['openh264'] },
    controlled: { serving: true, port: 52000, deviceCode: 'AB12CD34EF56', tempCodeEnabled: true },
    config: { password: 'SUPER_SECRET_SHOULD_NOT_APPEAR', preset: 'balanced', dirty_rect: true },
    logs,
    preview: true,
  }
  const report = buildDiagnosticReport(input)

  check('版本号被写入报告', () => assert.match(report, /应用版本: 0\.5\.6-test/))
  check('预览态被明确标注（不冒充真机证据）', () => assert.match(report, /浏览器预览（mock，非真机）/))
  check('访问密码脱敏（不出现明文）', () => {
    assert.ok(!report.includes('SUPER_SECRET_SHOULD_NOT_APPEAR'))
    assert.match(report, /访问密码: 已设置/)
  })
  check('机器信息进入快照', () => {
    assert.match(report, /设备名: MacBook-Pro/)
    assert.match(report, /监听端口: 52000/)
    assert.match(report, /编码器: openh264/)
  })
  check('可疑事件筛出降级/超时', () => {
    assert.match(report, /capture degraded/)
    assert.match(report, /timeout/)
    assert.match(report, /---- 可疑事件（最近 2 条）----/)
  })
  check('全部日志条数正确且时间格式微秒可读', () => {
    assert.match(report, /---- 全部日志（3 条）----/)
    assert.match(report, /\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}\]/)
  })
  check('未设置密码显示未设置', () => {
    const r = buildDiagnosticReport({ ...input, config: { ...input.config, password: '' } })
    assert.match(r, /访问密码: 未设置/)
  })
  check('空日志不崩且明确标注', () => {
    const r = buildDiagnosticReport({ ...input, logs: [] })
    assert.match(r, /---- 全部日志（0 条）----/)
    assert.match(r, /（无明显降级\/失败\/超时关键词）/)
  })
  check('文件名格式 landsdk-diagnostic-YYYYMMDD-HHMMSS.txt', () => {
    assert.equal(diagnosticFilename(new Date(2026, 10, 24, 19, 13, 34)), 'landsdk-diagnostic-20261124-191334.txt')
  })

  console.log(`\n诊断包验证通过：${pass}/${pass} 项`)
} finally {
  fs.unlinkSync(tmp)
}
