import {
  createProtocolApi,
  type RawOn,
  type RawSend,
} from '@browseros/cdp-protocol/create-api'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { EXIT_CODES } from '@browseros/shared/constants/exit-codes'
import { CDP_LIMITS } from '@browseros/shared/constants/limits'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { logger } from '../../lib/logger'
import type { CdpTarget, CdpBackend as ICdpBackend } from './types'

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (reason: Error) => void
  timer: ReturnType<typeof setTimeout>
}

interface CdpVersion {
  webSocketDebuggerUrl: string
}

const LOOPBACK_DISCOVERY_HOSTS = ['127.0.0.1', 'localhost', '[::1]'] as const
type LoopbackDiscoveryHost = (typeof LOOPBACK_DISCOVERY_HOSTS)[number]

interface CdpBackendConfig {
  port: number
  exitOnReconnectFailure?: boolean
}

// biome-ignore lint/correctness/noUnusedVariables: declaration merging adds ProtocolApi properties to the class
interface CdpBackend extends ProtocolApi {}
// biome-ignore lint/suspicious/noUnsafeDeclarationMerging: intentional — Object.assign fills these at runtime
class CdpBackend implements ICdpBackend {
  private port: number
  private exitOnReconnectFailure: boolean
  private ws: WebSocket | null = null
  private messageId = 0
  private pending = new Map<number, PendingRequest>()
  private connected = false
  private disconnecting = false
  private reconnecting = false
  private reconnectRequested = false
  private eventHandlers = new Map<string, ((params: unknown) => void)[]>()
  private sessionEventHandlers = new Map<
    string,
    ((params: unknown, sessionId: string) => void)[]
  >()
  private sessionCache = new Map<string, ProtocolApi>()
  private keepaliveTimer: ReturnType<typeof setInterval> | null = null
  private preferredDiscoveryHost: LoopbackDiscoveryHost | null = null

  constructor(config: CdpBackendConfig) {
    this.port = config.port
    this.exitOnReconnectFailure = config.exitOnReconnectFailure ?? true

    const rawSend: RawSend = (method, params) => this.rawSend(method, params)
    const rawOn: RawOn = (event, handler) => this.rawOn(event, handler)
    Object.assign(this, createProtocolApi(rawSend, rawOn))
  }

