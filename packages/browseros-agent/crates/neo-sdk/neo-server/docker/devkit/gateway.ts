/**
 * The dev kit's localhost gateway in front of claw-sdk's host API (dev only, never in an image).
 *
 * claw-sdk's host listener wants `Authorization: Bearer <token>` on every request and refuses a
 * token in the URL. A browser page cannot send that header on a WebSocket or an EventSource, so
 * the viewer (`/`) and the trace UI (`/trace`) talk to this gateway instead, and the gateway adds
 * the header on its way to the host listener:
 *
 *   browser ── http://127.0.0.1:<port> ──▶ gateway ── +Authorization ──▶ <upstream>/v1/...
 *
 * That turns a bearer-protected API into an unauthenticated one on localhost, so the guards in
 * `refusal()` run before any route. Run with Bun 1.4.2:
 *
 *   bun gateway.ts --port 16090 --upstream http://127.0.0.1:19300 --token-file <f> \
 *     --novnc-dir <d> --traces-dir <d>
 */

import { existsSync, readFileSync } from 'node:fs'
import { join, resolve, sep } from 'node:path'
import { parseArgs } from 'node:util'
import type { Server, ServerWebSocket } from 'bun'

type Config = {
  port: number
  upstream: URL
  token: string
  novncDir: string
  tracesDir: string
}

/** Reads the CLI. Every failure is one line on stderr and a non-zero exit, for dev.sh's log. */
function parseConfig(argv: string[]): Config {
  const fail = (message: string): never => {
    console.error(`gateway: ${message}`)
    process.exit(2)
  }
  let values: Record<string, string | undefined>
  try {
    values = parseArgs({
      args: argv,
      options: {
        port: { type: 'string', default: '16090' },
        upstream: { type: 'string', default: 'http://127.0.0.1:19300' },
        'token-file': { type: 'string' },
        'novnc-dir': { type: 'string' },
        'traces-dir': { type: 'string' },
      },
      strict: true,
    }).values
  } catch (error) {
    return fail((error as Error).message)
  }
  const port = Number(values.port)
  if (!Number.isInteger(port) || port < 0 || port > 65535)
    fail(`--port ${values.port} is not a port`)
  let upstream: URL
  try {
    upstream = new URL(values.upstream!)
  } catch {
    return fail(`--upstream ${values.upstream} is not a URL`)
  }
  if (upstream.protocol !== 'http:')
    fail(`--upstream must be http://host:port, got ${upstream.href}`)
  const tokenFile = values['token-file'] ?? fail('--token-file is required')
  let token = ''
  try {
    token = readFileSync(tokenFile, 'utf8').trim()
  } catch (error) {
    fail(
      `cannot read the host token file ${tokenFile}: ${(error as Error).message}`,
    )
  }
  if (!token) fail(`the host token file ${tokenFile} is empty`)
  const novncDir = resolve(
    values['novnc-dir'] ?? fail('--novnc-dir is required'),
  )
  if (!existsSync(join(novncDir, 'core', 'rfb.js'))) {
    fail(
      `--novnc-dir ${novncDir} has no core/rfb.js; copy noVNC out of the container first`,
    )
  }
  const tracesDir = resolve(
    values['traces-dir'] ?? fail('--traces-dir is required'),
  )
  return { port, upstream, token, novncDir, tracesDir }
}

const config = parseConfig(Bun.argv.slice(2))
const kitDir = import.meta.dir
const webDir = join(kitDir, 'web')

/** The gateway's own refusals use the contract's `Error` shape, so the pages show them like claw-sdk's. */
function errorResponse(
  status: number,
  code: string,
  message: string,
): Response {
  return Response.json({ code, message }, { status })
}

/**
 * Refuses what must never reach a route. The gateway holds the bearer token, so:
 * - `Host` must name this gateway (127.0.0.1 or localhost at its port). A DNS-rebinding page
 *   reaches 127.0.0.1 under its own host name, and that name is what it sends.
 * - `Origin`, when present, must be this gateway's own. Any other page in the owner's browser
 *   could otherwise open a control display (WebSockets skip CORS) or POST a lock. Browsers send
 *   Origin on every WebSocket and every POST; a same-origin GET may omit it, which is fine.
 * - No `token` or `access_token` query key. claw-sdk refuses them too; this fails early, plainly.
 */
const QUERY_TOKEN_KEYS = new Set(['token', 'access_token'])

