// Headless Browser Controller
// ควบคุม Chromium ผ่าน CDP (Chrome DevTools Protocol)
// ไม่ต้องติดตั้ง Puppeteer — ใช้ CDP ตรงผ่าน WebSocket

import type { HeadlessServerConfig } from './config'

/** ข้อมูลแท็บที่เปิดอยู่ */
export interface BrowserTab {
  id: string
  url: string
  title: string
  wsUrl: string
}

/**
 * ควบคุม headless Chromium ผ่าน CDP
 * เปิด Chromium ด้วย --remote-debugging-port แล้วสื่อสารผ่าน WebSocket
 */
export class HeadlessBrowser {
  private config: HeadlessServerConfig
  private chromeProcess: any = null
  private debugPort = 9222
  private sessions = new Map<string, WebSocket>()

  constructor(config: HeadlessServerConfig) {
    this.config = config
  }

  /** เริ่ม Chromium headless */
  async start(): Promise<void> {
    const { spawn } = await import('child_process')
    const { mkdirSync, existsSync } = await import('fs')

    // สร้างโฟลเดอร์ user data ถ้ายังไม่มี
    if (!existsSync(this.config.userDataDir)) {
      mkdirSync(this.config.userDataDir, { recursive: true })
    }

    // หาพอร์ต debug ที่ว่าง
    this.debugPort = await this.findFreePort()

    const args = [
      '--headless=new',
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--disable-gpu',
      '--disable-dev-shm-usage',
      '--disable-extensions',
      '--disable-background-timer-throttling',
      '--disable-backgrounding-occluded-windows',
      '--disable-renderer-backgrounding',
      '--no-first-run',
      '--disable-default-apps',
      `--remote-debugging-port=${this.debugPort}`,
      `--user-data-dir=${this.config.userDataDir}`,
      `--window-size=${this.config.viewportWidth},${this.config.viewportHeight}`,
    ]

    this.log(`กำลังเริ่ม Chromium: ${this.config.chromiumPath} ${args.join(' ')}`)

    this.chromeProcess = spawn(this.config.chromiumPath, args, {
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, DISPLAY: '' },
    })

    this.chromeProcess.stdout?.on('data', (data: Buffer) => {
      if (this.config.debug) {
        this.log(`Chrome stdout: ${data.toString().trim()}`)
      }
    })

    this.chromeProcess.stderr?.on('data', (data: Buffer) => {
      if (this.config.debug) {
        this.log(`Chrome stderr: ${data.toString().trim()}`)
      }
    })

    this.chromeProcess.on('exit', (code: number) => {
      this.log(`Chromium ปิดตัวลง (code: ${code})`)
    })

