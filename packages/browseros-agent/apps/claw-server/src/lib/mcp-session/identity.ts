/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-session identity map. In v2 the cockpit exposes one standard
 * MCP endpoint at `POST /cockpit/mcp` and every agent connects to it
 * over a `StreamableHTTPTransport` session. The transport assigns an
 * `mcp-session-id` at handshake time; we use that id as the key into
 * this map so subsequent `tools/call` requests on the same session
 * can be attributed to the connecting client.
 *
 * Records live for the lifetime of the session. When the transport
 * reports `close` (clean disconnect) or `error` (abrupt), the route
 * layer is expected to call `dropSession(sessionId)`. Polled reads
 * tolerate a stale identity; the homepage degrades to "no label"
 * rather than crashing.
 *
 * `agentIdentityFromClient` is the bridge between the identity map
 * and the existing `tabActivityRegistry`, which keys on
 * `{ agentId, slug }`. The cleaner produces a stable handle from
 * `clientInfo.name` when the client identifies itself politely, or
 * falls back to a session-derived `unknown-<hash>` so the registry
 * key is always non-empty.
 */

export interface ClientIdentity {
  sessionId: string
  clientName: string
  clientVersion: string
  clientTitle: string | null
  firstSeenAt: number
}

export interface IdentityService {
  registerInitialize(input: {
    sessionId: string
    clientInfo: {
      name?: string | undefined
      version?: string | undefined
      title?: string | undefined
    }
  }): ClientIdentity
  getIdentity(sessionId: string): ClientIdentity | null
  dropSession(sessionId: string): void
  /** Snapshot of every live identity. Used by the tabs route to enrich registry records by agentId. */
  list(): ClientIdentity[]
  // Test-only escape hatches mirroring the tab-activity registry.
  size(): number
  clear(): void
}

export interface IdentityServiceDeps {
  now?: () => number
}

const SLUG_MAX_LEN = 64
const HASH_TAIL_LEN = 6

export function createIdentityService(
  deps: IdentityServiceDeps = {},
): IdentityService {
  const records = new Map<string, ClientIdentity>()
  const now = deps.now ?? (() => Date.now())

  return {
    registerInitialize(input) {
      const record: ClientIdentity = {
        sessionId: input.sessionId,
        clientName: input.clientInfo.name?.trim() ?? '',
        clientVersion: input.clientInfo.version?.trim() ?? '',
        clientTitle: input.clientInfo.title?.trim() || null,
        firstSeenAt: now(),
      }
      records.set(input.sessionId, record)
      return record
    },
    getIdentity(sessionId) {
      return records.get(sessionId) ?? null
    },
    dropSession(sessionId) {
      records.delete(sessionId)
    },
    list() {
      return Array.from(records.values())
    },
    size() {
      return records.size
    },
    clear() {
      records.clear()
    },
  }
}

/**
 * Lowercase alphanumeric + hyphen, trimmed, capped. Returns the
 * cleaned handle, or an empty string when nothing usable remains
 * (caller falls back to the synthetic hash).
 */
export function slugifyClientName(raw: string): string {
  const cleaned = raw
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  if (cleaned.length === 0) return ''
  return cleaned.slice(0, SLUG_MAX_LEN)
}

/**
 * Stable, obviously-synthetic fallback handle derived from the
 * session id. Same session always produces the same hash so the
 * registry sees one "agent" even if `clientInfo.name` is missing.
 */
export function fallbackSlugForSession(sessionId: string): string {
  // FNV-1a 32-bit; deterministic and dependency-free.
  let hash = 0x811c9dc5
  for (let i = 0; i < sessionId.length; i++) {
    hash ^= sessionId.charCodeAt(i)
    hash =
      (hash +
        ((hash << 1) +
          (hash << 4) +
          (hash << 7) +
          (hash << 8) +
          (hash << 24))) >>>
      0
  }
  const tail = hash.toString(16).padStart(8, '0').slice(0, HASH_TAIL_LEN)
  return `unknown-${tail}`
}

/**
 * Bridge from `ClientIdentity` to the `{ agentId, slug }` pair the
 * `tabActivityRegistry` expects. agentId and slug both equal the
 * cleaner output when `clientInfo.name` is usable, otherwise both
 * equal the session-derived fallback.
 */
export function agentIdentityFromClient(identity: ClientIdentity): {
  agentId: string
  slug: string
} {
  const cleaned = slugifyClientName(identity.clientName)
  if (cleaned.length > 0) {
    return { agentId: cleaned, slug: cleaned }
  }
  const fallback = fallbackSlugForSession(identity.sessionId)
  return { agentId: fallback, slug: fallback }
}
