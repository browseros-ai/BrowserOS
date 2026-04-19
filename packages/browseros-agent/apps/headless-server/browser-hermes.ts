// BrowserHermes — สมองเสริมสำหรับ BrowserOS
// ระบบเรียนรู้ จดจำ พัฒนาตัวเอง เหมือน Hermes แต่ทำงานบน browser
// ประกอบด้วย: Bug Patterns DB, Strategy DB, Shadow Advisor, Iron Law Gate, Self-Evolution

import * as fs from 'fs'
import * as path from 'path'

// ── ประเภทข้อมูล ──

export interface BugPattern {
  id: string
  context: string          // บริบท เช่น "shopee.co.th", "login page"
  error: string            // error message หรือ pattern
  errorType: string        // 'selector_not_found' | 'page_load_fail' | 'timeout' | 'navigation_error' | 'other'
  solution: string         // วิธีแก้
  successCount: number     // ครั้งที่แก้สำเร็จ
  failCount: number        // ครั้งที่แก้ไม่สำเร็จ
  lastSeen: number         // timestamp ล่าสุด
  createdAt: number
}

export interface Strategy {
  id: string
  domain: string           // เว็บไซต์ เช่น "shopee.co.th"
  task: string             // งาน เช่น "ดึงชื่อสินค้า"
  selector: string         // selector ที่ใช้
  method: string           // วิธีที่ใช้
  successCount: number
  totalCount: number
  successRate: number      // successCount / totalCount
  lastUsed: number
  createdAt: number
}

export interface RiskAssessment {
  allowed: boolean
  riskLevel: 'low' | 'medium' | 'high' | 'critical'
  confidence: number       // 0-1
  warnings: string[]
  suggestions: string[]
  ironLawViolations: string[]
}

export interface EvolutionRule {
  id: string
  trigger: string          // เงื่อนไข เช่น "selector .old-btn fail 3 ครั้ง"
  action: string           // สิ่งที่ต้องทำแทน
  source: string           // 'auto' | 'manual'
  occurrences: number      // จำนวนครั้งที่ trigger
  createdAt: number
  lastApplied?: number
}

export interface ShadowLog {
  timestamp: number
  action: string
  url: string
  riskLevel: string
  confidence: number
  warnings: string[]
  outcome: 'executed' | 'blocked' | 'modified'
}

// ── Storage ──

const DATA_DIR = path.resolve(import.meta.dir, 'data')
const BUG_PATTERNS_FILE = path.join(DATA_DIR, 'hermes-bug-patterns.json')
const STRATEGIES_FILE = path.join(DATA_DIR, 'hermes-strategies.json')
const EVOLUTION_RULES_FILE = path.join(DATA_DIR, 'hermes-evolution-rules.json')
const SHADOW_LOG_FILE = path.join(DATA_DIR, 'hermes-shadow-log.json')

function uid(prefix: string): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function loadJSON<T>(filePath: string, fallback: T): T {
  try {
    if (fs.existsSync(filePath)) {
      return JSON.parse(fs.readFileSync(filePath, 'utf-8'))
    }
  } catch { /* ignore */ }
  return fallback
}

function saveJSON(filePath: string, data: any): void {
  try {
    if (!fs.existsSync(DATA_DIR)) {
      fs.mkdirSync(DATA_DIR, { recursive: true })
    }
    fs.writeFileSync(filePath, JSON.stringify(data, null, 2))
  } catch { /* ignore */ }
}

// ── Bug Patterns DB ──

export class BugPatternsDB {
  private patterns: BugPattern[] = []

  constructor() {
    this.patterns = loadJSON(BUG_PATTERNS_FILE, [])
  }

  /** เพิ่ม bug pattern ใหม่ */
  add(data: { context: string; error: string; errorType: BugPattern['errorType']; solution: string }): BugPattern {
    // เช็คว่ามี pattern เดียวกันแล้วหรือยัง
    const existing = this.patterns.find(p =>
      p.context === data.context && p.error === data.error
    )

    if (existing) {
      existing.lastSeen = Date.now()
      this.save()
      return existing
    }

    const pattern: BugPattern = {
      id: uid('bug'),
      context: data.context,
      error: data.error,
      errorType: data.errorType,
      solution: data.solution,
      successCount: 0,
      failCount: 0,
      lastSeen: Date.now(),
      createdAt: Date.now(),
    }
    this.patterns.push(pattern)
    this.save()
    return pattern
  }

