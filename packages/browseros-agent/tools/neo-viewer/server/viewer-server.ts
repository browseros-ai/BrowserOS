/**
 * Builds and starts the viewer: one Bun server on 127.0.0.1 that serves the agent's MCP
 * endpoint (`/mcp`, proxied to Neo), the JSON API (`/api/*`), the live screen's relay to Neo's
 * display (`/api/live/*`) and the web page (`/`).
 */

import type { ViewerInfo } from '../shared/api'
import { apiRouteTable, isLocalRequest } from './api/api-routes'
import type { ViewerConfig } from './config'
import { configureHostApi } from './hostApi'
import { type LiveHost, viewerHost } from './live-screen/live-host'
import { liveScreen } from './live-screen/live-routes'
import { socketSessions } from './live-screen/socket-session'
import { RunStore } from './log/run-store'
import { containerUp, runCommand } from './neo-container'
import { joinNeoRuns } from './neo-runs'
import { buildPage, type PageFile } from './page'
import { handleMcp, type McpProxyDeps } from './proxy/mcp-proxy'

/** The longest `stop` waits for open connections to close. */
const STOP_WAIT_MS = 2_000

/** Seams tests replace: the clock and Neo's host API. */
export interface ViewerSeams {
  now?: () => number
  liveHost?: LiveHost
}

export interface RunningViewer {
  url: string
  mcpUrl: string
  store: RunStore
  stop(): Promise<void>
}

export async function startViewer(
  config: ViewerConfig,
  seams: ViewerSeams = {},
): Promise<RunningViewer> {
  const now = seams.now ?? Date.now
  const [store, page] = await Promise.all([
    RunStore.open(config.dataDir),
    config.page ?? buildPage(),
    configureHostApi(config.hostApi ?? null),
  ])
  const live = liveScreen(seams.liveHost ?? viewerHost)
  const proxy: McpProxyDeps = {
    upstream: config.upstream,
    store,
    clients: new Map(),
    now,
  }
  // Neo's record of each run, read through the host API once its reply is in.
  const joiner = joinNeoRuns(store)
  const info: Omit<ViewerInfo, 'container'> = {
    mcpUrl: '',
    upstreamUrl: config.upstream.href,
    dataDir: config.dataDir,
    startedAt: new Date(now()).toISOString(),
  }
  const container = config.container ?? null
  const currentInfo = async (): Promise<ViewerInfo> => ({
    ...info,
    container: container
      ? {
          name: container.names.container,
          image: container.image,
          up: await containerUp(runCommand, container.names),
        }
      : null,
  })

  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: config.port,
    // A run can stream for minutes; Neo's keep-alive arrives every 5 s, but never time out a call.
    idleTimeout: 0,
    routes: {
      ...pageRoutes(page),
      ...apiRouteTable(store, currentInfo),
      ...live.routes,
    },
    // Each upgraded socket carries its own session (the live screen's display relay).
    websocket: socketSessions,
    fetch(req) {
      const { pathname } = new URL(req.url)
      if (pathname === '/mcp' || pathname.startsWith('/mcp/'))
        return handleMcp(req, proxy)
      return new Response('not found\n', { status: 404 })
    },
  })

  const port = server.port ?? config.port
  const url = `http://127.0.0.1:${port}`
  info.mcpUrl = `${url}/mcp`
  return {
    url,
    mcpUrl: info.mcpUrl,
    store,
    async stop() {
      await live.stop()
      // Bun before 1.4.2 (the repo's pin) never settles `stop` once the server has closed a
      // WebSocket itself, as the display relay does; a local Bun can be older, and Ctrl+C
      // must still exit.
      await Promise.race([server.stop(true), Bun.sleep(STOP_WAIT_MS)])
      await joiner.stop()
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
