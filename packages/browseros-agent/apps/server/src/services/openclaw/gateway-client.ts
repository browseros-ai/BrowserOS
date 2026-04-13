/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * WebSocket client for the OpenClaw Gateway protocol.
 * Handles handshake (challenge → connect → hello-ok) with Ed25519 device
 * identity signing, JSON-RPC over WS, and auto-reconnect.
 * Used for agent CRUD and health — chat uses HTTP.
 */

import crypto from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { logger } from '../../lib/logger'

const RPC_TIMEOUT_MS = 15_000
const RECONNECT_DELAY_MS = 2_000
const MAX_RECONNECT_RETRIES = 5
const CONTAINER_HOME = '/home/node/.openclaw'

const SCOPES = [
  'operator.read',
  'operator.write',
  'operator.admin',
  'operator.approvals',
  'operator.pairing',
]

interface DeviceIdentity {
  deviceId: string
  publicKeyPem: string
  privateKeyPem: string
}

interface PendingRequest {
  resolve: (value: unknown) => void
  reject: (reason: Error) => void
  timer: ReturnType<typeof setTimeout>
}

interface WsFrame {
  type: 'req' | 'res' | 'event'
  id?: string
  method?: string
  params?: Record<string, unknown>
  ok?: boolean
  payload?: Record<string, unknown>
  error?: { message: string; code?: string }
  event?: string
}

export interface OpenClawStreamEvent {
  type:
    | 'text-delta'
    | 'thinking'
    | 'tool-start'
    | 'tool-end'
    | 'tool-output'
    | 'lifecycle'
    | 'done'
    | 'error'
  data: Record<string, unknown>
}

export interface GatewayAgentEntry {
  agentId: string
  name: string
  workspace: string
  model?: string
}

// ── Device Identity Helpers ─────────────────────────────────────────

function rawPublicKeyFromPem(pem: string): Buffer {
  const der = Buffer.from(
    pem.replace(/-----[^-]+-----/g, '').replace(/\s/g, ''),
    'base64',
  )
  return der.subarray(12)
}

function signChallenge(
  device: DeviceIdentity,
  nonce: string,
  token: string,
): { signature: string; signedAt: number; publicKey: string } {
  const signedAt = Date.now()
  const payload = `v3|${device.deviceId}|cli|cli|operator|${SCOPES.join(',')}|${signedAt}|${token}|${nonce}|${process.platform}|`
  const privateKey = crypto.createPrivateKey(device.privateKeyPem)
  const sig = crypto.sign(null, Buffer.from(payload, 'utf-8'), privateKey)

  return {
    signature: sig.toString('base64url'),
    signedAt,
    publicKey: rawPublicKeyFromPem(device.publicKeyPem).toString('base64url'),
  }
}

/**
 * Generates a client Ed25519 identity and pre-seeds it into the gateway's
 * paired devices file so the gateway trusts it on next boot.
 * Must be called before compose up (or requires a restart after).
 */
export function ensureClientIdentity(openclawDir: string): DeviceIdentity {
  const identityPath = join(openclawDir, 'client-identity.json')

  try {
    return JSON.parse(readFileSync(identityPath, 'utf-8'))
  } catch {
    // Generate new identity
  }

  const { publicKey, privateKey } = crypto.generateKeyPairSync('ed25519')
  const publicKeyPem = publicKey
    .export({ type: 'spki', format: 'pem' })
    .toString()
  const privateKeyPem = privateKey
    .export({ type: 'pkcs8', format: 'pem' })
    .toString()

  const rawPub = rawPublicKeyFromPem(publicKeyPem)
  const deviceId = crypto.createHash('sha256').update(rawPub).digest('hex')

  const identity: DeviceIdentity = { deviceId, publicKeyPem, privateKeyPem }
  writeFileSync(identityPath, JSON.stringify(identity, null, 2), {
    mode: 0o600,
  })

  seedPairedDevice(openclawDir, identity)
  logger.info('Generated client device identity and pre-seeded pairing')

  return identity
}

