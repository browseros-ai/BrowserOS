/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * trios-mcp-rag Client — Resilient stdio connection with retry & health checks.
 * Connects to trios-mcp-rag Rust MCP server via stdio subprocess.
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'
import { CircuitBreaker } from '../circuit-breaker.js'

const RETRY_ATTEMPTS = 3
const RETRY_BASE_MS = 200
const HEALTH_CHECK_MS = 30000

export class TriosRagClient {
  private client: Client | null = null
  private transport: StdioClientTransport | null = null
  private cliPath: string
  private databaseUrl: string | null
  private connecting = false
  private healthCheckTimer?: ReturnType<typeof setInterval>
  readonly circuit: CircuitBreaker

  constructor(cliPath: string, databaseUrl?: string) {
    this.cliPath = cliPath
    this.databaseUrl = databaseUrl || null
    this.circuit = new CircuitBreaker({
      name: 'trios-rag',
      failureThreshold: 3,
      cooldownMs: 10000,
    })
  }

  /** Establish stdio connection with exponential-backoff retry */
  async connect(): Promise<void> {
    if (this.connecting) {
      while (this.connecting) await sleep(50)
      if (this.client) return
    }

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
          const env: Record<string, string> = {}
          for (const [k, v] of Object.entries(process.env)) {
            if (v !== undefined) env[k] = v
          }
          if (this.databaseUrl) {
            env.DATABASE_URL = this.databaseUrl
          }

          this.transport = new StdioClientTransport({
            command: this.cliPath,
            args: [],
            env,
          })

          this.client = new Client({
            name: 'trios-mcp-bridge',
            version: '0.1.0',
          })

          await this.client.connect(this.transport)
          console.log('[TriosRag] Connected to trios-mcp-rag MCP')
          return
        } catch (err) {
          lastErr = err
          if (attempt < RETRY_ATTEMPTS) {
            const delay = RETRY_BASE_MS * 2 ** (attempt - 1)
            console.warn(
              `[TriosRag] Connect attempt ${attempt} failed, retrying in ${delay}ms…`,
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

  async disconnect(): Promise<void> {
    this.stopHealthCheck()
    await this.destroy()
    console.log('[TriosRag] Disconnected')
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
          '[TriosRag] Health check failed, will reconnect on next use:',
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

  /** List available tools from trios-mcp-rag */
  async listTools(): Promise<string[]> {
    return this.circuit.exec(async () => {
      const client = await this.ensureConnected()
      const result = await client.listTools()
      return result.tools.map((t) => t.name)
    })
  }

  /** Call any tool on trios-mcp-rag by name with arguments */
  async callTool(name: string, args: Record<string, unknown>): Promise<any> {
    return this.circuit.exec(async () => {
      const client = await this.ensureConnected()
      return await client.callTool({ name, arguments: args })
    })
  }

  /** Extract text content from MCP result */
  extractText(result: any): string {
    if (!result?.content) return ''
    return result.content
      .filter((c: any) => c.type === 'text')
      .map((c: any) => c.text)
      .join('\n')
  }

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
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
