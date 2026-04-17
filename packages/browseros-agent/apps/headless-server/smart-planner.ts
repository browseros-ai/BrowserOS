// Smart Planner — รับคำสั่งภาษาธรรมดา วางแผนเป็น steps แล้ว execute อัตโนมัติ
// ใช้ LLM (GLM) ในการวางแผน

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { ContextEngine } from './context-engine'

export type PlanStatus = 'draft' | 'confirmed' | 'running' | 'completed' | 'failed' | 'cancelled'

export interface PlanStep {
  id: number
  action: string       // navigate, click, type, extract, wait, evaluate, ask_user
  description: string  // อธิบายเป็นภาษาไทย
  params: Record<string, any>
  status: 'pending' | 'running' | 'done' | 'error' | 'skipped'
  result?: any
  error?: string
  retries: number
}

export interface Plan {
  id: string
  sessionId: string
  goal: string          // คำสั่งเดิมของ user
  steps: PlanStep[]
  status: PlanStatus
  currentStep: number
  createdAt: number
  updatedAt: number
  completedAt?: number
}

/** สร้าง plan ID */
function planId(): string {
  return `plan_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

/**
 * Smart Planner — วางแผนและ execute อัตโนมัติ
 * - รับคำสั่งภาษาธรรมดา → ส่งให้ LLM วางแผนเป็น steps
 * - แสดง plan ให้ user ยืนยัน
 * - Execute ทีละ step อัตโนมัติ
 * - ลองแก้ error 3 ครั้ง
 */
export class SmartPlanner {
  private browser: HeadlessBrowser
  private sessions: SessionManager
  private contextEngine: ContextEngine
  private activePlans = new Map<string, Plan>()
  private llmEndpoint: string
  private llmModel: string

  constructor(
    browser: HeadlessBrowser,
    sessions: SessionManager,
    contextEngine: ContextEngine,
    opts?: { llmEndpoint?: string; llmModel?: string }
  ) {
    this.browser = browser
    this.sessions = sessions
    this.contextEngine = contextEngine
    this.llmEndpoint = opts?.llmEndpoint || process.env.LLM_ENDPOINT || 'https://open.bigmodel.cn/api/paas/v4/chat/completions'
    this.llmModel = opts?.llmModel || process.env.LLM_MODEL || 'glm-4-flash'
  }

  /** สร้างแผนจากคำสั่งภาษาธรรมดา */
  async createPlan(sessionId: string, goal: string): Promise<Plan> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error(`ไม่พบ session ${sessionId}`)

    // ดึง context หน้าเว็บปัจจุบัน
    let contextStr = ''
    try {
      await this.contextEngine.extractContext(sessionId)
      contextStr = this.contextEngine.buildAISummary(sessionId)
    } catch {
      contextStr = 'ไม่สามารถอ่านบริบทหน้าเว็บได้'
    }

    // สร้าง prompt สำหรับ LLM
    const prompt = this.buildPlanningPrompt(goal, contextStr)

    // เรียก LLM
    const steps = await this.callLLMForPlan(prompt)

    const plan: Plan = {
      id: planId(),
      sessionId,
      goal,
      steps,
      status: 'draft',
      currentStep: 0,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }

    this.activePlans.set(plan.id, plan)
    return plan
  }

  /** ยืนยันและเริ่ม execute plan */
  async executePlan(planId: string): Promise<Plan> {
    const plan = this.activePlans.get(planId)
    if (!plan) throw new Error(`ไม่พบแผน ${planId}`)
    if (plan.status !== 'draft' && plan.status !== 'confirmed') {
      throw new Error(`แผนอยู่ในสถานะ ${plan.status} ไม่สามารถ execute ได้`)
    }

    plan.status = 'running'
    plan.updatedAt = Date.now()

    // Execute ทีละ step
    for (let i = 0; i < plan.steps.length; i++) {
      if (plan.status !== 'running') break // อาจถูก cancel

      plan.currentStep = i
      const step = plan.steps[i]
      step.status = 'running'

      const maxRetries = 3
      let success = false

      for (let retry = 0; retry < maxRetries && !success; retry++) {
        try {
          step.result = await this.executeStep(plan, step)
          step.status = 'done'
          success = true
        } catch (err: any) {
          step.retries = retry + 1
          step.error = err.message
          if (retry < maxRetries - 1) {
            // รอก่อนลองใหม่
            await new Promise(r => setTimeout(r, 1000 * (retry + 1)))
          }
        }
      }

      if (!success) {
        step.status = 'error'
        plan.status = 'failed'
        plan.updatedAt = Date.now()
        return plan
      }

      plan.updatedAt = Date.now()
    }

    plan.status = 'completed'
    plan.completedAt = Date.now()
    plan.updatedAt = Date.now()
    return plan
  }

  /** ยกเลิก plan */
  cancelPlan(planId: string): Plan {
    const plan = this.activePlans.get(planId)
    if (!plan) throw new Error(`ไม่พบแผน ${planId}`)
    plan.status = 'cancelled'
    plan.updatedAt = Date.now()
    return plan
  }

  /** ดึงสถานะ plan */
  getPlan(planId: string): Plan | undefined {
    return this.activePlans.get(planId)
  }

  /** ดึง plan ของ session */
  getSessionPlans(sessionId: string): Plan[] {
    return Array.from(this.activePlans.values())
      .filter(p => p.sessionId === sessionId)
  }

  // ── Private ──

  /** Execute หนึ่ง step */
  private async executeStep(plan: Plan, step: PlanStep): Promise<any> {
    const session = this.sessions.get(plan.sessionId)
    if (!session) throw new Error('session หาย')

    switch (step.action) {
      case 'navigate': {
        const r = await this.browser.navigate(session.tabId, step.params.url)
        await this.browser.waitForLoad(session.tabId)
        // อ่าน context ใหม่หลังเปลี่ยนหน้า
        await this.contextEngine.extractContext(plan.sessionId)
        const url = await this.browser.getCurrentUrl(session.tabId)
        const title = await this.browser.getTitle(session.tabId)
        return { url, title, ...r }
      }

      case 'click': {
        const clicked = await this.browser.click(session.tabId, step.params.selector)
        await new Promise(r => setTimeout(r, step.params.waitMs || 1000))
        return { clicked }
      }

      case 'type': {
        const typed = await this.browser.type(session.tabId, step.params.selector, step.params.text)
        return { typed }
      }

      case 'extract': {
        if (step.params.expression) {
          return await this.browser.evaluate(session.tabId, step.params.expression)
        }
        if (step.params.selector) {
          return await this.browser.extractText(session.tabId, step.params.selector)
        }
        return await this.browser.extractText(session.tabId)
      }

      case 'wait': {
        const ms = step.params.ms || 2000
        await new Promise(r => setTimeout(r, ms))
        return { waited: ms }
      }

      case 'evaluate': {
        return await this.browser.evaluate(session.tabId, step.params.expression)
      }

      case 'fill': {
        const results: Record<string, boolean> = {}
        for (const [selector, value] of Object.entries(step.params.fields || {})) {
          results[selector] = await this.browser.type(session.tabId, selector, String(value))
        }
        if (step.params.submitSelector) {
          await this.browser.click(session.tabId, step.params.submitSelector)
          await new Promise(r => setTimeout(r, 1500))
        }
        return results
      }

      case 'screenshot': {
        const base64 = await this.browser.screenshot(session.tabId, { fullPage: step.params.fullPage })
        return { captured: true, size: base64.length }
      }

      default:
        throw new Error(`ไม่รู้จัก action: ${step.action}`)
    }
  }

  /** สร้าง prompt สำหรับ LLM */
  private buildPlanningPrompt(goal: string, context: string): string {
    return `คุณคือ AI ที่วางแผนการใช้เว็บเบราว์เซอร์อัตโนมัติ
จงวางแผนเป็น steps เพื่อทำภารกิจต่อไปนี้

ภารกิจ: ${goal}

บริบทหน้าเว็บปัจจุบัน:
${context}

ตอบเป็น JSON array เท่านั้น (ไม่ต้องมี markdown):
[
  {
    "action": "navigate|click|type|extract|wait|evaluate|fill|screenshot",
    "description": "อธิบายเป็นภาษาไทยสั้นๆ",
    "params": { ... }
  }
]

action ที่ใช้ได้:
- navigate: { "url": "https://..." }
- click: { "selector": "CSS selector" }
- type: { "selector": "CSS selector", "text": "ข้อความ" }
- fill: { "fields": { "selector": "value" }, "submitSelector": "optional" }
- extract: { "selector": "optional CSS", "expression": "optional JS" }
- wait: { "ms": 2000 }
- evaluate: { "expression": "JS code" }
- screenshot: { "fullPage": false }

ตัวอย่าง selector: "input[name='q']", "#search-btn", ".product-item a", "button[type='submit']"

วางแผนให้กระชับที่สุด (3-8 steps) และใช้ selector ที่เป็นไปได้จริง`
  }

  /** เรียก LLM เพื่อวางแผน */
  private async callLLMForPlan(prompt: string): Promise<PlanStep[]> {
    const apiKey = process.env.LLM_API_KEY || process.env.GLM_API_KEY || ''
    
    const body = {
      model: this.llmModel,
      messages: [
        { role: 'system', content: 'คุณคือ AI วางแผนการใช้เบราว์เซอร์ ตอบเป็น JSON array เท่านั้น' },
        { role: 'user', content: prompt }
      ],
      temperature: 0.3,
      max_tokens: 2000,
    }

    const resp = await fetch(this.llmEndpoint, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(apiKey ? { 'Authorization': `Bearer ${apiKey}` } : {}),
      },
      body: JSON.stringify(body),
    })

    if (!resp.ok) {
      const text = await resp.text()
      throw new Error(`LLM error ${resp.status}: ${text.substring(0, 200)}`)
    }

    const data = await resp.json()
    const content = data.choices?.[0]?.message?.content || '[]'

    // Parse JSON จาก response (อาจมี markdown wrapper)
    let jsonStr = content
    const mdMatch = content.match(/```(?:json)?\s*([\s\S]*?)```/)
    if (mdMatch) jsonStr = mdMatch[1]
    
    let steps: any[]
    try {
      steps = JSON.parse(jsonStr.trim())
    } catch {
      throw new Error(`ไม่สามารถแปลงผล LLM เป็น JSON: ${content.substring(0, 200)}`)
    }

    if (!Array.isArray(steps)) steps = [steps]

    return steps.map((s, i) => ({
      id: i + 1,
      action: s.action || 'wait',
      description: s.description || `Step ${i + 1}`,
      params: s.params || {},
      status: 'pending' as const,
      retries: 0,
    }))
  }
}
