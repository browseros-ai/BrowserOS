#!/usr/bin/env bun
/**
 * BrowserOS Headless Server — Entry Point
 * 
 * รัน BrowserOS บน Linux server ได้โดยไม่ต้องมี GUI
 * ควบคุม Chromium ผ่าน CDP (Chrome DevTools Protocol)
 * รับคำสั่งผ่าน HTTP API และ WebSocket
 * 
 * วิธีใช้:
 *   bun run packages/browseros-agent/apps/headless-server/index.ts
 *   # หรือ
 *   HEADLESS_PORT=3100 bun run packages/browseros-agent/apps/headless-server/index.ts
 */

import { loadConfig } from './config'
import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { createRoutes, createRouter } from './api-routes'
import { createWebSocketHandler } from './websocket-handler'

// โหลดการตั้งค่า
const config = loadConfig()

console.log('═══════════════════════════════════════════════')
console.log('  🖥️  BrowserOS Headless Server')
console.log('═══════════════════════════════════════════════')
console.log(`  พอร์ต:      ${config.port}`)
console.log(`  โฮสต์:      ${config.host}`)
console.log(`  Chromium:   ${config.chromiumPath}`)
console.log(`  Viewport:   ${config.viewportWidth}x${config.viewportHeight}`)
console.log(`  Debug:      ${config.debug ? 'เปิด' : 'ปิด'}`)
console.log(`  User Data:  ${config.userDataDir}`)
console.log('═══════════════════════════════════════════════')

// สร้าง browser controller และ session manager
const browser = new HeadlessBrowser(config)
const sessions = new SessionManager(browser)

// เริ่ม Chromium
console.log('\n⏳ กำลังเริ่ม Chromium...')
await browser.start()

// สร้าง API routes
const routes = createRoutes({ browser, sessions, config })
const router = createRouter(routes, config)

// สร้าง WebSocket handler
const wsHandler = createWebSocketHandler(browser, sessions)

// เก็บ active WebSocket connections
const wsClients = new Set<any>()

// สร้าง Bun HTTP server พร้อม WebSocket upgrade
const server = Bun.serve({
  port: config.port,
  hostname: config.host,

  fetch(req, server) {
    const url = new URL(req.url)

    // WebSocket upgrade
    if (url.pathname === '/ws') {
      const upgraded = server.upgrade(req)
      if (upgraded) {
        return undefined as any // WebSocket handled
      }
      return new Response('WebSocket upgrade failed', { status: 500 })
    }

    // HTTP API
    return router(req)
  },

  websocket: {
    open(ws) {
      wsClients.add(ws)
      console.log(`[WS] ลูกค้าเชื่อมต่อ (ทั้งหมด: ${wsClients.size})`)
    },

    async message(ws, raw) {
      const text = typeof raw === 'string' ? raw : raw.toString()
      await wsHandler.handleMessage(ws, text)
    },

    close(ws) {
      wsClients.delete(ws)
      console.log(`[WS] ลูกค้าตัดการเชื่อมต่อ (เหลือ: ${wsClients.size})`)
    },
  },
})

console.log(`\n✅ Server พร้อมที่ http://${config.host}:${config.port}`)
console.log(`   HTTP API:     http://${config.host}:${config.port}/api/status`)
console.log(`   WebSocket:    ws://${config.host}:${config.port}/ws`)
console.log('')

// จัดการเมื่อปิด server
process.on('SIGINT', async () => {
  console.log('\n\n⏹️  กำลังปิด server...')
  await sessions.destroyAll()
  await browser.stop()
  server.stop()
  console.log('👋 ปิด server แล้ว')
  process.exit(0)
})

process.on('SIGTERM', async () => {
  console.log('\n⏹️  ได้รับสัญญาณ SIGTERM...')
  await sessions.destroyAll()
  await browser.stop()
  server.stop()
  process.exit(0)
})

// ทำความสะอาด session ที่ไม่ได้ใช้ทุก 30 นาที
setInterval(
  async () => {
    const cleaned = await sessions.cleanupIdleSessions(30 * 60 * 1000) // 30 นาที
    if (cleaned > 0) {
      console.log(`[Cleanup] ลบ session ที่ไม่ได้ใช้ ${cleaned} รายการ`)
    }
  },
  30 * 60 * 1000,
)
