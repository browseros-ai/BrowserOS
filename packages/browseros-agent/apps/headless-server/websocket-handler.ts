// WebSocket Handler
// จัดการ WebSocket connections สำหรับ real-time communication
// รองรับคำสั่งเดียวกับ HTTP API แต่ผ่าน WebSocket แทน

import { HeadlessBrowser } from './headless-browser'
import { SessionManager, type BrowserSession } from './session-manager'

interface WSMessage {
  id: string
  action: string
  data: Record<string, any>
}

interface WSResponse {
  id: string
  action: string
  success: boolean
  data?: any
  error?: string
}

/**
 * สร้าง WebSocket handler สำหรับจัดการ real-time browser control
 */
export function createWebSocketHandler(browser: HeadlessBrowser, sessions: SessionManager) {
  /** จัดการข้อความที่รับเข้ามา */
  async function handleMessage(
    ws: any,
    raw: string,
  ): Promise<void> {
    let msg: WSMessage
    try {
      msg = JSON.parse(raw)
    } catch {
      send(ws, { id: '0', action: 'error', success: false, error: 'JSON ไม่ถูกต้อง' })
      return
    }

    const { id, action, data } = msg

    try {
      let result: any

      switch (action) {
        // สร้าง session
        case 'session.create': {
          const session = await sessions.create(data.sessionId, data.startUrl)
          result = { sessionId: session.id, tabId: session.tabId }
          break
        }

        // ปิด session
        case 'session.destroy': {
          await sessions.destroy(data.sessionId)
          result = { destroyed: true }
          break
        }

        // นำทางไปยัง URL
        case 'navigate': {
          const session = getSession(data.sessionId)
          const nav = await browser.navigate(session.tabId, data.url)
          if (data.wait !== false) {
            await browser.waitForLoad(session.tabId)
          }
          const currentUrl = await browser.getCurrentUrl(session.tabId)
          const title = await browser.getTitle(session.tabId)
          sessions.addHistory(session, 'navigate', data.url, currentUrl)
          result = { url: currentUrl, title, frameId: nav.frameId }
          break
        }

        // คลิก element
        case 'click': {
          const session = getSession(data.sessionId)
          const clicked = await browser.click(session.tabId, data.selector)
          await new Promise((r) => setTimeout(r, 500))
          const currentUrl = await browser.getCurrentUrl(session.tabId)
          sessions.addHistory(session, 'click', undefined, data.selector, clicked)
          result = { clicked, currentUrl }
          break
        }

        // พิมพ์ข้อความ
        case 'type': {
          const session = getSession(data.sessionId)
          const typed = await browser.type(session.tabId, data.selector, data.text)
          sessions.addHistory(session, 'type', undefined, data.selector, typed)
          result = { typed }
          break
        }

        // จับภาพหน้าจอ
        case 'screenshot': {
          const session = getSession(data.sessionId)
          const base64 = await browser.screenshot(session.tabId, {
            quality: data.quality,
            fullPage: data.fullPage,
          })
          sessions.addHistory(session, 'screenshot')
          result = { image: `data:image/png;base64,${base64}` }
          break
        }

        // ดึงข้อมูล
        case 'extract': {
          const session = getSession(data.sessionId)
          let extracted: any
          if (data.expression) {
            extracted = await browser.evaluate(session.tabId, data.expression)
          } else if (data.type === 'html') {
            extracted = await browser.extractHTML(session.tabId, data.selector || 'body')
          } else {
            extracted = await browser.extractText(session.tabId, data.selector)
          }
          sessions.addHistory(session, 'extract')
          result = { data: extracted }
          break
        }

        // กรอกฟอร์ม
        case 'fillForm': {
          const session = getSession(data.sessionId)
          const filled = await browser.fillForm(session.tabId, data.fields)
          if (data.submitSelector) {
            await browser.click(session.tabId, data.submitSelector)
            await new Promise((r) => setTimeout(r, 1000))
          }
          sessions.addHistory(session, 'fillForm')
          result = { results: filled }
          break
        }

        // รัน JavaScript
        case 'evaluate': {
          const session = getSession(data.sessionId)
          result = { result: await browser.evaluate(session.tabId, data.expression) }
          sessions.addHistory(session, 'evaluate')
          break
        }

        // ดึงสถานะ
        case 'status': {
          const tabs = await browser.getTabs()
          const allSessions = sessions.getAll()
          result = {
            chromium: { running: true, tabs: tabs.length },
            sessions: allSessions.map((s) => ({
              id: s.id,
              tabId: s.tabId,
              historyCount: s.history.length,
              lastActivity: s.lastActivity,
            })),
            uptime: process.uptime(),
          }
          break
        }

        default:
          send(ws, {
            id,
            action,
            success: false,
            error: `ไม่รู้จักคำสั่ง: ${action}`,
          })
          return
      }

      send(ws, { id, action, success: true, data: result })
    } catch (err: any) {
      send(ws, { id, action, success: false, error: err.message })
    }
  }

  /** ดึง session จาก ID หรือโยน error */
  function getSession(sessionId: string): BrowserSession {
    const session = sessions.get(sessionId)
    if (!session) throw new Error(`ไม่พบ session ${sessionId}`)
    return session
  }

  /** ส่งข้อความผ่าน WebSocket */
  function send(ws: any, response: WSResponse): void {
    try {
      ws.send(JSON.stringify(response))
    } catch {
      // WebSocket ปิดไปแล้ว
    }
  }

  return { handleMessage }
}
