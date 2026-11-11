/**
 * The viewer's settings, from flags or environment. The viewer always listens on 127.0.0.1;
 * only the port moves.
 */

import { homedir } from 'node:os'
import { join, resolve } from 'node:path'
import { parseArgs } from 'node:util'
import type { HostApiConfig } from './hostApi'

/** The port the agent's MCP URL and the UI use (the build contract's `:7310`). */
const DEFAULT_PORT = 7310

/** What the running viewer serves from. */
export interface ViewerConfig {
  /** 0 picks a free port (tests). */
  port: number
  /** Neo's `/mcp`. */
  upstream: URL
  dataDir: string
  /** Neo's host API over `CLAW_HOST_TCP`; none when null. */
  hostApi?: HostApiConfig | null
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
  /** With `--upstream`: a host API to use (the one command sets its own). */
  hostApi: HostApiConfig | null
}

const USAGE = `usage: bun run neo:viewer [-- options]   (from packages/browseros-agent)
  Builds this checkout's Neo SDK image if needed, starts it fresh, and serves the viewer.
  --keep-data       keep the browser profile and the run history between starts
  --no-build        use the newest neo-sdk image on this machine
  --image TAG       use this image
  --upstream URL    forward to this Neo MCP endpoint and start no container
  --port PORT       the viewer's port (7310)
  --data-dir DIR    the viewer's state (~/.local/state/neo-viewer)
  --host-token-file FILE [--host-api URL]
                    with --upstream: Neo's host API (CLAW_HOST_TCP, default http://127.0.0.1:9300)
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
      'host-api': { type: 'string' },
      'host-token-file': { type: 'string' },
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
    // Absolute: the token file inside it is bind-mounted into the container by path.
    dataDir: resolve(
      values['data-dir'] ?? env.NEO_VIEWER_DATA_DIR ?? defaultDataDir(env),
    ),
    upstream: upstream ? new URL(upstream) : null,
    keepData: values['keep-data'] ?? false,
    build: !values['no-build'],
    image: values.image ?? null,
    hostApi: hostApiFrom(values['host-api'], values['host-token-file']),
  }
}

/** A host API named on the command line: its token file, and its URL or the usual port. */
function hostApiFrom(
  url: string | undefined,
  tokenFile: string | undefined,
): HostApiConfig | null {
  if (url && !tokenFile)
    throw new Error(
      `a host API needs its token file (--host-token-file)\n${USAGE}`,
    )
  return tokenFile
    ? {
        url: new URL(url ?? 'http://127.0.0.1:9300'),
        tokenFile: resolve(tokenFile),
      }
    : null
}

/** `$XDG_STATE_HOME/neo-viewer`, else `~/.local/state/neo-viewer`: outside any checkout. */
function defaultDataDir(env: Record<string, string | undefined>): string {
  return join(
    env.XDG_STATE_HOME ?? join(homedir(), '.local', 'state'),
    'neo-viewer',
  )
}