function seedPairedDevice(openclawDir: string, identity: DeviceIdentity): void {
  const devicesDir = join(openclawDir, 'devices')
  mkdirSync(devicesDir, { recursive: true })

  const pairedPath = join(devicesDir, 'paired.json')
  let paired: Record<string, unknown> = {}
  try {
    paired = JSON.parse(readFileSync(pairedPath, 'utf-8'))
  } catch {
    // First time
  }

  const rawPub = rawPublicKeyFromPem(identity.publicKeyPem)
  paired[identity.deviceId] = {
    deviceId: identity.deviceId,
    publicKey: rawPub.toString('base64url'),
    platform: process.platform,
    clientId: 'cli',
    clientMode: 'cli',
    role: 'operator',
    roles: ['operator'],
    scopes: SCOPES,
    pairedAt: Date.now(),
    label: 'browseros-server',
  }

  writeFileSync(pairedPath, JSON.stringify(paired, null, 2), { mode: 0o600 })
}

// ── Gateway Client ──────────────────────────────────────────────────

export class GatewayClient {
  private ws: WebSocket | null = null
  private _connected = false
  private pendingRequests = new Map<string, PendingRequest>()
  private reconnectAttempts = 0
  private shouldReconnect = true
  private version: string
  private device: DeviceIdentity | null = null

  constructor(
    private port: number,
    private token: string,
    openclawDir: string,
    version = '1.0.0',
  ) {
    this.version = version
    try {
      const identityPath = join(openclawDir, 'client-identity.json')
      this.device = JSON.parse(readFileSync(identityPath, 'utf-8'))
    } catch {
      logger.warn('Client device identity not found, WS auth may fail')
    }
  }

  get isConnected(): boolean {
    return this._connected
  }

  async connect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const url = `ws://127.0.0.1:${this.port}`
      this.ws = new WebSocket(url, {
        headers: { Origin: `http://127.0.0.1:${this.port}` },
      } as unknown as string[])

      let handshakeComplete = false
      let connectReqId: string | null = null

      this.ws.onmessage = (event) => {
        let frame: WsFrame
        try {
          frame = JSON.parse(
            typeof event.data === 'string'
              ? event.data
              : new TextDecoder().decode(event.data as ArrayBuffer),
          )
        } catch {
          return
        }

        if (!handshakeComplete) {
          if (frame.type === 'event' && frame.event === 'connect.challenge') {
            const nonce = (frame.payload as Record<string, unknown>)
              ?.nonce as string
            connectReqId = globalThis.crypto.randomUUID()

            const params: Record<string, unknown> = {
              minProtocol: 3,
              maxProtocol: 3,
              client: {
                id: 'cli',
                version: this.version,
                platform: process.platform,
                mode: 'cli',
              },
              role: 'operator',
              scopes: SCOPES,
              caps: [],
              commands: [],
              permissions: {},
              auth: { token: this.token },
              locale: 'en-US',
              userAgent: `browseros-server/${this.version}`,
            }

            if (this.device && nonce) {
              const signed = signChallenge(this.device, nonce, this.token)
              params.device = {
                id: this.device.deviceId,
                publicKey: signed.publicKey,
                signature: signed.signature,
                signedAt: signed.signedAt,
                nonce,
              }
            }

            this.ws?.send(
              JSON.stringify({
                type: 'req',
                id: connectReqId,
                method: 'connect',
                params,
              }),
            )
            return
          }

          if (frame.type === 'res' && frame.id === connectReqId) {
            if (frame.ok) {
              handshakeComplete = true
              this._connected = true
              this.reconnectAttempts = 0
              logger.info('Gateway WS connected')
              resolve()
            } else {
              const msg = frame.error?.message ?? 'Handshake failed'
              logger.error('Gateway WS handshake rejected', {
                error: msg,
                code: frame.error?.code,
              })
              reject(new Error(msg))
            }
            return
          }
          return
        }

        if (frame.type === 'res' && frame.id) {
          const pending = this.pendingRequests.get(frame.id)
          if (pending) {
            this.pendingRequests.delete(frame.id)
            clearTimeout(pending.timer)
            if (frame.ok) {
              pending.resolve(frame.payload)
            } else {
              pending.reject(new Error(frame.error?.message ?? 'RPC error'))
            }
          }
        }
      }

      this.ws.onerror = (err) => {
        if (!handshakeComplete) {
          reject(
            new Error(
              `WS connection error: ${err instanceof Error ? err.message : 'unknown'}`,
            ),
          )
        }
      }

