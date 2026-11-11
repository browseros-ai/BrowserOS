/**
 * The viewer's settings, from flags or environment. The viewer always listens on 127.0.0.1;
 * only the port moves.
 */

import { homedir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'

/** The port the agent's MCP URL and the UI use (the build contract's `:7310`). */
const DEFAULT_PORT = 7310

/** What the running viewer serves from. */
export interface ViewerConfig {
  /** 0 picks a free port (tests). */
  port: number
  /** Neo's `/mcp`. */
  upstream: URL
  dataDir: string
  /** The container `bun run neo:viewer` started, shown read-only on the page; null with `--upstream`. */
  container?: { name: string; image: string } | null
}

/** What `bun run neo:viewer` was asked to do. */
export interface LaunchOptions {
  port: number
  dataDir: string
  /** Forward to this Neo and start no container. */
  upstream: URL | null
  /** Keep the browser profile and the run history between starts. */
  keepData: boolean
  /** False: use the newest neo-sdk image on the machine instead of this checkout's. */
  build: boolean
  /** Run this image; nothing is built. */
  image: string | null
}

const USAGE = `usage: bun run neo:viewer [-- options]   (from packages/browseros-agent)
  Builds this checkout's Neo SDK image if needed, starts it fresh, and serves the viewer.
  --keep-data       keep the browser profile and the run history between starts
  --no-build        use the newest neo-sdk image on this machine
  --image TAG       use this image
  --upstream URL    forward to this Neo MCP endpoint and start no container
  --port PORT       the viewer's port (7310)
  --data-dir DIR    the viewer's state (~/.local/state/neo-viewer)
  env: NEO_VIEWER_PORT, NEO_VIEWER_UPSTREAM, NEO_VIEWER_DATA_DIR`

export function launchOptionsFrom(
  argv: string[],
  env: Record<string, string | undefined>,
): LaunchOptions {
  const { values } = parseArgs({
    args: argv,
    options: {
      port: { type: 'string' },
      upstream: { type: 'string' },
      'data-dir': { type: 'string' },
      'keep-data': { type: 'boolean' },
      'no-build': { type: 'boolean' },
      image: { type: 'string' },
      help: { type: 'boolean' },
    },
  })
  if (values.help) {
    console.log(USAGE)
    process.exit(0)
  }
  const port = Number(values.port ?? env.NEO_VIEWER_PORT ?? DEFAULT_PORT)
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`bad port: ${port}\n${USAGE}`)
  }
  const upstream = values.upstream ?? env.NEO_VIEWER_UPSTREAM
  return {
    port,
    dataDir:
      values['data-dir'] ?? env.NEO_VIEWER_DATA_DIR ?? defaultDataDir(env),
    upstream: upstream ? new URL(upstream) : null,
    keepData: values['keep-data'] ?? false,
    build: !values['no-build'],
    image: values.image ?? null,
  }
}

/** `$XDG_STATE_HOME/neo-viewer`, else `~/.local/state/neo-viewer`: outside any checkout. */
function defaultDataDir(env: Record<string, string | undefined>): string {
  return join(
    env.XDG_STATE_HOME ?? join(homedir(), '.local', 'state'),
    'neo-viewer',
  )
}
