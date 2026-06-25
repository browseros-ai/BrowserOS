/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wire-level client for the Cloudflare agent-control-worker. Mirrors the
 * shape of KlavisClient — encapsulates fetch + auth, no application logic.
 * Owned by `RemoteHermesService` which combines this client with the
 * WS bridge and the AI SDK stream wiring.
 */

import {
  REMOTE_HERMES_AGENT_KIND,
  REMOTE_HERMES_DEFAULT_AGENT_ID,
} from '@browseros/shared/constants/hermes'
import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'
import { mintLaptopJwt } from './auth'
import { WS_SUBPROTOCOL } from './constants'

export interface VmStatusView {
  status:
    | 'cold'
    | 'starting'
    | 'running'
    | 'stopping'
    | 'stopped'
    | 'destroying'
    | 'error'
  progress?: string
  lastError?: { code: string; message: string } | null
  desiredImageTag?: string
  currentImageTag?: string | null
  flyMachineId?: string | null
}

export interface PostTurnInput {
  conversationId: string
  message: string
  modelId?: string | null
}

// fallow-ignore-next-line unused-type
export interface PostTurnResult {
  taskId: string
}

export interface RemoteHermesClientOptions {
  browserosId: string
  jwtSecret: string
  baseUrl?: string
}

export class RemoteHermesClient {
  /** Stable per-install identifier used for worker auth and local MCP attribution. */
  readonly browserosId: string
  private readonly jwtSecret: string
  private readonly baseUrl: string

  constructor(opts: RemoteHermesClientOptions) {
    this.browserosId = opts.browserosId
    this.jwtSecret = opts.jwtSecret
    this.baseUrl = opts.baseUrl ?? EXTERNAL_URLS.AGENT_CONTROL_WORKER
  }

  /** WebSocket URL derived from the HTTP base. */
  get wsUrl(): string {
    return `${this.baseUrl.replace(/^http/, 'ws').replace(/\/$/, '')}/v1/laptop/ws`
  }

  /** partysocket protocol callback. Returns [name, freshJwt] each call. */
  async wsSubprotocol(): Promise<[string, string]> {
    return [WS_SUBPROTOCOL, await this.mintJwt()]
  }

  async startVm(signal?: AbortSignal): Promise<Response> {
    return this.authedFetch('/v1/laptop/vm/start', { method: 'POST', signal })
  }

  async destroyVm(signal?: AbortSignal): Promise<Response> {
    return this.authedFetch('/v1/laptop/vm/destroy', { method: 'POST', signal })
  }

  async getVmStatus(signal?: AbortSignal): Promise<VmStatusView> {
    const res = await this.authedFetch('/v1/laptop/vm/status', { signal })
    if (!res.ok) throw new Error(`/vm/status returned ${res.status}`)
    return (await res.json()) as VmStatusView
  }

  /**
   * Returns the raw response so the caller can branch on `status` (the
   * 503 cold-VM path needs the status code, not the body). Body is JSON
   * `{taskId}` when status is 2xx.
   */
  async postTurn(input: PostTurnInput, signal: AbortSignal): Promise<Response> {
    const path = `/v1/laptop/threads/${encodeURIComponent(input.conversationId)}/turn`
    return this.authedFetch(path, {
      method: 'POST',
      signal,
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        message: input.message,
        agentId: REMOTE_HERMES_DEFAULT_AGENT_ID,
        agentKind: REMOTE_HERMES_AGENT_KIND,
        model: input.modelId ?? null,
      }),
    })
  }

  /** Returns the live SSE response; caller pumps the body stream. */
  async openTaskEvents(taskId: string, signal: AbortSignal): Promise<Response> {
    const path = `/v1/laptop/tasks/${encodeURIComponent(taskId)}/events`
    return this.authedFetch(path, {
      signal,
      headers: { accept: 'text/event-stream' },
    })
  }

  /** Fire-and-forget; we don't wait on the worker to confirm cancellation. */
  async abortTask(taskId: string): Promise<void> {
    await this.authedFetch(
      `/v1/laptop/tasks/${encodeURIComponent(taskId)}/abort`,
      { method: 'POST' },
    )
  }

  private async authedFetch(
    path: string,
    init: RequestInit = {},
  ): Promise<Response> {
    const jwt = await this.mintJwt()
    return fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        ...(init.headers ?? {}),
        authorization: `Bearer ${jwt}`,
      },
    })
  }

  private mintJwt(): Promise<string> {
    return mintLaptopJwt({
      browserosId: this.browserosId,
      secret: this.jwtSecret,
    })
  }
}
