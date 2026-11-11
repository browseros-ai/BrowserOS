/**
 * The viewer's client for Neo's host API (`/v1`: runs, operations, tabs, the display,
 * takeovers; `crates/neo-sdk/neo-sdk-api/openapi.yaml`). Every viewer route that needs Neo's
 * records or screen goes through these functions, so the token lives only here: the page never
 * sees it.
 *
 * Neo serves the host API on a Unix socket, which macOS cannot reach inside Docker Desktop's VM,
 * so the viewer uses the image's development option `CLAW_HOST_TCP` (token only). Measured on
 * neo-sdk:0.52.1.2-f852e2692: inside the container it must bind `0.0.0.0:<port>`; bound to
 * `127.0.0.1` the published port answers nothing, since Docker's port forward arrives from
 * outside the container's loopback. Publish it on the host's 127.0.0.1 only, on the image's own
 * Docker network.
 *
 * One host API at a time: `bun run neo:viewer` configures it with the container it starts (a new
 * token each start); with `--upstream`, `--host-token-file` (and `--host-api`) name one.
 */

import { readFile } from 'node:fs/promises'

/** Where the host API listens and the file holding its bearer token (`CLAW_HOST_TOKEN_FILE`). */
export interface HostApiConfig {
  url: URL
  tokenFile: string
}

interface HostApiTarget {
  url: URL
  token: string
}

let target: HostApiTarget | null = null

/**
 * Points the client at a host API, reading its token now; null turns it off. The token must be
 * what Neo accepts: 32 or more visible ASCII bytes.
 */
export async function configureHostApi(
  config: HostApiConfig | null,
): Promise<void> {
  if (!config) {
    target = null
    return
  }
  const token = (await readFile(config.tokenFile, 'utf8')).trim()
  if (!/^[\x21-\x7e]{32,}$/.test(token)) {
    throw new Error(
      `neo-viewer: ${config.tokenFile} must hold 32 or more visible ASCII characters (CLAW_HOST_TOKEN_FILE)`,
    )
  }
  target = { url: new URL(config.url.href), token }
}

/**
 * A host API request: `path` is the route and query (`/v1/runs/…`), `init` as for `fetch`. The
 * answer is Neo's own `Response`, for `.json()` or `.arrayBuffer()`. When there is no host API
 * the viewer answers in Neo's error shape, `{ code, message }`: `503 no_host_api` when none is
 * configured, `502 host_api_unreachable` when nothing answers (the image is stopped or still
 * starting).
 */
export async function hostFetch(
  path: string,
  init: RequestInit = {},
): Promise<Response> {
  const current = target
  if (!current) {
    return problem(
      503,
      'no_host_api',
      'the viewer has no host API configured; start the image',
    )
  }
  // Outside the try: a bad path is the caller's mistake, not an unreachable Neo.
  const url = routeUrl(current.url, path)
  const headers = new Headers(init.headers)
  headers.set('authorization', `Bearer ${current.token}`)
  try {
    return await fetch(url, {
      ...init,
      headers,
      redirect: 'manual',
    })
  } catch (error) {
    if (init.signal?.aborted) throw error
    return problem(
      502,
      'host_api_unreachable',
      `Neo's host API at ${current.url.origin} did not answer: ${String(error)}`,
    )
  }
}

/** What a server-side WebSocket needs to open a host API upgrade route. */
export interface HostWebSocketTarget {
  /** The route's `ws:` URL. It never carries the token. */
  url: string
  /** `Authorization: Bearer …`: Neo takes the token only as a header, never in a query string. */
  headers: Record<string, string>
}

/**
 * The target for a host API upgrade route (`/v1/display?…`, `/v1/tabs/{tabId}/screencast`), for
 * Bun's client: `new WebSocket(target.url, { headers: target.headers })`. The viewer's relay
 * opens it server-side, so the token never reaches the browser. Throws when no host API is
 * configured (the image is not running under `bun run neo:viewer`).
 */
export function hostWebSocketTarget(path: string): HostWebSocketTarget {
  const current = target
  if (!current)
    throw new Error(
      'neo-viewer: no host API configured; start the image with bun run neo:viewer',
    )
  const url = routeUrl(current.url, path)
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:'
  return {
    url: url.href,
    headers: { authorization: `Bearer ${current.token}` },
  }
}

/** The route on the host API's origin. Only a path is taken, so no caller can send the token elsewhere. */
function routeUrl(base: URL, path: string): URL {
  if (!path.startsWith('/') || path.startsWith('//')) {
    throw new Error(`neo-viewer: a host API path starts with one "/": ${path}`)
  }
  return new URL(path, base.origin)
}

function problem(status: number, code: string, message: string): Response {
  return Response.json({ code, message }, { status })
}
