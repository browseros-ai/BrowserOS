/**
 * The viewer's settings, from flags or environment. The viewer always listens on 127.0.0.1;
 * only the port moves.
 */

import { homedir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'

/** The port the agent's MCP URL and the UI use (the build contract's `:7310`). */
const DEFAULT_PORT = 7310
/** The Neo SDK image's MCP endpoint, published on 127.0.0.1 as its README shows. */
const DEFAULT_UPSTREAM = 'http://127.0.0.1:9010/mcp'

export interface ViewerConfig {
  /** 0 picks a free port (tests). */
  port: number
  upstream: URL
  dataDir: string
}

const USAGE = `usage: bun server/main.ts [--port 7310] [--upstream http://127.0.0.1:9010/mcp] [--data-dir DIR]
  env: NEO_VIEWER_PORT, NEO_VIEWER_UPSTREAM, NEO_VIEWER_DATA_DIR`

export function configFrom(
  argv: string[],
  env: Record<string, string | undefined>,
): ViewerConfig {
  const { values } = parseArgs({
    args: argv,
    options: {
      port: { type: 'string' },
      upstream: { type: 'string' },
      'data-dir': { type: 'string' },
      help: { type: 'boolean' },
    },
  })
  if (values.help) {
    console.log(USAGE)
    process.exit(0)
  }
  const port = Number(values.port ?? env.NEO_VIEWER_PORT ?? DEFAULT_PORT)
  if (!Number.isInteger(port) || port < 0 || port > 65535)
    throw new Error(`bad port: ${port}\n${USAGE}`)
  return {
    port,
    upstream: new URL(
      values.upstream ?? env.NEO_VIEWER_UPSTREAM ?? DEFAULT_UPSTREAM,
    ),
    dataDir:
      values['data-dir'] ?? env.NEO_VIEWER_DATA_DIR ?? defaultDataDir(env),
  }
}

/** `$XDG_STATE_HOME/neo-viewer`, else `~/.local/state/neo-viewer`: outside any checkout. */
function defaultDataDir(env: Record<string, string | undefined>): string {
  return join(
    env.XDG_STATE_HOME ?? join(homedir(), '.local', 'state'),
    'neo-viewer',
  )
}
