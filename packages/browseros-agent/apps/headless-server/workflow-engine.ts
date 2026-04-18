// Workflow Engine — ระบบ Workflow Builder
// สร้าง workflow อัตโนมัติ: trigger → steps → output
// รองรับ: manual, scheduled, webhook trigger

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { ContextEngine } from './context-engine'
import { trackCost } from './cost-tracker'

export type TriggerType = 'manual' | 'scheduled' | 'webhook'
export type StepType = 'navigate' | 'click' | 'type' | 'extract' | 'wait' | 'condition' | 'loop' | 'notify'
export type WorkflowStatus = 'idle' | 'running' | 'paused' | 'completed' | 'failed' | 'stopped'

export interface WorkflowStep {
  id: number
  type: StepType
  label: string
  params: Record<string, any>
  // condition
  condition?: {
    check: string   // 'contains' | 'equals' | 'exists'
    selector?: string
    value?: string
    thenStep?: number  // ไป step ไหนถ้าเงื่อนไขเป็นจริง
    elseStep?: number  // ไป step ไหนถ้าเงื่อนไขเป็นเท็จ
  }
  // loop
  loop?: {
    items: string[] | { expression: string }  // รายการหรือ JS expression
    stepId: number  // step ที่จะวนซ้ำ
  }
}

export interface WorkflowTrigger {
  type: TriggerType
  cron?: string       // สำหรับ scheduled
  webhookPath?: string // สำหรับ webhook
}

export interface WorkflowRun {
  id: string
  workflowId: string
  status: WorkflowStatus
  currentStep: number
  variables: Record<string, any>
  stepResults: Record<number, any>
  startedAt: number
  completedAt?: number
  error?: string
}

export interface Workflow {
  id: string
  name: string
  description: string
  trigger: WorkflowTrigger
  steps: WorkflowStep[]
  variables: Record<string, any>  // default variables
  status: WorkflowStatus
  runs: WorkflowRun[]
  createdAt: number
  updatedAt: number
}

