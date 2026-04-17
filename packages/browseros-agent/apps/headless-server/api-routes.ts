// API Routes
// กำหนด HTTP API endpoints สำหรับ headless server

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { ContextEngine } from './context-engine'
import { SmartPlanner } from './smart-planner'
import type { HeadlessServerConfig } from './config'

interface RouteContext {
  browser: HeadlessBrowser
  sessions: SessionManager
  config: HeadlessServerConfig
  contextEngine: ContextEngine
  smartPlanner: SmartPlanner
}

/** สร้าง API routes ทั้งหมด */
export function createRoutes(ctx: RouteContext) {
  const { browser, sessions, config, contextEngine, smartPlanner } = ctx

  return {
    /** GET /api/status — สถานะ server */
    status: async () => {
      const tabs = await browser.getTabs()
      const allSessions = sessions.getAll()
      return Response.json({
        status: 'ok',
        chromium: { running: true, tabs: tabs.length },
        sessions: allSessions.map((s) => ({
          id: s.id,
          tabId: s.tabId,
          url: '—',
          createdAt: s.createdAt,
          lastActivity: s.lastActivity,
          historyCount: s.history.length,
        })),
        uptime: process.uptime(),
      })
    },

    /** POST /api/navigate — เปิดเว็บ */
    navigate: async (body: { url: string; sessionId?: string; wait?: boolean }) => {
      const { url, sessionId, wait = true } = body
      const session = sessionId
        ? sessions.get(sessionId)
        : await sessions.create()

      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const result = await browser.navigate(session.tabId, url)
        if (wait) {
          await browser.waitForLoad(session.tabId)
        }
        const currentUrl = await browser.getCurrentUrl(session.tabId)
        const title = await browser.getTitle(session.tabId)
        sessions.addHistory(session, 'navigate', url, `ไปที่ ${currentUrl}`)
        
        return Response.json({
          success: true,
          sessionId: session.id,
          url: currentUrl,
          title,
          frameId: result.frameId,
        })
      } catch (err: any) {
        sessions.addHistory(session, 'navigate', url, err.message, false)
        return Response.json({ error: err.message, sessionId: session.id }, { status: 500 })
      }
    },

    /** POST /api/click — คลิก element */
    click: async (body: { sessionId: string; selector: string }) => {
      const { sessionId, selector } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const clicked = await browser.click(session.tabId, selector)
        sessions.addHistory(session, 'click', undefined, `คลิก ${selector}`, clicked)
        
        // รอให้หน้าตอบสนอง
        await new Promise((r) => setTimeout(r, 500))
        const currentUrl = await browser.getCurrentUrl(session.tabId)
        
        return Response.json({ success: clicked, selector, currentUrl })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/type — พิมพ์ข้อความ */
    type: async (body: { sessionId: string; selector: string; text: string }) => {
      const { sessionId, selector, text } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const typed = await browser.type(session.tabId, selector, text)
        sessions.addHistory(session, 'type', undefined, `พิมพ์ "${text.substring(0, 50)}" ที่ ${selector}`, typed)
        return Response.json({ success: typed, selector, text })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/screenshot — จับภาพหน้าจอ */
    screenshot: async (body: {
      sessionId: string
      format?: 'base64' | 'binary'
      quality?: number
      fullPage?: boolean
    }) => {
      const { sessionId, format = 'base64', quality, fullPage } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const base64 = await browser.screenshot(session.tabId, { quality, fullPage })
        sessions.addHistory(session, 'screenshot', undefined, 'จับภาพหน้าจอสำเร็จ')

        if (format === 'binary') {
          // ส่งเป็น PNG binary
          const binary = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0))
          return new Response(binary, {
            headers: { 'Content-Type': 'image/png' },
          })
        }

        return Response.json({
          success: true,
          image: `data:image/png;base64,${base64}`,
        })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/extract — ดึงข้อมูลจากหน้าเว็บ */
    extract: async (body: {
      sessionId: string
      selector?: string
      type?: 'text' | 'html' | 'json'
      expression?: string
    }) => {
      const { sessionId, selector, type = 'text', expression } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        let data: any

        if (expression) {
          // รัน JavaScript expression
          data = await browser.evaluate(session.tabId, expression)
        } else if (type === 'html') {
          data = await browser.extractHTML(session.tabId, selector || 'body')
        } else {
          data = await browser.extractText(session.tabId, selector)
        }

        sessions.addHistory(session, 'extract', undefined, `ดึงข้อมูล ${type} สำเร็จ`)
        return Response.json({ success: true, data })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/session — สร้าง session ใหม่ */
    createSession: async (body: { sessionId?: string; startUrl?: string }) => {
      try {
        const session = await sessions.create(body.sessionId, body.startUrl)
        return Response.json({
          success: true,
          sessionId: session.id,
          tabId: session.tabId,
        })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** DELETE /api/session — ปิด session */
    destroySession: async (body: { sessionId: string }) => {
      const { sessionId } = body
      await sessions.destroy(sessionId)
      return Response.json({ success: true, sessionId })
    },

    /** POST /api/fill-form — กรอกฟอร์ม */
    fillForm: async (body: {
      sessionId: string
      fields: Record<string, string>
      submitSelector?: string
    }) => {
      const { sessionId, fields, submitSelector } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const results = await browser.fillForm(session.tabId, fields)

        // กด submit ถ้าระบุ
        if (submitSelector) {
          await browser.click(session.tabId, submitSelector)
          await new Promise((r) => setTimeout(r, 1000))
        }

        sessions.addHistory(session, 'fillForm', undefined, 'กรอกฟอร์มเสร็จ')
        return Response.json({ success: true, results })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/evaluate — รัน JavaScript */
    evaluate: async (body: { sessionId: string; expression: string }) => {
      const { sessionId, expression } = body
      const session = sessions.get(sessionId)
      if (!session) {
        return Response.json({ error: 'ไม่พบ session' }, { status: 404 })
      }

      try {
        const result = await browser.evaluate(session.tabId, expression)
        sessions.addHistory(session, 'evaluate', undefined, 'รัน JS เสร็จ')
        return Response.json({ success: true, result })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET /api/cookies — ดึง cookies */
    getCookies: async (sessionId: string) => {
      try {
        const cookies = await sessions.getCookies(sessionId)
        return Response.json({ success: true, cookies })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/cookies — ตั้ง cookies */
    setCookies: async (body: { sessionId: string; cookies: any[] }) => {
      const { sessionId, cookies } = body
      try {
        await sessions.setCookies(sessionId, cookies)
        return Response.json({ success: true })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Context API ──

    /** POST /api/context — ดึง context หน้าเว็บปัจจุบัน */
    context: async (body: { sessionId: string }) => {
      try {
        const ctx = await contextEngine.extractContext(body.sessionId)
        return Response.json({ success: true, context: ctx })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Plan API ──

    /** POST /api/plan — สร้าง plan จากคำสั่ง */
    createPlan: async (body: { sessionId: string; goal: string }) => {
      try {
        const plan = await smartPlanner.createPlan(body.sessionId, body.goal)
        return Response.json({ success: true, plan })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/plan/execute — execute plan */
    executePlan: async (body: { planId: string }) => {
      try {
        const plan = await smartPlanner.executePlan(body.planId)
        return Response.json({ success: true, plan })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/plan/cancel — ยกเลิก plan */
    cancelPlan: async (body: { planId: string }) => {
      try {
        const plan = smartPlanner.cancelPlan(body.planId)
        return Response.json({ success: true, plan })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET plan status */
    planStatus: async (planId: string) => {
      const plan = smartPlanner.getPlan(planId)
      if (!plan) return Response.json({ error: 'ไม่พบแผน' }, { status: 404 })
      return Response.json({ success: true, plan })
    },
  }
}

/** สร้าง HTTP router จาก routes */
export function createRouter(routes: ReturnType<typeof createRoutes>, config: HeadlessServerConfig) {
  return async (req: Request): Promise<Response> => {
    const url = new URL(req.url)
    const path = url.pathname
    const method = req.method

    // ตรวจสอบ API key (ถ้าตั้งไว้)
    if (config.apiKey) {
      const authHeader = req.headers.get('Authorization')
      const key = authHeader?.replace('Bearer ', '')
      if (key !== config.apiKey) {
        return Response.json({ error: 'ไม่ได้รับอนุญาต' }, { status: 401 })
      }
    }

    try {
      // GET routes
      if (method === 'GET') {
        if (path === '/api/status') return await routes.status()
        if (path.startsWith('/api/cookies/')) {
          const sessionId = path.split('/').pop()!
          return await routes.getCookies(sessionId)
        }
        if (path === '/api/plan/status') {
          const planId = url.searchParams.get('planId') || ''
          return await routes.planStatus(planId)
        }
        if (path.startsWith('/api/context/')) {
          const sessionId = path.split('/').pop()!
          return await routes.context({ sessionId })
        }
      }

      // POST / DELETE routes
      if (method === 'POST' || method === 'DELETE') {
        const body = method === 'POST' ? await req.json() : await req.json().catch(() => ({}))

        if (path === '/api/navigate') return await routes.navigate(body)
        if (path === '/api/click') return await routes.click(body)
        if (path === '/api/type') return await routes.type(body)
        if (path === '/api/screenshot') return await routes.screenshot(body)
        if (path === '/api/extract') return await routes.extract(body)
        if (path === '/api/session' && method === 'POST') return await routes.createSession(body)
        if (path === '/api/session' && method === 'DELETE') return await routes.destroySession(body)
        if (path === '/api/fill-form') return await routes.fillForm(body)
        if (path === '/api/evaluate') return await routes.evaluate(body)
        if (path === '/api/cookies' && method === 'POST') return await routes.setCookies(body)

        // Context & Planner API
        if (path === '/api/context') return await routes.context(body)
        if (path === '/api/plan' && method === 'POST') return await routes.createPlan(body)
        if (path === '/api/plan/execute') return await routes.executePlan(body)
        if (path === '/api/plan/cancel') return await routes.cancelPlan(body)
      }

      return Response.json({ error: 'ไม่พบ endpoint', path }, { status: 404 })
    } catch (err: any) {
      console.error(`[API Error] ${method} ${path}:`, err)
      return Response.json({ error: 'เกิดข้อผิดพลาดภายใน server', details: err.message }, { status: 500 })
    }
  }
}
