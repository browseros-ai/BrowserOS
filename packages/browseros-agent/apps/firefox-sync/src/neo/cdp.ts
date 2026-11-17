/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * A one-shot CDP connection to the browser target. The sync runs a few
 * hundred request/response calls and exits, so this skips the reconnect,
 * keepalive and event plumbing of `@browseros/browser-core`'s long-lived
 * backend. Page work goes through flattened sessions (`sessionId` on each
 * message) on the same socket.
 */

const CALL_TIMEOUT_MS = 30_000

interface Pending {
  method: string
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

export class CdpError extends Error {
  constructor(
    public readonly method: string,
    message: string,
  ) {
    super(`${method}: ${message}`)
  }
}

export class CdpConnection {
  private nextId = 0
  private readonly pending = new Map<number, Pending>()

  private constructor(private readonly ws: WebSocket) {
    ws.addEventListener('message', (event) => this.onMessage(event))
    ws.addEventListener('close', () => {
      for (const [id, call] of this.pending) {
        clearTimeout(call.timer)
        call.reject(new Error('CDP connection closed'))
        this.pending.delete(id)
      }
    })
  }

  static open(wsUrl: string): Promise<CdpConnection> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(wsUrl)
      ws.addEventListener('open', () => resolve(new CdpConnection(ws)), {
        once: true,
      })
      ws.addEventListener(
        'error',
        () => reject(new Error(`Cannot connect to ${wsUrl}`)),
        { once: true },
      )
    })
  }

  send<T = Record<string, unknown>>(
    method: string,
    params: Record<string, unknown> = {},
    sessionId?: string,
  ): Promise<T> {
    const id = ++this.nextId
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new CdpError(method, 'timed out'))
      }, CALL_TIMEOUT_MS)
      this.pending.set(id, {
        method,
        resolve: resolve as (value: unknown) => void,
        reject,
        timer,
      })
      this.ws.send(JSON.stringify({ id, method, params, sessionId }))
    })
  }

  close(): void {
    this.ws.close()
  }

  private onMessage(event: MessageEvent): void {
    const message = JSON.parse(String(event.data)) as {
      id?: number
      result?: unknown
      error?: { message: string }
    }
    if (message.id === undefined) return
    const call = this.pending.get(message.id)
    if (!call) return
    this.pending.delete(message.id)
    clearTimeout(call.timer)
    if (message.error) {
      call.reject(new CdpError(call.method, message.error.message))
    } else {
      call.resolve(message.result)
    }
  }
}
