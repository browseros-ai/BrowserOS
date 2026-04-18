// Multi-Agent System — ระบบหลาย Agent ทำงานร่วมกัน
// รับคำสั่ง complex → แบ่งเป็น sub-tasks → ส่งให้ agent แต่ละตัวทำงาน parallel

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { ContextEngine } from './context-engine'
import { SmartPlanner } from './smart-planner'
import { CostTracker, trackCost } from './cost-tracker'

export type AgentType = 'browser-agent' | 'extract-agent' | 'analysis-agent' | 'report-agent'
export type TaskStatus = 'pending' | 'running' | 'completed' | 'failed' | 'cancelled'

export interface AgentTask {
  id: string
  type: AgentType
  description: string
  params: Record<string, any>
  status: TaskStatus
  result?: any
  error?: string
  createdAt: number
  completedAt?: number
}

export interface AgentJob {
  id: string
  goal: string
  tasks: AgentTask[]
  status: TaskStatus
  sharedMemory: Map<string, any>
  createdAt: number
  completedAt?: number
}

export interface AgentInfo {
  type: AgentType
  description: string
  status: 'idle' | 'busy'
  currentTask?: string
}

// รายละเอียด agent แต่ละตัว
const AGENT_DEFINITIONS: Record<AgentType, string> = {
  'browser-agent': 'ควบคุม browser — เปิดเว็บ, คลิก, พิมพ์, กรอกฟอร์ม',
  'extract-agent': 'ดึงข้อมูลจากหน้าเว็บ — ข้อความ, HTML, JSON',
  'analysis-agent': 'วิเคราะห์ข้อมูล — สรุป, เปรียบเทียบ, หา pattern',
  'report-agent': 'สร้างรายงาน — จัดรูปแบบ, สรุปผล, export',
}