    // รอให้ Chromium พร้อม
    await this.waitForChrome()
    this.log('Chromium พร้อมใช้งานแล้ว!')
  }

  /** ปิด Chromium */
  async stop(): Promise<void> {
    if (this.chromeProcess) {
      this.chromeProcess.kill('SIGTERM')
      this.chromeProcess = null
    }
    // ปิด WebSocket sessions ทั้งหมด
    for (const [id, ws] of this.sessions) {
      ws.close()
    }
    this.sessions.clear()
    this.log('ปิด Chromium แล้ว')
  }

  /** ดึงรายการแท็บที่เปิดอยู่ */
  async getTabs(): Promise<BrowserTab[]> {
    const resp = await fetch(`http://127.0.0.1:${this.debugPort}/json`)
    const tabs = (await resp.json()) as any[]
    return tabs
      .filter((t) => t.type === 'page')
      .map((t) => ({
        id: t.id,
        url: t.url,
        title: t.title,
        wsUrl: t.webSocketDebuggerUrl,
      }))
  }

  /** เปิดแท็บใหม่ */
  async newTab(url?: string): Promise<BrowserTab> {
    const targetUrl = url || 'about:blank'
    const resp = await fetch(`http://127.0.0.1:${this.debugPort}/json/new?${targetUrl}`)
    const tab = (await resp.json()) as any
    return {
      id: tab.id,
      url: tab.url,
      title: tab.title,
      wsUrl: tab.webSocketDebuggerUrl,
    }
  }

  /** ปิดแท็บ */
  async closeTab(tabId: string): Promise<void> {
    await fetch(`http://127.0.0.1:${this.debugPort}/json/close/${tabId}`)
    this.sessions.delete(tabId)
  }

  /** ส่งคำสั่ง CDP ไปยังแท็บ */
  async sendCDPCommand(
    tabId: string,
    method: string,
    params: Record<string, any> = {},
  ): Promise<any> {
    const tabs = await this.getTabs()
    const tab = tabs.find((t) => t.id === tabId)
    if (!tab) throw new Error(`ไม่พบแท็บ ${tabId}`)

    return new Promise((resolve, reject) => {
      const ws = new WebSocket(tab.wsUrl)
      const msgId = Date.now()

      ws.addEventListener('open', () => {
        ws.send(JSON.stringify({ id: msgId, method, params }))
      })

      ws.addEventListener('message', (event) => {
        const data = JSON.parse(event.data as string)
        if (data.id === msgId) {
          ws.close()
          if (data.error) {
            reject(new Error(data.error.message || 'CDP error'))
          } else {
            resolve(data.result)
          }
        }
      })

      ws.addEventListener('error', () => {
        ws.close()
        reject(new Error('WebSocket connection error'))
      })
    })
  }

  /** นำทางไปยัง URL */
  async navigate(tabId: string, url: string): Promise<{ frameId: string; loaderId: string }> {
    return this.sendCDPCommand(tabId, 'Page.navigate', { url })
  }

  /** รอให้หน้าโหลดเสร็จ */
  async waitForLoad(tabId: string, timeout?: number): Promise<void> {
    const ms = timeout || this.config.pageLoadTimeout
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(new Error(`รอโหลดหน้าเกินเวลา ${ms}ms`))
      }, ms)

      // ใช้ DOMContentLoaded event ผ่าน Runtime.evaluate
      this.sendCDPCommand(tabId, 'Runtime.evaluate', {
        expression: `new Promise(r => { if(document.readyState==='complete') r(); else window.addEventListener('load', r); })`,
        awaitPromise: true,
        returnByValue: true,
      })
        .then(() => {
          clearTimeout(timer)
          resolve()
        })
        .catch((err) => {
          clearTimeout(timer)
          reject(err)
        })
    })
  }

  /** จับภาพหน้าจอ */
  async screenshot(tabId: string, options?: { quality?: number; fullPage?: boolean }): Promise<string> {
    const result = await this.sendCDPCommand(tabId, 'Page.captureScreenshot', {
      format: 'png',
      quality: options?.quality,
      captureBeyondViewport: options?.fullPage,
    })
    return result.data
  }

  /** ดึงข้อความจากหน้าเว็บ */
  async extractText(tabId: string, selector?: string): Promise<string> {
    const expr = selector
      ? `document.querySelector('${selector}')?.innerText || ''`
      : `document.body.innerText`
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: expr,
      returnByValue: true,
    })
    return result.result?.value || ''
  }

  /** ดึงข้อมูล DOM ตาม selector */
  async extractHTML(tabId: string, selector: string): Promise<string> {
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: `document.querySelector('${selector}')?.outerHTML || ''`,
      returnByValue: true,
    })
    return result.result?.value || ''
  }

  /** คลิกที่ element */
  async click(tabId: string, selector: string): Promise<boolean> {
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: `
        (function() {
          const el = document.querySelector('${selector}');
          if (!el) return false;
          el.scrollIntoView({ block: 'center' });
          el.click();
          return true;
        })()
      `,
      returnByValue: true,
    })
    return result.result?.value === true
  }

  /** พิมพ์ข้อความลงใน input */
  async type(tabId: string, selector: string, text: string): Promise<boolean> {
    // โฟกัสที่ element แล้วเคลียร์ค่าเดิม
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: `
        (function() {
          const el = document.querySelector('${selector}');
          if (!el) return false;
          el.scrollIntoView({ block: 'center' });
          el.focus();
          el.value = '';
          el.dispatchEvent(new Event('input', { bubbles: true }));
          return true;
        })()
      `,
      returnByValue: true,
    })

    if (result.result?.value !== true) return false

    // พิมพ์ทีละตัวเพื่อให้ event handlers ทำงาน
    for (const char of text) {
      await this.sendCDPCommand(tabId, 'Input.dispatchKeyEvent', {
        type: 'keyDown',
        text: char,
      })
      await this.sendCDPCommand(tabId, 'Input.dispatchKeyEvent', {
        type: 'keyUp',
        text: char,
      })
    }

    return true
  }

  /** กรอกข้อมูลในฟอร์ม (หลายฟิลด์พร้อมกัน) */
  async fillForm(
    tabId: string,
    fields: Record<string, string>,
  ): Promise<Record<string, boolean>> {
    const results: Record<string, boolean> = {}
    for (const [selector, value] of Object.entries(fields)) {
      results[selector] = await this.type(tabId, selector, value)
    }
    return results
  }

  /** รัน JavaScript บนหน้าเว็บ */
  async evaluate(tabId: string, expression: string): Promise<any> {
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression,
      returnByValue: true,
    })
    return result.result?.value
  }

  /** ดึง cookies ของหน้าปัจจุบัน */
  async getCookies(tabId: string): Promise<any[]> {
    const result = await this.sendCDPCommand(tabId, 'Network.getCookies')
    return result.cookies || []
  }

  /** ตั้งค่า cookies */
  async setCookies(tabId: string, cookies: any[]): Promise<void> {
    await this.sendCDPCommand(tabId, 'Network.setCookies', { cookies })
  }

  /** ดึง URL ปัจจุบัน */
  async getCurrentUrl(tabId: string): Promise<string> {
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: 'window.location.href',
      returnByValue: true,
    })
    return result.result?.value || ''
  }

  /** ดึง title ปัจจุบัน */
  async getTitle(tabId: string): Promise<string> {
    const result = await this.sendCDPCommand(tabId, 'Runtime.evaluate', {
      expression: 'document.title',
      returnByValue: true,
    })
    return result.result?.value || ''
  }

  // --- private helpers ---

  private log(msg: string): void {
    console.log(`[HeadlessBrowser] ${msg}`)
  }

  private async findFreePort(): Promise<number> {
    const { createServer } = await import('net')
    return new Promise((resolve, reject) => {
      const server = createServer()
      server.listen(0, '127.0.0.1', () => {
        const addr = server.address() as any
        server.close(() => resolve(addr.port))
      })
      server.on('error', reject)
    })
  }

  private async waitForChrome(maxRetries = 20): Promise<void> {
    for (let i = 0; i < maxRetries; i++) {
      try {
        const resp = await fetch(`http://127.0.0.1:${this.debugPort}/json/version`)
        if (resp.ok) return
      } catch {
        // ยังไม่พร้อม รอสักครู่
      }
      await new Promise((r) => setTimeout(r, 500))
    }
    throw new Error('รอ Chromium เริ่มตัวเกินเวลา')
  }
}