      this.ws.onclose = () => {
        this._connected = false
        this.rejectAllPending('WebSocket closed')
        if (handshakeComplete) {
          logger.info('Gateway WS disconnected')
          this.tryReconnect()
        }
      }
    })
  }

  disconnect(): void {
    this.shouldReconnect = false
    this._connected = false
    this.rejectAllPending('Client disconnecting')
    if (this.ws) {
      this.ws.onclose = null
      this.ws.close()
      this.ws = null
    }
  }

  // ── RPC ──────────────────────────────────────────────────────────────

  async rpc<T = Record<string, unknown>>(
    method: string,
    params: Record<string, unknown> = {},
  ): Promise<T> {
    if (!this._connected || !this.ws) {
      throw new Error('Gateway WS not connected')
    }

    const id = globalThis.crypto.randomUUID()

    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pendingRequests.delete(id)
        reject(new Error(`RPC timeout: ${method}`))
      }, RPC_TIMEOUT_MS)

      this.pendingRequests.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      })

      this.ws?.send(JSON.stringify({ type: 'req', id, method, params }))
    })
  }

  // ── Agent Methods ────────────────────────────────────────────────────

  async listAgents(): Promise<GatewayAgentEntry[]> {
    const result = await this.rpc<{
      agents: Array<{
        id: string
        name?: string
        workspace: string
        model?: string
      }>
    }>('agents.list')

    return (result.agents ?? []).map((a) => ({
      agentId: a.id,
      name: a.name ?? a.id,
      workspace: a.workspace,
      model: a.model,
    }))
  }

  async createAgent(input: {
    name: string
    workspace: string
    model?: string
  }): Promise<GatewayAgentEntry> {
    const result = await this.rpc<{
      agentId?: string
      id?: string
      name?: string
      workspace?: string
      model?: string
    }>('agents.create', input)

    return {
      agentId: result.agentId ?? result.id ?? input.name,
      name: result.name ?? input.name,
      workspace: result.workspace ?? input.workspace,
      model: result.model ?? input.model,
    }
  }

  async deleteAgent(agentId: string): Promise<void> {
    await this.rpc('agents.delete', { id: agentId })
  }

  // ── Health ───────────────────────────────────────────────────────────

  async getHealth(): Promise<Record<string, unknown>> {
    return this.rpc('health')
  }

  // ── Chat Stream ─────────────────────────────────────────────────────

  chatStream(
    agentId: string,
    sessionKey: string,
    message: string,
  ): ReadableStream<OpenClawStreamEvent> {
    if (!this._connected || !this.ws) {
      throw new Error('Gateway WS not connected')
    }

    const ws = this.ws
    const pendingRequests = this.pendingRequests
    const fullSessionKey = `agent:${agentId}:browseros-${sessionKey}`
    const idempotencyKey = globalThis.crypto.randomUUID()
    const originalOnMessage = ws.onmessage

    const restore = () => {
      ws.onmessage = originalOnMessage
    }

    return new ReadableStream<OpenClawStreamEvent>({
      start: (controller) => {
        const subscribeId = globalThis.crypto.randomUUID()
        const agentReqId = globalThis.crypto.randomUUID()

        ws.onmessage = (event) => {
          let frame: WsFrame
          try {
            frame = JSON.parse(
              typeof event.data === 'string'
                ? event.data
                : new TextDecoder().decode(event.data as ArrayBuffer),
            )
          } catch {
            return
          }

          if (frame.type === 'res' && frame.id) {
            if (frame.id === subscribeId || frame.id === agentReqId) {
              if (!frame.ok) {
                controller.enqueue({
                  type: 'error',
                  data: {
                    message: frame.error?.message ?? 'RPC error',
                    code: frame.error?.code,
                  },
                })
                controller.close()
                restore()
              }
              return
            }

            const pending = pendingRequests.get(frame.id)
            if (pending) {
              pendingRequests.delete(frame.id)
              clearTimeout(pending.timer)
              if (frame.ok) {
                pending.resolve(frame.payload)
              } else {
                pending.reject(new Error(frame.error?.message ?? 'RPC error'))
              }
            }
            return
          }

          const anyFrame = frame as any
          const eventName = anyFrame.event as string | undefined
          const payload = anyFrame.payload as
            | Record<string, unknown>
            | undefined

          if (!eventName || !payload) return

          if (eventName === 'agent') {
            const streamType = payload.stream as string | undefined
            const data = payload.data as Record<string, unknown> | undefined

            if (streamType === 'assistant' && data?.delta) {
              controller.enqueue({
                type: 'text-delta',
                data: { text: data.delta },
              })
              return
            }

            if (streamType === 'item' && data) {
              const phase = data.phase as string | undefined
              if (phase === 'start') {
                controller.enqueue({
                  type: 'tool-start',
                  data: {
                    toolCallId: data.toolCallId ?? data.id,
                    toolName: data.name ?? data.title,
                    kind: data.kind,
                  },
                })
                return
              }
              if (phase === 'end') {
                controller.enqueue({
                  type: 'tool-end',
                  data: {
                    toolCallId: data.toolCallId ?? data.id,
                    status: data.status,
                    durationMs: data.durationMs,
                  },
                })
                return
              }
            }

            if (streamType === 'lifecycle') {
              controller.enqueue({
                type: 'lifecycle',
                data: { phase: data?.phase ?? payload.phase },
              })
              return
            }
          }

          if (eventName === 'session.tool') {
            const toolData =
              (payload.data as Record<string, unknown>) ?? payload
            const phase =
              (toolData.phase as string) ?? (payload.phase as string)
            if (phase === 'result') {
              controller.enqueue({
                type: 'tool-output',
                data: {
                  toolCallId: toolData.toolCallId,
                  isError: toolData.isError ?? false,
                  meta: toolData.meta,
                },
              })
              return
            }
          }

          if (eventName === 'session.message') {
            const msg = payload.message as Record<string, unknown> | undefined
            if (msg?.role === 'assistant') {
              const content = msg.content as
                | Array<Record<string, unknown>>
                | undefined
              if (content) {
                for (const block of content) {
                  if (block.type === 'thinking') {
                    const text =
                      (block.thinking as string) ??
                      (block.content as string) ??
                      (block.text as string) ??
                      ''
                    if (text) {
                      controller.enqueue({
                        type: 'thinking',
                        data: { text },
                      })
                    }
                  }
                }
              }
            }
          }

          if (eventName === 'chat') {
            const state = payload.state as string | undefined
            if (state === 'final') {
              controller.enqueue({
                type: 'done',
                data: { text: (payload.text as string) ?? '' },
              })
              controller.close()
              restore()
              return
            }
          }
        }

        ws.send(
          JSON.stringify({
            type: 'req',
            id: subscribeId,
            method: 'sessions.subscribe',
            params: { sessionKey: fullSessionKey },
          }),
        )

        ws.send(
          JSON.stringify({
            type: 'req',
            id: agentReqId,
            method: 'agent',
            params: {
              message,
              sessionKey: fullSessionKey,
              idempotencyKey,
            },
          }),
        )
      },
      cancel: () => {
        restore()
      },
    })
  }

  // ── Helpers ──────────────────────────────────────────────────────────

  static agentWorkspace(name: string): string {
    return name === 'main'
      ? `${CONTAINER_HOME}/workspace`
      : `${CONTAINER_HOME}/workspace-${name}`
  }

  private tryReconnect(): void {
    if (!this.shouldReconnect) return
    if (this.reconnectAttempts >= MAX_RECONNECT_RETRIES) {
      logger.warn('Gateway WS max reconnect attempts reached')
      return
    }

    this.reconnectAttempts++
    logger.info('Gateway WS reconnecting...', {
      attempt: this.reconnectAttempts,
    })

    setTimeout(() => {
      this.connect().catch((err) => {
        logger.warn('Gateway WS reconnect failed', {
          error: err instanceof Error ? err.message : String(err),
          attempt: this.reconnectAttempts,
        })
      })
    }, RECONNECT_DELAY_MS)
  }

  private rejectAllPending(reason: string): void {
    for (const [id, pending] of this.pendingRequests) {
      clearTimeout(pending.timer)
      pending.reject(new Error(reason))
      this.pendingRequests.delete(id)
    }
  }
}
