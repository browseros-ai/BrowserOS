/**
 * Screenshot Capture for eval
 *
 * Supports two modes:
 * - Direct CDP: uses a Browser instance (single-agent path)
 * - MCP: calls take_screenshot via MCP server (orchestrator path)
 */

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { Browser } from '@browseros/server/browser'
import { callMcpTool } from '../utils/mcp-client'

export class ScreenshotCapture {
  private count = 0
  private outputDir: string
  private mcpUrl: string
  private browser: Browser | null
  private devicePixelRatio: number | null = null

  constructor(serverUrl: string, outputDir: string, browser?: Browser) {
    this.mcpUrl = `${serverUrl}/mcp`
    this.outputDir = join(outputDir, 'screenshots')
    this.browser = browser ?? null
  }

  async init(): Promise<void> {
    await mkdir(this.outputDir, { recursive: true })
  }

  setBrowser(browser: Browser): void {
    this.browser = browser
  }

  /**
   * Capture screenshot and save to disk.
   * Uses direct CDP if a browser instance is available, otherwise falls back to MCP.
   */
  async capture(pageId: number): Promise<number> {
    this.count++

    try {
      let base64Data: string | null = null

      if (this.browser) {
        base64Data = await this.captureDirect(pageId)
      } else {
        base64Data = await this.captureMcp(pageId)
      }

      if (base64Data) {
        const filepath = join(this.outputDir, `${this.count}.png`)
        const buffer = Buffer.from(base64Data, 'base64')
        await writeFile(filepath, buffer)
      }

      return this.count
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error)
      console.warn(`Screenshot ${this.count} skipped: ${errorMsg}`)
      return this.count
    }
  }

  private async captureDirect(pageId: number): Promise<string | null> {
    if (!this.browser) return null
    try {
      const result = await this.browser.screenshot(pageId, {
        format: 'png',
        fullPage: false,
      })
      this.devicePixelRatio = result.devicePixelRatio
      return result.data
    } catch (error) {
      // If page ID is invalid, try listing pages and use the first one
      try {
        const pages = await this.browser.listPages()
        if (pages.length > 0) {
          const result = await this.browser.screenshot(pages[0].pageId, {
            format: 'png',
            fullPage: false,
          })
          this.devicePixelRatio = result.devicePixelRatio
          return result.data
        }
      } catch {
        // Give up
      }
      console.warn(
        `Screenshot ${this.count}: CDP error - ${error instanceof Error ? error.message : String(error)}`,
      )
      return null
    }
  }

  private async captureMcp(pageId: number): Promise<string | null> {
    const result = await callMcpTool(this.mcpUrl, 'take_screenshot', {
      format: 'png',
      page: pageId,
    })

    if (result.isError) {
      const errorText =
        result.content?.find((c: { type: string }) => c.type === 'text')
          ?.text || 'Unknown error'
      console.warn(
        `Screenshot ${this.count}: Tool returned error - ${errorText}`,
      )
      return null
    }

    const imageContent = result.content?.find(
      (c: { type: string }) => c.type === 'image',
    )
    return imageContent?.data ?? null
  }

  getCount(): number {
    return this.count
  }

  getOutputDir(): string {
    return this.outputDir
  }

  getDevicePixelRatio(): number {
    return this.devicePixelRatio ?? 1
  }
}
