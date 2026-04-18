// File Manager — จัดการไฟล์ที่ดาวน์โหลดจากเว็บ
// ดาวน์โหลด, จัดโฟลเดอร์, แปลง format, รายการไฟล์

import { HeadlessBrowser } from './headless-browser'
import { SessionManager } from './session-manager'
import { mkdir, readdir, stat, unlink, rename, copyFile } from 'fs/promises'
import { existsSync, writeFileSync, readFileSync } from 'fs'
import { join, extname, basename, dirname } from 'path'

export interface FileInfo {
  name: string
  path: string
  size: number
  extension: string
  modifiedAt: number
  type: 'file' | 'directory'
}

export interface DownloadResult {
  success: boolean
  path: string
  size: number
  filename: string
}

export interface OrganizeResult {
  moved: number
  created: number
  errors: string[]
}

const DEFAULT_DOWNLOAD_DIR = '/tmp/browseros-downloads'

/**
 * File Manager
 * ดาวน์โหลดไฟล์จากเว็บ, จัดโฟลเดอร์, แปลง format
 */
export class FileManager {
  private browser: HeadlessBrowser
  private sessions: SessionManager
  private downloadDir: string

  constructor(
    browser: HeadlessBrowser,
    sessions: SessionManager,
    downloadDir?: string,
  ) {
    this.browser = browser
    this.sessions = sessions
    this.downloadDir = downloadDir || DEFAULT_DOWNLOAD_DIR
    this.ensureDir(this.downloadDir)
  }

