/**
 * Names the client behind a call. A 2026-07-28 client (Claude Code 2.1) sends `clientInfo` in
 * every request's `_meta`; an older one sends it once, in `initialize`, often on another
 * connection than its later calls. So the viewer remembers each `initialize` by process, by
 * connection and by User-Agent, and looks them up in that order. A client that never said falls
 * back to its User-Agent's product name, then "unknown".
 *
 * A connection's address and port are reused once it closes, so a name remembered by connection
 * is trusted only for a short while after it was last seen.
 */

import type { ClientInfo } from '../../shared/api'

const REMEMBERED = 256
const CONNECTION_NAME_MS = 30_000

export interface ClientHints {
  connection: string | null
  pid: number | null
  userAgent: string | null
}

interface Seen {
  client: ClientInfo
  at: number
}

export class ClientNames {
  private readonly byConnection = new Map<string, Seen>()
  private readonly byPid = new Map<number, ClientInfo>()
  private readonly byUserAgent = new Map<string, ClientInfo>()

  constructor(private readonly now: () => number = Date.now) {}

  remember(client: ClientInfo, hints: ClientHints): void {
    if (hints.connection) {
      remember(this.byConnection, hints.connection, { client, at: this.now() })
    }
    if (hints.pid !== null) remember(this.byPid, hints.pid, client)
    if (hints.userAgent) remember(this.byUserAgent, hints.userAgent, client)
  }

  /** The client: what the call itself says, else what its process, connection or UA said. */
  resolve(sent: ClientInfo | null, hints: ClientHints): ClientInfo {
    if (sent) return sent
    const byPid = hints.pid !== null ? this.byPid.get(hints.pid) : undefined
    if (byPid) return byPid
    const byConnection = this.fromConnection(hints.connection)
    if (byConnection) return byConnection
    const byUserAgent = hints.userAgent
      ? this.byUserAgent.get(hints.userAgent)
      : undefined
    return byUserAgent ?? { name: productOf(hints.userAgent) ?? 'unknown' }
  }

  private fromConnection(connection: string | null): ClientInfo | undefined {
    const seen = connection ? this.byConnection.get(connection) : undefined
    if (!seen || !connection) return undefined
    const now = this.now()
    if (now - seen.at > CONNECTION_NAME_MS) {
      this.byConnection.delete(connection)
      return undefined
    }
    seen.at = now
    return seen.client
  }
}

/** `claude-code/2.1.322 (sdk-cli)` → `claude-code`. */
function productOf(userAgent: string | null): string | null {
  const product = userAgent?.split(/[\s/]/)[0]
  return product ? product : null
}

/** A small LRU: a re-remembered key moves to the end, the oldest goes past `REMEMBERED`. */
function remember<K, V>(map: Map<K, V>, key: K, value: V): void {
  map.delete(key)
  map.set(key, value)
  if (map.size > REMEMBERED) {
    const oldest = map.keys().next()
    if (!oldest.done) map.delete(oldest.value)
  }
}