  /** ค้นหาวิธีแก้สำหรับ error */
  suggest(context: string, error: string): BugPattern | undefined {
    // หา pattern ที่ตรงที่สุด
    let best: BugPattern | undefined
    let bestScore = 0

    for (const p of this.patterns) {
      let score = 0
      if (p.context && context.toLowerCase().includes(p.context.toLowerCase())) score += 2
      if (p.error && error.toLowerCase().includes(p.error.toLowerCase())) score += 3
      score += (p.successCount / Math.max(1, p.successCount + p.failCount)) * 2
      if (score > bestScore) {
        bestScore = score
        best = p
      }
    }

    return bestScore >= 2 ? best : undefined
  }

  /** บันทึกผลการใช้ solution */
  recordResult(patternId: string, success: boolean): void {
    const pattern = this.patterns.find(p => p.id === patternId)
    if (!pattern) return
    if (success) pattern.successCount++
    else pattern.failCount++
    pattern.lastSeen = Date.now()
    this.save()
  }

  /** ดึง patterns ทั้งหมด */
  getAll(): BugPattern[] {
    return this.patterns
  }

  /** นับจำนวนครั้งที่ error เกิด (ใช้สำหรับ self-evolution) */
  countOccurrences(context: string, error: string): number {
    return this.patterns.filter(p =>
      p.context === context && p.error === error
    ).reduce((sum, p) => sum + p.failCount, 0)
  }

  private save(): void {
    saveJSON(BUG_PATTERNS_FILE, this.patterns)
  }
}

// ── Strategy DB ──

export class StrategyDB {
  private strategies: Strategy[] = []

  constructor() {
    this.strategies = loadJSON(STRATEGIES_FILE, [])
  }

  /** เพิ่มหรืออัปเดต strategy */
  record(data: { domain: string; task: string; selector: string; method: string; success: boolean }): Strategy {
    const existing = this.strategies.find(s =>
      s.domain === data.domain && s.task === data.task && s.selector === data.selector
    )

    if (existing) {
      existing.totalCount++
      if (data.success) existing.successCount++
      existing.successRate = existing.successCount / existing.totalCount
      existing.lastUsed = Date.now()
      this.save()
      return existing
    }

    const strategy: Strategy = {
      id: uid('strat'),
      domain: data.domain,
      task: data.task,
      selector: data.selector,
      method: data.method,
      successCount: data.success ? 1 : 0,
      totalCount: 1,
      successRate: data.success ? 1 : 0,
      lastUsed: Date.now(),
      createdAt: Date.now(),
    }
    this.strategies.push(strategy)
    this.save()
    return strategy
  }

  /** แนะนำ strategy สำหรับเว็บและงาน */
  suggest(domain: string, task: string): Strategy | undefined {
    const matching = this.strategies
      .filter(s => s.domain === domain && (!task || s.task === task))
      .sort((a, b) => b.successRate - a.successRate || b.totalCount - a.totalCount)

    return matching[0]?.successRate >= 0.5 ? matching[0] : undefined
  }

  /** ดึง strategies ทั้งหมด */
  getAll(): Strategy[] {
    return this.strategies
  }

  private save(): void {
    saveJSON(STRATEGIES_FILE, this.strategies)
  }
}

// ── Shadow Advisor ──

export class ShadowAdvisor {
  private bugDB: BugPatternsDB
  private strategyDB: StrategyDB
  private log: ShadowLog[] = []

  constructor(bugDB: BugPatternsDB, strategyDB: StrategyDB) {
    this.bugDB = bugDB
    this.strategyDB = strategyDB
    this.log = loadJSON(SHADOW_LOG_FILE, [])
  }

