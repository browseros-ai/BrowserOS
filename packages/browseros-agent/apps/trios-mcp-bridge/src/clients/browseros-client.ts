/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * BrowserOS MCP Client — Robust connection with retry, health checks, circuit breaker.
 * Connects to BrowserOS MCP server for screenshots, snapshots, and browser control.
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { CircuitBreaker } from '../circuit-breaker.js'
import type { ScreenshotResult, SnapshotResult } from '../types.js'

const RETRY_ATTEMPTS = 3
const RETRY_BASE_MS = 200
const HEALTH_CHECK_MS = 30000

export class BrowserOSClient {
  private client: Client | null = null
  private transport: StreamableHTTPClientTransport | null = null
  private serverUrl: string
  private connecting = false
  private healthCheckTimer?: ReturnType<typeof setInterval>
  readonly circuit: CircuitBreaker

  constructor(serverUrl: string) {
    this.serverUrl = serverUrl
    this.circuit = new CircuitBreaker({
      name: 'browseros',
      failureThreshold: 3,
      cooldownMs: 10000,
    })
  }

  /** Establish connection with exponential-backoff retry */
  async connect(): Promise<void> {
    if (this.connecting) {
      while (this.connecting) await sleep(50)
      if (this.client) return
    }

    // Quick ping if already wired
    if (this.client && this.transport) {
      try {
        await this.client.listTools()
        return
      } catch {
        await this.destroy()
      }
    }

    this.connecting = true
    try {
      let lastErr: unknown
      for (let attempt = 1; attempt <= RETRY_ATTEMPTS; attempt++) {
        try {
          await this.destroy()
          this.transport = new StreamableHTTPClientTransport(
            new URL(this.serverUrl),
          )
          this.client = new Client({
            name: 'trios-mcp-bridge',
            version: '0.1.0',
          })
          await this.client.connect(this.transport)
          console.log(`[BrowserOS] Connected to ${this.serverUrl}`)
          return
        } catch (err) {
          lastErr = err
          if (attempt < RETRY_ATTEMPTS) {
            const delay = RETRY_BASE_MS * 2 ** (attempt - 1)
            console.warn(
              `[BrowserOS] Connect attempt ${attempt} failed, retrying in ${delay}ms…`,
            )
            await sleep(delay)
          }
        }
      }
      throw lastErr
    } finally {
      this.connecting = false
    }
  }

  /** Graceful disconnect */
  async disconnect(): Promise<void> {
    this.stopHealthCheck()
    await this.destroy()
    console.log('[BrowserOS] Disconnected')
  }

  get isConnected(): boolean {
    return this.client !== null
  }

  /** Start periodic keep-alive ping */
  startHealthCheck(ms = HEALTH_CHECK_MS): void {
    this.stopHealthCheck()
    this.healthCheckTimer = setInterval(async () => {
      try {
        const c = await this.ensureConnected()
        await c.listTools()
      } catch (err) {
        console.warn(
          '[BrowserOS] Health check failed, will reconnect on next use:',
          err,
        )
        await this.destroy()
      }
    }, ms)
  }

  stopHealthCheck(): void {
    if (this.healthCheckTimer) {
      clearInterval(this.healthCheckTimer)
      this.healthCheckTimer = undefined
    }
  }

  /** Ensure client is healthy, reconnect if needed */
  private async ensureConnected(): Promise<Client> {
    if (this.client && this.transport) {
      try {
        await this.client.listTools()
        return this.client
      } catch {
        await this.destroy()
      }
    }
    await this.connect()
    return this.client!
  }

  /** Call a BrowserOS tool with automatic reconnect on failure */
  private async callBrowserTool(
    name: string,
    args: Record<string, unknown>,
  ): Promise<any> {
    return this.circuit.exec(async () => {
      const client = await this.ensureConnected()
      try {
        return await client.callTool({ name, arguments: args })
      } catch (err) {
        await this.destroy()
        throw err
      }
    })
  }

  // ------------------------------------------------------------------
  // Public APIs
  // ------------------------------------------------------------------

  async listPages(): Promise<
    Array<{ id: number; url: string; title: string }>
  > {
    const result = await this.callBrowserTool('list_pages', {})
    const text = this.extractText(result)
    return this.parsePagesList(text)
  }

