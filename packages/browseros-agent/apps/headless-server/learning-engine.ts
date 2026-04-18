// Learning Engine — ระบบเรียนรู้จากการใช้งาน
// เก็บประวัติ interaction, สร้าง rules อัตโนมัติ, apply เมื่อเจอบริบทเดียวกัน

export interface LearningRule {
  id: string
  context: string       // เงื่อนไข เช่น "หน้า login", "ค้นหา"
  pattern: string       // pattern ที่ตรวจจับ เช่น "login", "search"
  correction: string    // สิ่งที่ควรทำ เช่น "ใช้ email ไม่ใช่ username"
  originalAction: string // action เดิมที่ผิด
  correctedAction: string // action ที่ถูกต้อง
  confidence: number    // 0-1
  occurrences: number   // จำนวนครั้งที่เกิด
  createdAt: number
  lastApplied?: number
  autoApply: boolean    // apply อัตโนมัติ?
}

export interface InteractionRecord {
  id: string
  sessionId: string
  url: string
  action: string
  params: Record<string, any>
  result: string
  success: boolean
  isCorrection: boolean  // user แก้ไข?
  timestamp: number
}

const STORAGE_PATH = '/tmp/browseros-learning.json'

function ruleId(): string {
  return `rule_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function recordId(): string {
  return `rec_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

/**
 * Learning Engine
 * เรียนรู้จากการใช้งาน สร้าง rules อัตโนมัติ
 */
export class LearningEngine {
  private rules: LearningRule[] = []
  private history: InteractionRecord[] = []
  private maxHistory = 1000
  private maxRules = 100

  constructor() {
    this.loadFromStorage()
  }

  /** บันทึก interaction */
  recordInteraction(data: {
    sessionId: string
    url: string
    action: string
    params: Record<string, any>
    result: string
    success: boolean
    isCorrection?: boolean
  }): InteractionRecord {
    const record: InteractionRecord = {
      id: recordId(),
      ...data,
      isCorrection: data.isCorrection || false,
      timestamp: Date.now(),
    }

    this.history.push(record)
    if (this.history.length > this.maxHistory) {
      this.history = this.history.slice(-this.maxHistory)
    }

    // ถ้าเป็น correction → ลองสร้าง rule
    if (record.isCorrection) {
      this.tryCreateRule(record)
    }

    this.saveToStorage()
    return record
  }

  /** ดึง rules ทั้งหมด */
  getRules(): LearningRule[] {
    return this.rules
  }

  /** ลบ rule */
  deleteRule(id: string): boolean {
    const idx = this.rules.findIndex(r => r.id === id)
    if (idx === -1) return false
    this.rules.splice(idx, 1)
    this.saveToStorage()
    return true
  }

  /** ดึงประวัติ */
  getHistory(limit = 100): InteractionRecord[] {
    return this.history.slice(-limit)
  }

  /** ตรวจสอบว่ามี rule สำหรับ context นี้หรือไม่ */
  getApplicableRules(url: string, action: string): LearningRule[] {
    return this.rules.filter(rule => {
      if (!rule.autoApply) return false
      // ตรวจสอบ context
      const urlMatch = url.toLowerCase().includes(rule.pattern.toLowerCase())
      const actionMatch = action.toLowerCase().includes(rule.originalAction.toLowerCase())
      return urlMatch || actionMatch
    })
  }

  /** Apply rules ได้ไหม — คืน correction ถ้ามี */
  checkForCorrections(url: string, action: string, params: Record<string, any>): {
    hasCorrection: boolean
    rules: LearningRule[]
    suggestion?: string
  } {
    const applicable = this.getApplicableRules(url, action)

    if (applicable.length > 0) {
      return {
        hasCorrection: true,
        rules: applicable,
        suggestion: applicable.map(r => r.correction).join('; '),
      }
    }

    return { hasCorrection: false, rules: [] }
  }

  /** เพิ่ม rule ด้วยมือ */
  addRule(data: {
    context: string
    pattern: string
    correction: string
    originalAction: string
    correctedAction: string
    autoApply?: boolean
  }): LearningRule {
    const rule: LearningRule = {
      id: ruleId(),
      context: data.context,
      pattern: data.pattern,
      correction: data.correction,
      originalAction: data.originalAction,
      correctedAction: data.correctedAction,
      confidence: 0.5,
      occurrences: 1,
      createdAt: Date.now(),
      autoApply: data.autoApply !== false,
    }

    this.rules.push(rule)
    if (this.rules.length > this.maxRules) {
      this.rules = this.rules.slice(-this.maxRules)
    }

    this.saveToStorage()
    return rule
  }

  /** เปิด/ปิด auto-apply */
  toggleAutoApply(ruleId: string): LearningRule | undefined {
    const rule = this.rules.find(r => r.id === ruleId)
    if (rule) {
      rule.autoApply = !rule.autoApply
      this.saveToStorage()
    }
    return rule
  }

  // ── Private ──

  private tryCreateRule(record: InteractionRecord): void {
    // ดู history ย้อนหลังหา action เดิมที่ถูกแก้
    const recentActions = this.history
      .filter(r => r.sessionId === record.sessionId && r.timestamp < record.timestamp)
      .slice(-5)

    const originalAction = recentActions.find(r => r.action === record.action && !r.isCorrection)
    if (!originalAction) return

    // ตรวจสอบว่ามี rule ที่ pattern เดียวกันแล้วหรือยัง
    const existingRule = this.rules.find(r =>
      r.pattern === record.url && r.originalAction === record.action
    )

    if (existingRule) {
      // เพิ่ม confidence
      existingRule.occurrences++
      existingRule.confidence = Math.min(1, existingRule.confidence + 0.1)
      return
    }

    // สร้าง rule ใหม่
    const rule: LearningRule = {
      id: ruleId(),
      context: this.inferContext(record.url),
      pattern: record.url,
      correction: `${record.action}: ${JSON.stringify(record.params)} → ${record.result}`,
      originalAction: originalAction.action,
      correctedAction: record.action,
      confidence: 0.3,
      occurrences: 1,
      createdAt: Date.now(),
      autoApply: false, // ต้องเปิดเอง
    }

    this.rules.push(rule)
  }

  private inferContext(url: string): string {
    if (url.includes('login') || url.includes('signin')) return 'หน้า login'
    if (url.includes('search') || url.includes('search')) return 'หน้าค้นหา'
    if (url.includes('checkout') || url.includes('cart')) return 'หน้าชำระเงิน'
    if (url.includes('register') || url.includes('signup')) return 'หน้าสมัคร'
    return 'ทั่วไป'
  }

  private saveToStorage(): void {
    try {
      const fs = require('fs')
      fs.writeFileSync(STORAGE_PATH, JSON.stringify({
        rules: this.rules,
        history: this.history.slice(-200),
      }, null, 2))
    } catch {
      // ignore
    }
  }

  private loadFromStorage(): void {
    try {
      const fs = require('fs')
      if (fs.existsSync(STORAGE_PATH)) {
        const raw = fs.readFileSync(STORAGE_PATH, 'utf-8')
        const data = JSON.parse(raw)
        this.rules = data.rules || []
        this.history = data.history || []
      }
    } catch {
      // ignore
    }
  }
}
