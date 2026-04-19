// BrowserSkills — ระบบสกิลสำหรับ BrowserOS
// สกิลที่จำวิธีใช้ + optimize ตัวเอง
// เก็บ skill history + success rate

import * as fs from 'fs'
import * as path from 'path'

// ── ประเภทข้อมูล ──

export interface SkillDefinition {
  name: string
  description: string          // ภาษาไทย
  category: string             // 'scraping' | 'interaction' | 'navigation' | 'monitoring' | 'download'
  parameters: SkillParam[]
  execute: (ctx: SkillContext) => Promise<SkillResult>
}

export interface SkillParam {
  name: string
  type: 'string' | 'number' | 'boolean' | 'selector'
  required: boolean
  description: string          // ภาษาไทย
  defaultValue?: any
}

export interface SkillContext {
  sessionId: string
  browser: any                 // HeadlessBrowser instance
  params: Record<string, any>
  hermes?: any                 // BrowserHermes instance (optional)
}

export interface SkillResult {
  success: boolean
  data?: any
  error?: string
  metadata?: Record<string, any>
}

export interface SkillHistoryEntry {
  id: string
  skillName: string
  params: Record<string, any>
  success: boolean
  duration: number             // ms
  timestamp: number
  error?: string
}

export interface SkillStats {
  name: string
  totalRuns: number
  successRuns: number
  successRate: number
  avgDuration: number          // ms
  lastRun: number
}

// ── Storage ──

const DATA_DIR = path.resolve(import.meta.dir, 'data')
const SKILLS_HISTORY_FILE = path.join(DATA_DIR, 'browser-skills-history.json')

function uid(): string {
  return `skill_${Date.now()}_${Math.random().toString(36).substring(2, 6)}`
}

function loadHistory(): SkillHistoryEntry[] {
  try {
    if (fs.existsSync(SKILLS_HISTORY_FILE)) {
      return JSON.parse(fs.readFileSync(SKILLS_HISTORY_FILE, 'utf-8'))
    }
  } catch { /* ignore */ }
  return []
}

function saveHistory(history: SkillHistoryEntry[]): void {
  try {
    if (!fs.existsSync(DATA_DIR)) fs.mkdirSync(DATA_DIR, { recursive: true })
    fs.writeFileSync(SKILLS_HISTORY_FILE, JSON.stringify(history.slice(-1000), null, 2))
  } catch { /* ignore */ }
}

// ── Built-in Skills ──