  async findGitButlerPage(): Promise<{
    id: number
    url: string
    title: string
  } | null> {
    const pages = await this.listPages()
    return (
      pages.find(
        (p) =>
          p.url.includes('gitbutler') ||
          p.title.toLowerCase().includes('gitbutler'),
      ) ?? null
    )
  }

  async takeScreenshot(pageId: number): Promise<ScreenshotResult> {
    const result = await this.callBrowserTool('take_screenshot', {
      page: pageId,
      format: 'png',
    })

    const imageContent = result.content?.find((c: any) => c.type === 'image')
    if (imageContent?.data) {
      return {
        data: imageContent.data,
        mimeType: imageContent.mimeType || 'image/png',
        devicePixelRatio: 1,
      }
    }
    throw new Error('No screenshot data returned from BrowserOS')
  }

  async takeSnapshot(pageId: number): Promise<SnapshotResult> {
    const result = await this.callBrowserTool('take_snapshot', {
      page: pageId,
    })
    const text = this.extractText(result)
    return { snapshot: text }
  }

  async getPageContent(pageId: number): Promise<string> {
    const result = await this.callBrowserTool('get_page_content', {
      page: pageId,
    })
    return this.extractText(result)
  }

  async clickAt(pageId: number, x: number, y: number): Promise<string> {
    const result = await this.callBrowserTool('click_at', {
      page: pageId,
      x,
      y,
    })
    return this.extractText(result)
  }

  async navigate(pageId: number, url: string): Promise<string> {
    const result = await this.callBrowserTool('navigate_page', {
      page: pageId,
      action: 'url',
      url,
    })
    return this.extractText(result)
  }

  async listTools(): Promise<string[]> {
    return this.circuit.exec(async () => {
      const client = await this.ensureConnected()
      const result = await client.listTools()
      return result.tools.map((t) => t.name)
    })
  }

  async listTabGroups(): Promise<
    Array<{ id: number; title: string; color: string; pageIds: number[] }>
  > {
    try {
      const result = await this.callBrowserTool('list_tab_groups', {})
      const text = this.extractText(result)
      return this.parseTabGroups(text)
    } catch {
      return []
    }
  }

  async createTabGroup(
    pageIds: number[],
    title?: string,
    color?: string,
  ): Promise<{ ok: boolean; groupId?: number }> {
    try {
      const args: Record<string, unknown> = { pageIds }
      if (title) args.title = title
      if (color) args.color = color

      const result = await this.callBrowserTool('group_tabs', args)
      const text = this.extractText(result)
      const match = text.match(/group.*?(\d+)/i)
      return { ok: true, groupId: match ? Number(match[1]) : undefined }
    } catch {
      return { ok: false }
    }
  }

  // ------------------------------------------------------------------
  // Helpers
  // ------------------------------------------------------------------

  private async destroy(): Promise<void> {
    if (this.transport) {
      await this.transport.close().catch(() => {})
      this.transport = null
    }
    if (this.client) {
      await this.client.close().catch(() => {})
      this.client = null
    }
  }

  private extractText(result: any): string {
    if (!result?.content) return ''
    return result.content
      .filter((c: any) => c.type === 'text')
      .map((c: any) => c.text)
      .join('\n')
  }

  private parsePagesList(
    text: string,
  ): Array<{ id: number; url: string; title: string }> {
    const pages: Array<{ id: number; url: string; title: string }> = []
    const lines = text.split('\n')
    for (const line of lines) {
      const match = line.match(/\[(\d+)\]\s+(.+?)\s+(https?:\/\/\S+)/)
      if (match) {
        pages.push({
          id: Number(match[1]),
          title: match[2].trim(),
          url: match[3].trim(),
        })
      }
    }
    return pages
  }

  private parseTabGroups(
    text: string,
  ): Array<{ id: number; title: string; color: string; pageIds: number[] }> {
    try {
      const parsed = JSON.parse(text)
      if (Array.isArray(parsed)) return parsed
    } catch {
      /* fall through */
    }
    const groups: Array<{
      id: number
      title: string
      color: string
      pageIds: number[]
    }> = []
    const lines = text.split('\n')
    for (const line of lines) {
      const match = line.match(
        /group.*?(\d+).*?title[:\s]+(\w+).*?color[:\s]+(\w+)/i,
      )
      if (match) {
        groups.push({
          id: Number(match[1]),
          title: match[2],
          color: match[3],
          pageIds: [],
        })
      }
    }
    return groups
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
