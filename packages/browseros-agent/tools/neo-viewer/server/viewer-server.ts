/**
 * Builds and starts the viewer: one Bun server on 127.0.0.1 that serves the agent's MCP
 * endpoint (`/mcp`, proxied to Neo), the JSON API (`/api/*`) and the web page (`/`).
 */

import type { ViewerInfo } from '../shared/api'
import { apiRouteTable, isLocalRequest } from './api/api-routes'
import type { ViewerConfig } from './config'
import { TaskStore } from './log/task-store'
import { buildPage, type PageFile } from './page'
import { handleMcp, type McpProxyDeps } from './proxy/mcp-proxy'
import { ClientNames } from './tasks/client-names'
import { PeerProcessFinder, type ProcessFinder } from './tasks/peer-process'
import { type CallOrigin, DEFAULT_IDLE_MS, TaskRule } from './tasks/task-rule'

/** Seams tests replace: the clock, the process lookup and the task id generator. */
export interface ViewerSeams {
  now?: () => number
  processes?: (listenPort: number) => ProcessFinder
  newTaskId?: (origin: CallOrigin) => string
  idleMs?: number
}

export interface RunningViewer {
  url: string
  mcpUrl: string
  store: TaskStore
  stop(): Promise<void>
}

export async function startViewer(
  config: ViewerConfig,
  seams: ViewerSeams = {},
): Promise<RunningViewer> {
  const now = seams.now ?? Date.now
  const rule = new TaskRule(
    seams.newTaskId ?? newTaskId,
    seams.idleMs ?? DEFAULT_IDLE_MS,
  )
  const [store, page] = await Promise.all([
    TaskStore.open(config.dataDir, rule),
    buildPage(),
  ])
  // The proxy needs the bound port (for the process lookup) before the first request.
  let proxy: McpProxyDeps | null = null
  const info: ViewerInfo = {
    mcpUrl: '',
    upstreamUrl: config.upstream.href,
    dataDir: config.dataDir,
    startedAt: new Date(now()).toISOString(),
  }

  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: config.port,
    // A run can stream for minutes; Neo's keep-alive arrives every 5 s, but never time out a call.
    idleTimeout: 0,
    routes: {
      ...pageRoutes(page),
      ...apiRouteTable(store, info),
    },
    fetch(req, server) {
      const { pathname } = new URL(req.url)
      if ((pathname === '/mcp' || pathname.startsWith('/mcp/')) && proxy) {
        const ip = server.requestIP(req)
        return handleMcp(
          req,
          ip ? { address: ip.address, port: ip.port } : null,
          proxy,
        )
      }
      return new Response('not found\n', { status: 404 })
    },
  })

  const port = server.port ?? config.port
  const url = `http://127.0.0.1:${port}`
  info.mcpUrl = `${url}/mcp`
  proxy = {
    upstream: config.upstream,
    store,
    rule,
    processes:
      seams.processes?.(port) ?? new PeerProcessFinder({ listenPort: port }),
    clients: new ClientNames(now),
    now,
  }
  return {
    url,
    mcpUrl: info.mcpUrl,
    store,
    async stop() {
      await server.stop(true)
      await store.flush()
    },
  }
}

/** `/` and each built asset, by its name in the built `index.html`. */
function pageRoutes(
  files: Map<string, PageFile>,
): Record<string, (req: Request) => Response> {
  const routes: Record<string, (req: Request) => Response> = {}
  for (const [name, file] of files) {
    const serve = (req: Request) =>
      isLocalRequest(req)
        ? new Response(file.body, { headers: { 'content-type': file.type } })
        : new Response('forbidden host\n', { status: 403 })
    routes[`/${name}`] = serve
    if (name === 'index.html') routes['/'] = serve
  }
  return routes
}

/** `claude-code-20261111-101403-3f9a`: the client, the time, and enough randomness to be unique. */
function newTaskId(origin: CallOrigin): string {
  const slug =
    origin.client.name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-|-$/g, '')
      .slice(0, 40) || 'client'
  const stamp = new Date(origin.at)
    .toISOString()
    .replace(/[-:]/g, '')
    .replace('T', '-')
    .slice(0, 15)
  const suffix = crypto
    .getRandomValues(new Uint8Array(2))
    .reduce((text, byte) => text + byte.toString(16).padStart(2, '0'), '')
  return `${slug}-${stamp}-${suffix}`
}