const builtinSkills: SkillDefinition[] = [
  {
    name: 'web-scrape',
    description: 'ดึงข้อมูลจากเว็บ (เลือก selector อัตโนมัติ)',
    category: 'scraping',
    parameters: [
      { name: 'selector', type: 'selector', required: false, description: 'CSS selector (ถ้าไม่ระบุจะเลือกอัตโนมัติ)' },
      { name: 'fields', type: 'string', required: false, description: 'ฟิลด์ที่ต้องการดึง (คั่นด้วย comma)' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, sessionId, params } = ctx
      try {
        const selector = params.selector || 'body'
        const text = await browser.extractText(sessionId, selector)
        return { success: true, data: { text }, metadata: { selector } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'form-fill',
    description: 'กรอกฟอร์มอัตโนมัติ (เข้าใจ field types)',
    category: 'interaction',
    parameters: [
      { name: 'fields', type: 'string', required: true, description: 'ฟิลด์และค่าที่จะกรอก (JSON)' },
      { name: 'submit', type: 'selector', required: false, description: 'ปุ่ม submit selector' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, sessionId, params } = ctx
      try {
        const fields = typeof params.fields === 'string' ? JSON.parse(params.fields) : params.fields
        const results = await browser.fillForm(sessionId, fields)
        if (params.submit) {
          await browser.click(sessionId, params.submit)
          await new Promise(r => setTimeout(r, 1000))
        }
        return { success: true, data: { results }, metadata: { fields } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'login',
    description: 'login อัตโนมัติ (จำข้อมูล login)',
    category: 'interaction',
    parameters: [
      { name: 'username', type: 'string', required: true, description: 'ชื่อผู้ใช้หรืออีเมล' },
      { name: 'password', type: 'string', required: true, description: 'รหัสผ่าน' },
      { name: 'usernameSelector', type: 'selector', required: false, description: 'selector ช่อง username' },
      { name: 'passwordSelector', type: 'selector', required: false, description: 'selector ช่อง password' },
      { name: 'submitSelector', type: 'selector', required: false, description: 'selector ปุ่ม login' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, sessionId, params } = ctx
      try {
        const uSel = params.usernameSelector || 'input[type="email"], input[name="username"], input[name="email"], #username'
        const pSel = params.passwordSelector || 'input[type="password"], input[name="password"], #password'
        const sSel = params.submitSelector || 'button[type="submit"], input[type="submit"], button:has-text("Login"), button:has-text("เข้าสู่ระบบ")'

        await browser.fillForm(sessionId, {
          [uSel]: params.username,
          [pSel]: params.password,
        })
        await new Promise(r => setTimeout(r, 500))
        await browser.click(sessionId, sSel)
        await new Promise(r => setTimeout(r, 2000))

        return { success: true, data: { loggedIn: true }, metadata: { usernameSelector: uSel } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'search',
    description: 'ค้นหา + กรองผลลัพธ์',
    category: 'navigation',
    parameters: [
      { name: 'query', type: 'string', required: true, description: 'คำค้นหา' },
      { name: 'searchSelector', type: 'selector', required: false, description: 'selector ช่องค้นหา' },
      { name: 'resultSelector', type: 'selector', required: false, description: 'selector ผลลัพธ์' },
      { name: 'maxResults', type: 'number', required: false, description: 'จำนวนผลลัพธ์สูงสุด', defaultValue: 10 },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, sessionId, params } = ctx
      try {
        const sSel = params.searchSelector || 'input[type="search"], input[name="q"], input[name="search"], input[placeholder*="search"]'
        await browser.fillForm(sessionId, { [sSel]: params.query })
        await new Promise(r => setTimeout(r, 500))
        // กด Enter
        await browser.evaluate(sessionId, `document.querySelector('${sSel}')?.form?.submit() || document.querySelector('${sSel}')?.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter'}))`)
        await new Promise(r => setTimeout(r, 2000))

        const rSel = params.resultSelector || '.result, .search-result, [class*="result"], [class*="item"]'
        const results = await browser.extractText(sessionId, rSel)
        return { success: true, data: { results }, metadata: { query: params.query } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'compare',
    description: 'เปรียบเทียบข้อมูลหลายเพจ',
    category: 'scraping',
    parameters: [
      { name: 'urls', type: 'string', required: true, description: 'URL ที่จะเปรียบเทียบ (คั่นด้วย pipe |)' },
      { name: 'selector', type: 'selector', required: false, description: 'selector ที่จะดึงเปรียบเทียบ' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, params } = ctx
      try {
        const urls = params.urls.split('|').map((u: string) => u.trim())
        const selector = params.selector || 'body'
        const results: Record<string, string> = {}

        for (const url of urls) {
          // สร้าง session ชั่วคราว
          const tab = await browser.createTab()
          await browser.navigate(tab, url)
          await browser.waitForLoad(tab)
          const text = await browser.extractText(tab, selector)
          results[url] = text
          await browser.closeTab(tab)
        }

        return { success: true, data: { comparison: results } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'download',
    description: 'ดาวน์โหลดไฟล์',
    category: 'download',
    parameters: [
      { name: 'url', type: 'string', required: true, description: 'URL ไฟล์ที่จะดาวน์โหลด' },
      { name: 'filename', type: 'string', required: false, description: 'ชื่อไฟล์' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { params } = ctx
      try {
        // ตรวจสอบไฟล์อันตราย
        const dangerousExts = ['.exe', '.bat', '.sh', '.cmd', '.ps1', '.vbs']
        const filename = params.filename || params.url.split('/').pop() || 'download'
        if (dangerousExts.some(ext => filename.toLowerCase().endsWith(ext))) {
          return { success: false, error: `ห้ามดาวน์โหลดไฟล์ประเภท ${path.extname(filename)}` }
        }

        // ใช้ fetch ดาวน์โหลด
        const response = await fetch(params.url)
        if (!response.ok) return { success: false, error: `HTTP ${response.status}` }

        const buffer = await response.arrayBuffer()
        const savePath = path.join(DATA_DIR, 'downloads', filename)
        if (!fs.existsSync(path.dirname(savePath))) {
          fs.mkdirSync(path.dirname(savePath), { recursive: true })
        }
        fs.writeFileSync(savePath, Buffer.from(buffer))

        return { success: true, data: { path: savePath, size: buffer.byteLength } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'screenshot',
    description: 'จับภาพหน้าจอ + annotate',
    category: 'interaction',
    parameters: [
      { name: 'fullPage', type: 'boolean', required: false, description: 'จับทั้งหน้า', defaultValue: false },
      { name: 'selector', type: 'selector', required: false, description: 'selector ที่จะจับ (ถ้าไม่ระบุ = ทั้งหน้า)' },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, sessionId, params } = ctx
      try {
        const base64 = await browser.screenshot(sessionId, { fullPage: params.fullPage })
        return { success: true, data: { image: `data:image/png;base64,${base64}` }, metadata: { fullPage: params.fullPage } }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
  {
    name: 'monitor',
    description: 'ติดตามเว็บ (เช็คทุก X นาที)',
    category: 'monitoring',
    parameters: [
      { name: 'url', type: 'string', required: true, description: 'URL ที่จะติดตาม' },
      { name: 'selector', type: 'selector', required: false, description: 'selector ที่จะเฝ้าระวัง' },
      { name: 'intervalMinutes', type: 'number', required: false, description: 'เช็คทุก X นาที', defaultValue: 5 },
      { name: 'checkOnce', type: 'boolean', required: false, description: 'เช็คครั้งเดียว (ไม่ต่อเนื่อง)', defaultValue: true },
    ],
    async execute(ctx): Promise<SkillResult> {
      const { browser, params } = ctx
      try {
        const tab = await browser.createTab()
        await browser.navigate(tab, params.url)
        await browser.waitForLoad(tab)

        const selector = params.selector || 'body'
        const content = await browser.extractText(tab, selector)
        await browser.closeTab(tab)

        return {
          success: true,
          data: { content, url: params.url, timestamp: Date.now() },
          metadata: { intervalMinutes: params.intervalMinutes || 5 },
        }
      } catch (err: any) {
        return { success: false, error: err.message }
      }
    },
  },
]

// ── BrowserSkills ──

export class BrowserSkills {
  private skills: Map<string, SkillDefinition> = new Map()
  private history: SkillHistoryEntry[] = []

  constructor() {
    this.history = loadHistory()
    // ลงทะเบียน built-in skills
    for (const skill of builtinSkills) {
      this.skills.set(skill.name, skill)
    }
  }

  /** ลงทะเบียน skill ใหม่ */
  register(skill: SkillDefinition): void {
    this.skills.set(skill.name, skill)
  }

  /** รัน skill */
  async execute(name: string, ctx: SkillContext): Promise<SkillResult> {
    const skill = this.skills.get(name)
    if (!skill) {
      return { success: false, error: `ไม่พบสกิล "${name}"` }
    }

    const start = Date.now()
    try {
      const result = await skill.execute(ctx)
      this.recordHistory(name, ctx.params, result.success, Date.now() - start, result.error)
      return result
    } catch (err: any) {
      this.recordHistory(name, ctx.params, false, Date.now() - start, err.message)
      return { success: false, error: err.message }
    }
  }

  /** ดึง skill ทั้งหมด */
  getAll(): SkillDefinition[] {
    return Array.from(this.skills.values())
  }

  /** ดึง skill ตามชื่อ */
  get(name: string): SkillDefinition | undefined {
    return this.skills.get(name)
  }

  /** สถิติแต่ละ skill */
  getStats(): SkillStats[] {
    const stats: Map<string, { total: number; success: number; durations: number[]; lastRun: number }> = new Map()

    for (const entry of this.history) {
      const s = stats.get(entry.skillName) || { total: 0, success: 0, durations: [], lastRun: 0 }
      s.total++
      if (entry.success) s.success++
      s.durations.push(entry.duration)
      s.lastRun = Math.max(s.lastRun, entry.timestamp)
      stats.set(entry.skillName, s)
    }

    return Array.from(stats.entries()).map(([name, s]) => ({
      name,
      totalRuns: s.total,
      successRuns: s.success,
      successRate: s.total > 0 ? s.success / s.total : 0,
      avgDuration: s.durations.length > 0 ? s.durations.reduce((a, b) => a + b, 0) / s.durations.length : 0,
      lastRun: s.lastRun,
    }))
  }

  /** ดึงประวัติ */
  getHistory(limit = 100): SkillHistoryEntry[] {
    return this.history.slice(-limit)
  }

  private recordHistory(skillName: string, params: Record<string, any>, success: boolean, duration: number, error?: string): void {
    this.history.push({
      id: uid(),
      skillName,
      params,
      success,
      duration,
      timestamp: Date.now(),
      error,
    })
    if (this.history.length > 1000) {
      this.history = this.history.slice(-1000)
    }
    saveHistory(this.history)
  }
}