function refusal(req: Request, url: URL): Response | null {
  const host = req.headers.get('host') ?? ''
  if (
    host !== `127.0.0.1:${config.port}` &&
    host !== `localhost:${config.port}`
  ) {
    return errorResponse(
      403,
      'forbidden_host',
      `gateway: Host ${JSON.stringify(host)} is not this gateway`,
    )
  }
  const origin = req.headers.get('origin')
  if (origin !== null && origin !== `http://${host}`) {
    return errorResponse(
      403,
      'forbidden_origin',
      `gateway: Origin ${JSON.stringify(origin)} is not http://${host}`,
    )
  }
  // claw-sdk matches these keys in any case; so does this, or a mixed-case key would reach
  // claw-sdk and the gateway's own refusal log.
  if (
    [...url.searchParams.keys()].some((key) =>
      QUERY_TOKEN_KEYS.has(key.toLowerCase()),
    )
  ) {
    return errorResponse(
      400,
      'token_in_query',
      'gateway: tokens are never accepted in the URL; the gateway adds the header',
    )
  }
  return null
}

/** Serves `rel` from inside `dir` only; a path that resolves outside it is a 404. */
async function serveFile(dir: string, rel: string): Promise<Response> {
  let path: string
  try {
    path = resolve(dir, `.${sep}${decodeURIComponent(rel)}`)
  } catch {
    return errorResponse(400, 'invalid_request', 'gateway: malformed path')
  }
  const file = Bun.file(path)
  if (!path.startsWith(dir + sep) || !(await file.exists())) {
    return errorResponse(404, 'not_found', `gateway: no file ${rel}`)
  }
  // No page may frame the kit: a framed viewer opens its display with the gateway's own Origin,
  // so the Origin guard cannot stop a foreign page from clickjacking control mode.
  return new Response(file, {
    headers: {
      'cache-control': 'no-store',
      'content-security-policy': "frame-ancestors 'none'",
      'x-frame-options': 'DENY',
    },
  })
}

/**
 * The CLI run log (design.md shape 2) as one JSON array. Lines are appended under flock by
 * `dev.sh run`, so a line that does not parse is one being written right now, or a damaged one:
 * either way it is skipped, never fatal.
 */
async function devRuns(): Promise<Response> {
  const file = Bun.file(join(config.tracesDir, 'runs.jsonl'))
  const records: unknown[] = []
  if (await file.exists()) {
    for (const line of (await file.text()).split('\n')) {
      if (!line.trim()) continue
      try {
        records.push(JSON.parse(line))
      } catch {
        // A partial or damaged line; see above.
      }
    }
  }
  return Response.json(records, { headers: { 'cache-control': 'no-store' } })
}

/** Request headers the pages may send on to claw-sdk. Cookies and the browser's Origin stay here. */
const FORWARDED_REQUEST_HEADERS = [
  'accept',
  'content-type',
  'last-event-id',
  'cache-control',
]
/** Hop-by-hop headers, plus the length fetch may have changed. */
const DROPPED_RESPONSE_HEADERS = new Set([
  'connection',
  'keep-alive',
  'transfer-encoding',
  'content-length',
  'content-encoding',
])

/**
 * Proxies `/v1/<rest>` to claw-sdk with the bearer header added; method, query, body, status and
 * headers pass through. claw-sdk holds some requests open on purpose: `/v1/events` streams for
 * as long as it is read (an SSE comment every 15 s), and `wait=` holds a run or takeover read for
 * up to 60 s. Bun.serve closes a connection after 10 s without bytes, so the idle timeout is off
 * for every proxied request, and the upstream body is handed to the browser as a stream, never
 * buffered.
 */
async function proxy(
  req: Request,
  url: URL,
  server: Server<DisplaySocket>,
): Promise<Response> {
  server.timeout(req, 0)
  const headers = new Headers({
    authorization: `Bearer ${config.token}`,
    // fetch would decompress a compressed body while its headers still said otherwise.
    'accept-encoding': 'identity',
  })
  for (const name of FORWARDED_REQUEST_HEADERS) {
    const value = req.headers.get(name)
    if (value !== null) headers.set(name, value)
  }
  const hasBody = req.method !== 'GET' && req.method !== 'HEAD'
  let upstream: Response
  try {
    upstream = await fetch(
      new URL(url.pathname + url.search, config.upstream),
      {
        method: req.method,
        headers,
        body: hasBody ? await req.arrayBuffer() : undefined,
        redirect: 'manual',
        // The browser leaving (an EventSource closed, a tab gone) ends the upstream request too.
        signal: req.signal,
      },
    )
  } catch (error) {
    return errorResponse(
      502,
      'upstream_unreachable',
      `gateway: cannot reach claw-sdk at ${config.upstream.origin}: ${(error as Error).message}`,
    )
  }
  const out = new Headers()
  for (const [name, value] of upstream.headers) {
    if (!DROPPED_RESPONSE_HEADERS.has(name)) out.set(name, value)
  }
  const eventStream =
    upstream.ok &&
    upstream.body &&
    out.get('content-type')?.startsWith('text/event-stream')
  const body = eventStream ? openedEventStream(upstream.body!) : upstream.body
  return new Response(body, {
    status: upstream.status,
    statusText: upstream.statusText,
    headers: out,
  })
}

