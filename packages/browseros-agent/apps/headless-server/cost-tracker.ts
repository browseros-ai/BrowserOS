// Cost Tracker — ติดตามค่าใช้จ่าย API
// บันทึกทุก API call, คำนวณต้นทุน, สรุปรายวัน/สัปดาห์/เดือน
// ตั้งงบ + แจ้งเตือนเกิน

import { existsSync, writeFileSync, readFileSync } from 'fs'

export interface CostEntry {
  id: string
  timestamp: number
  provider: string
  model: string
  inputTokens: number
  outputTokens: number
  costUsd: number
  metadata?: Record<string, any>
}

export interface BudgetConfig {
  dailyLimit: number    // USD
  monthlyLimit: number  // USD
  alertThreshold: number // 0-1, แจ้งเมื่อถึง % ของงบ
}

export interface CostSummary {
  today: number
  week: number
  month: number
  total: number
  entries: number
  byProvider: Record<string, number>
  byModel: Record<string, number>
}

const STORAGE_PATH = '/tmp/browseros-costs.json'
const DEFAULT_BUDGET: BudgetConfig = {
  dailyLimit: 5,
  monthlyLimit: 100,
  alertThreshold: 0.8,
}

// Pricing per 1M tokens (USD)
const PRICING: Record<string, { input: number; output: number }> = {
  'glm': { input: 0.1, output: 0.3 },
  'glm-4': { input: 0.1, output: 0.3 },
  'glm-5': { input: 0.1, output: 0.3 },
  'gpt-4o': { input: 2.5, output: 10 },
  'gpt-4o-mini': { input: 0.15, output: 0.6 },
  'claude-sonnet': { input: 3, output: 15 },
  'claude-haiku': { input: 0.25, output: 1.25 },
  'gemini-pro': { input: 0.5, output: 1.5 },
}

function entryId(): string {
  return `cost_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function getDayKey(ts: number): string {
  return new Date(ts).toISOString().slice(0, 10)
}

/**
 * Cost Tracker
 * ติดตามค่า API ทุก call
 */
export class CostTracker {
  private entries: CostEntry[] = []
  private budget: BudgetConfig
  private maxEntries = 10000

  constructor(budget?: Partial<BudgetConfig>) {
    this.budget = { ...DEFAULT_BUDGET, ...budget }
    this.loadFromStorage()
  }

  /** บันทึกการใช้ API */
  record(entry: Omit<CostEntry, 'id' | 'costUsd'>): CostEntry {
    const costUsd = this.calculateCost(entry.provider, entry.model, entry.inputTokens, entry.outputTokens)
    const full: CostEntry = {
      ...entry,
      id: entryId(),
      costUsd,
    }
    this.entries.push(full)

    // ตัด entries เก่า
    if (this.entries.length > this.maxEntries) {
      this.entries = this.entries.slice(-this.maxEntries)
    }

    this.saveToStorage()

    // เช็คงบ
    this.checkBudget()

    return full
  }

  /** คำนวณต้นทุน */
  private calculateCost(provider: string, model: string, inputTokens: number, outputTokens: number): number {
    const key = model.toLowerCase()
    const pricing = PRICING[key] || PRICING[provider.toLowerCase()] || { input: 0.1, output: 0.3 }
    const inputCost = (inputTokens / 1_000_000) * pricing.input
    const outputCost = (outputTokens / 1_000_000) * pricing.output
    return inputCost + outputCost
  }

  /** สรุปค่าใช้จ่าย */
  getSummary(): CostSummary {
    const now = Date.now()
    const todayStart = new Date(new Date().toISOString().slice(0, 10)).getTime()
    const weekStart = now - 7 * 24 * 60 * 60 * 1000
    const monthStart = new Date(new Date().getFullYear(), new Date().getMonth(), 1).getTime()

    let today = 0, week = 0, month = 0, total = 0
    const byProvider: Record<string, number> = {}
    const byModel: Record<string, number> = {}

    for (const e of this.entries) {
      total += e.costUsd
      if (e.timestamp >= todayStart) today += e.costUsd
      if (e.timestamp >= weekStart) week += e.costUsd
      if (e.timestamp >= monthStart) month += e.costUsd
      byProvider[e.provider] = (byProvider[e.provider] || 0) + e.costUsd
      byModel[e.model] = (byModel[e.model] || 0) + e.costUsd
    }

    return { today, week, month, total, entries: this.entries.length, byProvider, byModel }
  }

  /** ดึงประวัติ (ล่าสุด) */
  getHistory(limit = 100, since?: number): CostEntry[] {
    let filtered = since ? this.entries.filter(e => e.timestamp >= since) : this.entries
    return filtered.slice(-limit)
  }

  /** ตั้งงบ */
  setBudget(config: Partial<BudgetConfig>): BudgetConfig {
    this.budget = { ...this.budget, ...config }
    this.saveToStorage()
    return this.budget
  }

  /** ดึงงบปัจจุบัน */
  getBudget(): BudgetConfig {
    return { ...this.budget }
  }

  /** เช็คเกินงบ */
  private checkBudget(): void {
    const summary = this.getSummary()
    if (summary.today >= this.budget.dailyLimit * this.budget.alertThreshold) {
      console.warn(`[CostTracker] ⚠️ ค่าใช้จ่ายวันนี้ $${summary.today.toFixed(4)} ใกล้ถึงงบ $${this.budget.dailyLimit}`)
    }
    if (summary.month >= this.budget.monthlyLimit * this.budget.alertThreshold) {
      console.warn(`[CostTracker] ⚠️ ค่าใช้จ่ายเดือนนี้ $${summary.month.toFixed(4)} ใกล้ถึงงบ $${this.budget.monthlyLimit}`)
    }
  }

  private saveToStorage(): void {
    try {
      writeFileSync(STORAGE_PATH, JSON.stringify({
        entries: this.entries.slice(-5000),
        budget: this.budget,
      }, null, 2))
    } catch { /* ignore */ }
  }

  private loadFromStorage(): void {
    try {
      if (existsSync(STORAGE_PATH)) {
        const raw = readFileSync(STORAGE_PATH, 'utf-8')
        const data = JSON.parse(raw)
        this.entries = data.entries || []
        if (data.budget) this.budget = { ...this.budget, ...data.budget }
      }
    } catch { /* ignore */ }
  }
}

/** Helper: track cost decorator-like function */
export function trackCost(tracker: CostTracker, provider: string, model: string, inputTokens: number, outputTokens: number): CostEntry {
  return tracker.record({ timestamp: Date.now(), provider, model, inputTokens, outputTokens })
}
