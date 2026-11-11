/**
 * A stand-in for Neo's host API in tests: the display WebSocket (`/v1/display`) and the JSON
 * routes the live screen calls (`/v1/tabs`, `/v1/trace`, `/v1/takeovers…`). It checks the bearer
 * token like Neo, records what reached it, and lets a test script each answer.
 */

import type { ServerWebSocket } from 'bun'
import type { LiveHost } from '../live-screen/live-host'

export const FAKE_TOKEN = 'fake-host-token-0123456789abcdef0123456789'

export interface HostRequest {
  method: string
  /** Path and query, as sent. */
  path: string
  authorization: string | null
  body: string
}

/** One display socket Neo accepted. */
export interface FakeDisplay {
  /** Path and query of the upgrade, as sent. */
  path: string
  authorization: string | null
  /** Frames the viewer sent, in order. */
  received: Array<string | Uint8Array>
  /** How the viewer closed it, once it did. */
  closedWith: { code: number; reason: string } | null
  socket: ServerWebSocket<FakeDisplay>
}

export interface FakeNeoHost {
  /** For `startViewer({…}, { liveHost })`. */
  host: LiveHost
  /** Its base URL, for `startViewer({ hostApi: { url, tokenFile } })`. */
  url: string
  requests: HostRequest[]
  displays: FakeDisplay[]
  /**
   * Answers a request instead of the default: return a Response, or undefined for the default.
   * For `/v1/display` a Response refuses the upgrade (Neo's 4xx/503 before upgrading).
   */
  respond: (request: HostRequest) => Response | undefined
  /** Called when a display opens; the default sends Neo's RFB version greeting. */
  onDisplay: (display: FakeDisplay) => void
  /** Resolves once a display has been opened and then closed by the viewer. */
  nextDisplayClose(): Promise<FakeDisplay>
  stop(): Promise<void>
}

export function startFakeNeoHost(): FakeNeoHost {
  const requests: HostRequest[] = []
  const displays: FakeDisplay[] = []
  const closeWaiters: Array<(display: FakeDisplay) => void> = []
  const fake: FakeNeoHost = {
    host: null as unknown as LiveHost,
    url: '',
    requests,
    displays,
    respond: () => undefined,
    onDisplay: (display) =>
      display.socket.sendBinary(new TextEncoder().encode('RFB 003.008\n')),
    nextDisplayClose: () =>
      new Promise((resolve) => {
        closeWaiters.push(resolve)
      }),
    stop: async () => {
      // Bun before 1.4.2 never settles `stop` after the server closed a WebSocket itself.
      await Promise.race([server.stop(true), Bun.sleep(500)])
    },
  }

  const server = Bun.serve<FakeDisplay>({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(req, server) {
      const url = new URL(req.url)
      const request: HostRequest = {
        method: req.method,
        path: `${url.pathname}${url.search}`,
        authorization: req.headers.get('authorization'),
        body: await req.text(),
      }
      requests.push(request)
      if (request.authorization !== `Bearer ${FAKE_TOKEN}`)
        return Response.json(
          { code: 'unauthorized', message: 'missing bearer token' },
          { status: 401 },
        )
      const scripted = fake.respond(request)
      if (scripted) return scripted
      if (url.pathname === '/v1/display') {
        const display = {
          path: request.path,
          authorization: request.authorization,
          received: [],
          closedWith: null,
        } as unknown as FakeDisplay
        if (server.upgrade(req, { data: display })) return undefined
        // Neo answers a plain GET of a good query with axum's upgrade rejection.
        return Response.json(
          { code: 'invalid_request', message: 'Bad Request' },
          { status: 400 },
        )
      }
      return defaultAnswer(request)
    },
    websocket: {
      open(ws) {
        ws.data.socket = ws
        displays.push(ws.data)
        fake.onDisplay(ws.data)
      },
      message(ws, message) {
        ws.data.received.push(
          typeof message === 'string' ? message : new Uint8Array(message),
        )
      },
      close(ws, code, reason) {
        ws.data.closedWith = { code, reason }
        closeWaiters.shift()?.(ws.data)
      },
    },
  })

  const base = `http://127.0.0.1:${server.port}`
  fake.url = base
  fake.host = {
    fetch: (path, init = {}) => {
      const headers = new Headers(init.headers)
      headers.set('authorization', `Bearer ${FAKE_TOKEN}`)
      return fetch(`${base}${path}`, { ...init, headers })
    },
    socket: (path) => ({
      url: `${base.replace('http', 'ws')}${path}`,
      headers: { Authorization: `Bearer ${FAKE_TOKEN}` },
    }),
  }
  return fake
}

/** Neo's answers when a test scripts none: no tabs, an empty trace. */
function defaultAnswer(request: HostRequest): Response {
  const path = request.path.split('?')[0]
  if (path === '/v1/tabs') return Response.json({ items: [] })
  if (path === '/v1/trace')
    return Response.json({ runs: [], operations: [], truncated: false })
  return Response.json(
    { code: 'not_found', message: 'nothing has this id' },
    { status: 404 },
  )
}

/** A `LiveHost` whose Neo is not there: nothing listens on the port. */
export function unreachableHost(): LiveHost {
  const base = 'http://127.0.0.1:9'
  return {
    fetch: (path, init) => fetch(`${base}${path}`, init),
    socket: (path) => ({
      url: `ws://127.0.0.1:9${path}`,
      headers: { Authorization: `Bearer ${FAKE_TOKEN}` },
    }),
  }
}
