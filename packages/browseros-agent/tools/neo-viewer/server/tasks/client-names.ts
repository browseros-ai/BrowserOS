/**
 * Names the client behind a call. A 2026-07-28 client (Claude Code 2.1) sends `clientInfo` in
 * every request's `_meta`; an older one sends it once, in `initialize`, often on another
 * connection than its later calls. So the viewer remembers each `initialize` by connection, by
 * process and by User-Agent, and looks them up in that order. A client that never said falls
 * back to its User-Agent's product name, then "unknown".
 */

import type { ClientInfo } from '../../shared/api'

const REMEMBERED = 256

export interface ClientHints {
  connection: string | null
  pid: number | null
  userAgent: string | null
}

export class ClientNames {
  private readonly byConnection = new Map<string, ClientInfo>()
  private readonly byPid = new Map<number, ClientInfo>()
  private readonly byUserAgent = new Map<string, ClientInfo>()

  remember(client: ClientInfo, hints: ClientHints): void {
    if (hints.connection) remember(this.byConnection, hints.connection, client)
    if (hints.pid !== null) remember(this.byPid, hints.pid, client)
    if (hints.userAgent) remember(this.byUserAgent, hints.userAgent, client)
  }

  /** The client: what the call itself says, else what this connection, process or UA said. */
  resolve(sent: ClientInfo | null, hints: ClientHints): ClientInfo {
    if (sent) return sent
    return (
      (hints.connection
        ? this.byConnection.get(hints.connection)
        : undefined) ??
      (hints.pid !== null ? this.byPid.get(hints.pid) : undefined) ??
      (hints.userAgent ? this.byUserAgent.get(hints.userAgent) : undefined) ?? {
        name: productOf(hints.userAgent) ?? 'unknown',
      }
    )
  }
}

/** `claude-code/2.1.322 (sdk-cli)` → `claude-code`. */
function productOf(userAgent: string | null): string | null {
  const product = userAgent?.split(/[\s/]/)[0]
  return product ? product : null
}

/** A small LRU: a re-remembered key moves to the end, the oldest goes past `REMEMBERED`. */
function remember<K>(
  map: Map<K, ClientInfo>,
  key: K,
  client: ClientInfo,
): void {
  map.delete(key)
  map.set(key, client)
  if (map.size > REMEMBERED) {
    const oldest = map.keys().next()
    if (!oldest.done) map.delete(oldest.value)
  }
}
