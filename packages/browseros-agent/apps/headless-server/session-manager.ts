// Session Manager
// จัดการ sessions ของผู้ใช้ — เก็บสถานะ cookies, history, และข้อมูลแท็บ

import { HeadlessBrowser } from './headless-browser'

export interface BrowserSession {
  id: string
  tabId: string
  createdAt: number
  lastActivity: number
  history: SessionHistoryEntry[]
}

export interface SessionHistoryEntry {
  timestamp: number
  action: string
  url?: string
  result?: string
  success: boolean
}

/**
 * จัดการหลาย session พร้อมกัน
 * แต่ละ session มีแท็บ Chromium ของตัวเอง
 */
export class SessionManager {
  private sessions = new Map<string, BrowserSession>()
  private browser: HeadlessBrowser

  constructor(browser: HeadlessBrowser) {
    this.browser = browser
  }

  /** สร้าง session ใหม่ */
  async create(sessionId?: string, startUrl?: string): Promise<BrowserSession> {
    const id = sessionId || this.generateId()
    
    // ถ้ามี session เดิมอยู่แล้ว ให้ปิดก่อน
    if (this.sessions.has(id)) {
      await this.destroy(id)
    }

    // เปิดแท็บใหม่ใน Chromium
    const tab = await this.browser.newTab(startUrl)

    const session: BrowserSession = {
      id,
      tabId: tab.id,
      createdAt: Date.now(),
      lastActivity: Date.now(),
      history: [],
    }

    this.sessions.set(id, session)
    this.addHistory(session, 'create', startUrl, 'สร้าง session ใหม่')
    
    return session
  }

  /** ดึงข้อมูล session */
  get(sessionId: string): BrowserSession | undefined {
    return this.sessions.get(sessionId)
  }

  /** ดึง session ทั้งหมด */
  getAll(): BrowserSession[] {
    return Array.from(this.sessions.values())
  }

  /** ปิด session */
  async destroy(sessionId: string): Promise<void> {
    const session = this.sessions.get(sessionId)
    if (!session) return

    try {
      await this.browser.closeTab(session.tabId)
    } catch {
      // ไม่ต้อง error ถ้าแท็บถูกปิดไปแล้ว
    }

    this.sessions.delete(sessionId)
  }

  /** ปิด session ทั้งหมด */
  async destroyAll(): Promise<void> {
    const ids = Array.from(this.sessions.keys())
    await Promise.all(ids.map((id) => this.destroy(id)))
  }

  /** ตรวจสอบว่า session มีอยู่จริง */
  exists(sessionId: string): boolean {
    return this.sessions.has(sessionId)
  }

  /** อัปเดตเวลาใช้งานล่าสุด */
  touch(sessionId: string): void {
    const session = this.sessions.get(sessionId)
    if (session) {
      session.lastActivity = Date.now()
    }
  }

  /** เพิ่มรายการประวัติ */
  addHistory(
    session: BrowserSession,
    action: string,
    url?: string,
    result?: string,
    success = true,
  ): void {
    session.history.push({
      timestamp: Date.now(),
      action,
      url,
      result: result?.substring(0, 500), // จำกัดความยาว
      success,
    })

    // เก็บประวัติสูงสุด 100 รายการ
    if (session.history.length > 100) {
      session.history = session.history.slice(-100)
    }

    session.lastActivity = Date.now()
  }

  /** ดึงข้อมูล cookies ของ session */
  async getCookies(sessionId: string): Promise<any[]> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error(`ไม่พบ session ${sessionId}`)
    return this.browser.getCookies(session.tabId)
  }

  /** ตั้งค่า cookies ให้ session */
  async setCookies(sessionId: string, cookies: any[]): Promise<void> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error(`ไม่พบ session ${sessionId}`)
    await this.browser.setCookies(session.tabId, cookies)
  }

  /** ลบ session ที่ไม่มีการใช้งานเกินเวลาที่กำหนด */
  async cleanupIdleSessions(maxIdleMs: number): Promise<number> {
    const now = Date.now()
    let cleaned = 0

    for (const [id, session] of this.sessions) {
      if (now - session.lastActivity > maxIdleMs) {
        await this.destroy(id)
        cleaned++
      }
    }

    return cleaned
  }

  private generateId(): string {
    return `sess_${Date.now()}_${Math.random().toString(36).substring(2, 8)}`
  }
}
