// การตั้งค่า Headless Server Mode
// อ่านค่าจาก environment variables หรือใช้ค่าเริ่มต้น

import { existsSync } from 'fs'

export interface HeadlessServerConfig {
  /** พอร์ตที่ server รัน */
  port: number
  /** โฮสต์ที่ bind */
  host: string
  /** พาธไปยัง Chromium binary */
  chromiumPath: string
  /** โหมด debug (แสดง log ละเอียด) */
  debug: boolean
  /** เวลารอโหลดหน้าเว็บสูงสุด (มิลลิวินาที) */
  pageLoadTimeout: number
  /** เวลารอหา element สูงสุด (มิลลิวินาที) */
  waitTimeout: number
  /** ขนาด viewport กว้าง */
  viewportWidth: number
  /** ขนาด viewport สูง */
  viewportHeight: number
  /** โฟลเดอร์เก็บ user data (cookies, localStorage ฯลฯ) */
  userDataDir: string
  /** API key สำหรับยืนยันตัวตน */
  apiKey: string
}

/** หา Chromium ที่ติดตั้งอยู่ในระบบ */
function findChromium(): string {
  const candidates = [
    '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome',
    '/usr/bin/chromium',
    '/snap/bin/chromium',
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  ]

  for (const path of candidates) {
    if (existsSync(path)) return path
  }

  return 'chromium-browser'
}

/** โหลด config จาก environment variables */
export function loadConfig(): HeadlessServerConfig {
  return {
    port: Number(process.env.HEADLESS_PORT || 3100),
    host: process.env.HEADLESS_HOST || '0.0.0.0',
    chromiumPath: process.env.CHROMIUM_PATH || findChromium(),
    debug: process.env.HEADLESS_DEBUG === 'true',
    pageLoadTimeout: Number(process.env.PAGE_LOAD_TIMEOUT || 30000),
    waitTimeout: Number(process.env.WAIT_TIMEOUT || 10000),
    viewportWidth: Number(process.env.VIEWPORT_WIDTH || 1280),
    viewportHeight: Number(process.env.VIEWPORT_HEIGHT || 720),
    userDataDir: process.env.USER_DATA_DIR || '/tmp/browseros-headless-user-data',
    apiKey: process.env.HEADLESS_API_KEY || '',
  }
}