/**
 * Bun.serve ends a streamed response's header block only with the body's first byte (measured
 * on 1.4.2, also with an explicit Transfer-Encoding or a direct stream's flush). claw-sdk sends
 * nothing on a quiet stream until its 15 s keep-alive, so an EventSource would sit in
 * "connecting" that long. An SSE comment first gets the headers out at once; EventSource ignores
 * comments, and they never move Last-Event-ID. Cancelling this stream (the browser left) cancels
 * the upstream body, which closes the upstream connection.
 */
function openedEventStream(
  upstream: ReadableStream<Uint8Array>,
): ReadableStream<Uint8Array> {
  const reader = upstream.getReader()
  return new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(': gateway connected\n\n'))
    },
    async pull(controller) {
      const { value, done } = await reader.read()
      if (done) controller.close()
      else controller.enqueue(value)
    },
    cancel(reason) {
      return reader.cancel(reason)
    },
  })
}

/**
 * One browser display socket and its upstream twin. Bytes from the browser that arrive before
 * the upstream opens wait in `pending` (noVNC speaks second in RFB, so this is normally empty).
 */
type DisplaySocket = {
  search: string
  upstream: WebSocket | null
  upstreamOpen: boolean
  pending: (string | Buffer)[]
  browserGone: boolean
}

/**
 * Close codes the gateway itself sends, besides claw-sdk's own (4001, 4002, 1008, 1003, 1011),
 * which it relays unchanged:
 * - `4000 + HTTP status` (4400, 4401, 4404, 4409, 4503, ...): claw-sdk refused the upgrade; the
 *   reason is `<status> <error code>: <message>`, e.g. `409 display_paused: ...`.
 * - `4502`: the gateway could not reach claw-sdk, or lost it without a close frame.
 */
const GATEWAY_UNREACHABLE = 4502

/** RFC 6455 caps a close reason at 123 bytes of UTF-8. */
function closeReason(text: string): string {
  const bytes = new TextEncoder().encode(text)
  if (bytes.length <= 123) return text
  return (
    new TextDecoder().decode(bytes.slice(0, 120)).replace(/\uFFFD$/, '') + '...'
  )
}

function closeBrowser(
  ws: ServerWebSocket<DisplaySocket>,
  code: number,
  reason: string,
): void {
  if (ws.data.browserGone) return
  ws.close(code, closeReason(reason))
}

function upstreamDisplayUrl(search: string, scheme: 'ws:' | 'http:'): string {
  const base = new URL(config.upstream)
  base.protocol = scheme
  return new URL(`/v1/display${search}`, base).href
}

/**
 * claw-sdk admits a display before the upgrade, so a refusal (409 display_paused, 404 or 409 for
 * a bad `release`, 400, 503) is an HTTP status, and Bun's WebSocket client reports every one as
 * close 1002 "Expected 101 status code", status and body discarded (measured on Bun 1.4.2). The
 * same handshake is replayed with fetch, which does return them, and the browser gets
 * `4000 + status`. If the replay is admitted (state changed in between), that display is dropped
 * at once and the browser is told to retry.
 */
async function explainRefusal(
  ws: ServerWebSocket<DisplaySocket>,
): Promise<void> {
  try {
    const reply = await fetch(upstreamDisplayUrl(ws.data.search, 'http:'), {
      headers: {
        authorization: `Bearer ${config.token}`,
        connection: 'Upgrade',
        upgrade: 'websocket',
        'sec-websocket-version': '13',
        'sec-websocket-key': Buffer.from(
          crypto.getRandomValues(new Uint8Array(16)),
        ).toString('base64'),
      },
      signal: AbortSignal.timeout(5000),
    })
    if (reply.status === 101) {
      await reply.body?.cancel()
      closeBrowser(
        ws,
        GATEWAY_UNREACHABLE,
        'claw-sdk refused the display, then admitted it on a second try; reconnect',
      )
      return
    }
    const body = (await reply.json().catch(() => null)) as {
      code?: string
      message?: string
    } | null
    const reason = `${reply.status} ${body?.code ?? reply.statusText}: ${body?.message ?? ''}`
    console.error(`gateway: display${ws.data.search} refused: ${reason}`)
    closeBrowser(ws, 4000 + reply.status, reason)
  } catch (error) {
    closeBrowser(
      ws,
      GATEWAY_UNREACHABLE,
      `gateway: cannot reach claw-sdk at ${config.upstream.origin}: ${(error as Error).message}`,
    )
  }
}