function wfId(): string {
  return `wf_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function runId(): string {
  return `run_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

/**
 * Workflow Engine
 * สร้าง, รัน, หยุด workflow อัตโนมัติ
 */
export class WorkflowEngine {
  private browser: HeadlessBrowser
  private sessions: SessionManager
  private contextEngine: ContextEngine
  private workflows = new Map<string, Workflow>()
  private activeRun: WorkflowRun | null = null
  private scheduledTimers = new Map<string, any>()
  private storagePath: string

  constructor(
    browser: HeadlessBrowser,
    sessions: SessionManager,
    contextEngine: ContextEngine,
    storagePath?: string,
  ) {
    this.browser = browser
    this.sessions = sessions
    this.contextEngine = contextEngine
    this.storagePath = storagePath || '/tmp/browseros-workflows.json'
    this.loadFromStorage()
  }

  /** สร้าง workflow ใหม่ */
  createWorkflow(data: {
    name: string
    description: string
    trigger: WorkflowTrigger
    steps: WorkflowStep[]
    variables?: Record<string, any>
  }): Workflow {
    const wf: Workflow = {
      id: wfId(),
      name: data.name,
      description: data.description,
      trigger: data.trigger,
      steps: data.steps,
      variables: data.variables || {},
      status: 'idle',
      runs: [],
      createdAt: Date.now(),
      updatedAt: Date.now(),
    }

    this.workflows.set(wf.id, wf)
    this.saveToStorage()

    // ตั้ง schedule ถ้าเป็น scheduled trigger
    if (wf.trigger.type === 'scheduled' && wf.trigger.cron) {
      this.setupSchedule(wf)
    }

    return wf
  }

  /** ดึง workflow ทั้งหมด */
  getAllWorkflows(): Workflow[] {
    return Array.from(this.workflows.values())
  }

  /** ดึง workflow ตาม ID */
  getWorkflow(id: string): Workflow | undefined {
    return this.workflows.get(id)
  }

  /** ลบ workflow */
  deleteWorkflow(id: string): boolean {
    const wf = this.workflows.get(id)
    if (!wf) return false

    // ลบ schedule
    const timer = this.scheduledTimers.get(id)
    if (timer) {
      clearInterval(timer)
      this.scheduledTimers.delete(id)
    }

    this.workflows.delete(id)
    this.saveToStorage()
    return true
  }

  /** รัน workflow */
  @trackCost('workflow', 'glm-4-flash', 1)
  async runWorkflow(id: string, inputVars?: Record<string, any>): Promise<WorkflowRun> {
    const wf = this.workflows.get(id)
    if (!wf) throw new Error(`ไม่พบ workflow ${id}`)

    const run: WorkflowRun = {
      id: runId(),
      workflowId: id,
      status: 'running',
      currentStep: 0,
      variables: { ...wf.variables, ...inputVars },
      stepResults: {},
      startedAt: Date.now(),
    }

    wf.runs.push(run)
    wf.status = 'running'
    this.activeRun = run

    try {
      await this.executeRun(wf, run)
    } catch (err: any) {
      run.status = 'failed'
      run.error = err.message
      run.completedAt = Date.now()
      wf.status = 'failed'
    }

    wf.updatedAt = Date.now()
    this.saveToStorage()
    this.activeRun = null
    return run
  }

  /** หยุด workflow ที่กำลังรัน */
  stopWorkflow(id: string): boolean {
    const wf = this.workflows.get(id)
    if (!wf || wf.status !== 'running') return false

    const run = wf.runs[wf.runs.length - 1]
    if (run) {
      run.status = 'stopped'
      run.completedAt = Date.now()
    }

    wf.status = 'stopped'
    this.activeRun = null
    this.saveToStorage()
    return true
  }

  // ── Private ──

  private async executeRun(wf: Workflow, run: WorkflowRun): Promise<void> {
    // สร้าง session ใหม่สำหรับรัน
    const session = await this.sessions.create()
    let stepIndex = 0

    try {
      while (stepIndex < wf.steps.length && run.status === 'running') {
        const step = wf.steps[stepIndex]
        run.currentStep = stepIndex

        try {
          const result = await this.executeStep(session.id, step, run.variables)
          run.stepResults[stepIndex] = result

          // เก็บผลลัพธ์ลง variables
          if (step.type === 'extract' && result) {
            run.variables[`step_${step.id}_result`] = result
          }

          // จัดการ condition
          if (step.type === 'condition' && step.condition) {
            const conditionMet = this.evaluateCondition(step.condition, run.variables, session.id)
            stepIndex = conditionMet
              ? (step.condition.thenStep ?? stepIndex + 1)
              : (step.condition.elseStep ?? stepIndex + 1)
            continue
          }

          // จัดการ loop
          if (step.type === 'loop' && step.loop) {
            let items = step.loop.items
            if (!Array.isArray(items) && (items as any).expression) {
              const expr = (items as any).expression
              try {
                const session2 = this.sessions.get(session.id)
                if (session2) {
                  items = await this.browser.evaluate(session2.tabId, expr)
                }
              } catch {
                items = []
              }
            }
            for (const item of (items as any[])) {
              if (run.status !== 'running') break
              run.variables['loop_item'] = item
              const loopStep = wf.steps.find(s => s.id === step.loop!.stepId)
              if (loopStep) {
                await this.executeStep(session.id, loopStep, run.variables)
              }
            }
          }

          stepIndex++
        } catch (err: any) {
          run.stepResults[stepIndex] = { error: err.message }
          // ข้าม step ที่ error ไปต่อ
          stepIndex++
        }
      }

      run.status = run.status === 'running' ? 'completed' : run.status
      run.completedAt = Date.now()
      wf.status = run.status === 'completed' ? 'idle' : run.status
    } finally {
      await this.sessions.destroy(session.id)
    }
  }

  private async executeStep(sessionId: string, step: WorkflowStep, vars: Record<string, any>): Promise<any> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error('session หาย')

    // แทนที่ {{variable}} ใน params
    const resolvedParams = this.resolveVariables(step.params, vars)

    switch (step.type) {
      case 'navigate': {
        await this.browser.navigate(session.tabId, resolvedParams.url)
        await this.browser.waitForLoad(session.tabId)
        const url = await this.browser.getCurrentUrl(session.tabId)
        const title = await this.browser.getTitle(session.tabId)
        return { url, title }
      }
      case 'click': {
        const clicked = await this.browser.click(session.tabId, resolvedParams.selector)
        await new Promise(r => setTimeout(r, resolvedParams.waitMs || 500))
        return { clicked }
      }
      case 'type': {
        const typed = await this.browser.type(session.tabId, resolvedParams.selector, resolvedParams.text)
        return { typed }
      }
      case 'extract': {
        if (resolvedParams.expression) {
          return await this.browser.evaluate(session.tabId, resolvedParams.expression)
        }
        if (resolvedParams.selector) {
          return await this.browser.extractText(session.tabId, resolvedParams.selector)
        }
        return await this.browser.extractText(session.tabId)
      }
      case 'wait': {
        const ms = resolvedParams.ms || 2000
        await new Promise(r => setTimeout(r, ms))
        return { waited: ms }
      }
      case 'condition': {
        return { evaluated: true }
      }
      case 'loop': {
        return { looped: true }
      }
      case 'notify': {
        // notify แบบง่าย — log ไว้
        console.log(`[Workflow Notify] ${resolvedParams.message || 'workflow step completed'}`)
        return { notified: true, message: resolvedParams.message }
      }
      default:
        throw new Error(`ไม่รู้จัก step type: ${step.type}`)
    }
  }

  private evaluateCondition(
    condition: NonNullable<WorkflowStep['condition']>,
    vars: Record<string, any>,
    sessionId: string,
  ): boolean {
    const value = vars[`step_${condition.value}_result`] || condition.value
    switch (condition.check) {
      case 'contains':
        return String(value).includes(condition.value || '')
      case 'equals':
        return String(value) === (condition.value || '')
      case 'exists':
        return !!value
      default:
        return false
    }
  }

  private resolveVariables(params: Record<string, any>, vars: Record<string, any>): Record<string, any> {
    const resolved: Record<string, any> = {}
    for (const [key, val] of Object.entries(params)) {
      if (typeof val === 'string') {
        resolved[key] = val.replace(/\{\{(\w+)\}\}/g, (_, v) => String(vars[v] ?? ''))
      } else {
        resolved[key] = val
      }
    }
    return resolved
  }

  private setupSchedule(wf: Workflow): void {
    // scheduled แบบง่าย — รันทุก N วินาที (parse cron แบบ basic)
    // รองรับ: "*/30 * * * *" → ทุก 30 นาที
    const cron = wf.trigger.cron || ''
    const match = cron.match(/\*\/(\d+)/)
    if (match) {
      const minutes = parseInt(match[1])
      const timer = setInterval(async () => {
        try {
          await this.runWorkflow(wf.id)
        } catch (err) {
          console.error(`[Workflow Schedule] ${wf.name} error:`, err)
        }
      }, minutes * 60 * 1000)
      this.scheduledTimers.set(wf.id, timer)
    }
  }

  private saveToStorage(): void {
    try {
      const data = Array.from(this.workflows.values()).map(wf => ({
        ...wf,
        runs: wf.runs.slice(-20), // เก็บ 20 ครั้งล่าสุด
      }))
      Bun.write(this.storagePath, JSON.stringify(data, null, 2))
    } catch {
      // ignore
    }
  }

  private loadFromStorage(): void {
    try {
      const file = Bun.file(this.storagePath)
      const text = file.text ? null : null // Bun.file is lazy
      // ใช้ sync read แบบง่าย
      const fs = require('fs')
      if (fs.existsSync(this.storagePath)) {
        const raw = fs.readFileSync(this.storagePath, 'utf-8')
        const data = JSON.parse(raw)
        for (const wf of data) {
          // restore Map-like structure
          wf.runs = wf.runs || []
          this.workflows.set(wf.id, wf)
        }
      }
    } catch {
      // ignore
    }
  }
}
