/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * Railway MCP Client — Robust HTTP connection with retry, health checks, circuit breaker.
 * Connects to trios-railway-mcp-production.up.railway.app for deployment orchestration.
 */

import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { CircuitBreaker } from '../circuit-breaker.js'

const RETRY_ATTEMPTS = 3
const RETRY_BASE_MS = 200
const HEALTH_CHECK_MS = 30000

export class RailwayMcpClient {
  private client: Client | null = null
  private transport: StreamableHTTPClientTransport | null = null
  private serverUrl: string
  private connecting = false
  private healthCheckTimer?: ReturnType<typeof setInterval>
  readonly circuit: CircuitBreaker

  constructor(serverUrl: string) {
    this.serverUrl = serverUrl
    this.circuit = new CircuitBreaker({
      name: 'railway',
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
          console.log(`[Railway] Connected to ${this.serverUrl}`)
          return
        } catch (err) {
          lastErr = err
          if (attempt < RETRY_ATTEMPTS) {
            const delay = RETRY_BASE_MS * 2 ** (attempt - 1)
            console.warn(
              `[Railway] Connect attempt ${attempt} failed, retrying in ${delay}ms…`,
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
    console.log('[Railway] Disconnected')
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
          '[Railway] Health check failed, will reconnect on next use:',
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

  /** Call a Railway tool with automatic reconnect on failure */
  private async callRailwayTool(
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

  /** List available Railway tools */
  async listTools(): Promise<string[]> {
    return this.circuit.exec(async () => {
      const client = await this.ensureConnected()
      const result = await client.listTools()
      return result.tools.map((t) => t.name)
    })
  }

  /** Redeploy a Railway service */
  async redeploy(
    serviceId?: string,
    project?: string,
    environment?: string,
  ): Promise<string> {
    const result = await this.callRailwayTool('railway_service_redeploy', {
      service_id: serviceId || undefined,
      project: project || undefined,
      environment: environment || undefined,
    })
    return this.extractText(result)
  }

  /** Deploy (create or update) a Railway service */
  async deploy(args: {
    serviceName: string
    image?: string
    env?: Record<string, string>
    existingServiceId?: string
    project?: string
    environment?: string
  }): Promise<string> {
    const result = await this.callRailwayTool('railway_service_deploy', {
      service_name: args.serviceName,
      image: args.image || undefined,
      env: args.env || undefined,
      existing_service_id: args.existingServiceId || undefined,
      project: args.project || undefined,
      environment: args.environment || undefined,
    })
    return this.extractText(result)
  }

  /** List Railway services in the project */
  async listServices(
    project?: string,
  ): Promise<Array<{ id: string; name: string; created_at: string }>> {
    const result = await this.callRailwayTool('railway_service_list', {
      project: project || undefined,
    })
    const text = this.extractText(result)
    try {
      return JSON.parse(text)
    } catch {
      return []
    }
  }

  /** Batch redeploy services on an account */
  async batchRedeploy(account: number, filter?: string): Promise<string> {
    const result = await this.callRailwayTool('service_batch_redeploy', {
      account,
      filter: filter || undefined,
    })
    return this.extractText(result)
  }

  /** Get fleet health across all accounts */
  async fleetHealth(): Promise<string> {
    const result = await this.callRailwayTool('fleet_health', {})
    return this.extractText(result)
  }

  /** Get worker status from Neon database */
  async workerStatus(): Promise<string> {
    const result = await this.callRailwayTool('worker_status', {})
    return this.extractText(result)
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
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