  /** ประเมินความเสี่ยงก่อน execute */
  assess(action: string, url: string, params: Record<string, any> = {}): RiskAssessment {
    const warnings: string[] = []
    const suggestions: string[] = []
    const ironLawViolations: string[] = []
    let riskLevel: RiskAssessment['riskLevel'] = 'low'
    let confidence = 0.9

    // เช็ค bug patterns — selector เคย fail ไหม?
    if (params.selector) {
      const bug = this.bugDB.suggest(url, params.selector)
      if (bug && bug.failCount > bug.successCount) {
        warnings.push(`selector "${params.selector}" เคย fail ${bug.failCount} ครั้งบน ${url}`)
        suggestions.push(bug.solution || `ลองใช้ selector อื่น`)
        confidence -= 0.2
        riskLevel = 'medium'
      }
    }

    // เช็ค strategy — เว็บนี้เคยมีปัญหาไหม?
    try {
      const domain = new URL(url).hostname
      const strategy = this.strategyDB.suggest(domain, action)
      if (strategy && strategy.successRate < 0.3) {
        warnings.push(`เว็บ ${domain} มีอัตราสำเร็จต่ำ (${(strategy.successRate * 100).toFixed(0)}%)`)
        confidence -= 0.15
        if (riskLevel === 'low') riskLevel = 'medium'
      }
    } catch { /* URL ไม่ถูกต้อง */ }

    // Iron Law Gate
    // 1. ห้ามกรอกข้อมูลส่วนตัวบนเว็บที่ไม่ HTTPS
    if ((action === 'type' || action === 'fillForm') && url.startsWith('http://')) {
      const personalFields = ['password', 'email', 'phone', 'credit', 'ssn', 'id-card']
      const hasPersonal = Object.keys(params).some(k => personalFields.some(p => k.toLowerCase().includes(p)))
      if (hasPersonal) {
        ironLawViolations.push('ห้ามกรอกข้อมูลส่วนตัวบนเว็บที่ไม่ HTTPS')
        riskLevel = 'critical'
        confidence = 0
      }
    }

    // 2. ห้ามดาวน์โหลดไฟล์ .exe, .bat, .sh
    if (action === 'download' || action === 'navigate') {
      const dangerousExts = ['.exe', '.bat', '.sh', '.cmd', '.ps1', '.vbs']
      const target = params.url || params.filename || ''
      if (dangerousExts.some(ext => target.toLowerCase().endsWith(ext))) {
        ironLawViolations.push(`ห้ามดาวน์โหลดไฟล์ ${path.extname(target)}`)
        riskLevel = 'critical'
        confidence = 0
      }
    }

    // 3. ห้ามคลิกโฆษณา
    if (action === 'click' && params.selector) {
      const adPatterns = ['[class*="ad"]', '[class*="ads"]', '[id*="ad"]', '[class*="sponsor"]', '[class*="banner"]']
      if (adPatterns.some(p => params.selector?.toLowerCase().includes(p.replace(/[\[\]*"]/g, '').substring(0, 5)))) {
        warnings.push('selector นี้อาจเป็นโฆษณา')
        suggestions.push('ตรวจสอบ selector ก่อนคลิก')
        confidence -= 0.1
      }
    }

    // 4. ห้ามลบข้อมูลโดยไม่ยืนยัน
    if (action === 'click' && params.selector) {
      const deletePatterns = /delete|remove|trash|del|ลบ|clear/i
      if (deletePatterns.test(params.selector)) {
        ironLawViolations.push('การลบข้อมูลต้องการการยืนยัน')
        riskLevel = riskLevel === 'critical' ? 'critical' : 'high'
        confidence -= 0.3
        suggestions.push('กรุณายืนยันก่อนดำเนินการลบ')
      }
    }

    confidence = Math.max(0, Math.min(1, confidence))

    return {
      allowed: ironLawViolations.length === 0 && riskLevel !== 'critical',
      riskLevel,
      confidence,
      warnings,
      suggestions,
      ironLawViolations,
    }
  }

  /** บันทึก log */
  logAssessment(action: string, url: string, assessment: RiskAssessment, outcome: ShadowLog['outcome']): void {
    this.log.push({
      timestamp: Date.now(),
      action,
      url,
      riskLevel: assessment.riskLevel,
      confidence: assessment.confidence,
      warnings: assessment.warnings,
      outcome,
    })
    // เก็บแค่ 500 รายการล่าสุด
    if (this.log.length > 500) {
      this.log = this.log.slice(-500)
    }
    saveJSON(SHADOW_LOG_FILE, this.log)
  }

  /** ดึง log */
  getLog(limit = 100): ShadowLog[] {
    return this.log.slice(-limit)
  }
}

// ── Self-Evolution ──

export class SelfEvolution {
  private rules: EvolutionRule[] = []
  private bugDB: BugPatternsDB

  constructor(bugDB: BugPatternsDB) {
    this.bugDB = bugDB
    this.rules = loadJSON(EVOLUTION_RULES_FILE, [])
  }

  /** ตรวจสอบและสร้าง rules อัตโนมัติ */
  checkAndEvolve(): EvolutionRule[] {
    const newRules: EvolutionRule[] = []

    for (const pattern of this.bugDB.getAll()) {
      // ถ้า error pattern เกิด 3+ ครั้ง → สร้าง rule
      if (pattern.failCount >= 3) {
        const trigger = `${pattern.context}: ${pattern.error}`
        const existing = this.rules.find(r => r.trigger === trigger)

        if (!existing) {
          const rule: EvolutionRule = {
            id: uid('evo'),
            trigger,
            action: pattern.solution || `หลีกเลี่ยง ${pattern.errorType} บน ${pattern.context}`,
            source: 'auto',
            occurrences: pattern.failCount,
            createdAt: Date.now(),
          }
          this.rules.push(rule)
          newRules.push(rule)
        } else {
          existing.occurrences = pattern.failCount
        }
      }
    }

    if (newRules.length > 0) {
      this.save()
    }
    return newRules
  }

  /** ตรวจสอบว่ามี rule สำหรับบริบทนี้ไหม */
  getApplicableRules(context: string, error?: string): EvolutionRule[] {
    return this.rules.filter(r => {
      return context.toLowerCase().includes(r.trigger.split(':')[0]?.trim().toLowerCase())
    })
  }

  /** เพิ่ม rule ด้วยมือ */
  addRule(trigger: string, action: string): EvolutionRule {
    const rule: EvolutionRule = {
      id: uid('evo'),
      trigger,
      action,
      source: 'manual',
      occurrences: 0,
      createdAt: Date.now(),
    }
    this.rules.push(rule)
    this.save()
    return rule
  }

  /** ดึง rules ทั้งหมด */
  getAll(): EvolutionRule[] {
    return this.rules
  }

  /** ลบ rule */
  deleteRule(id: string): boolean {
    const idx = this.rules.findIndex(r => r.id === id)
    if (idx === -1) return false
    this.rules.splice(idx, 1)
    this.save()
    return true
  }

  private save(): void {
    saveJSON(EVOLUTION_RULES_FILE, this.rules)
  }
}

// ── BrowserHermes (รวมทุกอย่าง) ──

export class BrowserHermes {
  readonly bugDB: BugPatternsDB
  readonly strategyDB: StrategyDB
  readonly shadowAdvisor: ShadowAdvisor
  readonly selfEvolution: SelfEvolution

  constructor() {
    this.bugDB = new BugPatternsDB()
    this.strategyDB = new StrategyDB()
    this.shadowAdvisor = new ShadowAdvisor(this.bugDB, this.strategyDB)
    this.selfEvolution = new SelfEvolution(this.bugDB)
  }

  /** ประเมิน action ก่อน execute (Iron Law + Shadow Advisor) */
  async preAssess(action: string, url: string, params: Record<string, any> = {}): Promise<RiskAssessment> {
    const assessment = this.shadowAdvisor.assess(action, url, params)

    if (!assessment.allowed) {
      this.shadowAdvisor.logAssessment(action, url, assessment, 'blocked')
      return assessment
    }

    // ตรวจ evolution rules
    const rules = this.selfEvolution.getApplicableRules(url)
    if (rules.length > 0) {
      assessment.suggestions.push(...rules.map(r => `กฎอัตโนมัติ: ${r.action}`))
    }

    return assessment
  }

  /** บันทึกผล action (สำเร็จหรือ fail) */
  recordOutcome(action: string, url: string, params: Record<string, any>, success: boolean, error?: string): void {
    // บันทึก strategy
    try {
      const domain = new URL(url).hostname
      this.strategyDB.record({
        domain,
        task: action,
        selector: params.selector || '',
        method: action,
        success,
      })
    } catch { /* ignore */ }

    // บันทึก bug pattern ถ้า fail
    if (!success && error) {
      this.bugDB.add({
        context: url,
        error,
        errorType: this.classifyError(error),
        solution: '',
      })
    }

    // เช็ค self-evolution
    this.selfEvolution.checkAndEvolve()
  }

  /** จำแนกประเภท error */
  private classifyError(error: string): BugPattern['errorType'] {
    const msg = error.toLowerCase()
    if (msg.includes('selector') || msg.includes('not found') || msg.includes('no element')) return 'selector_not_found'
    if (msg.includes('load') || msg.includes('navigate') || msg.includes('domcontentloaded')) return 'page_load_fail'
    if (msg.includes('timeout') || msg.includes('timed out')) return 'timeout'
    if (msg.includes('navigation') || msg.includes('net::')) return 'navigation_error'
    return 'other'
  }

  /** สถิติทั้งหมด */
  getStats() {
    return {
      bugPatterns: this.bugDB.getAll().length,
      strategies: this.strategyDB.getAll().length,
      evolutionRules: this.selfEvolution.getAll().length,
      shadowLogs: this.shadowAdvisor.getLog(1).length,
    }
  }

  /** สถานะทั้งหมด */
  getStatus() {
    return {
      bugPatterns: this.bugDB.getAll(),
      strategies: this.strategyDB.getAll(),
      evolutionRules: this.selfEvolution.getAll(),
      shadowLog: this.shadowAdvisor.getLog(50),
      stats: this.getStats(),
    }
  }
}