  /** ดาวน์โหลดไฟล์จาก URL */
  async downloadFile(url: string, filename?: string, subdir?: string): Promise<DownloadResult> {
    const dir = subdir ? join(this.downloadDir, subdir) : this.downloadDir
    this.ensureDir(dir)

    const fname = filename || basename(new URL(url).pathname) || `download_${Date.now()}`
    const filePath = join(dir, fname)

    try {
      const resp = await fetch(url)
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`)

      const buffer = await resp.arrayBuffer()
      writeFileSync(filePath, Buffer.from(buffer))

      return {
        success: true,
        path: filePath,
        size: buffer.byteLength,
        filename: fname,
      }
    } catch (err: any) {
      return {
        success: false,
        path: filePath,
        size: 0,
        filename: fname,
      }
    }
  }

  /** ดึงข้อมูลจากหน้าเว็บแล้วบันทึกเป็นไฟล์ */
  async extractAndSave(
    sessionId: string,
    options: {
      selector?: string
      format: 'txt' | 'html' | 'json'
      filename?: string
    }
  ): Promise<DownloadResult> {
    const session = this.sessions.get(sessionId)
    if (!session) throw new Error('ไม่พบ session')

    let content: string
    let fname = options.filename || `extract_${Date.now()}`

    switch (options.format) {
      case 'html': {
        content = await this.browser.extractHTML(session.tabId, options.selector || 'body')
        fname += '.html'
        break
      }
      case 'json': {
        const text = await this.browser.extractText(session.tabId, options.selector)
        content = JSON.stringify({ extracted: text, url: await this.browser.getCurrentUrl(session.tabId), timestamp: Date.now() }, null, 2)
        fname += '.json'
        break
      }
      default: {
        content = await this.browser.extractText(session.tabId, options.selector)
        fname += '.txt'
        break
      }
    }

    const filePath = join(this.downloadDir, fname)
    writeFileSync(filePath, content, 'utf-8')

    return {
      success: true,
      path: filePath,
      size: Buffer.byteLength(content),
      filename: fname,
    }
  }

  /** รายการไฟล์ */
  async listFiles(subdir?: string): Promise<FileInfo[]> {
    const dir = subdir ? join(this.downloadDir, subdir) : this.downloadDir
    if (!existsSync(dir)) return []

    const entries = await readdir(dir)
    const files: FileInfo[] = []

    for (const name of entries) {
      const fullPath = join(dir, name)
      try {
        const s = await stat(fullPath)
        files.push({
          name,
          path: fullPath,
          size: s.size,
          extension: extname(name),
          modifiedAt: s.mtimeMs,
          type: s.isDirectory() ? 'directory' : 'file',
        })
      } catch {
        // skip
      }
    }

    return files.sort((a, b) => b.modifiedAt - a.modifiedAt)
  }

  /** จัดระเบียบไฟล์ — ย้ายตาม extension */
  async organizeFiles(subdir?: string): Promise<OrganizeResult> {
    const result: OrganizeResult = { moved: 0, created: 0, errors: [] }
    const dir = subdir ? join(this.downloadDir, subdir) : this.downloadDir

    const files = await this.listFiles(subdir)
    const categories: Record<string, string[]> = {
      'images': ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.svg'],
      'documents': ['.pdf', '.doc', '.docx', '.txt', '.md'],
      'data': ['.json', '.csv', '.xml', '.xlsx'],
      'html': ['.html', '.htm'],
      'other': [],
    }

    for (const file of files) {
      if (file.type === 'directory') continue

      // หา category
      let category = 'other'
      for (const [cat, exts] of Object.entries(categories)) {
        if (exts.includes(file.extension.toLowerCase())) {
          category = cat
          break
        }
      }

      const targetDir = join(dir, category)
      this.ensureDir(targetDir)
      result.created++

      const targetPath = join(targetDir, file.name)
      if (targetPath !== file.path) {
        try {
          await rename(file.path, targetPath)
          result.moved++
        } catch (err: any) {
          result.errors.push(`${file.name}: ${err.message}`)
        }
      }
    }

    return result
  }

  /** ลบไฟล์ */
  async deleteFile(path: string): Promise<boolean> {
    // ตรวจสอบว่า path อยู่ใน download dir
    if (!path.startsWith(this.downloadDir)) {
      throw new Error('ไม่อนุญาตให้ลบไฟล์นอกโฟลเดอร์ดาวน์โหลด')
    }

    try {
      await unlink(path)
      return true
    } catch {
      return false
    }
  }

  /** แปลง format */
  async convertFormat(inputPath: string, outputFormat: string): Promise<DownloadResult> {
    if (!existsSync(inputPath)) {
      throw new Error('ไม่พบไฟล์ต้นทาง')
    }

    const content = readFileSync(inputPath, 'utf-8')
    const ext = extname(inputPath)
    const base = basename(inputPath, ext)
    let output: string
    let outExt: string

    switch (`${ext}.${outputFormat}`) {
      case '.html.pdf':
      case '.htm.pdf': {
        // HTML → text (simplified, no real PDF without deps)
        output = content.replace(/<[^>]+>/g, '').trim()
        outExt = '.txt'
        break
      }
      case '.csv.json': {
        // CSV → JSON
        const lines = content.trim().split('\n')
        const headers = lines[0].split(',')
        const data = lines.slice(1).map(line => {
          const values = line.split(',')
          const obj: Record<string, string> = {}
          headers.forEach((h, i) => { obj[h.trim()] = (values[i] || '').trim() })
          return obj
        })
        output = JSON.stringify(data, null, 2)
        outExt = '.json'
        break
      }
      case '.json.csv': {
        // JSON → CSV
        try {
          const arr = JSON.parse(content)
          if (Array.isArray(arr) && arr.length > 0) {
            const headers = Object.keys(arr[0])
            const csv = [headers.join(','), ...arr.map((row: any) => headers.map(h => row[h] ?? '').join(','))].join('\n')
            output = csv
          } else {
            output = content
          }
        } catch {
          output = content
        }
        outExt = '.csv'
        break
      }
      default: {
        output = content
        outExt = `.${outputFormat}`
      }
    }

    const outputPath = join(dirname(inputPath), `${base}_converted${outExt}`)
    writeFileSync(outputPath, output, 'utf-8')

    return {
      success: true,
      path: outputPath,
      size: Buffer.byteLength(output),
      filename: basename(outputPath),
    }
  }

  /** ตั้งค่าโฟลเดอร์ดาวน์โหลด */
  setDownloadDir(path: string): void {
    this.downloadDir = path
    this.ensureDir(path)
  }

  /** ดึงโฟลเดอร์ดาวน์โหลดปัจจุบัน */
  getDownloadDir(): string {
    return this.downloadDir
  }

  private ensureDir(dir: string): void {
    if (!existsSync(dir)) {
      try {
        const fs = require('fs')
        fs.mkdirSync(dir, { recursive: true })
      } catch {
        // ignore
      }
    }
  }
}