function taskId(): string {
  return `task_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function jobId(): string {
  return `job_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

/**
 * Multi-Agent Orchestrator
 * รับคำสั่ง complex แบ่งเป็น sub-tasks ให้ agent แต่ละตัวทำงาน parallel
 */
export class MultiAgentOrchestrator {
  private browser: HeadlessBrowser
  private sessions: SessionManager
  private contextEngine: ContextEngine
  private smartPlanner: SmartPlanner
  private activeJobs = new Map<string, AgentJob>()
  private agentStatus = new Map<AgentType, { status: 'idle' | 'busy'; currentTask?: string }>()

  constructor(
    browser: HeadlessBrowser,
    sessions: SessionManager,
    contextEngine: ContextEngine,
    smartPlanner: SmartPlanner,
  ) {
    this.browser = browser
    this.sessions = sessions
    this.contextEngine = contextEngine
    this.smartPlanner = smartPlanner

    // เริ่มต้น agent ทุกตัวเป็น idle
    for (const type of Object.keys(AGENT_DEFINITIONS) as AgentType[]) {
      this.agentStatus.set(type, { status: 'idle' })
    }
  }

  /** ส่ง task complex ให้ระบบ multi-agent */
  @trackCost('multi-agent', 'glm-4-flash', 1)
  async submitTask(goal: string, sessionId?: string): Promise<AgentJob> {
    const job: AgentJob = {
      id: jobId(),
      goal,
      tasks: [],
      sharedMemory: new Map(),
      status: 'pending',
      createdAt: Date.now(),
    }

    // แบ่ง task อัตโนมัติจาก goal
    const subTasks = await this.decomposeTask(goal, sessionId)
    job.tasks = subTasks
    job.status = 'running'

    this.activeJobs.set(job.id, job)

    // รัน tasks (parallel ที่ทำได้)
    await this.executeJob(job, sessionId)

    return job
  }

  /** ดึงสถานะ agents ทั้งหมด */
  getAgentsStatus(): AgentInfo[] {
    return (Object.keys(AGENT_DEFINITIONS) as AgentType[]).map(type => {
      const info = this.agentStatus.get(type)!
      return {
        type,
        description: AGENT_DEFINITIONS[type],
        status: info.status,
        currentTask: info.currentTask,
      }
    })
  }

  /** ดึงสถานะ job */
  getJobStatus(jobId: string): AgentJob | undefined {
    return this.activeJobs.get(jobId)
  }

  /** ยกเลิก job */
  cancelJob(jobId: string): boolean {
    const job = this.activeJobs.get(jobId)
    if (!job || job.status !== 'running') return false
    job.status = 'cancelled'
    // reset agent status
    for (const [, info] of this.agentStatus) {
      if (info.currentTask?.startsWith(jobId)) {
        info.status = 'idle'
        info.currentTask = undefined
      }
    }
    return true
  }

  /** ดึง shared memory */
  getSharedMemory(jobId: string): Record<string, any> {
    const job = this.activeJobs.get(jobId)
    if (!job) return {}
    return Object.fromEntries(job.sharedMemory)
  }

  /** ตั้งค่า shared memory */
  setSharedMemory(jobId: string, key: string, value: any): boolean {
    const job = this.activeJobs.get(jobId)
    if (!job) return false
    job.sharedMemory.set(key, value)
    return true
  }

  // ── Private ──

  /** แบ่ง task complex เป็น sub-tasks */
  private async decomposeTask(goal: string, sessionId?: string): Promise<AgentTask[]> {
    // แบ่งตาม keyword ง่ายๆ
    const tasks: AgentTask[] = []
    const lowerGoal = goal.toLowerCase()

    if (lowerGoal.includes('เปิด') || lowerGoal.includes('ไป') || lowerGoal.includes('เข้า') || lowerGoal.includes('navigate') || lowerGoal.includes('open')) {
      // ดึง URL จาก goal
      const urlMatch = goal.match(/https?:\/\/[^\s]+/)
      if (urlMatch) {
        tasks.push({
          id: taskId(), type: 'browser-agent',
          description: `เปิดเว็บ ${urlMatch[0]}`,
          params: { action: 'navigate', url: urlMatch[0] },
          status: 'pending', createdAt: Date.now(),
        })
      }
    }

    if (lowerGoal.includes('ดึง') || lowerGoal.includes('อ่าน') || lowerGoal.includes('extract') || lowerGoal.includes('ดู')) {
      tasks.push({
        id: taskId(), type: 'extract-agent',
        description: 'ดึงข้อมูลจากหน้าเว็บ',
        params: { action: 'extract', type: 'text' },
        status: 'pending', createdAt: Date.now(),
      })
    }

    if (lowerGoal.includes('วิเคราะห์') || lowerGoal.includes('เปรียบเทียบ') || lowerGoal.includes('analyze') || lowerGoal.includes('หา')) {
      tasks.push({
        id: taskId(), type: 'analysis-agent',
        description: 'วิเคราะห์ข้อมูลที่ได้มา',
        params: { action: 'analyze', goal },
        status: 'pending', createdAt: Date.now(),
      })
    }

    if (lowerGoal.includes('รายงาน') || lowerGoal.includes('สรุป') || lowerGoal.includes('report') || lowerGoal.includes('ส่ง')) {
      tasks.push({
        id: taskId(), type: 'report-agent',
        description: 'สร้างรายงานสรุปผล',
        params: { action: 'report', goal },
        status: 'pending', createdAt: Date.now(),
      })
    }

    // ถ้าไม่มี task เลย สร้าง browser + extract
    if (tasks.length === 0) {
      tasks.push({
        id: taskId(), type: 'browser-agent',
        description: 'ดำเนินการตามคำสั่ง',
        params: { action: 'auto', goal },
        status: 'pending', createdAt: Date.now(),
      })
      tasks.push({
        id: taskId(), type: 'extract-agent',
        description: 'ดึงข้อมูลผลลัพธ์',
        params: { action: 'extract', type: 'text' },
        status: 'pending', createdAt: Date.now(),
      })
    }

    return tasks
  }

  /** execute job ทั้งหมด */
  private async executeJob(job: AgentJob, sessionId?: string): Promise<void> {
    // แยก tasks ตาม type เพื่อรัน parallel ในแต่ละกลุ่ม
    // แต่ต้องรันตามลำดับ: browser → extract → analysis → report
    const order: AgentType[] = ['browser-agent', 'extract-agent', 'analysis-agent', 'report-agent']

    for (const agentType of order) {
      if (job.status === 'cancelled') break

      const tasksOfType = job.tasks.filter(t => t.type === agentType && t.status === 'pending')
      if (tasksOfType.length === 0) continue

      // รัน tasks ของแต่ละ type แบบ parallel
      await Promise.all(tasksOfType.map(task => this.executeTask(job, task, sessionId)))
    }

    // อัปเดต job status
    const allDone = job.tasks.every(t => t.status === 'completed')
    const anyFailed = job.tasks.some(t => t.status === 'failed')
    job.status = anyFailed ? 'failed' : allDone ? 'completed' : job.status
    job.completedAt = Date.now()
  }

  /** execute หนึ่ง task */
  private async executeTask(job: AgentJob, task: AgentTask, sessionId?: string): Promise<void> {
    const agentInfo = this.agentStatus.get(task.type)!
    agentInfo.status = 'busy'
    agentInfo.currentTask = `${job.id}/${task.id}`
    task.status = 'running'

    try {
      const sid = sessionId || this.sessions.getAll()[0]?.id
      if (!sid) throw new Error('ไม่มี session ให้ใช้งาน')

      switch (task.type) {
        case 'browser-agent':
          task.result = await this.runBrowserAgent(task, sid)
          break
        case 'extract-agent':
          task.result = await this.runExtractAgent(task, sid)
          break
        case 'analysis-agent':
          task.result = await this.runAnalysisAgent(task, job)
          break
        case 'report-agent':
          task.result = await this.runReportAgent(task, job)
          break
      }

      task.status = 'completed'
      task.completedAt = Date.now()

      // เก็บผลลัพธ์ใน shared memory
      job.sharedMemory.set(`result_${task.type}`, task.result)
    } catch (err: any) {
      task.status = 'failed'
      task.error = err.message
      task.completedAt = Date.now()
    } finally {
      agentInfo.status = 'idle'
      agentInfo.currentTask = undefined
    }
  }

  private async runBrowserAgent(task: AgentTask, sessionId: string): Promise<any> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error('ไม่พบ session')

    const { params } = task
    if (params.action === 'navigate' && params.url) {
      await this.browser.navigate(session.tabId, params.url)
      await this.browser.waitForLoad(session.tabId)
      const url = await this.browser.getCurrentUrl(session.tabId)
      const title = await this.browser.getTitle(session.tabId)
      return { url, title }
    }

    // auto mode — ใช้ smart planner
    return { message: 'ใช้ smart planner แทน', goal: params.goal }
  }

  private async runExtractAgent(task: AgentTask, sessionId: string): Promise<any> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error('ไม่พบ session')

    const { params } = task
    if (params.selector) {
      return await this.browser.extractText(session.tabId, params.selector)
    }

    // ดึง context ทั้งหมด
    try {
      await this.contextEngine.extractContext(sessionId)
      const ctx = this.contextEngine.getCurrentContext(sessionId)
      return ctx
    } catch {
      return await this.browser.extractText(session.tabId)
    }
  }

  private async runAnalysisAgent(task: AgentTask, job: AgentJob): Promise<any> {
    // ดึงข้อมูลจาก shared memory
    const extractResult = job.sharedMemory.get('result_extract-agent')
    const analysis = {
      goal: task.params.goal,
      dataAvailable: !!extractResult,
      summary: extractResult?.textSummary?.substring(0, 500) || 'ไม่มีข้อมูลให้วิเคราะห์',
      analyzedAt: Date.now(),
    }
    return analysis
  }

  private async runReportAgent(task: AgentTask, job: AgentJob): Promise<any> {
    const allResults = Object.fromEntries(job.sharedMemory)
    const report = {
      goal: job.goal,
      summary: `รายงานผลการทำงาน: ${job.goal}`,
      tasks: job.tasks.map(t => ({
        type: t.type,
        description: t.description,
        status: t.status,
        hasResult: !!t.result,
      })),
      sharedMemory: allResults,
      createdAt: Date.now(),
    }
    return report
  }
}
