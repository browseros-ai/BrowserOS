// API Routes
// กำหนด HTTP API endpoints สำหรับ headless server

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { ContextEngine } from './context-engine'
import { SmartPlanner } from './smart-planner'
import { MultiAgentOrchestrator } from './multi-agent'
import { WorkflowEngine } from './workflow-engine'
import { LearningEngine } from './learning-engine'
import { FileManager } from './file-manager'
import { CostTracker } from './cost-tracker'
import { BrowserHermes } from './browser-hermes'
import { BrowserSkills } from './browser-skills'
import type { HeadlessServerConfig } from './config'

interface RouteContext {
  browser: HeadlessBrowser
  sessions: SessionManager
  config: HeadlessServerConfig
  contextEngine: ContextEngine
  smartPlanner: SmartPlanner
  multiAgent: MultiAgentOrchestrator
  workflowEngine: WorkflowEngine
  learningEngine: LearningEngine
  fileManager: FileManager
  costTracker: CostTracker
  hermes: BrowserHermes
  skills: BrowserSkills
}

/** สร้าง API routes ทั้งหมด */
export function createRoutes(ctx: RouteContext) {
  const { browser, sessions, config, contextEngine, smartPlanner, multiAgent, workflowEngine, learningEngine, fileManager, costTracker, hermes, skills } = ctx

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

    // ── Multi-Agent API ──

    /** POST /api/agents/task — ส่ง task complex */
    agentTask: async (body: { goal: string; sessionId?: string }) => {
      try {
        const job = await multiAgent.submitTask(body.goal, body.sessionId)
        return Response.json({ success: true, job })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET agents status */
    agentStatus: async () => {
      try {
        const agents = multiAgent.getAgentsStatus()
        const jobs = Array.from((multiAgent as any).activeJobs?.values?.() || [])
        return Response.json({ success: true, agents, activeJobs: jobs })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/agents/cancel — ยกเลิก job */
    agentCancel: async (body: { jobId: string }) => {
      try {
        const ok = multiAgent.cancelJob(body.jobId)
        return Response.json({ success: ok })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Workflow API ──

    /** POST /api/workflow — สร้าง workflow */
    workflowCreate: async (body: { name: string; description?: string; steps: any[]; trigger?: any; variables?: Record<string, any> }) => {
      try {
        const wf = workflowEngine.createWorkflow({
          name: body.name,
          description: body.description || '',
          trigger: body.trigger || { type: 'manual' },
          steps: body.steps,
          variables: body.variables,
        })
        return Response.json({ success: true, workflow: wf })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET all workflows */
    workflowList: async () => {
      try {
        const workflows = workflowEngine.getAllWorkflows()
        return Response.json({ success: true, workflows })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET workflow by id */
    workflowGet: async (id: string) => {
      try {
        const wf = workflowEngine.getWorkflow(id)
        if (!wf) return Response.json({ error: 'ไม่พบ workflow' }, { status: 404 })
        return Response.json({ success: true, workflow: wf })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/workflow/:id/run — รัน workflow */
    workflowRun: async (id: string) => {
      try {
        const run = await workflowEngine.runWorkflow(id)
        return Response.json({ success: true, run })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/workflow/:id/stop — หยุด workflow */
    workflowStop: async (id: string) => {
      try {
        workflowEngine.stopWorkflow(id)
        return Response.json({ success: true })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** DELETE workflow */
    workflowDelete: async (id: string) => {
      try {
        workflowEngine.deleteWorkflow(id)
        return Response.json({ success: true })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Learning API ──

    /** GET /api/learning/rules */
    learningRules: async () => {
      try {
        const rules = learningEngine.getRules()
        return Response.json({ success: true, rules })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** DELETE /api/learning/rules/:id */
    learningRuleDelete: async (id: string) => {
      try {
        learningEngine.deleteRule(id)
        return Response.json({ success: true })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET /api/learning/history */
    learningHistory: async () => {
      try {
        const history = learningEngine.getHistory()
        return Response.json({ success: true, history })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Files API ──

    /** POST /api/files/download — ดาวน์โหลดไฟล์ */
    fileDownload: async (body: { url: string; filename?: string; subdir?: string }) => {
      try {
        const result = await fileManager.downloadFile(body.url, body.filename, body.subdir)
        return Response.json({ success: true, result })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** GET /api/files/list */
    fileList: async () => {
      try {
        const files = await fileManager.listFiles()
        return Response.json({ success: true, files })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** POST /api/files/organize — จัดโฟลเดอร์ */
    fileOrganize: async (body: { subdir?: string }) => {
      try {
        const result = await fileManager.organizeFiles(body.subdir)
        return Response.json({ success: true, result })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    /** DELETE /api/files/:path */
    fileDelete: async (filePath: string) => {
      try {
        await fileManager.deleteFile(decodeURIComponent(filePath))
        return Response.json({ success: true })
      } catch (err: any) {
        return Response.json({ error: err.message }, { status: 500 })
      }
    },

    // ── Costs API ──

    /** GET /api/costs/summary */
    costSummary: async () => {
      const summary = costTracker.getSummary()
      return Response.json({ success: true, summary })
    },

    /** GET /api/costs/history */
    costHistory: async (limit?: number) => {
      const history = costTracker.getHistory(limit || 100)
      return Response.json({ success: true, history })
    },

    /** POST /api/costs/budget — ตั้งงบ */
    costSetBudget: async (body: { dailyLimit?: number; monthlyLimit?: number; alertThreshold?: number }) => {
      const budget = costTracker.setBudget(body)
      return Response.json({ success: true, budget })
    },

    /** GET /api/costs/budget */
    costGetBudget: async () => {
      const budget = costTracker.getBudget()
      return Response.json({ success: true, budget })
    },

       // ── Hermes API ──

    /** GET /api/hermes/status — สถานะ BrowserHermes ทั้งหมด */
    hermesStatus: async () => {
      return Response.json({ success: true, ...hermes.getStatus() })
    },

    /** GET /api/hermes/bug-patterns — ดู bug patterns */
    hermesBugPatterns: async () => {
      return Response.json({ success: true, patterns: hermes.bugDB.getAll() })
    },

    /** GET /api/hermes/strategies — ดู strategies */
    hermesStrategies: async () => {
      return Response.json({ success: true, strategies: hermes.strategyDB.getAll() })
    },

    /** GET /api/hermes/skills — ดู skills ทั้งหมด */
    hermesSkills: async () => {
      return Response.json({ success: true, skills: skills.getAll(), stats: skills.getStats() })
    },

    /** POST /api/hermes/skills/:name/execute — รัน skill */
    hermesSkillExecute: async (skillName: string, body: { sessionId: string; params?: Record<string, any> }) => {
      const result = await skills.execute(skillName, {
        sessionId: body.sessionId,
        browser,
        params: body.params || {},
        hermes,
      })
      return Response.json({ success: result.success, result })
    },

    /** GET /api/hermes/rules — ดู rules ที่สร้างอัตโนมัติ */
    hermesRules: async () => {
      return Response.json({ success: true, rules: hermes.selfEvolution.getAll() })
    },

    /** GET /api/hermes/stats — สถิติการเรียนรู้ */
    hermesStats: async () => {
      const hermesStats = hermes.getStats()
      const learningStats = learningEngine.getStats()
      const skillStats = skills.getStats()
      return Response.json({ success: true, hermes: hermesStats, learning: learningStats, skills: skillStats })
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

        // ── Phase 3 GET routes ──
        if (path === '/api/agents/status') return await routes.agentStatus()
        if (path === '/api/workflow') return await routes.workflowList()
        if (path.match(/^\/api\/workflow\/[^/]+$/)) {
          const id = path.split('/').pop()!
          return await routes.workflowGet(id)
        }
        if (path === '/api/learning/rules') return await routes.learningRules()
        if (path.startsWith('/api/learning/rules/')) {
          const id = path.split('/').pop()!
          return await routes.learningRuleDelete(id)
        }
        if (path === '/api/learning/history') return await routes.learningHistory()
        if (path === '/api/files/list') return await routes.fileList()
        if (path.startsWith('/api/files/') && path !== '/api/files/download' && path !== '/api/files/organize' && path !== '/api/files/list') {
          const filePath = path.replace('/api/files/', '')
          return await routes.fileDelete(filePath)
        }
        if (path === '/api/costs/summary') return await routes.costSummary()
        if (path === '/api/costs/history') return await routes.costHistory()
        if (path === '/api/costs/budget') return await routes.costGetBudget()

               // ── Hermes GET routes ──
        if (path === '/api/hermes/status') return await routes.hermesStatus()
        if (path === '/api/hermes/bug-patterns') return await routes.hermesBugPatterns()
        if (path === '/api/hermes/strategies') return await routes.hermesStrategies()
        if (path === '/api/hermes/skills') return await routes.hermesSkills()
        if (path === '/api/hermes/rules') return await routes.hermesRules()
        if (path === '/api/hermes/stats') return await routes.hermesStats()
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

        // ── Multi-Agent API ──
        if (path === '/api/agents/task') return await routes.agentTask(body)
        if (path === '/api/agents/cancel') return await routes.agentCancel(body)

        // ── Workflow API ──
        if (path === '/api/workflow' && method === 'POST') return await routes.workflowCreate(body)
        if (path.match(/^\/api\/workflow\/[^/]+\/run$/)) {
          const id = path.split('/')[3]
          return await routes.workflowRun(id)
        }
        if (path.match(/^\/api\/workflow\/[^/]+\/stop$/)) {
          const id = path.split('/')[3]
          return await routes.workflowStop(id)
        }

        // ── Learning API ──
        // (GET routes handled above)

        // ── Files API ──
        if (path === '/api/files/download') return await routes.fileDownload(body)
        if (path === '/api/files/organize') return await routes.fileOrganize(body)

        // ── Costs API ──
        if (path === '/api/costs/budget' && method === 'POST') return await routes.costSetBudget(body)

               // ── Hermes Skill Execute ──
        if (path.match(/^\/api\/hermes\/skills\/[^/]+\/execute$/)) {
          const skillName = path.split('/')[4]
          return await routes.hermesSkillExecute(skillName, body)
        }
      }

      // DELETE method routes
      if (method === 'DELETE') {
        if (path === '/api/session') return await routes.destroySession(await req.json().catch(() => ({})))
        if (path.match(/^\/api\/workflow\/[^/]+$/)) {
          const id = path.split('/').pop()!
          return await routes.workflowDelete(id)
        }
        if (path.startsWith('/api/learning/rules/')) {
          const id = path.split('/').pop()!
          return await routes.learningRuleDelete(id)
        }
        if (path.startsWith('/api/files/') && path !== '/api/files/list' && path !== '/api/files/download' && path !== '/api/files/organize') {
          const filePath = path.replace('/api/files/', '')
          return await routes.fileDelete(filePath)
        }
      }

      return Response.json({ error: 'ไม่พบ endpoint', path }, { status: 404 })
    } catch (err: any) {
      console.error(`[API Error] ${method} ${path}:`, err)
      return Response.json({ error: 'เกิดข้อผิดพลาดภายใน server', details: err.message }, { status: 500 })
    }
  }
}
