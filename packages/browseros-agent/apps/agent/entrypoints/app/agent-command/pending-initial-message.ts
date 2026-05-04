import type { StagedAttachment } from '@/lib/attachments'

/**
 * Same-tab in-memory handoff between the `/home` composer and the
 * chat screen at `/home/agents/:agentId`. URL search params (`?q=`)
 * carry the text fine, but cannot carry binary attachments — a multi-
 * megabyte image dataUrl would explode URL length limits and round-
 * trip badly. This module is the rich-data side channel for the same
 * navigation: the composer writes here, the chat screen reads here on
 * mount.
 *
 * Intentionally module-scope. Same render tree, same tab — no need
 * for sessionStorage (which would force JSON-serialising the dataUrls
 * and re-parsing on the read side). Cross-tab handoff is out of
 * scope: the user typing at home in tab A and switching to tab B's
 * chat would surface an empty registry there, which is the correct
 * behaviour.
 */

export interface PendingInitialMessage {
  agentId: string
  text: string
  attachments: StagedAttachment[]
  createdAt: number
}

/**
 * 10s TTL on the entry. A stale entry from a back-button journey
 * shouldn't fire on a future visit; if real-world latency makes 10s
 * too tight under slow harness boot, bump but never make it
 * indefinite.
 */
const PENDING_TTL_MS = 10_000

let pending: PendingInitialMessage | null = null
let pendingTimer: ReturnType<typeof setTimeout> | null = null

function clearPending(): void {
  pending = null
  if (pendingTimer !== null) {
    clearTimeout(pendingTimer)
    pendingTimer = null
  }
}

export function setPendingInitialMessage(payload: PendingInitialMessage): void {
  // Defensive: the home composer should never call this without an
  // agent selected. If it somehow does, no-op rather than holding a
  // payload we can't route.
  if (!payload.agentId) return
  clearPending()
  pending = payload
  pendingTimer = setTimeout(clearPending, PENDING_TTL_MS)
}

/**
 * Destructive read. Returns the entry only if `agentId` matches and
 * the entry is fresh; clears the entry on success so Strict-Mode
 * double-invokes can't double-send.
 */
export function consumePendingInitialMessage(
  agentId: string,
): PendingInitialMessage | null {
  if (!pending) return null
  if (pending.agentId !== agentId) return null
  if (Date.now() - pending.createdAt >= PENDING_TTL_MS) {
    clearPending()
    return null
  }
  const entry = pending
  clearPending()
  return entry
}

/**
 * Non-mutating read for tests. Production code should never need this
 * — use `consume` and own the lifecycle.
 */
export function peekPendingInitialMessage(): PendingInitialMessage | null {
  return pending
}