/** Opens the upstream display for a browser socket that just opened, and pipes the two together. */
function bridgeDisplay(ws: ServerWebSocket<DisplaySocket>): void {
  const upstream = new WebSocket(upstreamDisplayUrl(ws.data.search, 'ws:'), {
    headers: { authorization: `Bearer ${config.token}` },
  })
  upstream.binaryType = 'arraybuffer'
  ws.data.upstream = upstream
  upstream.addEventListener('open', () => {
    ws.data.upstreamOpen = true
    for (const message of ws.data.pending.splice(0)) upstream.send(message)
  })
  upstream.addEventListener('message', (event) => {
    if (!ws.data.browserGone) ws.send(event.data as ArrayBuffer)
  })
  upstream.addEventListener('close', (event) => {
    if (ws.data.browserGone) return
    if (!ws.data.upstreamOpen) {
      void explainRefusal(ws)
      return
    }
    // 1005 (no code) and 1006 (no close frame) are reserved: they describe a close and can
    // never be sent in one.
    if (event.code === 1005) closeBrowser(ws, 1000, event.reason)
    else if (event.code === 1006)
      closeBrowser(
        ws,
        GATEWAY_UNREACHABLE,
        'gateway: the connection to claw-sdk dropped without a close frame',
      )
    else closeBrowser(ws, event.code, event.reason)
  })
}

type Route = {
  method: 'GET' | 'ANY'
  path: string
  /** Matches `path` and everything under it; the handler gets the rest. */
  prefix?: true
  handle: (
    req: Request,
    url: URL,
    server: Server<DisplaySocket>,
    rest: string,
  ) => Response | Promise<Response> | undefined
}

/** First match wins, so `/v1/display` sits above the `/v1/` catch-all. */
const ROUTES: Route[] = [
  { method: 'GET', path: '/', handle: () => serveFile(kitDir, 'viewer.html') },
  {
    method: 'GET',
    path: '/trace',
    handle: () => serveFile(kitDir, 'trace.html'),
  },
  {
    method: 'GET',
    path: '/kit/',
    prefix: true,
    handle: (_req, _url, _server, rest) => serveFile(webDir, rest),
  },
  {
    method: 'GET',
    path: '/novnc/',
    prefix: true,
    handle: (_req, _url, _server, rest) => serveFile(config.novncDir, rest),
  },
  { method: 'GET', path: '/dev/runs', handle: () => devRuns() },
  {
    method: 'GET',
    path: '/v1/display',
    handle: (req, url, server) => {
      const data: DisplaySocket = {
        search: url.search,
        upstream: null,
        upstreamOpen: false,
        pending: [],
        browserGone: false,
      }
      // A plain GET (no upgrade) goes through as any other request and gets claw-sdk's answer.
      if (server.upgrade(req, { data })) return undefined
      return proxy(req, url, server)
    },
  },
  {
    method: 'ANY',
    path: '/v1/',
    prefix: true,
    handle: (req, url, server) => proxy(req, url, server),
  },
]

function route(
  req: Request,
  url: URL,
  server: Server<DisplaySocket>,
): Response | Promise<Response> | undefined {
  for (const r of ROUTES) {
    if (r.method !== 'ANY' && r.method !== req.method) continue
    if (r.prefix ? url.pathname.startsWith(r.path) : url.pathname === r.path) {
      return r.handle(req, url, server, url.pathname.slice(r.path.length))
    }
  }
  return errorResponse(
    404,
    'not_found',
    `gateway: no route for ${req.method} ${url.pathname}`,
  )
}

let server: Server<DisplaySocket>
try {
  server = Bun.serve({
    hostname: '127.0.0.1',
    port: config.port,
    fetch(req, server) {
      const url = new URL(req.url)
      return refusal(req, url) ?? route(req, url, server)
    },
    websocket: {
      data: {} as DisplaySocket,
      // An RFB stream that drops a message is corrupt from then on, so a browser that cannot
      // keep up is closed rather than skipped. An idle display (a still screen) stays open:
      // Bun pings it, and the browser's pongs count as traffic.
      closeOnBackpressureLimit: true,
      backpressureLimit: 64 * 1024 * 1024,
      open: bridgeDisplay,
      message(ws, message) {
        // Text frames go on as text, so claw-sdk answers them as the contract says (1003).
        if (ws.data.upstreamOpen) ws.data.upstream!.send(message)
        else ws.data.pending.push(message)
      },
      close(ws, code) {
        ws.data.browserGone = true
        // A browser may only send 1000 or 3000-4999; the upstream reads any of them as "the viewer left".
        ws.data.upstream?.close(
          code === 1000 || (code >= 3000 && code <= 4999) ? code : 1000,
        )
      },
    },
  })
} catch (error) {
  console.error(
    `gateway: cannot listen on 127.0.0.1:${config.port}: ${(error as Error).message}`,
  )
  process.exit(1)
}

// dev.sh waits for this exact line.
console.log(`gateway: http://127.0.0.1:${server.port}`)