  async connect(): Promise<void> {
    const maxRetries = CDP_LIMITS.CONNECT_MAX_RETRIES
    const retryDelay = TIMEOUTS.CDP_CONNECT_RETRY_DELAY

    for (let attempt = 1; attempt <= maxRetries; attempt++) {
      try {
        await this.attemptConnect()
        this.startKeepalive()
        return
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error)
        if (attempt < maxRetries) {
          logger.warn(
            `CDP connection attempt ${attempt}/${maxRetries} failed: ${msg}. Retrying in ${retryDelay}ms...`,
          )
          await Bun.sleep(retryDelay)
        } else {
          throw new Error(
            `CDP connection failed after ${maxRetries} attempts: ${msg}`,
          )
        }
      }
    }
  }

  private async attemptConnect(): Promise<void> {
    const { host, version } = await this.discoverVersion()
    const wsUrl = this.resolveWebSocketUrl(version.webSocketDebuggerUrl, host)

    return new Promise<void>((resolve, reject) => {
      let opened = false
      let settled = false
      const ws = new WebSocket(wsUrl)
      const connectTimeout = setTimeout(() => {
        if (settled) return
        settled = true
        try {
          ws.close()
        } catch {
          // Ignore close errors from half-open sockets
        }
        reject(
          new Error(
            `CDP WebSocket connect timeout after ${TIMEOUTS.CDP_CONNECT}ms`,
          ),
        )
      }, TIMEOUTS.CDP_CONNECT)

      ws.onopen = () => {
        if (settled) return
        settled = true
        clearTimeout(connectTimeout)
        opened = true
        this.ws = ws
        this.connected = true
        this.disconnecting = false
        resolve()
      }

      ws.onerror = (event) => {
        if (!opened && !settled) {
          settled = true
          clearTimeout(connectTimeout)
          reject(new Error(`CDP WebSocket error: ${event}`))
        }
      }

      ws.onclose = () => {
        clearTimeout(connectTimeout)
        // Guard against stale onclose from a replaced socket
        if (this.ws !== ws) return
        this.connected = false
        this.ws = null
        if (opened) this.handleUnexpectedClose()
      }

      ws.onmessage = (event) => {
        this.handleMessage(event.data as string)
      }
    })
  }

  private getDiscoveryHosts(): LoopbackDiscoveryHost[] {
    if (!this.preferredDiscoveryHost) {
      return [...LOOPBACK_DISCOVERY_HOSTS]
    }
    return [
      this.preferredDiscoveryHost,
      ...LOOPBACK_DISCOVERY_HOSTS.filter(
        (host) => host !== this.preferredDiscoveryHost,
      ),
    ]
  }

  private async discoverVersion(): Promise<{
    host: LoopbackDiscoveryHost
    version: CdpVersion
  }> {
    const failures: string[] = []

    for (const host of this.getDiscoveryHosts()) {
      try {
        const version = await this.fetchVersionFromHost(host)
        this.preferredDiscoveryHost = host
        return { host, version }
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error)
        failures.push(`${host}: ${msg}`)
        logger.debug(`CDP discovery failed via ${host}: ${msg}`)
      }
    }

    throw new Error(
      `CDP /json/version failed on all loopback hosts (${failures.join('; ')})`,
    )
  }

  private async fetchVersionFromHost(
    host: LoopbackDiscoveryHost,
  ): Promise<CdpVersion> {
    const response = await fetch(`http://${host}:${this.port}/json/version`, {
      signal: AbortSignal.timeout(TIMEOUTS.CDP_CONNECT),
    })
    if (!response.ok) {
      throw new Error(
        `CDP /json/version failed with HTTP ${response.status}: ${response.statusText}`,
      )
    }

    const version = (await response.json()) as Partial<CdpVersion>
    if (typeof version.webSocketDebuggerUrl !== 'string') {
      throw new Error('CDP /json/version missing webSocketDebuggerUrl')
    }

    return { webSocketDebuggerUrl: version.webSocketDebuggerUrl }
  }

  private resolveWebSocketUrl(
    wsUrl: string,
    host: LoopbackDiscoveryHost,
  ): string {
    try {
      const parsedUrl = new URL(wsUrl)
      parsedUrl.hostname = this.normalizeHost(host)
      return parsedUrl.toString()
    } catch {
      return wsUrl
    }
  }

  private normalizeHost(host: LoopbackDiscoveryHost): string {
    if (host === '[::1]') {
      return '::1'
    }
    return host
  }

  private startKeepalive(): void {
    this.stopKeepalive()

    const interval = TIMEOUTS.CDP_KEEPALIVE_INTERVAL
    const timeout = TIMEOUTS.CDP_KEEPALIVE_TIMEOUT

    this.keepaliveTimer = setInterval(async () => {
      if (!this.ws || !this.connected || this.disconnecting) return

      let timeoutId: ReturnType<typeof setTimeout> | undefined
      try {
        await Promise.race([
          this.rawSend('Browser.getVersion'),
          new Promise((_, reject) => {
            timeoutId = setTimeout(
              () => reject(new Error('CDP keepalive timeout')),
              timeout,
            )
          }),
        ])
        clearTimeout(timeoutId)
      } catch {
        clearTimeout(timeoutId)
        logger.warn('CDP keepalive failed, connection may be dead')
        this.handleDeadConnection()
      }
    }, interval)
  }

  private stopKeepalive(): void {
    if (this.keepaliveTimer) {
      clearInterval(this.keepaliveTimer)
      this.keepaliveTimer = null
    }
  }

  /**
   * Force-close a zombie WebSocket that stopped responding but never
   * fired onclose. This triggers the normal reconnection path.
   */
  private handleDeadConnection(): void {
    if (this.disconnecting || this.reconnecting) return

    this.stopKeepalive()

    if (this.ws) {
      try {
        this.ws.close()
      } catch {
        // Already dead, ignore
      }
      this.ws = null
    }
    this.connected = false
    this.handleUnexpectedClose()
  }

  private handleUnexpectedClose(): void {
    if (this.disconnecting) return

    this.stopKeepalive()
    this.rejectPendingRequests()

    // If a freshly opened socket closes before the previous reconnect loop
    // finishes, queue another reconnect instead of dropping into a dead state.
    if (this.reconnecting) {
      this.reconnectRequested = true
      logger.warn('CDP closed while reconnecting, queueing another reconnect')
      return
    }

    logger.error(
      'CDP WebSocket closed unexpectedly, attempting reconnection...',
    )
    this.reconnecting = true
    this.reconnectRequested = false
    this.reconnectLoop().finally(() => {
      this.reconnecting = false
    })
  }

  private async reconnectLoop(): Promise<void> {
    do {
      this.reconnectRequested = false
      const reconnected = await this.reconnectWithRetries()
      if (!reconnected) return
    } while (
      !this.disconnecting &&
      (this.reconnectRequested || !this.connected)
    )
  }

  private rejectPendingRequests(): void {
    const error = new Error('CDP connection lost')
    for (const request of this.pending.values()) {
      clearTimeout(request.timer)
      request.reject(error)
    }
    this.pending.clear()
  }

  private async reconnectWithRetries(): Promise<boolean> {
    const maxRetries = CDP_LIMITS.RECONNECT_MAX_RETRIES
    const delay = TIMEOUTS.CDP_RECONNECT_DELAY

    for (let attempt = 1; attempt <= maxRetries; attempt++) {
      if (this.disconnecting) return false

      try {
        logger.info(`CDP reconnection attempt ${attempt}/${maxRetries}...`)
        await Bun.sleep(delay)
        await this.attemptConnect()
        this.startKeepalive()
        logger.info('CDP reconnected successfully')
        return true
      } catch (error) {
        const msg = error instanceof Error ? error.message : String(error)
        logger.warn(
          `CDP reconnection attempt ${attempt}/${maxRetries} failed: ${msg}`,
        )
      }
    }

    if (this.exitOnReconnectFailure) {
      logger.error(
        `CDP reconnection failed after ${maxRetries} attempts, exiting for restart`,
      )
      process.exit(EXIT_CODES.GENERAL_ERROR)
    }
    logger.error(`CDP reconnection failed after ${maxRetries} attempts`)
    return false
  }

  async disconnect(): Promise<void> {
    this.disconnecting = true
    this.stopKeepalive()
    if (this.ws) {
      this.ws.close()
      this.ws = null
      this.connected = false
    }
    this.rejectPendingRequests()
  }

  isConnected(): boolean {
    return this.connected
  }

  session(sessionId: string): ProtocolApi {
    let cached = this.sessionCache.get(sessionId)
    if (!cached) {
      cached = createProtocolApi(
        (method, params) => this.rawSend(method, params, sessionId),
        (event, handler) => this.rawOn(event, handler),
      )
      this.sessionCache.set(sessionId, cached)
    }
    return cached
  }

  async getTargets(): Promise<CdpTarget[]> {
    const result = await this.Target.getTargets()

    return result.targetInfos.map((t) => ({
      id: t.targetId,
      type: t.type,
      title: t.title,
      url: t.url,
      tabId: t.tabId,
      windowId: t.windowId,
    }))
  }

  private async rawSend(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown> {
    if (!this.ws || !this.connected) {
      throw new Error('CDP not connected')
    }

    const id = ++this.messageId
    const message: Record<string, unknown> = {
      id,
      method,
      params: params ?? {},
    }
    if (sessionId) {
      message.sessionId = sessionId
    }

    const ws = this.ws
    return new Promise<unknown>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`CDP request timeout: ${method} (id=${id})`))
      }, TIMEOUTS.CDP_REQUEST_TIMEOUT)

      this.pending.set(id, { resolve, reject, timer })

      try {
        ws.send(JSON.stringify(message))
      } catch (err) {
        clearTimeout(timer)
        this.pending.delete(id)
        const msg = err instanceof Error ? err.message : String(err)
        reject(new Error(`CDP send failed: ${msg}`))

        // send() failure likely means the socket is dead
        this.handleDeadConnection()
      }
    })
  }

  private rawOn(event: string, handler: (params: unknown) => void): () => void {
    if (!this.eventHandlers.has(event)) {
      this.eventHandlers.set(event, [])
    }
    const handlers = this.eventHandlers.get(event)
    if (handlers) {
      handlers.push(handler)
    }
    return () => {
      const list = this.eventHandlers.get(event)
      if (list) {
        const idx = list.indexOf(handler)
        if (idx !== -1) list.splice(idx, 1)
      }
    }
  }

  onSessionEvent(
    event: string,
    handler: (params: unknown, sessionId: string) => void,
  ): () => void {
    if (!this.sessionEventHandlers.has(event)) {
      this.sessionEventHandlers.set(event, [])
    }
    const handlers = this.sessionEventHandlers.get(event)
    if (handlers) {
      handlers.push(handler)
    }
    return () => {
      const list = this.sessionEventHandlers.get(event)
      if (list) {
        const idx = list.indexOf(handler)
        if (idx !== -1) list.splice(idx, 1)
      }
    }
  }

  private handleMessage(data: string): void {
    const message = JSON.parse(data) as {
      id?: number
      method?: string
      params?: unknown
      result?: unknown
      error?: { message: string; code: number }
      sessionId?: string
    }

    // Route responses to pending requests
    if (message.id !== undefined) {
      const pending = this.pending.get(message.id)
      if (pending) {
        clearTimeout(pending.timer)
        this.pending.delete(message.id)
        if (message.error) {
          pending.reject(new Error(`CDP error: ${message.error.message}`))
        } else {
          pending.resolve(message.result)
        }
      }
    } else if (message.method) {
      // Dispatch to global event handlers
      const handlers = this.eventHandlers.get(message.method)
      if (handlers) {
        for (const handler of handlers) {
          handler(message.params)
        }
      }

      // Dispatch to session-aware handlers when sessionId is present
      if (message.sessionId) {
        const sessionHandlers = this.sessionEventHandlers.get(message.method)
        if (sessionHandlers) {
          for (const handler of sessionHandlers) {
            handler(message.params, message.sessionId)
          }
        }
      }
    }
  }
}

export { CdpBackend }
